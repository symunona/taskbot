package com.hermes.history

import com.hermes.connection.WebSocketClient
import com.hermes.ui.ChatMessage
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.debounce
import kotlinx.serialization.json.*

data class HistoryItem(
    val threadId: String,
    val transcript: String,
    val summary: String,
    val isClosed: Boolean = false
)

class SyncQueue(
    private val webSocketClient: WebSocketClient,
    private val scope: CoroutineScope
) {
    private val pendingItems = mutableMapOf<String, HistoryItem>()
    private val triggerFlow = MutableStateFlow(0)
    private var isFlushing = false

    init {
        scope.launch {
            triggerFlow
                .debounce(60_000L) // 60 seconds trailing debounce
                .collectLatest { 
                    if (it > 0) {
                        flush()
                    }
                }
        }
    }

    fun enqueue(threadId: String, messages: List<ChatMessage>, summary: String, isClosed: Boolean = false) {
        val transcript = HistorySerializer.serialize(messages)
        pendingItems[threadId] = HistoryItem(threadId, transcript, summary, isClosed)
        triggerFlow.value += 1
    }

    suspend fun flush() {
        if (pendingItems.isEmpty() || isFlushing) return
        
        isFlushing = true
        val itemsToSync = pendingItems.values.toList()
        pendingItems.clear()
        
        if (webSocketClient.connectionState.value == WebSocketClient.ConnectionState.Connected) {
            val payload = buildJsonObject {
                put("items", buildJsonArray {
                    itemsToSync.forEach { item ->
                        add(buildJsonObject {
                            put("thread_id", item.threadId)
                            put("transcript", item.transcript)
                            put("summary", item.summary)
                            put("is_closed", item.isClosed)
                        })
                    }
                })
            }
            
            webSocketClient.send(buildJsonObject {
                put("event", "history.save_batch")
                put("id", "req_${kotlin.random.Random.nextInt()}")
                put("ts", 0)
                put("payload", payload)
            })
        } else {
            // If not connected, put them back
            itemsToSync.forEach { item ->
                if (!pendingItems.containsKey(item.threadId)) {
                    pendingItems[item.threadId] = item
                }
            }
        }
        
        isFlushing = false
    }
}
