#!/usr/bin/env bash
# Build the debug APK on the configured build node. Run via `just build-android-remote`.
#
#   tools/build-android-remote.sh [patch|minor|major]
#
# probe node -> bump version here -> rsync working tree -> gradle there.
# Prints nothing to collect; the caller (`just _apk-collect`) fetches the APK.
#
# Source travels by rsync, not git push: the tree may be uncommitted and the node
# builds exactly what is on disk here.
set -euo pipefail
. "$(dirname "$0")/lib.sh"
cd "$REPO"

level="${1:-patch}"
[ -f config/build-node.env ] || die "no build node: just setup-build-node <ssh-dest>   (or: just build-android-local)"
# shellcheck source=/dev/null
. config/build-node.env
: "${BUILD_NODE_DEST:?config/build-node.env lacks BUILD_NODE_DEST}"
: "${BUILD_NODE_WS:?config/build-node.env lacks BUILD_NODE_WS}"
: "${BUILD_NODE_GRADLE_HOME:?config/build-node.env lacks BUILD_NODE_GRADLE_HOME}"
: "${BUILD_NODE_ANDROID_HOME:?config/build-node.env lacks BUILD_NODE_ANDROID_HOME}"
: "${BUILD_NODE_JDK:?config/build-node.env lacks BUILD_NODE_JDK}"
# --delete below is scoped to this directory. Refuse anything that is not a dedicated one.
case "$BUILD_NODE_WS" in
  /*/*/*) ;;
  *) die "BUILD_NODE_WS '$BUILD_NODE_WS' is too shallow to rsync --delete into" ;;
esac

# Three tries at 15s: a node on weak wifi answers in 2-5s, one tight probe would call it dead.
reachable=""
for _ in 1 2 3; do
  ssh -o BatchMode=yes -o ConnectTimeout=15 "$BUILD_NODE_DEST" true 2>/dev/null && { reachable=1; break; }
done
[ -n "$reachable" ] || die "build node $BUILD_NODE_DEST unreachable: wake it, or 'just build-android-local'"

# This machine stays the ONLY version bumper. Bump after the probe so a dead node
# does not burn a version.
tools/bump-version.sh "$level"
tools/apk-keystore.sh

t0=$SECONDS
echo "-> sync working tree to $BUILD_NODE_DEST:$BUILD_NODE_WS"
ssh "$BUILD_NODE_DEST" "mkdir -p '$BUILD_NODE_WS/secrets'"
# - ':- .gitignore' applies every .gitignore as excludes, so build output, caches and
#   node_modules neither travel nor get deleted on the node (excluded = protected).
# - --delete-after, not --delete: the receiver reads each .gitignore from ITS side, so
#   they must land before the delete pass or a first sync wipes the node's build dirs.
# - Deletion only ever touches $BUILD_NODE_WS (trailing-slash dir sync).
# - secrets/ never travels wholesale; the keystore goes alone, right after.
rsync -az --delete-after \
  --exclude='/.git/' --exclude='/secrets/' --exclude='/dist/' --exclude='/config/build-node.env' \
  --exclude='/.beads/' --exclude='/e2e/' \
  --filter=':- .gitignore' \
  ./ "$BUILD_NODE_DEST:$BUILD_NODE_WS/"
rsync -q --chmod=F600 secrets/debug.keystore "$BUILD_NODE_DEST:$BUILD_NODE_WS/secrets/debug.keystore"

echo "-> build $(prop VERSION_NAME) (code $(prop VERSION_CODE)) on $BUILD_NODE_DEST"
ssh "$BUILD_NODE_DEST" bash -s -- "$BUILD_NODE_WS" "$BUILD_NODE_GRADLE_HOME" "$BUILD_NODE_ANDROID_HOME" "$BUILD_NODE_JDK" <<'REMOTE'
set -euo pipefail
WS=$1; GRADLE_HOME=$2; SDK=$3; JDK=$4
export PATH="$HOME/.local/bin:/usr/local/bin:$PATH"   # non-login ssh shell
export GRADLE_USER_HOME="$GRADLE_HOME" ANDROID_HOME="$SDK" ANDROID_SDK_ROOT="$SDK" JAVA_HOME="$JDK"
cd "$WS"
just _apk-gradle
REMOTE
echo "remote build took $((SECONDS - t0))s"

daemons="$(ssh "$BUILD_NODE_DEST" 'pgrep -c -f GradleDaemon || true')"
[ "${daemons:-0}" = "0" ] || echo "note: $daemons gradle daemon(s) resident on $BUILD_NODE_DEST (idle timeout 30 min)"
