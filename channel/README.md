# channel

Push prompts into a running interactive Claude Code session. Get spoken replies back.
Contract: `docs/protocol.md`, "Agent sessions".

## What

- `phone-channel.mjs` — MCP server `phone`, stdio, spawned by Claude Code. Declares `claude/channel`.
  - `GET /health` → 200 `{ok, project, pid, queued, claude_status}`. 503 until MCP handshake done.
  - `POST /prompt`, body = text, header `x-task-id` = server task id (optional) → 200 `ok` (emitted now) or 200 `queued` (session not idle). 400 empty, 503 not ready.
  - tool `say(text)` → `POST <server>/internal/agent/say {project, text, tasks}` → `sent`, or `not sent: ...` (never throws).
  - turn progress → `POST <server>/internal/agent/event {project, kind, detail, tasks}`. `kind` = `start`, `activity`, `waiting`, `resume`, `stop`. `tasks` = `x-task-id` of the prompts in the turn. Drives the server task state: `docs/protocol.md` "Tasks".
  - `activity`: last `tool_use` in the session transcript, read every 2 s while busy, sent when changed.
  - env: `TALTOS_PROJECT`, `TALTOS_SHIM_PORT`, `TALTOS_SERVER_URL`.
- `spawn-session.sh` — detached tmux session running `claude`, answers startup dialogs, waits for `/health`.
- `kill-session.sh` — kills the tmux session, removes state files.
- State: `~/.local/state/taltosbot/channel/<name>.mcp.json` (MCP config + env), `<name>.json` (spawn args, session id), `<name>.lock`.

Mode: real channels (`--dangerously-load-development-channels`). Works on `claude` 2.1.296, Max login. No tmux paste fallback needed.

## Setup

```sh
cd channel && npm install        # node >= 20, tmux, jq, curl, uuidgen, flock
```

## Run by hand

```sh
channel/spawn-session.sh --name tb-vault --project vault --dir /path/to/vault \
    --port 55600 --server http://127.0.0.1:55566 --permission-mode acceptEdits
curl -d 'create hello.txt containing hi, then tell me' http://127.0.0.1:55600/prompt
curl http://127.0.0.1:55600/health
tmux attach -t tb-vault          # watch; detach with C-b d
channel/kill-session.sh --name tb-vault
```

Command the script runs inside tmux:

```sh
claude --name <name> --remote-control <name> --session-id <uuid> \
    --mcp-config ~/.local/state/taltosbot/channel/<name>.mcp.json \
    --dangerously-load-development-channels server:phone \
    --permission-mode <mode> --allowedTools mcp__phone__say
```

- `--name`: needed. `--remote-control <name>` alone leaves `~/.claude/sessions/<pid>.json` `.name` as derived `<dirname>-xx`.
- `--allowedTools mcp__phone__say`: reply tool never waits on a permission prompt.
- Success: exit 0, one JSON line on stdout `{name, claude_session_id, port, answered}`. Already alive: exit 0, no output.
- Knobs: `TALTOS_SPAWN_TIMEOUT` (75 s, keep below the server's 90 s), `TALTOS_CLAUDE_BIN`, `TALTOS_NODE_BIN`, `TALTOS_CHANNEL_STATE`.

## Startup dialogs

Script polls `tmux capture-pane`, matches text, moves the `❯` cursor with Up/Down, presses Enter only when the captured screen shows the cursor on the wanted line.

| dialog, matched on | options seen | answer |
|---|---|---|
| Workspace trust: `Yes, I trust this folder` (first run per dir) | `❯ No, exit` / `Yes, I trust this folder` — default is No | Down, Enter |
| `WARNING: Loading development channels` (every start) | `❯ 1. I am using this for local development` / `2. Exit` | Enter |
| `New MCP server found` | `Use this MCP server` | handled, never seen (`--mcp-config` does not trigger it) |
| `Bypass Permissions mode` | `Yes, I accept` | handled, never seen (`skipDangerousModePermissionPrompt` set on this machine) |

Ready screen shows:

```
▎ Channels (experimental) messages from server:phone inject directly in this session · ...
▎ server:phone · no MCP server configured with that name
```

Second line is a false alarm with `--mcp-config`; events arrive anyway.

## Busy sessions

- Claude Code injects a mid-turn channel event into the running turn as a "queued command". The model then says it cannot confirm the message is from the user and skips it. Seen twice, also with instructions saying otherwise.
- So the shim holds prompts while the session is not `idle`, emits when idle. Several held prompts go in as one message, one turn.
- Idle/busy source: `~/.claude/sessions/<ppid>.json` `.status` (shim's parent is `claude`). Unreadable → emit at once.
- Queue is in memory. Session dies → queued prompts lost.

## Failure modes

- Unknown dialog / claude never ready → timeout, exit 1, pane on stderr, tmux session killed.
- claude exits at startup → exit 1, exit status on stderr.
- Port already answering without the tmux session → exit 2, nothing started.
- Port taken when the shim starts → shim exits 1, `/health` never matches the project, spawn times out.
- `blocked by org policy` on screen → exit 1.
- Server down when `say` is called → tool returns `not sent: server unreachable ...`, Claude sees it, user hears nothing.
- Permission prompt in the session (e.g. Bash in `acceptEdits`; not with the default `bypassPermissions`) → session sits in `status: "waiting"`, `waitingFor: "permission prompt"`. Nobody answers it. New prompts queue behind it. No relay built.
- Session alive but shim dead (crashed MCP server) → spawn still exits 0 (contract: tmux alive → no-op). `/health` fails. Fix: kill, spawn.
- `~/.claude/sessions/*.json` is Claude Code internal. Format change breaks queueing (falls back to immediate emit) and server status.
- Channels are a research preview. Flag and protocol may change.
