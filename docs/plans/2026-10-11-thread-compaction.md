# Thread compaction: thread memory

Status: built 2026-10-11.

## Problem

- Context = raw tail of the thread. `app/.../llm/History.kt`.
- Voice: last 40 entries / 12k chars, in the Live `systemInstruction`.
- Text: last 80 entries / 40k chars, as turns. Resent every turn.
- Older than the window: dropped. Decisions lost.
- Raw history is the wrong thing to carry. Latest status quo is what matters.

## Goal

- Each thread keeps a **thread memory**: where things stand now. Not a history.
- Context sent = base prompt + memory + turns since the last compact.
- User can read the memory any time, and force a compact.
- Never block speech. Never lose an entry.

## Thread memory

Plain text, caveman, keywords, bullets. Cap 1500 chars. Two sections:

```
## State
- vault notes go under projects/garden/
- watering plan: drip, not sprinkler (was: sprinkler)
- delegated vault: split plan.md per bed -> done, 4 files
- open: which pump

## Recent
- asked pH for tomatoes -> 6.2 to 6.8
- asked where plan.md is -> projects/garden/plan.md
```

| section | holds | rule |
|---|---|---|
| State | things agreed, decisions, goals, open questions, delegated tasks + outcome, exact vault paths | Latest wins: a changed decision replaces the old line, `(was: ...)` only when the change matters. No dates, no narration. |
| Recent | last 5 to 10 turns, one line each: `asked X -> Y` | Rolls. Older lines fall out. |

What goes where:

| turn | State | Recent |
|---|---|---|
| user decides / agrees / sets a goal | yes | no |
| user asks for info ("what is X") | no | yes, while recent |
| chit-chat, greetings | no | no |
| tool call | no. Path stays if a decision names it. | one-line gist at most |
| agent answer from `delegate` | task + outcome | no |

- Same language as the conversation.
- Archetypes (memory shape per thread type): later. Now: one shape, this one.

## Compact

Runs when any of:

| trigger | detail |
|---|---|
| turns | more than 15 user turns since the last compact |
| voice stop | `marker` `voice_ended` arrives in `threads.append`, at least 1 new turn |
| button | COMPACT in the app → `threads.compact` |

Turn = one `user` entry (typed or spoken).

Steps:

1. Server spawns a task. `threads.append` reply does not wait. `threads.compact` reply does wait.
2. One compact per thread at a time (lock per thread). Trigger during one → re-check when it ends.
3. Input: old memory + entries after the fold point, up to the cut.
4. One `generateContent` call → new memory. Hard cut at 1500 chars server-side.
5. Store memory + fold point in one `UPDATE ... WHERE summary_upto = <old>`. Lost race = drop the result.
6. Push `threads.memory`.

Cut = how far a compact folds:

- Leaves the last 4 counted entries verbatim. Every trigger.
- Stops before a `tool` entry with no `result`.

Counted = `user`, `model`, `agent` with text, `tool`. `marker`, `system` skipped.

| knob | value |
|---|---|
| input per entry | text 2000 chars, tool args 120, tool result 300 |
| input total | 24k chars per call. More → fold the oldest 24k, run again for the rest. |
| model | `gemini_summary_model`, default = `gemini_text_model` |
| timeout | 30 s |

After a compact the context is: memory + the last 4 entries. The Recent section carries the rest of the short-term thread.

### Failure

| case | result |
|---|---|
| no key, HTTP error, timeout, empty answer | old memory stays, fold point stays. `tracing::warn`. Button: `error` reply, shown in the panel. |
| retry | next trigger. Auto triggers: none within 60 s of a failure, per thread. Button: always. |
| no memory ever | app sends its tail window, same caps as today |

Fallback = today's behaviour.

## Where it runs: server

| | server | app |
|---|---|---|
| devices | one memory, all devices | each device compacts, races |
| runs when | after `threads.append`, app may be gone | voice stop often = app backgrounded |
| tests | Rust, mock Gemini, existing harness | no app test setup today |
| cost | `reqwest` needs TLS (`rustls-tls` feature) | none |

## Storage

`threads` gets columns (migration: `ALTER TABLE ... ADD COLUMN`, guarded):

| column | meaning |
|---|---|
| `summary` | memory text, NULL = none |
| `summary_upto` | `seq` of last folded entry, 0 = none |
| `summary_at` | epoch ms of last compact |
| `tokens_in`, `tokens_out` | all-time token use of the thread |

Entries untouched. Memory is derived: drop it, nothing lost.

## Tokens

| number | source |
|---|---|
| current context | `promptTokenCount` of the newest model call on the thread. App state, not stored. None yet → `~chars / 4`, marked `~`. |
| all time, thread | sum of `usageMetadata` over every call: text turns, voice sessions (app reports), compacts (server adds) |

- App → server: `threads.usage {id, input, output}` = increments. Sent after a text turn, and at voice stop.
- Voice: Live `usageMetadata` messages, summed per session.

## Protocol (doc first)

| C→S | payload | reply | payload |
|---|---|---|---|
| `threads.get` | `{id}` | `threads.get.result` | adds `memory?` and `usage` |
| `threads.compact` | `{id}` | `threads.compact.result` | `{memory}`. Nothing to fold → current memory. |
| `threads.usage` | `{id, input, output}` | `threads.usage.result` | `{usage}` |

- `memory` = `{text, upto_id, at}`. Absent = none. `upto_id` = `id` of the last folded entry.
- `usage` = `{input, output}`, token counts, all time.
- Push `threads.memory`: `{thread_id, memory, usage}`.
- `Thread` shape unchanged. Memory stays out of `threads.list`.
- "Server keeps entries opaque" relaxed: compact reads `text`, `project`, `tool`, and marker text. Never rewrites.
- Config: `gemini_summary_model` optional. `gemini_base_url` optional, default Google, for tests.
- App rules: history = memory + entries after `upto_id`.

## App

### Context (`History.kt`)

- Both builders take the memory.
- Entries after `upto_id` only. `upto_id` not in the loaded entries → ignore memory, tail window.
- Old caps stay as the safety net (40 / 12k voice, 80 / 40k text).
- Voice: memory block before the transcript block in `systemInstruction`.
- Text: memory appended to the system instruction. Not a fake turn.
- Tool results, text mode: 1500-char clip only after the last `model` entry. Older: 200.
- Running voice session: prompt is fixed at start. New memory applies at next voice start. Text: next turn.

### UI

- Right side = this thread. Top bar, right end: hamburger. Opens a panel sliding in from the right.
- Left side = threads. Thread list moves into the left drawer, under the navigation. Threads icon on the right goes.

```
Thread                          [x]
Garden plan
created  2026-10-09 18:02
last     5 min ago
entries  84

Context now     ~3.1k tokens
All time        212k in / 9k out

[ Long CTX > ]
```

Long CTX = second page of the same panel:

```
< Long CTX
compacted 5 min ago, 6 turns since

## State
- ...
## Recent
- ...

[ COMPACT ]
```

- Memory text, read-only, selectable, scrolls.
- COMPACT: spinner while it runs, error line on failure. Disabled when disconnected.
- No memory yet: "nothing compacted yet" + the button.
- `threads.memory` push while open → text updates.
- New file `ui/ThreadPanel.kt`. `App.kt`: one icon, one panel call.

## Expected size

Long thread, per turn:

| | today | after |
|---|---|---|
| voice | up to 12k chars | 1.5k memory + 4 entries to ~15 turns |
| text | up to 40k chars | 1.5k memory + 4 entries to ~15 turns, less tool bulk |

Sawtooth: grows to 15 turns, compact cuts it back.

## Measure

- App: log chars of history sent, per text turn and per voice start (`EventLogger`).
- Token numbers in the panel = the same measure, live.
- Server test: long synthetic thread, context chars before vs after.
- Report real numbers from the live db for one long thread.

## Tests (server)

- Triggers: 15 turns → none, 16 → one call. `voice_ended` marker → one call. `threads.compact` → one call, reply carries the memory.
- Cut: 4 entries left, pending tool entry stops the cut.
- Compact: memory stored, fold point moves, entries intact, `threads.get` returns `memory`, push arrives.
- Second compact feeds the old memory in.
- Fallback: Gemini 500 / timeout → memory unchanged, backoff holds, button gets `error`.
- Race: two triggers, one call.
- Cap: 5k-char answer stored as 1500.
- Usage: `threads.usage` adds up, compact adds its own tokens.
- Migration: db without the columns opens.

Mock Gemini = local axum listener, `gemini_base_url` points at it.

## Files

| file | change | shared with |
|---|---|---|
| `docs/protocol.md` | first | |
| `server/src/compact.rs` | new: prompt, call, triggers | |
| `server/src/state.rs` | columns, store fns | main thread: small hunks |
| `server/src/server.rs` | trigger after append, 2 events, get reply | main thread: small hunks |
| `server/src/config.rs`, `config.example.toml` | 2 keys | |
| `server/Cargo.toml` | `rustls-tls` | |
| `server/tests/integration.rs` | above | |
| `app/.../llm/History.kt` | memory in, tool shrink | |
| `app/.../llm/GeminiTextClient.kt`, `voice/*` | read `usageMetadata` | |
| `app/.../connection/*`, `ui/AppController.kt` | memory, usage, compact call, push | app threads: small hunks |
| `app/.../ui/ThreadPanel.kt` | new | |
| `app/.../ui/App.kt`, `ui/ThreadsPanel.kt` | right icon + panel, thread list into the left drawer | main thread: small hunks |
| `e2e/mock-server.mjs` | `threads.compact`, `threads.usage` stubs | |
| `README.md` | config keys | |

## Not in scope

- Memory archetypes per thread type.
- Editing the memory by hand.
- Compacting `delegate` answers (`agent.say` text).
- Deleting or rewriting stored entries.
- Refreshing the prompt of a running voice session.
- Token cost in money.

## Built: notes

- Real model check, `gemini-flash-latest`, 20-turn synthetic thread: 2571 chars folded → 769-char memory. Second compact: 882 chars.
- Fold took 14 s and ~2000 output tokens: the text model thinks. Cheaper: set `gemini_summary_model` to a lite model.
- Live db had no long thread to measure. App logs `Context: ...` per turn: read the Logs screen on a real thread.
- Timeout path: same code as HTTP error, no own test (would take 30 s).
- Model puts decision turns into Recent too. Fine: Recent = short-term thread.
