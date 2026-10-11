# Agent instructions

Project: taltosbot. Overview, recipes, layout: `README.md`. Contract between parts: `docs/protocol.md`.

## Rules

- Ask before executing a plan, unless the prompt says to skip that.
- Doc style: terse keywords, bullets and tables over prose, complete facts, zero filler.
- Protocol change: edit `docs/protocol.md` first, then code.
- Use `just` recipes. New repeatable step: add a recipe, document it in `README.md`.
- Not done until the linter is clean and a local build passed (see Done checklist).
- Never commit `secrets/`, `dist/`, `config/build-node.env`, `server/config.toml`, API keys.
- Never regenerate or overwrite `secrets/debug.keystore`. It is the app's install-over identity.

## Environment

- `. tools/env.sh` before any bare `./gradlew`, `cargo`, `adb`, `keytool`. `just` recipes source it themselves.
- Sets `JAVA_HOME`, `GRADLE_USER_HOME`, `CARGO_HOME`, `RUSTUP_HOME`, `TMPDIR`, `ANDROID_HOME`.
- Caches and temp stay on the data SSD: `/mnt/data_ssd/cache`, `/mnt/data_ssd/tmp`.
- Never write to `/tmp`, `~/.gradle`, `~/.cargo`, `~/.cache`. Temp files: `$TMPDIR`.
- Reason: build and cache churn on the system SSD wore it out. See `/mnt/data_ssd/CLAUDE.md`.
- Do not edit `tools/env.sh` to point anything back at `/`.

## Server (`server/`, Rust)

| task | command |
|---|---|
| build | `just build-server` |
| test | `just test-server` |
| lint | `just lint-server` (`cargo fmt --check` + `cargo clippy --all-targets -- -D warnings`) |
| format | `just fmt-server` |
| clippy auto-fix | `. tools/env.sh && cargo clippy --manifest-path server/Cargo.toml --fix --allow-dirty --allow-staged` |
| run | `just serve` |

- After each task: run the linter, fix every finding, then commit.
- Do not ignore or silence linter warnings. Fix them.
- Before calling a feature done: local build + tests.

## App (`app/`, `androidApp/`, Kotlin Multiplatform)

| task | command |
|---|---|
| debug APK, no version bump | `. tools/env.sh && ./gradlew :androidApp:assembleDebug` |
| release APK into `dist/` | `just build-android-local` (or `just build-android` with a build node) |
| web build | `just build-web` |
| install on phone | `just install-apk` |

- Version: `version.properties`, written only by `tools/bump-version.sh`. Never hand-edit `versionCode` in Gradle.
- An APK that goes to a phone or to the landing must come from `just build-android*`: bump, sidecar, verify.
- `tools/verify-apk.sh` fails: do not deploy. Fix the cause (signer, version, sidecar).

Android test cycle:

| step | command |
|---|---|
| build + install | `just build-android-local && just install-apk` |
| clear logs | `adb logcat -c` |
| start | `adb shell monkey -p <applicationId> -c android.intent.category.LAUNCHER 1` (`applicationId` in `androidApp/build.gradle.kts`) |
| find a control | `adb shell uiautomator dump && adb shell cat /sdcard/window_dump.xml \| grep -o 'content-desc="<label>"[^>]*'` |
| tap | `adb shell input tap <x> <y>` |
| read logs | `adb logcat -d \| grep -i "<tag>"` |

## Channel shim (`channel/`, Node)

- Install deps: `just setup-channel`.
- Server shells out to `channel/spawn-session.sh`. Interface: `docs/protocol.md` "Agent sessions".

## Tooling (`justfile`, `tools/`)

- After changing a script: `just lint-tools` (`bash -n` + `shellcheck`).
- After changing the justfile: `just --list`, `just --dry-run <recipe>`.
- Scripts source `tools/lib.sh`. Portable shell only: this host runs uutils coreutils (no `date +%3N`).

## Remote hosts

| host | touched by | rule |
|---|---|---|
| build node (`config/build-node.env`) | `setup-build-node`, `build-android`, `build-android-remote` | no sudo, writes only under `$HOME`; `rsync --delete-after` only inside the workspace |
| web host `taskbot` | `deploy-landing`, `release` | rsync into `~/wwwroot/taskbot.tmpx.space/` only, no `--delete` |

- Run a deploy or a remote build only when the user asked for it.

## Done checklist

1. `just lint-server` clean (server changed).
2. `just test-server` passes (server changed).
3. `just build-server` / Gradle build passes for what changed.
4. `just lint-tools` clean (scripts changed).
5. `README.md`, `docs/protocol.md` match the change.
