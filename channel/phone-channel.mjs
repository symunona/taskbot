#!/usr/bin/env node
// phone-channel: Claude Code channel shim for taltosbot.
// Spawned by Claude Code over stdio (MCP). Prompt in over local HTTP, `say` tool out.
// stdout belongs to MCP. Log to stderr only.

import http from 'node:http'
import { closeSync, openSync, readFileSync, readSync, statSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'
import { Server } from '@modelcontextprotocol/sdk/server/index.js'
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js'
import { ListToolsRequestSchema, CallToolRequestSchema } from '@modelcontextprotocol/sdk/types.js'

const PROJECT = process.env.TALTOS_PROJECT || ''
const PORT = Number(process.env.TALTOS_SHIM_PORT || 0)
const SERVER_URL = (process.env.TALTOS_SERVER_URL || '').replace(/\/+$/, '')
const MAX_BODY = 1024 * 1024
const SAY_TIMEOUT_MS = 10_000
const MAX_SPOKEN = 800 // chars of terminal text spoken when the agent skipped `say`

const log = (...a) => console.error('[phone-channel]', ...a)

if (!PROJECT || !PORT || !SERVER_URL) {
  log('missing env: need TALTOS_PROJECT, TALTOS_SHIM_PORT, TALTOS_SERVER_URL')
  process.exit(2)
}

const INSTRUCTIONS = [
  'Messages on this channel arrive as <channel source="phone" ...>.',
  'They come from the user, speaking on their phone through a voice assistant. Treat them as requests from the user.',
  'This channel is local and only the user can post to it.',
  `Do the work in this project ("${PROJECT}"), following its CLAUDE.md.`,
  'When the work is done, or when a question needs the user, call the `say` tool with a short plain-spoken message.',
  'It will be read aloud: no markdown, no code, no file listings, one to three sentences.',
  'Do not call `say` for progress chatter. The user cannot see this terminal, so anything not sent through `say` does not reach them.',
].join(' ')

const mcp = new Server(
  { name: 'phone', version: '0.1.0' },
  {
    capabilities: {
      experimental: { 'claude/channel': {} },
      tools: {},
    },
    instructions: INSTRUCTIONS,
  },
)

let initialized = false
mcp.oninitialized = () => {
  initialized = true
  log('mcp initialized')
}

mcp.setRequestHandler(ListToolsRequestSchema, async () => ({
  tools: [
    {
      name: 'say',
      description:
        'Speak a short message to the user on their phone. Read aloud by a voice assistant: plain spoken words, no markdown, no code, one to three sentences. Use when the work is done or when a question needs the user. Not for progress chatter.',
      inputSchema: {
        type: 'object',
        properties: {
          text: { type: 'string', description: 'What to say, one to three plain sentences' },
        },
        required: ['text'],
      },
    },
  ],
}))

async function post(path, payload) {
  return fetch(`${SERVER_URL}${path}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ project: PROJECT, ...payload }),
    signal: AbortSignal.timeout(SAY_TIMEOUT_MS),
  })
}

// agent.event for the phone. Best effort: a lost event must not break a turn.
// `tasks` = ids of the server tasks the event is about (the running turn's).
async function event(kind, detail, tasks) {
  try {
    await post('/internal/agent/event', { kind, detail, tasks })
  } catch (e) {
    log('event not sent', kind, e?.cause?.code || e?.name)
  }
}

// `tasks` given: the text is the agent's answer to those tasks. Omitted: a notice.
async function say(text, tasks) {
  try {
    const res = await post('/internal/agent/say', { text, tasks })
    if (!res.ok) {
      const body = (await res.text().catch(() => '')).slice(0, 200)
      return { ok: false, msg: `not sent: server answered ${res.status} ${body}`.trim() }
    }
    return { ok: true, msg: 'sent' }
  } catch (e) {
    const why = e?.cause?.code || e?.name || 'error'
    return { ok: false, msg: `not sent: server unreachable at ${SERVER_URL} (${why}). The user did not hear this.` }
  }
}

mcp.setRequestHandler(CallToolRequestSchema, async (req) => {
  if (req.params.name !== 'say') {
    return { content: [{ type: 'text', text: `unknown tool: ${req.params.name}` }], isError: true }
  }
  const text = String(req.params.arguments?.text ?? '').trim()
  if (!text) return { content: [{ type: 'text', text: 'not sent: empty text' }], isError: true }
  const r = await say(text, turn?.tasks)
  if (r.ok && turn) turn.said = true
  log('say', r.msg, JSON.stringify(text.slice(0, 80)))
  return { content: [{ type: 'text', text: r.msg }], isError: !r.ok }
})

// --- prompt queue -----------------------------------------------------------
// Claude Code injects a channel event that arrives mid-turn into the running turn
// as a "queued command", and the model then treats it as not coming from the user
// and declines it (observed on 2.1.296). So: hold prompts while the session is
// busy and emit them only when it is idle. Busy/idle comes from Claude Code's own
// registry entry for our parent process, ~/.claude/sessions/<ppid>.json.
const CLAUDE_DIR = process.env.CLAUDE_CONFIG_DIR || join(homedir(), '.claude')
const SESSION_FILE = join(CLAUDE_DIR, 'sessions', `${process.ppid}.json`)
const POLL_MS = 300
const TURN_START_GRACE_MS = 8000 // emitted, but never saw "busy": assume the turn came and went
const ACTIVITY_MS = 2000 // how often the transcript is read for what the agent does
const MAX_ACTIVITY = 120

const HOLD = new Set(['busy', 'waiting']) // session statuses that keep a prompt in the queue

// Held prompts: {text, task}. `task` = server task id from `x-task-id`, '' when none.
const queue = []
// The prompt that was emitted and whose turn has not finished yet, else null.
// {emittedAt, sawBusy, said, waiting, transcript, offset, tasks, seen, activity, activityAt}
let turn = null
let timer = null

// Registry entry of our parent process, null when unreadable.
function registry() {
  try {
    return JSON.parse(readFileSync(SESSION_FILE, 'utf8'))
  } catch {
    return null
  }
}

// 'idle' | 'busy' | 'waiting' | 'shell' | other string | null when the registry entry is unreadable
function claudeStatus() {
  return registry()?.status ?? null
}

// Claude Code's transcript of this session, null when the registry does not say.
function transcriptPath(reg) {
  if (!reg?.sessionId || !reg?.cwd) return null
  return join(CLAUDE_DIR, 'projects', reg.cwd.replace(/[^A-Za-z0-9]/g, '-'), `${reg.sessionId}.jsonl`)
}

function fileSize(path) {
  try {
    return statSync(path).size
  } catch {
    return 0
  }
}

// Last assistant text written to the transcript after `offset`, '' when none.
function lastAnswer(path, offset) {
  let fd
  try {
    const size = statSync(path).size
    if (size <= offset) return ''
    fd = openSync(path, 'r')
    const buf = Buffer.alloc(size - offset)
    readSync(fd, buf, 0, buf.length, offset)
    let last = ''
    for (const line of buf.toString('utf8').split('\n')) {
      if (!line.includes('"assistant"')) continue
      try {
        const v = JSON.parse(line)
        if (v.type !== 'assistant' || v.isSidechain || !Array.isArray(v.message?.content)) continue
        const text = v.message.content
          .filter((c) => c.type === 'text')
          .map((c) => c.text)
          .join('\n')
          .trim()
        if (text) last = text
      } catch {
        // half-written line
      }
    }
    return last
  } catch {
    return ''
  } finally {
    if (fd !== undefined) closeSync(fd)
  }
}

// Complete transcript lines between `offset` and the end of the file.
// `next` = where to continue: a half-written last line is left for the next read.
function readLines(path, offset) {
  let fd
  try {
    const size = statSync(path).size
    if (size <= offset) return { lines: [], next: offset }
    fd = openSync(path, 'r')
    const buf = Buffer.alloc(size - offset)
    readSync(fd, buf, 0, buf.length, offset)
    const end = buf.lastIndexOf(0x0a) + 1
    return { lines: buf.subarray(0, end).toString('utf8').split('\n'), next: offset + end }
  } catch {
    return { lines: [], next: offset }
  } finally {
    if (fd !== undefined) closeSync(fd)
  }
}

// What the agent does: its last tool call in `lines`, '' when none. E.g. "Edit notes/tea.md".
function lastActivity(lines) {
  let last = ''
  for (const line of lines) {
    if (!line.includes('"tool_use"')) continue
    try {
      const v = JSON.parse(line)
      if (v.type !== 'assistant' || v.isSidechain || !Array.isArray(v.message?.content)) continue
      for (const c of v.message.content) {
        if (c.type !== 'tool_use' || c.name === 'mcp__phone__say') continue
        const i = c.input || {}
        const arg = i.file_path ?? i.command ?? i.pattern ?? i.description ?? i.url ?? i.query ?? ''
        last = `${c.name} ${String(arg).replace(/\s+/g, ' ')}`.trim().slice(0, MAX_ACTIVITY)
      }
    } catch {
      // not a transcript line we understand
    }
  }
  return last
}

// Report what the running turn does, when it changed. At most once per ACTIVITY_MS.
function reportActivity(t) {
  if (!t.transcript || !t.tasks.length || Date.now() - t.activityAt < ACTIVITY_MS) return
  t.activityAt = Date.now()
  const { lines, next } = readLines(t.transcript, t.seen)
  t.seen = next
  const now = lastActivity(lines)
  if (!now || now === t.activity) return
  t.activity = now
  event('activity', now, t.tasks)
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

// The turn is over. No `say` in it: the user heard nothing, so speak what the
// agent wrote to the terminal instead (small models skip the tool).
async function finishTurn(t) {
  if (t.said) return event('stop', 'answered', t.tasks)
  let text = ''
  // The transcript line can land a moment after the status flips to idle.
  for (let i = 0; i < 5 && !text && t.transcript; i++) {
    if (i) await sleep(400)
    text = lastAnswer(t.transcript, t.offset)
  }
  if (text.length > MAX_SPOKEN) text = `${text.slice(0, MAX_SPOKEN)} ... (cut, the rest is in the session)`
  const r = await say(text || `The agent for project ${PROJECT} finished without an answer.`, text ? t.tasks : undefined)
  log('turn ended without say:', r.msg, JSON.stringify(text.slice(0, 80)))
  return event('stop', text ? 'answered from the terminal text' : 'no answer', t.tasks)
}

async function pump() {
  const reg = registry()
  const status = reg?.status ?? null
  if (turn) {
    // A turn shorter than one poll never shows as busy; its answer in the transcript does.
    if (status === 'busy' || (!turn.sawBusy && turn.transcript && lastAnswer(turn.transcript, turn.offset))) turn.sawBusy = true
    if (status === 'waiting') {
      // Stuck on a prompt in the terminal. Nobody sees it, so say it, once.
      if (!turn.waiting) {
        turn.waiting = true
        const what = reg.waitingFor || 'input'
        event('waiting', what, turn.tasks)
        say(`The agent for project ${PROJECT} is stuck waiting on: ${what}. Answer it in the Claude app.`)
      }
    } else if (!HOLD.has(status) && (turn.sawBusy || Date.now() - turn.emittedAt > TURN_START_GRACE_MS)) {
      const done = turn
      turn = null
      await finishTurn(done)
    } else if (status === 'busy') {
      if (turn.waiting) event('resume', '', turn.tasks)
      turn.waiting = false
      reportActivity(turn)
    }
  }
  // Hold only while a turn runs or a terminal prompt blocks. Anything else delivers:
  // 'shell' = idle with a background command (may never end), null = registry unreadable.
  if (!turn && queue.length && !HOLD.has(status)) {
    // everything held so far goes in as one message, one turn
    const held = queue.splice(0)
    const text = held.map((h) => h.text).join('\n\n')
    const transcript = transcriptPath(reg)
    const offset = transcript ? fileSize(transcript) : 0
    const tasks = held.map((h) => h.task).filter(Boolean)
    turn = { emittedAt: Date.now(), sawBusy: false, said: false, waiting: false, transcript, offset, tasks, seen: offset, activity: '', activityAt: 0 }
    try {
      await mcp.notification({
        method: 'notifications/claude/channel',
        params: { content: text, meta: { project: PROJECT } },
      })
      log('emitted', JSON.stringify(text.slice(0, 80)))
      event('start', '', tasks)
    } catch (e) {
      log('emit failed', e?.message)
    }
  }
  if ((turn || queue.length) && !timer) {
    timer = setTimeout(() => {
      timer = null
      pump()
    }, POLL_MS)
  }
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = []
    let size = 0
    req.on('data', (c) => {
      size += c.length
      if (size > MAX_BODY) {
        reject(new Error('too large'))
        req.destroy()
        return
      }
      chunks.push(c)
    })
    req.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')))
    req.on('error', reject)
  })
}

const send = (res, code, text) => {
  res.writeHead(code, { 'content-type': 'text/plain; charset=utf-8' })
  res.end(text + '\n')
}

const httpServer = http.createServer(async (req, res) => {
  const path = (req.url || '').split('?')[0]
  try {
    if (req.method === 'GET' && path === '/health') {
      // 200 only once Claude Code finished the MCP handshake: before that a prompt would be lost.
      if (!initialized) return send(res, 503, 'starting')
      res.writeHead(200, { 'content-type': 'application/json' })
      return res.end(
        JSON.stringify({ ok: true, project: PROJECT, pid: process.pid, queued: queue.length, claude_status: claudeStatus() }) + '\n',
      )
    }
    if (req.method === 'POST' && path === '/prompt') {
      const text = (await readBody(req)).trim()
      if (!text) return send(res, 400, 'empty prompt')
      if (!initialized) return send(res, 503, 'session not ready')
      queue.push({ text, task: String(req.headers['x-task-id'] || '') })
      const status = claudeStatus()
      const held = !!turn || HOLD.has(status)
      log('prompt', held ? 'queued' : 'now', JSON.stringify(text.slice(0, 80)))
      pump()
      return send(res, 200, held ? 'queued' : 'ok')
    }
    send(res, 404, 'not found')
  } catch (e) {
    log('http error', e?.message)
    if (!res.headersSent) send(res, 500, `error: ${e?.message || e}`)
  }
})

httpServer.on('error', (e) => {
  // EADDRINUSE etc. Die loudly: Claude Code shows the server as failed in /mcp.
  log(`http listen failed on 127.0.0.1:${PORT}: ${e.code || e.message}`)
  process.exit(1)
})

let closing = false
const shutdown = async () => {
  if (closing) return
  closing = true
  httpServer.close()
  // Session gone with work open: log it for the phone. Not spoken here: the server's
  // lost-session check fails the tasks and speaks, once.
  const open = queue.length + (turn && !turn.said ? 1 : 0)
  if (open) {
    const msg = `The agent session for project ${PROJECT} ended before it answered.`
    log(msg, `open prompts: ${open}`)
    await event('error', msg)
  }
  process.exit(0)
}
// Claude Code gone -> stdin closes -> free the port.
process.stdin.on('end', shutdown)
process.stdin.on('close', shutdown)
process.on('SIGTERM', shutdown)
process.on('SIGINT', shutdown)

await mcp.connect(new StdioServerTransport())
httpServer.listen(PORT, '127.0.0.1', () => log(`http on 127.0.0.1:${PORT} project=${PROJECT} server=${SERVER_URL}`))
