package space.tmpx.taltosbot.ui

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
import android.net.Uri
import android.widget.Toast

object AndroidStorage {
    var context: Context? = null
    private val prefs: SharedPreferences?
        get() = context?.getSharedPreferences("taltosbot_prefs", Context.MODE_PRIVATE)

    fun getString(key: String): String? = prefs?.getString(key, null)
    fun setString(key: String, value: String) {
        prefs?.edit()?.putString(key, value)?.apply()
    }
    fun remove(key: String) {
        prefs?.edit()?.remove(key)?.apply()
    }
}

class AndroidKeyValueStorage : KeyValueStorage {
    override fun getString(key: String): String? = AndroidStorage.getString(key)
    override fun setString(key: String, value: String) = AndroidStorage.setString(key, value)
    override fun remove(key: String) = AndroidStorage.remove(key)
}

actual fun getStorage(): KeyValueStorage = AndroidKeyValueStorage()

actual fun showPlatformToast(message: String) {
    AndroidStorage.context?.let { context ->
        Toast.makeText(context, message, Toast.LENGTH_SHORT).show()
    }
}

actual fun openUrl(url: String) {
    val context = AndroidStorage.context ?: return
    try {
        context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    } catch (e: ActivityNotFoundException) {
        showPlatformToast("No app can open $url")
    }
}

actual val platformDeviceName: String = "Taltosbot Android"
actual val platformVoiceSupported: Boolean = true
actual val platformUpdateCheckSupported: Boolean = true
