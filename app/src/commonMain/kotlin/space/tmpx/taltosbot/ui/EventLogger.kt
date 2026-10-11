package space.tmpx.taltosbot.ui

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.datetime.Clock

/** Ordered by severity. VERBOSE = per-frame noise, DEBUG = protocol detail, INFO = lifecycle + tool calls. */
enum class LogLevel(val label: String) {
    VERBOSE("Verbose"),
    DEBUG("Debug"),
    INFO("Info"),
    ERROR("Errors"),
}

data class LogEvent(
    val timestamp: Long,
    val message: String,
    val isError: Boolean = false,
    val level: LogLevel = if (isError) LogLevel.ERROR else LogLevel.INFO,
    val tag: String = EventLogger.tagOf(message),
    /** How many consecutive collapsed events this line stands for. */
    val count: Int = 1,
    val collapseKey: String? = null,
)

expect fun platformLog(tag: String, message: String, level: LogLevel)

object EventLogger {
    const val TAG = "Taltosbot"

    private val _logs = MutableStateFlow<List<LogEvent>>(emptyList())
    val logs: StateFlow<List<LogEvent>> = _logs.asStateFlow()

    private const val MAX_LOGS = 500

    /** A collapsed run reaches the platform log on its first event and then every this many. */
    private const val PLATFORM_LOG_EVERY = 100

    private val SECRET_PARAM = Regex("""([?&](?:key|token|access_token|api_key)=)[^&\s"')]+""", RegexOption.IGNORE_CASE)
    private val GOOGLE_API_KEY = Regex("""AIza[0-9A-Za-z_\-]{20,}""")
    private val BEARER = Regex("""(Bearer\s+)[A-Za-z0-9._\-]+""", RegexOption.IGNORE_CASE)

    /** Last line of defence: callers should not pass secrets, this masks the usual shapes anyway. */
    fun sanitize(message: String): String = message
        .replace(SECRET_PARAM, "$1***")
        .replace(GOOGLE_API_KEY, "***")
        .replace(BEARER, "$1***")

    /** Tag from the message prefix: "GeminiLive: ..." -> "GeminiLive", "agent[proj] ..." -> "agent". */
    fun tagOf(message: String): String {
        val head = message.substringBefore(' ')
        return when {
            head.startsWith("agent[") -> "agent"
            head.endsWith(":") && head.length > 1 -> head.dropLast(1)
            else -> "App"
        }
    }

    /**
     * @param collapseKey consecutive events with the same key fold into one counted line
     *   (showing the latest message) instead of flooding the ring and the platform log.
     */
    fun log(
        message: String,
        isError: Boolean = false,
        level: LogLevel = if (isError) LogLevel.ERROR else LogLevel.INFO,
        collapseKey: String? = null,
    ) {
        val clean = sanitize(message)
        val now = Clock.System.now().toEpochMilliseconds()
        var count = 1
        _logs.update { current ->
            val last = current.lastOrNull()
            if (collapseKey != null && last != null && last.collapseKey == collapseKey) {
                count = last.count + 1
                current.dropLast(1) + last.copy(timestamp = now, message = clean, level = level, count = count)
            } else {
                count = 1
                val event = LogEvent(now, clean, level == LogLevel.ERROR, level, tagOf(clean), 1, collapseKey)
                if (current.size >= MAX_LOGS) current.drop(current.size - MAX_LOGS + 1) + event else current + event
            }
        }
        if (count == 1) {
            platformLog(TAG, clean, level)
        } else if (count % PLATFORM_LOG_EVERY == 0) {
            platformLog(TAG, "$clean (x$count)", level)
        }
    }

    fun clear() {
        _logs.value = emptyList()
    }
}
