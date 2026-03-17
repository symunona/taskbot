#!/bin/bash
set -e

# Get absolute path to the project root
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVER_DIR="$PROJECT_ROOT/server"

echo "Building tx server..."
cd "$SERVER_DIR"
cargo build --release

echo "========================================"
echo "Running Network Setup Subsystem..."
"$PROJECT_ROOT/network-setup"

echo "Setting up systemd service..."
USER_SERVICE_DIR="$HOME/.config/systemd/user"
mkdir -p "$USER_SERVICE_DIR"
SERVICE_FILE="$USER_SERVICE_DIR/hermes-tx.service"

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
ExecStart=$SERVER_DIR/target/release/tx serve
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
$SERVER_DIR/target/release/tx pair
echo "========================================"
