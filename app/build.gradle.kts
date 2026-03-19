plugins {
    kotlin("multiplatform")
    kotlin("plugin.serialization")
    id("com.android.library")
    id("org.jetbrains.compose")
}

group = "com.hermes"
version = "1.0-SNAPSHOT"

android {
    namespace = "com.hermes.app"
    compileSdk = 34
    defaultConfig {
        minSdk = 24
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
}

val generateBuildConfig by tasks.registering {
    // Check if we're running an Android build or explicitly requested an env
    val isAndroidRelease = gradle.startParameter.taskNames.any { it.contains("Release") }
    val isAndroidDebug = gradle.startParameter.taskNames.any { it.contains("Debug") }
    val defaultEnv = if (isAndroidRelease || isAndroidDebug) "prod" else "dev"
    val env = System.getenv("ENV") ?: defaultEnv
    
    val outputDir = layout.buildDirectory.dir("generated/source/buildConfig/commonMain/kotlin").get().asFile
    outputs.dir(outputDir)
    doLast {
        outputDir.mkdirs()
        val file = File(outputDir, "com/hermes/BuildEnv.kt")
        file.parentFile.mkdirs()
        file.writeText("""
            package com.hermes
            
            object BuildEnv {
                val ENV = "$env"
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
                implementation("io.ktor:ktor-client-content-negotiation:2.3.8")
                implementation("io.ktor:ktor-serialization-kotlinx-json:2.3.8")
                implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.6.2")
            }
        }
        val androidMain by getting {
            dependencies {
                implementation("io.ktor:ktor-client-okhttp:2.3.8")
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
