package space.tmpx.taltosbot.llm

import io.ktor.client.HttpClient
import io.ktor.client.plugins.HttpTimeout
import io.ktor.client.request.header
import io.ktor.client.request.post
import io.ktor.client.request.setBody
import io.ktor.client.statement.bodyAsText
import io.ktor.http.ContentType
import io.ktor.http.contentType
import kotlinx.coroutines.CancellationException
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put

const val GEMINI_BASE_URL = "https://generativelanguage.googleapis.com"

class GeminiTextException(message: String) : Exception(message)

/** `usageMetadata` of one model call. [output] = everything that is not prompt (answer + thinking). */
data class CallUsage(val input: Long, val output: Long)

/** Works for `generateContent` replies and Live `usageMetadata` messages. Null = no counts in it. */
internal fun parseCallUsage(meta: JsonObject?): CallUsage? {
    val input = (meta?.get("promptTokenCount") as? JsonPrimitive)?.longOrNull ?: return null
    val total = (meta["totalTokenCount"] as? JsonPrimitive)?.longOrNull ?: input
    return CallUsage(input, (total - input).coerceAtLeast(0))
}

/**
 * Text mode: Gemini non-interactive `generateContent` (docs/protocol.md "App rules").
 * The key travels in the `x-goog-api-key` header only; it is never put in a URL and never logged.
 */
class GeminiTextClient(private val baseUrl: String = GEMINI_BASE_URL) {
    private val client = HttpClient {
        expectSuccess = false
        install(HttpTimeout) {
            requestTimeoutMillis = 90_000L
            connectTimeoutMillis = 10_000L
        }
    }

    /** One `generateContent` call. Returns the first candidate's `content`, untouched. */
    suspend fun generate(
        apiKey: String,
        model: String,
        systemInstruction: String,
        tools: JsonArray,
        contents: JsonArray,
        onUsage: (CallUsage) -> Unit = {}
    ): JsonObject {
        val modelPath = if (model.startsWith("models/")) model else "models/$model"
        val body = buildJsonObject {
            put("systemInstruction", buildJsonObject {
                put("parts", buildJsonArray { add(buildJsonObject { put("text", systemInstruction) }) })
            })
            put("contents", contents)
            if (tools.isNotEmpty()) {
                put("tools", buildJsonArray { add(buildJsonObject { put("functionDeclarations", tools) }) })
            }
        }
        val (status, text) = try {
            val response = client.post("$baseUrl/v1beta/$modelPath:generateContent") {
                header("x-goog-api-key", apiKey)
                contentType(ContentType.Application.Json)
                setBody(body.toString())
            }
            response.status.value to response.bodyAsText()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Throwable) {
            throw GeminiTextException("Gemini unreachable (${redact(e.message ?: e.toString(), apiKey)})")
        }
        val reply = try {
            Json.parseToJsonElement(text).jsonObject
        } catch (e: Exception) {
            throw GeminiTextException("Gemini sent a non-JSON reply (HTTP $status)")
        }
        (reply["error"] as? JsonObject)?.let {
            throw GeminiTextException(redact("Gemini error: ${it.string("message") ?: "HTTP $status"}", apiKey))
        }
        if (status !in 200..299) throw GeminiTextException("Gemini error: HTTP $status")
        parseCallUsage(reply["usageMetadata"] as? JsonObject)?.let(onUsage)

        val candidate = (reply["candidates"] as? JsonArray)?.firstOrNull() as? JsonObject
        if (candidate == null) {
            val block = (reply["promptFeedback"] as? JsonObject)?.string("blockReason")
            throw GeminiTextException("Gemini gave no answer" + (block?.let { " (blocked: $it)" } ?: ""))
        }
        val content = candidate["content"] as? JsonObject
        if (content == null || (content["parts"] as? JsonArray).isNullOrEmpty()) {
            throw GeminiTextException("Gemini gave an empty answer (${candidate.string("finishReason") ?: "no finish reason"})")
        }
        return content
    }

    private fun redact(text: String, apiKey: String) = if (apiKey.isBlank()) text else text.replace(apiKey, "***")
}

private fun JsonObject.string(key: String): String? = (this[key] as? JsonPrimitive)?.contentOrNull

/** `functionCall` objects of a model `content`, in order. */
internal fun functionCallsOf(content: JsonObject): List<JsonObject> =
    (content["parts"] as? JsonArray).orEmpty().mapNotNull { (it as? JsonObject)?.get("functionCall") as? JsonObject }

/** Visible text of a model `content`: text parts joined, thought parts left out. */
internal fun textOf(content: JsonObject): String =
    (content["parts"] as? JsonArray).orEmpty().mapNotNull { it as? JsonObject }
        .filter { (it["thought"] as? JsonPrimitive)?.booleanOrNull != true }
        .mapNotNull { it.string("text") }
        .joinToString("")
        .trim()

internal fun textContent(role: String, text: String): JsonObject = buildJsonObject {
    put("role", role)
    put("parts", buildJsonArray { add(buildJsonObject { put("text", text) }) })
}

/**
 * One text turn: call Gemini, run the tools it asks for, repeat until it answers in text.
 *
 * [history] ends with the user's new message. Inside the loop the model's own `content` goes
 * back unmodified, so `thoughtSignature` parts survive (thinking models reject the follow-up
 * otherwise). Tool calls run one after another, in the order the model listed them.
 * [onUsage] hears the token counts of every call of the turn.
 */
suspend fun runTextTurn(
    client: GeminiTextClient,
    apiKey: String,
    model: String,
    systemInstruction: String,
    tools: JsonArray,
    history: List<JsonObject>,
    onUsage: (CallUsage) -> Unit = {},
    executeTool: suspend (name: String, args: JsonObject) -> JsonObject
): String {
    val contents = history.toMutableList<JsonElement>()
    repeat(MAX_TOOL_ROUNDS) {
        val content = client.generate(apiKey, model, systemInstruction, tools, JsonArray(contents), onUsage)
        val calls = functionCallsOf(content)
        if (calls.isEmpty()) {
            return textOf(content).ifBlank { throw GeminiTextException("Gemini gave an empty answer") }
        }
        contents += content
        contents += buildJsonObject {
            put("role", "user")
            put("parts", buildJsonArray {
                for (call in calls) {
                    val name = call.string("name") ?: continue
                    val result = executeTool(name, call["args"] as? JsonObject ?: JsonObject(emptyMap()))
                    add(buildJsonObject {
                        put("functionResponse", buildJsonObject {
                            call.string("id")?.let { put("id", it) }
                            put("name", name)
                            put("response", result)
                        })
                    })
                }
            })
        }
    }
    throw GeminiTextException("Gemini kept calling tools ($MAX_TOOL_ROUNDS rounds), gave up")
}

private const val MAX_TOOL_ROUNDS = 12
