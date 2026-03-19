apk_path := "androidApp/build/outputs/apk/debug/androidApp-debug.apk"

[group('build')]
build: server-build android-build
    @echo "Built backend and Android APK."

[group('server')]
server-build:
    cargo build --manifest-path server/Cargo.toml

[group('server')]
server-lint:
    cargo clippy --manifest-path server/Cargo.toml --all-targets

alias apk := android-build

[group('android')]
android-build:
    ./build_apk.sh

[group('android')]
deploy: android-build
    adb install -r {{apk_path}}

[group('ops')]
[linux]
install:
    ./install.sh
