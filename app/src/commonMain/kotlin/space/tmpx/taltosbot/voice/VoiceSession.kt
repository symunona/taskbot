package space.tmpx.taltosbot.voice

import space.tmpx.taltosbot.llm.parseCallUsage
import space.tmpx.taltosbot.ui.EventLogger
import space.tmpx.taltosbot.ui.LogLevel
import kotlinx.coroutines.CoroutineExceptionHandler
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlin.concurrent.Volatile
import kotlin.io.encoding.Base64
import kotlin.io.encoding.ExperimentalEncodingApi
import kotlin.time.Duration.Companion.milliseconds
import kotlin.time.TimeMark
import kotlin.time.TimeSource

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
    /** Tool name -> sound played when the call starts. */
    private val toolCues: Map<String, VoiceCue> = emptyMap(),
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

    // Live `toolCallCancellation`: the user talked over a turn, its calls must not run or answer.
    private val toolLock = Mutex()
    private val runningCalls = mutableMapOf<String, Deferred<JsonObject>>()
    /** Cancelled before the call got to run. */
    private val cancelledCalls = mutableSetOf<String>()

    // Relays wait for a quiet moment: a text turn sent mid-speech cuts the speech off.
    private val relayQueue = Channel<String>(capacity = Channel.UNLIMITED)
    /** The model owes speech: it started a turn, or was just handed a tool result or a relay. */
    @Volatile
    private var turnOpen = false
    /** Written under [toolLock]. */
    @Volatile
    private var toolBatchesInFlight = 0
    @Volatile
    private var lastModelActivity: TimeMark? = null
    @Volatile
    private var lastUserSpeech: TimeMark? = null

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

        scope.launch {
            for (text in relayQueue) {
                awaitQuiet()
                try {
                    client.sendText(text)
                    modelOwesSpeech() // the next relay waits for this one to be read out
                } catch (t: Throwable) {
                    EventLogger.log(client.redact("VoiceSession: relay send failed: ${t.message}"), isError = true)
                }
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

    /**
     * Queues a text turn for Gemini to read out (an agent's answer). Announced by a cue at once,
     * handed over when the current speech has finished playing. Several relays keep their order.
     */
    fun relay(text: String) {
        voiceManager.playCue(VoiceCue.AGENT_REPLY)
        relayQueue.trySend(text)
    }

    private fun modelOwesSpeech() {
        turnOpen = true
        lastModelActivity = TimeSource.Monotonic.markNow()
    }

    private fun within(mark: TimeMark?, ms: Long) = mark != null && mark.elapsedNow() < ms.milliseconds

    private fun busy(): Boolean =
        voiceManager.isPlaying.value ||
            toolBatchesInFlight > 0 ||
            within(lastUserSpeech, USER_QUIET_MS) ||
            // A turn with no `turnComplete` must not hold the queue forever.
            (turnOpen && within(lastModelActivity, TURN_STALL_MS))

    /** Returns once nobody has been speaking, and no tool running, for [RELAY_SETTLE_MS]. */
    private suspend fun awaitQuiet() {
        var quietMs = 0L
        while (quietMs < RELAY_SETTLE_MS) {
            delay(RELAY_POLL_MS)
            quietMs = if (busy()) 0L else quietMs + RELAY_POLL_MS
        }
    }

    @OptIn(ExperimentalEncodingApi::class)
    private suspend fun handleIncomingMessage(message: JsonObject) {
        val serverContent = message["serverContent"]?.jsonObject
        val toolCall = message["toolCall"]?.jsonObject
        message["usageMetadata"]?.let { EventLogger.log("GeminiLive: usage $it", level = LogLevel.DEBUG) }
        (message["toolCallCancellation"] as? JsonObject)?.get("ids")?.jsonArray
            ?.mapNotNull { it.jsonPrimitive.contentOrNull }
            ?.let { cancelToolCalls(it) }
        parseCallUsage(message["usageMetadata"] as? JsonObject)?.let {
            tokensIn += it.input
            tokensOut += it.output
            contextTokens = it.input
        }

        if (serverContent != null) {
            if (serverContent["interrupted"] != null) {
                EventLogger.log("GeminiLive: response interrupted")
                voiceManager.stopPlayback()
                turnOpen = false
            }
            if (serverContent["turnComplete"] != null) turnOpen = false
            if (serverContent["inputTranscription"] != null) lastUserSpeech = TimeSource.Monotonic.markNow()

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
            if (parts != null) modelOwesSpeech()
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
            toolLock.withLock { toolBatchesInFlight++ }
            scope.launch {
                try {
                    val responses = buildJsonArray {
                        functionCalls.forEach { functionCallElement ->
                            val functionCall = functionCallElement.jsonObject
                            val name = functionCall["name"]?.jsonPrimitive?.contentOrNull ?: return@forEach
                            val callId = functionCall["id"]?.jsonPrimitive?.contentOrNull ?: return@forEach
                            val args = functionCall["args"]?.jsonObject ?: buildJsonObject {}
                            EventLogger.log("VoiceSession: tool call $name")
                            toolCues[name]?.let(voiceManager::playCue)
                            val result = runCancellable(callId) { executeTool(name, args) }
                            if (result == null) {
                                EventLogger.log("VoiceSession: tool call $name cancelled, no response sent")
                                return@forEach
                            }
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
                        modelOwesSpeech()
                    }
                } finally {
                    withContext(NonCancellable) { toolLock.withLock { toolBatchesInFlight-- } }
                }
            }
        }
    }

    /** Runs one tool call. Null = Gemini cancelled it: no response is owed. */
    private suspend fun runCancellable(callId: String, block: suspend () -> JsonObject): JsonObject? = coroutineScope {
        val call = async(start = CoroutineStart.LAZY) { block() }
        val skip = toolLock.withLock {
            cancelledCalls.remove(callId) || run { runningCalls[callId] = call; false }
        }
        if (skip) {
            call.cancel()
            return@coroutineScope null
        }
        try {
            call.await()
        } catch (e: CancellationException) {
            ensureActive() // the session itself is going down: not a tool cancel
            null
        } finally {
            withContext(NonCancellable) { toolLock.withLock { runningCalls.remove(callId) } }
        }
    }

    private suspend fun cancelToolCalls(ids: List<String>) {
        toolLock.withLock {
            ids.forEach { id -> runningCalls[id]?.cancel() ?: cancelledCalls.add(id) }
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
        relayQueue.close()
        client.disconnect(sendAudioStreamEnd = true)
        scope.cancel()
    }

    private companion object {
        const val RELAY_POLL_MS = 100L
        const val RELAY_SETTLE_MS = 400L
        /** Input transcripts trail the voice; inside this window the user may still be talking. */
        const val USER_QUIET_MS = 2_000L
        const val TURN_STALL_MS = 5_000L
    }
}
