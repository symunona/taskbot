package com.hermes.tools

import com.hermes.connection.WebSocketClient
import com.hermes.llm.FunctionDeclaration
import kotlinx.serialization.json.*

class RemoteTopicSwitchTool(
    wsClient: WebSocketClient,
    private val onTopicSwitch: suspend (String?) -> Unit
) : RemoteTool(wsClient) {
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
        val payload = buildJsonObject {
            if (summary != null) put("summary", summary)
        }
        val result = sendAndWait("thread.archive_segment", payload)
        onTopicSwitch(summary)
        return result
    }
}

class RemoteEndConversationTool(
    wsClient: WebSocketClient,
    private val onEndConversation: suspend () -> Unit
) : RemoteTool(wsClient) {
    override val declaration = FunctionDeclaration(
        name = "end_conversation",
        description = "Gracefully close + archive thread, clear active context.",
        parameters = buildJsonObject {
            put("type", "object")
            put("properties", buildJsonObject {})
        }
    )

    override suspend fun execute(args: JsonObject): JsonObject {
        val result = sendAndWait("thread.close", buildJsonObject {})
        onEndConversation()
        return result
    }
}
