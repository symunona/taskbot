#!/bin/bash
set -e

# Get absolute path to the project root
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVER_DIR="$PROJECT_ROOT/server"

# Ask user where to store config
DEFAULT_CONFIG_PATH="$HOME/.config/taskbot/config.toml"
echo "Where should the config file be stored? [$DEFAULT_CONFIG_PATH]:"
read USER_CONFIG_PATH
CONFIG_PATH="${USER_CONFIG_PATH:-$DEFAULT_CONFIG_PATH}"

# Ensure config directory exists
mkdir -p "$(dirname "$CONFIG_PATH")"

echo "Config file will be stored at: $CONFIG_PATH"
export HERMES_CONFIG="$CONFIG_PATH"

echo "Building tx server..."
cd "$SERVER_DIR"
cargo build --release

if [ ! -f "$CONFIG_PATH" ]; then
    if [ -f "$SERVER_DIR/config.toml" ]; then
        echo "Migrating existing server config to $CONFIG_PATH..."
        cp "$SERVER_DIR/config.toml" "$CONFIG_PATH"
    elif [ -f "$PROJECT_ROOT/config.toml" ]; then
        echo "Migrating existing root config to $CONFIG_PATH..."
        cp "$PROJECT_ROOT/config.toml" "$CONFIG_PATH"
    fi
fi

# Ensure Gemini API key is set
if [ -f "$CONFIG_PATH" ]; then
    if ! grep -q "gemini_api_key" "$CONFIG_PATH"; then
        echo "========================================"
        echo "No Gemini API key found in your configuration."
        echo "Please enter your Gemini API key (or press Enter to skip):"
        read -s GEMINI_API_KEY
        if [ -n "$GEMINI_API_KEY" ]; then
            if ! grep -q "^\[keys\]" "$CONFIG_PATH"; then
                echo "" >> "$CONFIG_PATH"
                echo "[keys]" >> "$CONFIG_PATH"
            fi
            echo "gemini_api_key = \"$GEMINI_API_KEY\"" >> "$CONFIG_PATH"
            echo "Gemini API key saved to $CONFIG_PATH."
        else
            echo "Skipping Gemini API key setup."
        fi
    fi
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

# Check if service is currently running and stop it before replacing the binary
if systemctl --user is-active --quiet hermes-tx.service; then
    echo "Stopping existing hermes-tx service..."
    systemctl --user stop hermes-tx.service
fi

# In case the binary is running outside of systemd, kill it
if pgrep -x "tx" > /dev/null; then
    echo "Killing running tx processes..."
    pkill -x "tx" || true
    sleep 1
fi

# Prefer a stable installed binary path for systemd and CLI usage.
echo "Installing tx to user bin path..."
USER_BIN_DIR="$HOME/.local/bin"
mkdir -p "$USER_BIN_DIR"

# Use rm before cp to avoid "Text file busy" error if it was still locked
if [ -f "$USER_BIN_DIR/tx" ]; then
    rm -f "$USER_BIN_DIR/tx"
fi
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
