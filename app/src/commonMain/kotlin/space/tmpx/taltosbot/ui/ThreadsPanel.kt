package space.tmpx.taltosbot.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Edit
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import space.tmpx.taltosbot.connection.ThreadInfo
import kotlinx.datetime.Clock

private const val THREAD_NAME_MAX = 80

/**
 * Thread list of the left drawer: new thread, pick, rename.
 * [open] = the drawer is showing; [onPicked] = a thread was chosen or created.
 */
@Composable
fun ColumnScope.ThreadsList(controller: AppController, open: Boolean, onPicked: () -> Unit) {
    // Fresh list (and fresh "5 min ago") every time it opens.
    LaunchedEffect(open) { if (open) controller.refreshThreads() }
    val now = remember(open, controller.threads) { Clock.System.now().toEpochMilliseconds() }
    var renaming by remember(open) { mutableStateOf<String?>(null) }

    Text("Threads", color = Color.Gray, style = MaterialTheme.typography.caption, modifier = Modifier.padding(horizontal = 16.dp))
    OutlinedButton(
        onClick = {
            controller.newThread()
            onPicked()
        },
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp)
    ) {
        Icon(Icons.Filled.Add, contentDescription = null, modifier = Modifier.size(18.dp))
        Spacer(modifier = Modifier.width(8.dp))
        Text("New thread")
    }
    controller.threadsError?.let {
        Text(it, color = Color(0xFFFF8A80), style = MaterialTheme.typography.caption, modifier = Modifier.padding(horizontal = 16.dp))
    }
    if (controller.threads.isEmpty() && controller.threadsError == null) {
        Text("No threads yet.", color = Color.Gray, modifier = Modifier.padding(16.dp))
    }
    LazyColumn(modifier = Modifier.weight(1f)) {
        items(controller.threads, key = { it.id }) { thread ->
            if (renaming == thread.id) {
                RenameRow(
                    thread,
                    onSave = {
                        controller.renameThread(thread.id, it)
                        renaming = null
                    },
                    onCancel = { renaming = null }
                )
            } else {
                ThreadRow(
                    thread,
                    current = thread.id == controller.currentThread?.id,
                    accessed = relativeTime(thread.lastAccessedAt, now),
                    onClick = {
                        controller.selectThread(thread.id)
                        onPicked()
                    },
                    onRename = { renaming = thread.id }
                )
            }
        }
    }
}

@Composable
private fun ThreadRow(thread: ThreadInfo, current: Boolean, accessed: String, onClick: () -> Unit, onRename: () -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth()
            .background(if (current) Color(0xFF2D2D2D) else Color.Transparent)
            .clickable(onClick = onClick)
            .padding(start = 16.dp, end = 4.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Column(modifier = Modifier.weight(1f).padding(vertical = 8.dp)) {
            Text(
                thread.name,
                color = if (current) MaterialTheme.colors.primary else Color.White,
                fontWeight = if (current) FontWeight.Bold else FontWeight.Normal,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
            Text(accessed, color = Color.Gray, style = MaterialTheme.typography.caption)
        }
        IconButton(onClick = onRename) {
            Icon(Icons.Filled.Edit, contentDescription = "Rename ${thread.name}", tint = Color.Gray, modifier = Modifier.size(18.dp))
        }
    }
}

/** The row itself turns into the name field. */
@Composable
private fun RenameRow(thread: ThreadInfo, onSave: (String) -> Unit, onCancel: () -> Unit) {
    var name by remember(thread.id) { mutableStateOf(TextFieldValue(thread.name, TextRange(thread.name.length))) }
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    Row(modifier = Modifier.fillMaxWidth().padding(start = 12.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        OutlinedTextField(
            value = name,
            onValueChange = { if (it.text.length <= THREAD_NAME_MAX) name = it },
            modifier = Modifier.weight(1f).focusRequester(focus),
            singleLine = true
        )
        IconButton(onClick = { onSave(name.text) }, enabled = name.text.isNotBlank()) {
            Icon(Icons.Filled.Check, contentDescription = "Save name")
        }
        IconButton(onClick = onCancel) {
            Icon(Icons.Filled.Close, contentDescription = "Cancel rename")
        }
    }
}
