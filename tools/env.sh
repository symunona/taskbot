#!/usr/bin/env bash
# Build env. Source it: `. tools/env.sh`
# Toolchains + caches on data SSD, not on `/`. Override any var before sourcing.
_c="${TALTOS_CACHE:-/mnt/data_ssd/cache}"
if [ -d "$_c" ]; then
  export JAVA_HOME="${JAVA_HOME:-$_c/jdk/current}"
  export GRADLE_USER_HOME="${GRADLE_USER_HOME:-$_c/gradle}"
  export CARGO_HOME="${CARGO_HOME:-$_c/cargo}"
  export RUSTUP_HOME="${RUSTUP_HOME:-$_c/rustup}"
  export TMPDIR="${TMPDIR:-/mnt/data_ssd/tmp}"
  export PATH="$JAVA_HOME/bin:$CARGO_HOME/bin:$PATH"
fi
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
export ANDROID_SDK_ROOT="$ANDROID_HOME"
export PATH="$ANDROID_HOME/platform-tools:$ANDROID_HOME/cmdline-tools/latest/bin:$PATH"
unset _c
