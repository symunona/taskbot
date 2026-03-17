#!/bin/bash
set -e

# Get absolute path to the project root
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "Building Android APK..."
cd "$PROJECT_ROOT"
ENV=prod ./gradlew :androidApp:assembleDebug

APK_PATH="$PROJECT_ROOT/androidApp/build/outputs/apk/debug/androidApp-debug.apk"

if [ -f "$APK_PATH" ]; then
    echo "========================================"
    echo "APK built successfully!"
    echo "Location: $APK_PATH"
    echo "========================================"
else
    echo "APK build failed."
    exit 1
fi
