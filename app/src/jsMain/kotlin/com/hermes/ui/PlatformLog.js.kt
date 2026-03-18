package com.hermes.ui

actual fun platformLog(tag: String, message: String, isError: Boolean) {
    if (isError) {
        console.error("[$tag] $message")
    } else {
        console.log("[$tag] $message")
    }
}
