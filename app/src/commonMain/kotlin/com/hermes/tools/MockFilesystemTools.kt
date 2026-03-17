package com.hermes.tools

import com.hermes.llm.FunctionDeclaration
import kotlinx.serialization.json.*

class MockFilesystem {
    val files = mutableMapOf<String, String>(
        "README.md" to "# Hermes Project\nWelcome to the mock filesystem.",
        "todo.md" to "- [ ] Implement tools\n- [ ] Build UI\n- [ ] Test",
        "notes/ideas.md" to "Maybe we should add voice support later."
    )
}

class ReadFileTool(private val fs: MockFilesystem) : Tool() {
    override val declaration = FunctionDeclaration(
        name = "read_file",
        description = "Reads the content of a file from the mock filesystem.",
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
        val content = fs.files[path] ?: return buildJsonObject { put("error", "File not found") }
        return buildJsonObject {
            put("content", content)
        }
    }
}

class CreateFileTool(private val fs: MockFilesystem) : Tool() {
    override val declaration = FunctionDeclaration(
        name = "create_file",
        description = "Creates a new file or overwrites an existing one in the mock filesystem.",
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
        fs.files[path] = content
        return buildJsonObject {
            put("status", "success")
        }
    }
}

class SearchKeywordTool(private val fs: MockFilesystem) : Tool() {
    override val declaration = FunctionDeclaration(
        name = "search_keyword",
        description = "Searches for a keyword in all files in the mock filesystem.",
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
        val results = buildJsonArray {
            for ((path, content) in fs.files) {
                if (content.contains(query, ignoreCase = true)) {
                    add(buildJsonObject {
                        put("path", path)
                        put("match", content.lines().firstOrNull { it.contains(query, ignoreCase = true) } ?: "")
                    })
                }
            }
        }
        return buildJsonObject {
            put("results", results)
        }
    }
}

class ContextTool(private val fs: MockFilesystem) : Tool() {
    override val declaration = FunctionDeclaration(
        name = "context",
        description = "Returns the current state of the mock filesystem, including all available files.",
        parameters = buildJsonObject {
            put("type", "object")
            put("properties", buildJsonObject {})
        }
    )

    override suspend fun execute(args: JsonObject): JsonObject {
        val fileList = buildJsonArray {
            for (path in fs.files.keys) {
                add(path)
            }
        }
        return buildJsonObject {
            put("files", fileList)
        }
    }
}
