package space.tmpx.taltosbot.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Close
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import space.tmpx.taltosbot.connection.ConnState
import space.tmpx.taltosbot.connection.EntryKind
import kotlinx.datetime.Clock

/** "950", "3.1k", "212k", "1.4M". */
internal fun tokenCount(n: Long): String = when {
    n < 1_000 -> n.toString()
    n < 10_000 -> "${n / 1_000}.${n % 1_000 / 100}k"
    n < 1_000_000 -> "${n / 1_000}k"
    else -> "${n / 1_000_000}.${n % 1_000_000 / 100_000}M"
}

/**
 * Right-side drawer over the whole screen, about the thread on screen: metadata, token use, and
 * on its second page ("Long CTX") the thread memory the model works from, with the COMPACT button.
 */
@Composable
fun BoxScope.ThreadPanel(controller: AppController, open: Boolean, onClose: () -> Unit) {
    // A `threads.memory` push may have been missed while offline.
    LaunchedEffect(open) { if (open) controller.refreshMemory() }
    val now = remember(open, controller.memory, controller.currentThread) { Clock.System.now().toEpochMilliseconds() }
    var longCtx by remember(open) { mutableStateOf(false) }

    AnimatedVisibility(visible = open, enter = fadeIn(), exit = fadeOut()) {
        Box(
            modifier = Modifier.fillMaxSize().background(Color(0x99000000))
                .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onClose)
        )
    }
    AnimatedVisibility(
        visible = open,
        enter = slideInHorizontally { it },
        exit = slideOutHorizontally { it },
        modifier = Modifier.align(Alignment.CenterEnd)
    ) {
        Surface(modifier = Modifier.fillMaxHeight().widthIn(max = 320.dp).fillMaxWidth(0.85f), elevation = 16.dp) {
            Column(modifier = Modifier.fillMaxSize().padding(vertical = 12.dp)) {
                Row(modifier = Modifier.padding(start = if (longCtx) 4.dp else 16.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                    if (longCtx) {
                        IconButton(onClick = { longCtx = false }) {
                            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back to thread")
                        }
                    }
                    Text(if (longCtx) "Long CTX" else "Thread", style = MaterialTheme.typography.h6, modifier = Modifier.weight(1f))
                    IconButton(onClick = onClose) { Icon(Icons.Filled.Close, contentDescription = "Close thread panel") }
                }
                if (longCtx) LongCtxPage(controller, now) else ThreadPage(controller, now, onLongCtx = { longCtx = true })
            }
        }
    }
}

@Composable
private fun ColumnScope.ThreadPage(controller: AppController, now: Long, onLongCtx: () -> Unit) {
    val thread = controller.currentThread
    Column(modifier = Modifier.weight(1f).padding(horizontal = 16.dp)) {
        if (thread == null) {
            Text("No thread yet. Say or type something.", color = Color.Gray, modifier = Modifier.padding(vertical = 8.dp))
            return@Column
        }
        Text(thread.name, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(vertical = 8.dp))
        InfoRow("created", relativeTime(thread.createdAt, now))
        InfoRow("last used", relativeTime(thread.lastAccessedAt, now))
        InfoRow("entries", controller.entries.count { it.kind != EntryKind.MARKER && it.kind != EntryKind.SYSTEM }.toString())

        Spacer(modifier = Modifier.height(16.dp))
        Text("Tokens", color = Color.Gray, style = MaterialTheme.typography.caption)
        // Real count once Gemini has answered on this device; before that, a guess from the text.
        InfoRow("context now", controller.contextTokens?.let(::tokenCount) ?: "~${tokenCount(controller.contextEstimate)}")
        InfoRow("all time", "${tokenCount(controller.usage.input)} in / ${tokenCount(controller.usage.output)} out")

        Spacer(modifier = Modifier.height(16.dp))
        OutlinedButton(onClick = onLongCtx, modifier = Modifier.fillMaxWidth()) {
            Text("Long CTX", modifier = Modifier.weight(1f))
            Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, contentDescription = null, modifier = Modifier.size(18.dp))
        }
        Text(
            compactedLine(controller, now),
            color = Color.Gray,
            style = MaterialTheme.typography.caption,
            modifier = Modifier.padding(top = 4.dp)
        )
    }
}

private fun compactedLine(controller: AppController, now: Long): String {
    val turns = controller.turnsSinceCompact
    val since = "$turns ${if (turns == 1) "turn" else "turns"} since"
    return controller.memory?.let { "compacted ${relativeTime(it.at, now)}, $since" } ?: "nothing compacted yet, $turns ${if (turns == 1) "turn" else "turns"}"
}

/** The thread memory, as the model gets it, and the COMPACT button. */
@Composable
private fun ColumnScope.LongCtxPage(controller: AppController, now: Long) {
    val connectionState by controller.connection.state.collectAsState()
    Text(
        compactedLine(controller, now),
        color = Color.Gray,
        style = MaterialTheme.typography.caption,
        modifier = Modifier.padding(horizontal = 16.dp)
    )
    Box(modifier = Modifier.weight(1f).fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).verticalScroll(rememberScrollState())) {
        val memory = controller.memory
        if (memory == null) {
            Text("The model still gets the conversation itself. Compact folds it into a short memory.", color = Color.Gray)
        } else {
            SelectionContainer {
                Text(memory.text, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.body2)
            }
        }
    }
    controller.compactError?.let {
        Text(it, color = Color(0xFFFF8A80), style = MaterialTheme.typography.caption, modifier = Modifier.padding(horizontal = 16.dp))
    }
    Button(
        onClick = controller::compact,
        enabled = !controller.compacting && controller.currentThread != null && connectionState == ConnState.Connected,
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp)
    ) {
        if (controller.compacting) {
            CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
            Spacer(modifier = Modifier.width(8.dp))
        }
        Text("COMPACT")
    }
}

@Composable
private fun InfoRow(label: String, value: String) {
    Row(modifier = Modifier.fillMaxWidth().padding(vertical = 2.dp)) {
        Text(label, color = Color.Gray, modifier = Modifier.width(104.dp))
        Text(value)
    }
}
