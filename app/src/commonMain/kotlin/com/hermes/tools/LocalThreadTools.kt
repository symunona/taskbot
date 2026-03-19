package com.hermes.tools

import com.hermes.llm.FunctionDeclaration
import kotlinx.serialization.json.*

class LocalTopicSwitchTool(private val onTopicSwitch: suspend (String?) -> Unit) : Tool() {
    override val declaration = FunctionDeclaration(
        name = "topic_switch",
        description = "Summarize + archive current segment, start fresh sub-topic.",
        parameters = buildJsonObject {
            put("type", "object")
            put("properties", buildJsonObject {
                put("summary", buildJsonObject { put("type", "string") })
            })
        }
    )

    override suspend fun execute(args: JsonObject): JsonObject {
        val summary = args["summary"]?.jsonPrimitive?.content
        onTopicSwitch(summary)
        return buildJsonObject { put("status", "success") }
    }
}

class LocalEndConversationTool(private val onEndConversation: suspend () -> Unit) : Tool() {
    override val declaration = FunctionDeclaration(
        name = "end_conversation",
        description = "Gracefully close + archive thread, clear active context.",
        parameters = buildJsonObject {
            put("type", "object")
            put("properties", buildJsonObject {})
        }
    )

    override suspend fun execute(args: JsonObject): JsonObject {
        onEndConversation()
        return buildJsonObject { put("status", "success") }
    }
}
