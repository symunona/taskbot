package com.hermes.ui

import androidx.compose.animation.core.*
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Menu
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.PlayArrow
import com.hermes.connection.*
import com.hermes.llm.*
import com.hermes.tools.*
import com.hermes.voice.VoiceSession
import com.hermes.voice.VoiceTranscript
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.json.*
import kotlin.time.Duration.Companion.seconds
import kotlinx.datetime.Instant
import kotlinx.datetime.TimeZone
import kotlinx.datetime.toLocalDateTime

// A simple interface for local storage that we'll implement per-platform
interface KeyValueStorage {
    fun getString(key: String): String?
    fun setString(key: String, value: String)
}

expect fun getStorage(): KeyValueStorage
expect fun showPlatformToast(message: String)

enum class AppScreen {
    CHAT, SETTINGS, LOGS
}

data class ChatMessage(
    val role: String,
    val content: String,
    val isToolResult: Boolean = false,
    val isVoiceTranscript: Boolean = false,
    val isVoicePartial: Boolean = false
)
data class ThreadInfo(val id: String, val summary: String)

private fun upsertVoiceTranscript(messages: List<ChatMessage>, transcript: VoiceTranscript): List<ChatMessage> {
    val normalizedText = transcript.text.trim()
    if (normalizedText.isBlank()) {
        return messages
    }

    val lastMessage = messages.lastOrNull()

    // Always merge consecutive voice transcripts from the same role into one bubble
    if (lastMessage?.isVoiceTranscript == true && lastMessage.role == transcript.role) {
        val newContent = when {
            // Partial update: new text extends existing (cumulative transcription)
            transcript.isPartial || normalizedText.startsWith(lastMessage.content) ->
                normalizedText
            lastMessage.isVoicePartial || lastMessage.content.startsWith(normalizedText) ->
                normalizedText
            // Same content, skip
            lastMessage.content == normalizedText -> return messages
            // New segment from same role: append
            else -> "${lastMessage.content} ${normalizedText}"
        }
        if (newContent == lastMessage.content) return messages
        return messages.dropLast(1) + lastMessage.copy(
            content = newContent,
            isVoicePartial = transcript.isPartial
        )
    }

    return messages + ChatMessage(
        role = transcript.role,
        content = normalizedText,
        isVoiceTranscript = true,
        isVoicePartial = transcript.isPartial
    )
}

@Composable
fun TypingIndicator() {
    val infiniteTransition = rememberInfiniteTransition()
    
    val alpha1 by infiniteTransition.animateFloat(
        initialValue = 0.2f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(
            animation = tween(durationMillis = 600, easing = LinearEasing),
            repeatMode = RepeatMode.Reverse
        )
    )
    
    val alpha2 by infiniteTransition.animateFloat(
        initialValue = 0.2f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(
            animation = tween(durationMillis = 600, delayMillis = 200, easing = LinearEasing),
            repeatMode = RepeatMode.Reverse
        )
    )
    
    val alpha3 by infiniteTransition.animateFloat(
        initialValue = 0.2f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(
            animation = tween(durationMillis = 600, delayMillis = 400, easing = LinearEasing),
            repeatMode = RepeatMode.Reverse
        )
    )

    Row(
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.padding(4.dp)
    ) {
        Box(modifier = Modifier.size(8.dp).alpha(alpha1).background(Color.White, CircleShape))
        Box(modifier = Modifier.size(8.dp).alpha(alpha2).background(Color.White, CircleShape))
        Box(modifier = Modifier.size(8.dp).alpha(alpha3).background(Color.White, CircleShape))
    }
}

@Composable
fun App() {
    val storage = remember { getStorage() }
    var connectionString by remember { mutableStateOf(storage.getString("connection_string") ?: "") }
    var clientToken by remember { mutableStateOf(storage.getString("client_token") ?: "") }
    var apiKey by remember { mutableStateOf(storage.getString("gemini_api_key") ?: "") }
    var useStandalone by remember { mutableStateOf(storage.getString("use_standalone") == "true") }
    var showConnectionPrompt by remember { mutableStateOf(connectionString.isEmpty() && !useStandalone) }
    var showApiKeyPrompt by remember { mutableStateOf(useStandalone && apiKey.isBlank()) }
    LaunchedEffect(connectionString, useStandalone) {
        showConnectionPrompt = connectionString.isEmpty() && !useStandalone
        if (showConnectionPrompt) {
            showApiKeyPrompt = false
        }
    }

    MaterialTheme(colors = darkColors()) {
        Surface(modifier = Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.systemBars).imePadding()) {
            when {
                showConnectionPrompt -> {
                    ConnectionScreen(
                        onConnect = { str ->
                            if (str.startsWith("hermes://")) {
                                val parts = str.substring(9).split("?")
                                if (parts.size == 2) {
                                    val queryParts = parts[1].split("&")
                                    val expStr = queryParts.find { it.startsWith("exp=") }?.substring(4)
                                    if (expStr != null) {
                                        val exp = expStr.toLongOrNull()
                                        if (exp != null) {
                                            val now = kotlinx.datetime.Clock.System.now().epochSeconds
                                            if (now > exp) {
                                                showPlatformToast("This QR code has expired. Please generate a new one on the server.")
                                                return@ConnectionScreen
                                            }
                                        }
                                    }
                                }
                            }
                            
                            connectionString = str
                            clientToken = "" // Reset client token when scanning a new QR code
                            storage.setString("connection_string", str)
                            storage.setString("client_token", "")
                            useStandalone = false
                            storage.setString("use_standalone", "false")
                            showConnectionPrompt = false
                            showApiKeyPrompt = false
                        },
                        onStandalone = {
                            useStandalone = true
                            storage.setString("use_standalone", "true")
                            showConnectionPrompt = false
                            showApiKeyPrompt = apiKey.isBlank()
                        }
                    )
                }
                showApiKeyPrompt -> {
                    ApiKeyScreen(
                        onKeySubmit = { newKey ->
                            apiKey = newKey
                            storage.setString("gemini_api_key", newKey)
                            showApiKeyPrompt = false
                        },
                        onSkip = {
                            showApiKeyPrompt = false
                        }
                    )
                }
                else -> {
                    MainAppScreen(
                        connectionString = connectionString,
                        clientToken = clientToken,
                        apiKey = apiKey,
                        storage = storage,
                        onApiKeyReceived = { newKey ->
                            apiKey = newKey
                            storage.setString("gemini_api_key", newKey)
                            if (newKey.isNotBlank()) {
                                showApiKeyPrompt = false
                            }
                        },
                        onOpenConnectionScreen = {
                            showConnectionPrompt = true
                        },
                        onConnectionStringChanged = { str ->
                            connectionString = str
                            storage.setString("connection_string", str)
                        },
                        onClientTokenReceived = { token ->
                            clientToken = token
                            storage.setString("client_token", token)
                        }
                    )
                }
            }
        }
    }
}

@Composable
fun ConnectionScreen(onConnect: (String) -> Unit, onStandalone: () -> Unit) {
    var connInput by remember { mutableStateOf("") }

    Column(
        modifier = Modifier.fillMaxSize().padding(20.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center
    ) {
        Text("Connect to Hermes Server", style = MaterialTheme.typography.h5)
        Spacer(modifier = Modifier.height(32.dp))
        
        QrScannerButton(onScanned = { onConnect(it) })
        
        Spacer(modifier = Modifier.height(32.dp))
        Text("OR Paste your connection string:")
        Spacer(modifier = Modifier.height(16.dp))
        
        OutlinedTextField(
            value = connInput,
            onValueChange = { connInput = it },
            modifier = Modifier.width(300.dp)
        )
        
        Spacer(modifier = Modifier.height(16.dp))
        
        Button(onClick = { onConnect(connInput) }) {
            Text("Connect")
        }
        
        Spacer(modifier = Modifier.height(32.dp))
        
        TextButton(onClick = onStandalone) {
            Text("Nah, use standalone mode", color = Color.Gray)
        }
    }
}

@Composable
fun ApiKeyScreen(onKeySubmit: (String) -> Unit, onSkip: () -> Unit) {
    var keyInput by remember { mutableStateOf("") }

    Column(
        modifier = Modifier.fillMaxSize().padding(20.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center
    ) {
        Text("API Key Required", style = MaterialTheme.typography.h5)
        Spacer(modifier = Modifier.height(16.dp))
        Text("Please enter your Gemini API Key:")
        Spacer(modifier = Modifier.height(16.dp))
        
        OutlinedTextField(
            value = keyInput,
            onValueChange = { keyInput = it },
            visualTransformation = PasswordVisualTransformation(),
            modifier = Modifier.width(300.dp)
        )
        
        Spacer(modifier = Modifier.height(16.dp))
        
        Button(onClick = { onKeySubmit(keyInput) }, enabled = keyInput.isNotBlank()) {
            Text("Save & Continue")
        }

        Spacer(modifier = Modifier.height(12.dp))

        TextButton(onClick = onSkip) {
            Text("Skip for now", color = Color.Gray)
        }
    }
}

@Composable
fun MainAppScreen(
    connectionString: String,
    clientToken: String,
    apiKey: String,
    storage: KeyValueStorage,
    onApiKeyReceived: (String) -> Unit,
    onOpenConnectionScreen: () -> Unit,
    onConnectionStringChanged: (String) -> Unit,
    onClientTokenReceived: (String) -> Unit
) {
    val connectionManager = remember { ConnectionManager() }
    val connectionState by connectionManager.webSocketClient.connectionState.collectAsState()
    
    LaunchedEffect(Unit) {
        connectionManager.webSocketClient.onClientTokenReceived = onClientTokenReceived
    }

    val scaffoldState = rememberScaffoldState()
    val coroutineScope = rememberCoroutineScope()
    var currentScreen by remember { mutableStateOf(AppScreen.CHAT) }
    val newThreadEvent = remember { kotlinx.coroutines.flow.MutableSharedFlow<Unit>(extraBufferCapacity = 1) }

    // Load threads initially from storage
    var threads by remember { 
        mutableStateOf(
            try {
                val saved = storage.getString("saved_threads")
                if (saved != null) {
                    Json.parseToJsonElement(saved).jsonArray.map {
                        val obj = it.jsonObject
                        ThreadInfo(
                            obj["id"]?.jsonPrimitive?.content ?: "",
                            obj["summary"]?.jsonPrimitive?.content ?: ""
                        )
                    }
                } else emptyList()
            } catch (e: Exception) { emptyList() }
        )
    }
    var currentThreadId by remember { mutableStateOf<String?>(storage.getString("last_thread_id")?.takeIf { it.isNotEmpty() }) }

    // Save threads to local storage
    LaunchedEffect(threads) {
        if (threads.isNotEmpty()) {
            val arr = buildJsonArray {
                threads.forEach { t ->
                    add(buildJsonObject {
                        put("id", t.id)
                        put("summary", t.summary)
                    })
                }
            }
            storage.setString("saved_threads", arr.toString())
        }
    }
    LaunchedEffect(currentThreadId) {
        val selectedThreadId = currentThreadId
        if (selectedThreadId != null) {
            storage.setString("last_thread_id", selectedThreadId)
        }
    }

    var isConnectionActive by remember { mutableStateOf(true) }

    LaunchedEffect(connectionString, clientToken, isConnectionActive) {
        if (connectionString.isNotEmpty() && isConnectionActive) {
            connectionManager.connectFromString(connectionString, clientToken)
        } else {
            connectionManager.webSocketClient.disconnect()
        }
    }
    
    LaunchedEffect(connectionState) {
        if (connectionState == com.hermes.connection.WebSocketClient.ConnectionState.Connected) {
            val reqId = "req_${kotlin.random.Random.nextInt()}"
            connectionManager.webSocketClient.send(buildJsonObject {
                put("event", "system.config.get")
                put("id", reqId)
                put("ts", 0)
                put("payload", buildJsonObject {})
            })

            val res = withTimeoutOrNull(5.seconds) {
                connectionManager.webSocketClient.events.first { it["ref_id"]?.jsonPrimitive?.content == reqId }
            }

            if (res == null) {
                showPlatformToast("Connected, but the server did not return config in time.")
            } else {
                val payload = res.get("payload")?.jsonObject
                val keysObj = payload?.get("keys")?.jsonObject
                keysObj?.forEach { (k, v) ->
                    storage.setString("key_$k", v.jsonPrimitive.content)
                }
                val fetchedKey = payload?.get("google_api_key")?.jsonPrimitive?.contentOrNull
                if (!fetchedKey.isNullOrEmpty()) {
                    onApiKeyReceived(fetchedKey)
                } else if (apiKey.isEmpty()) {
                    showPlatformToast("Connected, but no Gemini API key was provided by the server. Add one in Settings to enable Gemini features.")
                }
            }
        }
    }

    Scaffold(
        scaffoldState = scaffoldState,
        topBar = {
            TopAppBar(
                title = { Text("Hermes") },
                navigationIcon = {
                    IconButton(onClick = { coroutineScope.launch { scaffoldState.drawerState.open() } }) {
                        Icon(Icons.Filled.Menu, contentDescription = "Menu")
                    }
                }
            )
        },
        drawerContent = {
            Column(modifier = Modifier.fillMaxSize().padding(16.dp)) {
                Text("Menu", style = MaterialTheme.typography.h6)
                Spacer(modifier = Modifier.height(16.dp))
                Text(
                    "Chat",
                    modifier = Modifier.fillMaxWidth().clickable {
                        currentScreen = AppScreen.CHAT
                        coroutineScope.launch { scaffoldState.drawerState.close() }
                    }.padding(vertical = 8.dp)
                )
                Text(
                    "Settings",
                    modifier = Modifier.fillMaxWidth().clickable {
                        currentScreen = AppScreen.SETTINGS
                        coroutineScope.launch { scaffoldState.drawerState.close() }
                    }.padding(vertical = 8.dp)
                )
                Text(
                    "Logs",
                    modifier = Modifier.fillMaxWidth().clickable {
                        currentScreen = AppScreen.LOGS
                        coroutineScope.launch { scaffoldState.drawerState.close() }
                    }.padding(vertical = 8.dp)
                )
                
                Spacer(modifier = Modifier.height(24.dp))
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text("Threads", style = MaterialTheme.typography.subtitle1, color = Color.Gray)
                    Text(
                        "+ New",
                        color = MaterialTheme.colors.primary,
                        modifier = Modifier.clickable {
                            currentThreadId = null
                            currentScreen = AppScreen.CHAT
                            coroutineScope.launch { scaffoldState.drawerState.close() }
                        }.padding(4.dp)
                    )
                }
                Divider(color = Color.DarkGray, modifier = Modifier.padding(vertical = 8.dp))
                LazyColumn(modifier = Modifier.weight(1f)) {
                    items(threads) { thread ->
                        val isActive = thread.id == currentThreadId
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .clickable {
                                    currentThreadId = thread.id
                                    currentScreen = AppScreen.CHAT
                                    coroutineScope.launch { scaffoldState.drawerState.close() }
                                }
                                .background(if (isActive) Color(0xFF4D4D4D) else Color.Transparent)
                                .padding(8.dp),
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Text(thread.summary, color = Color.White, modifier = Modifier.weight(1f))
                            Text("x", color = Color.Gray, modifier = Modifier.clickable {
                                coroutineScope.launch {
                                    val reqId = "req_${kotlin.random.Random.nextInt()}"
                                    connectionManager.webSocketClient.send(buildJsonObject {
                                        put("event", "thread.close")
                                        put("id", reqId)
                                        put("ts", 0)
                                        put("payload", buildJsonObject { put("thread_id", thread.id) })
                                    })
                                    threads = threads.filter { it.id != thread.id }
                                    if (currentThreadId == thread.id) {
                                        currentThreadId = threads.firstOrNull()?.id
                                    }
                                }
                            })
                        }
                    }
                }
                
                val statusText = when (connectionState) {
                    com.hermes.connection.WebSocketClient.ConnectionState.Connected -> "Connected"
                    com.hermes.connection.WebSocketClient.ConnectionState.Connecting -> "Connecting..."
                    com.hermes.connection.WebSocketClient.ConnectionState.Error -> "Error"
                    com.hermes.connection.WebSocketClient.ConnectionState.Disconnected -> "Standalone / Disconnected"
                }
                val statusColor = when (connectionState) {
                    com.hermes.connection.WebSocketClient.ConnectionState.Connected -> Color.Green
                    com.hermes.connection.WebSocketClient.ConnectionState.Connecting -> Color.Yellow
                    com.hermes.connection.WebSocketClient.ConnectionState.Error -> Color.Red
                    com.hermes.connection.WebSocketClient.ConnectionState.Disconnected -> Color.Gray
                }
                Text("Status: $statusText", color = statusColor)
                Spacer(modifier = Modifier.height(8.dp))
                
                if (connectionString.isNotEmpty()) {
                    Button(
                        onClick = {
                            isConnectionActive = !isConnectionActive
                            coroutineScope.launch { scaffoldState.drawerState.close() }
                        },
                        modifier = Modifier.fillMaxWidth(),
                        colors = ButtonDefaults.buttonColors(backgroundColor = Color.DarkGray)
                    ) {
                        Text(if (isConnectionActive) "Disconnect" else "Connect", color = Color.White)
                    }
                } else {
                    Button(
                        onClick = {
                            onOpenConnectionScreen()
                            coroutineScope.launch { scaffoldState.drawerState.close() }
                        },
                        modifier = Modifier.fillMaxWidth()
                    ) {
                        Text("Pair Device")
                    }
                }
            }
        }
    ) { paddingValues ->
        Box(modifier = Modifier.padding(paddingValues).fillMaxSize()) {
            when (currentScreen) {
                AppScreen.CHAT -> ChatScreen(
                    apiKey = apiKey,
                    connectionManager = connectionManager,
                    storage = storage,
                    newThreadEvent = newThreadEvent,
                    threads = threads,
                    onThreadsChange = { threads = it },
                    currentThreadId = currentThreadId,
                    onCurrentThreadIdChange = { currentThreadId = it }
                )
                AppScreen.SETTINGS -> SettingsScreen(
                    apiKey = apiKey,
                    connectionString = connectionString,
                    connectionState = connectionState,
                    onConnectionStringChanged = onConnectionStringChanged,
                    onOpenConnectionScreen = onOpenConnectionScreen,
                    onApiKeyReceived = {
                        onApiKeyReceived(it)
                    }
                )
                AppScreen.LOGS -> LogsScreen()
            }
        }
    }
}

@Composable
fun SettingsScreen(
    apiKey: String,
    connectionString: String,
    connectionState: com.hermes.connection.WebSocketClient.ConnectionState,
    onConnectionStringChanged: (String) -> Unit,
    onOpenConnectionScreen: () -> Unit,
    onApiKeyReceived: (String) -> Unit
) {
    // Parse addresses from connectionString
    var addresses by remember(connectionString) {
        mutableStateOf(
            if (connectionString.startsWith("hermes://")) {
                val parts = connectionString.substring(9).split("?")
                if (parts.size == 2) {
                    parts[1].split("&").filter { it.startsWith("addrs=") }.map { it.substring(6) }
                } else emptyList()
            } else emptyList()
        )
    }
    var keyInput by remember(apiKey) { mutableStateOf(apiKey) }

    Column(modifier = Modifier.fillMaxSize().padding(16.dp)) {
        Text("Settings", style = MaterialTheme.typography.h5)
        Spacer(modifier = Modifier.height(16.dp))
        
        if (connectionString.isEmpty()) {
            Text("Connection:", color = Color.Gray)
            Text("None (Standalone Mode)", modifier = Modifier.padding(vertical = 8.dp))
        } else {
            Text("Server Addresses:", color = Color.Gray)
            
            Column(modifier = Modifier.fillMaxWidth().padding(vertical = 8.dp)) {
                addresses.forEachIndexed { index, address ->
                    Row(
                        modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        OutlinedTextField(
                            value = address,
                            onValueChange = { newValue ->
                                val newList = addresses.toMutableList()
                                newList[index] = newValue
                                addresses = newList
                            },
                            modifier = Modifier.weight(1f),
                            singleLine = true
                        )
                        IconButton(onClick = {
                            val newList = addresses.toMutableList()
                            newList.removeAt(index)
                            addresses = newList
                        }) {
                            Icon(Icons.Filled.Close, contentDescription = "Remove")
                        }
                    }
                }
            }
            
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                TextButton(onClick = {
                    addresses = addresses + ""
                }) {
                    Text("+ Add Address")
                }
                
                Button(onClick = {
                    // Rebuild connection string
                    if (connectionString.startsWith("hermes://")) {
                        val parts = connectionString.substring(9).split("?")
                        if (parts.size == 2) {
                            val base = parts[0]
                            val queryParts = parts[1].split("&").filter { !it.startsWith("addrs=") }.toMutableList()
                            addresses.filter { it.isNotBlank() }.forEach { addr ->
                                queryParts.add("addrs=$addr")
                            }
                            val newString = "hermes://$base?${queryParts.joinToString("&")}"
                            onConnectionStringChanged(newString)
                        }
                    }
                }) {
                    Text("Save Addresses")
                }
            }
        }
        
        Spacer(modifier = Modifier.height(16.dp))
        Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = onOpenConnectionScreen, modifier = Modifier.weight(1f)) {
                Text("Pair New Device")
            }
            if (connectionString.isNotEmpty()) {
                OutlinedButton(onClick = { onConnectionStringChanged("") }, modifier = Modifier.weight(1f)) {
                    Text("Unpair", color = Color.Red)
                }
            }
        }
        
        Spacer(modifier = Modifier.height(16.dp))
        Text("API Key:", color = Color.Gray)
        Text(if (apiKey.isNotEmpty()) "Set (${apiKey.take(5)}...)" else "Not Set", modifier = Modifier.padding(vertical = 8.dp))
        OutlinedTextField(
            value = keyInput,
            onValueChange = { keyInput = it },
            modifier = Modifier.fillMaxWidth(),
            label = { Text("Gemini API Key") },
            visualTransformation = PasswordVisualTransformation(),
            singleLine = true
        )
        Spacer(modifier = Modifier.height(8.dp))
        Button(
            onClick = { onApiKeyReceived(keyInput) },
            enabled = keyInput.isNotBlank()
        ) {
            Text(if (apiKey.isNotEmpty()) "Update API Key" else "Save API Key")
        }
        Spacer(modifier = Modifier.height(16.dp))
        val statusText = when (connectionState) {
            com.hermes.connection.WebSocketClient.ConnectionState.Connected -> "Connected"
            com.hermes.connection.WebSocketClient.ConnectionState.Connecting -> "Connecting..."
            com.hermes.connection.WebSocketClient.ConnectionState.Error -> "Error"
            com.hermes.connection.WebSocketClient.ConnectionState.Disconnected -> "Standalone / Disconnected"
        }
        val statusColor = when (connectionState) {
            com.hermes.connection.WebSocketClient.ConnectionState.Connected -> Color.Green
            com.hermes.connection.WebSocketClient.ConnectionState.Connecting -> Color.Yellow
            com.hermes.connection.WebSocketClient.ConnectionState.Error -> Color.Red
            com.hermes.connection.WebSocketClient.ConnectionState.Disconnected -> Color.Gray
        }
        Text("Status: $statusText", color = statusColor)
    }
}

@Composable
fun LogsScreen() {
    val logs by EventLogger.logs.collectAsState()
    
    Column(modifier = Modifier.fillMaxSize().padding(8.dp)) {
        Text("Event Logs", style = MaterialTheme.typography.h6)
        Spacer(modifier = Modifier.height(8.dp))
        Divider(color = Color.Gray)
        LazyColumn(modifier = Modifier.fillMaxSize()) {
            items(logs.reversed()) { log ->
                val time = Instant.fromEpochMilliseconds(log.timestamp).toLocalDateTime(TimeZone.currentSystemDefault())
                val timeStr = "${time.hour.toString().padStart(2, '0')}:${time.minute.toString().padStart(2, '0')}:${time.second.toString().padStart(2, '0')}"
                
                val textColor = when {
                    log.isError -> Color.Red
                    log.message.contains("Authenticated successfully") -> Color.Green
                    log.message.contains("Sending authentication token") -> Color.Cyan
                    log.message.contains("Connecting to WebSocket") -> Color.Cyan
                    else -> Color.White
                }
                
                Text(
                    "[$timeStr] ${log.message}",
                    style = MaterialTheme.typography.caption,
                    color = textColor
                )
            }
        }
    }
}

@Composable
fun ChatScreen(
    apiKey: String,
    connectionManager: ConnectionManager,
    storage: KeyValueStorage,
    newThreadEvent: kotlinx.coroutines.flow.SharedFlow<Unit>,
    threads: List<ThreadInfo>,
    onThreadsChange: (List<ThreadInfo>) -> Unit,
    currentThreadId: String?,
    onCurrentThreadIdChange: (String?) -> Unit
) {
    val toolRegistry = remember {
        ToolRegistry().apply {
            register(RemoteReadFileTool(connectionManager.webSocketClient))
            register(RemoteCreateFileTool(connectionManager.webSocketClient))
            register(RemoteSearchKeywordTool(connectionManager.webSocketClient))
            register(RemoteSystemInfoTool(connectionManager.webSocketClient))
        }
    }
    
    // Toggle to use Mock LLM instead of Gemini based on ENV
    val useMockLlm = com.hermes.BuildEnv.ENV == "test" || com.hermes.BuildEnv.ENV == "dev"
    val canUseAssistantFeatures = useMockLlm || apiKey.isNotBlank()
    
    val geminiLlm = remember(apiKey) { GeminiTextInterface(apiKey, toolRegistry) }
    val mockLlm = remember { MockLlmInterface(toolRegistry) }
    
    val isGenerating by if (useMockLlm) mockLlm.isGenerating.collectAsState() else geminiLlm.isGenerating.collectAsState()
    
    val coroutineScope = rememberCoroutineScope()
    
    var messages by remember { mutableStateOf(listOf<ChatMessage>()) }
    var input by remember { mutableStateOf("") }
    var replyToMessage by remember { mutableStateOf<ChatMessage?>(null) }
    var isVoiceActive by remember { mutableStateOf(false) }
    var isVoiceConnecting by remember { mutableStateOf(false) }
    var voiceSession by remember { mutableStateOf<VoiceSession?>(null) }
    var voiceCollectorJob by remember { mutableStateOf<Job?>(null) }
    val focusManager = LocalFocusManager.current

    fun stopVoiceSession() {
        voiceCollectorJob?.cancel()
        voiceCollectorJob = null
        voiceSession?.stop()
        voiceSession = null
        isVoiceActive = false
    }

    suspend fun ensureThreadExists(): String {
        currentThreadId?.let { return it }

        val localId = "local_${kotlin.random.Random.nextInt()}"
        val newThread = ThreadInfo(localId, "New Thread")
        onThreadsChange(threads + newThread)
        onCurrentThreadIdChange(localId)

        val reqId = "req_${kotlin.random.Random.nextInt()}"
        val res = connectionManager.webSocketClient.sendAndWait(buildJsonObject {
            put("event", "thread.create")
            put("id", reqId)
            put("ts", 0)
            put("payload", buildJsonObject {})
        })

        val realId = res
            ?.get("payload")
            ?.jsonObject
            ?.get("thread")
            ?.jsonObject
            ?.get("thread_id")
            ?.jsonPrimitive
            ?.contentOrNull

        if (res != null && res["event"]?.jsonPrimitive?.content == "thread.create.result" && realId != null) {
            onThreadsChange(threads.map { if (it.id == localId) it.copy(id = realId) else it })
            if (currentThreadId == localId) {
                onCurrentThreadIdChange(realId)
            }
            return realId
        }

        return localId
    }

    DisposableEffect(Unit) {
        onDispose {
            stopVoiceSession()
        }
    }

    // Save threads to local storage
    LaunchedEffect(threads) {
        if (threads.isNotEmpty()) {
            val arr = buildJsonArray {
                threads.forEach { t ->
                    add(buildJsonObject {
                        put("id", t.id)
                        put("summary", t.summary)
                    })
                }
            }
            storage.setString("saved_threads", arr.toString())
        }
    }

    LaunchedEffect(newThreadEvent) {
        newThreadEvent.collect {
            val localId = "local_${kotlin.random.Random.nextInt()}"
            val newThread = ThreadInfo(localId, "New Thread")
            onThreadsChange(threads + newThread)
            onCurrentThreadIdChange(localId)
            
            launch {
                val reqId = "req_${kotlin.random.Random.nextInt()}"
                val res = connectionManager.webSocketClient.sendAndWait(buildJsonObject {
                    put("event", "thread.create")
                    put("id", reqId)
                    put("ts", 0)
                    put("payload", buildJsonObject {})
                })
                
                if (res != null && res["event"]?.jsonPrimitive?.content == "thread.create.result") {
                    val threadObj = res["payload"]?.jsonObject?.get("thread")?.jsonObject
                    val realId = threadObj?.get("thread_id")?.jsonPrimitive?.content
                    if (realId != null) {
                        onThreadsChange(threads.map { if (it.id == localId) it.copy(id = realId) else it })
                        if (currentThreadId == localId) onCurrentThreadIdChange(realId)
                    }
                }
            }
        }
    }

    // Fetch threads on load if connected
    LaunchedEffect(Unit) {
        val reqId = "req_${kotlin.random.Random.nextInt()}"
        val res = connectionManager.webSocketClient.sendAndWait(buildJsonObject {
            put("event", "thread.list")
            put("id", reqId)
            put("ts", 0)
            put("payload", buildJsonObject {})
        })
        
        if (res != null && res["event"]?.jsonPrimitive?.content == "thread.list.result") {
            val payload = res["payload"]?.jsonObject
            val threadList = payload?.get("threads")?.jsonArray
            if (threadList != null) {
                val newThreads = mutableListOf<ThreadInfo>()
                for (t in threadList) {
                    val tObj = t.jsonObject
                    val id = tObj["thread_id"]?.jsonPrimitive?.content ?: ""
                    val summary = tObj["summary"]?.jsonPrimitive?.content ?: "New Thread"
                    val status = tObj["status"]?.jsonPrimitive?.content ?: ""
                    if (status != "archived" && id.isNotEmpty()) {
                        newThreads.add(ThreadInfo(id, if (summary.isEmpty()) "New Thread" else summary))
                    }
                }
                onThreadsChange(newThreads)
                if (currentThreadId == null && newThreads.isNotEmpty()) {
                    onCurrentThreadIdChange(newThreads.first().id)
                }
            }
        }
    }

    // Load thread content when thread changes
    LaunchedEffect(currentThreadId) {
        val selectedThreadId = currentThreadId
        if (selectedThreadId != null) {
            storage.setString("last_thread_id", selectedThreadId)
            
            val localMsgsStr = storage.getString("thread_msgs_$selectedThreadId")
            if (localMsgsStr != null) {
                try {
                    val parsed = Json.parseToJsonElement(localMsgsStr).jsonArray
                    messages = parsed.map { 
                        val obj = it.jsonObject
                        ChatMessage(
                            obj["role"]?.jsonPrimitive?.content ?: "user",
                            obj["content"]?.jsonPrimitive?.content ?: "",
                            obj["isToolResult"]?.jsonPrimitive?.boolean ?: false,
                            obj["isVoiceTranscript"]?.jsonPrimitive?.boolean ?: false,
                            obj["isVoicePartial"]?.jsonPrimitive?.boolean ?: false
                        )
                    }
                } catch (e: Exception) {
                    messages = listOf()
                }
            } else {
                messages = listOf()
            }
            
            val reqId = "req_${kotlin.random.Random.nextInt()}"
            connectionManager.webSocketClient.send(buildJsonObject {
                put("event", "thread.get")
                put("id", reqId)
                put("ts", 0)
                put("payload", buildJsonObject { put("thread_id", currentThreadId) })
            })
        } else {
            messages = listOf()
        }
    }

    // Save messages (debounced to avoid freezing during voice streaming)
    LaunchedEffect(messages, currentThreadId) {
        if (currentThreadId != null && messages.isNotEmpty()) {
            kotlinx.coroutines.delay(1000L) // debounce: wait 1s of no changes before saving
            val arr = buildJsonArray {
                messages.forEach { msg ->
                    add(buildJsonObject {
                        put("role", msg.role)
                        put("content", msg.content)
                        put("isToolResult", msg.isToolResult)
                        put("isVoiceTranscript", msg.isVoiceTranscript)
                        put("isVoicePartial", msg.isVoicePartial)
                    })
                }
            }
            storage.setString("thread_msgs_$currentThreadId", arr.toString())
        }
    }

    val listState = androidx.compose.foundation.lazy.rememberLazyListState()

    // Auto-scroll to bottom when messages change, if near the bottom
    LaunchedEffect(messages.size, messages.lastOrNull()?.content?.length) {
        val layoutInfo = listState.layoutInfo
        val lastVisibleIndex = layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0
        val totalItems = layoutInfo.totalItemsCount
        // Scroll if within ~2 items of the bottom
        if (totalItems > 0 && totalItems - lastVisibleIndex <= 3) {
            listState.animateScrollToItem(totalItems - 1)
        }
    }

    Column(modifier = Modifier.fillMaxSize()) {
        // Message List
        LazyColumn(
            state = listState,
            modifier = Modifier.weight(1f).padding(horizontal = 10.dp, vertical = 5.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            if (currentThreadId == null) {
                item {
                    val emptyStateText = if (canUseAssistantFeatures) {
                        "Type below to start a new thread."
                    } else {
                        "Add your Gemini API key in Settings to enable chat and voice."
                    }
                    Text(emptyStateText, color = Color.Gray, modifier = Modifier.fillMaxWidth())
                }
            } else {
                items(messages) { msg ->
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = if (msg.role == "user") Arrangement.End else Arrangement.Start
                    ) {
                        val bubbleColor = when {
                            msg.role == "user" && msg.isVoiceTranscript -> Color(0xFF1565C0)
                            msg.role == "model" && msg.isVoiceTranscript -> Color(0xFF2E7D32)
                            msg.role == "user" -> Color(0xFF0056B3)
                            msg.isToolResult -> Color(0xFF444444)
                            else -> Color(0xFF333333)
                        }
                        Box(
                            modifier = Modifier
                                .background(
                                    color = bubbleColor,
                                    shape = MaterialTheme.shapes.medium
                                )
                                .clickable { replyToMessage = msg }
                                .padding(10.dp)
                                .fillMaxWidth(0.8f)
                        ) {
                            Text(msg.content, color = Color.White)
                        }
                    }
                }
                if (isGenerating && (messages.isEmpty() || messages.last().role != "model" || messages.last().content.isEmpty())) {
                    item {
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.Start
                        ) {
                            Box(
                                modifier = Modifier
                                    .background(
                                        color = Color(0xFF333333),
                                        shape = MaterialTheme.shapes.medium
                                    )
                                    .padding(10.dp)
                            ) {
                                TypingIndicator()
                            }
                        }
                    }
                }
            }
        }

        // Input Bar
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .background(Color(0xFF2D2D2D))
        ) {
            if (replyToMessage != null) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .background(Color(0xFF3D3D3D))
                        .padding(horizontal = 10.dp, vertical = 5.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            text = if (replyToMessage!!.role == "user") "Replying to yourself" else "Replying to Hermes",
                            color = Color.Cyan,
                            style = MaterialTheme.typography.caption
                        )
                        Text(
                            text = replyToMessage!!.content,
                            color = Color.White,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            style = MaterialTheme.typography.body2
                        )
                    }
                    IconButton(onClick = { replyToMessage = null }) {
                        Icon(Icons.Filled.Close, contentDescription = "Remove reply", tint = Color.Gray)
                    }
                }
            }
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(10.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                OutlinedTextField(
                    value = input,
                    onValueChange = { input = it },
                    modifier = Modifier.weight(1f),
                    colors = TextFieldDefaults.outlinedTextFieldColors(
                        backgroundColor = Color(0xFF1E1E1E),
                        textColor = Color.White
                    ),
                    trailingIcon = {
                        IconButton(
                            onClick = {
                                if (canUseAssistantFeatures && input.isNotBlank() && !isGenerating) {
                                    val text = if (replyToMessage != null) {
                                        val replyContext = replyToMessage!!.content.lines().joinToString("\n") { "> $it" }
                                        "Response to:\n$replyContext\n\n$input"
                                    } else {
                                        input
                                    }
                                    input = ""
                                    replyToMessage = null
                                    focusManager.clearFocus()
                                
                                coroutineScope.launch {
                                    var targetThreadId = currentThreadId
                                    
                                    // If no thread exists, create one locally first
                                    if (targetThreadId == null) {
                                        val localId = "local_${kotlin.random.Random.nextInt()}"
                                        val newThread = ThreadInfo(localId, "New Thread")
                                        onThreadsChange(threads + newThread)
                                        onCurrentThreadIdChange(localId)
                                        targetThreadId = localId
                                        
                                        // Post to backend asynchronously
                                        launch {
                                            val reqId = "req_${kotlin.random.Random.nextInt()}"
                                            val res = connectionManager.webSocketClient.sendAndWait(buildJsonObject {
                                                put("event", "thread.create")
                                                put("id", reqId)
                                                put("ts", 0)
                                                put("payload", buildJsonObject {})
                                            })
                                            
                                            if (res != null && res["event"]?.jsonPrimitive?.content == "thread.create.result") {
                                                val threadObj = res["payload"]?.jsonObject?.get("thread")?.jsonObject
                                                val realId = threadObj?.get("thread_id")?.jsonPrimitive?.content
                                                if (realId != null) {
                                                    // Update local ID to real ID
                                                    onThreadsChange(threads.map { if (it.id == localId) it.copy(id = realId) else it })
                                                    if (currentThreadId == localId) onCurrentThreadIdChange(realId)
                                                    targetThreadId = realId
                                                }
                                            }
                                        }
                                    }
                                    
                                    if (targetThreadId != null) {
                                        messages = messages + ChatMessage("user", text)
                                        
                                        // Append user message to backend
                                launch {
                                    connectionManager.webSocketClient.send(buildJsonObject {
                                        put("event", "thread.append")
                                        put("id", "req_${kotlin.random.Random.nextInt()}")
                                        put("ts", 0)
                                        put("payload", buildJsonObject { 
                                            put("thread_id", targetThreadId)
                                            put("message", "**User**:\n$text\n\n")
                                        })
                                    })
                                }
                                
                                // Add empty model message for streaming
                                messages = messages + ChatMessage("model", "")
                                val modelMsgIndex = messages.lastIndex
                                
                                val response = if (useMockLlm) {
                                    mockLlm.generateResponseStream(
                                        userText = text,
                                        systemPrompt = "You are Hermes, a helpful assistant with access to a remote server. Use tools when necessary."
                                    ) { chunk ->
                                        messages = messages.toMutableList().apply {
                                            val current = this[modelMsgIndex]
                                            this[modelMsgIndex] = current.copy(content = current.content + chunk)
                                        }
                                    }
                                } else {
                                    geminiLlm.generateResponseStream(
                                        userText = text,
                                        systemPrompt = "You are Hermes, a helpful assistant with access to a remote server. Use tools when necessary."
                                    ) { chunk ->
                                        messages = messages.toMutableList().apply {
                                            val current = this[modelMsgIndex]
                                            this[modelMsgIndex] = current.copy(content = current.content + chunk)
                                        }
                                    }
                                }
                                
                                // Append model response to backend
                                launch {
                                    connectionManager.webSocketClient.send(buildJsonObject {
                                        put("event", "thread.append")
                                        put("id", "req_${kotlin.random.Random.nextInt()}")
                                        put("ts", 0)
                                        put("payload", buildJsonObject { 
                                            put("thread_id", targetThreadId)
                                            put("message", "**Model**:\n> ${response.replace("\n", "\n> ")}\n\n")
                                        })
                                    })
                                }
                            }
                        }
                    }
                }) {
                    Icon(Icons.AutoMirrored.Filled.Send, contentDescription = "Send", tint = if (isGenerating || input.isBlank()) Color.Gray else Color.White)
                        }
                    }
                )
                IconButton(onClick = {
                    if (!canUseAssistantFeatures) {
                        showPlatformToast("Gemini API key is missing. Add it in Settings or connect to a server that provides one.")
                        return@IconButton
                    }
                    if (isVoiceActive) {
                        stopVoiceSession()
                    } else {
                        coroutineScope.launch {
                            ensureThreadExists()
                            val toolsArray = buildJsonArray {
                                toolRegistry.getDeclarations().forEach { decl ->
                                    add(buildJsonObject {
                                        put("name", decl.name)
                                        put("description", decl.description)
                                        put("parameters", decl.parameters)
                                    })
                                }
                            }

                            val threadContext = messages.joinToString("\n") { "${it.role}: ${it.content}" }
                            val personaContext = "You are Hermes, a helpful assistant with access to a remote server. Use tools when necessary.\n\nCurrent thread context:\n$threadContext"

                            val session = VoiceSession(
                                apiKey = apiKey,
                                systemInstruction = personaContext,
                                tools = toolsArray,
                                toolRegistry = toolRegistry,
                                onError = { throwable ->
                                    messages = messages + ChatMessage(
                                        "model",
                                        "Voice session error: ${throwable.message ?: "Unknown error"}",
                                        isToolResult = true
                                    )
                                    stopVoiceSession()
                                }
                            )
                            voiceSession = session
                            isVoiceActive = true
                            voiceCollectorJob = launch {
                                session.transcripts.collect { transcript ->
                                    messages = upsertVoiceTranscript(messages, transcript)
                                }
                            }

                            isVoiceConnecting = true
                            try {
                                session.start()
                                isVoiceConnecting = false
                            } catch (e: Throwable) {
                                isVoiceConnecting = false
                                val errorDetail = e.message ?: "Unknown error"
                                EventLogger.log("VoiceSession: failed to start: $errorDetail", isError = true)
                                val userMessage = when {
                                    "VIOLATED_POLICY" in errorDetail -> "Voice connection rejected: model not supported for live audio. Check API configuration."
                                    "502" in errorDetail || "Bad Gateway" in errorDetail -> "Voice server unavailable (502). Try again in a moment."
                                    "Timed out" in errorDetail -> "Voice connection timed out. Check your network and API key."
                                    "goAway" in errorDetail -> "Voice server disconnected: $errorDetail"
                                    else -> "Voice session failed: $errorDetail"
                                }
                                messages = messages + ChatMessage(
                                    "model",
                                    userMessage,
                                    isToolResult = true
                                )
                                if (voiceSession == session) {
                                    stopVoiceSession()
                                } else {
                                    voiceCollectorJob?.cancel()
                                }
                            }
                        }
                    }
                }) {
                    if (isVoiceConnecting) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(24.dp),
                            color = Color.White,
                            strokeWidth = 2.dp
                        )
                    } else {
                        Icon(
                            if (isVoiceActive) Icons.Filled.Close else Icons.Filled.PlayArrow,
                            contentDescription = "Toggle Voice",
                            tint = when {
                                !canUseAssistantFeatures -> Color.Gray
                                isVoiceActive -> Color.Red
                                else -> Color.White
                            }
                        )
                    }
                }
            }
        }
    }
}
