package space.tmpx.taltosbot.ui

import androidx.compose.material.OutlinedButton
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import kotlinx.browser.window

/**
 * No camera scanning on web. The canvas text field cannot be relied on for system-clipboard
 * paste, so offer the browser's own prompt, where paste always works.
 */
@Composable
actual fun QrScannerButton(onScanned: (String) -> Unit) {
    OutlinedButton(onClick = {
        val pasted = window.prompt("Paste the connect string printed by `taltosbot pair`")
        if (!pasted.isNullOrBlank()) onScanned(pasted)
    }) {
        Text("Paste connect string...")
    }
}
