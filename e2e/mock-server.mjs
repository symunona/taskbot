// Mock of the app-facing surface of docs/protocol.md (HTTP + WebSocket). Node built-ins only.
//   node e2e/mock-server.mjs [port]      prints a connect string to paste into the app
// Extra, mock-only:
//   POST /mock/revoke                             forgets all device tokens (to exercise the 401 path)
//   POST /mock/down?ms=8000                       drops the sockets and answers 503 for that long (offline path)
//   POST /v1beta/models/<model>:generateContent   fake Gemini text endpoint. The app uses it when
//        localStorage `gemini_text_base_url` = this server's URL. Scripted: user text with "read" ->
//        functionCall read_file, with "rename" -> functionCall rename_thread, with "delegate" ->
//        functionCall delegate, else an echo.
//   threads.compact answers with a canned memory (everything but the last entry folded) and pushes `threads.memory`.
//   POST /mock/task  body = Task                  upserts it and pushes `tasks.changed` (any state by hand)
// The `delegate` tool plays a whole task: queued -> spawning -> sent -> working (+ activity) -> done + agent.say.
import http from 'node:http';
import crypto from 'node:crypto';

const port = Number(process.argv[2] || 55566);
const url = `http://127.0.0.1:${port}`;
const code = 'K7M2QX9P';
const tokens = new Set();
const sockets = new Set();
let codeUsed = false;
let downUntil = 0;

const tools = [
  { name: 'read_file', description: 'Read a vault file', parameters: { type: 'OBJECT', properties: { path: { type: 'STRING', description: 'Vault-relative path, no leading slash' } }, required: ['path'] } },
  { name: 'projects', description: 'List projects', parameters: { type: 'OBJECT', properties: {} } },
  { name: 'rename_thread', description: 'Name the current thread', parameters: { type: 'OBJECT', properties: { name: { type: 'STRING', description: 'Short topic, max 80 chars' } }, required: ['name'] } },
  { name: 'delegate', description: 'Send work to a project agent', parameters: { type: 'OBJECT', properties: { project: { type: 'STRING' }, prompt: { type: 'STRING' } }, required: ['project', 'prompt'] } },
];

// Threads: docs/protocol.md "Threads". One seeded, a day old, so the list shows "yesterday".
const threads = new Map(); // id -> { thread, entries: Map(entryId -> entry) }
const newThread = (name, at = Date.now()) => {
  const thread = { id: crypto.randomBytes(8).toString('hex'), name: name || 'New thread', created_at: at, last_accessed_at: at };
  threads.set(thread.id, { thread, entries: new Map() });
  return thread;
};
{
  const at = Date.now() - 26 * 3600 * 1000;
  const seeded = threads.get(newThread('Garden plan', at).id);
  seeded.entries.set('seed-1', { id: 'seed-1', ts: at, kind: 'user', text: 'What did I note about the garden?', voice: true });
  seeded.entries.set('seed-2', { id: 'seed-2', ts: at + 900, kind: 'tool', text: '', tool: { name: 'read_file', args: { path: 'garden.md' }, result: { content: 'Plant beans in May.' }, called_at: at + 900, result_at: at + 1400 } });
  seeded.entries.set('seed-3', { id: 'seed-3', ts: at + 2000, kind: 'model', text: 'Beans in May.', voice: true });
}
// Pins + project memory: docs/protocol.md "Pins", "Project memory". Thread pins live in t.pins.
const projects = [{ name: 'vault', dir: '/mock/vault' }, { name: 'taskbot', dir: '/mock/taskbot' }];
const always = [{ text: 'answer in short sentences', at: Date.now() }];
const projectFiles = new Map([['vault', '# Taskbot: vault\n\n## Pinned\n- notes go under inbox/\n\n## Tasks\n- 2026-10-12 add a note about tea -> done: added\n']]);
const pinsOf = (list, limit) => ({ pins: list, size: list.reduce((n, p) => n + p.text.length + 1, 0), limit });
const projectMemory = (project) => (projectFiles.has(project) ? { project_memory: { project, text: projectFiles.get(project), size: projectFiles.get(project).length, limit: 3000 } } : {});
const setProject = (id, project) => {
  const t = threads.get(id);
  if (!t) return { error: 'unknown thread' };
  if (project && !projects.some((p) => p.name === project)) return { error: `unknown project ${project}` };
  if (project) t.thread.project = project; else delete t.thread.project;
  broadcast('threads.changed', { thread: t.thread });
  return { thread: t.thread };
};
// Plain add, scopes `thread` and `always` only. No `replaces`, no limits.
const memoryNote = (args, thread_id) => {
  const t = threads.get(thread_id), pin = { text: String(args?.text ?? '').trim().slice(0, 300), at: Date.now() };
  if (args?.scope === 'always') {
    always.push(pin);
    broadcast('memory.always', pinsOf(always, 1200));
    return { scope: 'always', ...pinsOf(always, 1200) };
  }
  if (!t) return { error: 'no thread' };
  (t.pins ??= []).push(pin);
  broadcast('threads.memory', { thread_id, ...(t.memory ? { memory: t.memory } : {}), pins: pinsOf(t.pins, 1500), usage: t.usage ?? { input: 0, output: 0 } });
  return { scope: 'thread', ...pinsOf(t.pins, 1500) };
};
const cleanName = (name) => String(name ?? '').replace(/[\u0000-\u001f]/g, '').trim().slice(0, 80);
const rename = (id, name) => {
  const t = threads.get(id), clean = cleanName(name);
  if (!t) return { error: 'unknown thread' };
  if (!clean) return { error: 'empty name' };
  t.thread.name = clean;
  broadcast('threads.changed', { thread: t.thread });
  return { thread: t.thread };
};
// Tasks: docs/protocol.md "Tasks".
const tasks = new Map(); // id -> Task
const putTask = (id, change) => {
  const task = { ...tasks.get(id), ...change, updated_at: Date.now() };
  if (task.state !== 'working') delete task.last_activity;
  tasks.set(id, task);
  broadcast('tasks.changed', { task });
  return task;
};
const delegate = (args, thread_id) => {
  const id = crypto.randomBytes(8).toString('hex');
  const task = putTask(id, { id, project: args?.project || 'vault', ...(thread_id ? { thread_id } : {}), prompt: args?.prompt || '', state: 'queued', queue_pos: 0, created_at: Date.now() });
  const answer = `Mock agent finished: ${task.prompt}`;
  const steps = [
    [300, () => putTask(id, { state: 'spawning' })],
    [1200, () => putTask(id, { state: 'sent' })],
    [2000, () => putTask(id, { state: 'working', queue_pos: undefined, started_at: Date.now() })],
    [3500, () => putTask(id, { last_activity: 'Read inbox/today.md' })],
    [5500, () => putTask(id, { last_activity: 'Edit notes/mock.md' })],
    [8000, () => { putTask(id, { answer }); broadcast('agent.say', { project: task.project, text: answer }); }],
    [8200, () => putTask(id, { state: 'done' })],
  ];
  steps.forEach(([ms, step]) => setTimeout(step, ms));
  return { status: 'sent', project: task.project, task_id: id, state: 'queued', queue_pos: 0 };
};
const sessions = [
  { project: 'vault', tmux: 'tb-vault', status: 'idle', claude_session_id: 'abc', url: 'https://claude.ai/code/session_mock' },
  { project: 'taskbot', tmux: 'tb-taskbot', status: 'busy', claude_session_id: null, url: null },
];

const cors = { 'Access-Control-Allow-Origin': '*', 'Access-Control-Allow-Headers': 'Authorization, Content-Type, x-goog-api-key', 'Access-Control-Allow-Methods': 'GET, POST, DELETE, OPTIONS' };
const send = (res, status, body) => { res.writeHead(status, { ...cors, 'Content-Type': 'application/json' }); res.end(body === undefined ? '' : JSON.stringify(body)); };
const bearerOk = (req) => tokens.has((req.headers.authorization || '').replace(/^Bearer /, ''));
const readBody = (req) => new Promise((resolve) => { let b = ''; req.on('data', (c) => (b += c)); req.on('end', () => { try { resolve(JSON.parse(b || '{}')); } catch { resolve({}); } }); });

// Minimal RFC 6455 text framing: enough for JSON envelopes.
const frame = (text) => {
  const data = Buffer.from(text);
  let head;
  if (data.length < 126) head = Buffer.from([0x81, data.length]);
  else if (data.length < 65536) head = Buffer.from([0x81, 126, data.length >> 8, data.length & 255]);
  else { head = Buffer.alloc(10); head[0] = 0x81; head[1] = 127; head.writeBigUInt64BE(BigInt(data.length), 2); }
  return Buffer.concat([head, data]);
};
const wsSend = (sock, event, payload, refId) => sock.write(frame(JSON.stringify({ event, id: crypto.randomUUID(), ...(refId ? { ref_id: refId } : {}), ts: Date.now(), payload })));
const broadcast = (event, payload) => sockets.forEach((s) => s.authed && wsSend(s, event, payload));

function onMessage(sock, msg) {
  if (!sock.authed) {
    if (msg.event === 'auth' && tokens.has(msg.payload?.token)) { sock.authed = true; return wsSend(sock, 'auth.ok', { device_id: 'dev-1' }); }
    wsSend(sock, 'auth.error', { reason: 'invalid_token' });
    return sock.end();
  }
  console.log('ws <-', msg.event, JSON.stringify(msg.payload));
  if (msg.event === 'tools.list') return wsSend(sock, 'tools.list.result', { tools }, msg.id);
  if (msg.event === 'sessions.list') return wsSend(sock, 'sessions.list.result', { sessions }, msg.id);
  if (msg.event === 'tasks.list') return wsSend(sock, 'tasks.list.result', { tasks: [...tasks.values()].reverse() }, msg.id);
  if (msg.event === 'tool.call') {
    const { name, args, thread_id } = msg.payload;
    let result = { status: 'sent', echo: msg.payload };
    if (name === 'read_file') result = { content: `mock content of ${args?.path}` };
    if (name === 'rename_thread') result = thread_id ? rename(thread_id, args?.name) : { error: 'no thread' };
    if (name === 'delegate') result = delegate(args, thread_id);
    if (name === 'tasks') result = { tasks: [...tasks.values()].reverse() };
    if (name === 'projects') result = { projects };
    if (name === 'set_project') result = thread_id ? setProject(thread_id, args?.project) : { error: 'no thread' };
    if (name === 'memory_note') result = memoryNote(args, thread_id);
    // Slow enough to see the "running" state of the chat entry.
    return setTimeout(() => wsSend(sock, 'tool.result', { result }, msg.id), 1500);
  }
  if (msg.event === 'threads.list') {
    const list = [...threads.values()].map((t) => t.thread).sort((a, b) => b.last_accessed_at - a.last_accessed_at);
    return wsSend(sock, 'threads.list.result', { threads: list }, msg.id);
  }
  if (msg.event === 'threads.create') {
    const thread = newThread(cleanName(msg.payload?.name));
    broadcast('threads.changed', { thread });
    return wsSend(sock, 'threads.create.result', { thread, always: pinsOf(always, 1200) }, msg.id);
  }
  if (msg.event === 'threads.set_project') {
    const r = setProject(msg.payload?.id, msg.payload?.project);
    return r.error ? wsSend(sock, 'error', r, msg.id) : wsSend(sock, 'threads.set_project.result', r, msg.id);
  }
  if (msg.event === 'threads.rename') {
    const r = rename(msg.payload?.id, msg.payload?.name);
    return r.error ? wsSend(sock, 'error', r, msg.id) : wsSend(sock, 'threads.rename.result', r, msg.id);
  }
  if (['threads.get', 'threads.append', 'threads.compact', 'threads.usage'].includes(msg.event)) {
    const t = threads.get(msg.payload?.id);
    if (!t) return wsSend(sock, 'error', { error: 'unknown thread' }, msg.id);
    t.usage ??= { input: 0, output: 0 };
    const state = () => ({ ...(t.memory ? { memory: t.memory } : {}), usage: t.usage, pins: pinsOf(t.pins ?? [], 1500) });
    if (msg.event === 'threads.usage') {
      t.usage = { input: t.usage.input + msg.payload.input, output: t.usage.output + msg.payload.output };
      return wsSend(sock, 'threads.usage.result', { usage: t.usage }, msg.id);
    }
    if (msg.event === 'threads.compact') {
      // Canned memory: folds everything but the last entry.
      const ids = [...t.entries.keys()];
      if (ids.length < 2) return wsSend(sock, 'threads.compact.result', state(), msg.id);
      t.memory = { text: '## State\n- mock memory of ' + (ids.length - 1) + ' entries\n- open: nothing\n## Recent\n- asked things -> got answers', upto_id: ids[ids.length - 2], at: Date.now() };
      t.usage = { input: t.usage.input + 900, output: t.usage.output + 60 };
      broadcast('threads.memory', { thread_id: t.thread.id, ...state() });
      return wsSend(sock, 'threads.compact.result', state(), msg.id);
    }
    t.thread.last_accessed_at = Date.now();
    if (msg.event === 'threads.get') return wsSend(sock, 'threads.get.result', { thread: t.thread, entries: [...t.entries.values()], ...state(), always: pinsOf(always, 1200), ...projectMemory(t.thread.project) }, msg.id);
    const entries = msg.payload.entries || [];
    if (entries.length > 200) return wsSend(sock, 'error', { error: 'too many entries' }, msg.id);
    entries.forEach((e) => t.entries.set(e.id, e)); // Map keeps the first-insert slot on replace
    return wsSend(sock, 'threads.append.result', { count: entries.length }, msg.id);
  }
  wsSend(sock, 'error', { error: `unknown event ${msg.event}` }, msg.id);
}

const server = http.createServer(async (req, res) => {
  const route = `${req.method} ${req.url}`;
  if (req.method === 'OPTIONS') return send(res, 204);
  if (req.url.startsWith('/mock/down')) {
    downUntil = Date.now() + Number(new URL(req.url, url).searchParams.get('ms') || 8000);
    sockets.forEach((s) => s.end());
    return send(res, 200, { ok: true });
  }
  if (Date.now() < downUntil) return send(res, 503, { error: 'mock is down' });
  if (route === 'GET /api/v1/health') return send(res, 200, { app: 'taltosbot', status: 'ok', version: 'mock' });
  if (route === 'POST /api/v1/pair') {
    const body = await readBody(req);
    console.log('pair <-', JSON.stringify(body));
    if (body.code !== code || codeUsed) return send(res, 401, { error: 'invalid or expired code' });
    codeUsed = true;
    const token = crypto.randomBytes(32).toString('hex');
    tokens.add(token);
    return send(res, 200, { device_token: token, device_id: 'dev-1', server_urls: [url] });
  }
  if (route === 'GET /api/v1/me') return bearerOk(req) ? send(res, 200, { device_id: 'dev-1', name: 'mock', server_version: 'mock' }) : send(res, 401, { error: 'unauthorized' });
  if (route === 'GET /api/v1/config') return bearerOk(req) ? send(res, 200, { gemini_api_key: 'mock-key', gemini_model: 'mock-live-model', gemini_text_model: 'mock-text-model' }) : send(res, 401, { error: 'unauthorized' });
  if (route === 'POST /internal/agent/say') { broadcast('agent.say', await readBody(req)); return send(res, 200, { ok: true }); }
  if (route === 'POST /internal/agent/event') { broadcast('agent.event', await readBody(req)); return send(res, 200, { ok: true }); }
  if (req.method === 'POST' && /^\/v1beta\/models\/[^/]+:generateContent$/.test(req.url)) {
    if (req.headers['x-goog-api-key'] !== 'mock-key') return send(res, 403, { error: { code: 403, message: 'bad or missing x-goog-api-key' } });
    const body = await readBody(req);
    const contents = body.contents || [];
    const last = contents[contents.length - 1] || {};
    const fnResponse = (last.parts || []).find((p) => p.functionResponse)?.functionResponse;
    const userText = (last.parts || []).map((p) => p.text || '').join(' ');
    console.log('gemini <-', req.url, `contents=${contents.length}`, `system=${(body.systemInstruction?.parts?.[0]?.text || '').length}ch`,
      `tools=${body.tools?.[0]?.functionDeclarations?.length ?? 0}`, 'roles=' + contents.map((c) => c.role).join(','));
    let parts;
    if (fnResponse) {
      // The model turn before it must come back untouched, thoughtSignature included.
      const echoed = (contents[contents.length - 2]?.parts || []).some((p) => p.thoughtSignature === 'mock-signature');
      console.log('gemini    functionResponse', JSON.stringify(fnResponse), 'thoughtSignature echoed:', echoed);
      parts = [{ text: `Tool ${fnResponse.name} said: ${JSON.stringify(fnResponse.response)}` + (echoed ? '' : ' (thoughtSignature LOST)') }];
    } else if (/rename/i.test(userText)) {
      parts = [{ functionCall: { name: 'rename_thread', args: { name: 'Mock topic' } }, thoughtSignature: 'mock-signature' }];
    } else if (/delegate/i.test(userText)) {
      parts = [{ functionCall: { name: 'delegate', args: { project: 'vault', prompt: 'Add a note about tea.' } }, thoughtSignature: 'mock-signature' }];
    } else if (/read/i.test(userText)) {
      parts = [{ functionCall: { name: 'read_file', args: { path: 'inbox/today.md' } }, thoughtSignature: 'mock-signature' }];
    } else {
      parts = [{ text: `Mock answer to: ${userText}` }];
    }
    return send(res, 200, { candidates: [{ content: { role: 'model', parts }, finishReason: 'STOP' }], usageMetadata: { promptTokenCount: 1234, totalTokenCount: 1300 } });
  }
  if (route === 'POST /mock/task') {
    const body = await readBody(req);
    if (!body.id) return send(res, 400, { error: 'task needs an id' });
    return send(res, 200, { task: putTask(body.id, { created_at: Date.now(), ...body }) });
  }
  if (route === 'POST /mock/revoke') { tokens.clear(); sockets.forEach((s) => s.end()); return send(res, 200, { ok: true }); }
  send(res, 404, { error: 'not found' });
});

server.on('upgrade', (req, sock) => {
  if (req.url !== '/api/v1/ws' || Date.now() < downUntil) return sock.destroy();
  const accept = crypto.createHash('sha1').update(req.headers['sec-websocket-key'] + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
  sock.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
  sockets.add(sock);
  let buf = Buffer.alloc(0);
  sock.on('data', (chunk) => {
    buf = Buffer.concat([buf, chunk]);
    while (buf.length >= 2) {
      const opcode = buf[0] & 15;
      let len = buf[1] & 127, off = 2;
      if (len === 126) { if (buf.length < 4) return; len = buf.readUInt16BE(2); off = 4; }
      else if (len === 127) { if (buf.length < 10) return; len = Number(buf.readBigUInt64BE(2)); off = 10; }
      if (buf.length < off + 4 + len) return;
      const mask = buf.subarray(off, off + 4);
      const data = Buffer.from(buf.subarray(off + 4, off + 4 + len)).map((b, i) => b ^ mask[i % 4]);
      buf = buf.subarray(off + 4 + len);
      if (opcode === 8) return sock.end();
      if (opcode === 9) sock.write(Buffer.concat([Buffer.from([0x8a, data.length]), data]));
      if (opcode === 1) { try { onMessage(sock, JSON.parse(data.toString())); } catch (e) { console.log('bad frame', e.message); } }
    }
  });
  const drop = () => sockets.delete(sock);
  sock.on('close', drop);
  sock.on('error', drop);
});

server.listen(port, '127.0.0.1', () => {
  console.log(`mock taltosbot server on ${url}`);
  console.log('connect string:', Buffer.from(JSON.stringify({ v: 1, code, urls: [url] })).toString('base64'));
});
