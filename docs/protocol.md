# Taltosbot protocol

Contract between app (Kotlin), server (Rust), channel shim (Node). Change here first, then code.

## Parts

| part | where | job |
|---|---|---|
| app | phone / web | mic + speaker, Gemini Live, UI. No work done here. |
| server `taltosbot` | home machine, tailnet | pairing, auth, read-only vault tools, spawns agent sessions |
| channel shim | one per agent session, spawned by Claude Code | prompt in, `say` out |
| landing | `https://taskbot.tmpx.space` | static page, APK, APK sidecar |

Server listens on one port, HTTP + WebSocket. Default `55566`. Reach over tailnet, plain `http`/`ws`.

## Pairing (samizdat way)

1. Operator: `taltosbot pair` on server host.
2. CLI: `POST http://127.0.0.1:<port>/api/v1/admin/pair/new` → `{code, expires_at, server_urls}`.
3. CLI prints QR + same string as text.
4. App scans or pastes, `POST <url>/api/v1/pair {code, name}` → `{device_token, device_id, server_urls}`.
5. App stores `{token, deviceId, serverUrls}`. Token = bearer for everything after.

QR string = standard base64 of compact JSON:

```json
{"v":1,"code":"K7M2QX9P","urls":["http://100.81.125.107:55566"]}
```

- `urls`: full URLs with scheme. No scheme → app assumes `http://`. Scanned URL is primary.
- Code: 8 chars, alphabet `ABCDEFGHJKLMNPQRSTUVWXYZ23456789`, CSPRNG, TTL 5 min, single use.
- Claim = one conditional `UPDATE ... WHERE code=? AND used_at IS NULL AND expires_at>?`, check rows affected.
- Token: 32 random bytes hex. Server stores SHA-256 only.
- `/api/v1/pair`: rate limit, 5 failures per minute per remote IP → 429.
- Admin routes: loopback remote addr only.
- State in sqlite at `~/.local/state/taltosbot/taltosbot.db`. Never inside the vault.

## HTTP

Errors: `{"error":"<msg>"}`. CORS `*` (web target).

| method path | guard | request | 200 response |
|---|---|---|---|
| `GET /api/v1/health` | none | | `{app:"taltosbot", status:"ok", version}` |
| `POST /api/v1/admin/pair/new` | loopback | `{}` | `{code, expires_at, server_urls}` |
| `GET /api/v1/admin/devices` | loopback | | `{devices:[{id,name,created_at,last_seen_at}]}` |
| `DELETE /api/v1/admin/devices/{id}` | loopback | | 204 |
| `POST /api/v1/pair` | none | `{code, name}` | `{device_token, device_id, server_urls}`; 401 bad code |
| `GET /api/v1/me` | bearer | | `{device_id, name, server_version}` |
| `GET /api/v1/config` | bearer | | `{gemini_api_key, gemini_model, gemini_text_model}` |
| `GET /api/v1/ws` | first frame | upgrade | WebSocket |
| `POST /internal/agent/say` | loopback | `{project, text, tasks?}` | `{ok:true}` |
| `POST /internal/agent/event` | loopback | `{project, kind, detail, tasks?}` | `{ok:true}` |

## WebSocket

Text frames, JSON. Browsers cannot set headers, so auth is the first frame.

```json
C→S {"event":"auth","payload":{"token":"<device_token>"}}
S→C {"event":"auth.ok","payload":{"device_id":"..."}}
S→C {"event":"auth.error","payload":{"reason":"invalid_token"}}   then server closes
```

No auth frame within 10 s → close.

Envelope after auth: `{"event", "id", "ref_id"?, "ts", "payload"}`. Reply carries `ref_id` = request `id`.

| C→S event | payload | S→C reply | payload |
|---|---|---|---|
| `tools.list` | `{}` | `tools.list.result` | `{tools:[{name, description, parameters}]}` |
| `tool.call` | `{name, args, thread_id?}` | `tool.result` | `{result:{...}}` |
| `sessions.list` | `{}` | `sessions.list.result` | `{sessions:[Session]}` |
| `tasks.list` | `{}` | `tasks.list.result` | `{tasks:[Task]}`, newest first. Initial sync, again after every reconnect. |
| `threads.list` | `{}` | `threads.list.result` | `{threads:[Thread]}`, newest `last_accessed_at` first |
| `threads.create` | `{name?}` | `threads.create.result` | `{thread:Thread}` |
| `threads.get` | `{id}` | `threads.get.result` | `{thread:Thread, entries:[Entry], memory?:Memory, usage:Usage}`, `seq` order. Touches `last_accessed_at`. |
| `threads.rename` | `{id, name}` | `threads.rename.result` | `{thread:Thread}` |
| `threads.append` | `{id, entries:[Entry]}` | `threads.append.result` | `{count}`. Upsert by entry `id`. Touches `last_accessed_at`. |
| `threads.compact` | `{id}` | `threads.compact.result` | `{memory?:Memory, usage:Usage}`. Waits for the compact. Nothing to fold → current memory. Failure → `error`. |
| `threads.usage` | `{id, input, output}` | `threads.usage.result` | `{usage:Usage}`. Adds token counts (integers ≥ 0) to the thread. |
| any | | `error` | `{error:"..."}` |

- `parameters` = Gemini `functionDeclarations[].parameters` shape (`{type:"OBJECT", properties, required}`).
- Server owns tool declarations. App registers whatever `tools.list` returns. No mock fallback: disconnected → tool returns `{error:"server not connected"}`.
- Handlers run off the socket loop. App-side timeout 30 s.

Server push, no `ref_id`:

| event | payload | app does |
|---|---|---|
| `agent.say` | `{project, text}` | live voice: inject as text turn so Gemini speaks it. Else: chat bubble. |
| `agent.event` | `{project, kind, detail}` | log. Later: earcons. `kind` = `spawned`, `prompt`, `start`, `activity`, `waiting`, `resume`, `stop`, `error`. |
| `tasks.changed` | `{task:Task}` | upsert by `task.id`, redraw. UI only: never injected into the voice model. |
| `threads.changed` | `{thread:Thread}` | update thread list + title. Sent on create, rename. |
| `threads.memory` | `{thread_id, memory:Memory, usage:Usage}` | swap the memory of that thread. Sent after every compact that stored one. |

`Session` = `{project, tmux, status, claude_session_id, url}`. `status` = `idle` / `busy` / `waiting` / `dead`. `waiting` = stuck on a permission prompt, answer it in the Claude app. `url` = `https://claude.ai/code/<bridgeSessionId>` or null.

## Tasks

One task = one `delegate` call. Own model, own store. Not part of `Session`, not a thread entry.

`Task` = `{id, project, thread_id?, prompt, state, queue_pos?, created_at, started_at?, updated_at, last_activity?, answer?, error?}`.

| field | meaning |
|---|---|
| `id` | 16 hex, server-made |
| `project` | project name |
| `thread_id` | `tool.call.thread_id` of the `delegate` call. Absent = none given. |
| `prompt` | text handed to the agent |
| `state` | see below |
| `queue_pos` | unfinished tasks of the same project created earlier. `0` = next. Only in `queued`, `spawning`, `sent`. |
| `created_at` | epoch ms, `delegate` call |
| `started_at` | epoch ms, first `working`. Absent = agent turn not started. |
| `updated_at` | epoch ms, last change of any field but `queue_pos`. Finished task: finish time. |
| `last_activity` | `working` only: what the agent does now, e.g. `Edit notes/tea.md`. Max 120 chars. |
| `answer` | last text the agent said for this task |
| `error` | `failed` only: why |

| `state` | meaning | set by |
|---|---|---|
| `queued` | waits for an earlier delivery to the same project | `delegate` |
| `spawning` | no live session, `spawn-session.sh` runs | server, before spawn |
| `sent` | shim took the prompt. Agent turn not started (session busy with an earlier turn). | server, `POST /prompt` 2xx |
| `working` | agent turn runs | shim `start`, `resume`, `activity` |
| `waiting` | agent stuck on a permission prompt | shim `waiting` |
| `done` | turn over. `answer` set unless the agent said nothing. | shim `stop` |
| `failed` | never delivered, or session lost | server |

- `done`, `failed` = final. No change after.
- States only move forward: `queued` → `spawning` → `sent` → `working` ⇄ `waiting` → `done`. `failed` from any unfinished state. Steps may be skipped (live session: no `spawning`).
- Every change of a `Task` field → one `tasks.changed {task}`. Task finishes → `tasks.changed` also for each later task of the project whose `queue_pos` moved.
- `last_activity` change alone → `tasks.changed`, at most one per task per 2 s (shim throttles).
- Push only. Model hears: final answer and failures (`agent.say`), state when it calls the `tasks` tool. No status text turns: they interrupt live speech.
- `tasks.list`: every unfinished task + finished ones of the last 24 h, max 100.
- Stored in the server sqlite (table `tasks`). Written on state change only, not on `last_activity`. Finished tasks older than 7 days: dropped at start.
- Server restart: `queued`, `spawning` → `failed` (`server restarted before the prompt was delivered`). `sent`, `working`, `waiting` kept: the shim outlives the server and still reports.
- Lost session: server checks the shim `/health` of every project with a `sent` / `working` / `waiting` task, every 10 s. Two misses in a row → `failed` (`agent session is gone`) + `agent.say`.

Shim → server, how a task moves (`tasks` = ids of the tasks in the turn):

| shim call | when | server does |
|---|---|---|
| `event start` | prompt emitted into the session | `working`, `started_at` |
| `event activity`, `detail` = text | agent ran a tool, seen in the transcript | `last_activity`, `working` |
| `event waiting`, `detail` = what for | registry status `waiting` | `waiting` |
| `event resume` | registry status `busy` after `waiting` | `working` |
| `say` | agent called `say`, or turn ended without it | `answer` |
| `event stop` | registry status left `busy` | `done` |

- Prompts held by the shim go in as one message = one turn: all their tasks move together, same `answer`.
- `tasks` absent (shim older than this contract): `event` applies to every `sent` / `working` / `waiting` task of the project; `say` sets no `answer`.
- Unknown or finished ids: ignored.

## Threads

One thread = one conversation, voice and text mixed. Stored in the server sqlite, shared by all paired devices.

`Thread` = `{id, name, created_at, last_accessed_at}`. Times = epoch ms. `id` = 16 hex. `name`: control chars dropped, trimmed, cut to 80 chars; empty after that → `error`. `threads.create` without `name` → `New thread`.

`Entry` = `{id, ts, kind, text, voice?, project?, error?, tool?}`.

| field | meaning |
|---|---|
| `id` | client-made, unique in the thread. Same `id` again = replace (partial transcript, tool result arriving). |
| `ts` | epoch ms, client clock |
| `kind` | `user`, `model`, `agent`, `tool`, `marker`, `system` |
| `text` | message text. `marker`: `voice_started` / `voice_ended`. `tool`: empty. |
| `voice` | true = voice transcript, not typed |
| `project` | `agent` only |
| `error` | true = show as error |
| `tool` | `tool` only: `{name, args, result?, called_at, result_at?}`. No `result` = still running. |

- Server keeps entries opaque past `id`, `ts`, `kind`. Order = first insert (`seq`); replace keeps the slot. Exception: the compact reads `text`, `project`, `tool`. It never rewrites an entry.
- Unknown thread `id` → `error`.
- `threads.append`: entry without string `id`, integer `ts`, or a listed `kind` → `error`, nothing stored. Same `id` twice in one call: last wins.
- Limits: 200 entries per `threads.append`, frame cap 1 MiB.
- `tool.result` over 60k chars: app stores `{truncated:true, chars, preview}`. Model still gets the full result.
- No delete yet.

App rules:

- Text mode: `POST https://generativelanguage.googleapis.com/v1beta/models/<gemini_text_model>:generateContent`, key in `x-goog-api-key`. Same system prompt, same tools as voice. History = thread memory (in the system instruction) + entries after `memory.upto_id` as turns.
- Voice start on a thread with entries: history goes into the Live `systemInstruction`: memory block, then entries after `memory.upto_id` as a transcript block. No `clientContent` seeding (differs per model).
- No memory, or `upto_id` not among the loaded entries: plain tail window of the entries.
- Token use: after each text turn and at voice stop, send `threads.usage` with the `usageMetadata` sums (`input` = prompt tokens, `output` = the rest).
- Voice start / stop: append `marker` entry.
- `tool.call` from the app always carries the current `thread_id`.

### Thread memory

Where the thread stands now. Not a history. Derived from the entries, made by the server, shared by all devices.

`Memory` = `{text, upto_id, at}`. `upto_id` = `id` of the last entry folded in. `at` = epoch ms of the compact. Absent = never compacted.

`Usage` = `{input, output}`. Token counts of the thread, all time: text turns + voice sessions (app reports), compacts (server adds).

`text`: plain text, terse keywords, max 1500 chars, same language as the conversation. Two sections:

| section | holds |
|---|---|
| `## State` | bullets: things agreed, decisions, goals, open questions, delegated tasks + outcome, exact vault paths. Latest wins: a changed decision replaces its line. |
| `## Recent` | last 5 to 10 turns, one line each (`asked X -> Y`). Rolls. |

- Info questions: `Recent` only, never `State`. Chit-chat: nowhere. Tool result bodies: nowhere.

Compact runs when any of:

| trigger | detail |
|---|---|
| turns | more than 15 `user` entries after `upto_id` |
| voice stop | `marker` entry `voice_ended` in a `threads.append`, at least one `user` entry after `upto_id` |
| request | `threads.compact` |

- Fold = one `generateContent` call on the server: old memory + entries after `upto_id` → new memory.
- Counted entries: `user`, `model`, `agent` with text, and `tool`. The last 4 counted entries stay unfolded. A `tool` entry without `result` stops the fold before it.
- One compact per thread at a time. `threads.append` never waits for it.
- Failure (no key, HTTP error, timeout, empty answer): memory and `upto_id` stay. Auto triggers pause 60 s for that thread. `threads.compact` always tries.
- Fold input over 24k chars: oldest part first, then again for the rest.

## Tools (served via `tools.list`)

Gemini gets read-only vault tools. All writes go through an agent session, so `CLAUDE.md` in the project rules.

| tool | args | result |
|---|---|---|
| `read_file` | `{path, start_line?, end_line?}` | Small file: `{content}`. Big file, no range: `{outline:[{line, text}], lines}`. Range, or big file without outline: `{content, start_line, end_line, lines, next_line?}`. See "read_file". |
| `list_directory` | `{path}` | `{items:[name], truncated?, total?}`. Sorted. Dir names end `/`. `""` = vault root. Max 200 items; more → `truncated:true` + `total`. |
| `search_regexp` | `{pattern}` | `{results:[{path, match}], truncated?}`. One result per file, max 50. `match` = first matching line, max 300 chars. `truncated:true` = more files matched. |
| `projects` | `{}` | `{projects:[{name, dir}]}` |
| `sessions` | `{}` | `{sessions:[Session]}` |
| `tasks` | `{project?}` | `{tasks:[Task]}`, newest first. Every unfinished task + the 5 last finished. `project` given: that project only. |
| `rename_thread` | `{name}` | `{thread:Thread}`. Renames the thread in `tool.call.thread_id`; none → `{error}`. Pushes `threads.changed`. |
| `delegate` | `{project, prompt}` | `{status:"sent", project, task_id, state, queue_pos}`. Returns at once: `state` = `queued`. Progress: `tasks.changed`. Answer arrives later as `agent.say`. |

- Any tool failure → `{error:"..."}`.
- Vault paths: relative to `vault_path`, no leading `/`. Canonicalise, reject anything outside `vault_path`.
- Vault tools skip dot-entries. Search skips files > 1 MB and non-UTF-8 files.
- `search_regexp`: Rust regex syntax.
- Several files: several `read_file` calls. No multi-path tool.

### read_file

| case | result |
|---|---|
| no range, file ≤ 300 lines and ≤ 20k chars | `{content}`, whole file |
| no range, bigger, outline found | `{outline:[{line, text}], lines}`. No content. Model then reads a range. |
| no range, bigger, no outline | first page, as a range read from line 1 |
| `start_line` and/or `end_line` | `{content, start_line, end_line, lines, next_line?}` |

- Lines 1-based, `end_line` inclusive. `lines` = total lines in file.
- Page cap: 300 lines and 20k chars. Cut → `next_line` = where to continue. Absent = reached `end_line` or end of file.
- `start_line` past end of file, `start_line` < 1, `end_line` < `start_line` → `{error}`.
- File > 1 MB or non-UTF-8 → `{error}`.
- Outline: max 200 entries (`outline_truncated:true` if cut), `text` = the line, trimmed, max 120 chars.

| file type | outline entries |
|---|---|
| `md`, `markdown` | `#` headings, fenced code blocks skipped |
| `rs` | `fn`, `struct`, `enum`, `trait`, `impl`, `mod`, `type`, `macro_rules!` |
| `py` | `def`, `class` |
| `js`, `mjs`, `cjs`, `jsx`, `ts`, `tsx` | `function`, `class`, `interface`, `type`, `enum`, top-level `const x = (…) =>` |
| `kt`, `kts` | `fun`, `class`, `interface`, `object`, `typealias` |
| `java` | `class`, `interface`, `enum`, `record`, methods with a visibility keyword |
| `go` | `func`, `type` |
| `sh`, `bash` | function definitions |
| `toml`, `ini` | `[section]` |
| other | none |

- Outline = line regex per type, not a parser. Misses are fine: the range read still works.
- `delegate`: unknown project or empty prompt → `{error}`. No session → spawn, wait until ready, then send prompt. Failure after `sent` → spoken via `agent.say`.
- `delegate`: one job per project at a time, prompts stay in order.
- `delegate`: makes one `Task`, see "Tasks". Its `thread_id` = `tool.call.thread_id`.

## Agent sessions

Server config:

```toml
listen_port = 55566
vault_path = "/path/to/vault"
gemini_model = "..."                      # Live (voice)
gemini_text_model = "gemini-flash-latest" # text mode, default shown
gemini_summary_model = "..."              # thread memory compacts. Default: gemini_text_model
gemini_base_url = "https://generativelanguage.googleapis.com"  # default shown. Server-side calls only
channel_dir = "/path/to/repo/channel"     # holds spawn-session.sh
channel_port_base = 55600

[keys]
gemini_api_key = "..."

[projects.vault]
dir = "/path/to/vault"
permission_mode = "bypassPermissions"     # default. Others: acceptEdits, default, plan
model = "haiku"                           # optional. Alias or full id, passed to `claude --model`
```

Server → shim scripts (server shells out, never types into tmux itself):

```sh
channel/spawn-session.sh --name tb-<project> --project <project> --dir <dir> \
    --port <shim port> --server http://127.0.0.1:<listen_port> --permission-mode <mode> \
    [--model <model>]
```

- `model` omitted → no `--model`, Claude Code default. Applies at spawn only: a live session keeps its model.
- `permission_mode` omitted → `bypassPermissions` (= `claude --dangerously-skip-permissions`): no permission prompts, so no session stuck in `waiting`. The agent can run any command as the server user.
- Idempotent. tmux session `tb-<project>` alive and shim answers `/health` → exit 0, nothing else.
- Session alive, shim dead (3 s of `/health` misses) → session killed, started fresh. Context of the old session is lost. Session not started by the script → exit 2, left alone.
- Own timeout 75 s (`TALTOS_SPAWN_TIMEOUT`), below the server's 90 s: the script must get to kill the half-started session and report. Failure: stderr ends with the reason line; server speaks that line only.
- Exit 0 only when the shim answers `GET http://127.0.0.1:<shim port>/health`.
- Shim port = `channel_port_base` + stable index per project. Server stores the map.

Server → shim: `POST http://127.0.0.1:<shim port>/prompt`, body = prompt text, header `x-task-id: <Task id>` → `200`, body `ok` or `queued`.

- Shim holds prompts while the session is `busy` or `waiting`, then sends them as one message. Mid-turn injection gets refused by the model. Registry status `shell` (no turn, background command running) counts as idle, in the shim and in `Session.status`.
- Turn ends without a `say` call (small models skip the tool): shim speaks the last terminal text of the turn, cut to 800 chars. No text → `The agent for project <p> finished without an answer.`
- Session `waiting`: shim says so once per turn (`... is stuck waiting on: permission prompt ...`).
- Session exits (tmux killed, claude quit) with a turn or held prompts open: shim sends `event error` (`... ended before it answered.`) before it dies. Not spoken by the shim: the lost-session check in "Tasks" fails the tasks and speaks once, also when the shim itself was killed.
- `agent.event` kind `prompt`: `detail` = prompt, first 500 chars.
- `GET /health` → `503` until the MCP handshake is done, then `200 {ok, project, pid, queued, claude_status}`.
- Registry entry appears ~1 s after spawn. `status` and `bridgeSessionId` arrive later. Treat missing as `busy` / null.
- Mode: Claude Code channels (`--dangerously-load-development-channels server:phone`). Verified on 2.1.296.

Shim → server: `POST /internal/agent/say {project, text, tasks}` when the agent calls its `say` tool, or ended a turn without it (shim then sends the terminal text).

Shim → server: `POST /internal/agent/event {project, kind, detail, tasks}`. Kinds and effect: "Tasks". Server pushes each one on as `agent.event`.

- `tasks` = `x-task-id` values of the prompts in the running turn. Notices the shim makes up itself (`waiting for ...`) carry no `tasks`.
- `activity`: shim reads new transcript lines every 2 s while the turn runs; last `tool_use` → `<tool> <file_path | command | pattern | description>`, cut to 120 chars. Sent only when changed.

Session status and `url` come from `~/.claude/sessions/*.json` (match `name` == `tb-<project>`; fields `status`, `sessionId`, `bridgeSessionId`). Internal Claude Code file, may change.

## APK update

- Landing hosts `taltosbot.apk` and `taltosbot.apk.json` at `https://taskbot.tmpx.space/`.
- Sidecar: `{"version":"0.1.0","version_code":1447075,"size":123,"built_at":"<ISO of build epoch>"}`.
- Version source: `version.properties` at repo root: `VERSION_NAME`, `VERSION_CODE`, `BUILD_EPOCH` (ms).
- App: fetch sidecar, update if `version_code` > installed, or equal and `built_at` > own `BUILD_EPOCH`.
- Install = open APK URL in browser. Same signing key every build (`secrets/debug.keystore`, gitignored).
