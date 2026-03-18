package com.hermes.ui

import android.util.Log

actual fun platformLog(tag: String, message: String, isError: Boolean) {
    if (isError) {
        Log.e(tag, message)
    } else {
        Log.d(tag, message)
    }
}
