package space.tmpx.taltosbot.connection

import kotlin.coroutines.ContinuationInterceptor
import kotlin.coroutines.CoroutineContext
import space.tmpx.taltosbot.ui.EventLogger
import io.ktor.client.HttpClient
import io.ktor.client.plugins.HttpTimeout
import io.ktor.client.plugins.websocket.DefaultClientWebSocketSession
import io.ktor.client.plugins.websocket.WebSockets
import io.ktor.client.plugins.websocket.webSocketSession
import io.ktor.websocket.Frame
import io.ktor.websocket.close
import io.ktor.websocket.readText
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.cancel
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.datetime.Clock
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.put

enum class ConnState { Disconnected, Connecting, Connected }

sealed class ServerPush {
    /** [id] = envelope id, when the server sent one. */
    data class AgentSay(val project: String, val text: String, val id: String? = null) : ServerPush()
    data class AgentEvent(val project: String, val kind: String, val detail: String) : ServerPush()
    data class ThreadChanged(val thread: ThreadInfo) : ServerPush()
    data class ThreadMemoryChanged(val threadId: String, val memory: ThreadMemory?, val usage: TokenUse) : ServerPush()
    /** `tasks.changed`: state for the UI. Never goes to the voice model. */
    data class TaskChanged(val task: TaskInfo) : ServerPush()
}

data class SessionInfo(val project: String, val status: String, val tmux: String, val url: String?)

class ServerErrorException(message: String) : Exception(message)
private class UnauthorizedException : Exception("Device token rejected")

/**
 * Owns the one WebSocket to the server: probe -> connect -> auth -> serve, reconnecting forever.
 * A rejected token (HTTP 401 or `auth.error`) stops the loop and calls [onUnauthorized].
 */
class ServerConnection(
    private val scope: CoroutineScope,
    private val api: ServerApi,
    private val onUnauthorized: () -> Unit
) {
    private val wsClient = HttpClient {
        install(HttpTimeout) {
            requestTimeoutMillis = HttpTimeout.INFINITE_TIMEOUT_MS
            connectTimeoutMillis = 10_000L
            socketTimeoutMillis = HttpTimeout.INFINITE_TIMEOUT_MS
        }
        install(WebSockets) { pingInterval = 20_000L }
    }
    private val json = Json { ignoreUnknownKeys = true }

    private val _state = MutableStateFlow(ConnState.Disconnected)
    val state: StateFlow<ConnState> = _state

    private val _activeUrl = MutableStateFlow<String?>(null)
    val activeUrl: StateFlow<String?> = _activeUrl

    /** Gemini key + model from `GET /api/v1/config`. Memory only; never logged, never stored. */
    private val _config = MutableStateFlow<GeminiConfig?>(null)
    val config: StateFlow<GeminiConfig?> = _config

    /** Gemini `functionDeclarations`, exactly what `tools.list` returned. */
    private val _tools = MutableStateFlow(JsonArray(emptyList()))
    val tools: StateFlow<JsonArray> = _tools

    private val _pushes = MutableSharedFlow<ServerPush>(extraBufferCapacity = 64)
    val pushes: SharedFlow<ServerPush> = _pushes

    // All socket state is touched only on the owning scope's dispatcher (Main); callers on other
    // dispatchers (the voice session) are hopped over in request().
    private val confined: CoroutineContext = scope.coroutineContext[ContinuationInterceptor] ?: Dispatchers.Main

    private var loopJob: Job? = null
    private var session: DefaultClientWebSocketSession? = null
    private val pending = mutableMapOf<String, CompletableDeferred<JsonObject>>()
    private var nextId = 0

    fun start(pairing: Pairing) {
        val old = loopJob
        loopJob = scope.launch {
            old?.cancelAndJoin() // the old socket is closed before a new one is opened
            runLoop(pairing)
        }
    }

    fun stop() {
        loopJob?.cancel()
        loopJob = null
        _state.value = ConnState.Disconnected
        _activeUrl.value = null
        _config.value = null
        _tools.value = JsonArray(emptyList())
    }

    private suspend fun runLoop(pairing: Pairing) {
        var backoff = BACKOFF_MIN_MS
        try {
            while (true) {
                _state.value = ConnState.Connecting
                val url = probe(pairing)
                if (url != null && runSocket(url, pairing.token)) backoff = BACKOFF_MIN_MS
                _state.value = ConnState.Disconnected
                _activeUrl.value = null
                EventLogger.log("Server: reconnecting in ${backoff / 1000}s")
                delay(backoff)
                backoff = (backoff * 2).coerceAtMost(BACKOFF_MAX_MS)
            }
        } catch (e: UnauthorizedException) {
            EventLogger.log("Server: device token rejected, pairing cleared", isError = true)
            _state.value = ConnState.Disconnected
            _activeUrl.value = null
            onUnauthorized()
        }
    }

    /** First stored URL whose `/api/v1/me` answers within 5 s. */
    private suspend fun probe(pairing: Pairing): String? {
        for (url in pairing.serverUrls) {
            try {
                api.me(url, pairing.token, timeoutMs = 5_000L)
                return url
            } catch (e: CancellationException) {
                throw e
            } catch (e: HttpStatusException) {
                if (e.status == 401) throw UnauthorizedException()
                EventLogger.log("Server: $url answered ${e.status}", isError = true)
            } catch (e: Throwable) {
                EventLogger.log("Server: $url unreachable (${e.message ?: e})", isError = true)
            }
        }
        return null
    }

    /** Runs one socket until it closes. Returns true if it got as far as `auth.ok`. */
    private suspend fun runSocket(url: String, token: String): Boolean {
        val wsUrl = url.replaceFirst("http", "ws") + "/api/v1/ws"
        var authed = false
        val ws = try {
            wsClient.webSocketSession(wsUrl)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Throwable) {
            EventLogger.log("Server: WebSocket connect failed (${e.message ?: e})", isError = true)
            return false
        }
        try {
            ws.send(Frame.Text(buildJsonObject {
                put("event", "auth")
                put("payload", buildJsonObject { put("token", token) })
            }.toString()))

            val first = withTimeoutOrNull(AUTH_TIMEOUT_MS) { nextJson(ws) }
            when (first?.str("event")) {
                "auth.ok" -> Unit
                "auth.error" -> throw UnauthorizedException()
                else -> {
                    EventLogger.log("Server: no auth.ok (got ${first?.str("event")})", isError = true)
                    return false
                }
            }
            authed = true
            session = ws
            _activeUrl.value = url
            _state.value = ConnState.Connected
            EventLogger.log("Server: connected to $url")

            coroutineScope {
                val setup = launch { afterConnect(url, token) }
                try {
                    while (true) {
                        val msg = nextJson(ws) ?: break
                        dispatch(msg)
                    }
                } finally {
                    setup.cancel()
                }
            }
            EventLogger.log("Server: socket closed")
        } catch (e: CancellationException) {
            throw e
        } catch (e: UnauthorizedException) {
            throw e
        } catch (e: Throwable) {
            EventLogger.log("Server: socket error (${e.message ?: e})", isError = true)
        } finally {
            session = null
            val waiting = pending.values.toList()
            pending.clear()
            waiting.forEach { it.completeExceptionally(ServerErrorException("server not connected")) }
            withContext(NonCancellable) {
                withTimeoutOrNull(2_000L) { runCatching { ws.close() } }
            }
            ws.cancel()
        }
        return authed
    }

    /** Next text frame as JSON; null when the socket is closed. Unparseable frames are skipped. */
    private suspend fun nextJson(ws: DefaultClientWebSocketSession): JsonObject? {
        while (true) {
            val frame = ws.incoming.receiveCatching().getOrNull() ?: return null
            if (frame !is Frame.Text) continue
            try {
                return json.parseToJsonElement(frame.readText()).jsonObject
            } catch (e: Exception) {
                EventLogger.log("Server: unparseable frame skipped", isError = true)
            }
        }
    }

    private suspend fun afterConnect(url: String, token: String) {
        try {
            val cfg = api.config(url, token)
            _config.value = cfg
            EventLogger.log("Server: config received (model ${cfg.model.ifBlank { "<none>" }}, key ${if (cfg.apiKey.isBlank()) "missing" else "present"})")
        } catch (e: CancellationException) {
            throw e
        } catch (e: HttpStatusException) {
            if (e.status == 401) {
                // Surface through the socket loop: closing makes runSocket return, the next probe sees the 401.
                EventLogger.log("Server: config returned 401", isError = true)
                runCatching { session?.close() }
                return
            }
            EventLogger.log("Server: config failed (${e.message})", isError = true)
        } catch (e: Throwable) {
            EventLogger.log("Server: config failed (${e.message ?: e})", isError = true)
        }
        try {
            val reply = request("tools.list", buildJsonObject {})
            val list = reply["tools"] as? JsonArray ?: JsonArray(emptyList())
            _tools.value = toFunctionDeclarations(list)
            EventLogger.log("Server: ${_tools.value.size} tools registered")
        } catch (e: CancellationException) {
            throw e
        } catch (e: Throwable) {
            EventLogger.log("Server: tools.list failed (${e.message ?: e})", isError = true)
        }
    }

    private fun toFunctionDeclarations(tools: JsonArray): JsonArray = buildJsonArray {
        for (tool in tools) {
            val obj = tool as? JsonObject ?: continue
            val name = obj.str("name") ?: continue
            add(buildJsonObject {
                put("name", name)
                put("description", obj.str("description") ?: "")
                // Gemini rejects an OBJECT schema with no properties; a no-arg tool simply has no `parameters`.
                val params = obj["parameters"] as? JsonObject
                val props = params?.get("properties") as? JsonObject
                if (params != null && !props.isNullOrEmpty()) put("parameters", params)
            })
        }
    }

    private suspend fun dispatch(msg: JsonObject) {
        val refId = msg.str("ref_id")
        if (refId != null) {
            pending.remove(refId)?.complete(msg)
            return
        }
        val payload = msg["payload"] as? JsonObject ?: JsonObject(emptyMap())
        when (msg.str("event")) {
            "agent.say" -> _pushes.emit(
                ServerPush.AgentSay(payload.str("project") ?: "?", payload.str("text") ?: "", msg.str("id"))
            )
            "agent.event" -> _pushes.emit(
                ServerPush.AgentEvent(payload.str("project") ?: "?", payload.str("kind") ?: "?", payload["detail"].asText())
            )
            "threads.memory" -> payload.str("thread_id")?.let {
                _pushes.emit(ServerPush.ThreadMemoryChanged(it, parseMemory(payload["memory"] as? JsonObject), parseUsage(payload["usage"] as? JsonObject)))
            }
            "threads.changed" -> parseThread(payload["thread"] as? JsonObject)?.let {
                _pushes.emit(ServerPush.ThreadChanged(it))
            }
            "tasks.changed" -> parseTask(payload["task"] as? JsonObject)?.let {
                _pushes.emit(ServerPush.TaskChanged(it))
            }
            else -> EventLogger.log("Server: unhandled event ${msg.str("event")}")
        }
    }

    private fun JsonElement?.asText(): String = when (this) {
        null -> ""
        is JsonPrimitive -> contentOrNull ?: ""
        else -> toString()
    }

    /** Sends a request envelope and waits for the frame whose `ref_id` matches. Returns its payload. */
    suspend fun request(event: String, payload: JsonObject, timeoutMs: Long = REQUEST_TIMEOUT_MS): JsonObject =
        withContext(confined) { requestConfined(event, payload, timeoutMs) }

    private suspend fun requestConfined(event: String, payload: JsonObject, timeoutMs: Long): JsonObject {
        val ws = session ?: throw ServerErrorException("server not connected")
        val id = "c${++nextId}-${Clock.System.now().toEpochMilliseconds()}"
        val waiter = CompletableDeferred<JsonObject>()
        pending[id] = waiter
        try {
            ws.send(Frame.Text(buildJsonObject {
                put("event", event)
                put("id", id)
                put("ts", Clock.System.now().toEpochMilliseconds())
                put("payload", payload)
            }.toString()))
            val reply = withTimeoutOrNull(timeoutMs) { waiter.await() }
                ?: throw ServerErrorException("$event timed out")
            val replyPayload = reply["payload"] as? JsonObject ?: JsonObject(emptyMap())
            if (reply.str("event") == "error") {
                throw ServerErrorException(replyPayload.str("error") ?: "server error")
            }
            return replyPayload
        } finally {
            pending.remove(id)
        }
    }

    /** Every Gemini tool call goes through here. Always returns an object Gemini can take as a response. */
    suspend fun callTool(name: String, args: JsonObject, threadId: String? = null): JsonObject {
        if (session == null) return errorResult("server not connected")
        return try {
            val reply = request("tool.call", buildJsonObject {
                put("name", name)
                put("args", args)
                threadId?.let { put("thread_id", it) }
            })
            when (val result = reply["result"]) {
                is JsonObject -> result
                null -> reply
                else -> buildJsonObject { put("result", result) }
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Throwable) {
            errorResult(e.message ?: "tool call failed")
        }
    }

    suspend fun listSessions(): List<SessionInfo> {
        val reply = request("sessions.list", buildJsonObject {})
        return (reply["sessions"] as? JsonArray).orEmpty().mapNotNull { it as? JsonObject }.map {
            SessionInfo(
                project = it.str("project") ?: "?",
                status = it.str("status") ?: "?",
                tmux = it.str("tmux") ?: "",
                url = it.str("url")?.takeIf { u -> u.isNotBlank() }
            )
        }
    }

    /** Initial sync of delegated tasks, newest first. Changes after that arrive as `tasks.changed`. */
    suspend fun listTasks(): List<TaskInfo> {
        val reply = request("tasks.list", buildJsonObject {})
        return (reply["tasks"] as? JsonArray).orEmpty().mapNotNull { parseTask(it as? JsonObject) }
    }

    // ---- threads (docs/protocol.md "Threads") ----

    suspend fun listThreads(): List<ThreadInfo> {
        val reply = request("threads.list", buildJsonObject {})
        return (reply["threads"] as? JsonArray).orEmpty().mapNotNull { parseThread(it as? JsonObject) }
    }

    suspend fun createThread(name: String? = null): ThreadInfo {
        val reply = request("threads.create", buildJsonObject { name?.let { put("name", it) } })
        return parseThread(reply["thread"] as? JsonObject) ?: throw ServerErrorException("threads.create: no thread in reply")
    }

    /** Thread + its entries in `seq` order. */
    suspend fun getThread(id: String): LoadedThread {
        val reply = request("threads.get", buildJsonObject { put("id", id) })
        val thread = parseThread(reply["thread"] as? JsonObject) ?: throw ServerErrorException("threads.get: no thread in reply")
        val entries = (reply["entries"] as? JsonArray).orEmpty().mapNotNull { (it as? JsonObject)?.let(::parseEntry) }
        return LoadedThread(thread, entries, parseMemory(reply["memory"] as? JsonObject), parseUsage(reply["usage"] as? JsonObject))
    }

    /** Compact now. Waits for the server's Gemini call, so it gets a longer timeout than other requests. */
    suspend fun compactThread(id: String): Pair<ThreadMemory?, TokenUse> {
        val reply = request("threads.compact", buildJsonObject { put("id", id) }, COMPACT_TIMEOUT_MS)
        return parseMemory(reply["memory"] as? JsonObject) to parseUsage(reply["usage"] as? JsonObject)
    }

    /** Adds token counts to the thread's all-time use; returns the new totals. */
    suspend fun reportUsage(id: String, input: Long, output: Long): TokenUse {
        val reply = request("threads.usage", buildJsonObject {
            put("id", id)
            put("input", input)
            put("output", output)
        })
        return parseUsage(reply["usage"] as? JsonObject)
    }

    suspend fun renameThread(id: String, name: String): ThreadInfo {
        val reply = request("threads.rename", buildJsonObject {
            put("id", id)
            put("name", name)
        })
        return parseThread(reply["thread"] as? JsonObject) ?: throw ServerErrorException("threads.rename: no thread in reply")
    }

    /** Upsert by entry `id`. Caller keeps a batch within the protocol limits (200 entries, 1 MiB frame). */
    suspend fun appendEntries(id: String, entries: List<Entry>) {
        request("threads.append", buildJsonObject {
            put("id", id)
            put("entries", buildJsonArray { entries.forEach { add(it.toJson()) } })
        })
    }

    private fun errorResult(message: String) = buildJsonObject { put("error", message) }

    private companion object {
        const val BACKOFF_MIN_MS = 1_000L
        const val BACKOFF_MAX_MS = 30_000L
        const val AUTH_TIMEOUT_MS = 10_000L
        const val REQUEST_TIMEOUT_MS = 30_000L
        const val COMPACT_TIMEOUT_MS = 100_000L
    }
}
