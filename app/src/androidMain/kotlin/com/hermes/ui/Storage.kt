package com.hermes.ui

import android.content.Context
import android.content.SharedPreferences

object AndroidStorage {
    var context: Context? = null
    private val prefs: SharedPreferences?
        get() = context?.getSharedPreferences("hermes_prefs", Context.MODE_PRIVATE)

    fun getString(key: String): String? = prefs?.getString(key, null)
    fun setString(key: String, value: String) {
        prefs?.edit()?.putString(key, value)?.apply()
    }
}

class AndroidKeyValueStorage : KeyValueStorage {
    override fun getString(key: String): String? = AndroidStorage.getString(key)
    override fun setString(key: String, value: String) = AndroidStorage.setString(key, value)
}

actual fun getStorage(): KeyValueStorage = AndroidKeyValueStorage()
