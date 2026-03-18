package com.hermes.voice

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

expect class VoiceManager() {
    val isRecording: StateFlow<Boolean>
    val isPlaying: StateFlow<Boolean>
    
    fun startRecording(onAudioData: (ByteArray) -> Unit)
    fun stopRecording()
    
    fun playAudio(pcmData: ByteArray)
    fun stopPlayback()
}
