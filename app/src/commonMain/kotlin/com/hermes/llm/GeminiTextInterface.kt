package com.hermes.llm

import com.hermes.tools.ToolRegistry
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.serialization.json.JsonObject

class GeminiTextInterface(
    private val apiKey: String,
    private val toolRegistry: ToolRegistry
) {
    private val api = GeminiApi(apiKey)
    private val history = mutableListOf<Content>()
    
    private val _isGenerating = MutableStateFlow(false)
    val isGenerating: StateFlow<Boolean> = _isGenerating

    suspend fun generateResponseStream(userText: String, systemPrompt: String? = null, onChunk: (String) -> Unit): String {
        _isGenerating.value = true
        try {
            history.add(Content("user", listOf(Part(text = userText))))
            
            var fullResponseText = ""
            var keepGoing = true
            
            while (keepGoing) {
                val systemInstruction = systemPrompt?.let { Content("system", listOf(Part(text = it))) }
                val tools = if (toolRegistry.getDeclarations().isNotEmpty()) {
                    listOf(Tool(toolRegistry.getDeclarations()))
                } else null
                
                val request = GeminiRequest(
                    contents = history.toList(),
                    tools = tools,
                    systemInstruction = systemInstruction
                )
                
                var currentChunkText = ""
                var functionCall: FunctionCall? = null
                
                api.streamGenerateContent(request).collect { response ->
                    val candidate = response.candidates?.firstOrNull() ?: return@collect
                    
                    val textPart = candidate.content.parts.find { it.text != null }
                    if (textPart != null) {
                        val chunk = textPart.text ?: ""
                        currentChunkText += chunk
                        fullResponseText += chunk
                        onChunk(chunk)
                    }
                    
                    val fc = candidate.content.parts.find { it.functionCall != null }?.functionCall
                    if (fc != null) {
                        functionCall = fc
                    }
                }
                
                if (currentChunkText.isNotEmpty() || functionCall != null) {
                    val parts = mutableListOf<Part>()
                    if (currentChunkText.isNotEmpty()) parts.add(Part(text = currentChunkText))
                    if (functionCall != null) parts.add(Part(functionCall = functionCall))
                    history.add(Content("model", parts))
                }
                
                if (functionCall != null) {
                    val result = toolRegistry.execute(functionCall!!.name, functionCall!!.args)
                    history.add(Content("user", listOf(Part(
                        functionResponse = FunctionResponse(
                            name = functionCall!!.name,
                            response = result
                        )
                    ))))
                } else {
                    keepGoing = false
                }
            }
            
            return fullResponseText
        } finally {
            _isGenerating.value = false
        }
    }
}
