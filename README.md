# Hermes 🛰️

Hermes is a Kotlin Multiplatform + Rust project for secure message and task exchange across Android and web clients.
It ships with a Rust backend service (`tx`) and a shared Compose UI module used by Android and JS targets.
The `:app` module contains shared logic and UI, while `:androidApp` packages the Android application.
A helper installer builds the backend, sets up networking, and installs a user-level systemd service.
This repository also includes a shortcut script for producing a debug Android APK.

## Installation ⚙️

### 1. Prerequisites 📦
- JDK 17+
- Android SDK
- Rust toolchain and Cargo
- Linux with `systemd --user` support

### 2. Clone the repository 📥
```bash
git clone <your-repo-url>
cd taskbot
```

### 3. Install the backend service 🦀
```bash
chmod +x install.sh
./install.sh
```
This script builds `tx`, uses `server/config.toml` as the active config, migrates `config.toml` from the repo root if needed, runs `network-setup`, installs `tx` to `~/.local/bin/tx`, and enables `hermes-tx.service`.

### 4. Build the Android debug APK 🤖
```bash
chmod +x build_apk.sh
./build_apk.sh
```
The APK will be created at `androidApp/build/outputs/apk/debug/androidApp-debug.apk`.

## Useful Commands 🧰
- Backend debug build: `cd server && cargo build`
- Backend release build: `cd server && cargo build --release`
- Backend lint: `cd server && cargo clippy`
- Backend format: `cd server && cargo fmt`
- Android debug assemble: `ENV=prod ./gradlew :androidApp:assembleDebug`
