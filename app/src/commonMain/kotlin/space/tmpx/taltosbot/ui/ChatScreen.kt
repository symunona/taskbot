package space.tmpx.taltosbot.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Build
import androidx.compose.material.icons.filled.ExpandLess
import androidx.compose.material.icons.filled.ExpandMore
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isShiftPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import space.tmpx.taltosbot.connection.Entry
import space.tmpx.taltosbot.connection.EntryKind
import space.tmpx.taltosbot.connection.MARKER_VOICE_ENDED
import space.tmpx.taltosbot.connection.MARKER_VOICE_STARTED
import space.tmpx.taltosbot.connection.ToolCall
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject

@Composable
fun ChatScreen(controller: AppController) {
    val entries = controller.entries
    val listState = rememberLazyListState()

    // Follow the end, unless the user scrolled up to read. Back at the end: following again.
    var follow by remember { mutableStateOf(true) }
    // Past the end of the last row: a long bubble shows its newest lines, not its top.
    suspend fun scrollToEnd() {
        val rows = entries.size + if (controller.textBusy) 1 else 0
        if (rows > 0) listState.scrollToItem(rows - 1, Int.MAX_VALUE)
    }
    LaunchedEffect(listState) {
        snapshotFlow { listState.isScrollInProgress }.collect { scrolling ->
            if (!scrolling) follow = !listState.canScrollForward
        }
    }
    // Content changed: new entry, a growing transcript, a tool result, the thinking row.
    LaunchedEffect(entries.size, entries.lastOrNull(), controller.textBusy) {
        // Own message: always shown, wherever the list was.
        if (entries.lastOrNull()?.kind == EntryKind.USER) follow = true
        if (follow) scrollToEnd()
    }
    // Keyboard opened or the task strip grew: the list got shorter.
    LaunchedEffect(listState) {
        snapshotFlow { listState.layoutInfo.viewportSize.height }.collect { if (follow) scrollToEnd() }
    }
    // Another thread opened: start at its end.
    LaunchedEffect(controller.currentThread?.id) {
        follow = true
        scrollToEnd()
    }

    Column(modifier = Modifier.fillMaxSize()) {
        if (controller.threadLoading) LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
        TasksStrip(controller)
        LazyColumn(
            state = listState,
            modifier = Modifier.weight(1f).padding(horizontal = 10.dp, vertical = 5.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            if (entries.isEmpty()) {
                item {
                    Text(
                        if (platformVoiceSupported) "Nothing yet. Talk or type; messages from project agents show up here too."
                        else "Nothing yet. Type below; messages from project agents show up here too.",
                        color = Color.Gray,
                        modifier = Modifier.fillMaxWidth().padding(top = 16.dp)
                    )
                }
            }
            items(entries, key = { it.id }) { entry ->
                EntryRow(entry, toolRunning = entry.id in controller.runningTools)
            }
            if (controller.textBusy) {
                item { ThinkingRow() }
            }
        }
        ChatInput(controller)
    }
}

@Composable
private fun EntryRow(entry: Entry, toolRunning: Boolean) {
    val tool = entry.tool
    when {
        entry.kind == EntryKind.MARKER -> MarkerRow(entry)
        entry.kind == EntryKind.TOOL && tool != null -> ToolRow(entry.id, tool, toolRunning)
        entry.kind == EntryKind.SYSTEM -> SystemRow(entry)
        else -> MessageBubble(entry)
    }
}

/** Thin centered divider: "voice started · 14:02". */
@Composable
private fun MarkerRow(entry: Entry) {
    val label = when (entry.text) {
        MARKER_VOICE_STARTED -> "voice started"
        MARKER_VOICE_ENDED -> "voice ended"
        else -> entry.text.replace('_', ' ')
    }
    Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Divider(color = Color.DarkGray, modifier = Modifier.weight(1f))
        Text(
            "$label · ${clockTime(entry.ts)}",
            color = Color.Gray,
            style = MaterialTheme.typography.caption,
            modifier = Modifier.padding(horizontal = 8.dp)
        )
        Divider(color = Color.DarkGray, modifier = Modifier.weight(1f))
    }
}

@Composable
private fun SystemRow(entry: Entry) {
    Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.Center) {
        Box(modifier = Modifier.background(Color(0xFF555555), MaterialTheme.shapes.small).padding(8.dp)) {
            Text(
                entry.text,
                color = if (entry.error) Color(0xFFFF8A80) else Color.LightGray,
                style = MaterialTheme.typography.caption
            )
        }
    }
}

@Composable
private fun MessageBubble(entry: Entry) {
    val bubbleColor = when (entry.kind) {
        EntryKind.USER -> Color(0xFF1565C0)
        EntryKind.MODEL -> Color(0xFF2E7D32)
        EntryKind.AGENT -> Color(0xFF6A1B9A)
        else -> Color(0xFF333333)
    }
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = if (entry.kind == EntryKind.USER) Arrangement.End else Arrangement.Start
    ) {
        Column(
            modifier = Modifier.fillMaxWidth(0.8f).background(bubbleColor, MaterialTheme.shapes.medium).padding(10.dp)
        ) {
            if (entry.kind == EntryKind.AGENT) {
                Text("Agent: ${entry.project ?: "?"}", color = Color(0xFFE1BEE7), style = MaterialTheme.typography.caption)
            }
            Text(entry.text, color = if (entry.error) Color(0xFFFF8A80) else Color.White)
        }
    }
}

private val prettyJson = Json { prettyPrint = true }
private const val TOOL_DETAIL_MAX_CHARS = 4_000

private fun pretty(element: JsonElement): String {
    val text = prettyJson.encodeToString(JsonElement.serializer(), element)
    return if (text.length <= TOOL_DETAIL_MAX_CHARS) text
    else text.take(TOOL_DETAIL_MAX_CHARS) + "\n… (${text.length - TOOL_DETAIL_MAX_CHARS} more chars)"
}

/**
 * One tool call. Closed: a single line, tool name + state. Open: arguments, result, when it was
 * called, when the result came.
 */
@Composable
private fun ToolRow(id: String, tool: ToolCall, running: Boolean) {
    var open by rememberSaveable(id) { mutableStateOf(false) }
    val failed = (tool.result as? JsonObject)?.containsKey("error") == true
    val status = when {
        tool.result != null -> if (failed) "failed" else null
        running -> "running"
        else -> "no result"
    }

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(MaterialTheme.shapes.small)
            .background(Color(0xFF262626))
            .clickable { open = !open }
            .semantics { contentDescription = "Tool ${tool.name}" + (status?.let { ", $it" } ?: "") }
            .padding(horizontal = 8.dp, vertical = 4.dp)
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Filled.Build, contentDescription = null, tint = Color.Gray, modifier = Modifier.size(12.dp))
            Text(
                tool.name,
                color = Color.LightGray,
                style = MaterialTheme.typography.caption,
                fontFamily = FontFamily.Monospace,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f).padding(horizontal = 6.dp)
            )
            if (status == "running") {
                CircularProgressIndicator(modifier = Modifier.size(12.dp), color = Color.Yellow, strokeWidth = 2.dp)
            } else if (status != null) {
                Text(status, color = if (failed) Color(0xFFFF8A80) else Color.Gray, style = MaterialTheme.typography.caption)
            }
            Icon(
                if (open) Icons.Filled.ExpandLess else Icons.Filled.ExpandMore,
                contentDescription = null,
                tint = Color.Gray,
                modifier = Modifier.padding(start = 6.dp).size(16.dp)
            )
        }
        if (open) {
            ToolDetail("Arguments", pretty(tool.args))
            ToolDetail("Result", tool.result?.let(::pretty) ?: if (running) "still running…" else "none")
            val took = tool.resultAt?.let { " (${it - tool.calledAt} ms)" } ?: ""
            Text(
                "Called ${clockTime(tool.calledAt, seconds = true)}  ·  Result " +
                    (tool.resultAt?.let { clockTime(it, seconds = true) } ?: "—") + took,
                color = Color.Gray,
                style = MaterialTheme.typography.caption,
                modifier = Modifier.padding(top = 6.dp, bottom = 2.dp)
            )
        }
    }
}

@Composable
private fun ToolDetail(label: String, body: String) {
    Text(label, color = Color.Gray, style = MaterialTheme.typography.caption, modifier = Modifier.padding(top = 6.dp))
    Text(body, color = Color.LightGray, style = MaterialTheme.typography.caption, fontFamily = FontFamily.Monospace)
}

/** Text turn in flight. */
@Composable
private fun ThinkingRow() {
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(4.dp)) {
        CircularProgressIndicator(modifier = Modifier.size(14.dp), color = Color.Gray, strokeWidth = 2.dp)
        Text("thinking…", color = Color.Gray, style = MaterialTheme.typography.caption, modifier = Modifier.padding(start = 8.dp))
    }
}

/**
 * Fixed under the list. Grows with the text up to 6 lines, then scrolls inside.
 * Hardware keyboard: Enter sends, Shift+Enter breaks the line.
 */
@Composable
private fun ChatInput(controller: AppController) {
    var draft by rememberSaveable(stateSaver = TextFieldValue.Saver) { mutableStateOf(TextFieldValue("")) }
    val voiceActive = controller.voiceState == VoiceState.Active
    val canSend = draft.text.isNotBlank() && !controller.textBusy && controller.voiceState != VoiceState.Connecting
    val send = { if (canSend && controller.sendMessage(draft.text)) draft = TextFieldValue("") }

    Row(
        modifier = Modifier.fillMaxWidth().background(Color(0xFF2D2D2D)).padding(start = 8.dp, top = 6.dp, bottom = 6.dp),
        verticalAlignment = Alignment.Bottom
    ) {
        OutlinedTextField(
            value = draft,
            onValueChange = { draft = it },
            modifier = Modifier.weight(1f).onPreviewKeyEvent { event ->
                if (event.key != Key.Enter) return@onPreviewKeyEvent false
                if (event.type == KeyEventType.KeyDown) {
                    if (event.isShiftPressed) {
                        // The text field itself has no binding for Shift+Enter.
                        val selection = draft.selection
                        draft = TextFieldValue(
                            draft.text.replaceRange(selection.min, selection.max, "\n"),
                            TextRange(selection.min + 1)
                        )
                    } else {
                        send()
                    }
                }
                true
            },
            placeholder = { Text(if (voiceActive) "Type into the voice conversation" else "Message") },
            maxLines = 6
        )
        IconButton(onClick = send, enabled = canSend, modifier = Modifier.padding(bottom = 4.dp)) {
            Icon(
                Icons.AutoMirrored.Filled.Send,
                contentDescription = "Send",
                tint = if (canSend) MaterialTheme.colors.primary else Color.Gray
            )
        }
    }
}
