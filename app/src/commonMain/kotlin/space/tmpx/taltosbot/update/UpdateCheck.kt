package space.tmpx.taltosbot.update

import space.tmpx.taltosbot.BuildEnv
import space.tmpx.taltosbot.connection.ServerApi
import kotlinx.datetime.Instant
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.longOrNull

const val APK_URL = "https://taskbot.tmpx.space/taltosbot.apk"
const val APK_SIDECAR_URL = "https://taskbot.tmpx.space/taltosbot.apk.json"

data class UpdateInfo(val version: String, val versionCode: Long, val builtAt: String)

/** Contract rule: newer `version_code`, or the same code built later than this build. */
fun isNewer(remoteCode: Long, remoteBuiltAtMs: Long?, ownCode: Long, ownBuildEpochMs: Long): Boolean =
    remoteCode > ownCode || (remoteCode == ownCode && remoteBuiltAtMs != null && remoteBuiltAtMs > ownBuildEpochMs)

/** Returns the available update, or null when this build is current. Throws on network/parse errors. */
suspend fun checkForUpdate(api: ServerApi): UpdateInfo? {
    val sidecar = api.getJson(APK_SIDECAR_URL)
    val code = (sidecar["version_code"] as? JsonPrimitive)?.longOrNull
        ?: throw IllegalStateException("Sidecar has no version_code")
    val builtAt = (sidecar["built_at"] as? JsonPrimitive)?.contentOrNull ?: ""
    val builtAtMs = runCatching { Instant.parse(builtAt).toEpochMilliseconds() }.getOrNull()
    val version = (sidecar["version"] as? JsonPrimitive)?.contentOrNull ?: "?"
    return if (isNewer(code, builtAtMs, BuildEnv.VERSION_CODE.toLong(), BuildEnv.BUILD_EPOCH)) {
        UpdateInfo(version, code, builtAt)
    } else {
        null
    }
}
