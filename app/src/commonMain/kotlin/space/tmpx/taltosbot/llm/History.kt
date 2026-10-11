package space.tmpx.taltosbot.llm

import space.tmpx.taltosbot.connection.Entry
import space.tmpx.taltosbot.connection.EntryKind
import space.tmpx.taltosbot.connection.ThreadMemory
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

// Thread -> model context. Text and voice read the same thread, so both directions
// (text after voice, voice after text) see one conversation.
//
// Context = thread memory (docs/protocol.md "Thread memory") + the entries after its fold point.
// No usable memory: the plain tail window. The window caps below stay on either way, as the
// safety net for a thread whose compact keeps failing.

private const val VOICE_MAX_ENTRIES = 40
private const val VOICE_MAX_CHARS = 12_000
private const val TEXT_MAX_ENTRIES = 80
private const val TEXT_MAX_CHARS = 40_000
private const val ENTRY_MAX_CHARS = 2_000
private const val TOOL_ARGS_MAX_CHARS = 120
private const val TOOL_RESULT_MAX_CHARS = 1_500
/** A tool result the model has already answered from. */
private const val TOOL_RESULT_OLD_MAX_CHARS = 200

private fun String.clip(max: Int) = if (length <= max) this else take(max) + "…"

/** Tool entry as one short line: which tool, with what. */
private fun toolLine(entry: Entry): String? {
    val tool = entry.tool ?: return null
    val args = if (tool.args.isEmpty()) "" else tool.args.toString().clip(TOOL_ARGS_MAX_CHARS)
    return "[tool ${tool.name}($args)]"
}

/** Markers and system notes are UI only; they are not part of what was said. */
private fun spoken(entries: List<Entry>) = entries.filter {
    when (it.kind) {
        EntryKind.USER, EntryKind.MODEL, EntryKind.AGENT -> it.text.isNotBlank()
        EntryKind.TOOL -> it.tool != null
        else -> false
    }
}

/**
 * What the model still needs verbatim: the entries after the memory's fold point, and the memory
 * if it applies. A fold point that is not among [entries] (not loaded, thread switched) makes the
 * memory unusable: all entries, no memory.
 */
private fun unfolded(entries: List<Entry>, memory: ThreadMemory?): Pair<List<Entry>, ThreadMemory?> {
    if (memory == null) return entries to null
    val at = entries.indexOfLast { it.id == memory.uptoId }
    return if (at < 0) entries to null else entries.drop(at + 1) to memory
}

private fun memoryBlock(memory: ThreadMemory) =
    "Memory of this conversation so far: where things stand. Treat it as known; the user sees the " +
        "same text. `State` = what was agreed, `Recent` = the latest turns in short.\n" +
        "<memory>\n" + memory.text + "\n</memory>"

/** Newest lines that fit [maxChars], oldest first. */
private fun <T> tailWithin(items: List<T>, maxChars: Int, size: (T) -> Int): List<T> {
    var used = 0
    val kept = ArrayDeque<T>()
    for (item in items.asReversed()) {
        used += size(item)
        if (used > maxChars && kept.isNotEmpty()) break
        kept.addFirst(item)
    }
    return kept
}

/**
 * Live `systemInstruction` for a voice session on a thread that already has entries: the base
 * prompt, the thread memory, and the entries after it as a transcript block. Live gets no
 * `clientContent` seeding (docs/protocol.md "App rules").
 */
fun voiceSystemInstruction(base: String, entries: List<Entry>, memory: ThreadMemory? = null): String {
    val (recent, used) = unfolded(entries, memory)
    val head = if (used == null) base else base + "\n\n" + memoryBlock(used)
    val lines = spoken(recent).takeLast(VOICE_MAX_ENTRIES).mapNotNull { entry ->
        when (entry.kind) {
            EntryKind.USER -> "User: ${entry.text.clip(ENTRY_MAX_CHARS)}"
            EntryKind.MODEL -> "You: ${entry.text.clip(ENTRY_MAX_CHARS)}"
            EntryKind.AGENT -> "Agent (${entry.project ?: "?"}): ${entry.text.clip(ENTRY_MAX_CHARS)}"
            else -> toolLine(entry)
        }
    }
    val kept = tailWithin(lines, VOICE_MAX_CHARS) { it.length + 1 }
    if (kept.isEmpty()) return head
    return head + "\n\n" +
        "This conversation is already running; part of it was typed, part spoken. Transcript of the " +
        "latest part, oldest first (`[tool ...]` = a tool you called). Continue from it. Do not read " +
        "it out, do not greet again, do not repeat answers already given.\n" +
        "<transcript>\n" + kept.joinToString("\n") + "\n</transcript>"
}

/** System instruction for a text turn: the base prompt plus the thread memory, when it applies. */
fun textSystemInstruction(base: String, entries: List<Entry>, memory: ThreadMemory? = null): String {
    val used = unfolded(entries, memory).second ?: return base
    return base + "\n\n" + memoryBlock(used)
}

/**
 * `contents` for a text turn: the entries after the memory's fold point, flattened to plain text
 * turns. Tool calls of earlier turns become a bracketed note (the structured call/response pair
 * only lives inside one tool loop); once the model has answered from a result, only its start is
 * kept. Consecutive same-role turns are merged, and the list starts with a user turn.
 */
fun textHistory(entries: List<Entry>, memory: ThreadMemory? = null): List<JsonObject> {
    val recent = spoken(unfolded(entries, memory).first).takeLast(TEXT_MAX_ENTRIES)
    val lastAnswer = recent.indexOfLast { it.kind == EntryKind.MODEL }
    val turns = recent.mapIndexedNotNull { index, entry ->
        when (entry.kind) {
            EntryKind.USER -> "user" to entry.text.clip(ENTRY_MAX_CHARS)
            EntryKind.MODEL -> "model" to entry.text.clip(ENTRY_MAX_CHARS)
            EntryKind.AGENT -> "user" to "[Not the user: message from the agent working on project " +
                "\"${entry.project ?: "?"}\"]\n${entry.text.clip(ENTRY_MAX_CHARS)}"
            else -> entry.tool?.let { tool ->
                val max = if (index > lastAnswer) TOOL_RESULT_MAX_CHARS else TOOL_RESULT_OLD_MAX_CHARS
                val result = tool.result?.toString()?.clip(max) ?: "no result"
                "user" to "[Earlier tool call, for context: ${tool.name}(${tool.args.toString().clip(TOOL_ARGS_MAX_CHARS)}) -> $result]"
            }
        }
    }
    val kept = tailWithin(turns, TEXT_MAX_CHARS) { it.second.length }.dropWhile { it.first != "user" }
    val merged = mutableListOf<Pair<String, String>>()
    for ((role, text) in kept) {
        val last = merged.lastOrNull()
        if (last != null && last.first == role) merged[merged.lastIndex] = role to (last.second + "\n" + text)
        else merged += role to text
    }
    return merged.map { (role, text) -> textContent(role, text) }
}

/** Characters of text in [contents]: the size of a history as sent, for logs and estimates. */
fun historyChars(contents: List<JsonObject>): Int = contents.sumOf { content ->
    (content["parts"] as? JsonArray).orEmpty().sumOf { ((it as? JsonObject)?.get("text") as? JsonPrimitive)?.content?.length ?: 0 }
}

/** Rough token count of [chars] of prompt text. Shown with a `~`: it is a guess until Gemini reports a real one. */
fun estimateTokens(chars: Int): Long = (chars / 4).toLong()
