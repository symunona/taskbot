# taltosbot

Voice front end for your home machine. Talk on the phone, the work happens at home.

Formerly taskbot / hermes. Contract between the parts: `docs/protocol.md`.

## Parts

| part | dir | stack | job |
|---|---|---|---|
| app | `app/`, `androidApp/` | Kotlin Multiplatform, Compose | mic, speaker, Gemini Live (voice), Gemini `generateContent` (text), UI |
| server | `server/` | Rust, binary `taltosbot` | pairing, auth, read-only vault tools, spawns agent sessions |
| channel shim | `channel/` | Node | one per agent session: prompt in, `say` out |
| landing | `landing/` | one static HTML file | welcome page, hosts APK + sidecar |
| tooling | `justfile`, `tools/` | bash, just | build, version, release |

## Hosts

| role | host | notes |
|---|---|---|
| home machine | this one | server on port `55566`, reached over the tailnet |
| build node | ssh alias `fanni-air` | optional, builds the APK. Config `config/build-node.env` |
| web host | ssh alias `taskbot` | static root `~/wwwroot/taskbot.tmpx.space/`, public `https://taskbot.tmpx.space` |

## Prerequisites

- `just`, `rsync`, `curl`, `jq`, `ssh`
- JDK 17, Android SDK, Rust: paths come from `. tools/env.sh`
- Node (channel shim), `tmux`, `claude` (agent sessions)
- Caches and temp stay on the data SSD (`/mnt/data_ssd/cache`, `/mnt/data_ssd/tmp`). Never `/tmp`, never `~/.gradle`, never `~/.cargo`. Recipes source `tools/env.sh` themselves; in a bare shell run `. tools/env.sh` first.

## Quick start

```sh
just                      # list recipes
just setup-channel        # npm install in channel/
just install-server       # build, install ~/.local/bin/taltosbot, enable user service
$EDITOR ~/.config/taltosbot/config.toml   # vault_path, channel_dir, [keys], [projects.*]
just build-android-local  # APK -> dist/taltosbot.apk
just install-apk          # adb install onto the phone
just pair                 # QR for the phone to scan
```

## Recipes

| group | recipe | does |
|---|---|---|
| setup | `setup-build-node dest [ws]` | provision the build node over ssh, no sudo, only what is missing. Writes `config/build-node.env` |
| setup | `setup-channel` | `npm install` in `channel/` |
| setup | `keystore` | mint `secrets/debug.keystore` if missing |
| build | `bump [level]` | bump `version.properties` |
| build | `build-android [level]` | bump, build on the build node, collect, verify. Fails if no node or node offline |
| build | `build-android-remote [level]` | same, without the pre-check |
| build | `build-android-local [level]` | bump, build here, collect, verify |
| build | `verify-apk` | run the APK gate on `dist/taltosbot.apk` |
| build | `build-web` | `:app:jsBrowserDistribution` |
| build | `build-server` | `cargo build --release` |
| quality | `test-server` | `cargo test` |
| quality | `lint-server` | `cargo fmt --check` + `cargo clippy -D warnings` |
| quality | `fmt-server` | `cargo fmt` |
| quality | `lint-tools` | `bash -n` + `shellcheck` on `tools/*.sh` |
| run | `serve` | server in the foreground |
| run | `pair` | pairing QR + code |
| run | `status` | version, APK, keystore, build node, server health, service. Local only |
| run | `install-apk` | `adb install -r dist/taltosbot.apk` |
| deploy | `deploy-landing` | verify, rsync `landing/` + APK + sidecar to the web host, then `check-landing` |
| deploy | `check-landing` | public sidecar and APK size vs `dist/` |
| deploy | `release [level]` | `build-android` + `deploy-landing` |
| deploy | `install-server` | binary to `~/.local/bin`, user unit `taltosbot.service`, enable + restart |

`level` = `patch` (default) / `minor` / `major`.

## Versioning

| item | value |
|---|---|
| source | `version.properties`: `VERSION_NAME`, `VERSION_CODE`, `BUILD_EPOCH` (ms) |
| writer | `tools/bump-version.sh` only. Gradle and the tools read it |
| `VERSION_CODE` | `max(old + 1, minutes since 2024-01-01 UTC)`. Always increases, even when a bump was not committed |
| `BUILD_EPOCH` | bump time in ms. Sidecar `built_at` derives from it, never from "now" |
| bumped by | every `build-android*`. A build without a bump cannot update an installed app |

## APK release

| step | where | detail |
|---|---|---|
| bump | here | `tools/bump-version.sh` |
| build | node or here | `./gradlew :androidApp:assembleDebug`, signed with `secrets/debug.keystore` |
| collect | here | `dist/taltosbot.apk`; previous build kept as `dist/taltosbot.apk.prev` (+ `.prev.json`) |
| sidecar | here | `dist/taltosbot.apk.json` = `{version, version_code, size, built_at}` |
| verify | here | `tools/verify-apk.sh`: signer == previous APK and == keystore, manifest == `version.properties`, code > previous, sidecar size and `built_at` |
| deploy | web host | `just deploy-landing`: APK first, sidecar last, no `--delete` |
| check | public URL | served sidecar byte-equal to local, APK `Content-Length` == local size |

Signing key:

- `secrets/debug.keystore` = install-over identity of the app. Gitignored.
- Lose it: every phone must uninstall before the next build installs. Back it up outside the repo.
- Minted once by `tools/apk-keystore.sh`. Never overwritten.

## Build node

- `just setup-build-node fanni-air`: installs under `$HOME` on the node only what is missing: `just`, Temurin JDK 17 (`~/.jdks`), Android cmdline-tools, platform + build-tools for the project's `compileSdk`.
- Needs from the node's package manager: `curl unzip rsync tar`. Linux x86_64 only.
- Gradle home isolated: `<workspace>/../gradle-taltosbot`. Default workspace `~/build/taltosbot`.
- Source travels by `rsync` of the working tree, uncommitted changes included. `.gitignore` entries do not travel and are not deleted on the node. `secrets/` does not travel, the keystore is sent alone.
- Node offline: `just build-android-local`.

## App

Targets: Android (voice + text), web (text only). Rules: `docs/protocol.md` "Threads".

| where | what |
|---|---|
| top bar | menu (left), thread name, Talk toggle (outline idle / spinner connecting / red live), connection dot (tap = Settings), thread menu (right) |
| left drawer | Conversation, Sessions, Settings, Logs. Below: threads: name + last accessed, New thread, rename (pencil) |
| right drawer | the thread on screen: name, created, last used, entries, tokens (context now, all time in / out), Long CTX |
| Long CTX | second page of the right drawer: thread memory as the model gets it, COMPACT button |
| task strip | above the chat, one line per delegated task: project, state, what the agent does now, elapsed time. Tap: prompt, answer, error, times. Unfinished tasks + finished ones for 5 min, max 4 lines |
| chat | entries of one thread: user / model / agent bubbles, tool calls, voice markers, system notes |
| tool entry | one line, tool name + state (running / failed / no result). Tap: arguments, result, called at, result at |
| input | fixed at the bottom, grows to 6 lines. Enter sends, Shift+Enter breaks the line (hardware keyboard) |
| Settings | server, models, key state, "N tools" (open: name + description; tool open: parameters) |

- Voice idle: typed message = text turn, Gemini `generateContent`, same prompt + tools, tool loop until a text answer.
- Voice live: typed message goes into the voice session as a text turn.
- Context sent = thread memory + entries after its fold point (`docs/protocol.md` "Thread memory"). Server compacts: over 15 turns, voice stop, COMPACT button.
- Voice start on a thread with entries: memory + those entries ride in the Live system prompt. Caps without a memory: 40 entries / 12k chars (text: 80 / 40k).
- "context now" = prompt tokens of the newest Gemini call. `~` = estimate (chars / 4) until one happened on this device.
- Logs screen: `Context: ...` line per text turn and voice start = chars sent.
- Every entry stored with `threads.append`, debounced 0.6 s, upsert by `id`. Offline: queued, sent on reconnect.
- Stored tool result capped at 60k chars (`{truncated, chars, preview}`). The model gets the full result.
- Last thread id remembered, reopened on connect. No thread: first message / voice start creates one.
- Disconnected: chat readable, send refused with a system note, draft kept.
- Tasks: `tasks.list` on every connect, then `tasks.changed` pushes. Shown, never sent to Gemini: a text turn cuts into live speech. Model hears the answer (`agent.say`) and asks with the `tasks` tool. Rules: `docs/protocol.md` "Tasks".
- Voice cues (synthesized, no assets): two-note tick when `web_search` / `search_regexp` starts, chime when `agent.say` arrives. The answer itself is queued: read out after the current speech has finished playing.

Web smoke check, no real server, no Gemini:

| step | command |
|---|---|
| build | `just build-web` |
| run | `e2e/web-check.sh` (`AGENT_BROWSER_ARGS=--no-sandbox` where Chromium has no usable sandbox; `WEB_PORT`, `API_PORT` when 8765 / 55577 are taken) |
| result | screenshots + mock log in `e2e/evidence/`. Judge by eye |

- `e2e/mock-server.mjs`: protocol mock + fake Gemini text endpoint. Mock-only routes: file header.
- Mock `delegate` plays one whole task in ~8 s (queued → working → done). `POST /mock/task` pushes any `Task` by hand.
- Test seam: storage key `gemini_text_base_url` moves text mode off Google onto that URL. Unset in normal use.

## Server

| item | value |
|---|---|
| config | `$TALTOSBOT_CONFIG`, else `~/.config/taltosbot/config.toml`. Example `server/config.example.toml` |
| config keys | `vault_path` (required), `listen_addr`, `listen_port`, `gemini_model` (voice), `gemini_text_model` (text mode, default `gemini-flash-latest`), `gemini_summary_model` (thread memory compacts, default = text model), `channel_dir`, `channel_port_base`, `public_urls`, `[keys]`, `[projects.*]` |
| state | `~/.local/state/taltosbot/`. sqlite: devices, pair codes, shim ports, threads, tasks |
| tasks | `server/src/tasks.rs`. One `Task` per `delegate` call, own table. Every change pushed as `tasks.changed`. Survives a restart. Shim gone → task `failed` within 20 s |
| service | `systemctl --user status taltosbot`, logs `journalctl --user -u taltosbot -f` |
| unit template | `tools/taltosbot.service.in`. `PATH` pinned at install time (tmux, node, claude) |
| run without a login session | `loginctl enable-linger $USER` |

## Layout

| path | content |
|---|---|
| `justfile` | all recipes |
| `tools/env.sh` | toolchain + cache paths |
| `tools/lib.sh` | shared helpers for the scripts |
| `tools/bump-version.sh`, `apk-keystore.sh`, `write-apk-sidecar.sh`, `verify-apk.sh` | version + APK tooling |
| `tools/setup-build-node.sh`, `build-android-remote.sh` | build node |
| `tools/taltosbot.service.in` | systemd user unit template |
| `version.properties` | version source |
| `config/build-node.env` | build node, machine-local, gitignored. Shape: `config/build-node.env.example` |
| `secrets/` | keystore, gitignored |
| `dist/` | collected APK + sidecar, gitignored |
| `landing/index.html` | the site |
| `e2e/mock-server.mjs`, `e2e/web-check.sh` | web smoke check against a mock server |
| `docs/protocol.md` | contract: pairing, HTTP, WebSocket, tasks, threads, tools, APK update |
