#!/usr/bin/env bash
# Ensure secrets/debug.keystore exists. Mints it once; never overwrites.
#
# The keystore is the app's INSTALL-OVER IDENTITY: an APK signed by a different key
# cannot update an installed build (Android refuses, the phone shows nothing useful).
# secrets/ is gitignored, so the file lives outside git. Back it up.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

KS="$REPO/secrets/debug.keystore"
kt="${JAVA_HOME:+$JAVA_HOME/bin/keytool}"
[ -n "$kt" ] && [ -x "$kt" ] || kt="$(command -v keytool || true)"
[ -n "$kt" ] || die "no keytool: '. tools/env.sh' for the JDK, or restore secrets/debug.keystore from backup"

fingerprint() {
  "$kt" -list -v -keystore "$KS" -storepass android -alias androiddebugkey 2>/dev/null \
    | sed -n 's/^[[:space:]]*SHA256:[[:space:]]*//p' | head -n 1 | tr -d ':' | tr 'A-F' 'a-f'
}

if [ -f "$KS" ]; then
  [ "${1:-}" = "--fingerprint" ] && { fingerprint; exit 0; }
  echo "keystore ok: secrets/debug.keystore ($(fingerprint | cut -c1-16)...)"
  exit 0
fi
[ "${1:-}" = "--fingerprint" ] && exit 0

mkdir -p "$REPO/secrets"
chmod 700 "$REPO/secrets"
"$kt" -genkeypair -keystore "$KS" -storepass android -keypass android \
  -alias androiddebugkey -keyalg RSA -keysize 2048 -validity 10950 \
  -dname "CN=Android Debug,O=Android,C=US" >/dev/null 2>&1
chmod 600 "$KS"
echo "WARNING minted a NEW secrets/debug.keystore ($(fingerprint | cut -c1-16)...) = a NEW install-over identity."
echo "  - Losing this file breaks install-over updates: every phone must uninstall first."
echo "  - Phones running an APK signed by another key must uninstall before this one installs."
echo "  - Gitignored. Back it up outside the repo."
