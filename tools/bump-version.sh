#!/usr/bin/env bash
# Bump version.properties: VERSION_NAME, VERSION_CODE, BUILD_EPOCH.
#
#   tools/bump-version.sh            # patch: 0.1.0 -> 0.1.1
#   tools/bump-version.sh minor      #        0.1.1 -> 0.2.0
#   tools/bump-version.sh major      #        0.2.0 -> 1.0.0
#
# VERSION_CODE = max(old + 1, minutes since 2024-01-01 UTC). Not a plain +1: the file
# is git-tracked, so an uncommitted bump (or another checkout) would hand the next
# build a code already used. Android then refuses the install-over and the in-app
# updater (served code > installed code) never offers it. Wall-clock minutes always
# advance. Ceiling is 2.1e9, ~4000 years away.
# BUILD_EPOCH = now in ms. The sidecar's built_at derives from it (docs/protocol.md).
set -euo pipefail
. "$(dirname "$0")/lib.sh"

level="${1:-patch}"
case "$level" in patch|minor|major) ;; *) die "unknown level '$level' (patch|minor|major)" ;; esac

old_name="$(prop VERSION_NAME)"; old_name="${old_name:-0.1.0}"
old_code="$(prop VERSION_CODE)"; old_code="${old_code:-0}"
[[ "$old_name" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]] || die "VERSION_NAME '$old_name' is not X.Y.Z"
major="${BASH_REMATCH[1]}"; minor="${BASH_REMATCH[2]}"; patch="${BASH_REMATCH[3]}"
[[ "$old_code" =~ ^[0-9]+$ ]] || die "VERSION_CODE '$old_code' is not a number"

case "$level" in
  major) major=$((major + 1)); minor=0; patch=0 ;;
  minor) minor=$((minor + 1)); patch=0 ;;
  patch) patch=$((patch + 1)) ;;
esac

now_ms="$(now_ms)"
epoch_2024=1704067200            # 2024-01-01T00:00:00Z
wall_code=$(( (now_ms / 1000 - epoch_2024) / 60 ))
new_code=$(( old_code + 1 > wall_code ? old_code + 1 : wall_code ))
new_name="$major.$minor.$patch"

tmp="$VERSION_FILE.tmp.$$"       # same dir: atomic rename, nothing in /tmp
cat > "$tmp" <<EOF
# Written by tools/bump-version.sh. Read by Gradle and tools/*.sh. See docs/protocol.md "APK update".
VERSION_NAME=$new_name
VERSION_CODE=$new_code
BUILD_EPOCH=$now_ms
EOF
mv "$tmp" "$VERSION_FILE"
echo "version $old_name -> $new_name ($level), versionCode $old_code -> $new_code, BUILD_EPOCH $now_ms"
