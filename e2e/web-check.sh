#!/usr/bin/env bash
# Web smoke check: serve the built JS app + the mock server, pair by pasting a connect string,
# then walk chat / threads / text mode / settings, one screenshot per state into e2e/evidence/.
# The app is a canvas, so this drives it by coordinates (viewport 420x800) and the result is
# judged by looking at the screenshots / the logs printed.
#
#   just build-web && e2e/web-check.sh
#
# Needs: node, python3, agent-browser, a Chromium (AGENT_BROWSER_EXECUTABLE_PATH if not installed;
# AGENT_BROWSER_ARGS=--no-sandbox where Chromium has no usable sandbox).
# Text mode talks to the mock's fake Gemini: localStorage `gemini_text_base_url` points there.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
dist="${DIST:-$root/app/build/dist/js/productionExecutable}"
out="$root/e2e/evidence"; mkdir -p "$out"
web="${WEB_PORT:-8765}"; api="${API_PORT:-55577}"
export AGENT_BROWSER_SESSION="${AGENT_BROWSER_SESSION:-taltos-web-check}"

python3 -m http.server "$web" --bind 127.0.0.1 -d "$dist" >/dev/null 2>&1 & p1=$!
node "$root/e2e/mock-server.mjs" "$api" > "$out/mock.log" 2>&1 & p2=$!
trap 'kill $p1 $p2 2>/dev/null; agent-browser close >/dev/null 2>&1 || true' EXIT
sleep 1
connect="$(sed -n 's/^connect string: //p' "$out/mock.log")"

ab() { agent-browser "$@" >/dev/null; }
click() { ab mouse move "$1" "$2"; ab mouse down; ab mouse up; }
shot() { ab wait "${2:-1000}"; ab screenshot "$out/$1.png"; echo "  $out/$1.png"; }
# The canvas only takes real key presses, one per character.
typ() { local s="$1" i c; for ((i = 0; i < ${#s}; i++)); do c="${s:i:1}"; [ "$c" = " " ] && c=Space; ab press "$c"; done; }
say() { click 190 766; ab wait 300; typ "$1"; ab press Enter; }
mock() { curl -sf -X POST "http://127.0.0.1:$api$1" ${2:+-d "$2"} >/dev/null; }

ab open "http://127.0.0.1:$web/"; ab set viewport 420 800
ab eval "localStorage.clear(); localStorage.setItem('gemini_text_base_url', 'http://127.0.0.1:$api')"; ab reload
shot 01-pairing 4000

# "Paste connect string..." opens window.prompt, which blocks the page: the mouse-up never
# returns, so let it time out, then answer the dialog.
ab mouse move 210 326; ab mouse down; timeout 5 agent-browser mouse up >/dev/null 2>&1 || true
ab dialog accept "$connect"
shot 02-connected 3000

# Text mode: first message creates the thread; the fake Gemini asks for read_file (1.5 s in the mock).
say "read my inbox"
shot 03-tool-running 700
shot 04-tool-collapsed 3000
click 200 125;                            shot 05-tool-expanded 600
say "rename it";                          shot 06-renamed-by-model 3500   # rename_thread -> threads.changed -> title

# Input grows to 6 lines, then scrolls inside (Shift+Enter = line break). Enter sends it.
click 190 766; for n in 1 2 3 4 5 6 7; do typ "line $n"; ab press Shift+Enter; ab wait 200; done; typ "line 8"
shot 07-input-grows 400
ab press Enter; ab wait 1500

# Delegated task: the strip above the chat follows `tasks.changed` (mock plays one task in ~8 s).
say "delegate it";                        shot 07a-task-starting 2600
shot 07b-task-working 3000                # spinner, last_activity, elapsed time ticking
shot 07c-task-done 4500                   # green dot + answer; agent bubble below
click 200 70;                             shot 07d-task-expanded 600

# Thread panel (right hamburger): metadata + tokens, then Long CTX with the memory after COMPACT.
click 394 28;                             shot 07e-thread-panel 1200
click 260 312;                            shot 07f-long-ctx-empty 600
click 260 762;                            shot 07g-long-ctx-compacted 1200
click 50 400; ab wait 600                 # scrim closes it

# Left drawer: navigation + threads. Select the seeded one, rename it, start a new one.
click 28 28;                              shot 08-threads 1200
click 150 430;                            shot 09-thread-selected 1500
click 28 28; ab wait 1200; click 336 374; ab wait 500; typ " 2"
shot 10-rename-inline 300
click 288 374;                            shot 11-thread-renamed 1200
click 180 313;                            shot 12-new-thread 1500

mock /internal/agent/say '{"project":"vault","text":"Saved the note to inbox/2026-10-10.md."}'
mock /internal/agent/event '{"project":"vault","kind":"stop","detail":"turn finished"}'
shot 13-agent-say

click 347 28; ab wait 700; click 200 416; ab wait 500; click 200 520
shot 14-settings-tools 600                # dot -> Settings, "N tools" open, one tool open

click 28 28; ab wait 700; click 50 130;  shot 15-sessions
click 28 28; ab wait 700; click 34 226;  shot 16-logs

# Reload reopens the last thread. That one is empty, so pick the seeded thread first.
click 28 28; ab wait 1200; click 150 486; ab wait 1200
ab reload;                                shot 17-reopened-after-reload 5000

mock "/mock/down?ms=7000"                 # server gone: chat stays readable, a send fails visibly
ab wait 1500; say "anyone there"
shot 18-offline-send 500
ab wait 9000

mock /mock/revoke                         # token gone -> 401 -> back to pairing
shot 19-revoked 4000

echo "--- mock server saw:"; cut -c1-400 "$out/mock.log"
echo "--- app console:"; agent-browser console | grep -F '[Taltosbot]' || true
