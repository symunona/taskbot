# delegate: end-to-end test

Run 2026-10-11. Real Claude Code 2.1.296 sessions, model `haiku` (Haiku 5.5), tmux + channel shim.
Contract: `docs/protocol.md` "Tools", "Tasks", "Agent sessions".

## Rig

| part | value |
|---|---|
| dir | `/mnt/data_ssd/tmp/tb-dtest` (throwaway, safe to delete) |
| server | debug binary, `127.0.0.1:55577`, own config + `TALTOSBOT_STATE_DIR` + `TALTOS_CHANNEL_STATE` |
| shim ports | `55710`+ |
| projects | `dta`, `dtb` (bypassPermissions), `dtw` (`permission_mode = "default"`), `dtnodir` (missing dir), all `model = "haiku"` |
| fake phone | `phone.mjs`: pairs, `listen` logs every push to `logs/phone.jsonl`; `delegate <project> <prompt>` = one `tool.call` |
| broken spawns | `config-x.toml` + `chan-x/spawn-session.sh` wrapper: `dtfail` (claude exits 3), `dtslow` (claude hangs), `dtscript` (script sleeps 100 s) |
| run | `./start-server.sh [config]`, `node phone.mjs listen &`, `./run.sh <wait s> <project> <prompt>`, `./log.sh <t0 ms>` |

Production server (`:55566`) and real `tb-*` sessions: not touched. Test sessions, server, listener: killed at the end.

## Added for the test

- Per-project `model` (`config.rs` `Project.model`, `spawn-session.sh --model`, `config.example.toml`, protocol "Agent sessions"). Omitted → Claude Code default.

## Latency (haiku, trivial prompt)

| measure | value |
|---|---|
| `delegate` tool reply | 0–15 ms |
| cold: delegate → `spawned` | 1.8–2.7 s (first run in a new dir, with trust dialog: 2.6 s) |
| cold: delegate → first `agent.say` | 3.1–4.5 s |
| warm: delegate → `prompt` event | 0–0.1 s |
| warm: delegate → `agent.say` | 1.1–3.4 s |
| heal (shim dead, session restarted) → `agent.say` | 6.4–7.1 s |
| long task (174 s shell command) | one `agent.say` at +173 s |
| turn end → `stop` event | ~1 s after the `say` call (300 ms poll + final text) |

## Cases

`fixed` = failed first, passes after a fix from this run. Evidence = phone log, seconds after the delegate call.

| # | case | result | evidence |
|---|---|---|---|
| 1 | cold spawn (dead → spawn → prompt) | fixed | Before: `spawned` +2.6 s, `prompt` +2.6 s, then nothing. Haiku wrote `51` in the terminal, never called `say`. After: `agent.say "51"` +3.6 s, `stop` |
| 2 | warm prompt | fixed | same cause. After: `agent.say "Budapest"` +1.2 s |
| 3 | two delegates fast, one live project | pass | `prompt` ×2 in order, answers `Blue` +1.2 s, `8` +2.5 s |
| 4 | three delegates at once, dead project | pass, see D1 | one `spawned`, one tmux session, 3 `prompt`. Second and third merged into one turn → 2 answers for 3 delegates |
| 5 | two projects in parallel | pass | `dta` and `dtb` spawn and answer independently, +3.9 s both done |
| 6 | delegate while busy | pass | second prompt held (`/health` `queued:1`), answered after the first turn |
| 7 | session idle with a background command (registry status `shell`) | fixed | Before: prompt held 26 s until the command ended (dev server = forever); `sessions` said `busy`. After: delivered at once, `Cold.` +1.4 s; `sessions` says `idle` |
| 8 | agent asks a question back | pass, see D2 | `agent.say "... Which one should I delete?"` +6.5 s, session `idle`. Second delegate `The beta one.` → file deleted, `agent.say` +2.5 s |
| 9 | `waiting` (`permission_mode = "default"`, Write tool) | fixed | Before: silent forever. After: `event waiting` + `agent.say "... stuck waiting on: permission prompt ..."` +4.5 s, once. Second delegate held. Prompt answered in tmux → both answered, `resume` event |
| 10 | long task (174 s) | pass, see D3 | `agent.say "The token is walrus."` +173 s. One answer |
| 11 | agent killed mid-task (`tmux kill-session`) | fixed | Before: silent, `sessions` → `dead`. After: shim `event error` at once, server lost-session check speaks once at +13–25 s, tasks `failed`. Held prompt covered too |
| 12 | shim dead (`kill -9`), tmux alive | fixed | Before: false `spawned` event, `prompt not delivered ... :55711/prompt`, same for every later delegate. After: script kills + restarts the session, answer +6.4 s. Mid-turn kill: spoken by the lost-session check at +26 s |
| 13 | spawn fails fast (claude exits 3) | pass, polished | error event + spoken error +0.4 s. Before: spoken text held the raw pane dump (blank lines, `----- pane (tb-dt`). After: reason line only |
| 14 | spawn hangs (claude never ready) | fixed | Before: server timeout 90 s = script timeout 90 s; server won, SIGKILLed the script, tmux `tb-dtslow` leaked, no pane text. After: script timeout 75 s → `timeout after 75s waiting for .../health`, session and state files removed |
| 15 | script itself slower than 90 s | pass | `spawn-session.sh did not finish within 90 s` spoken at +90.6 s |
| 16 | second delegate behind a slow spawn | pass, see D4 | waits on the project lock, then spawns again: second error at +180 s |
| 17 | unknown project | pass | `{error: unknown project "nope"; known projects: [...]}` at once |
| 18 | empty / whitespace prompt, missing or non-string args | pass | `{error: "prompt is empty"}`, `missing string argument "prompt"` |
| 19 | project dir missing | pass | `no such --dir` error event + spoken, +0.1 s |
| 20 | unicode + multiline + quotes, `$`, backticks, tags | pass | repeated back exactly: `árvíztűrő tükörfúrógép 🦊 日本語` |
| 21 | 200 KB prompt | pass | `agent.say "11"` +1.4 s |
| 22 | 900 KB prompt | fixed | answered +5.9 s. Before: `prompt` event echoed all 900 KB to every phone. After: first 500 chars |
| 23 | 2 MB prompt (over the 1 MiB frame cap) | fail, see D5 | no `tool.result`, no error, call times out on the app side |
| 24 | server down while the agent answers | fail, see D6 | `say` → `not sent: server unreachable`. Agent knows, user never hears. Asked later: agent repeats it |
| 25 | server restart with live sessions | pass | port map kept, sessions `idle`, warm delegate answered +1.8 s, no respawn |
| 26 | answer with no phone connected | fail, see D6 | agent got `sent`, phone reconnected 8 s later: nothing. Now kept in `Task.answer`, still never spoken |

"Exactly one answer or one spoken error": holds for every case after the fixes, except D1 (merged turns), D2 (interim `say`), 23, 24, 26.

## Fixes made

| file | change |
|---|---|
| `channel/phone-channel.mjs` | turn ended without `say` → speak the last terminal text of the turn (transcript read from the offset at emit time, max 800 chars); none → `finished without an answer` |
| `channel/phone-channel.mjs` | `stop` event at turn end (was in the protocol, never sent) |
| `channel/phone-channel.mjs` | status `waiting` → `waiting` event + spoken notice, once per turn. `waiting` no longer counted as turn end |
| `channel/phone-channel.mjs` | hold prompts only on `busy` / `waiting`. `shell` and unknown statuses deliver |
| `channel/phone-channel.mjs` | turn shorter than one 300 ms poll: seen through the transcript, not after the 8 s grace |
| `channel/phone-channel.mjs` | session exits with open work → `event error` before the shim dies |
| `channel/spawn-session.sh` | `--model` |
| `channel/spawn-session.sh` | tmux alive + shim dead (3 s) → kill, fresh spawn. Not our session → exit 2 |
| `channel/spawn-session.sh` | timeout 90 → 75 s (below the server's 90 s) |
| `channel/spawn-session.sh` | failure output: blank pane lines dropped, last 15 lines, reason repeated as the last line |
| `server/src/agents.rs` | registry `shell` → `Session.status` `idle` |
| `server/src/agents.rs` | `prompt` event detail cut to 500 chars |
| `server/src/agents.rs` | spoken spawn error = reason line only |
| `server/src/config.rs` | `Project.model`, checked charset, test |

Checks: `just lint-server`, `just test-server` (42 + 9 pass), `just lint-tools` clean.

## Open (not built)

| id | issue | note |
|---|---|---|
| D1 | Prompts held by the shim merge into one turn → one answer for N delegates | By design (mid-turn injection gets refused). Tasks all get the same `answer`. Voice model may wait for a second answer that never comes |
| D2 | Question vs final answer look the same: `agent.say` + `stop` | Agent can also `say` twice in one turn (interim + final). No "needs reply" flag. Fix idea: `say` tool arg `question: true` → task state `asking` |
| D3 | Haiku ignores the `say` instruction for plain questions (every one in this run, ~10), follows it when the prompt names the tool or after one use | Fallback speaks terminal text: may hold markdown (`**42**`, `×`). Fix idea: append a one-line reminder to each channel message, or strip markdown in the fallback |
| D4 | Failed spawn is retried by every queued delegate, each up to 75 s | Fix idea: fail all queued tasks of the project when a spawn fails |
| D5 | Prompt over the 1 MiB WebSocket frame cap: no `{error}`, socket-level failure only | `server.rs`. Fix idea: cap `prompt` in `delegate` (e.g. 100k chars) with a clear error; app-side check |
| D6 | `agent.say` with no socket connected, or server down: lost | Broadcast only. Fix idea: server queues unspoken `agent.say` per device and replays on reconnect; shim retries `say` while the server is down |
| D7 | `kill -9` of the shim mid-turn leaves the Claude session running its task with nobody listening | Lost-session check speaks after ~20 s; the orphan session is restarted only by the next delegate |
| D8 | Shim-dead restart loses the session context | Fix idea: respawn with `claude --resume <claude_session_id>` from the state file. Not tested |
| D9 | `waiting` relay is one way | User hears "stuck on permission prompt", can only answer in the Claude app / tmux. No approve-by-voice |
| D10 | Shim state is memory only | Shim restart or session death drops held prompts (reported as failed by the lost-session check) |
| D11 | Turn tracking reads `~/.claude/sessions/<pid>.json` and the transcript `.jsonl` | Claude Code internals. Format change → fallback text and `activity` stop, prompts still deliver |
| D12 | User global output style (here: Caveman) and `~/.claude` settings apply to agent sessions | Spoken answers inherit it. Fix idea: `--settings` / own `CLAUDE_CONFIG_DIR` per session |
| D13 | Server still SIGKILLs a script that overruns 90 s (flock wait, slow tmux) | Not seen after the 75 s script timeout. Would leak a half-started tmux session; the shim-dead restart heals it on the next delegate (seen once before the fix) |

## Signals for streaming status

| stage | signal now | missing |
|---|---|---|
| accepted | `tool.result {status:"sent", task_id, state, queue_pos}` | |
| queued behind the project lock | `Task.state queued`, `queue_pos` | |
| spawning | `Task.state spawning`, then `agent.event spawned` | no progress inside the 2–75 s spawn (dialog answered, MCP up) |
| prompt at the shim | `agent.event prompt`, `Task.state sent` | shim reply `ok` vs `queued` is dropped by the server: "held behind a running turn" not visible |
| turn started | `agent.event start`, `Task.state working` | |
| working | `agent.event activity` (last tool, 2 s throttle) | no text deltas, no percent; a 170 s shell command shows one `activity` line then silence |
| blocked | `agent.event waiting` + spoken notice, `resume` | what exactly is asked (tool, file) is not relayed, only `permission prompt` |
| question to the user | `agent.say` | not marked as a question (D2) |
| done | `agent.say` + `agent.event stop` (`answered` / `answered from the terminal text` / `no answer`) | |
| failed before delivery | `agent.event error` + `agent.say` | |
| session lost | shim `event error` (clean exit only), lost-session check 10–20 s | |
| session state | `sessions` tool: `idle` / `busy` / `waiting` / `dead` | `shell` (background command) folded into `idle` |
