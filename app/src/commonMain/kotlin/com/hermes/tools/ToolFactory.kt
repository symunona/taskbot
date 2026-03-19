package com.hermes.tools

import com.hermes.connection.ConnectionManager
import com.hermes.llm.FunctionDeclaration
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

class NoOpTool(override val declaration: FunctionDeclaration) : Tool() {
    override suspend fun execute(args: JsonObject): JsonObject {
        return buildJsonObject { put("error", "Not implemented in this environment") }
    }
}

fun buildToolRegistry(
    connectionManager: ConnectionManager,
    onTopicSwitch: suspend (String?) -> Unit = {},
    onEndConversation: suspend () -> Unit = {}
): ToolRegistry {
    val registry = ToolRegistry()
    val mockFs = MockFilesystem()

    // read_file
    registry.register(
        DelegatingTool(
            declaration = ReadFileTool(mockFs).declaration,
            localImpl = ReadFileTool(mockFs),
            remoteImpl = RemoteReadFileTool(connectionManager.webSocketClient),
            connectionManager = connectionManager
        )
    )

    // create_file
    registry.register(
        DelegatingTool(
            declaration = CreateFileTool(mockFs).declaration,
            localImpl = CreateFileTool(mockFs),
            remoteImpl = RemoteCreateFileTool(connectionManager.webSocketClient),
            connectionManager = connectionManager
        )
    )

    // update_file
    registry.register(
        DelegatingTool(
            declaration = UpdateFileTool(mockFs).declaration,
            localImpl = UpdateFileTool(mockFs),
            remoteImpl = RemoteUpdateFileTool(connectionManager.webSocketClient),
            connectionManager = connectionManager
        )
    )

    // edit_file
    registry.register(
        DelegatingTool(
            declaration = EditFileTool(mockFs).declaration,
            localImpl = EditFileTool(mockFs),
            remoteImpl = RemoteEditFileTool(connectionManager.webSocketClient),
            connectionManager = connectionManager
        )
    )

    // delete_file
    registry.register(
        DelegatingTool(
            declaration = DeleteFileTool(mockFs).declaration,
            localImpl = DeleteFileTool(mockFs),
            remoteImpl = RemoteDeleteFileTool(connectionManager.webSocketClient),
            connectionManager = connectionManager
        )
    )

    // rename_file
    registry.register(
        DelegatingTool(
            declaration = RenameFileTool(mockFs).declaration,
            localImpl = RenameFileTool(mockFs),
            remoteImpl = RemoteRenameFileTool(connectionManager.webSocketClient),
            connectionManager = connectionManager
        )
    )

    // move_file
    registry.register(
        DelegatingTool(
            declaration = MoveFileTool(mockFs).declaration,
            localImpl = MoveFileTool(mockFs),
            remoteImpl = RemoteMoveFileTool(connectionManager.webSocketClient),
            connectionManager = connectionManager
        )
    )

    // search_keyword
    registry.register(
        DelegatingTool(
            declaration = SearchKeywordTool(mockFs).declaration,
            localImpl = SearchKeywordTool(mockFs),
            remoteImpl = RemoteSearchKeywordTool(connectionManager.webSocketClient),
            connectionManager = connectionManager
        )
    )

    // system_info
    registry.register(
        DelegatingTool(
            declaration = RemoteSystemInfoTool(connectionManager.webSocketClient).declaration,
            localImpl = NoOpTool(RemoteSystemInfoTool(connectionManager.webSocketClient).declaration),
            remoteImpl = RemoteSystemInfoTool(connectionManager.webSocketClient),
            connectionManager = connectionManager
        )
    )

    // context
    registry.register(
        DelegatingTool(
            declaration = ContextTool(mockFs).declaration,
            localImpl = ContextTool(mockFs),
            remoteImpl = NoOpTool(ContextTool(mockFs).declaration),
            connectionManager = connectionManager
        )
    )


    // Directory Tools
    registry.register(
        DelegatingTool(
            declaration = ListDirectoryTool(mockFs).declaration,
            localImpl = ListDirectoryTool(mockFs),
            remoteImpl = RemoteListDirectoryTool(connectionManager.webSocketClient),
            connectionManager = connectionManager
        )
    )

    registry.register(
        DelegatingTool(
            declaration = ListVaultFilesTool(mockFs).declaration,
            localImpl = ListVaultFilesTool(mockFs),
            remoteImpl = RemoteListVaultFilesTool(connectionManager.webSocketClient),
            connectionManager = connectionManager
        )
    )

    registry.register(
        DelegatingTool(
            declaration = GetFolderTreeTool(mockFs).declaration,
            localImpl = GetFolderTreeTool(mockFs),
            remoteImpl = RemoteGetFolderTreeTool(connectionManager.webSocketClient),
            connectionManager = connectionManager
        )
    )

    registry.register(
        DelegatingTool(
            declaration = CreateDirectoryTool(mockFs).declaration,
            localImpl = CreateDirectoryTool(mockFs),
            remoteImpl = RemoteCreateDirectoryTool(connectionManager.webSocketClient),
            connectionManager = connectionManager
        )
    )

    // Search & Read Tools
    registry.register(
        DelegatingTool(
            declaration = SearchRegexpTool(mockFs).declaration,
            localImpl = SearchRegexpTool(mockFs),
            remoteImpl = RemoteSearchRegexpTool(connectionManager.webSocketClient),
            connectionManager = connectionManager
        )
    )

    registry.register(
        DelegatingTool(
            declaration = SearchReplaceFileTool(mockFs).declaration,
            localImpl = SearchReplaceFileTool(mockFs),
            remoteImpl = RemoteSearchReplaceFileTool(connectionManager.webSocketClient),
            connectionManager = connectionManager
        )
    )

    registry.register(
        DelegatingTool(
            declaration = SearchReplaceGlobalTool(mockFs).declaration,
            localImpl = SearchReplaceGlobalTool(mockFs),
            remoteImpl = RemoteSearchReplaceGlobalTool(connectionManager.webSocketClient),
            connectionManager = connectionManager
        )
    )

    registry.register(
        DelegatingTool(
            declaration = ReadNotesTool(mockFs).declaration,
            localImpl = ReadNotesTool(mockFs),
            remoteImpl = RemoteReadNotesTool(connectionManager.webSocketClient),
            connectionManager = connectionManager
        )
    )

    // Thread Tools
    registry.register(
        DelegatingTool(
            declaration = LocalTopicSwitchTool(onTopicSwitch).declaration,
            localImpl = LocalTopicSwitchTool(onTopicSwitch),
            remoteImpl = RemoteTopicSwitchTool(connectionManager.webSocketClient, onTopicSwitch),
            connectionManager = connectionManager
        )
    )

    registry.register(
        DelegatingTool(
            declaration = LocalEndConversationTool(onEndConversation).declaration,
            localImpl = LocalEndConversationTool(onEndConversation),
            remoteImpl = RemoteEndConversationTool(connectionManager.webSocketClient, onEndConversation),
            connectionManager = connectionManager
        )
    )

    return registry
}
