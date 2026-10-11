# taltosbot task runner — https://github.com/casey/just
# `just` lists recipes by group. Every recipe sources tools/env.sh: toolchains and
# caches live on the data SSD, never on `/`.

set shell := ["bash", "-euo", "pipefail", "-c"]

# ── Variables ─────────────────────────────────────────────────────────────────

# Landing host: ssh alias, static root (relative to its $HOME), public URL.
web_host := "taskbot"
web_root := "wwwroot/taskbot.tmpx.space"
web_url  := "https://taskbot.tmpx.space"

# Collected release artifact (+ .json sidecar, + .prev copies). Names = docs/protocol.md "APK update".
apk      := "dist/taltosbot.apk"
# Where Gradle leaves the debug build.
apk_out  := "androidApp/build/outputs/apk/debug/androidApp-debug.apk"

server_manifest := "server/Cargo.toml"
server_bin      := "server/target/release/taltosbot"
port            := env("TALTOSBOT_PORT", "55566")
env_sh          := ". '" + justfile_directory() + "/tools/env.sh'"

# List available recipes
default:
    @just --list

# ── Setup ─────────────────────────────────────────────────────────────────────

[group('setup')]
[doc('Register + provision a remote Android build node (dest = ssh alias|user@host). No sudo, only what is missing: just, JDK 17, Android SDK, isolated gradle home. Writes config/build-node.env')]
setup-build-node dest ws="":
    tools/setup-build-node.sh "{{dest}}" "{{ws}}"

[group('setup')]
[doc('npm install for the channel shim (channel/)')]
setup-channel:
    cd channel && npm install --no-audit --no-fund

[group('setup')]
[doc('Mint secrets/debug.keystore if missing (the APK install-over identity). Never overwrites')]
keystore:
    tools/apk-keystore.sh

# ── Build ─────────────────────────────────────────────────────────────────────

[group('build')]
[doc('Bump version.properties (level=patch|minor|major): versionCode = max(old+1, minutes since 2024), BUILD_EPOCH = now')]
bump level="patch":
    tools/bump-version.sh {{level}}

[group('build')]
[doc('Bump + build the APK on the build node, collect to dist/, verify. Fails (no silent local fallback) when no node is configured or it is offline (level=patch|minor|major)')]
build-android level="patch":
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}"
    [ -f config/build-node.env ] || {
      echo "FAIL no build node configured."
      echo "  set one up:  just setup-build-node <ssh-dest> [workspace]"
      echo "  build here:  just build-android-local {{level}}"
      exit 1
    }
    . config/build-node.env
    ssh -o BatchMode=yes -o ConnectTimeout=15 "$BUILD_NODE_DEST" true 2>/dev/null || {
      echo "FAIL build node $BUILD_NODE_DEST does not answer ssh."
      echo "  wake it and retry, or build here:  just build-android-local {{level}}"
      exit 1
    }
    just build-android-remote {{level}}

[group('build')]
[doc('Bump, rsync the working tree (uncommitted included) to the build node, build there, fetch the APK, verify (level=patch|minor|major)')]
build-android-remote level="patch":
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}"
    tools/build-android-remote.sh {{level}}
    . config/build-node.env
    just _apk-collect "${BUILD_NODE_DEST}:${BUILD_NODE_WS}/{{apk_out}}"

[group('build')]
[doc('Bump + build the APK on THIS machine, collect to dist/, verify (level=patch|minor|major)')]
build-android-local level="patch":
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}"
    tools/bump-version.sh {{level}}
    tools/apk-keystore.sh
    t0=$SECONDS
    just _apk-gradle
    echo "local build took $((SECONDS - t0))s"
    just _apk-collect "{{apk_out}}"

# The one Gradle invocation, shared by the local and the remote path so they cannot
# drift. Host specifics come from the environment (JAVA_HOME, ANDROID_HOME,
# GRADLE_USER_HOME): tools/env.sh here, tools/build-android-remote.sh on the node.
# Gradle reads version.properties and signs with secrets/debug.keystore.
_apk-gradle:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}"
    {{env_sh}}
    [ -f secrets/debug.keystore ] || { echo "FAIL secrets/debug.keystore missing: the APK would get a throwaway signer"; exit 1; }
    rm -f "{{apk_out}}"    # a failed build must not leave the previous APK to be collected
    nice -n 10 ./gradlew :androidApp:assembleDebug
    [ -f "{{apk_out}}" ] || { echo "FAIL gradle finished without {{apk_out}}"; exit 1; }

# Take a freshly built APK (src = local path or rsync host:path), keep the outgoing
# one as .prev (the verify baseline), write the sidecar from the LOCAL
# version.properties, verify. Shared by both build paths.
_apk-collect src:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}"
    APK="{{apk}}"
    mkdir -p "$(dirname "$APK")"
    if [ -f "$APK" ]; then
      cp -p "$APK" "$APK.prev"
      if [ -f "$APK.json" ]; then cp -p "$APK.json" "$APK.prev.json"; else rm -f "$APK.prev.json"; fi
    fi
    rsync -q "{{src}}" "$APK"
    tools/write-apk-sidecar.sh "$APK"
    echo "APK -> $APK ($(du -h "$APK" | cut -f1))"
    tools/verify-apk.sh "$APK"

[group('build')]
[doc('Verify dist/taltosbot.apk: signer, version vs version.properties, versionCode order, sidecar')]
verify-apk:
    tools/verify-apk.sh "{{apk}}"

[group('build')]
[doc('Web build of the app -> app/build/dist/js/productionExecutable')]
build-web:
    {{env_sh}} && ./gradlew :app:jsBrowserDistribution

[group('build')]
[doc('Release build of the server -> server/target/release/taltosbot')]
build-server:
    {{env_sh}} && cargo build --release --manifest-path {{server_manifest}}

# ── Quality ───────────────────────────────────────────────────────────────────

[group('quality')]
[doc('Server unit + integration tests')]
test-server:
    {{env_sh}} && cargo test --manifest-path {{server_manifest}}

[group('quality')]
[doc('Server lint: rustfmt check + clippy, warnings are errors. Run before calling a server change done')]
lint-server:
    {{env_sh}} && cargo fmt --manifest-path {{server_manifest}} --check
    {{env_sh}} && cargo clippy --manifest-path {{server_manifest}} --all-targets -- -D warnings

[group('quality')]
[doc('Format the server crate')]
fmt-server:
    {{env_sh}} && cargo fmt --manifest-path {{server_manifest}}

[group('quality')]
[doc('bash -n + shellcheck over tools/*.sh and channel/*.sh')]
lint-tools:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}"
    for f in tools/*.sh channel/*.sh; do [ -f "$f" ] && bash -n "$f"; done
    if command -v shellcheck >/dev/null; then shellcheck -x -S warning tools/*.sh; else echo "shellcheck missing, bash -n only"; fi

# ── Run ───────────────────────────────────────────────────────────────────────

[group('run')]
[doc('Run the server in the foreground (config: $TALTOSBOT_CONFIG, else ~/.config/taltosbot/config.toml)')]
serve: build-server
    {{server_bin}} serve

[group('run')]
[doc('Print a pairing QR + code for the phone (needs the server running on this machine)')]
pair: build-server
    {{server_bin}} pair

[group('run')]
[doc('Local state: version, collected APK, build node config, server health, service')]
status:
    #!/usr/bin/env bash
    set -uo pipefail
    cd "{{justfile_directory()}}"
    . tools/lib.sh
    echo "version   : $(prop VERSION_NAME) / code $(prop VERSION_CODE) / built_at $(epoch_ms_to_iso "$(prop BUILD_EPOCH)" 2>/dev/null)"
    if [ -f "{{apk}}" ]; then
      echo "apk       : {{apk}} ($(du -h "{{apk}}" | cut -f1)) sidecar $(cat "{{apk}}.json" 2>/dev/null || echo MISSING)"
    else
      echo "apk       : none (just build-android)"
    fi
    if [ -f secrets/debug.keystore ]; then
      echo "keystore  : secrets/debug.keystore $(tools/apk-keystore.sh --fingerprint | cut -c1-16)..."
    else
      echo "keystore  : MISSING (just keystore)"
    fi
    if [ -f config/build-node.env ]; then
      echo "build node: $(sed -n 's/^BUILD_NODE_DEST=//p' config/build-node.env):$(sed -n 's/^BUILD_NODE_WS=//p' config/build-node.env) (not probed)"
    else
      echo "build node: none (just setup-build-node <ssh-dest>)"
    fi
    health="$(curl -fsS -m 3 "http://127.0.0.1:{{port}}/api/v1/health" 2>/dev/null || true)"
    echo "server    : ${health:-not answering on 127.0.0.1:{{port}}}"
    if systemctl --user cat taltosbot.service >/dev/null 2>&1; then
      echo "service   : $(systemctl --user is-active taltosbot.service 2>/dev/null || true) / $(systemctl --user is-enabled taltosbot.service 2>/dev/null || true)"
    else
      echo "service   : not installed (just install-server)"
    fi
    echo "landing   : {{web_url}} (just check-landing)"

[group('run')]
[doc('adb install -r dist/taltosbot.apk onto the connected phone')]
install-apk:
    {{env_sh}} && adb install -r "{{apk}}"

# ── Deploy ────────────────────────────────────────────────────────────────────

[group('deploy')]
[doc('Verify, then rsync landing/ + dist/taltosbot.apk + sidecar to the web host, then check-landing. No --delete')]
deploy-landing:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}"
    APK="{{apk}}"; DEST="{{web_host}}:{{web_root}}/"
    [ -f "$APK" ] && [ -f "$APK.json" ] || { echo "FAIL no $APK + sidecar: just build-android"; exit 1; }
    [ -f landing/index.html ] || { echo "FAIL landing/index.html missing"; exit 1; }
    tools/verify-apk.sh "$APK"
    ssh -o BatchMode=yes -o ConnectTimeout=15 "{{web_host}}" "mkdir -p '{{web_root}}'"
    # APK first, sidecar last: the sidecar must never advertise a build that is not there yet.
    rsync -az --chmod=D755,F644 landing/ "$APK" "$DEST"
    rsync -az --chmod=F644 "$APK.json" "$DEST"
    echo "deployed -> $DEST"
    just check-landing

[group('deploy')]
[doc('Compare what the public URL serves (sidecar, APK size) with dist/')]
check-landing:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}"
    APK="{{apk}}"; URL="{{web_url}}/$(basename "{{apk}}")"
    [ -f "$APK.json" ] || { echo "FAIL no local $APK.json to compare with"; exit 1; }
    served="$(curl -fsS -m 20 "$URL.json?t=$(date +%s)")" || { echo "FAIL $URL.json not reachable"; exit 1; }
    echo "local  : $(cat "$APK.json")"
    echo "served : $served"
    [ "$served" = "$(cat "$APK.json")" ] || { echo "FAIL public sidecar differs from $APK.json"; exit 1; }
    want="$(stat -c %s "$APK")"
    got="$(curl -fsSI -m 20 "$URL" | tr -d '\r' | awk 'tolower($1)=="content-length:"{v=$2} END{print v}')"
    [ "$got" = "$want" ] || { echo "FAIL $URL Content-Length ${got:-?} != local $want"; exit 1; }
    curl -fsS -m 20 -o /dev/null "{{web_url}}/" || { echo "FAIL {{web_url}}/ not reachable"; exit 1; }
    echo "ok {{web_url}} serves this build ($want bytes)"

[group('deploy')]
[doc('build-android + deploy-landing (level=patch|minor|major)')]
release level="patch":
    just build-android {{level}}
    just deploy-landing

[group('deploy')]
[doc('Install the server on THIS machine: release binary -> ~/.local/bin/taltosbot, user unit taltosbot.service, enable + (re)start')]
install-server: build-server
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}"
    BIN="$HOME/.local/bin/taltosbot"
    UNIT_DIR="$HOME/.config/systemd/user"
    mkdir -p "$(dirname "$BIN")" "$UNIT_DIR"
    # Replace via rename: copying over a running binary fails with "Text file busy".
    install -m 755 "{{server_bin}}" "$BIN.new" && mv -f "$BIN.new" "$BIN"
    # User units get a bare PATH. The server shells out to channel/spawn-session.sh,
    # which needs tmux, node and claude: resolve them now and pin their directories.
    path="$HOME/.local/bin"
    for t in tmux node claude; do
      p="$(command -v "$t" || true)"
      if [ -n "$p" ]; then path="$path:$(dirname "$(readlink -f "$p")")"; else echo "warn '$t' not on PATH: agent sessions will not spawn"; fi
    done
    path="$path:/usr/local/bin:/usr/bin:/bin"
    sed -e "s|@BIN@|$BIN|g" -e "s|@PATH@|$path|g" -e "s|@REPO@|{{justfile_directory()}}|g" \
      tools/taltosbot.service.in > "$UNIT_DIR/taltosbot.service"
    systemctl --user daemon-reload
    systemctl --user enable taltosbot.service
    systemctl --user restart taltosbot.service
    sleep 1
    systemctl --user --no-pager --lines=5 status taltosbot.service || true
    echo "config: ${TALTOSBOT_CONFIG:-$HOME/.config/taltosbot/config.toml} (written with defaults on first start; set vault_path, channel_dir, keys)"
    echo "logs  : journalctl --user -u taltosbot -f      pair: taltosbot pair"
    echo "boot  : 'loginctl enable-linger $USER' keeps it running without a login session"
