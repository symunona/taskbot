package com.hermes.llm

import io.ktor.client.*
import io.ktor.client.request.*
import io.ktor.client.statement.*
import io.ktor.http.*
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.*
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.Flow
import io.ktor.utils.io.readUTF8Line

@Serializable
data class GeminiRequest(
    val contents: List<Content>,
    val tools: List<Tool>? = null,
    val systemInstruction: Content? = null
)

@Serializable
data class Content(
    val role: String,
    val parts: List<Part>
)

@Serializable
data class Part(
    val text: String? = null,
    val functionCall: FunctionCall? = null,
    val functionResponse: FunctionResponse? = null
)

@Serializable
data class FunctionCall(
    val name: String,
    val args: JsonObject
)

@Serializable
data class FunctionResponse(
    val name: String,
    val response: JsonObject
)

@Serializable
data class Tool(
    val functionDeclarations: List<FunctionDeclaration>
)

@Serializable
data class FunctionDeclaration(
    val name: String,
    val description: String,
    val parameters: JsonObject
)

@Serializable
data class GeminiResponse(
    val candidates: List<Candidate>? = null
)

@Serializable
data class Candidate(
    val content: Content
)

class GeminiApi(private val apiKey: String) {
    private val client = HttpClient()
    private val json = Json { ignoreUnknownKeys = true; encodeDefaults = true }

    suspend fun generateContent(request: GeminiRequest): GeminiResponse {
        val response = client.post("https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent?key=$apiKey") {
            contentType(ContentType.Application.Json)
            setBody(json.encodeToString(GeminiRequest.serializer(), request))
        }
        return json.decodeFromString(GeminiResponse.serializer(), response.bodyAsText())
    }

    suspend fun streamGenerateContent(request: GeminiRequest): Flow<GeminiResponse> = flow {
        client.preparePost("https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse&key=$apiKey") {
            contentType(ContentType.Application.Json)
            setBody(json.encodeToString(GeminiRequest.serializer(), request))
        }.execute { response ->
            val channel = response.bodyAsChannel()
            while (!channel.isClosedForRead) {
                val line = channel.readUTF8Line() ?: break
                if (line.startsWith("data: ")) {
                    val data = line.substring(6)
                    if (data.trim() == "[DONE]") continue
                    try {
                        val parsed = json.decodeFromString(GeminiResponse.serializer(), data)
                        emit(parsed)
                    } catch (e: Exception) {
                        // ignore parsing errors for partial chunks
                    }
                }
            }
        }
    }
}
