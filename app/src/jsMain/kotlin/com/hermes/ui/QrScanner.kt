package com.hermes.ui

import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

@Composable
actual fun QrScannerButton(onScanned: (String) -> Unit) {
    Text("QR Scanning is available on the Android app.", color = Color.Gray)
}
