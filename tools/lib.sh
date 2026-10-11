#!/usr/bin/env bash
# Shared helpers for tools/*.sh. Source it: `. "$(dirname "$0")/lib.sh"`.
# Sets REPO, VERSION_FILE. Pulls in tools/env.sh (toolchains + caches on the data SSD).

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION_FILE="$REPO/version.properties"
# shellcheck source=tools/env.sh
. "$REPO/tools/env.sh"

die()  { printf 'FAIL %s\n' "$*" >&2; exit 1; }

# prop KEY [file] -> value from a .properties file ("" when absent).
prop() {
  local file="${2:-$VERSION_FILE}"
  [ -f "$file" ] || return 0
  sed -n "s/^[[:space:]]*$1[[:space:]]*=[[:space:]]*//p" "$file" | tail -n 1 | tr -d '\r'
}

# build_tools_dir -> newest $ANDROID_HOME/build-tools/* carrying apksigner + aapt2.
build_tools_dir() {
  local d
  while read -r d; do
    [ -n "$d" ] || continue
    if [ -x "$ANDROID_HOME/build-tools/$d/apksigner" ] && [ -x "$ANDROID_HOME/build-tools/$d/aapt2" ]; then
      printf '%s\n' "$ANDROID_HOME/build-tools/$d"
      return 0
    fi
  done < <(ls -1 "$ANDROID_HOME/build-tools" 2>/dev/null | sort -Vr)
  return 1
}

# epoch_ms_to_iso 1790000000123 -> 2026-09-21T12:53:20.123Z (UTC, ms kept so it round-trips).
epoch_ms_to_iso() {
  local ms="$1"
  printf '%s.%03dZ\n' "$(date -u -d "@$((ms / 1000))" +%Y-%m-%dT%H:%M:%S)" "$((ms % 1000))"
}

# now_ms -> current epoch in ms. Via %N: `+%s%3N` is GNU-only (uutils date prints all 9 digits).
now_ms() {
  local ns; ns="$(date +%s%N)"
  printf '%s\n' "$((ns / 1000000))"
}

# iso_to_epoch_ms 2026-09-21T12:53:20.123Z -> 1790000000123 ("" when unparseable).
iso_to_epoch_ms() {
  local ns; ns="$(date -u -d "$1" +%s%N 2>/dev/null)" || return 0
  [[ "$ns" =~ ^[0-9]+$ ]] && printf '%s\n' "$((ns / 1000000))"
  return 0
}

# json_field file key -> value of a top-level scalar in a flat JSON object.
json_field() {
  jq -r --arg k "$2" '.[$k] // empty' "$1" 2>/dev/null || true
}
