package com.hermes.connection

import com.hermes.ui.EventLogger
import io.ktor.client.*
import io.ktor.client.plugins.websocket.*
import io.ktor.http.*
import io.ktor.websocket.*
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.first
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.channels.ClosedReceiveChannelException
import io.ktor.client.plugins.HttpTimeout
import io.ktor.client.plugins.websocket.*

class WebSocketClient {
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
    
    private val _events = MutableSharedFlow<JsonObject>(replay = 100, extraBufferCapacity = 100)
    val events: SharedFlow<JsonObject> = _events
    
    private val _connectionState = MutableStateFlow<ConnectionState>(ConnectionState.Disconnected)
    val connectionState: StateFlow<ConnectionState> = _connectionState
    
    var isManualDisconnect = false
        private set
    
    enum class ConnectionState {
        Disconnected, Connecting, Connected, Error
    }
    
    private val json = Json { ignoreUnknownKeys = true }

    suspend fun connect(url: String, token: String) {
        isManualDisconnect = false
        try {
            _connectionState.value = ConnectionState.Connecting
            EventLogger.log("Connecting to WebSocket: $url")
            session = client.webSocketSession(url)
            
            // Send auth
            EventLogger.log("Sending authentication token")
            val authMsg = """{"event":"auth.token","payload":{"token":"$token"}}"""
            session?.send(Frame.Text(authMsg))
            
            // Listen for messages
            for (frame in session?.incoming!!) {
                if (frame is Frame.Text) {
                    val text = frame.readText()
                    if (text == "Auth OK") {
                        EventLogger.log("Authenticated successfully")
                        println("Authenticated successfully")
                        _connectionState.value = ConnectionState.Connected
                    } else if (text == "Auth failed") {
                        EventLogger.log("Authentication failed", isError = true)
                        println("Authentication failed")
                        _connectionState.value = ConnectionState.Error
                        session?.close()
                        break
                    } else {
                        try {
                            val event = json.parseToJsonElement(text).jsonObject
                            EventLogger.log("Received event: ${event["event"]?.jsonPrimitive?.content ?: "unknown"}")
                            _events.emit(event)
                        } catch (e: Exception) {
                            EventLogger.log("Failed to parse event: $text", isError = true)
                            println("Failed to parse event: $text")
                        }
                    }
                }
            }
            EventLogger.log("WebSocket connection closed")
            if (_connectionState.value != ConnectionState.Error) {
                _connectionState.value = ConnectionState.Disconnected
            }
        } catch (e: ClosedReceiveChannelException) {
            EventLogger.log("WebSocket connection closed normally")
            println("WebSocket connection closed normally")
            _connectionState.value = ConnectionState.Disconnected
        } catch (e: io.ktor.client.plugins.websocket.WebSocketException) {
            val errorMsg = e.message ?: e.toString()
            EventLogger.log("WebSocket error: $errorMsg", isError = true)
            println("WebSocket error: $errorMsg")
            _connectionState.value = ConnectionState.Disconnected
        } catch (e: Exception) {
            val errorMsg = e.message ?: e.toString()
            EventLogger.log("WebSocket connection error: $errorMsg", isError = true)
            println("WebSocket connection error: $errorMsg")
            e.printStackTrace()
            _connectionState.value = ConnectionState.Error
        }
    }

    suspend fun send(event: JsonObject) {
        EventLogger.log("Sending event: ${event["event"]?.jsonPrimitive?.content ?: "unknown"}")
        session?.send(Frame.Text(event.toString()))
    }
    
    suspend fun sendAndWait(event: JsonObject, timeoutMillis: Long = 5000): JsonObject? {
        val reqId = event["id"]?.jsonPrimitive?.content ?: return null
        send(event)
        return kotlinx.coroutines.withTimeoutOrNull(timeoutMillis) {
            events.first { it["ref_id"]?.jsonPrimitive?.content == reqId }
        }
    }
    
    suspend fun disconnect() {
        isManualDisconnect = true
        EventLogger.log("Disconnecting WebSocket")
        session?.close()
        session = null
    }
}
