package com.hermes.llm

import com.hermes.tools.ToolRegistry
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

class MockLlmInterface(
    private val toolRegistry: ToolRegistry
) {
    private val _isGenerating = MutableStateFlow(false)
    val isGenerating: StateFlow<Boolean> = _isGenerating

    suspend fun generateResponseStream(
        userText: String, 
        systemPrompt: String? = null, 
        chatHistory: List<Content> = emptyList(),
        onChunk: (String) -> Unit
    ): String {
        _isGenerating.value = true
        try {
            // Simulate network delay
            delay(500)
            
            var fullResponseText = ""
            
            if (userText.lowercase().contains("tool")) {
                // Simulate a tool call and response
                val toolMsg = "[LLMMOCK] You mentioned a tool. I will simulate calling system_info."
                toolMsg.split(" ").forEach { word ->
                    onChunk("$word ")
                    fullResponseText += "$word "
                    delay(50)
                }
                onChunk("\n")
                fullResponseText += "\n"
                
                // Execute tool
                try {
                    val result = toolRegistry.execute("system_info", buildJsonObject {})
                    val resultMsg = "\n[LLMMOCK] Tool result: $result"
                    resultMsg.split(" ").forEach { word ->
                        onChunk("$word ")
                        fullResponseText += "$word "
                        delay(50)
                    }
                } catch (e: Exception) {
                    val errMsg = "\n[LLMMOCK] Tool error: ${e.message}"
                    onChunk(errMsg)
                    fullResponseText += errMsg
                }
            } else if (userText.lowercase().contains("search")) {
                val toolMsg = "[LLMMOCK] You mentioned search. I will simulate calling search_keyword."
                toolMsg.split(" ").forEach { word ->
                    onChunk("$word ")
                    fullResponseText += "$word "
                    delay(50)
                }
                onChunk("\n")
                fullResponseText += "\n"
                
                // Execute tool
                try {
                    val result = toolRegistry.execute("search_keyword", buildJsonObject {
                        put("keyword", "test")
                    })
                    val resultMsg = "\n[LLMMOCK] Tool result: $result"
                    resultMsg.split(" ").forEach { word ->
                        onChunk("$word ")
                        fullResponseText += "$word "
                        delay(50)
                    }
                } catch (e: Exception) {
                    val errMsg = "\n[LLMMOCK] Tool error: ${e.message}"
                    onChunk(errMsg)
                    fullResponseText += errMsg
                }
            } else {
                // Pre-determined answer
                val msg = "[LLMMOCK] This is a pre-determined response. You said: '$userText'. The weather is sunny and the system is operational."
                msg.split(" ").forEach { word ->
                    onChunk("$word ")
                    fullResponseText += "$word "
                    delay(50)
                }
            }
            
            return fullResponseText
        } finally {
            _isGenerating.value = false
        }
    }
}