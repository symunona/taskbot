package com.hermes.tools

import com.hermes.connection.ConnectionManager
import com.hermes.llm.FunctionDeclaration
import kotlinx.serialization.json.JsonObject

class DelegatingTool(
    override val declaration: FunctionDeclaration,
    private val localImpl: Tool,
    private val remoteImpl: Tool,
    private val connectionManager: ConnectionManager
) : Tool() {
    override suspend fun execute(args: JsonObject): JsonObject {
        return if (connectionManager.isConnected()) {
            remoteImpl.execute(args)
        } else {
            localImpl.execute(args)
        }
    }
}
