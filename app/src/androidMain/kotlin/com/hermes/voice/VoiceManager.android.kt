package com.hermes.voice

import android.media.AudioFormat
import android.media.AudioRecord
import android.media.AudioTrack
import android.media.MediaRecorder
import android.media.AudioManager
import android.content.Intent
import android.os.Build
import androidx.core.content.ContextCompat
import com.hermes.ui.AndroidStorage
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

actual class VoiceManager {
    private val _isRecording = MutableStateFlow(false)
    actual val isRecording: StateFlow<Boolean> = _isRecording

    private val _isPlaying = MutableStateFlow(false)
    actual val isPlaying: StateFlow<Boolean> = _isPlaying

    private var audioRecord: AudioRecord? = null
    private var audioTrack: AudioTrack? = null
    private var recordingJob: Job? = null
    private val scope = CoroutineScope(Dispatchers.IO + Job())

    actual fun startRecording(onAudioData: (ByteArray) -> Unit) {
        if (_isRecording.value) return

        val sampleRate = 16000
        val channelConfig = AudioFormat.CHANNEL_IN_MONO
        val audioFormat = AudioFormat.ENCODING_PCM_16BIT
        val bufferSize = AudioRecord.getMinBufferSize(sampleRate, channelConfig, audioFormat)

        try {
            audioRecord = AudioRecord(
                MediaRecorder.AudioSource.MIC,
                sampleRate,
                channelConfig,
                audioFormat,
                bufferSize
            )

            audioRecord?.startRecording()
            _isRecording.value = true
            
            AndroidStorage.context?.let { ctx ->
                val intent = Intent(ctx, HermesVoiceService::class.java)
                ContextCompat.startForegroundService(ctx, intent)
            }

            recordingJob = scope.launch {
                val buffer = ByteArray(bufferSize)
                while (isActive && _isRecording.value) {
                    val readResult = audioRecord?.read(buffer, 0, bufferSize) ?: 0
                    if (readResult > 0) {
                        val data = buffer.copyOf(readResult)
                        onAudioData(data)
                    }
                }
            }
        } catch (e: SecurityException) {
            e.printStackTrace()
        }
    }

    actual fun stopRecording() {
        _isRecording.value = false
        recordingJob?.cancel()
        audioRecord?.stop()
        audioRecord?.release()
        audioRecord = null
        
        AndroidStorage.context?.let { ctx ->
            val intent = Intent(ctx, HermesVoiceService::class.java)
            intent.action = "STOP"
            ctx.startService(intent)
        }
    }

    actual fun playAudio(pcmData: ByteArray) {
        if (audioTrack == null) {
            val sampleRate = 24000
            val channelConfig = AudioFormat.CHANNEL_OUT_MONO
            val audioFormat = AudioFormat.ENCODING_PCM_16BIT
            val bufferSize = AudioTrack.getMinBufferSize(sampleRate, channelConfig, audioFormat)

            audioTrack = AudioTrack(
                AudioManager.STREAM_MUSIC,
                sampleRate,
                channelConfig,
                audioFormat,
                bufferSize,
                AudioTrack.MODE_STREAM
            )
            audioTrack?.play()
        }

        _isPlaying.value = true
        scope.launch {
            audioTrack?.write(pcmData, 0, pcmData.size)
            _isPlaying.value = false
        }
    }

    actual fun stopPlayback() {
        _isPlaying.value = false
        audioTrack?.stop()
        audioTrack?.flush()
        // We keep the track around for next playback, or release it? Let's release for now.
        audioTrack?.release()
        audioTrack = null
    }
}
