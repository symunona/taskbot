package space.tmpx.taltosbot.voice

import android.media.AudioFormat
import android.media.AudioRecord
import android.media.AudioTrack
import android.media.MediaRecorder
import android.media.AudioManager
import android.content.Intent
import android.os.Build
import androidx.core.content.ContextCompat
import space.tmpx.taltosbot.ui.AndroidStorage
import space.tmpx.taltosbot.ui.EventLogger
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

    /** Frames handed to the track since the last flush. The track's head position counts from the same point. */
    @Volatile
    private var framesWritten = 0L

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
                val intent = Intent(ctx, TaltosVoiceService::class.java)
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
            val intent = Intent(ctx, TaltosVoiceService::class.java)
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
                var lastHead = -1L
                var stalledPolls = 0
                try {
                    while (isActive) {
                        // Queue empty while speaking: poll until the track has played out what it was given.
                        val chunk = playbackChannel.tryReceive().getOrNull()
                            ?: if (_isPlaying.value) {
                                withTimeoutOrNull(DRAIN_POLL_MS) { playbackChannel.receive() }
                            } else {
                                playbackChannel.receive()
                            }
                        if (chunk == null) {
                            val head = (audioTrack?.playbackHeadPosition ?: 0).toLong() and 0xFFFFFFFFL
                            // A head that stopped moving short of the end is an underrun: nothing more will come out.
                            stalledPolls = if (head == lastHead) stalledPolls + 1 else 0
                            lastHead = head
                            if (head >= framesWritten || stalledPolls >= DRAIN_STALL_POLLS) _isPlaying.value = false
                            continue
                        }
                        _isPlaying.value = true
                        stalledPolls = 0
                        val written = audioTrack?.write(chunk, 0, chunk.size) ?: 0
                        if (written > 0 && isActive) framesWritten += written / BYTES_PER_FRAME
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
        framesWritten = 0L // flush resets the head position too
        _isPlaying.value = false
        // Keep AudioTrack alive - playAudio() will resume it
    }

    actual fun playCue(cue: VoiceCue) {
        val pcm = cuePcm(cue)
        scope.launch {
            var track: AudioTrack? = null
            try {
                // Own static track: mixed with the speech stream, and stopPlayback() does not cut it.
                track = AudioTrack(
                    AudioManager.STREAM_MUSIC,
                    CUE_SAMPLE_RATE,
                    AudioFormat.CHANNEL_OUT_MONO,
                    AudioFormat.ENCODING_PCM_16BIT,
                    pcm.size,
                    AudioTrack.MODE_STATIC
                )
                track.write(pcm, 0, pcm.size)
                track.play()
                delay(pcm.size * 1000L / (CUE_SAMPLE_RATE * BYTES_PER_FRAME) + CUE_TAIL_MS)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                EventLogger.log("VoiceManager: cue $cue failed: ${e.message ?: e}", isError = true)
            } finally {
                track?.release()
            }
        }
    }

    private companion object {
        const val BYTES_PER_FRAME = 2 // PCM16 mono
        const val DRAIN_POLL_MS = 40L
        const val DRAIN_STALL_POLLS = 8
        const val CUE_TAIL_MS = 150L
    }
}
