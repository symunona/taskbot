package space.tmpx.taltosbot.voice

import space.tmpx.taltosbot.llm.parseCallUsage
import space.tmpx.taltosbot.ui.EventLogger
import space.tmpx.taltosbot.ui.LogLevel
import kotlinx.coroutines.CoroutineExceptionHandler
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.Channel
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

/**
 * Mic -> Gemini Live -> speaker, plus tool calls.
 *
 * [onEnded] fires at most once, from a background dispatcher, when the session dies on its own
 * (Gemini closed the socket, sent `goAway`, or a send failed). The owner must then call [stop].
 */
class VoiceSession(
    apiKey: String,
    model: String,
    private val systemInstruction: String? = null,
    private val tools: JsonArray? = null,
    private val executeTool: suspend (name: String, args: JsonObject) -> JsonObject,
    private val onEnded: (reason: String) -> Unit
) {
    private var ended = false
    private fun end(reason: String) {
        if (ended) return
        ended = true
        onEnded(reason)
    }

    private val client = GeminiLiveClient(apiKey, model, onClosed = ::end)
    private val voiceManager = VoiceManager()
    private val scope = CoroutineScope(
        Dispatchers.Default +
            Job() +
            CoroutineExceptionHandler { _, throwable ->
                EventLogger.log(client.redact("VoiceSession: uncaught: ${throwable.message ?: throwable}"), isError = true)
                end(client.redact("Voice session error: ${throwable.message ?: throwable}"))
            }
    )
    private var latestUserTranscript = ""
    private var latestModelTranscript = ""

    // Channel to serialize outgoing audio sends (prevents coroutine explosion)
    private val audioOutChannel = Channel<ByteArray>(
        capacity = 50,
        onBufferOverflow = kotlinx.coroutines.channels.BufferOverflow.DROP_OLDEST
    )

    private val _transcripts = MutableSharedFlow<VoiceTranscript>(extraBufferCapacity = 64)
    val transcripts: SharedFlow<VoiceTranscript> = _transcripts

    /** Token counts of the session so far, summed over Live `usageMetadata` messages. */
    var tokensIn = 0L
        private set
    var tokensOut = 0L
        private set
    /** Prompt size of the newest model turn: what the context holds now. Null = no turn yet. */
    var contextTokens: Long? = null
        private set

    suspend fun start() {
        EventLogger.log("VoiceSession: starting...")
        client.connect(systemInstruction, tools)
        EventLogger.log("VoiceSession: connected, starting audio capture")

        // Single coroutine to send audio - prevents Dispatchers.Default starvation
        scope.launch {
            var buffered = ByteArray(0)
            for (chunk in audioOutChannel) {
                buffered += chunk
                if (buffered.size >= 4096) {
                    try {
                        client.sendAudio(buffered)
                        buffered = ByteArray(0)
                    } catch (t: Throwable) {
                        EventLogger.log(client.redact("VoiceSession: audio send failed: ${t.message}"), isError = true)
                        end(client.redact("Voice connection lost (${t.message ?: t})"))
                        break
                    }
                }
            }
        }

        scope.launch {
            client.incomingMessages.collect { message ->
                handleIncomingMessage(message)
            }
        }

        voiceManager.startRecording { audioData ->
            // Drop oldest if buffer full to prevent backpressure freeze
            audioOutChannel.trySend(audioData)
        }
        EventLogger.log("VoiceSession: recording started")
    }

    /** Hands Gemini a text turn to respond to out loud (used to relay agent messages). */
    fun sendText(text: String) {
        scope.launch {
            try {
                client.sendText(text)
            } catch (t: Throwable) {
                EventLogger.log(client.redact("VoiceSession: text send failed: ${t.message}"), isError = true)
            }
        }
    }

    @OptIn(ExperimentalEncodingApi::class)
    private fun handleIncomingMessage(message: JsonObject) {
        val serverContent = message["serverContent"]?.jsonObject
        val toolCall = message["toolCall"]?.jsonObject
        parseCallUsage(message["usageMetadata"] as? JsonObject)?.let {
            tokensIn += it.input
            tokensOut += it.output
            contextTokens = it.input
        }

        if (serverContent != null) {
            if (serverContent["interrupted"] != null) {
                EventLogger.log("GeminiLive: response interrupted")
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
                // Text parts in modelTurn are model "thinking" - skip.
                // Real transcription comes via outputTranscription.
            }
        }

        if (toolCall != null) {
            val functionCalls = toolCall["functionCalls"]?.jsonArray ?: return
            scope.launch {
                val responses = buildJsonArray {
                    functionCalls.forEach { functionCallElement ->
                        val functionCall = functionCallElement.jsonObject
                        val name = functionCall["name"]?.jsonPrimitive?.contentOrNull ?: return@forEach
                        val callId = functionCall["id"]?.jsonPrimitive?.contentOrNull ?: return@forEach
                        val args = functionCall["args"]?.jsonObject ?: buildJsonObject {}
                        EventLogger.log("VoiceSession: tool call $name")
                        val result = executeTool(name, args)
                        add(buildJsonObject {
                            put("id", callId)
                            put("name", name)
                            put("response", result)
                        })
                    }
                }
                if (responses.isNotEmpty()) {
                    EventLogger.log("VoiceSession: sending tool response", level = LogLevel.DEBUG)
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
        _transcripts.tryEmit(VoiceTranscript(role = role, text = text, isPartial = isPartial))
    }

    fun stop() {
        ended = true // a deliberate stop is not an "ended on its own" event
        voiceManager.stopRecording()
        voiceManager.stopPlayback()
        audioOutChannel.close()
        client.disconnect(sendAudioStreamEnd = true)
        scope.cancel()
    }
}
