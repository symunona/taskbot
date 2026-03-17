package com.hermes.ui

import androidx.compose.runtime.Composable

@Composable
expect fun QrScannerButton(onScanned: (String) -> Unit)
