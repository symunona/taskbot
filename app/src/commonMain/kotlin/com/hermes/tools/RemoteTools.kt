package com.hermes.tools

import com.hermes.connection.WebSocketClient
import com.hermes.llm.FunctionDeclaration
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.json.*
import kotlin.time.Duration.Companion.seconds

abstract class RemoteTool(private val wsClient: WebSocketClient) : Tool() {
    protected suspend fun sendAndWait(event: String, payload: JsonObject): JsonObject {
        val id = "req_${kotlin.random.Random.nextInt()}"
        val envelope = buildJsonObject {
            put("event", event)
            put("id", id)
            put("ts", 0) // We don't really care on frontend
            put("payload", payload)
        }
        
        wsClient.send(envelope)
        
        // Wait for response with matching ref_id
        val response = withTimeoutOrNull(5.seconds) {
            wsClient.events.first { it["ref_id"]?.jsonPrimitive?.content == id }
        }
        
        return response?.get("payload")?.jsonObject ?: buildJsonObject { put("error", "Timeout waiting for server response") }
    }
}

class RemoteReadFileTool(wsClient: WebSocketClient) : RemoteTool(wsClient) {
    override val declaration = FunctionDeclaration(
        name = "read_file",
        description = "Reads the content of a file from the remote vault.",
        parameters = buildJsonObject {
            put("type", "object")
            put("properties", buildJsonObject {
                put("path", buildJsonObject {
                    put("type", "string")
                    put("description", "The path to the file to read.")
                })
            })
            put("required", buildJsonArray { add("path") })
        }
    )

    override suspend fun execute(args: JsonObject): JsonObject {
        val path = args["path"]?.jsonPrimitive?.content ?: return buildJsonObject { put("error", "Missing path") }
        return sendAndWait("kb.get", buildJsonObject { put("path", path) })
    }
}

class RemoteCreateFileTool(wsClient: WebSocketClient) : RemoteTool(wsClient) {
    override val declaration = FunctionDeclaration(
        name = "create_file",
        description = "Creates a new file or overwrites an existing one in the remote vault.",
        parameters = buildJsonObject {
            put("type", "object")
            put("properties", buildJsonObject {
                put("path", buildJsonObject {
                    put("type", "string")
                    put("description", "The path where to create the file.")
                })
                put("content", buildJsonObject {
                    put("type", "string")
                    put("description", "The content to write to the file.")
                })
            })
            put("required", buildJsonArray { add("path"); add("content") })
        }
    )

    override suspend fun execute(args: JsonObject): JsonObject {
        val path = args["path"]?.jsonPrimitive?.content ?: return buildJsonObject { put("error", "Missing path") }
        val content = args["content"]?.jsonPrimitive?.content ?: return buildJsonObject { put("error", "Missing content") }
        return sendAndWait("kb.create", buildJsonObject { put("path", path); put("content", content) })
    }
}

class RemoteSearchKeywordTool(wsClient: WebSocketClient) : RemoteTool(wsClient) {
    override val declaration = FunctionDeclaration(
        name = "search_keyword",
        description = "Searches for a keyword in all files in the remote vault.",
        parameters = buildJsonObject {
            put("type", "object")
            put("properties", buildJsonObject {
                put("query", buildJsonObject {
                    put("type", "string")
                    put("description", "The keyword to search for.")
                })
            })
            put("required", buildJsonArray { add("query") })
        }
    )

    override suspend fun execute(args: JsonObject): JsonObject {
        val query = args["query"]?.jsonPrimitive?.content ?: return buildJsonObject { put("error", "Missing query") }
        return sendAndWait("kb.search", buildJsonObject { put("query", query) })
    }
}

class RemoteSystemInfoTool(wsClient: WebSocketClient) : RemoteTool(wsClient) {
    override val declaration = FunctionDeclaration(
        name = "system_info",
        description = "Returns information about the remote system.",
        parameters = buildJsonObject {
            put("type", "object")
            put("properties", buildJsonObject {})
        }
    )

    override suspend fun execute(args: JsonObject): JsonObject {
        return sendAndWait("system.info", buildJsonObject {})
    }
}
