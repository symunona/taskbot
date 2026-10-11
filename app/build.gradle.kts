import java.util.Properties

plugins {
    kotlin("multiplatform")
    kotlin("plugin.serialization")
    id("com.android.library")
    id("org.jetbrains.compose")
}

group = "space.tmpx.taltosbot"
version = "1.0-SNAPSHOT"

android {
    namespace = "space.tmpx.taltosbot.app"
    compileSdk = 34
    defaultConfig {
        minSdk = 24
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
}

// version.properties at the repo root is written by tools/bump-version.sh; absent -> fallbacks.
val versionProps = Properties().apply {
    val f = rootProject.file("version.properties")
    if (f.exists()) f.inputStream().use { load(it) }
}
val appVersionName = versionProps.getProperty("VERSION_NAME", "0.1.0").trim()
val appVersionCode = versionProps.getProperty("VERSION_CODE", "1").trim().toInt()
val appBuildEpoch = versionProps.getProperty("BUILD_EPOCH", "0").trim().toLong()

val generateBuildConfig by tasks.registering {
    val outputDir = layout.buildDirectory.dir("generated/source/buildConfig/commonMain/kotlin").get().asFile
    inputs.property("versionName", appVersionName)
    inputs.property("versionCode", appVersionCode)
    inputs.property("buildEpoch", appBuildEpoch)
    outputs.dir(outputDir)
    doLast {
        outputDir.deleteRecursively()
        val file = File(outputDir, "space/tmpx/taltosbot/BuildEnv.kt")
        file.parentFile.mkdirs()
        file.writeText("""
            package space.tmpx.taltosbot

            object BuildEnv {
                const val VERSION_NAME = "$appVersionName"
                const val VERSION_CODE = $appVersionCode
                const val BUILD_EPOCH = ${appBuildEpoch}L
            }
        """.trimIndent())
    }
}

kotlin {
    androidTarget {
        compilations.all {
            kotlinOptions {
                jvmTarget = "1.8"
            }
        }
    }
    
    js(IR) {
        browser {
            commonWebpackConfig {
                cssSupport {
                    enabled.set(true)
                }
            }
        }
        binaries.executable()
    }

    sourceSets {
        val commonMain by getting {
            kotlin.srcDir(generateBuildConfig.map { it.outputs.files.singleFile })
            dependencies {
                implementation(compose.runtime)
                implementation(compose.foundation)
                implementation(compose.material)
                implementation(compose.materialIconsExtended)
                implementation(compose.ui)
                implementation(compose.components.resources)
                implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.8.0")
                implementation("org.jetbrains.kotlinx:kotlinx-datetime:0.5.0")
                implementation("io.ktor:ktor-client-core:2.3.8")
                implementation("io.ktor:ktor-client-websockets:2.3.8")
                implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.6.2")
            }
        }
        val androidMain by getting {
            dependencies {
                implementation("io.ktor:ktor-client-okhttp:2.3.8")
                implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.8.0")
                implementation("androidx.activity:activity-compose:1.8.2")
                implementation("com.journeyapps:zxing-android-embedded:4.3.0")
            }
        }
        val jsMain by getting {
            dependencies {
                implementation("io.ktor:ktor-client-js:2.3.8")
            }
        }
    }
}

// Ships skiko.js + skiko.wasm with the JS distribution; without it the canvas app never starts.
compose.experimental {
    web.application {}
}
