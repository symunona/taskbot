package com.hermes.voice

import com.hermes.tools.ToolRegistry
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import kotlinx.serialization.json.*
import kotlin.io.encoding.Base64
import kotlin.io.encoding.ExperimentalEncodingApi

class VoiceSession(
    private val apiKey: String,
    private val systemInstruction: String? = null,
    private val tools: JsonArray? = null,
    private val toolRegistry: ToolRegistry? = null
) {
    private val client = GeminiLiveClient(apiKey)
    private val voiceManager = VoiceManager()
    private val scope = CoroutineScope(Dispatchers.Default + Job())

    private val _transcripts = MutableSharedFlow<String>()
    val transcripts: SharedFlow<String> = _transcripts.asSharedFlow()

    @OptIn(ExperimentalEncodingApi::class)
    suspend fun start() {
        client.connect(systemInstruction, tools)

        voiceManager.startRecording { audioData ->
            scope.launch {
                client.sendAudio(audioData)
            }
        }

        scope.launch {
            client.incomingMessages.collect { message ->
                handleIncomingMessage(message)
            }
        }
    }

    @OptIn(ExperimentalEncodingApi::class)
    private fun handleIncomingMessage(message: JsonObject) {
        val serverContent = message["serverContent"]?.jsonObject
        val modelTurn = serverContent?.get("modelTurn")?.jsonObject
        val parts = modelTurn?.get("parts")?.jsonArray

        parts?.forEach { partElement ->
            val part = partElement.jsonObject
            
            // Handle audio
            val inlineData = part["inlineData"]?.jsonObject
            if (inlineData != null) {
                val mimeType = inlineData["mimeType"]?.jsonPrimitive?.content
                if (mimeType?.startsWith("audio/pcm") == true) {
                    val data = inlineData["data"]?.jsonPrimitive?.content
                    if (data != null) {
                        val pcmData = Base64.decode(data)
                        voiceManager.playAudio(pcmData)
                    }
                }
            }

            // Handle text
            val text = part["text"]?.jsonPrimitive?.content
            if (text != null) {
                scope.launch {
                    _transcripts.emit(text)
                }
            }

            // Handle function calls
            val functionCall = part["functionCall"]?.jsonObject
            if (functionCall != null) {
                val name = functionCall["name"]?.jsonPrimitive?.content
                val args = functionCall["args"]?.jsonObject ?: buildJsonObject {}
                val callId = functionCall["id"]?.jsonPrimitive?.content
                
                if (name != null && callId != null && toolRegistry != null) {
                    scope.launch {
                        val result = toolRegistry.execute(name, args)
                        client.sendToolResponse(callId, name, result)
                    }
                }
            }
        }
    }

    fun stop() {
        voiceManager.stopRecording()
        voiceManager.stopPlayback()
        client.disconnect()
        scope.cancel()
    }
}
