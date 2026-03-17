package com.hermes.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.compose.material.Button
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions

@Composable
actual fun QrScannerButton(onScanned: (String) -> Unit) {
    val launcher = rememberLauncherForActivityResult(ScanContract()) { result ->
        if (result.contents != null) {
            onScanned(result.contents)
        }
    }
    
    Button(onClick = {
        val options = ScanOptions()
        options.setDesiredBarcodeFormats(ScanOptions.QR_CODE)
        options.setPrompt("Scan Hermes Connection QR Code")
        options.setBeepEnabled(false)
        launcher.launch(options)
    }) {
        Text("Scan QR Code")
    }
}
