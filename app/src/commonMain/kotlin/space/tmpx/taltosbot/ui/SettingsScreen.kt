package space.tmpx.taltosbot.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ExpandLess
import androidx.compose.material.icons.filled.ExpandMore
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import space.tmpx.taltosbot.BuildEnv
import space.tmpx.taltosbot.connection.ConnState
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull

@Composable
fun SettingsScreen(controller: AppController, connectionState: ConnState) {
    val serverUrls = controller.pairing?.serverUrls ?: emptyList()
    val deviceId = controller.pairing?.deviceId ?: ""
    val activeUrl by controller.connection.activeUrl.collectAsState()
    val config by controller.connection.config.collectAsState()
    val tools by controller.connection.tools.collectAsState()

    Column(
        modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        Text("Status: ${connectionState.label()}", color = connectionState.color())

        Text("Server addresses", color = Color.Gray)
        serverUrls.forEach { url ->
            Text(if (url == activeUrl) "$url  (in use)" else url)
        }

        Text("Device", color = Color.Gray)
        Text("$platformDeviceName, id ${deviceId.ifBlank { "?" }}")

        Text("Voice model", color = Color.Gray)
        Text(config?.model?.ifBlank { null } ?: "not received yet")
        Text("Text model", color = Color.Gray)
        Text(config?.textModel?.ifBlank { null } ?: "not received yet")
        Text("Gemini key: ${if (config?.apiKey.isNullOrBlank()) "not received" else "received from server"}")

        ToolsSection(tools)

        Text("Version", color = Color.Gray)
        Text("${BuildEnv.VERSION_NAME} (${BuildEnv.VERSION_CODE}), build ${BuildEnv.BUILD_EPOCH}")

        if (platformUpdateCheckSupported) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                OutlinedButton(onClick = { controller.checkUpdate() }) { Text("Check for update") }
                controller.updateStatus?.let { Text(it, style = MaterialTheme.typography.caption) }
            }
        }

        Spacer(modifier = Modifier.height(8.dp))
        OutlinedButton(onClick = { controller.unpair() }) {
            Text("Unpair this device", color = Color.Red)
        }
    }
}

private fun JsonObject.text(key: String): String? = (this[key] as? JsonPrimitive)?.contentOrNull

/** "N tools", closed by default. Open: one row per tool; a tool row opens to its parameters. */
@Composable
private fun ToolsSection(tools: JsonArray) {
    var open by rememberSaveable { mutableStateOf(false) }

    Column(modifier = Modifier.fillMaxWidth().background(Color(0xFF2D2D2D), MaterialTheme.shapes.medium)) {
        Row(
            modifier = Modifier.fillMaxWidth().clickable { open = !open }.padding(12.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            Text("${tools.size} tools", modifier = Modifier.weight(1f))
            Icon(
                if (open) Icons.Filled.ExpandLess else Icons.Filled.ExpandMore,
                contentDescription = if (open) "Hide tools" else "Show tools"
            )
        }
        if (open) {
            tools.forEach { tool ->
                (tool as? JsonObject)?.let {
                    Divider(color = Color.DarkGray)
                    ToolItem(it)
                }
            }
        }
    }
}

@Composable
private fun ToolItem(tool: JsonObject) {
    val name = tool.text("name") ?: "?"
    var open by rememberSaveable(name) { mutableStateOf(false) }
    val parameters = tool["parameters"] as? JsonObject
    val properties = parameters?.get("properties") as? JsonObject
    val required = (parameters?.get("required") as? JsonArray).orEmpty().mapNotNull { (it as? JsonPrimitive)?.contentOrNull }

    Column(modifier = Modifier.fillMaxWidth().clickable { open = !open }.padding(horizontal = 12.dp, vertical = 8.dp)) {
        Text(name, fontFamily = FontFamily.Monospace)
        Text(
            tool.text("description") ?: "",
            color = Color.LightGray,
            style = MaterialTheme.typography.caption,
            maxLines = if (open) Int.MAX_VALUE else 2,
            overflow = TextOverflow.Ellipsis
        )
        if (open) {
            if (properties.isNullOrEmpty()) {
                Text("No parameters", color = Color.Gray, style = MaterialTheme.typography.caption, modifier = Modifier.padding(top = 6.dp))
            } else {
                properties.forEach { (param, schema) -> ParameterRow(param, schema as? JsonObject, param in required) }
            }
        }
    }
}

@Composable
private fun ParameterRow(name: String, schema: JsonObject?, required: Boolean) {
    val items = (schema?.get("items") as? JsonObject)?.text("type")
    val type = (schema?.text("type") ?: "?") + (items?.let { " of $it" } ?: "")
    Column(modifier = Modifier.padding(start = 8.dp, top = 6.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(name, style = MaterialTheme.typography.caption, fontFamily = FontFamily.Monospace)
            Text(type, color = Color.Gray, style = MaterialTheme.typography.caption)
            Text(
                if (required) "required" else "optional",
                color = if (required) Color(0xFFFFB74D) else Color.Gray,
                style = MaterialTheme.typography.caption
            )
        }
        schema?.text("description")?.takeIf { it.isNotBlank() }?.let {
            Text(it, color = Color.LightGray, style = MaterialTheme.typography.caption)
        }
    }
}
