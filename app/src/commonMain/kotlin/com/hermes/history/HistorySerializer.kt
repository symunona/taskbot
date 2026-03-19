package com.hermes.history

import com.hermes.ui.ChatMessage

object HistorySerializer {
    fun serialize(messages: List<ChatMessage>): String {
        return messages.joinToString("\n\n") { msg ->
            val role = when (msg.role) {
                "user" -> "User"
                "model" -> "Assistant"
                else -> msg.role.replaceFirstChar { if (it.isLowerCase()) it.titlecase() else it.toString() }
            }
            
            val content = if (msg.isToolResult) {
                "```json\n${msg.content}\n```"
            } else {
                msg.content
            }
            
            "**$role**:\n$content"
        }
    }
}
