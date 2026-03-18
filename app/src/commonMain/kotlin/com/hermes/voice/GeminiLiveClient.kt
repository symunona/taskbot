package com.hermes.voice

import com.hermes.ui.EventLogger
import io.ktor.client.*
import io.ktor.client.plugins.HttpTimeout
import io.ktor.client.plugins.websocket.*
import io.ktor.websocket.*
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlin.io.encoding.Base64
import kotlin.io.encoding.ExperimentalEncodingApi

class GeminiLiveClient(private val apiKey: String) {
    private val client = HttpClient {
        install(HttpTimeout) {
            requestTimeoutMillis = HttpTimeout.INFINITE_TIMEOUT_MS
            connectTimeoutMillis = 10_000L
            socketTimeoutMillis = HttpTimeout.INFINITE_TIMEOUT_MS
        }
        install(WebSockets) {
            pingInterval = 20_000L
        }
    }

    private var session: DefaultClientWebSocketSession? = null
    private var receiveJob: Job? = null
    private val scope = CoroutineScope(Dispatchers.Default + SupervisorJob())
    private val json = Json { ignoreUnknownKeys = true }
    private var setupComplete = CompletableDeferred<Unit>()

    private val _incomingMessages = MutableSharedFlow<JsonObject>(extraBufferCapacity = 64)
    val incomingMessages: SharedFlow<JsonObject> = _incomingMessages

    suspend fun connect(systemInstruction: String? = null, tools: JsonArray? = null) {
        val url = "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1alpha.GenerativeService.BidiGenerateContent?key=$apiKey"
        EventLogger.log("Connecting Gemini Live session")
        session = client.webSocketSession(url)
        setupComplete = CompletableDeferred()

        receiveJob?.cancel()
        receiveJob = scope.launch {
            try {
                for (frame in session!!.incoming) {
                    if (frame !is Frame.Text) {
                        EventLogger.log("Gemini Live non-text frame received: ${frame.frameType}")
                        continue
                    }
                    val payload = frame.readText()
                    EventLogger.log("Gemini Live raw message: $payload")
                    val message = json.parseToJsonElement(payload).jsonObject
                    when {
                        message["setupComplete"] != null -> {
                            EventLogger.log("Gemini Live session configured")
                            if (!setupComplete.isCompleted) {
                                setupComplete.complete(Unit)
                            }
                        }
                        message["goAway"] != null -> {
                            val reason = message["goAway"]?.jsonObject
                                ?.get("reason")
                                ?.jsonPrimitive
                                ?.contentOrNull
                                ?: "Server requested disconnect"
                            EventLogger.log("Gemini Live goAway: $reason", isError = true)
                        }
                        message["toolCallCancellation"] != null -> {
                            EventLogger.log("Gemini Live cancelled pending tool call")
                        }
                        else -> {
                            EventLogger.log("Gemini Live message received")
                        }
                    }
                    _incomingMessages.emit(message)
                }
                val reason = session?.closeReason?.await()
                EventLogger.log("Gemini Live incoming channel closed normally. Reason: $reason")
            } catch (e: Exception) {
                if (!setupComplete.isCompleted) {
                    setupComplete.completeExceptionally(e)
                }
                EventLogger.log("Gemini Live receive failed: ${e.message ?: e}", isError = true)
            }
        }

        val setupMessage = buildJsonObject {
            put("setup", buildJsonObject {
                put("model", "models/gemini-2.0-flash-exp")
                put("generationConfig", buildJsonObject {
                    put("responseModalities", buildJsonArray { add(JsonPrimitive("AUDIO")) })
                })
                if (systemInstruction != null) {
                    put("systemInstruction", buildJsonObject {
                        put("parts", buildJsonArray {
                            add(buildJsonObject { put("text", systemInstruction) })
                        })
                    })
                }
                if (tools != null && tools.isNotEmpty()) {
                    put("tools", buildJsonArray {
                        add(buildJsonObject {
                            put("functionDeclarations", tools)
                        })
                    })
                }
            })
        }

        sendJson(setupMessage)
        withTimeout(10_000L) {
            setupComplete.await()
        }
    }

    @OptIn(ExperimentalEncodingApi::class)
    suspend fun sendAudio(pcmData: ByteArray) {
        val base64Data = Base64.encode(pcmData)
        sendJson(
            buildJsonObject {
                put("realtimeInput", buildJsonObject {
                    put("audio", buildJsonObject {
                        put("data", base64Data)
                        put("mimeType", "audio/pcm;rate=16000")
                    })
                })
            }
        )
    }

    suspend fun endAudioStream() {
        sendJson(
            buildJsonObject {
                put("realtimeInput", buildJsonObject {
                    put("audioStreamEnd", true)
                })
            }
        )
    }

    suspend fun sendToolResponse(functionResponses: JsonArray) {
        sendJson(
            buildJsonObject {
                put("toolResponse", buildJsonObject {
                    put("functionResponses", functionResponses)
                })
            }
        )
    }

    private suspend fun sendJson(jsonObject: JsonObject) {
        session?.send(Frame.Text(jsonObject.toString()))
    }

    fun disconnect(sendAudioStreamEnd: Boolean = false) {
        scope.launch {
            try {
                if (sendAudioStreamEnd) {
                    endAudioStream()
                }
            } catch (_: Exception) {
            }
            session?.close()
            session = null
            receiveJob?.cancel()
            scope.cancel()
        }
    }
}
