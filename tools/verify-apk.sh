#!/usr/bin/env bash
# Post-build gate for an Android APK. Run by `just build-android*` and `just deploy-landing`.
#
# Usage: tools/verify-apk.sh <apk> [--against <old-apk>] [--sidecar <json>]
#   --against  default <apk>.prev        (its sidecar: <old-apk>.json)
#   --sidecar  default <apk>.json
#
# Checks:
#   1 signer cert == previous APK's (when one exists) and == secrets/debug.keystore.
#     A different key installs fine on a clean phone and is silently REFUSED as an
#     update. This is the only cheap place to catch it before the phone does.
#   2 manifest versionName/versionCode == version.properties.
#   3 versionCode strictly greater than the previous sidecar's.
#   4 sidecar: size == file size, version fields == version.properties,
#     built_at == BUILD_EPOCH (not "now", see tools/write-apk-sidecar.sh).
# Exit 0 all ok, 1 a check failed, 2 cannot run.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

APK=""; AGAINST=""; SIDECAR=""
while [ $# -gt 0 ]; do
  case "$1" in
    --against) AGAINST="$2"; shift 2 ;;
    --sidecar) SIDECAR="$2"; shift 2 ;;
    -h|--help) sed -n '2,16p' "$0"; exit 0 ;;
    *) APK="$1"; shift ;;
  esac
done
[ -n "$APK" ] || { echo "usage: tools/verify-apk.sh <apk> [--against <old-apk>] [--sidecar <json>]" >&2; exit 2; }
[ -f "$APK" ] || { echo "FAIL no APK at $APK (just build-android)" >&2; exit 2; }
: "${SIDECAR:=$APK.json}"
: "${AGAINST:=$APK.prev}"
command -v jq >/dev/null || { echo "FAIL jq missing" >&2; exit 2; }
BT="$(build_tools_dir)" || { echo "FAIL no build-tools with apksigner + aapt2 under $ANDROID_HOME/build-tools" >&2; exit 2; }

FAIL=0
ok()   { printf '  ok    %s\n' "$1"; }
bad()  { printf '  FAIL  %s\n' "$1"; FAIL=1; }
warn() { printf '  warn  %s\n' "$1"; }
signer() { "$BT/apksigner" verify --print-certs "$1" 2>/dev/null | sed -n 's/^Signer #1 certificate SHA-256 digest: //p'; }

echo "-- verify $(basename "$APK") (build-tools $(basename "$BT")) --"

WANT_NAME="$(prop VERSION_NAME)"; WANT_CODE="$(prop VERSION_CODE)"; WANT_EPOCH="$(prop BUILD_EPOCH)"

# 1 - signer identity.
NEW_SIGNER="$(signer "$APK")"
[ -n "$NEW_SIGNER" ] || bad "apksigner could not read a signer cert (unsigned or corrupt APK)"
if [ -f "$AGAINST" ]; then
  OLD_SIGNER="$(signer "$AGAINST")"
  if [ -n "$NEW_SIGNER" ] && [ "$NEW_SIGNER" = "$OLD_SIGNER" ]; then
    ok "signer unchanged (${NEW_SIGNER:0:16}...), installs over $(basename "$AGAINST")"
  else
    bad "SIGNER CHANGED ${OLD_SIGNER:0:16}... -> ${NEW_SIGNER:0:16}... : phones will refuse this update (uninstall first)"
  fi
else
  warn "no previous APK at $(basename "$AGAINST"), signer ${NEW_SIGNER:0:16}... not compared to a predecessor"
fi
KS_SIGNER="$("$REPO/tools/apk-keystore.sh" --fingerprint 2>/dev/null || true)"
if [ -n "$KS_SIGNER" ]; then
  if [ "$NEW_SIGNER" = "$KS_SIGNER" ]; then
    ok "signed with secrets/debug.keystore"
  else
    bad "NOT signed with secrets/debug.keystore (${KS_SIGNER:0:16}...): the build used another key"
  fi
else
  warn "no secrets/debug.keystore here, signer not checked against it"
fi

# 2 - manifest version. sed -n 1p, not head -1: head closes the pipe early, aapt2 dies
# on SIGPIPE, pipefail aborts.
BADGING="$("$BT/aapt2" dump badging "$APK" 2>/dev/null | sed -n '1p')"
GOT_CODE="$(printf '%s' "$BADGING" | sed -n "s/.*versionCode='\([0-9]*\)'.*/\1/p")"
GOT_NAME="$(printf '%s' "$BADGING" | sed -n "s/.*versionName='\([^']*\)'.*/\1/p")"
if [ -n "$GOT_CODE" ] && [ "$GOT_CODE" = "$WANT_CODE" ] && [ "$GOT_NAME" = "$WANT_NAME" ]; then
  ok "manifest $GOT_NAME / code $GOT_CODE == version.properties"
else
  bad "manifest ${GOT_NAME:-?} / code ${GOT_CODE:-?} != version.properties ${WANT_NAME:-?} / ${WANT_CODE:-?}"
fi

# 3 - strictly greater than the build being replaced.
if [ -f "$AGAINST.json" ]; then
  PREV_CODE="$(json_field "$AGAINST.json" version_code)"
  if [[ "$PREV_CODE" =~ ^[0-9]+$ ]] && [[ "$GOT_CODE" =~ ^[0-9]+$ ]] && [ "$GOT_CODE" -gt "$PREV_CODE" ]; then
    ok "versionCode $PREV_CODE -> $GOT_CODE (strictly greater)"
  else
    bad "versionCode ${GOT_CODE:-?} not greater than previous ${PREV_CODE:-?}: Android will not downgrade"
  fi
else
  warn "no previous sidecar at $(basename "$AGAINST").json, versionCode order unchecked"
fi

# 4 - sidecar agrees with the artifact and with version.properties.
if [ -f "$SIDECAR" ]; then
  S_SIZE="$(json_field "$SIDECAR" size)"; S_CODE="$(json_field "$SIDECAR" version_code)"
  S_NAME="$(json_field "$SIDECAR" version)"; S_AT="$(json_field "$SIDECAR" built_at)"
  A_SIZE="$(stat -c %s "$APK")"
  if [ "$S_SIZE" = "$A_SIZE" ]; then ok "sidecar size == apk ($A_SIZE)"; else bad "sidecar size ${S_SIZE:-?} != apk $A_SIZE"; fi
  if [ "$S_CODE" = "$WANT_CODE" ] && [ "$S_NAME" = "$WANT_NAME" ]; then
    ok "sidecar version $S_NAME / $S_CODE"
  else
    bad "sidecar ${S_NAME:-?} / ${S_CODE:-?} != version.properties ${WANT_NAME:-?} / ${WANT_CODE:-?}"
  fi
  S_AT_MS="$(iso_to_epoch_ms "$S_AT")"
  if [ -n "$S_AT_MS" ] && [ "$S_AT_MS" = "$WANT_EPOCH" ]; then
    ok "sidecar built_at == BUILD_EPOCH ($S_AT)"
  else
    bad "sidecar built_at ${S_AT:-?} (${S_AT_MS:-?}) != BUILD_EPOCH ${WANT_EPOCH:-?}: the app would offer an update against itself"
  fi
else
  bad "no sidecar at $SIDECAR (tools/write-apk-sidecar.sh)"
fi

if [ "$FAIL" = 0 ]; then echo "-- APK verified --"; else echo "-- APK VERIFICATION FAILED --"; fi
exit "$FAIL"
