package space.tmpx.taltosbot.ui

import kotlinx.browser.localStorage
import kotlinx.browser.window

class JsKeyValueStorage : KeyValueStorage {
    override fun getString(key: String): String? = localStorage.getItem(key)
    override fun setString(key: String, value: String) {
        localStorage.setItem(key, value)
    }
    override fun remove(key: String) {
        localStorage.removeItem(key)
    }
}

actual fun getStorage(): KeyValueStorage = JsKeyValueStorage()

actual fun showPlatformToast(message: String) {
    console.log(message)
}

actual fun openUrl(url: String) {
    window.open(url, "_blank")
}

actual val platformDeviceName: String = "Taltosbot Web"
actual val platformVoiceSupported: Boolean = false
actual val platformUpdateCheckSupported: Boolean = false
