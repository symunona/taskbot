#!/usr/bin/env bash
# Register + provision a remote Android build node. Run via `just setup-build-node`.
#
#   tools/setup-build-node.sh <ssh-dest> [workspace]
#
# - No sudo. Everything lands under $HOME on the node; present tools are reused.
# - Installs only what is missing: just, JDK 17 (Temurin), Android cmdline-tools,
#   platform + build-tools for the project's compileSdk.
# - Isolated GRADLE_USER_HOME next to the workspace, sized from the node's hardware.
# - Ships secrets/debug.keystore. Writes config/build-node.env (gitignored).
# Idempotent: rerun after a toolchain change.
set -euo pipefail
. "$(dirname "$0")/lib.sh"
cd "$REPO"

DEST="${1:-}"; WS="${2:-}"
[ -n "$DEST" ] || die "usage: tools/setup-build-node.sh <ssh-dest> [workspace]"
ssh -o BatchMode=yes -o ConnectTimeout=10 "$DEST" true 2>/dev/null \
  || die "cannot ssh to $DEST: needs a reachable host and a key that logs in without a password"

# shellcheck disable=SC2016  # $HOME must expand on the node
[ -n "$WS" ] || WS="$(ssh "$DEST" 'echo "$HOME/build/taltosbot"')"
case "$WS" in /*/*) ;; *) die "workspace '$WS' must be an absolute path below a parent directory" ;; esac
GRADLE_HOME="$(dirname "$WS")/gradle-taltosbot"

# What the project compiles against, read from the Android module (Gradle is the source).
SDK_API="$(sed -n 's/^[[:space:]]*compileSdk[[:space:]]*=[[:space:]]*\([0-9][0-9]*\).*/\1/p' androidApp/build.gradle.kts | head -n 1)"
[ -n "$SDK_API" ] || die "could not read compileSdk from androidApp/build.gradle.kts"

echo "-- provisioning $DEST (workspace $WS, compileSdk $SDK_API) --"
ssh "$DEST" bash -s -- "$WS" "$GRADLE_HOME" "$SDK_API" <<'REMOTE'
set -euo pipefail
WS=$1; GRADLE_HOME=$2; SDK_API=$3
export PATH="$HOME/.local/bin:/usr/local/bin:$PATH"   # non-login ssh shell

[ "$(uname -s)" = "Linux" ] || { echo "FAIL build node must be Linux, got $(uname -s)"; exit 1; }
case "$(uname -m)" in
  x86_64) ARCH=x64 ;;
  *) echo "FAIL build node is $(uname -m): Google ships Android build-tools (aapt2) for linux x86_64 only"; exit 1 ;;
esac

# Base tools need root to install, so they are a precondition, not something to provision.
missing=""
for t in curl unzip rsync tar; do command -v "$t" >/dev/null || missing="$missing $t"; done
[ -z "$missing" ] || { echo "FAIL build node needs these from its package manager:$missing"; exit 1; }
mkdir -p "$HOME/.local/bin" "$WS/secrets" "$GRADLE_HOME"

if ! command -v just >/dev/null; then
  echo "-> installing just into ~/.local/bin"
  curl --proto '=https' --tlsv1.2 -sSf https://just.systems/install.sh | bash -s -- --to "$HOME/.local/bin"
  hash -r
fi
command -v just >/dev/null || { echo "FAIL just still missing after install"; exit 1; }

# JDK 17: reuse any 17 already on the node, else Temurin under ~/.jdks.
is_jdk17() { [ -x "$1/bin/javac" ] && "$1/bin/java" -version 2>&1 | grep -q 'version "17\.'; }
JDK=""
for c in "${JAVA_HOME:-}" "$HOME"/.jdks/*17* /usr/lib/jvm/*17* "$HOME"/.sdkman/candidates/java/17*; do
  [ -n "$c" ] && [ -d "$c" ] && is_jdk17 "$c" && { JDK="$c"; break; }
done
if [ -z "$JDK" ]; then
  echo "-> installing Temurin JDK 17 into ~/.jdks"
  mkdir -p "$HOME/.jdks"
  curl -fsSL "https://api.adoptium.net/v3/binary/latest/17/ga/linux/$ARCH/jdk/hotspot/normal/eclipse" \
    | tar xz -C "$HOME/.jdks"
  JDK="$(ls -d "$HOME"/.jdks/jdk-17* 2>/dev/null | sort -V | tail -n 1)"
  [ -n "$JDK" ] && is_jdk17 "$JDK" || { echo "FAIL Temurin 17 did not unpack under ~/.jdks"; exit 1; }
fi

# Android SDK: reuse an existing one (Android Studio's), else start one under $HOME.
SDK=""
for c in "${ANDROID_HOME:-}" "${ANDROID_SDK_ROOT:-}" "$HOME/Android/Sdk" "$HOME/android-sdk"; do
  [ -n "$c" ] && [ -d "$c" ] && { SDK="$c"; break; }
done
[ -n "$SDK" ] || SDK="$HOME/Android/Sdk"
mkdir -p "$SDK"
[ -w "$SDK" ] || { echo "FAIL Android SDK at $SDK is not writable by $USER (no sudo here)"; exit 1; }
if [ ! -x "$SDK/cmdline-tools/latest/bin/sdkmanager" ]; then
  echo "-> installing Android cmdline-tools into $SDK"
  tmp="$(mktemp -d "$SDK/.clt.XXXXXX")"; trap 'rm -rf "$tmp"' EXIT
  curl -fsSL -o "$tmp/clt.zip" https://dl.google.com/android/repository/commandlinetools-linux-13114758_latest.zip
  unzip -q "$tmp/clt.zip" -d "$tmp"
  mkdir -p "$SDK/cmdline-tools/latest"
  mv "$tmp"/cmdline-tools/* "$SDK/cmdline-tools/latest/"
fi
export ANDROID_HOME="$SDK" ANDROID_SDK_ROOT="$SDK" JAVA_HOME="$JDK"
export PATH="$SDK/cmdline-tools/latest/bin:$SDK/platform-tools:$PATH"
# Accepted licenses let AGP fetch whatever else the build asks for, so the explicit
# installs below are a warm-up: version drift resolves itself on the first build.
yes | sdkmanager --licenses >/dev/null 2>&1 || true
want=""
[ -d "$SDK/platform-tools" ] || want="$want platform-tools"
[ -d "$SDK/platforms/android-$SDK_API" ] || want="$want platforms;android-$SDK_API"
ls -d "$SDK/build-tools/$SDK_API".* >/dev/null 2>&1 || want="$want build-tools;$SDK_API.0.0"
if [ -n "$want" ]; then
  echo "-> sdkmanager --install$want"
  # shellcheck disable=SC2086  # word-split the package list
  sdkmanager --install $want >/dev/null 2>&1 \
    || echo "warn SDK warm-up failed, AGP will fetch what it needs on the first build"
fi

# Gradle home: cache + memory tuning, isolated from the node's own ~/.gradle.
cores=$(nproc); memgb=$(awk '/MemTotal/{printf "%d", $2/1048576}' /proc/meminfo)
workers=$(( cores - 2 )); [ "$workers" -ge 1 ] || workers=1; [ "$workers" -le 8 ] || workers=8
heap=$(( memgb / 4 )); [ "$heap" -ge 2 ] || heap=2; [ "$heap" -le 6 ] || heap=6
cat > "$GRADLE_HOME/gradle.properties" <<EOF
# Written by tools/setup-build-node.sh, sized for ${cores} cores / ${memgb}GB.
org.gradle.java.installations.auto-download=false
org.gradle.java.installations.paths=$JDK
org.gradle.daemon=true
org.gradle.parallel=true
org.gradle.workers.max=$workers
org.gradle.caching=true
org.gradle.jvmargs=-Xmx${heap}g -XX:MaxMetaspaceSize=1g -Dfile.encoding=UTF-8
# Default socket timeout is 30s; one artifact can be 60MB. Costs a fast node nothing.
systemProp.org.gradle.internal.http.connectionTimeout=120000
systemProp.org.gradle.internal.http.socketTimeout=300000
# The node is somebody's desktop: do not sit on the heap for Gradle's default 3h.
org.gradle.daemon.idletimeout=1800000
EOF

# Report back; the caller turns this into config/build-node.env.
cat > "$GRADLE_HOME/.build-node.env" <<EOF
BUILD_NODE_ANDROID_HOME=$SDK
BUILD_NODE_JDK=$JDK
EOF
echo "  just   : $(command -v just) $(just --version | cut -d' ' -f2)"
echo "  jdk    : $JDK"
echo "  sdk    : $SDK (platforms: $(ls "$SDK/platforms" 2>/dev/null | tr '\n' ' '))"
echo "  gradle : $GRADLE_HOME ($workers workers, -Xmx${heap}g)"
echo "  space  : $(df -h --output=avail "$WS" | tail -n 1 | tr -d ' ') free at $WS"
REMOTE

# The keystore is gitignored and excluded from the tree sync, so it travels on its own.
# Without it the node's build is signed by another key and will not install over.
tools/apk-keystore.sh
rsync -q --chmod=F600 secrets/debug.keystore "$DEST:$WS/secrets/debug.keystore"

mkdir -p config
{
  echo "# Written by 'just setup-build-node $DEST'. Gitignored (machine-local)."
  echo "BUILD_NODE_DEST=$DEST"
  echo "BUILD_NODE_WS=$WS"
  echo "BUILD_NODE_GRADLE_HOME=$GRADLE_HOME"
  ssh "$DEST" "cat '$GRADLE_HOME/.build-node.env'"
} > config/build-node.env.tmp
mv config/build-node.env.tmp config/build-node.env

echo ""
echo "ok build node $DEST ready. 'just build-android' now builds there."
echo "  config: config/build-node.env    fallback: just build-android-local"
echo "  first build downloads the Gradle distribution + dependencies (slow once)."
