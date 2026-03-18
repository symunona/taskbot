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
    - Android SDK (including platform-tools / `adb` for `just deploy`)
- Rust toolchain and Cargo
- `just`
- Backend: Linux with `systemd --user` support

### 2. Clone the repository 📥
```bash
git clone <your-repo-url>
cd taskbot
```

### 3. Install or update the backend service 🦀
```bash
just install
```
This wraps `install.sh`, which builds `tx`, uses `server/config.toml` as the active config, migrates `config.toml` from the repo root if needed, runs `network-setup`, installs `tx` to `~/.local/bin/tx`, and enables `hermes-tx.service`.

### 4. Build the Android debug APK 🤖
```bash
just android-build
```
The APK will be created at `androidApp/build/outputs/apk/debug/androidApp-debug.apk`.

### 5. Deploy the APK to a connected device 📲
```bash
just deploy
```
This rebuilds the debug APK if needed and installs it on the connected device with `adb install -r`.

## Useful Commands 🧰
- List all grouped commands: `just --list --unsorted`
- Build everything: `just build`
- Install/update the backend service: `just install`
- Build the Android debug APK: `just android-build`
- Deploy the Android debug APK: `just deploy`
- Server debug build: `just server-build`
- Server lint: `just server-lint`
