package com.hermes.ui

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.datetime.Clock

data class LogEvent(val timestamp: Long, val message: String, val isError: Boolean = false)

object EventLogger {
    private val _logs = MutableStateFlow<List<LogEvent>>(emptyList())
    val logs: StateFlow<List<LogEvent>> = _logs.asStateFlow()

    fun log(message: String, isError: Boolean = false) {
        println("EventLogger: $message")
        val event = LogEvent(Clock.System.now().toEpochMilliseconds(), message, isError)
        _logs.value = _logs.value + event
    }
}
