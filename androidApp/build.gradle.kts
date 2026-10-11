import java.util.Properties

plugins {
    id("com.android.application")
    kotlin("android")
    id("org.jetbrains.compose")
}

val versionProps = Properties().apply {
    val f = rootProject.file("version.properties")
    if (f.exists()) f.inputStream().use { load(it) }
}

android {
    namespace = "space.tmpx.taltosbot"
    compileSdk = 34

    defaultConfig {
        applicationId = "space.tmpx.taltosbot"
        minSdk = 24
        targetSdk = 34
        versionCode = versionProps.getProperty("VERSION_CODE", "1").trim().toInt()
        versionName = versionProps.getProperty("VERSION_NAME", "0.1.0").trim()
    }

    // Shared debug key, so every build installs over the previous one. Absent -> AGP's per-machine key.
    val sharedDebugKeystore = rootProject.file("secrets/debug.keystore")
    if (sharedDebugKeystore.exists()) {
        signingConfigs {
            getByName("debug") {
                storeFile = sharedDebugKeystore
                storePassword = "android"
                keyAlias = "androiddebugkey"
                keyPassword = "android"
            }
        }
    }

    buildTypes {
        getByName("release") {
            isMinifyEnabled = false
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
}

dependencies {
    implementation(project(":app"))
    implementation("androidx.activity:activity-compose:1.8.2")
}
