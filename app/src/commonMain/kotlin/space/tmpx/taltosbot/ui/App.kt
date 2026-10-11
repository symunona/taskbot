package space.tmpx.taltosbot.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Menu
import androidx.compose.material.icons.filled.Mic
import androidx.compose.material.icons.filled.MicNone
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import space.tmpx.taltosbot.BuildEnv
import space.tmpx.taltosbot.connection.ConnState
import space.tmpx.taltosbot.connection.SessionInfo
import space.tmpx.taltosbot.update.APK_URL
import kotlinx.coroutines.launch
import kotlinx.datetime.Instant
import kotlinx.datetime.TimeZone
import kotlinx.datetime.toLocalDateTime

enum class AppScreen(val title: String) {
    CHAT("Conversation"), SESSIONS("Sessions"), SETTINGS("Settings"), LOGS("Logs")
}

internal fun ConnState.label() = when (this) {
    ConnState.Connected -> "Connected"
    ConnState.Connecting -> "Connecting..."
    ConnState.Disconnected -> "Disconnected"
}

internal fun ConnState.color() = when (this) {
    ConnState.Connected -> Color.Green
    ConnState.Connecting -> Color.Yellow
    ConnState.Disconnected -> Color.Gray
}

@Composable
fun App() {
    val controller = remember { AppController.instance }

    MaterialTheme(colors = darkColors()) {
        Surface(modifier = Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.systemBars).imePadding()) {
            if (controller.pairing == null) {
                PairingScreen(controller)
            } else {
                MainScreen(controller)
            }
        }
    }
}

@Composable
fun UpdateBanner(controller: AppController) {
    val update = controller.update
    if (update != null) {
        Row(
            modifier = Modifier.fillMaxWidth().background(Color(0xFF2E7D32)).padding(horizontal = 12.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            Text("Update available: ${update.version}", color = Color.White, modifier = Modifier.weight(1f))
            TextButton(onClick = { openUrl(APK_URL) }) {
                Text("Download", color = Color.White)
            }
        }
    }
}

@Composable
fun PairingScreen(controller: AppController) {
    var input by remember { mutableStateOf("") }

    Column(modifier = Modifier.fillMaxSize()) {
        UpdateBanner(controller)
        Column(
            modifier = Modifier.fillMaxSize().padding(20.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center
        ) {
            Text("Pair with Taltosbot server", style = MaterialTheme.typography.h5)
            Spacer(modifier = Modifier.height(8.dp))
            Text("Run `taltosbot pair` on the server.", color = Color.Gray)
            Spacer(modifier = Modifier.height(32.dp))

            QrScannerButton(onScanned = { controller.submitConnectString(it) })

            Spacer(modifier = Modifier.height(32.dp))
            Text("or type / paste the connect string:")
            Spacer(modifier = Modifier.height(16.dp))

            OutlinedTextField(
                value = input,
                onValueChange = { input = it },
                modifier = Modifier.width(320.dp),
                singleLine = true
            )

            Spacer(modifier = Modifier.height(16.dp))

            Button(
                onClick = { controller.submitConnectString(input) },
                enabled = input.isNotBlank() && !controller.pairingBusy
            ) {
                Text(if (controller.pairingBusy) "Pairing..." else "Pair")
            }

            controller.pairingError?.let {
                Spacer(modifier = Modifier.height(16.dp))
                Text(it, color = Color.Red)
            }

            Spacer(modifier = Modifier.height(32.dp))
            Text("Taltosbot ${BuildEnv.VERSION_NAME} (${BuildEnv.VERSION_CODE})", color = Color.Gray, style = MaterialTheme.typography.caption)
        }
    }
}

@Composable
fun MainScreen(controller: AppController) {
    val connectionState by controller.connection.state.collectAsState()
    val scaffoldState = rememberScaffoldState()
    val coroutineScope = rememberCoroutineScope()
    var currentScreen by remember { mutableStateOf(AppScreen.CHAT) }
    var threadPanelOpen by remember { mutableStateOf(false) }

    Box(modifier = Modifier.fillMaxSize()) {
        Scaffold(
            scaffoldState = scaffoldState,
            topBar = {
                TopAppBar(
                    title = {
                        Text(
                            if (currentScreen == AppScreen.CHAT) controller.currentThread?.name ?: "Taltosbot" else currentScreen.title,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis
                        )
                    },
                    navigationIcon = {
                        IconButton(onClick = { coroutineScope.launch { scaffoldState.drawerState.open() } }) {
                            Icon(Icons.Filled.Menu, contentDescription = "Menu")
                        }
                    },
                    actions = {
                        TalkToggle(controller)
                        ConnectionDot(connectionState, onClick = { currentScreen = AppScreen.SETTINGS })
                        IconButton(onClick = { threadPanelOpen = true }) {
                            Icon(Icons.Filled.Menu, contentDescription = "Thread")
                        }
                    }
                )
            },
            drawerContent = {
                Column(modifier = Modifier.fillMaxSize().padding(vertical = 16.dp)) {
                    Text("Taltosbot", style = MaterialTheme.typography.h6, modifier = Modifier.padding(horizontal = 16.dp))
                    Spacer(modifier = Modifier.height(16.dp))
                    AppScreen.values().forEach { screen ->
                        Text(
                            screen.title,
                            color = if (screen == currentScreen) MaterialTheme.colors.primary else Color.White,
                            modifier = Modifier.fillMaxWidth().clickable {
                                currentScreen = screen
                                coroutineScope.launch { scaffoldState.drawerState.close() }
                            }.padding(horizontal = 16.dp, vertical = 12.dp)
                        )
                    }
                    Divider(modifier = Modifier.padding(vertical = 8.dp))
                    // Left side: navigation and the thread list. Right side: the thread on screen.
                    ThreadsList(
                        controller = controller,
                        open = scaffoldState.drawerState.isOpen,
                        onPicked = {
                            currentScreen = AppScreen.CHAT
                            coroutineScope.launch { scaffoldState.drawerState.close() }
                        }
                    )
                    Text(
                        "Status: ${connectionState.label()}",
                        color = connectionState.color(),
                        modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 8.dp)
                    )
                }
            }
        ) { paddingValues ->
            Column(modifier = Modifier.padding(paddingValues).fillMaxSize()) {
                UpdateBanner(controller)
                when (currentScreen) {
                    AppScreen.CHAT -> ChatScreen(controller)
                    AppScreen.SESSIONS -> SessionsScreen(controller, connectionState)
                    AppScreen.SETTINGS -> SettingsScreen(controller, connectionState)
                    AppScreen.LOGS -> LogsScreen()
                }
            }
        }
        ThreadPanel(controller = controller, open = threadPanelOpen, onClose = { threadPanelOpen = false })
    }
}

/** Connection state as a bare dot. Tap = Settings, where the details are. */
@Composable
private fun ConnectionDot(state: ConnState, onClick: () -> Unit) {
    Box(
        modifier = Modifier
            .size(44.dp)
            .clip(CircleShape)
            .clickable(onClick = onClick)
            .semantics { contentDescription = "Connection: ${state.label()}. Open settings" },
        contentAlignment = Alignment.Center
    ) {
        Box(modifier = Modifier.size(12.dp).background(state.color(), CircleShape))
    }
}

/** Voice on/off. Outline = idle, spinner = connecting, red = live. Hidden where there is no voice (web). */
@Composable
private fun TalkToggle(controller: AppController) {
    if (!platformVoiceSupported) return
    val config by controller.connection.config.collectAsState()
    val state = controller.voiceState
    val enabled = config != null || state != VoiceState.Idle
    val shape = RoundedCornerShape(50)
    val tint = if (enabled) Color.White else Color.Gray

    Row(
        modifier = Modifier
            .clip(shape)
            .background(if (state == VoiceState.Active) Color(0xFFC62828) else Color.Transparent)
            .border(1.dp, if (state == VoiceState.Active) Color(0xFFC62828) else tint, shape)
            .clickable(enabled = enabled, onClick = { controller.toggleVoice() })
            .padding(horizontal = 12.dp, vertical = 6.dp)
            .semantics { contentDescription = if (state == VoiceState.Idle) "Start voice" else "Stop voice" },
        verticalAlignment = Alignment.CenterVertically
    ) {
        when (state) {
            VoiceState.Connecting -> CircularProgressIndicator(
                modifier = Modifier.size(18.dp), color = Color.White, strokeWidth = 2.dp
            )
            VoiceState.Active -> Icon(Icons.Filled.Mic, contentDescription = null, tint = tint, modifier = Modifier.size(18.dp))
            VoiceState.Idle -> Icon(Icons.Filled.MicNone, contentDescription = null, tint = tint, modifier = Modifier.size(18.dp))
        }
        Spacer(modifier = Modifier.width(6.dp))
        Text(if (state == VoiceState.Active) "Stop" else "Talk", color = tint, style = MaterialTheme.typography.button)
    }
}

@Composable
fun SessionsScreen(controller: AppController, connectionState: ConnState) {
    // Refresh on open, and again once the socket comes up.
    LaunchedEffect(connectionState) { controller.refreshSessions() }

    Column(modifier = Modifier.fillMaxSize().padding(16.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("Agent sessions", style = MaterialTheme.typography.h6, modifier = Modifier.weight(1f))
            TextButton(onClick = { controller.refreshSessions() }, enabled = !controller.sessionsLoading) {
                Text(if (controller.sessionsLoading) "Loading..." else "Refresh")
            }
        }
        controller.sessionsError?.let { Text(it, color = Color.Red) }
        Divider(color = Color.DarkGray, modifier = Modifier.padding(vertical = 8.dp))
        if (controller.sessions.isEmpty() && controller.sessionsError == null && !controller.sessionsLoading) {
            Text("No agent sessions. Ask Taltosbot to delegate something to a project.", color = Color.Gray)
        }
        LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            items(controller.sessions) { session -> SessionRow(session) }
        }
    }
}

@Composable
private fun SessionRow(session: SessionInfo) {
    val statusColor = when (session.status) {
        "busy" -> Color.Yellow
        "idle" -> Color.Green
        "dead" -> Color.Red
        else -> Color.Gray
    }
    Row(
        modifier = Modifier.fillMaxWidth().background(Color(0xFF2D2D2D), MaterialTheme.shapes.medium).padding(12.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Box(modifier = Modifier.size(10.dp).background(statusColor, CircleShape))
        Column(modifier = Modifier.weight(1f).padding(horizontal = 12.dp)) {
            Text(session.project)
            Text(session.status, color = statusColor, style = MaterialTheme.typography.caption)
        }
        val url = session.url
        Button(onClick = { url?.let(::openUrl) }, enabled = url != null) {
            Text("Open in Claude")
        }
    }
}
