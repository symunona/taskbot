package space.tmpx.taltosbot.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.datetime.Clock
import space.tmpx.taltosbot.connection.TaskInfo
import space.tmpx.taltosbot.connection.TaskState

/** A finished task stays in the strip this long, so its outcome is seen. */
internal const val TASK_LINGER_MS = 5 * 60_000L
private const val TASKS_SHOWN_MAX = 4

/** Tasks worth showing now: unfinished ones, and those that finished a moment ago. Newest first. */
internal fun visibleTasks(tasks: List<TaskInfo>, now: Long): List<TaskInfo> =
    tasks.filter { !it.finished || now - it.updatedAt < TASK_LINGER_MS }

/** "8 s", "3:07", "1:02:45". */
internal fun elapsedText(ms: Long): String {
    val s = ms / 1000
    val two = { n: Long -> n.toString().padStart(2, '0') }
    return when {
        s < 60 -> "$s s"
        s < 3600 -> "${s / 60}:${two(s % 60)}"
        else -> "${s / 3600}:${two(s % 3600 / 60)}:${two(s % 60)}"
    }
}

/** State as the user reads it: "queued, 2 ahead", "working", "needs you". */
internal fun taskStateLabel(task: TaskInfo): String = when (task.state) {
    TaskState.QUEUED, TaskState.SENT -> {
        val ahead = task.queuePos ?: 0
        if (ahead > 0) "queued, $ahead ahead" else if (task.state == TaskState.SENT) "sent" else "queued"
    }
    TaskState.SPAWNING -> "starting agent"
    TaskState.WAITING -> "needs you"
    else -> task.state
}

private fun taskColor(state: String): Color = when (state) {
    TaskState.WORKING -> Color(0xFFFFD54F)
    TaskState.WAITING -> Color(0xFFFFAB40)
    TaskState.DONE -> Color(0xFF81C784)
    TaskState.FAILED -> Color(0xFFFF8A80)
    else -> Color.Gray
}

/**
 * Live state of delegated tasks, fed by `tasks.changed`. Sits above the chat; nothing here goes
 * to the voice model. One line per task; tap = prompt, answer, error.
 */
@Composable
fun TasksStrip(controller: AppController) {
    var now by remember { mutableStateOf(Clock.System.now().toEpochMilliseconds()) }
    val shown = visibleTasks(controller.tasks, now)
    // Tick only while something is on screen: elapsed time, and finished tasks leaving.
    LaunchedEffect(shown.isNotEmpty()) {
        while (true) {
            now = Clock.System.now().toEpochMilliseconds()
            delay(1000L)
        }
    }
    if (shown.isEmpty()) return
    Column(
        modifier = Modifier.fillMaxWidth().background(Color(0xFF1E1E1E)).padding(horizontal = 10.dp, vertical = 4.dp),
        verticalArrangement = Arrangement.spacedBy(2.dp)
    ) {
        shown.take(TASKS_SHOWN_MAX).forEach { task -> key(task.id) { TaskRow(task, now) } }
        if (shown.size > TASKS_SHOWN_MAX) {
            Text("+${shown.size - TASKS_SHOWN_MAX} more tasks", color = Color.Gray, style = MaterialTheme.typography.caption)
        }
    }
}

@Composable
private fun TaskRow(task: TaskInfo, now: Long) {
    var open by rememberSaveable(task.id) { mutableStateOf(false) }
    val color = taskColor(task.state)
    val label = taskStateLabel(task)
    val detail = when {
        task.state == TaskState.FAILED -> task.error
        task.state == TaskState.DONE -> task.answer
        else -> task.lastActivity
    } ?: task.prompt
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clickable { open = !open }
            .semantics { contentDescription = "Task ${task.project}, $label" }
            .padding(vertical = 3.dp)
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            if (task.state == TaskState.WORKING || task.state == TaskState.SPAWNING) {
                CircularProgressIndicator(modifier = Modifier.size(10.dp), color = color, strokeWidth = 2.dp)
            } else {
                Box(modifier = Modifier.size(10.dp).background(color, CircleShape))
            }
            Text(
                task.project,
                color = Color.White,
                style = MaterialTheme.typography.caption,
                modifier = Modifier.padding(start = 6.dp)
            )
            Text(label, color = color, style = MaterialTheme.typography.caption, modifier = Modifier.padding(start = 6.dp))
            Text(
                detail.replace('\n', ' '),
                color = Color.Gray,
                style = MaterialTheme.typography.caption,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f).padding(horizontal = 6.dp)
            )
            Text(
                elapsedText(task.elapsedMs(now)),
                color = Color.Gray,
                style = MaterialTheme.typography.caption,
                fontFamily = FontFamily.Monospace
            )
        }
        if (open) {
            TaskDetail("Task", task.prompt)
            task.lastActivity?.let { TaskDetail("Now", it) }
            task.answer?.let { TaskDetail("Answer", it) }
            task.error?.let { TaskDetail("Error", it, Color(0xFFFF8A80)) }
            Text(
                "Handed over ${clockTime(task.createdAt, seconds = true)}" +
                    (task.startedAt?.let { "  ·  started ${clockTime(it, seconds = true)}" } ?: "") +
                    (if (task.finished) "  ·  ended ${clockTime(task.updatedAt, seconds = true)}" else ""),
                color = Color.Gray,
                style = MaterialTheme.typography.caption,
                modifier = Modifier.padding(start = 16.dp, top = 2.dp)
            )
        }
    }
}

@Composable
private fun TaskDetail(label: String, body: String, color: Color = Color.LightGray) {
    Row(modifier = Modifier.padding(start = 16.dp, top = 2.dp)) {
        Text(label, color = Color.Gray, style = MaterialTheme.typography.caption, modifier = Modifier.width(52.dp))
        Text(body, color = color, style = MaterialTheme.typography.caption, maxLines = 8, overflow = TextOverflow.Ellipsis)
    }
}
