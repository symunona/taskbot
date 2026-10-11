package space.tmpx.taltosbot.ui

interface KeyValueStorage {
    fun getString(key: String): String?
    fun setString(key: String, value: String)
    fun remove(key: String)
}

expect fun getStorage(): KeyValueStorage
expect fun showPlatformToast(message: String)

/** Opens a URL in the system browser (Android `ACTION_VIEW`, web `window.open`). */
expect fun openUrl(url: String)

/** Name sent to the server when pairing: "Taltosbot Android" / "Taltosbot Web". */
expect val platformDeviceName: String

/** False on web: the browser audio path is not implemented. */
expect val platformVoiceSupported: Boolean

/** True only where an APK can be installed. */
expect val platformUpdateCheckSupported: Boolean
