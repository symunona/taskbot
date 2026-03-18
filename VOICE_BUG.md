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

## Current State & Next Steps

1. **The Core Issue**: We need to find the correct combination of **Model Name** and **API Version** that supports the `BidiGenerateContent` WebSocket endpoint.
2. **The Timeout Issue**: The app waits for 10 seconds for a `setupComplete` message from the server. Because the WebSocket closes immediately with an error (either 502 or VIOLATED_POLICY), the `setupComplete` deferred is never resolved, causing the app to hang for 10 seconds before failing.

### Recommended Next Steps
1. **Find the Correct Model**: Research the current Google Gemini API documentation for the correct model string for the Realtime API. It is likely `models/gemini-2.0-flash-realtime-exp` or similar, running on the `v1alpha` endpoint.
2. **Fix the Timeout Hang**: In `GeminiLiveClient.kt`, when the WebSocket closes or receives a `goAway` message, it should immediately complete the `setupComplete` deferred exceptionally so the UI doesn't hang for 10 seconds. (I added logging for the close reason, but the timeout still occurs).
3. **Verify Audio Config**: Once the connection succeeds, verify that the `setupMessage` JSON payload matches the exact schema expected by the Realtime API (e.g., `voiceConfig`, `generationConfig`, etc.).