package space.tmpx.taltosbot.voice

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/**
 * Voice is disabled on web (`platformVoiceSupported = false`), so nothing calls this.
 * The previous implementation sent MediaRecorder webm chunks as if they were PCM; a real one
 * needs an AudioWorklet producing 16 kHz PCM16 and a 24 kHz PCM16 player.
 */
actual class VoiceManager {
    actual val isRecording: StateFlow<Boolean> = MutableStateFlow(false)
    actual val isPlaying: StateFlow<Boolean> = MutableStateFlow(false)

    actual fun startRecording(onAudioData: (ByteArray) -> Unit) {
        throw UnsupportedOperationException("Voice is not available in the web app")
    }

    actual fun stopRecording() {}
    actual fun playAudio(pcmData: ByteArray) {}
    actual fun stopPlayback() {}
    actual fun playCue(cue: VoiceCue) {}
}
