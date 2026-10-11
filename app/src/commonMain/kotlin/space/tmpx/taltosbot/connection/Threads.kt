package space.tmpx.taltosbot.connection

import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put

/** `Thread` of docs/protocol.md "Threads". Times = epoch ms. */
data class ThreadInfo(val id: String, val name: String, val createdAt: Long, val lastAccessedAt: Long)

/** `Entry.tool`. No [result] = still running (or the app died before the result came). */
data class ToolCall(
    val name: String,
    val args: JsonObject,
    val result: JsonElement? = null,
    val calledAt: Long,
    val resultAt: Long? = null
)

object EntryKind {
    const val USER = "user"
    const val MODEL = "model"
    const val AGENT = "agent"
    const val TOOL = "tool"
    const val MARKER = "marker"
    const val SYSTEM = "system"
}

const val MARKER_VOICE_STARTED = "voice_started"
const val MARKER_VOICE_ENDED = "voice_ended"

/** `Entry` of docs/protocol.md "Threads". [partial] is local only: a voice transcript still growing. */
data class Entry(
    val id: String,
    val ts: Long,
    val kind: String,
    val text: String = "",
    val voice: Boolean = false,
    val project: String? = null,
    val error: Boolean = false,
    val tool: ToolCall? = null,
    val partial: Boolean = false
)

internal fun JsonObject.long(key: String): Long? = (this[key] as? JsonPrimitive)?.longOrNull
internal fun JsonObject.bool(key: String): Boolean = (this[key] as? JsonPrimitive)?.booleanOrNull ?: false

internal fun parseThread(obj: JsonObject?): ThreadInfo? {
    val id = obj?.str("id")?.takeIf { it.isNotBlank() } ?: return null
    val created = obj.long("created_at") ?: 0L
    return ThreadInfo(id, obj.str("name") ?: "", created, obj.long("last_accessed_at") ?: created)
}

internal fun parseEntry(obj: JsonObject): Entry? {
    val id = obj.str("id")?.takeIf { it.isNotBlank() } ?: return null
    val tool = (obj["tool"] as? JsonObject)?.let { t ->
        ToolCall(
            name = t.str("name") ?: "?",
            args = t["args"] as? JsonObject ?: JsonObject(emptyMap()),
            result = t["result"]?.takeIf { it !is kotlinx.serialization.json.JsonNull },
            calledAt = t.long("called_at") ?: 0L,
            resultAt = t.long("result_at")
        )
    }
    return Entry(
        id = id,
        ts = obj.long("ts") ?: 0L,
        kind = obj.str("kind") ?: EntryKind.SYSTEM,
        text = obj.str("text") ?: "",
        voice = obj.bool("voice"),
        project = obj.str("project"),
        error = obj.bool("error"),
        tool = tool
    )
}

internal fun Entry.toJson(): JsonObject = buildJsonObject {
    put("id", id)
    put("ts", ts)
    put("kind", kind)
    put("text", text)
    if (voice) put("voice", true)
    project?.let { put("project", it) }
    if (error) put("error", true)
    tool?.let { t ->
        put("tool", buildJsonObject {
            put("name", t.name)
            put("args", t.args)
            t.result?.let { put("result", it) }
            put("called_at", t.calledAt)
            t.resultAt?.let { put("result_at", it) }
        })
    }
}

/** `Memory` of docs/protocol.md "Thread memory": where the thread stands, made by a server compact. */
data class ThreadMemory(val text: String, val uptoId: String, val at: Long)

/** `Usage`: token counts of a thread, all time. */
data class TokenUse(val input: Long = 0, val output: Long = 0)

/** `threads.get` reply. Destructures like the old `(thread, entries)` pair. */
data class LoadedThread(val thread: ThreadInfo, val entries: List<Entry>, val memory: ThreadMemory?, val usage: TokenUse)

internal fun parseMemory(obj: JsonObject?): ThreadMemory? {
    val text = obj?.str("text")?.takeIf { it.isNotBlank() } ?: return null
    val uptoId = obj.str("upto_id")?.takeIf { it.isNotBlank() } ?: return null
    return ThreadMemory(text, uptoId, obj.long("at") ?: 0L)
}

internal fun parseUsage(obj: JsonObject?): TokenUse = TokenUse(obj?.long("input") ?: 0L, obj?.long("output") ?: 0L)
