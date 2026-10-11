package space.tmpx.taltosbot.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import space.tmpx.taltosbot.connection.ConnState
import space.tmpx.taltosbot.connection.Entry
import space.tmpx.taltosbot.connection.EntryKind
import space.tmpx.taltosbot.connection.MARKER_VOICE_ENDED
import space.tmpx.taltosbot.connection.MARKER_VOICE_STARTED
import space.tmpx.taltosbot.connection.PairingStore
import space.tmpx.taltosbot.connection.ServerApi
import space.tmpx.taltosbot.connection.ServerConnection
import space.tmpx.taltosbot.connection.ServerPush
import space.tmpx.taltosbot.connection.SessionInfo
import space.tmpx.taltosbot.connection.TaskInfo
import space.tmpx.taltosbot.connection.ThreadInfo
import space.tmpx.taltosbot.connection.ThreadMemory
import space.tmpx.taltosbot.connection.TokenUse
import space.tmpx.taltosbot.connection.ToolCall
import space.tmpx.taltosbot.connection.parseConnectString
import space.tmpx.taltosbot.connection.toJson
import space.tmpx.taltosbot.connection.upsertTask
import space.tmpx.taltosbot.llm.GEMINI_BASE_URL
import space.tmpx.taltosbot.llm.GeminiTextClient
import space.tmpx.taltosbot.llm.estimateTokens
import space.tmpx.taltosbot.llm.historyChars
import space.tmpx.taltosbot.llm.runTextTurn
import space.tmpx.taltosbot.llm.textHistory
import space.tmpx.taltosbot.llm.textSystemInstruction
import space.tmpx.taltosbot.llm.voiceSystemInstruction
import space.tmpx.taltosbot.update.UpdateInfo
import space.tmpx.taltosbot.update.checkForUpdate
import space.tmpx.taltosbot.voice.VoiceSession
import space.tmpx.taltosbot.voice.VoiceTranscript
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.job
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.datetime.Clock
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlin.random.Random

const val SYSTEM_INSTRUCTION =
    "You are Taltosbot, a voice front end. You have read-only access to the user's notes vault " +
        "through your tools. You cannot write, save, build or do longer work yourself: for any of " +
        "that, call `delegate` with the right project and a clear prompt. `delegate` returns " +
        "immediately; the project agent's answer arrives later as a message for you to relay. " +
        "Speak caveman brief, except when asked to elaborate. Single word confirms, yes/no " +
        "answers when possible. " +
        "Paths, file and folder names: say only the relevant part, usually the last name, no " +
        "extension. Skip Johnny Decimal numbers: say \"Taxes\", not \"12.03 Taxes\" or " +
        "\"10-19 Finance\". Never read out server directories or full paths unless asked. " +
        "In tool calls use the exact path, numbers included. " +
        "Once the topic is clear, name the thread with `rename_thread` (a few words), without mentioning it."

enum class VoiceState { Idle, Connecting, Active }

/**
 * All app state and behaviour, outside the composables. One per process, so an activity
 * recreation does not drop the server socket or a running voice session.
 * Everything here runs on the main dispatcher.
 */
class AppController {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val api = ServerApi()
    private val storage = getStorage()
    private val store = PairingStore(storage)

    val connection = ServerConnection(scope, api, onUnauthorized = {
        clearPairing()
        pairingError = "This device is no longer paired. Pair again."
    })

    var pairing by mutableStateOf(store.load())
        private set
    var pairingBusy by mutableStateOf(false)
        private set
    var pairingError by mutableStateOf<String?>(null)
        private set

    /** Thread list, newest `lastAccessedAt` first. */
    var threads by mutableStateOf(listOf<ThreadInfo>())
        private set
    var threadsError by mutableStateOf<String?>(null)
        private set
    /** Null until the first message / voice start creates one, or the last one is reopened. */
    var currentThread by mutableStateOf<ThreadInfo?>(null)
        private set
    var threadLoading by mutableStateOf(false)
        private set

    /** Entries of [currentThread], in order. */
    var entries by mutableStateOf(listOf<Entry>())
        private set
    /** Thread memory of [currentThread]: what the model gets instead of the folded entries. Null = never compacted. */
    var memory by mutableStateOf<ThreadMemory?>(null)
        private set
    /** Token use of [currentThread], all time, as the server counts it. */
    var usage by mutableStateOf(TokenUse())
        private set
    /** Prompt tokens of the newest model call on [currentThread], as Gemini reported them. Null = none yet on this device. */
    var contextTokens by mutableStateOf<Long?>(null)
        private set
    var compacting by mutableStateOf(false)
        private set
    var compactError by mutableStateOf<String?>(null)
        private set
    /** Ids of tool entries whose call is in flight on this device. */
    var runningTools by mutableStateOf(setOf<String>())
        private set
    /** A text turn is waiting for Gemini. */
    var textBusy by mutableStateOf(false)
        private set

    var sessions by mutableStateOf(listOf<SessionInfo>())
        private set
    /** Delegated tasks, newest first. Synced on connect, then kept current by `tasks.changed`. */
    var tasks by mutableStateOf(listOf<TaskInfo>())
        private set
    var sessionsLoading by mutableStateOf(false)
        private set
    var sessionsError by mutableStateOf<String?>(null)
        private set

    var voiceState by mutableStateOf(VoiceState.Idle)
        private set
    private var voiceSession: VoiceSession? = null
    private var voiceJob: Job? = null
    /** Thread that got the `voice_started` marker and still owes a `voice_ended`. */
    private var voiceMarkedThread: String? = null

    // Test seam: e2e points text mode at the mock server through this storage key. Unset = Google.
    private val textClient = GeminiTextClient(storage.getString(KEY_TEXT_BASE_URL)?.takeIf { it.isNotBlank() } ?: GEMINI_BASE_URL)
    private var textJob: Job? = null

    private val threadMutex = Mutex()
    private var loadJob: Job? = null

    // Entries not yet stored on the server: thread id -> entry id -> latest version.
    private val dirty = LinkedHashMap<String, LinkedHashMap<String, Entry>>()
    private var flushJob: Job? = null
    private var idCounter = 0

    var update by mutableStateOf<UpdateInfo?>(null)
        private set
    var updateStatus by mutableStateOf<String?>(null)
        private set

    init {
        scope.launch { connection.pushes.collect(::onPush) }
        scope.launch { connection.state.collect { if (it == ConnState.Connected) onConnected() } }
        pairing?.let(connection::start)
        if (platformUpdateCheckSupported) checkUpdate()
    }

    // ---- pairing ----

    fun submitConnectString(raw: String) {
        if (pairingBusy) return
        val connect = try {
            parseConnectString(raw)
        } catch (e: IllegalArgumentException) {
            pairingError = e.message
            return
        }
        pairingBusy = true
        pairingError = null
        scope.launch {
            try {
                val paired = api.pair(connect, platformDeviceName)
                store.save(paired)
                pairing = paired
                EventLogger.log("Paired as device ${paired.deviceId}")
                connection.start(paired)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Throwable) {
                pairingError = "Pairing failed: ${e.message ?: e}"
                EventLogger.log("Pairing failed: ${e.message ?: e}", isError = true)
            } finally {
                pairingBusy = false
            }
        }
    }

    fun unpair() {
        clearPairing()
        pairingError = null
    }

    private fun clearPairing() {
        stopTurns()
        loadJob?.cancel()
        flushJob?.cancel()
        connection.stop()
        store.clear()
        storage.remove(KEY_LAST_THREAD)
        pairing = null
        sessions = emptyList()
        tasks = emptyList()
        threads = emptyList()
        threadsError = null
        currentThread = null
        threadLoading = false
        entries = emptyList()
        resetMemory()
        dirty.clear()
    }

    // ---- server pushes ----

    private fun onConnected() {
        refreshThreads()
        refreshTasks()
        // Reopen the last thread, unless something is already on screen.
        if (currentThread == null && entries.isEmpty()) {
            storage.getString(KEY_LAST_THREAD)?.takeIf { it.isNotBlank() }?.let(::selectThread)
        }
        scheduleFlush(0L)
    }

    private fun onPush(push: ServerPush) {
        when (push) {
            is ServerPush.AgentSay -> {
                // Entry id from the envelope id: the same push stored by two devices lands once.
                putEntry(Entry(push.id?.let { "a-$it" } ?: newId("a"), now(), EntryKind.AGENT, push.text, project = push.project))
                // Live voice: Gemini reads it out. Otherwise the bubble above is all there is.
                if (voiceState == VoiceState.Active) {
                    voiceSession?.sendText(
                        "[This is not the user speaking. It is a message from the agent working on project " +
                            "\"${push.project}\", answering an earlier delegated request. Relay it to the user " +
                            "now, briefly, in the language you are speaking with them.]\n\n${push.text}"
                    )
                }
            }
            // `activity` repeats every few seconds while an agent works; the task strip shows it.
            is ServerPush.AgentEvent -> if (push.kind != "activity") {
                EventLogger.log(
                    "agent[${push.project}] ${push.kind}" + if (push.detail.isBlank()) "" else ": ${push.detail}",
                    isError = push.kind == "error"
                )
                refreshSessions()
            }
            is ServerPush.ThreadChanged -> upsertThread(push.thread)
            is ServerPush.ThreadMemoryChanged -> if (push.threadId == currentThread?.id) {
                memory = push.memory ?: memory
                usage = push.usage
            }
            // UI state only. Deliberately not sent to Gemini: a text turn would cut into live speech.
            is ServerPush.TaskChanged -> tasks = upsertTask(tasks, push.task)
        }
    }

    // ---- entries ----

    private fun now() = Clock.System.now().toEpochMilliseconds()

    /** Client-made entry id, unique in the thread across devices. */
    private fun newId(prefix: String) = "$prefix${now().toString(36)}-${++idCounter}-${Random.nextInt().toUInt().toString(36)}"

    /**
     * Adds or replaces (same `id`) an entry of [threadId]. Shown when that thread is on screen,
     * and queued for `threads.append`. No thread yet: local only, until one is created.
     */
    private fun putEntry(entry: Entry, threadId: String? = currentThread?.id) {
        if (threadId == currentThread?.id) {
            val index = entries.indexOfLast { it.id == entry.id }
            entries = if (index >= 0) entries.toMutableList().also { it[index] = entry } else entries + entry
        }
        if (threadId != null) {
            dirty.getOrPut(threadId) { LinkedHashMap() }[entry.id] = entry
            scheduleFlush()
        }
    }

    private fun systemNote(text: String, isError: Boolean = false, threadId: String? = currentThread?.id) =
        putEntry(Entry(newId("s"), now(), EntryKind.SYSTEM, text, error = isError), threadId)

    /** Debounced: partial transcripts rewrite one entry many times a second. */
    private fun scheduleFlush(delayMs: Long = FLUSH_DELAY_MS) {
        if (flushJob?.isActive == true) return
        flushJob = scope.launch {
            delay(delayMs)
            val retry = !flush()
            flushJob = null
            if (retry && connection.state.value == ConnState.Connected) scheduleFlush(FLUSH_RETRY_MS)
        }
    }

    /** Sends everything queued. False = something is still queued (disconnected, or a send failed). */
    private suspend fun flush(): Boolean {
        for (threadId in dirty.keys.toList()) {
            val queued = dirty.remove(threadId)?.values?.toList() ?: continue
            var sent = 0
            try {
                for (batch in batches(queued)) {
                    connection.appendEntries(threadId, batch)
                    sent += batch.size
                }
            } catch (e: CancellationException) {
                requeue(threadId, queued.drop(sent))
                throw e
            } catch (e: Throwable) {
                val message = e.message ?: e.toString()
                if (connection.state.value != ConnState.Connected || "timed out" in message || "not connected" in message) {
                    requeue(threadId, queued.drop(sent))
                    return false
                }
                // The server refused it (unknown thread, over a limit). Retrying cannot help.
                EventLogger.log("Threads: append to $threadId refused, ${queued.size - sent} entries dropped ($message)", isError = true)
            }
        }
        return dirty.isEmpty()
    }

    /** Puts unsent entries back in front, keeping newer versions queued meanwhile. */
    private fun requeue(threadId: String, unsent: List<Entry>) {
        val merged = LinkedHashMap<String, Entry>()
        unsent.forEach { merged[it.id] = it }
        dirty[threadId]?.forEach { (id, entry) -> merged[id] = entry }
        dirty[threadId] = merged
    }

    /** Protocol limits: 200 entries per `threads.append`, 1 MiB per frame. */
    private fun batches(entries: List<Entry>): List<List<Entry>> {
        val result = mutableListOf<List<Entry>>()
        var batch = mutableListOf<Entry>()
        var bytes = 0
        for (entry in entries) {
            val size = entry.toJson().toString().encodeToByteArray().size
            if (batch.isNotEmpty() && (batch.size >= APPEND_MAX_ENTRIES || bytes + size > APPEND_MAX_BYTES)) {
                result += batch
                batch = mutableListOf()
                bytes = 0
            }
            batch += entry
            bytes += size
        }
        if (batch.isNotEmpty()) result += batch
        return result
    }

    // ---- tasks ----

    /** Full list after every (re)connect: pushes sent while offline are gone. */
    private fun refreshTasks() {
        scope.launch {
            try {
                // Merge, not replace: a push may have landed while the list was on its way.
                tasks = connection.listTasks().fold(tasks, ::upsertTask)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Throwable) {
                EventLogger.log("Server: tasks.list failed (${e.message ?: e})", isError = true)
            }
        }
    }

    // ---- threads ----

    fun refreshThreads() {
        if (connection.state.value != ConnState.Connected) return
        scope.launch {
            try {
                threads = connection.listThreads().sortedByDescending { it.lastAccessedAt }
                threads.firstOrNull { it.id == currentThread?.id }?.let { currentThread = it }
                threadsError = null
            } catch (e: CancellationException) {
                throw e
            } catch (e: Throwable) {
                threadsError = e.message ?: e.toString()
            }
        }
    }

    private fun upsertThread(thread: ThreadInfo) {
        threads = (threads.filter { it.id != thread.id } + thread).sortedByDescending { it.lastAccessedAt }
        if (currentThread?.id == thread.id) currentThread = thread
    }

    /** Makes [thread] the one on screen. */
    private fun showThread(thread: ThreadInfo, loaded: List<Entry>, loadedMemory: ThreadMemory? = null, loadedUsage: TokenUse = TokenUse()) {
        resetMemory()
        memory = loadedMemory
        usage = loadedUsage
        // Entries of this thread still waiting to be stored win over what the server has.
        val pending = dirty[thread.id].orEmpty()
        val known = loaded.distinctBy { it.id }.map { pending[it.id] ?: it }
        val ids = known.mapTo(HashSet()) { it.id }
        currentThread = thread
        entries = known + pending.values.filter { it.id !in ids }
        storage.setString(KEY_LAST_THREAD, thread.id)
        upsertThread(thread)
    }

    /** The current thread; none yet -> create one. Entries shown so far move into it. */
    private suspend fun ensureThread(): ThreadInfo = threadMutex.withLock {
        currentThread ?: connection.createThread().also { thread ->
            val local = entries
            showThread(thread, emptyList())
            local.forEach { putEntry(it, thread.id) }
        }
    }

    fun newThread() {
        if (!requireConnected()) return
        stopTurns()
        loadJob?.cancel()
        loadJob = scope.launch {
            try {
                showThread(connection.createThread(), emptyList())
                threadsError = null
            } catch (e: CancellationException) {
                throw e
            } catch (e: Throwable) {
                threadsError = "New thread failed: ${e.message ?: e}"
            }
        }
    }

    fun selectThread(id: String) {
        if (id == currentThread?.id) return
        if (!requireConnected()) return
        stopTurns()
        loadJob?.cancel()
        threadLoading = true
        loadJob = scope.launch {
            try {
                val got = connection.getThread(id)
                showThread(got.thread, got.entries, got.memory, got.usage)
                threadsError = null
            } catch (e: CancellationException) {
                throw e
            } catch (e: Throwable) {
                threadsError = "Thread did not open: ${e.message ?: e}"
                // The remembered thread is gone on the server: stop reopening it.
                if (connection.state.value == ConnState.Connected && storage.getString(KEY_LAST_THREAD) == id) {
                    storage.remove(KEY_LAST_THREAD)
                }
            } finally {
                if (loadJob === coroutineContext.job) threadLoading = false
            }
        }
    }

    fun renameThread(id: String, name: String) {
        val trimmed = name.trim()
        if (trimmed.isEmpty() || !requireConnected()) return
        scope.launch {
            try {
                upsertThread(connection.renameThread(id, trimmed))
                threadsError = null
            } catch (e: CancellationException) {
                throw e
            } catch (e: Throwable) {
                threadsError = "Rename failed: ${e.message ?: e}"
            }
        }
    }

    // ---- thread memory ----

    private fun resetMemory() {
        memory = null
        usage = TokenUse()
        contextTokens = null
        compacting = false
        compactError = null
    }

    /** User turns not folded into the memory yet. */
    val turnsSinceCompact: Int
        get() {
            val at = memory?.let { m -> entries.indexOfLast { it.id == m.uptoId } } ?: -1
            return entries.drop(at + 1).count { it.kind == EntryKind.USER && it.text.isNotBlank() }
        }

    /** Tokens of history + memory a text turn would send now. A guess: chars / 4, tool declarations not counted. */
    val contextEstimate: Long
        get() = estimateTokens(
            textSystemInstruction(SYSTEM_INSTRUCTION, entries, memory).length + historyChars(textHistory(entries, memory))
        )

    /** Memory + token use as the server has them now: a `threads.memory` push may have been missed while offline. */
    fun refreshMemory() {
        val threadId = currentThread?.id ?: return
        if (connection.state.value != ConnState.Connected) return
        scope.launch {
            try {
                val got = connection.getThread(threadId)
                if (currentThread?.id == threadId) {
                    memory = got.memory
                    usage = got.usage
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Throwable) {
                EventLogger.log("Threads: memory refresh failed (${e.message ?: e})", isError = true)
            }
        }
    }

    /** The COMPACT button: fold what is foldable into the memory now. */
    fun compact() {
        val threadId = currentThread?.id ?: return
        if (compacting) return
        if (connection.state.value != ConnState.Connected) {
            compactError = "Server not connected"
            return
        }
        compacting = true
        compactError = null
        scope.launch {
            try {
                // The server folds what it has stored: send the queued entries first.
                flush()
                val (newMemory, newUsage) = connection.compactThread(threadId)
                if (currentThread?.id == threadId) {
                    memory = newMemory ?: memory
                    usage = newUsage
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Throwable) {
                if (currentThread?.id == threadId) compactError = "Compact failed: ${e.message ?: e}"
            } finally {
                if (currentThread?.id == threadId) compacting = false
            }
        }
    }

    /** Adds one turn's / one voice session's token counts to the thread's all-time use on the server. */
    private fun reportUsage(threadId: String, input: Long, output: Long) {
        if (input <= 0 && output <= 0) return
        scope.launch {
            try {
                val total = connection.reportUsage(threadId, input, output)
                if (currentThread?.id == threadId) usage = total
            } catch (e: CancellationException) {
                throw e
            } catch (e: Throwable) {
                EventLogger.log("Threads: token use not stored (${e.message ?: e})", isError = true)
            }
        }
    }

    private fun requireConnected(): Boolean {
        val ok = connection.state.value == ConnState.Connected
        threadsError = if (ok) threadsError else "Server not connected"
        return ok
    }

    // ---- sessions ----

    fun refreshSessions() {
        if (connection.state.value != ConnState.Connected) {
            sessionsError = "Server not connected"
            return
        }
        sessionsLoading = true
        scope.launch {
            try {
                sessions = connection.listSessions()
                sessionsError = null
            } catch (e: CancellationException) {
                throw e
            } catch (e: Throwable) {
                sessionsError = e.message ?: e.toString()
            } finally {
                sessionsLoading = false
            }
        }
    }

    // ---- tools ----

    /**
     * Every tool call, voice or text: one `tool` entry, replaced in place when the result is in.
     * Voice calls this from a background dispatcher.
     */
    private suspend fun runTool(threadId: String, name: String, args: JsonObject): JsonObject = withContext(Dispatchers.Main) {
        val id = newId("t")
        val call = ToolCall(name, args, calledAt = now())
        fun finish(result: JsonObject) =
            putEntry(Entry(id, call.calledAt, EntryKind.TOOL, tool = call.copy(result = storedResult(result), resultAt = now())), threadId)

        putEntry(Entry(id, call.calledAt, EntryKind.TOOL, tool = call), threadId)
        runningTools = runningTools + id
        try {
            connection.callTool(name, args, threadId).also(::finish)
        } catch (e: CancellationException) {
            finish(buildJsonObject { put("error", "cancelled") })
            throw e
        } finally {
            runningTools = runningTools - id
        }
    }

    /** The model gets the full result; the stored entry gets a cut one, so a big file cannot blow the frame cap. */
    private fun storedResult(result: JsonObject): JsonElement {
        val text = result.toString()
        if (text.length <= TOOL_RESULT_STORE_MAX) return result
        return buildJsonObject {
            put("truncated", true)
            put("chars", text.length)
            put("preview", text.take(TOOL_RESULT_STORE_MAX))
        }
    }

    // ---- text mode ----

    /**
     * Send button. Voice running: the text goes into the voice session as a turn. Otherwise a
     * text turn against Gemini `generateContent`. False = not sent, keep the draft.
     */
    fun sendMessage(raw: String): Boolean {
        val text = raw.trim()
        if (text.isEmpty() || voiceState == VoiceState.Connecting || textBusy) return false
        val session = voiceSession
        if (voiceState == VoiceState.Active && session != null) {
            session.sendText(text)
            putEntry(Entry(newId("u"), now(), EntryKind.USER, text))
            return true
        }
        val config = connection.config.value
        if (connection.state.value != ConnState.Connected || config == null) {
            systemNote("Not sent: server not connected.", isError = true)
            return false
        }
        if (config.apiKey.isBlank() || config.textModel.isBlank()) {
            systemNote("Not sent: no Gemini key / text model from the server.", isError = true)
            return false
        }
        textBusy = true
        textJob = scope.launch {
            var threadId: String? = null
            var tokensIn = 0L
            var tokensOut = 0L
            try {
                val thread = ensureThread()
                threadId = thread.id
                putEntry(Entry(newId("u"), now(), EntryKind.USER, text), thread.id)
                val history = textHistory(entries, memory)
                val instruction = textSystemInstruction(SYSTEM_INSTRUCTION, entries, memory)
                EventLogger.log(
                    "Context: text turn sends ${historyChars(history)} chars of history in ${history.size} turns, " +
                        "${instruction.length - SYSTEM_INSTRUCTION.length} chars of memory (${entries.size} entries in thread)"
                )
                val answer = runTextTurn(
                    client = textClient,
                    apiKey = config.apiKey,
                    model = config.textModel,
                    systemInstruction = instruction,
                    tools = connection.tools.value,
                    history = history,
                    onUsage = { used ->
                        tokensIn += used.input
                        tokensOut += used.output
                        if (currentThread?.id == thread.id) contextTokens = used.input
                    },
                    executeTool = { name, args -> runTool(thread.id, name, args) }
                )
                putEntry(Entry(newId("m"), now(), EntryKind.MODEL, answer), thread.id)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Throwable) {
                val detail = (e.message ?: e.toString()).replace(config.apiKey, "***")
                EventLogger.log("TextMode: turn failed: $detail", isError = true)
                systemNote(detail, isError = true, threadId = threadId ?: currentThread?.id)
            } finally {
                threadId?.let { reportUsage(it, tokensIn, tokensOut) }
                if (textJob === coroutineContext.job) textBusy = false
            }
        }
        return true
    }

    // ---- voice ----

    fun toggleVoice() {
        when (voiceState) {
            VoiceState.Idle -> startVoice()
            else -> stopVoice(note = null)
        }
    }

    private fun startVoice() {
        if (!platformVoiceSupported || voiceState != VoiceState.Idle) return
        val config = connection.config.value
        if (config == null || config.apiKey.isBlank() || config.model.isBlank()) {
            systemNote("No Gemini key/model from the server yet. Connect to the server first.", isError = true)
            return
        }
        textJob?.cancel()
        textBusy = false
        voiceState = VoiceState.Connecting
        voiceJob = scope.launch {
            val thread = try {
                ensureThread()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Throwable) {
                stopVoice("Voice not started: ${e.message ?: e}", isError = true)
                return@launch
            }
            val instruction = voiceSystemInstruction(SYSTEM_INSTRUCTION, entries, memory)
            EventLogger.log(
                "Context: voice start sends ${instruction.length - SYSTEM_INSTRUCTION.length} chars of memory + history " +
                    "(${entries.size} entries in thread)"
            )
            lateinit var session: VoiceSession
            session = VoiceSession(
                apiKey = config.apiKey,
                model = config.model,
                // What was typed or said before on this thread rides along: memory + transcript block.
                systemInstruction = instruction,
                tools = connection.tools.value,
                executeTool = { name, args -> runTool(thread.id, name, args) },
                onEnded = { reason ->
                    // Called from a background dispatcher.
                    scope.launch { if (voiceSession === session) stopVoice(reason, isError = true) }
                }
            )
            voiceSession = session
            launch {
                session.transcripts.collect { transcript ->
                    if (currentThread?.id == thread.id) {
                        mergeVoiceTranscript(entries.lastOrNull(), transcript, newId("v"), now())?.let { putEntry(it, thread.id) }
                    }
                }
            }
            try {
                session.start()
                if (voiceSession === session) {
                    voiceState = VoiceState.Active
                    voiceMarkedThread = thread.id
                    putEntry(Entry(newId("k"), now(), EntryKind.MARKER, MARKER_VOICE_STARTED), thread.id)
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Throwable) {
                val detail = (e.message ?: e.toString()).replace(config.apiKey, "***")
                EventLogger.log("VoiceSession: failed to start: $detail", isError = true)
                if (voiceSession === session) stopVoice("Voice session failed: $detail", isError = true)
            }
        }
    }

    private fun stopVoice(note: String?, isError: Boolean = false) {
        val session = voiceSession
        voiceSession = null
        voiceJob?.cancel()
        voiceJob = null
        try {
            session?.stop()
        } catch (e: Throwable) {
            EventLogger.log("VoiceSession: stop failed: ${e.message ?: e}", isError = true)
        }
        voiceState = VoiceState.Idle
        val marked = voiceMarkedThread
        voiceMarkedThread = null
        if (session != null && marked != null) {
            if (currentThread?.id == marked) session.contextTokens?.let { contextTokens = it }
            reportUsage(marked, session.tokensIn, session.tokensOut)
        }
        if (note != null) systemNote(note, isError, threadId = marked ?: currentThread?.id)
        if (marked != null) putEntry(Entry(newId("k"), now(), EntryKind.MARKER, MARKER_VOICE_ENDED), marked)
    }

    /** Ends whatever is talking to Gemini: before the thread on screen changes, or on unpair. */
    private fun stopTurns() {
        stopVoice(note = null)
        textJob?.cancel()
        textJob = null
        textBusy = false
    }

    // ---- update check (Android) ----

    fun checkUpdate() {
        if (!platformUpdateCheckSupported) return
        updateStatus = "Checking..."
        scope.launch {
            try {
                update = checkForUpdate(api)
                updateStatus = if (update == null) "Up to date" else "Update available: ${update?.version}"
            } catch (e: CancellationException) {
                throw e
            } catch (e: Throwable) {
                updateStatus = "Update check failed: ${e.message ?: e}"
                EventLogger.log("Update check failed: ${e.message ?: e}", isError = true)
            }
        }
    }

    companion object {
        private const val KEY_LAST_THREAD = "last_thread_id"
        private const val KEY_TEXT_BASE_URL = "gemini_text_base_url"
        private const val FLUSH_DELAY_MS = 600L
        private const val FLUSH_RETRY_MS = 5_000L
        private const val APPEND_MAX_ENTRIES = 200
        private const val APPEND_MAX_BYTES = 600_000
        private const val TOOL_RESULT_STORE_MAX = 60_000
        val instance: AppController by lazy { AppController() }
    }
}

/**
 * Where a voice transcript goes: the grown/extended last entry (same `id`, so the stored entry
 * is replaced), a new entry, or null when there is nothing new.
 */
internal fun mergeVoiceTranscript(last: Entry?, transcript: VoiceTranscript, newId: String, now: Long): Entry? {
    val normalizedText = transcript.text.trim()
    if (normalizedText.isBlank()) return null

    // Always merge consecutive voice transcripts from the same role into one bubble
    if (last != null && last.voice && last.kind == transcript.role) {
        val newContent = when {
            // Partial update: new text extends existing (cumulative transcription)
            transcript.isPartial || normalizedText.startsWith(last.text) -> normalizedText
            last.partial || last.text.startsWith(normalizedText) -> normalizedText
            // New segment from same role: append
            else -> "${last.text} $normalizedText"
        }
        if (newContent == last.text) return null
        return last.copy(text = newContent, partial = transcript.isPartial)
    }

    return Entry(newId, now, transcript.role, normalizedText, voice = true, partial = transcript.isPartial)
}
