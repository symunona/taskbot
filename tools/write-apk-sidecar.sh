#!/usr/bin/env bash
# Write <apk>.json, the version manifest the landing serves to the in-app updater.
# Contract: docs/protocol.md "APK update".
#
#   tools/write-apk-sidecar.sh <apk> [sidecar.json]
#
# Everything but `size` comes from the local version.properties, so nothing about the
# version travels back from a build host.
# built_at derives from BUILD_EPOCH, NEVER from "now": at equal version_code the app
# updates when built_at > its own BUILD_EPOCH. BUILD_EPOCH is stamped at build start,
# this file is written at build end; a fresh timestamp is always later, so the app
# would report an update against its own build forever.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

apk="${1:-}"
[ -n "$apk" ] || die "usage: tools/write-apk-sidecar.sh <apk> [sidecar.json]"
[ -f "$apk" ] || die "no APK at $apk"
out="${2:-$apk.json}"

name="$(prop VERSION_NAME)"; code="$(prop VERSION_CODE)"; epoch="$(prop BUILD_EPOCH)"
[[ "$name" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version.properties: bad VERSION_NAME '$name'"
[[ "$code" =~ ^[0-9]+$ ]] || die "version.properties: bad VERSION_CODE '$code'"
[[ "$epoch" =~ ^[0-9]{13}$ ]] || die "version.properties: bad BUILD_EPOCH '$epoch' (ms). Run tools/bump-version.sh"

printf '{"version":"%s","version_code":%s,"size":%s,"built_at":"%s"}\n' \
  "$name" "$code" "$(stat -c %s "$apk")" "$(epoch_ms_to_iso "$epoch")" > "$out"
echo "sidecar -> $out ($name / code $code)"
