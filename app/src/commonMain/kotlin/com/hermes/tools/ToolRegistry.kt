package com.hermes.tools

import com.hermes.llm.FunctionDeclaration
import kotlinx.serialization.json.*

abstract class Tool {
    abstract val declaration: FunctionDeclaration
    abstract suspend fun execute(args: JsonObject): JsonObject
}

class ToolRegistry {
    private val tools = mutableMapOf<String, Tool>()

    fun register(tool: Tool) {
        tools[tool.declaration.name] = tool
    }

    fun getDeclarations(): List<FunctionDeclaration> = tools.values.map { it.declaration }

    suspend fun execute(name: String, args: JsonObject): JsonObject {
        val tool = tools[name] ?: return buildJsonObject { put("error", "Tool $name not found") }
        return try {
            tool.execute(args)
        } catch (e: Exception) {
            buildJsonObject { put("error", e.message ?: "Unknown error") }
        }
    }
}
