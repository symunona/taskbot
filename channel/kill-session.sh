#!/usr/bin/env bash
# Stop a session started by spawn-session.sh.
#   kill-session.sh --name tb-<project> [--force]
# Only touches tmux sessions this directory spawned (state file present), unless --force.
# Exit 0 when the session is gone afterwards, also when it never existed.
set -u

STATE="${TALTOS_CHANNEL_STATE:-$HOME/.local/state/taltosbot/channel}"
NAME="" FORCE=0
while [ $# -gt 0 ]; do
  case "$1" in
    --name) NAME="${2:-}"; shift 2 ;;
    --force) FORCE=1; shift ;;
    *) echo "kill-session: unknown argument: $1" >&2; exit 2 ;;
  esac
done
[[ "$NAME" =~ ^[A-Za-z0-9_-]+$ ]] || { echo "kill-session: need --name (letters, digits, _ and -)" >&2; exit 2; }

if tmux has-session -t "=$NAME" 2>/dev/null; then
  if [ ! -f "$STATE/$NAME.mcp.json" ] && [ "$FORCE" -ne 1 ]; then
    echo "kill-session: tmux session $NAME exists but was not started by spawn-session.sh (no $STATE/$NAME.mcp.json); use --force" >&2
    exit 1
  fi
  # SIGHUP to claude: it removes its ~/.claude/sessions entry, the shim exits when stdin closes.
  tmux kill-session -t "=$NAME" 2>/dev/null
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    tmux has-session -t "=$NAME" 2>/dev/null || break
    sleep 0.2
  done
  if tmux has-session -t "=$NAME" 2>/dev/null; then
    echo "kill-session: $NAME still alive" >&2
    exit 1
  fi
fi
rm -f "$STATE/$NAME.mcp.json" "$STATE/$NAME.json"
exit 0
