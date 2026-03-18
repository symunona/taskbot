package com.hermes.voice

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.browser.window
import org.w3c.dom.mediacapture.MediaStream

actual class VoiceManager {
    private val _isRecording = MutableStateFlow(false)
    actual val isRecording: StateFlow<Boolean> = _isRecording

    private val _isPlaying = MutableStateFlow(false)
    actual val isPlaying: StateFlow<Boolean> = _isPlaying

    private var mediaRecorder: dynamic = null
    private var audioContext: dynamic = null

    actual fun startRecording(onAudioData: (ByteArray) -> Unit) {
        if (_isRecording.value) return
        
        val navigator = window.navigator
        val constraints = js("{ audio: true }")
        
        navigator.asDynamic().mediaDevices.getUserMedia(constraints).then { stream: dynamic ->
            _isRecording.value = true
            
            // Set up MediaRecorder
            mediaRecorder = js("new MediaRecorder(stream, { mimeType: 'audio/webm' })")
            
            mediaRecorder.ondataavailable = { event: dynamic ->
                if (event.data.size > 0) {
                    // Note: In a real app, you need to convert webm to PCM 16kHz for Gemini
                    // For now, we're just sending the raw chunks to the callback
                    val blob = event.data
                    val reader = js("new FileReader()")
                    reader.onload = { e: dynamic ->
                        val arrayBuffer = e.target.result
                        val uint8Array = js("new Uint8Array(arrayBuffer)")
                        val byteArray = ByteArray(uint8Array.length as Int) { i -> uint8Array[i] as Byte }
                        onAudioData(byteArray)
                    }
                    reader.readAsArrayBuffer(blob)
                }
            }
            
            mediaRecorder.start(100) // 100ms chunks
        }.catch { error: dynamic ->
            println("Error accessing microphone: $error")
            null
        }
    }

    actual fun stopRecording() {
        if (!_isRecording.value) return
        mediaRecorder?.stop()
        _isRecording.value = false
    }

    actual fun playAudio(pcmData: ByteArray) {
        if (audioContext == null) {
            audioContext = js("new (window.AudioContext || window.webkitAudioContext)({ sampleRate: 24000 })")
        }
        
        _isPlaying.value = true
        
        // Convert PCM to AudioBuffer and play
        // Simplified implementation for JS target
        val arrayBuffer = js("new ArrayBuffer(pcmData.length)")
        val view = js("new Uint8Array(arrayBuffer)")
        for (i in pcmData.indices) {
            view[i] = pcmData[i]
        }
        
        audioContext.decodeAudioData(arrayBuffer).then { buffer: dynamic ->
            val source = audioContext.createBufferSource()
            source.buffer = buffer
            source.connect(audioContext.destination)
            source.onended = {
                _isPlaying.value = false
            }
            source.start()
        }.catch { e: dynamic ->
            println("Error playing audio: $e")
            _isPlaying.value = false
            null
        }
    }

    actual fun stopPlayback() {
        // Suspend current audio context
        audioContext?.suspend(); Unit
        _isPlaying.value = false
    }
}
