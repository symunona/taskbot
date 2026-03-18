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
import com.hermes.ui.EventLogger
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
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
    private var playbackJob: Job? = null
    private val playbackChannel = Channel<ByteArray>(capacity = Channel.UNLIMITED)
    private val scope = CoroutineScope(Dispatchers.IO + Job())

    actual fun startRecording(onAudioData: (ByteArray) -> Unit) {
        if (_isRecording.value) return

        val sampleRate = 16000
        val channelConfig = AudioFormat.CHANNEL_IN_MONO
        val audioFormat = AudioFormat.ENCODING_PCM_16BIT
        val bufferSize = AudioRecord.getMinBufferSize(sampleRate, channelConfig, audioFormat)

        try {
            @Suppress("MissingPermission")
            audioRecord = AudioRecord(
                MediaRecorder.AudioSource.VOICE_COMMUNICATION,
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
        // Also fully release playback resources
        stopPlayback()
        audioTrack?.stop()
        audioTrack?.release()
        audioTrack = null

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
            val minBuf = AudioTrack.getMinBufferSize(sampleRate, channelConfig, audioFormat)
            // Use 4x min buffer to prevent underruns during streaming
            val bufferSize = minBuf * 4

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

        // Start single writer coroutine if not running
        if (playbackJob == null || playbackJob?.isActive != true) {
            audioTrack?.play() // resume if paused by stopPlayback()
            playbackJob = scope.launch {
                _isPlaying.value = true
                try {
                    for (chunk in playbackChannel) {
                        audioTrack?.write(chunk, 0, chunk.size)
                    }
                } finally {
                    _isPlaying.value = false
                }
            }
        }

        // Queue the chunk - never blocks (UNLIMITED capacity)
        playbackChannel.trySend(pcmData)
    }

    actual fun stopPlayback() {
        // Cancel writer so it stops mid-chunk
        playbackJob?.cancel()
        playbackJob = null
        // Drain queued audio
        while (playbackChannel.tryReceive().isSuccess) { /* discard */ }
        // Pause + flush hardware buffer for immediate silence
        audioTrack?.pause()
        audioTrack?.flush()
        _isPlaying.value = false
        // Keep AudioTrack alive - playAudio() will resume it
    }
}
