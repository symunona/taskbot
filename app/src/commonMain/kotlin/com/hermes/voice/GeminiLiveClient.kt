package com.hermes.voice

import io.ktor.client.*
import io.ktor.client.plugins.websocket.*
import io.ktor.http.*
import io.ktor.websocket.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import kotlinx.serialization.json.*
import kotlin.io.encoding.Base64
import kotlin.io.encoding.ExperimentalEncodingApi

class GeminiLiveClient(private val apiKey: String) {
    private val client = HttpClient {
        install(WebSockets)
    }
    private var session: DefaultClientWebSocketSession? = null
    private val scope = CoroutineScope(Dispatchers.Default + Job())
    
    private val _incomingMessages = MutableSharedFlow<JsonObject>()
    val incomingMessages: SharedFlow<JsonObject> = _incomingMessages.asSharedFlow()

    suspend fun connect(systemInstruction: String? = null, tools: JsonArray? = null) {
        val url = "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1alpha.GenerativeService.BidiGenerateContent?key=$apiKey"
        session = client.webSocketSession(url)
        
        // Initial setup message
        val setupMessage = buildJsonObject {
            put("setup", buildJsonObject {
                put("model", "models/gemini-2.0-flash-exp")
                if (systemInstruction != null) {
                    put("systemInstruction", buildJsonObject {
                        put("parts", buildJsonArray {
                            add(buildJsonObject { put("text", systemInstruction) })
                        })
                    })
                }
                if (tools != null) {
                    put("tools", tools)
                }
            })
        }
        
        sendJson(setupMessage)

        scope.launch {
            try {
                for (frame in session!!.incoming) {
                    if (frame is Frame.Text) {
                        val text = frame.readText()
                        val json = Json.parseToJsonElement(text).jsonObject
                        _incomingMessages.emit(json)
                    }
                }
            } catch (e: Exception) {
                e.printStackTrace()
            }
        }
    }

    @OptIn(ExperimentalEncodingApi::class)
    suspend fun sendAudio(pcmData: ByteArray) {
        val base64Data = Base64.encode(pcmData)
        val msg = buildJsonObject {
            put("realtimeInput", buildJsonObject {
                put("mediaChunks", buildJsonArray {
                    add(buildJsonObject {
                        put("mimeType", "audio/pcm;rate=16000")
                        put("data", base64Data)
                    })
                })
            })
        }
        sendJson(msg)
    }

    suspend fun sendToolResponse(callId: String, name: String, response: JsonObject) {
        val msg = buildJsonObject {
            put("toolResponse", buildJsonObject {
                put("functionResponses", buildJsonArray {
                    add(buildJsonObject {
                        put("id", callId)
                        put("name", name)
                        put("response", response)
                    })
                })
            })
        }
        sendJson(msg)
    }

    private suspend fun sendJson(json: JsonObject) {
        session?.send(Frame.Text(json.toString()))
    }

    fun disconnect() {
        scope.launch {
            session?.close()
            session = null
        }
    }
}
