# Agent context: pull, not paste

Status: proposal. Not built.

## Problem

- Agent gets only the `delegate` prompt Gemini wrote + `.taskbot.md`.
- Prompt is a paraphrase. Real loss seen: user said "context", prompt said "contacts".
- To recover, agent had to find `~/.local/state/taltosbot/taltosbot.db` and query sqlite by hand. Fragile: schema knowledge, WAL, no contract.
- Pasting the whole thread into the prompt fixes loss but floods the agent's context on every task.

## Idea

Small pointer in the prompt. Everything else on demand, through the shim the agent already talks to.

| layer | what | size | when |
|---|---|---|---|
| 0. prompt header | thread id + name, task id, user's last words verbatim | ~300 chars | always |
| 1. memory | thread memory (State + Recent) + pins | ≤ 3k chars | agent wants the picture |
| 2. turns | last N spoken entries, clipped | ≤ 4k chars / call | agent needs exact wording |
| 3. search | hits in the thread: `seq` + one line | ≤ 2k chars | agent looks for one thing |
| 4. raw | one entry, full: tool result, long agent answer | ≤ 16k chars / call, paged | rarely |

Agent climbs only as far as needed. Most tasks stop at 0 or 1.

## Layer 0: prompt header

Server prepends to the prompt sent to the shim (after the `.taskbot.md` line):

```
From taskbot thread 1742..3b24 "Character Archetypes", task a68a..bd18.
User said: "Also, when I restarted the conversation and I asked do you have contacts..."
More: thread_context tool.
```

- `User said`: last 1–3 `user` entries before the `delegate` call, verbatim, max 400 chars. Fixes paraphrase loss at near-zero cost.
- `Task.prompt` stays as given (as with the notes line today).

## Layers 1–4: MCP tools on the shim

Shim (`channel/phone-channel.mjs`) already: runs in every agent session, has `TALTOS_SERVER_URL`, knows `x-task-id` of the running turn. Add read-only tools next to `say`:

| tool | args | returns |
|---|---|---|
| `thread_context` | `{thread?, part?, last?}` | `part` = `memory` (default) / `turns`. `thread` omitted = thread of the running task |
| `thread_search` | `{pattern, thread?}` | `{hits:[{seq, kind, line}], truncated}`. Regex, case-blind, max 30 hits |
| `thread_entries` | `{thread?, from_seq, to_seq?, full?}` | entries in range. `full:true` = no clipping, tool results included, paged by `next_seq` |
| `task_info` | `{task?}` | `Task`: prompt, state, answer, error, thread. Omitted = running task |

Shim maps each to a loopback server call. No sqlite access from agents.

## Server: loopback read endpoints

| method path | returns |
|---|---|
| `GET /internal/context/thread/{id}?part=memory\|turns&last=N` | layer 1 / 2 |
| `GET /internal/context/thread/{id}/search?pattern=` | layer 3 |
| `GET /internal/context/thread/{id}/entries?from=&to=&full=` | layer 4 |
| `GET /internal/context/task/{id}` | `Task` |

- Guard: `loopback_only`, same as `/internal/agent/*`.
- `{id}` = thread id or name part (resolver shared with `memory_read` / `switch_thread`).
- Reads only. Never writes. Never touches `.taskbot.md`.

## Rules that keep context clean

- Hard char cap per call. Over → `truncated:true` + cursor (`next_seq`). Never silently huge.
- Default clipping: entry text 500 chars. Tool entry = one line: `[tool web_search("...")] → 1.2k chars, entry t…`. Body only via `full`.
- Markers, system notes, partial transcripts: skipped unless `full`.
- No auto-push of history into a running session. Ever.
- Tool descriptions tell the agent the ladder: memory first, then search, then entries.

## Why this way

| option | verdict |
|---|---|
| paste thread into prompt | loses nothing, floods every task. No |
| agent reads sqlite | works today, no contract, breaks on schema change. No |
| CLI `taltosbot context ...` | fine for humans; agent must know it exists. Later, thin wrapper over the same endpoints |
| **MCP tools on the shim** | already in every session, discoverable, loopback, testable. **Yes** |

Security: no new exposure. Agent runs as the server user and could read the db anyway. Endpoints are loopback only.

## Related, separate

- Fresh agent session per topic (`delegate` `fresh:true`): cuts the other pollution, old tasks piling up in one session. With layer 0–1 a fresh session loses little.
- Voice side: Gemini Live session resumption, so voice restarts keep context.

## Work

| part | change | est. |
|---|---|---|
| `docs/protocol.md` | "Agent sessions": header lines, endpoints, shim tools. First | small |
| `server/src/` | header in `agents::delegate`; 4 endpoints; shared thread resolver; caps | ~200 lines |
| `server/tests/` | header text, caps, truncation, loopback guard, resolver | ~150 lines |
| `channel/phone-channel.mjs` | 4 tools, task id → default thread | ~100 lines |
| `README.md`, `channel/README.md` | tool list | small |

Order: layer 0 alone first. Smallest, fixes the real failure. Then endpoints + tools.
