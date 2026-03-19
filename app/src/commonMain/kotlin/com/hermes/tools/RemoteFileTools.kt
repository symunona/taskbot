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

class RemoteUpdateFileTool(wsClient: WebSocketClient) : RemoteTool(wsClient) {
    override val declaration = FunctionDeclaration(
        name = "update_file",
        description = "Replaces entire file content in the remote vault.",
        parameters = buildJsonObject {
            put("type", "object")
            put("properties", buildJsonObject {
                put("path", buildJsonObject { put("type", "string") })
                put("content", buildJsonObject { put("type", "string") })
            })
            put("required", buildJsonArray { add("path"); add("content") })
        }
    )

    override suspend fun execute(args: JsonObject): JsonObject {
        val path = args["path"]?.jsonPrimitive?.content ?: return buildJsonObject { put("error", "Missing path") }
        val content = args["content"]?.jsonPrimitive?.content ?: return buildJsonObject { put("error", "Missing content") }
        return sendAndWait("kb.update", buildJsonObject { put("path", path); put("content", content) })
    }
}

class RemoteEditFileTool(wsClient: WebSocketClient) : RemoteTool(wsClient) {
    override val declaration = FunctionDeclaration(
        name = "edit_file",
        description = "Line-range edit for a file in the remote vault.",
        parameters = buildJsonObject {
            put("type", "object")
            put("properties", buildJsonObject {
                put("path", buildJsonObject { put("type", "string") })
                put("start_line", buildJsonObject { put("type", "integer") })
                put("end_line", buildJsonObject { put("type", "integer") })
                put("new_content", buildJsonObject { put("type", "string") })
            })
            put("required", buildJsonArray { add("path"); add("start_line"); add("end_line"); add("new_content") })
        }
    )

    override suspend fun execute(args: JsonObject): JsonObject {
        val path = args["path"]?.jsonPrimitive?.content ?: return buildJsonObject { put("error", "Missing path") }
        val startLine = args["start_line"]?.jsonPrimitive?.intOrNull ?: return buildJsonObject { put("error", "Missing start_line") }
        val endLine = args["end_line"]?.jsonPrimitive?.intOrNull ?: return buildJsonObject { put("error", "Missing end_line") }
        val newContent = args["new_content"]?.jsonPrimitive?.content ?: return buildJsonObject { put("error", "Missing new_content") }
        return sendAndWait("kb.edit", buildJsonObject { 
            put("path", path)
            put("start_line", startLine)
            put("end_line", endLine)
            put("new_content", newContent)
        })
    }
}

class RemoteDeleteFileTool(wsClient: WebSocketClient) : RemoteTool(wsClient) {
    override val declaration = FunctionDeclaration(
        name = "delete_file",
        description = "Deletes a file in the remote vault.",
        parameters = buildJsonObject {
            put("type", "object")
            put("properties", buildJsonObject {
                put("path", buildJsonObject { put("type", "string") })
            })
            put("required", buildJsonArray { add("path") })
        }
    )

    override suspend fun execute(args: JsonObject): JsonObject {
        val path = args["path"]?.jsonPrimitive?.content ?: return buildJsonObject { put("error", "Missing path") }
        return sendAndWait("kb.delete", buildJsonObject { put("path", path) })
    }
}

class RemoteRenameFileTool(wsClient: WebSocketClient) : RemoteTool(wsClient) {
    override val declaration = FunctionDeclaration(
        name = "rename_file",
        description = "Renames a file in the remote vault.",
        parameters = buildJsonObject {
            put("type", "object")
            put("properties", buildJsonObject {
                put("path", buildJsonObject { put("type", "string") })
                put("new_path", buildJsonObject { put("type", "string") })
            })
            put("required", buildJsonArray { add("path"); add("new_path") })
        }
    )

    override suspend fun execute(args: JsonObject): JsonObject {
        val path = args["path"]?.jsonPrimitive?.content ?: return buildJsonObject { put("error", "Missing path") }
        val newPath = args["new_path"]?.jsonPrimitive?.content ?: return buildJsonObject { put("error", "Missing new_path") }
        return sendAndWait("kb.rename", buildJsonObject { put("path", path); put("new_path", newPath) })
    }
}

class RemoteMoveFileTool(wsClient: WebSocketClient) : RemoteTool(wsClient) {
    override val declaration = FunctionDeclaration(
        name = "move_file",
        description = "Moves a file to a different folder in the remote vault.",
        parameters = buildJsonObject {
            put("type", "object")
            put("properties", buildJsonObject {
                put("path", buildJsonObject { put("type", "string") })
                put("dest_dir", buildJsonObject { put("type", "string") })
            })
            put("required", buildJsonArray { add("path"); add("dest_dir") })
        }
    )

    override suspend fun execute(args: JsonObject): JsonObject {
        val path = args["path"]?.jsonPrimitive?.content ?: return buildJsonObject { put("error", "Missing path") }
        val destDir = args["dest_dir"]?.jsonPrimitive?.content ?: return buildJsonObject { put("error", "Missing dest_dir") }
        return sendAndWait("kb.move", buildJsonObject { put("path", path); put("dest_dir", destDir) })
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
