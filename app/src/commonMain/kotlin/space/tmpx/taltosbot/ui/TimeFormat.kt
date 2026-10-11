package space.tmpx.taltosbot.ui

import kotlinx.datetime.Instant
import kotlinx.datetime.LocalDateTime
import kotlinx.datetime.TimeZone
import kotlinx.datetime.toLocalDateTime

private fun local(ms: Long): LocalDateTime =
    Instant.fromEpochMilliseconds(ms).toLocalDateTime(TimeZone.currentSystemDefault())

private fun two(n: Int) = n.toString().padStart(2, '0')

/** "14:02", or "14:02:07" with [seconds]. */
internal fun clockTime(ms: Long, seconds: Boolean = false): String {
    val t = local(ms)
    return "${two(t.hour)}:${two(t.minute)}" + if (seconds) ":${two(t.second)}" else ""
}

/** "just now", "5 min ago", "3 h ago", "yesterday", "12 Oct", "12 Oct 2025". */
internal fun relativeTime(ms: Long, now: Long): String {
    val minutes = (now - ms) / 60_000L
    if (minutes < 1) return "just now"
    if (minutes < 60) return "$minutes min ago"
    val then = local(ms)
    val today = local(now)
    val days = today.date.toEpochDays() - then.date.toEpochDays()
    return when {
        days <= 0 -> "${minutes / 60} h ago"
        days == 1 -> "yesterday"
        else -> {
            val month = then.month.name.take(3).lowercase().replaceFirstChar { it.uppercase() }
            "${then.dayOfMonth} $month" + if (then.year != today.year) " ${then.year}" else ""
        }
    }
}
