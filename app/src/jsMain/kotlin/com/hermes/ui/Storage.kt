package com.hermes.ui

import kotlinx.browser.localStorage

class JsKeyValueStorage : KeyValueStorage {
    override fun getString(key: String): String? = localStorage.getItem(key)
    override fun setString(key: String, value: String) {
        localStorage.setItem(key, value)
    }
}

actual fun getStorage(): KeyValueStorage = JsKeyValueStorage()
