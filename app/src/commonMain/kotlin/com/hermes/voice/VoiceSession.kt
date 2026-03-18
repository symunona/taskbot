package com.hermes.voice

import com.hermes.tools.ToolRegistry
import com.hermes.ui.EventLogger
import kotlinx.coroutines.CoroutineExceptionHandler
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlin.io.encoding.Base64
import kotlin.io.encoding.ExperimentalEncodingApi

data class VoiceTranscript(
    val role: String,
    val text: String,
    val isPartial: Boolean = false
)

class VoiceSession(
    private val apiKey: String,
    private val systemInstruction: String? = null,
    private val tools: JsonArray? = null,
    private val toolRegistry: ToolRegistry? = null,
    private val onError: ((Throwable) -> Unit)? = null
) {
    private val client = GeminiLiveClient(apiKey)
    private val voiceManager = VoiceManager()
    private val scope = CoroutineScope(
        Dispatchers.Default +
            Job() +
            CoroutineExceptionHandler { _, throwable ->
                EventLogger.log("Voice session failed: ${throwable.message ?: throwable}", isError = true)
                onError?.invoke(throwable)
            }
    )
    private var latestUserTranscript = ""
    private var latestModelTranscript = ""

    private val _transcripts = MutableSharedFlow<VoiceTranscript>()
    val transcripts: SharedFlow<VoiceTranscript> = _transcripts

    suspend fun start() {
        client.connect(systemInstruction, tools)

        scope.launch {
            client.incomingMessages.collect { message ->
                handleIncomingMessage(message)
            }
        }

        voiceManager.startRecording { audioData ->
            scope.launch {
                try {
                    client.sendAudio(audioData)
                } catch (t: Throwable) {
                    EventLogger.log("Failed to stream audio: ${t.message ?: t}", isError = true)
                    onError?.invoke(t)
                }
            }
        }
    }

    @OptIn(ExperimentalEncodingApi::class)
    private fun handleIncomingMessage(message: JsonObject) {
        val serverContent = message["serverContent"]?.jsonObject
        val toolCall = message["toolCall"]?.jsonObject

        if (serverContent != null) {
            if (serverContent["interrupted"]?.jsonPrimitive?.contentOrNull == "true") {
                EventLogger.log("Gemini Live response interrupted")
                voiceManager.stopPlayback()
            }

            emitTranscript(
                role = "user",
                transcription = serverContent["inputTranscription"]?.jsonObject,
                previousText = latestUserTranscript,
                updatePrevious = { latestUserTranscript = it }
            )

            emitTranscript(
                role = "model",
                transcription = serverContent["outputTranscription"]?.jsonObject,
                previousText = latestModelTranscript,
                updatePrevious = { latestModelTranscript = it }
            )

            val modelTurn = serverContent["modelTurn"]?.jsonObject
            val parts = modelTurn?.get("parts")?.jsonArray
            parts?.forEach { partElement ->
                val part = partElement.jsonObject

                part["inlineData"]?.jsonObject?.let { inlineData ->
                    val mimeType = inlineData["mimeType"]?.jsonPrimitive?.contentOrNull
                    val data = inlineData["data"]?.jsonPrimitive?.contentOrNull
                    if (mimeType?.startsWith("audio/pcm") == true && data != null) {
                        val pcmData = Base64.decode(data)
                        voiceManager.playAudio(pcmData)
                    }
                }

                part["text"]?.jsonPrimitive?.contentOrNull
                    ?.trim()
                    ?.takeIf { it.isNotBlank() }
                    ?.let { text ->
                        scope.launch {
                            _transcripts.emit(VoiceTranscript(role = "model", text = text))
                        }
                    }
            }
        }

        if (toolCall != null && toolRegistry != null) {
            val functionCalls = toolCall["functionCalls"]?.jsonArray ?: return
            scope.launch {
                val responses = buildJsonArray {
                    functionCalls.forEach { functionCallElement ->
                        val functionCall = functionCallElement.jsonObject
                        val name = functionCall["name"]?.jsonPrimitive?.contentOrNull ?: return@forEach
                        val callId = functionCall["id"]?.jsonPrimitive?.contentOrNull ?: return@forEach
                        val args = functionCall["args"]?.jsonObject ?: buildJsonObject {}
                        val result = toolRegistry.execute(name, args)
                        add(buildJsonObject {
                            put("id", callId)
                            put("name", name)
                            put("response", result)
                        })
                    }
                }
                if (responses.isNotEmpty()) {
                    EventLogger.log("Sending Gemini Live tool response")
                    client.sendToolResponse(responses)
                }
            }
        }
    }

    private fun emitTranscript(
        role: String,
        transcription: JsonObject?,
        previousText: String,
        updatePrevious: (String) -> Unit
    ) {
        val text = transcription
            ?.get("text")
            ?.jsonPrimitive
            ?.contentOrNull
            ?.trim()
            ?.takeIf { it.isNotBlank() }
            ?: return

        if (text == previousText) {
            return
        }

        val isPartial = previousText.isNotBlank() && text.startsWith(previousText)
        updatePrevious(text)
        scope.launch {
            _transcripts.emit(VoiceTranscript(role = role, text = text, isPartial = isPartial))
        }
    }

    fun stop() {
        voiceManager.stopRecording()
        voiceManager.stopPlayback()
        client.disconnect(sendAudioStreamEnd = true)
        scope.cancel()
    }
}
