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

class UpdateFileTool(private val fs: MockFilesystem) : Tool() {
    override val declaration = FunctionDeclaration(
        name = "update_file",
        description = "Replaces entire file content in the mock filesystem.",
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
        fs.files[path] = content
        return buildJsonObject { put("status", "success") }
    }
}

class EditFileTool(private val fs: MockFilesystem) : Tool() {
    override val declaration = FunctionDeclaration(
        name = "edit_file",
        description = "Line-range edit for a file in the mock filesystem.",
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
        
        val content = fs.files[path] ?: return buildJsonObject { put("error", "File not found") }
        val lines = content.lines().toMutableList()
        if (startLine < 1 || startLine > lines.size + 1 || endLine < startLine - 1) {
            return buildJsonObject { put("error", "Invalid line range") }
        }
        
        val safeStart = (startLine - 1).coerceIn(0, lines.size)
        val safeEnd = endLine.coerceIn(0, lines.size)
        
        val newLines = newContent.lines()
        lines.subList(safeStart, safeEnd).clear()
        lines.addAll(safeStart, newLines)
        
        fs.files[path] = lines.joinToString("\n")
        return buildJsonObject { put("status", "success") }
    }
}

class DeleteFileTool(private val fs: MockFilesystem) : Tool() {
    override val declaration = FunctionDeclaration(
        name = "delete_file",
        description = "Deletes a file in the mock filesystem.",
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
        if (fs.files.remove(path) != null) {
            return buildJsonObject { put("status", "success") }
        }
        return buildJsonObject { put("error", "File not found") }
    }
}

class RenameFileTool(private val fs: MockFilesystem) : Tool() {
    override val declaration = FunctionDeclaration(
        name = "rename_file",
        description = "Renames a file in the mock filesystem.",
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
        val content = fs.files.remove(path) ?: return buildJsonObject { put("error", "File not found") }
        fs.files[newPath] = content
        return buildJsonObject { put("status", "success") }
    }
}

class MoveFileTool(private val fs: MockFilesystem) : Tool() {
    override val declaration = FunctionDeclaration(
        name = "move_file",
        description = "Moves a file to a different folder in the mock filesystem.",
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
        val content = fs.files.remove(path) ?: return buildJsonObject { put("error", "File not found") }
        
        val fileName = path.substringAfterLast("/")
        val newPath = if (destDir.endsWith("/")) "$destDir$fileName" else "$destDir/$fileName"
        
        fs.files[newPath] = content
        return buildJsonObject { put("status", "success") }
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
}class ListDirectoryTool(private val fs: MockFilesystem) : Tool() {
    override val declaration = FunctionDeclaration(
        name = "list_directory",
        description = "List directory contents (shallow) in the mock filesystem.",
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
        val prefix = if (path.isEmpty() || path == "/") "" else if (path.endsWith("/")) path else "$path/"
        
        val items = fs.files.keys
            .filter { it.startsWith(prefix) }
            .map { it.removePrefix(prefix).substringBefore("/") }
            .distinct()
            
        return buildJsonObject {
            put("items", buildJsonArray { items.forEach { add(it) } })
        }
    }
}

class ListVaultFilesTool(private val fs: MockFilesystem) : Tool() {
    override val declaration = FunctionDeclaration(
        name = "list_vault_files",
        description = "List all vault files in the mock filesystem.",
        parameters = buildJsonObject {
            put("type", "object")
            put("properties", buildJsonObject {
                put("filter", buildJsonObject { put("type", "string") })
            })
        }
    )

    override suspend fun execute(args: JsonObject): JsonObject {
        val filter = args["filter"]?.jsonPrimitive?.content
        val files = if (filter != null) {
            fs.files.keys.filter { it.contains(filter) }
        } else {
            fs.files.keys.toList()
        }
        
        return buildJsonObject {
            put("files", buildJsonArray { files.forEach { add(it) } })
        }
    }
}

class GetFolderTreeTool(private val fs: MockFilesystem) : Tool() {
    override val declaration = FunctionDeclaration(
        name = "get_folder_tree",
        description = "Recursive folder tree in the mock filesystem.",
        parameters = buildJsonObject {
            put("type", "object")
            put("properties", buildJsonObject {})
        }
    )

    override suspend fun execute(args: JsonObject): JsonObject {
        return buildJsonObject {
            put("tree", buildJsonArray { fs.files.keys.forEach { add(it) } })
        }
    }
}

class CreateDirectoryTool(private val fs: MockFilesystem) : Tool() {
    override val declaration = FunctionDeclaration(
        name = "create_directory",
        description = "Create directory in the mock filesystem (no-op).",
        parameters = buildJsonObject {
            put("type", "object")
            put("properties", buildJsonObject {
                put("path", buildJsonObject { put("type", "string") })
            })
            put("required", buildJsonArray { add("path") })
        }
    )

    override suspend fun execute(args: JsonObject): JsonObject {
        return buildJsonObject { put("status", "success") }
    }
}
class SearchRegexpTool(private val fs: MockFilesystem) : Tool() {
    override val declaration = FunctionDeclaration(
        name = "search_regexp",
        description = "Regex search across vault in the mock filesystem.",
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
        val regex = Regex(pattern)
        val results = buildJsonArray {
            for ((path, content) in fs.files) {
                if (regex.containsMatchIn(content)) {
                    add(buildJsonObject {
                        put("path", path)
                        put("match", content.lines().firstOrNull { regex.containsMatchIn(it) } ?: "")
                    })
                }
            }
        }
        return buildJsonObject { put("results", results) }
    }
}

class SearchReplaceFileTool(private val fs: MockFilesystem) : Tool() {
    override val declaration = FunctionDeclaration(
        name = "search_replace_file",
        description = "Search+replace in one file in the mock filesystem.",
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
        
        val content = fs.files[path] ?: return buildJsonObject { put("error", "File not found") }
        fs.files[path] = content.replace(search, replace)
        return buildJsonObject { put("status", "success") }
    }
}

class SearchReplaceGlobalTool(private val fs: MockFilesystem) : Tool() {
    override val declaration = FunctionDeclaration(
        name = "search_replace_global",
        description = "Search+replace across vault in the mock filesystem.",
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
        
        for ((path, content) in fs.files) {
            fs.files[path] = content.replace(search, replace)
        }
        return buildJsonObject { put("status", "success") }
    }
}

class ReadNotesTool(private val fs: MockFilesystem) : Tool() {
    override val declaration = FunctionDeclaration(
        name = "read_notes",
        description = "Read array of paths in the mock filesystem.",
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
        val paths = args["paths"]?.jsonArray?.map { it.jsonPrimitive.content } ?: return buildJsonObject { put("error", "Missing paths") }
        
        val files = buildJsonObject {
            for (path in paths) {
                fs.files[path]?.let { content ->
                    put(path, content)
                }
            }
        }
        return buildJsonObject { put("files", files) }
    }
}
