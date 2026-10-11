# Memory pins: model-callable memory

Status: built 2026-10-12, all three steps. Contract as built: `docs/protocol.md`. Not run against a live voice session yet.

Builds on `2026-10-11-thread-compaction.md`. Contract today: `docs/protocol.md` "Thread memory".

## Verdict

Yes. Four tools: note, forget, read, set project. Three scopes: `always`, `project`, `thread`.

| gap named | tool closes it? | why |
|---|---|---|
| user cannot say "remember this" / "forget that" | **yes** | Today the model says "noted" and nothing is stored until a fold, and no fold is bound to keep it. |
| decision lost across folds | **yes, for pinned lines** | fold is lossy, 1500-char cap, "keep every bullet" is a prompt wish. Pin = kept by code. |
| nothing carries from one thread to the next | **yes** | `always` pins + read of another thread's memory |
| compact during a voice session does not reach the running model | no, and not needed | running model holds the whole session in its own context. Memory = lossy copy of that. |
| fold costs 14 s / ~2000 tokens | no | folds still run. Fix apart: lite `gemini_summary_model`, or thinking off in the fold call. |
| Live context grows inside a session | no | Live API matter: context compression, resumption. Fork `spin-tb-live-limits`. |

## Decided (user, 2026-10-11)

| question | answer |
|---|---|
| organise how | two scopes: `always` (every thread) + topic. Topic = the thread. |
| topic memory in the prompt | own thread: yes. Other threads: on demand, by read tool. |
| `memory_forget` and `## State` bullets | pins only. State bullets stay; the summariser owns them. |
| caps | no cap on the number of pins. Upper limit in chars per scope. Near it: tell the user to clean up. |
| project | thread has an active project. Project memory = `.taskbot.md` in the project folder. |
| names, pins apart from State, summariser reads pins only | as proposed |

Three scopes now: `always`, `project`, `thread`. Project scope: own section below.

## Scopes

| | Always | Pinned (thread) | State (thread) |
|---|---|---|---|
| holds | rules and facts for every conversation | standing rules of this thread + things the user said to remember | where things stand now |
| example | `answer in Hungarian`, `vault notes: terse keyword style` | `budget max 200 EUR`, `notes go under projects/garden/` | `watering plan: drip` |
| written by | model, by tool | model, by tool | summariser, after the fact |
| changed by | tool only | tool only | next fold |
| in the prompt | every thread, always | its own thread, always | its own thread, always |
| from another thread | already there | `memory_read` | `memory_read` |
| lost when | never, until forgotten | never, until forgotten | fold drops it |

When the model pins, and where:

| case | pin | scope |
|---|---|---|
| user says remember / note / keep in mind / do not forget | yes | `thread` |
| user sets a standing rule for this conversation | yes, without mentioning it | `thread` |
| user says always / everywhere / in every conversation / from now on | yes | `always` |
| ordinary decision, goal, open question | no. State gets it at the next fold. | |
| info question, chit-chat | no | |

- Unsure which scope → `thread`. `always` costs tokens in every thread.
- Not called → nothing lost: the turn is an entry, the fold still reads it. Tool = fast path, summariser = backstop.

| knob | value |
|---|---|
| number of pins | no cap, any scope |
| one pin | one line: control chars dropped, newlines → space, trimmed. Over 300 chars → cut. Guard against a runaway call, not a budget. |
| language | as spoken |

## Limits

Size = chars of the scope as it goes into the prompt.

| scope | limit | warn at |
|---|---|---|
| `always` | 1200 | 80% |
| `thread` pins | 1500 | 80% |
| `project` file | 3000 | 80% |

Worst case in one prompt: 5.7k chars of pins + 1.5k thread memory, about 1.8k tokens.

| state | what happens |
|---|---|
| under warn | nothing |
| warn to limit | write still works. Tool result carries `warning`. Prompt block of that scope ends with one line: `<scope> memory <n>% full: tell the user once, offer to clean it up`. Panel: meter turns amber. |
| write would pass the limit | `{error:"<scope> memory full", size, limit, pins}`. Nothing stored. Model tells the user and offers to drop notes. `replaces` that shrinks the scope still works. |
| `.taskbot.md` grown past the limit by hand or by an agent | file left alone. Prompt gets the first 3000 chars, cut on a line end, plus the full line above. Panel: meter red. |

- Every reply and push that carries pins also carries `size` and `limit` of that scope.
- Clean up = by voice: the model has the list, proposes what to drop, calls `memory_forget`. Project file: also by hand, or by the agent.
- Why warn: every pin rides in every prompt. Long list = each line weighs less, old rules fight new ones.

## Tools

| tool | args | result |
|---|---|---|
| `memory_note` | `{text, scope?, replaces?}` | `{scope, pins:[Pin], size, limit, warning?}` = the list of that scope after the write. `scope` = `thread` (default) / `project` / `always`. `replaces` = part of an old pin of that scope: swapped in place. Same text again (case-insensitive) → no change, no error. |
| `memory_forget` | `{match}` | `{forgotten, from, pins:[Pin], size, limit}`. `from` = `thread` / `project` / `always`. Removes the one pin that contains `match`. |
| `memory_read` | `{thread?, project?}` | No arg: `{threads:[{id, name, project?, pins, memory}], projects:[name]}` = other threads that hold pins or a memory (newest `last_accessed_at` first, max 50) + projects that have a `.taskbot.md`. `pins` = count, `memory` = bool. With `thread`: `{thread:{id, name}, pins:[Pin], memory?}`, `memory` = the memory text. With `project`: `{project, text}` = its `.taskbot.md`. |
| `set_project` | `{project}` | `{thread:Thread}`. Sets the active project of the thread. `""` = none. Unknown name → `{error}` + the names. Pushes `threads.changed`. |

- Thread = `tool.call.thread_id`. `memory_note` scope `thread` without it → `{error}`. Same as `rename_thread`.
- Scope `project` on a thread with no active project → `{error:"no active project"}`.
- `match`, `replaces`: case-insensitive substring. `memory_forget` searches thread pins, then project pins, then `always`.
- No hit → `{error:"no pinned note matches", pins}`. More than one → `{error:"<n> notes match", matches:[text]}`. Model asks or retries.
- `replaces` with no hit → plain add.
- `memory_forget` never touches `## State` or `## Recent`. Not pinned → the error above. The "forget" turn is an entry: the next fold reads it and drops the bullet.
- `memory_read` `thread` = thread `id`, or part of its name (case-insensitive). One hit, else `{error}` + the names that match.
- `memory_read` reads only. Pins of another thread change only from inside that thread.
- `memory_note` / `memory_forget` push: thread scope → `threads.memory`, `always` → `memory.always`, `project` → `memory.project`.
- All four: sqlite or one small file, no model call. Fine as blocking Live tools.

## Project scope

Tracks a project across threads. Thread memory dies with its thread; the project file stays.

### Active project

- `Thread` gains `project?` = name from `[projects.*]`. One per thread. Absent = none.

| set by | how |
|---|---|
| `delegate` | thread has none → project of the call becomes active. Has one → no change. |
| model | `set_project`, when the user says what they work on, or switches |
| user | project picker in the thread panel → `threads.set_project` |

- Change → `threads.changed`. Next prompt carries the new project's file.
- Thread list can show and filter by project: later, the field is there.

### File

`<project dir>/.taskbot.md`. The file is the store: no copy in sqlite. Plain markdown, readable and editable by the user and by the project's agent.

```
# Taskbot: garden

## Pinned
- notes go under beds/, one file per bed
- budget max 200 EUR

## Tasks
- 2026-10-12 split plan.md per bed -> done: 4 files
- 2026-10-11 price list for pumps -> failed: agent session is gone
```

| section | holds | written by |
|---|---|---|
| `## Pinned` | rules and facts of the project | `memory_note` / `memory_forget`, scope `project` |
| `## Tasks` | last 20 `delegate` tasks of the project, newest first: date, prompt cut to 80 chars, `done` / `failed` + answer or error cut to 120 | server, when a task ends. No model call. |
| anything else | free | user, agent. Server never changes it. Goes into the prompt with the rest. |

Why `## Tasks`: task rows leave the db 7 days after they end. The file keeps what was done, with no model call and nothing made up.

Server write rules:

- Touches only lines under `## Pinned` and `## Tasks`. One bullet added, swapped or removed per write. All other bytes stay.
- Section missing → added at the end. File missing → made on the first write. Never made on read.
- Read, change, write under a lock per project. Write = temp file in the same folder + rename.
- Read fresh on every use. Hand edits and agent edits count at once.
- No `.gitignore` edit. Commit it or ignore it: user's call.
- Not UTF-8 or over 64 KiB → `{error}` on write, skipped on read, `tracing::warn`.

This breaks "the server only reads, all writes go through an agent". Kept narrow: one fixed file name, inside a configured project `dir`, two sections.

### In the prompt

- Thread with an active project: block `<project name="...">` = the file, after `always`, before `<pinned>`. Cut per "Limits".
- Other projects: `memory_read {project}`.
- Vault tools skip dot-entries, so `read_file` does not see the file. Server serves it.

### Agent side

- `delegate` to a project that has the file: one line added on top of the prompt: `Project notes kept by taskbot: .taskbot.md. Read it first.`
- Agent may edit the file. Same file, both models.

### Later

- `## State` of the project, made by a fold over the threads of that project. Second fold job, own lock, a model rewriting a file on disk: not in this step.

## Summariser

- Fold input gets a third block: `<pinned>` = `always` pins + project `## Pinned` + the thread's pins, read-only.
- Two new rules in `INSTRUCTION`:
  - Pinned notes live elsewhere. Never repeat one in State.
  - State bullet that a pinned note contradicts: drop it.
- Fold never writes pins. Pins are outside the 1500-char cap.
- `[tool memory_note(...)]` lines in the fold input: covered by the rules above.
- `store_memory` unchanged: no tool writes `summary`.

Who wins: thread pin beats project pin beats `always` pin beats State. User beats all (by `memory_forget` / `replaces`).

## Storage

New table (guarded `CREATE TABLE IF NOT EXISTS`):

| column | meaning |
|---|---|
| `seq` | integer primary key, autoincrement. List order. |
| `thread_id` | thread `id`. NULL = `always`. Index. |
| `text` | the pin |
| `at` | epoch ms, write or last replace |

`Pin` = `{text, at}`.

Open for archetypes: a `section` column later; an archetype then names its sections. Tool arg `section?` not offered now.

## Protocol (doc first)

| where | change |
|---|---|
| `threads.get.result` | adds `pins:[Pin]` (thread) and `always:[Pin]`. Always there, may be empty. |
| `threads.create.result` | adds `always` |
| `threads.compact.result` | adds `pins` |
| push `threads.memory` | `{thread_id, memory?, pins, usage}`. `memory` now optional: pins exist before the first compact. Sent after every stored compact and every thread pin change. |
| push `memory.always` (new) | `{pins:[Pin]}`. Sent after every `always` change. App swaps the list for all threads. |
| push `memory.project` (new) | `{project, text, size, limit}`. Sent after every server write to a `.taskbot.md`. Hand edits: no push, seen at the next `threads.get`. |
| `Thread` | adds `project?` |
| `threads.get.result` | adds `project_memory?` = `{project, text, size, limit}` when the thread has an active project with a file |
| `threads.set_project` (new) | `{id, project}` → `threads.set_project.result` `{thread:Thread}`. `""` = none. |
| pin lists everywhere | come as `{pins, size, limit}` |
| `Memory` | unchanged |
| "Tools" table | 4 rows. `delegate`: sets the active project, adds the notes line. Top line "Gemini gets read-only vault tools": exception for `.taskbot.md`. |
| "Thread memory" | sections "Pins" (scopes, who wins, limits) and "Project memory" (file, write rules) |

## App

### Context (`History.kt`)

- Both builders take `always` and the thread pins.
- Block order: base prompt, always, project, pinned, memory, transcript.
- Scope at or over its warn mark: the "tell the user once" line from "Limits" closes its block.
- Pin blocks go in always. Not tied to `upto_id`: a memory that cannot be used does not hide the pins.
- Empty list → no block.
- Block text: `Rules and facts for every conversation, set by the user.` + `<always>` lines. `Pinned for this conversation by the user or by you. Hold as true, above the memory below.` + `<pinned>` lines.
- Text mode: next turn sees new pins. Voice: running session already holds the tool call and its result; next voice start reads them from the prompt.
- New thread, no entries: `always` block still goes in.

### Prompt (`AppController.kt`, base prompt)

One paragraph:

- User says remember / note / do not forget → `memory_note`. Standing rule for this conversation → `memory_note`, without mentioning it.
- User says always / everywhere / from now on → `memory_note` with scope `always`.
- User says forget / drop / no longer true → `memory_forget`.
- One short keyword line per note. Changed rule → `replaces`. Not for ordinary decisions.
- User refers to another conversation or project ("what did we say about the garden") → `memory_read`.
- User says what project they work on → `set_project`. Rule or fact of the project → `memory_note` with scope `project`.
- Tool result has `warning`, or a memory block says it is nearly full → tell the user once, offer to clean up.

### UI (`ThreadPanel.kt`)

- First page: `project` row with a picker (`threads.set_project`).
- Long CTX page: `## Always`, `## Project`, `## Pinned` above the memory text. Read-only. Shown also when nothing is compacted yet.
- Each list: meter `size / limit`. Amber from the warn mark, red when full or cut.
- `threads.memory` / `memory.always` / `memory.project` push while open → lists update.

## Risks

| risk | answer |
|---|---|
| voice model does not call the tools unprompted | explicit commands are the main trigger. Rest = best effort, fold is the backstop. `memory_read` has no backstop: a miss = "I do not know". |
| pins pile up, prompt grows in every turn | limits per scope, warning from 80%, meter in the panel. App logs pin chars per context build (`EventLogger`). |
| model does not pass the warning on | the panel meter shows it without the model. |
| server write clobbers a hand or agent edit of `.taskbot.md` | fresh read under the lock right before the write, line-level change, rename. Window = milliseconds. |
| `.taskbot.md` shows up as untracked in project repos | wanted or ignored: user's call. Server never touches `.gitignore`. |
| agent writes junk into the file, it lands in every prompt of the project | limit cuts it, meter shows it, file is plain text to fix. |
| `delegate` sets the wrong active project (one-off task elsewhere) | only when the thread has none. Picker and `set_project` fix it. |
| model pins into `always` by mistake | default `thread`. Result names the scope; panel shows it. `memory_forget` reaches both. |
| model says "noted" without calling | same as today. Seen in the panel: no pin. |
| substring hits the wrong pin | one hit only, result names the pin removed. Model reads it back. |
| `memory_read` leaks one thread into another | wanted. One user, one vault. Read-only. |
| long session, Live sliding window drops a tool result | pin or read made mid-session falls out of the live context until the next start. Only if compression gets turned on. Then: re-inject pins. Needs the `spin-tb-live-limits` findings. |

## Measure

- Real voice thread, 12 scripted lines: 3 "remember", 2 "forget", 2 standing rules, 2 "always", 3 plain decisions. Second thread: 2 questions about the first.
- Count `memory_note` / `memory_forget` / `memory_read` tool entries against the script. Report hits, misses, wrong scope, wrong pins.
- Under 2 of 3 on the explicit commands → rework descriptions + prompt before calling it done.

## Tests (server)

- `memory_note`: adds, pushes `threads.memory`, `threads.get` returns `pins`. Scope `thread` without `thread_id` → `error`.
- Scope `always`: stored with no thread, pushes `memory.always`, shows in `threads.get` of two threads and in `threads.create.result`.
- Dedupe: same text twice → one pin.
- `replaces`: one hit swaps in place, keeps order; no hit adds.
- No count cap: 50 short pins stored. 1000-char text stored as 300, one line.
- Limits: write under warn → no `warning`; over 80% → `warning`; past the limit → `error`, nothing stored; shrinking `replaces` on a full scope works.
- Active project: `delegate` sets it once, not twice. `set_project`, `threads.set_project`: set, clear, unknown name → `error`. `threads.changed` pushed.
- Project file: first `memory_note` makes it; add, swap, remove touch one line; text outside the two sections byte-equal after a write; missing section added.
- Project file: task `done` and `failed` each add a `## Tasks` line; 21st drops the oldest.
- Project file: hand-made 5000-char file → `threads.get` gives 3000 cut on a line end, file unchanged. Non-UTF-8 file → write `error`, read skipped.
- Scope `project` with no active project → `error`.
- `delegate` prompt carries the notes line only when the file is there.
- `memory_forget`: thread hit, `always` hit (`from`), none, many. Text that sits only in `## State` → `error`, `summary` untouched.
- `memory_read`: list leaves out the calling thread and empty threads; by id; by name part; two names match → `error`.
- Fold keeps pins: compact with pins set → pins unchanged, mock Gemini request carries `<pinned>` with both scopes.
- Pins before any compact: `threads.get` has `pins`, no `memory`.
- Migration: db without the table opens.
- `exactly_the_contract_tools` list: 12 names.
- Temp dirs for project folders: under `$TMPDIR`, never `/tmp`.

## Files

| file | change | shared with |
|---|---|---|
| `docs/protocol.md` | first | |
| `server/src/tools.rs` | 3 declarations, 3 arms, test list | |
| `server/src/memory.rs` | new: pin rules, match, read, limits | |
| `server/src/project_file.rs` | new: parse sections, line edits, atomic write, lock per project | |
| `server/src/agents.rs` | `delegate`: active project, notes line. Task end: `## Tasks` line | other threads: small hunks |
| `server/src/state.rs` | `pins` table, read/write fns | other threads: small hunks |
| `server/src/compact.rs` | `<pinned>` block, 2 rules, `pins` in payload | |
| `server/src/server.rs` | `pins`, `always` in `threads.get` / `threads.create` replies | other threads: small hunks |
| `server/tests/compact.rs`, `integration.rs` | above | |
| `app/.../connection/*` | `pins`, `always` in replies, 2 pushes | app threads: small hunks |
| `app/.../llm/History.kt` | 2 pin blocks | |
| `app/.../ui/AppController.kt` | prompt paragraph, pins + always state, context log | app threads: small hunks |
| `app/.../ui/ThreadPanel.kt` | Always + Pinned lists | |
| `e2e/mock-server.mjs` | `pins`, `always` in stubs | |

## Not in scope

- Editing pins by hand in the app.
- Project `## State` made by a fold.
- Thread list filtered by project.
- Named topics apart from threads and projects.

## Build order

| step | holds | alone useful |
|---|---|---|
| 1 | pins: `thread` + `always`, 3 memory tools, limits + warning, panel lists | yes |
| 2 | active project: `Thread.project`, `set_project`, picker, `delegate` sets it | yes |
| 3 | `.taskbot.md`: scope `project`, `## Tasks`, prompt block, notes line for the agent | needs 2 |

Each step: `protocol.md` first, tests, `just lint-server` + `just test-server` green, app debug build.
- Search over the memory of all threads (`memory_read` goes by thread).
- Refreshing the prompt of a running voice session.
- Trimming the Live session context.
- Archetype sections.
- Cheaper fold (summary model, thinking off): own small change.

## Built: notes

- Tool list is 14 names, not 12: other tools landed meanwhile.
- `## Tasks` hook sits in `server/src/tasks.rs` (`tasks::push`), the one place every `done` / `failed` passes. Not `agents.rs`.
- Tasks failed at server start (`server restarted before the prompt was delivered`): no `## Tasks` line. They are marked before the app state exists.
- File lock: static map per path in `project_file.rs`.
- Project pins: `at` = last write of the file. The file holds no time per line.
- `memory_forget`: first scope with a hit decides; "n notes match" counts inside that scope.
- `<pinned>` fold block left out when there are no pins.
- App picker: project names from the `projects` tool, called when the thread panel opens.
- App: project change → `threads.get` again, to get `project_memory`.
- Gap: no push for "`.taskbot.md` is gone". App sees it at the next `threads.get`.
- Gap: project removed from the config → `Thread.project` still names it, no `project_memory`, scope `project` errors.
- "Measure" script: not run. Whether the voice model calls the tools unprompted: unknown.
