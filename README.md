# Taskbot (codename project Hermes) 🛰️

Taskbot is an interactive voice agent that:
- uses and syncs context specific memory
- builds it's own knowledge base over time, and lets you configure EVERYTHING about it interactively
- creates a bridge to your home computer or VPS via a secure WebSocket connection
- lets you work on different contexts simultaneously

Frontend: Kotlin Multiplatform (For now, Android App `TaskBot`)
Backend: Rust `tx-service` (daemon), `tx` (CLI tool)

## Installation ⚙️

### 1. Prerequisites 📦
- use SDKMAN! to manage Java versions
    - JDK 17+
    - Android SDK
- Rust toolchain and Cargo
- Backend: Linux with `systemd --user` support

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
