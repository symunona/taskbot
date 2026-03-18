package com.hermes.connection

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject
import io.ktor.util.decodeBase64String

class ConnectionManager {
    val webSocketClient = WebSocketClient()
    
    private var currentUrls: List<String> = emptyList()
    private var currentUrlIndex = 0
    private var currentToken: String? = null
    private var isPairingFlow: Boolean = false
    private var retryCount = 0
    private val maxRetries = 5
    private var connectJob: kotlinx.coroutines.Job? = null
    
    // Parses: hermes://<pubkey_base64>/<token>?addrs=<ip>:<port>&addrs=<domain>:443
    fun connectFromString(connectionString: String, clientToken: String? = null) {
        try {
            if (!connectionString.startsWith("hermes://")) {
                println("Invalid connection string format")
                return
            }
            
            val withoutScheme = connectionString.substring(9)
            val parts = withoutScheme.split("?")
            if (parts.size != 2) return
            
            val pathParts = parts[0].split("/")
            if (pathParts.size != 2) return
            
            val pubkeyBase64 = pathParts[0]
            val pairingToken = pathParts[1]
            
            val queryParts = parts[1].split("&")
            val addrs = mutableListOf<String>()
            for (query in queryParts) {
                if (query.startsWith("addrs=")) {
                    addrs.add(query.substring(6))
                }
            }
            
            if (addrs.isNotEmpty()) {
                currentUrls = addrs.map { addr ->
                    if (addr.endsWith(":443")) "wss://$addr" else "ws://$addr"
                }
                currentUrlIndex = 0
                
                if (!clientToken.isNullOrEmpty()) {
                    currentToken = clientToken
                    isPairingFlow = false
                } else {
                    currentToken = pairingToken
                    isPairingFlow = true
                }
                
                retryCount = 0
                
                connectJob?.cancel()
                connectJob = CoroutineScope(Dispatchers.Default).launch {
                    connectWithRetry()
                }
            }
        } catch (e: Exception) {
            println("Failed to parse connection string: ${e.message}")
        }
    }
    
    private suspend fun connectWithRetry() {
        if (currentUrls.isEmpty()) return
        val token = currentToken ?: return
        
        val stateObserverJob = CoroutineScope(Dispatchers.Default).launch {
            webSocketClient.connectionState.collect { state ->
                if (state == WebSocketClient.ConnectionState.Connected) {
                    retryCount = 0
                }
            }
        }
        
        try {
            while (retryCount < maxRetries) {
                val url = currentUrls[currentUrlIndex]
                webSocketClient.connect(url, token, isPairingFlow)
                
                // If we get here, the connection closed or failed
                if (webSocketClient.isManualDisconnect) {
                    break
                }
                
                retryCount++
                if (retryCount < maxRetries) {
                    currentUrlIndex = (currentUrlIndex + 1) % currentUrls.size
                    val nextUrl = currentUrls[currentUrlIndex]
                    com.hermes.ui.EventLogger.log("Connection failed. Trying next server: $nextUrl ($retryCount/$maxRetries) in 2s...", isError = true)
                    kotlinx.coroutines.delay(2000)
                }
            }
            
            if (retryCount >= maxRetries && !webSocketClient.isManualDisconnect) {
                com.hermes.ui.EventLogger.log("Max retries reached across all servers. Please reconnect manually.", isError = true)
            }
        } finally {
            stateObserverJob.cancel()
        }
    }
}
