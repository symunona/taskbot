package space.tmpx.taltosbot.connection

import space.tmpx.taltosbot.ui.KeyValueStorage
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.put
import kotlin.io.encoding.Base64
import kotlin.io.encoding.ExperimentalEncodingApi

/** What the app keeps after `POST /api/v1/pair`. */
data class Pairing(val token: String, val deviceId: String, val serverUrls: List<String>)

/** Decoded QR / pasted connect string: base64 of `{"v":1,"code":"...","urls":[...]}`. */
data class ConnectString(val code: String, val urls: List<String>)

fun normalizeServerUrl(raw: String): String {
    val trimmed = raw.trim().trimEnd('/')
    return if (trimmed.startsWith("http://") || trimmed.startsWith("https://")) trimmed else "http://$trimmed"
}

internal fun JsonObject.str(key: String): String? = (this[key] as? JsonPrimitive)?.contentOrNull

internal fun JsonObject.strList(key: String): List<String> =
    (this[key] as? JsonArray)?.mapNotNull { (it as? JsonPrimitive)?.contentOrNull } ?: emptyList()

@OptIn(ExperimentalEncodingApi::class)
fun parseConnectString(raw: String): ConnectString {
    val compact = raw.filterNot { it.isWhitespace() }
    require(compact.isNotEmpty()) { "Connect string is empty" }

    val jsonText = if (compact.startsWith("{")) {
        raw.trim()
    } else {
        // Standard base64; be lenient about URL-safe alphabet and stripped padding.
        val std = compact.replace('-', '+').replace('_', '/').trimEnd('=')
        val padded = std + "=".repeat((4 - std.length % 4) % 4)
        try {
            Base64.decode(padded).decodeToString()
        } catch (e: Exception) {
            throw IllegalArgumentException("Not a valid connect string (bad base64)")
        }
    }

    val obj = try {
        Json.parseToJsonElement(jsonText).jsonObject
    } catch (e: Exception) {
        throw IllegalArgumentException("Not a valid connect string (bad JSON)")
    }

    val code = obj.str("code")?.trim()?.uppercase()
    require(!code.isNullOrEmpty()) { "Connect string has no pairing code" }
    val urls = obj.strList("urls").filter { it.isNotBlank() }.map(::normalizeServerUrl).distinct()
    require(urls.isNotEmpty()) { "Connect string has no server URL" }
    return ConnectString(code, urls)
}

class PairingStore(private val storage: KeyValueStorage) {
    fun load(): Pairing? {
        val saved = storage.getString(KEY)?.takeIf { it.isNotBlank() } ?: return null
        return try {
            val obj = Json.parseToJsonElement(saved).jsonObject
            val token = obj.str("token") ?: return null
            val urls = obj.strList("serverUrls")
            if (token.isEmpty() || urls.isEmpty()) null else Pairing(token, obj.str("deviceId") ?: "", urls)
        } catch (e: Exception) {
            null
        }
    }

    fun save(pairing: Pairing) {
        storage.setString(KEY, buildJsonObject {
            put("token", pairing.token)
            put("deviceId", pairing.deviceId)
            put("serverUrls", buildJsonArray { pairing.serverUrls.forEach { add(JsonPrimitive(it)) } })
        }.toString())
    }

    fun clear() = storage.remove(KEY)

    private companion object {
        const val KEY = "pairing"
    }
}
