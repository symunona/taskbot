package space.tmpx.taltosbot.voice

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

expect class VoiceManager() {
    val isRecording: StateFlow<Boolean>
    /** True while speech is queued or still coming out of the speaker. */
    val isPlaying: StateFlow<Boolean>
    
    fun startRecording(onAudioData: (ByteArray) -> Unit)
    fun stopRecording()
    
    fun playAudio(pcmData: ByteArray)
    fun stopPlayback()

    /** Plays next to speech, never instead of it. */
    fun playCue(cue: VoiceCue)
}
