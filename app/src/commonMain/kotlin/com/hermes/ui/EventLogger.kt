package com.hermes.ui

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.datetime.Clock

data class LogEvent(val timestamp: Long, val message: String, val isError: Boolean = false)

expect fun platformLog(tag: String, message: String, isError: Boolean)

object EventLogger {
    const val TAG = "Hermes"

    private val _logs = MutableStateFlow<List<LogEvent>>(emptyList())
    val logs: StateFlow<List<LogEvent>> = _logs.asStateFlow()

    private const val MAX_LOGS = 200

    fun log(message: String, isError: Boolean = false) {
        platformLog(TAG, message, isError)
        val event = LogEvent(Clock.System.now().toEpochMilliseconds(), message, isError)
        val current = _logs.value
        _logs.value = if (current.size >= MAX_LOGS) {
            current.drop(current.size - MAX_LOGS + 1) + event
        } else {
            current + event
        }
    }
}
