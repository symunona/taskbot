package com.hermes.tools

import com.hermes.connection.WebSocketClient
import com.hermes.llm.FunctionDeclaration
import kotlinx.serialization.json.*

class RemoteListDirectoryTool(wsClient: WebSocketClient) : RemoteTool(wsClient) {
    override val declaration = FunctionDeclaration(
        name = "list_directory",
        description = "List directory contents (shallow) in the remote vault.",
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
        return sendAndWait("kb.list", buildJsonObject { put("path", path) })
    }
}

class RemoteListVaultFilesTool(wsClient: WebSocketClient) : RemoteTool(wsClient) {
    override val declaration = FunctionDeclaration(
        name = "list_vault_files",
        description = "List all vault files in the remote vault.",
        parameters = buildJsonObject {
            put("type", "object")
            put("properties", buildJsonObject {
                put("filter", buildJsonObject { put("type", "string") })
            })
        }
    )

    override suspend fun execute(args: JsonObject): JsonObject {
        val filter = args["filter"]?.jsonPrimitive?.content
        val payload = if (filter != null) buildJsonObject { put("filter", filter) } else buildJsonObject {}
        return sendAndWait("kb.list_all", payload)
    }
}

class RemoteGetFolderTreeTool(wsClient: WebSocketClient) : RemoteTool(wsClient) {
    override val declaration = FunctionDeclaration(
        name = "get_folder_tree",
        description = "Recursive folder tree in the remote vault.",
        parameters = buildJsonObject {
            put("type", "object")
            put("properties", buildJsonObject {})
        }
    )

    override suspend fun execute(args: JsonObject): JsonObject {
        return sendAndWait("kb.tree", buildJsonObject {})
    }
}

class RemoteCreateDirectoryTool(wsClient: WebSocketClient) : RemoteTool(wsClient) {
    override val declaration = FunctionDeclaration(
        name = "create_directory",
        description = "Create directory in the remote vault.",
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
        return sendAndWait("kb.mkdir", buildJsonObject { put("path", path) })
    }
}

class RemoteSearchRegexpTool(wsClient: WebSocketClient) : RemoteTool(wsClient) {
    override val declaration = FunctionDeclaration(
        name = "search_regexp",
        description = "Regex search across remote vault.",
        parameters = buildJsonObject {
            put("type", "object")
            put("properties", buildJsonObject {
                put("pattern", buildJsonObject { put("type", "string") })
            })
            put("required", buildJsonArray { add("pattern") })
        }
    )

    override suspend fun execute(args: JsonObject): JsonObject {
        val pattern = args["pattern"]?.jsonPrimitive?.content ?: return buildJsonObject { put("error", "Missing pattern") }
        return sendAndWait("kb.search_re", buildJsonObject { put("pattern", pattern) })
    }
}

class RemoteSearchReplaceFileTool(wsClient: WebSocketClient) : RemoteTool(wsClient) {
    override val declaration = FunctionDeclaration(
        name = "search_replace_file",
        description = "Search+replace in one file in the remote vault.",
        parameters = buildJsonObject {
            put("type", "object")
            put("properties", buildJsonObject {
                put("path", buildJsonObject { put("type", "string") })
                put("search", buildJsonObject { put("type", "string") })
                put("replace", buildJsonObject { put("type", "string") })
            })
            put("required", buildJsonArray { add("path"); add("search"); add("replace") })
        }
    )

    override suspend fun execute(args: JsonObject): JsonObject {
        val path = args["path"]?.jsonPrimitive?.content ?: return buildJsonObject { put("error", "Missing path") }
        val search = args["search"]?.jsonPrimitive?.content ?: return buildJsonObject { put("error", "Missing search") }
        val replace = args["replace"]?.jsonPrimitive?.content ?: return buildJsonObject { put("error", "Missing replace") }
        return sendAndWait("kb.sr_file", buildJsonObject { put("path", path); put("search", search); put("replace", replace) })
    }
}

class RemoteSearchReplaceGlobalTool(wsClient: WebSocketClient) : RemoteTool(wsClient) {
    override val declaration = FunctionDeclaration(
        name = "search_replace_global",
        description = "Search+replace across remote vault.",
        parameters = buildJsonObject {
            put("type", "object")
            put("properties", buildJsonObject {
                put("search", buildJsonObject { put("type", "string") })
                put("replace", buildJsonObject { put("type", "string") })
            })
            put("required", buildJsonArray { add("search"); add("replace") })
        }
    )

    override suspend fun execute(args: JsonObject): JsonObject {
        val search = args["search"]?.jsonPrimitive?.content ?: return buildJsonObject { put("error", "Missing search") }
        val replace = args["replace"]?.jsonPrimitive?.content ?: return buildJsonObject { put("error", "Missing replace") }
        return sendAndWait("kb.sr_global", buildJsonObject { put("search", search); put("replace", replace) })
    }
}

class RemoteReadNotesTool(wsClient: WebSocketClient) : RemoteTool(wsClient) {
    override val declaration = FunctionDeclaration(
        name = "read_notes",
        description = "Read array of paths in the remote vault.",
        parameters = buildJsonObject {
            put("type", "object")
            put("properties", buildJsonObject {
                put("paths", buildJsonObject {
                    put("type", "array")
                    put("items", buildJsonObject { put("type", "string") })
                })
            })
            put("required", buildJsonArray { add("paths") })
        }
    )

    override suspend fun execute(args: JsonObject): JsonObject {
        val paths = args["paths"]?.jsonArray ?: return buildJsonObject { put("error", "Missing paths") }
        return sendAndWait("kb.read_notes", buildJsonObject { put("paths", paths) })
    }
}
