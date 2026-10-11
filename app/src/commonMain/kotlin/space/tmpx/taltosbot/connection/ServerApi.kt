package space.tmpx.taltosbot.connection

import io.ktor.client.HttpClient
import io.ktor.client.plugins.HttpTimeout
import io.ktor.client.plugins.timeout
import io.ktor.client.request.HttpRequestBuilder
import io.ktor.client.request.get
import io.ktor.client.request.header
import io.ktor.client.request.post
import io.ktor.client.request.setBody
import io.ktor.client.statement.HttpResponse
import io.ktor.client.statement.bodyAsText
import io.ktor.http.ContentType
import io.ktor.http.HttpHeaders
import io.ktor.http.contentType
import kotlinx.coroutines.CancellationException
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.put

class HttpStatusException(val status: Int, message: String) : Exception(message)

/** [model] = Live (voice), [textModel] = `generateContent` (text mode). */
data class GeminiConfig(val apiKey: String, val model: String, val textModel: String)

/** The HTTP half of docs/protocol.md. */
class ServerApi {
    private val client = HttpClient {
        expectSuccess = false
        install(HttpTimeout) {
            requestTimeoutMillis = 10_000L
            connectTimeoutMillis = 5_000L
        }
    }

    private suspend fun HttpResponse.jsonOrThrow(): JsonObject {
        val text = bodyAsText()
        val obj = try {
            Json.parseToJsonElement(text).jsonObject
        } catch (e: Exception) {
            null
        }
        if (status.value !in 200..299) {
            throw HttpStatusException(status.value, obj?.str("error") ?: "HTTP ${status.value}")
        }
        return obj ?: throw IllegalStateException("Server sent a non-JSON reply")
    }

    private fun HttpRequestBuilder.bearer(token: String) = header(HttpHeaders.Authorization, "Bearer $token")

    /**
     * Claims the pairing code. Tries the connect string's URLs in order; the one that answers
     * becomes the primary stored URL.
     */
    suspend fun pair(connect: ConnectString, deviceName: String): Pairing {
        var lastError: Throwable? = null
        for (url in connect.urls) {
            try {
                val reply = client.post("$url/api/v1/pair") {
                    contentType(ContentType.Application.Json)
                    setBody(buildJsonObject {
                        put("code", connect.code)
                        put("name", deviceName)
                    }.toString())
                }.jsonOrThrow()
                val token = reply.str("device_token")
                    ?: throw IllegalStateException("Server reply has no device_token")
                val urls = (listOf(url) + connect.urls + reply.strList("server_urls").map(::normalizeServerUrl)).distinct()
                return Pairing(token, reply.str("device_id") ?: "", urls)
            } catch (e: CancellationException) {
                throw e
            } catch (e: HttpStatusException) {
                // The server answered: the code is wrong/expired/rate-limited. Other URLs are the same server.
                throw e
            } catch (e: Throwable) {
                lastError = e
            }
        }
        throw IllegalStateException("No server reachable (${lastError?.message ?: "unknown error"})")
    }

    /** Probe. Throws [HttpStatusException] (401 = token revoked) or a network error. */
    suspend fun me(url: String, token: String, timeoutMs: Long = 5_000L): JsonObject =
        client.get("$url/api/v1/me") {
            bearer(token)
            timeout {
                requestTimeoutMillis = timeoutMs
                connectTimeoutMillis = timeoutMs
            }
        }.jsonOrThrow()

    suspend fun config(url: String, token: String): GeminiConfig {
        val reply = client.get("$url/api/v1/config") { bearer(token) }.jsonOrThrow()
        return GeminiConfig(
            apiKey = reply.str("gemini_api_key") ?: "",
            model = reply.str("gemini_model") ?: "",
            textModel = reply.str("gemini_text_model") ?: ""
        )
    }

    /** APK sidecar; plain GET of an arbitrary JSON document. */
    suspend fun getJson(url: String): JsonObject = client.get(url).jsonOrThrow()
}
