// READING SOMEBODY'S TREE (sdk#239): `handle.tree(registerId)` on the
// person's OWN session, through the real wasm `Session` and the real
// `openSession` — only the socket is injected.
//
// Data is a forest: one tree per identity. A session writes its own tree and
// reads any tree by address. Each tree is a reader Session (the same type and
// read path, writing switched off) on the ONE socket; the socket's frames are
// shared by ownership, bounded by memory and by F57's 500 subscriptions.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { openSession, MAX_OPEN_TREES, MAX_TREE_SUBSCRIPTIONS } from "../../js/session.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const sdk = createRequire(import.meta.url)("../../pkg/node/craftworks_sdk.js");
const { Session, wasm_memory_bytes } = sdk;

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.stack ?? JSON.stringify(e)}\n`); }
};
const enc = s => new TextEncoder().encode(s);
const hex = b => Buffer.from(b).toString("hex");
const HEAD_A = "a1".repeat(32), HEAD_B = "b2".repeat(32);

/** Decode frames with the stdlib; also the node's ack for an unrelated PUT. */
function stdlib(frames = []) {
  const r = spawnSync("cargo", ["run", "-q", "-p", "wire", "--example", "node_answers", "--",
    hex(enc("somebody's contract")), "00", "00", "x", ...frames.map(hex)], { cwd: root, encoding: "utf8", maxBuffer: 1 << 26 });
  assert.equal(r.status, 0, r.stderr);
  const o = JSON.parse(r.stdout);
  return { frames: o.frames, strangersGet: new Uint8Array(Buffer.from(o.got, "hex")) };
}

/** The person's own session, over an injected socket we can drive. */
async function own() {
  const sock = {};
  const h = await openSession(Session, {
    port: 7999,
    // Every real session is an app's (open() requires one); a write with no
    // app is refused for THAT before the tree's read-only is ever asked.
    app: "notes-app",
    artefacts: { delegate: "d", block: "b", register: "r" },
    fetch: async url => ({ ok: true, arrayBuffer: async () => enc(`${url} code`).buffer }),
    connect: (engine, { onEvent }) => { sock.engine = engine; sock.emit = onEvent; return { pump() {}, close() {} }; },
    setInterval: () => 0, clearInterval() {}, setTimeout: () => 0, clearTimeout() {},
    addEventListener: null, removeEventListener: null, documentOf: null,
  });
  // Whatever provisioning queued goes first, so what follows is the tree's.
  sock.engine.sent(sock.engine.outbound().length);
  return { h, sock };
}
const frames = sock => { const out = sock.engine.outbound(); sock.engine.sent(out.length); return stdlib(out).frames; };

await t("**tree(id, { seq }) opens the view with its PUBLISHED-HEAD FLOOR (sdk#349): it says it waits for that version**", async () => {
  const { h } = await own();
  const floored = await h.tree(HEAD_A, { seq: 7 });
  assert.equal(floored.waitingFor(), "waiting for the published version (seq 7)", "the seq did not reach the view's floor");
  // THE CONTROL: with no published seq there is no floor, and nothing is said.
  const plain = await h.tree(HEAD_B);
  assert.equal(plain.waitingFor(), "", "a view with no published seq says it waits");
  // What a publisher records as that seq: the ACKNOWLEDGED head's (page-io's
  // published_seq, tested in wire_node), 0 before anything is published.
  assert.equal(h.headSeq(), 0, "a session that has published nothing names a published seq");
});

await t("**tree() is a read-only Db on the SAME session: every write refused, the head it names**", async () => {
  const { h } = await own();
  const tr = await h.tree(HEAD_A);
  assert.equal(tr.headId(), HEAD_A);
  for (const write of [() => tr.db.put("notes", { text: "x" }), () => tr.db.delete("notes", "00".repeat(24))]) {
    // BOUNDED: a write that is not refused is queued and never settles, and
    // that must fail here by name, not hang the file.
    const bounded = () => Promise.race([write(), new Promise((_, no) => setTimeout(() => no(new Error("the write was not refused: it is waiting on the node")), 2000))]);
    await assert.rejects(bounded, e => e.code === "REFUSED" && /^read-only: /.test(e.message));
  }
  assert.equal(h.canWrite().answer, "yes", "THE CONTROL: the person's own session still writes");
  assert.equal(h.canWrite(HEAD_A).answer !== "yes", true, "the own session may write somebody else's head");
});

await t("**a tree installs nothing on the node: its frames on the shared socket are GETs, its head watched**", async () => {
  const { h, sock } = await own();
  await h.tree(HEAD_A);
  const f = frames(sock);
  assert.ok(f.length > 0, "THE CONTROL: the tree sent nothing, so 'only GETs' would be vacuous");
  assert.deepEqual([...new Set(f.map(x => x.op))], ["get"], JSON.stringify(f));
  assert.ok(f.some(x => x.subscribe), "the tree's head was not watched");
});

await t("**N trees, one socket: each opens its own head reader, and a frame nobody asked for is counted ONCE**", async () => {
  const { h, sock } = await own();
  await h.tree(HEAD_A);
  await h.tree(HEAD_B);
  const heads = frames(sock).filter(x => x.subscribe).map(x => x.key);
  assert.equal(new Set(heads).size, 2, `two trees, ${heads.length} head reads: ${heads}`);
  const before = JSON.parse(h.session.live_mode()).foreignNotifications;
  sock.engine.on_inbound(stdlib().strangersGet);
  assert.equal(JSON.parse(h.session.live_mode()).foreignNotifications, before + 1, "an unowned frame was not counted exactly once");
});

await t(`**bounded: at most ${MAX_OPEN_TREES} open trees, and ${MAX_TREE_SUBSCRIPTIONS} tree subscriptions per socket (F57) — refused by name, never dropped**`, async () => {
  const { h, sock } = await own();
  const open = [];
  for (let i = 0; i < MAX_OPEN_TREES; i += 1) open.push(await h.tree(i.toString(16).padStart(64, "0")));
  await assert.rejects(() => h.tree(HEAD_A), /trees are already open/);
  open.pop().close();
  const again = await h.tree(HEAD_A); // a slot freed
  again.close();
  for (const tr of open) tr.close();
  // Closed trees keep their subscription (no unsubscribe) until a reconnect.
  let opened = MAX_OPEN_TREES + 1, refused = null;
  while (!refused) {
    try { (await h.tree(HEAD_B)).close(); opened += 1; } catch (e) { refused = e; }
    assert.ok(opened <= MAX_TREE_SUBSCRIPTIONS, "the subscription cap never fired");
  }
  assert.equal(opened, MAX_TREE_SUBSCRIPTIONS, `refused after ${opened}`);
  assert.match(refused.message, /500 per connection, F57/);
  sock.emit({ kind: "open" }); // a new socket: only the open trees' subscriptions remain
  (await h.tree(HEAD_B)).close();
});

await t("**provision: false — a reader opens the socket, installs NOTHING, and still reads trees**", async () => {
  const sock = {};
  const fetched = [];
  const h = await openSession(Session, {
    port: 7999,
    artefacts: { delegate: "d", block: "b", register: "r" },
    provision: false,
    fetch: async url => { fetched.push(url); return { ok: true, arrayBuffer: async () => enc(`${url} code`).buffer }; },
    connect: (engine, { onEvent }) => { sock.engine = engine; sock.emit = onEvent; return { pump() {}, close() {} }; },
    setInterval: () => 0, clearInterval() {}, setTimeout: () => 0, clearTimeout() {},
    addEventListener: null, removeEventListener: null, documentOf: null,
  });
  assert.deepEqual(fetched, [], "a reader fetched provisioning artefacts it will not install");
  await h.tree(HEAD_A);
  assert.deepEqual(fetched, ["b"], "tree() did not fetch the Block code, or fetched more");
  const ops = [...new Set(frames(sock).map(x => x.op))];
  assert.deepEqual(ops, ["get"], `a reader's socket carried ${ops}: something was installed`);
  assert.equal(h.provisioned(), false, "THE CONTROL: the reader's own tree was not provisioned");
});

await t("THE MEASUREMENT: one open tree reader's wasm memory, before any rows", async () => {
  // Many readers at once, so the growth is pages the allocator had to ADD,
  // not free space it already held (16 fit in that and read as 0 B).
  const n = 200;
  const before = wasm_memory_bytes();
  const readers = [];
  for (let i = 0; i < n; i += 1) {
    const s = new Session(7999);
    s.open_named(enc("block code"), (i + 1).toString(16).padStart(64, "0"), (i % 255) + 1, 0);
    readers.push(s);
  }
  const grew = wasm_memory_bytes() - before;
  process.stdout.write(`      memory: ${n} tree readers grew wasm memory by ${grew} B — ${Math.round(grew / n)} B per reader before any rows\n`);
  assert.ok(before > 0 && grew > 0, "THE CONTROL: the measurement saw the readers at all");
  for (const s of readers) s.free();
});

// A TREE'S RECORDING HAS ITS OWN READER (builder#160): a visitor's reads run on the tree's own Session, its own
// page, so the handle's `pageTrace()` (sdk#434) records the OTHER page. The tree handle reads its own.
await t("**a tree reads ITS OWN page's recording: `pageTrace()` on the tree is its Session's, not the handle's**", async () => {
  const { h, sock } = await own();
  const tr = await h.tree(HEAD_A);
  assert.equal(typeof tr.pageTrace, "function", "the tree has no pageTrace(): a visitor's reads have no reader");
  const handles = h.pageTrace(), mine = tr.pageTrace();
  assert.equal(typeof mine, "string");
  assert.ok(frames(sock).length > 0, "THE CONTROL: the tree sent nothing, so its recording could be empty");
  assert.notEqual(mine, handles, "the tree's pageTrace() is the handle's page's recording");
  // The tree's page reads its head (`recover-head`); the handle's page sent none: each dump is its own page's.
  assert.match(mine, /page::op::recover-head/, "the tree's recording does not name the head read its page sent");
  assert.doesNotMatch(handles, /page::op::recover-head/, "THE CONTROL: the handle's page also read that head, so the dumps are not told apart");
});

if (failures) { process.stdout.write(`${failures} failed\n`); process.exit(1); }
