# Gemini Voice Session Bug - Investigation Notes

## The Problem
When pressing the "Voice" toggle in the Android App, the app attempts to start a voice session but fails after 10 seconds with the error:
`Voice session failed to start: Timed out waiting for 10000 ms`

## Initial Observations
1. The connection attempts to use `BidiGenerateContent` WebSocket endpoint.
2. The initial failure in logs showed a `502 Bad Gateway` from the Google Generative AI WebSocket endpoint.
3. The app was originally using the `v1beta` API version with the `models/gemini-2.5-flash-native-audio-preview-12-2025` model.

## What Was Tried & Failed Assumptions

### Attempt 1: Changing the Model
**Assumption**: The `models/gemini-2.5-flash-native-audio-preview-12-2025` model might be deprecated or invalid, causing the `502 Bad Gateway`.
**Action**: Changed the model to `models/gemini-2.0-flash-exp` while keeping the `v1beta` API version.
**Result**: The server responded with a clear policy violation error instead of a 502:
`CloseReason(reason=VIOLATED_POLICY, message=models/gemini-2.0-flash-exp is not found for API version v1beta, or is not supported for bidiGenerateContent. Call ListMode)`
**Conclusion**: The model name was definitely part of the issue, but `gemini-2.0-flash-exp` isn't supported on `v1beta` for bidirectional streaming.

### Attempt 2: Changing the API Version
**Assumption**: The `bidiGenerateContent` endpoint for `models/gemini-2.0-flash-exp` requires the `v1alpha` API version.
**Action**: Changed the WebSocket URL from `v1beta` to `v1alpha` (`wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1alpha.GenerativeService.BidiGenerateContent?key=$apiKey`).
**Result**: The server still rejected it with a similar error:
`CloseReason(reason=VIOLATED_POLICY, message=models/gemini-2.0-flash-exp is not found for API version v1alpha, or is not supported for bidiGenerateContent. Call ListMod)`
**Conclusion**: `models/gemini-2.0-flash-exp` is explicitly not supported for the `bidiGenerateContent` WebSocket endpoint, regardless of whether it is `v1alpha` or `v1beta`.

## Resolution (2026-03-18)

### Root Causes Found
1. **Wrong model name**: `models/gemini-2.0-flash-exp` does NOT support `bidiGenerateContent`. Server rejects with `VIOLATED_POLICY`.
2. **Binary frame handling**: The Gemini Live API sends `setupComplete` and all other responses as **binary WebSocket frames**, not text frames. The code was only handling `Frame.Text` and skipping binary frames entirely.
3. **No early failure on close**: When the WebSocket closed with an error, `setupComplete` was never completed exceptionally, causing a 10-second timeout hang.

### Fixes Applied
1. **Model**: Changed to `models/gemini-2.5-flash-native-audio-preview-12-2025` on `v1alpha` endpoint.
2. **Binary frames**: `GeminiLiveClient.kt` now decodes both `Frame.Text` and `Frame.Binary` as JSON text.
3. **Early failure**: `setupComplete` is now completed exceptionally on WebSocket close or `goAway` message.
4. **Logging**: `EventLogger` now uses `android.util.Log` with tag `Hermes` for `adb logcat -s Hermes` filtering.
5. **UI errors**: Descriptive user-facing messages instead of raw "Timed out waiting for 10000 ms".

### Verified Working
- Voice session connects in ~1 second
- Audio streams bidirectionally (mic → server, server → speaker)
- Text transcripts display in green voice bubbles in the UI
- 51+ server messages received in a single test session