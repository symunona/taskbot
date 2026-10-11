package space.tmpx.taltosbot.ui

actual fun platformLog(tag: String, message: String, level: LogLevel) {
    when (level) {
        LogLevel.VERBOSE, LogLevel.DEBUG -> console.asDynamic().debug("[$tag] $message")
        LogLevel.INFO -> console.log("[$tag] $message")
        LogLevel.ERROR -> console.error("[$tag] $message")
    }
}
