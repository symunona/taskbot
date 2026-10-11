package space.tmpx.taltosbot.ui

import android.util.Log

actual fun platformLog(tag: String, message: String, level: LogLevel) {
    when (level) {
        LogLevel.VERBOSE -> Log.v(tag, message)
        LogLevel.DEBUG -> Log.d(tag, message)
        LogLevel.INFO -> Log.i(tag, message)
        LogLevel.ERROR -> Log.e(tag, message)
    }
}
