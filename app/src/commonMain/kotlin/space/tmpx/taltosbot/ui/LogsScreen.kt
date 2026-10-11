package space.tmpx.taltosbot.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import kotlinx.datetime.Instant
import kotlinx.datetime.TimeZone
import kotlinx.datetime.toLocalDateTime

/** Filter state outlives the screen so leaving and reopening Logs keeps the selection. */
object LogFilter {
    /** Default: errors + lifecycle + tool calls. Frame noise and protocol detail are opt-in. */
    val DEFAULT_LEVELS = setOf(LogLevel.INFO, LogLevel.ERROR)

    var levels by mutableStateOf(DEFAULT_LEVELS)
    var hiddenTags by mutableStateOf(emptySet<String>())

    fun matches(event: LogEvent): Boolean = event.level in levels && event.tag !in hiddenTags
}

@Composable
fun LogsScreen() {
    val logs by EventLogger.logs.collectAsState()
    val levelCounts = remember(logs) { logs.groupingBy { it.level }.eachCount() }
    val tags = remember(logs) { logs.map { it.tag }.distinct().sorted() }
    val visible = remember(logs, LogFilter.levels, LogFilter.hiddenTags) {
        logs.filter(LogFilter::matches).asReversed()
    }

    Column(modifier = Modifier.fillMaxSize().padding(8.dp)) {
        ChipRow {
            LogLevel.entries.reversed().forEach { level ->
                LogChip(
                    label = "${level.label} ${levelCounts[level] ?: 0}",
                    selected = level in LogFilter.levels,
                    color = levelColor(level),
                ) {
                    LogFilter.levels = LogFilter.levels.toggle(level)
                }
            }
        }
        ChipRow {
            tags.forEach { tag ->
                LogChip(label = tag, selected = tag !in LogFilter.hiddenTags, color = Color.White) {
                    LogFilter.hiddenTags = LogFilter.hiddenTags.toggle(tag)
                }
            }
            LogChip(label = "Reset", selected = false, color = Color.White) {
                LogFilter.levels = LogFilter.DEFAULT_LEVELS
                LogFilter.hiddenTags = emptySet()
            }
            LogChip(label = "Clear", selected = false, color = Color.White) { EventLogger.clear() }
        }
        Text(
            "${visible.size} of ${logs.size} shown",
            style = MaterialTheme.typography.caption,
            color = Color.Gray,
            modifier = Modifier.padding(vertical = 4.dp),
        )
        LazyColumn(modifier = Modifier.fillMaxSize()) {
            items(visible) { log ->
                val time = Instant.fromEpochMilliseconds(log.timestamp).toLocalDateTime(TimeZone.currentSystemDefault())
                val timeStr = "${time.hour.toString().padStart(2, '0')}:${time.minute.toString().padStart(2, '0')}:${time.second.toString().padStart(2, '0')}"
                val repeat = if (log.count > 1) " (x${log.count})" else ""
                Text(
                    "[$timeStr] ${log.message}$repeat",
                    style = MaterialTheme.typography.caption,
                    color = levelColor(log.level),
                )
            }
        }
    }
}

private fun <T> Set<T>.toggle(item: T): Set<T> = if (item in this) this - item else this + item

private fun levelColor(level: LogLevel): Color = when (level) {
    LogLevel.ERROR -> Color.Red
    LogLevel.INFO -> Color.White
    LogLevel.DEBUG -> Color.LightGray
    LogLevel.VERBOSE -> Color.Gray
}

@Composable
private fun ChipRow(content: @Composable RowScope.() -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(bottom = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(6.dp),
        verticalAlignment = Alignment.CenterVertically,
        content = content,
    )
}

@Composable
private fun LogChip(label: String, selected: Boolean, color: Color, onClick: () -> Unit) {
    Surface(
        shape = RoundedCornerShape(50),
        color = if (selected) color.copy(alpha = 0.22f) else Color.Transparent,
        border = androidx.compose.foundation.BorderStroke(1.dp, color.copy(alpha = if (selected) 0.9f else 0.35f)),
        modifier = Modifier.clickable(onClick = onClick),
    ) {
        Text(
            label,
            style = MaterialTheme.typography.caption,
            color = color.copy(alpha = if (selected) 1f else 0.6f),
            modifier = Modifier.padding(horizontal = 10.dp, vertical = 5.dp),
        )
    }
}
