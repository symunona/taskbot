#!/bin/bash
set -e

# Get absolute path to the project root
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVER_DIR="$PROJECT_ROOT/server"
CONFIG_PATH="$SERVER_DIR/config.toml"

export HERMES_CONFIG="$CONFIG_PATH"

echo "Building tx server..."
cd "$SERVER_DIR"
cargo build --release

if [ ! -f "$CONFIG_PATH" ] && [ -f "$PROJECT_ROOT/config.toml" ]; then
    echo "Migrating existing root config to server/config.toml..."
    cp "$PROJECT_ROOT/config.toml" "$CONFIG_PATH"
fi

echo "========================================"
echo "Running Network Setup Subsystem..."
"$PROJECT_ROOT/network-setup"

if [ -f "$CONFIG_PATH" ]; then
    echo "Using config: $CONFIG_PATH"
fi

echo "Setting up systemd service..."
USER_SERVICE_DIR="$HOME/.config/systemd/user"
mkdir -p "$USER_SERVICE_DIR"
SERVICE_FILE="$USER_SERVICE_DIR/hermes-tx.service"

# Prefer a stable installed binary path for systemd and CLI usage.
echo "Installing tx to user bin path..."
USER_BIN_DIR="$HOME/.local/bin"
mkdir -p "$USER_BIN_DIR"
cp "$SERVER_DIR/target/release/tx" "$USER_BIN_DIR/tx"

cat << INI > "$SERVICE_FILE"
[Unit]
Description=Hermes TX Server
After=network.target

[Service]
Type=simple
Environment=HERMES_CONFIG=$CONFIG_PATH
ExecStart=$USER_BIN_DIR/tx serve
WorkingDirectory=$SERVER_DIR
Restart=on-failure

[Install]
WantedBy=default.target
INI

systemctl --user daemon-reload
systemctl --user enable hermes-tx.service
systemctl --user restart hermes-tx.service

echo "========================================"
echo "Service installed and started successfully!"
echo "========================================"
echo "Pairing Information:"
"$USER_BIN_DIR/tx" pair
echo "========================================"
