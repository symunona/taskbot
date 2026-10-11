#!/usr/bin/env bash
# Start an interactive Claude Code session in tmux with the phone channel loaded.
# Contract: docs/protocol.md, "Agent sessions".
#   spawn-session.sh --name tb-<project> --project <project> --dir <dir> \
#       --port <shim port> --server http://127.0.0.1:<port> --permission-mode <mode> \
#       [--model <model>]
# Exit 0: tmux session alive and its shim answers /health (already, or just started).
# Alive but shim dead: session killed and started fresh.
# Exit non-zero: reason + last captured pane on stderr, session removed.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STATE="${TALTOS_CHANNEL_STATE:-$HOME/.local/state/taltosbot/channel}"
# Below the server's SPAWN_TIMEOUT (90 s): this script must get to clean up and report.
TIMEOUT="${TALTOS_SPAWN_TIMEOUT:-75}"
CLAUDE_BIN="${TALTOS_CLAUDE_BIN:-claude}"
NODE_BIN="${TALTOS_NODE_BIN:-node}"

NAME="" PROJECT="" DIR="" PORT="" SERVER="" MODE="" MODEL=""
while [ $# -gt 0 ]; do
  case "$1" in
    --name) NAME="${2:-}"; shift 2 ;;
    --project) PROJECT="${2:-}"; shift 2 ;;
    --dir) DIR="${2:-}"; shift 2 ;;
    --port) PORT="${2:-}"; shift 2 ;;
    --server) SERVER="${2:-}"; shift 2 ;;
    --permission-mode) MODE="${2:-}"; shift 2 ;;
    --model) MODEL="${2:-}"; shift 2 ;;
    *) echo "spawn-session: unknown argument: $1" >&2; exit 2 ;;
  esac
done

die() { echo "spawn-session: $*" >&2; exit 2; }
[ -n "$NAME" ] && [ -n "$PROJECT" ] && [ -n "$DIR" ] && [ -n "$PORT" ] && [ -n "$SERVER" ] && [ -n "$MODE" ] \
  || die "need --name --project --dir --port --server --permission-mode"
[[ "$NAME" =~ ^[A-Za-z0-9_-]+$ ]] || die "bad --name (letters, digits, _ and - only): $NAME"
[[ "$PORT" =~ ^[0-9]+$ ]] || die "bad --port: $PORT"
[ -d "$DIR" ] || die "no such --dir: $DIR"
DIR="$(cd "$DIR" && pwd)"
for bin in tmux jq curl uuidgen flock "$CLAUDE_BIN" "$NODE_BIN"; do
  command -v "$bin" >/dev/null || die "missing dependency: $bin"
done
NODE_PATH_ABS="$(command -v "$NODE_BIN")"
SHIM="$HERE/phone-channel.mjs"
[ -f "$SHIM" ] || die "missing $SHIM"
[ -d "$HERE/node_modules/@modelcontextprotocol/sdk" ] || die "run 'npm install' in $HERE first"

mkdir -p "$STATE" && chmod 700 "$STATE"

# One spawner per name at a time: a second concurrent call waits, then sees the session alive.
exec 9>"$STATE/$NAME.lock"
flock -w "$((TIMEOUT + 30))" 9 || die "could not get lock for $NAME"

alive() { tmux has-session -t "=$NAME" 2>/dev/null; }

HEALTH_URL="http://127.0.0.1:$PORT/health"
health_ok() {
  local body
  body="$(curl -s -f -m 2 "$HEALTH_URL" 2>/dev/null)" || return 1
  [ "$(jq -r '.project // empty' <<<"$body" 2>/dev/null)" = "$PROJECT" ]
}

if alive; then
  for _ in 1 2 3 4 5 6; do
    health_ok && exit 0
    sleep 0.5
  done
  # Session without its shim (MCP server crashed): no prompt can reach it. Start over.
  [ -f "$STATE/$NAME.mcp.json" ] \
    || die "tmux session $NAME exists, its shim does not answer on port $PORT, and spawn-session.sh did not start it"
  echo "spawn-session: $NAME alive but shim dead on port $PORT; restarting the session" >&2
  tmux kill-session -t "=$NAME" 2>/dev/null
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    alive || break
    sleep 0.2
  done
  alive && die "could not stop the broken session $NAME"
fi

if curl -s -o /dev/null -m 2 "$HEALTH_URL"; then
  die "port $PORT already answers HTTP but tmux session $NAME does not exist (stale shim or port clash)"
fi

SESSION_ID="$(uuidgen)"
MCP_JSON="$STATE/$NAME.mcp.json"
jq -n --arg node "$NODE_PATH_ABS" --arg shim "$SHIM" \
      --arg project "$PROJECT" --arg port "$PORT" --arg server "$SERVER" \
  '{mcpServers: {phone: {command: $node, args: [$shim],
     env: {TALTOS_PROJECT: $project, TALTOS_SHIM_PORT: $port, TALTOS_SERVER_URL: $server}}}}' \
  > "$MCP_JSON" || die "could not write $MCP_JSON"
chmod 600 "$MCP_JSON"
jq -n --arg name "$NAME" --arg project "$PROJECT" --arg dir "$DIR" --arg port "$PORT" \
      --arg server "$SERVER" --arg mode "$MODE" --arg sid "$SESSION_ID" \
  '{name: $name, project: $project, dir: $dir, port: ($port|tonumber), server: $server,
    permission_mode: $mode, claude_session_id: $sid}' > "$STATE/$NAME.json"

# --name sets the session registry name (~/.claude/sessions/<pid>.json .name);
# --remote-control <name> alone leaves it as a derived "<dirname>-xx".
# --allowedTools: the reply tool must never wait on a permission prompt nobody can see.
CMD=(env -u CLAUDECODE -u CLAUDE_CODE_ENTRYPOINT -u CLAUDE_CODE_SSE_PORT
     "$CLAUDE_BIN"
     --name "$NAME"
     --remote-control "$NAME"
     --session-id "$SESSION_ID"
     --mcp-config "$MCP_JSON"
     --dangerously-load-development-channels server:phone
     --permission-mode "$MODE"
     --allowedTools mcp__phone__say)
[ -n "$MODEL" ] && CMD+=(--model "$MODEL")
CMD_STR="$(printf '%q ' "${CMD[@]}")"

# Placeholder first, then respawn into claude with remain-on-exit already set: an early
# crash leaves a dead pane whose text can be captured. Switched off once ready.
tmux new-session -d -s "$NAME" -x 160 -y 50 -c "$DIR" "sleep 60" || die "tmux new-session failed"
tmux set-option -w -t "=$NAME:" remain-on-exit on
tmux respawn-pane -k -t "=$NAME:" -c "$DIR" "$CMD_STR" || { tmux kill-session -t "=$NAME"; die "tmux respawn-pane failed"; }

PANE=""
capture() { PANE="$(tmux capture-pane -p -t "=$NAME:" 2>/dev/null)"; }

# Reason goes last as well: the server keeps only the tail of stderr.
fail() {
  echo "spawn-session: $1" >&2
  echo "----- pane ($NAME) -----" >&2
  grep -v '^[[:space:]]*$' <<<"$PANE" | tail -n 15 >&2
  echo "spawn-session: $1" >&2
  tmux kill-session -t "=$NAME" 2>/dev/null
  rm -f "$MCP_JSON" "$STATE/$NAME.json"
  exit 1
}

# Move the dialog cursor onto the line matching $1, press Enter only when the
# captured screen shows the cursor on that line. One step per call; caller re-captures.
choose() {
  local want="$1" target cursor
  target="$(grep -n -m1 -E "$want" <<<"$PANE" | cut -d: -f1)"
  [ -n "$target" ] || return 1
  # cursor: a "❯" line within 6 lines of the target (dialog option block)
  cursor="$(awk -v t="$target" '/^[[:space:]]*❯[[:space:]]*[^[:space:]]/ && NR >= t-6 && NR <= t+6 {print NR; exit}' <<<"$PANE")"
  [ -n "$cursor" ] || return 1
  if [ "$cursor" -eq "$target" ]; then
    tmux send-keys -t "=$NAME:" Enter
  elif [ "$cursor" -lt "$target" ]; then
    tmux send-keys -t "=$NAME:" Down
  else
    tmux send-keys -t "=$NAME:" Up
  fi
  return 0
}

deadline=$((SECONDS + TIMEOUT))
answered=""
while [ $SECONDS -lt $deadline ]; do
  alive || fail "tmux session vanished during startup"
  capture
  if [ "$(tmux display-message -p -t "=$NAME:" '#{pane_dead}' 2>/dev/null)" = "1" ]; then
    capture
    fail "claude exited during startup (status $(tmux display-message -p -t "=$NAME:" '#{pane_dead_status}' 2>/dev/null))"
  fi

  if grep -q "Loading development channels" <<<"$PANE"; then
    # "❯ 1. I am using this for local development / 2. Exit"
    choose "I am using this for local development" && answered="$answered dev-channels"
    sleep 0.6; continue
  fi
  if grep -q "Yes, I trust this folder" <<<"$PANE"; then
    # "❯ No, exit / Yes, I trust this folder" (default is No)
    choose "Yes, I trust this folder" && answered="$answered trust"
    sleep 0.6; continue
  fi
  if grep -q "New MCP server found" <<<"$PANE"; then
    choose "Use this MCP server" && answered="$answered mcp-consent"
    sleep 0.6; continue
  fi
  if grep -q "Bypass Permissions mode" <<<"$PANE" && grep -q "Yes, I accept" <<<"$PANE"; then
    choose "Yes, I accept" && answered="$answered bypass"
    sleep 0.6; continue
  fi
  if grep -q -i "blocked by org policy" <<<"$PANE"; then
    fail "channels blocked by org policy (channelsEnabled)"
  fi

  if health_ok; then
    if ! grep -q "Channels (experimental)" <<<"$PANE"; then
      echo "spawn-session: warning: shim is up but the 'Channels (experimental)' notice is not on screen; prompts may be dropped" >&2
    fi
    tmux set-option -w -t "=$NAME:" remain-on-exit off 2>/dev/null
    echo "{\"name\":\"$NAME\",\"claude_session_id\":\"$SESSION_ID\",\"port\":$PORT,\"answered\":\"${answered# }\"}"
    exit 0
  fi
  sleep 0.5
done

capture
fail "timeout after ${TIMEOUT}s waiting for $HEALTH_URL"
