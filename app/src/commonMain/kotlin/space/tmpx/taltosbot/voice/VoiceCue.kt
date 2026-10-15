package space.tmpx.taltosbot.voice

import kotlin.math.PI
import kotlin.math.exp
import kotlin.math.min
import kotlin.math.sin

internal class CueNote(val hz: Double, val ms: Int)

/** Short synthesized sounds for things that happen without speech. No asset files. */
enum class VoiceCue(internal val notes: List<CueNote>) {
    /** A search tool started: the answer takes a moment. */
    SEARCH(listOf(CueNote(587.3, 55), CueNote(784.0, 70))),

    /** A delegated agent answered. Gemini reads it out once it is done speaking. */
    AGENT_REPLY(listOf(CueNote(880.0, 110), CueNote(1318.5, 220)))
}

const val CUE_SAMPLE_RATE = 24_000

/** Quiet on purpose: a cue can land on top of live speech. */
private const val CUE_AMPLITUDE = 0.16
private const val CUE_ATTACK_MS = 4.0

/** PCM16 mono little-endian at [CUE_SAMPLE_RATE]. */
fun cuePcm(cue: VoiceCue): ByteArray {
    val samples = cue.notes.sumOf { it.ms * CUE_SAMPLE_RATE / 1000 }
    val pcm = ByteArray(samples * 2)
    var offset = 0
    cue.notes.forEach { note ->
        val count = note.ms * CUE_SAMPLE_RATE / 1000
        for (i in 0 until count) {
            val ms = i * 1000.0 / CUE_SAMPLE_RATE
            // Fast attack, decay to near silence at the note end: no clicks at the joints.
            val envelope = min(1.0, ms / CUE_ATTACK_MS) * exp(-4.0 * ms / note.ms)
            val sample = (sin(2 * PI * note.hz * i / CUE_SAMPLE_RATE) * envelope * CUE_AMPLITUDE * Short.MAX_VALUE).toInt()
            pcm[offset++] = (sample and 0xFF).toByte()
            pcm[offset++] = ((sample shr 8) and 0xFF).toByte()
        }
    }
    return pcm
}
