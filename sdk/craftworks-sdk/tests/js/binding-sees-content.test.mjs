// craftworks-sdk#129: A BINDING SHOWS WHAT IS STORED, WHATEVER THE CLOCK SAID.
//
// Both JS backends decided "are these the same rows?" from `id`, `updated` and `state`. `Db::update`
// stamps `now_ms().max(previous_updated)`, so two edits in one millisecond — or any edit made after
// the clock went BACKWARDS — legitimately keep `updated` where it was. The record was written and the
// binding never found out: `reload()` answered false, no listener fired, the screen kept the old text.
//
// Fourth instance of one shape (#96 state, #114 status, #114's `why`): a change invisible to a
// comparison over a SUBSET of what a person can see. So the last section drives ONE table of
// before/after pairs through BOTH backends and asserts the verdicts agree — a fix on one surface
// alone is sdk#87 again.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { wrap } from "../../js/wrap.js";

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${String(e.message).split("\n")[0]}\n`); }
};
const settle = () => new Promise(r => setTimeout(r, 10));

// ---------- 1. the in-tab database, REAL wasm, and a clock that does not help ----------
const real = wrap(createRequire(import.meta.url)("../../pkg/node/craftworks_sdk.js"));
const withClock = async (clock, fn) => {
  const was = Date.now;
  Date.now = clock;
  try { return await fn(); } finally { Date.now = was; }
};
const scenario = async (nowAtPut, nowAtUpdate) => {
  let now = nowAtPut;
  return withClock(() => now, async () => {
    const db = new real.Db();
    await db.define("tasks", { type: "Task", fields: [{ name: "title", kind: "text" }] });
    const row = await db.put("tasks", { title: "before" });
    const b = db.bind("tasks");
    await b.reload();
    let signals = 0; b.subscribe(() => { signals += 1; });
    now = nowAtUpdate;
    const after = await db.update("tasks", row.id, { title: "after" });
    const changed = await b.reload();
    return { changed, signals, shown: b.getSnapshot()[0].fields.title, sameStamp: after.updated === row.updated };
  });
};

await t("**FROZEN CLOCK: an edit in the same millisecond reaches the binding**", async () => {
  const r = await scenario(123456, 123456);
  assert.equal(r.sameStamp, true, "the precondition did not hold: `updated` moved, so this is not the case under test");
  assert.deepEqual({ shown: r.shown, changed: r.changed, signals: r.signals }, { shown: "after", changed: true, signals: 1 });
});
await t("**CLOCK ROLLBACK: an edit made after the clock went backwards reaches the binding**", async () => {
  const r = await scenario(200000, 100000);
  assert.equal(r.sameStamp, true, "the precondition did not hold: `updated` moved under a rollback");
  assert.deepEqual({ shown: r.shown, changed: r.changed, signals: r.signals }, { shown: "after", changed: true, signals: 1 });
});
await t("CONTROL a moving clock already worked", async () => {
  const r = await scenario(100000, 200000);
  assert.equal(r.sameStamp, false);
  assert.deepEqual({ shown: r.shown, changed: r.changed, signals: r.signals }, { shown: "after", changed: true, signals: 1 });
});
await t("CONTROL nothing written: reload answers false, nobody is told, and the snapshot is THE SAME ARRAY", async () => {
  await withClock(() => 123456, async () => {
    const db = new real.Db();
    await db.define("tasks", { type: "Task", fields: [{ name: "title", kind: "text" }] });
    await db.put("tasks", { title: "only" });
    const b = db.bind("tasks"); await b.reload();
    const before = b.getSnapshot(); let signals = 0; b.subscribe(() => { signals += 1; });
    assert.equal(await b.reload(), false);
    assert.equal(signals, 0);
    assert.equal(b.getSnapshot(), before, "an unchanged reload replaced the snapshot: every screen re-renders on every reload");
  });
});

// ---------- 2. BOTH backends, one table ----------
// Each backend is fed row lists through a fake of the wasm surface, so the ONLY thing under test is
// the comparison each one makes.
function fakeRaw() {
  let rows = [];
  const scan = () => JSON.stringify(rows);
  const session = {
    url: () => "ws://127.0.0.1:17509/", outbound: () => [], sent() {}, reconnected() {}, provision() {}, set_app() {}, adopt_loader() {},
    take_progress: () => "[]", take_loads: () => "[]", provisioned: () => true, refused: () => "",
    exhausted: () => false, unusable: () => "[]",
    tick: () => JSON.stringify({ rolledBack: 0, stalled: null, loadsInFlight: 0 }), tick_ms: () => 7777, unsaved_writes: () => 0, cold_due_ms: () => -1, cold_tick() {},
    flush() {}, bind() {}, unbind() {}, rendered() {}, scan, root: () => "root",
    live_mode: () => JSON.stringify({ mode: "HeadSubscribed", why: "", foreignNotifications: 0 }),
    take_stale: () => "[]",
  };
  return {
    version: () => "0", buildInfo: () => "{}", blockId: () => "", parseBlockId: () => "{}",
    Db: function () { return { define() {}, domains: () => "[]", scan, root: () => "root" }; },
    Session: function () { return session; },
    set: next => { rows = next; },
  };
}
const backends = {
  "in-tab (wrap.js)": async raw => new (wrap(raw).Db)(),
  "engine (engine-db.js)": async raw => (await wrap(raw).open({ app: "test-app",
    port: 17509, fetch: async () => ({ ok: true, arrayBuffer: async () => new ArrayBuffer(4) }),
    connect: () => ({ close() {}, pump() {} }), setInterval: () => 1, clearInterval: () => {},
  })).db,
};
const row = (fields, over = {}) => ({ id: "a".repeat(32), created: 1, updated: 5, state: "clean", fields, ...over });
// [name, before, after, must the binding change?]
const TABLE = [
  ["a field's value differs, id/updated/state equal", [row({ title: "before" })], [row({ title: "after" })], true],
  ["a field APPEARS", [row({ title: "x" })], [row({ title: "x", done: true })], true],
  ["a field DISAPPEARS", [row({ title: "x", done: true })], [row({ title: "x" })], true],
  ["false becomes null-ish absent vs present-false", [row({ done: false })], [row({})], true],
  ["a NESTED value differs", [row({ tags: ["a", "b"], at: { x: 1 } })], [row({ tags: ["a", "b"], at: { x: 2 } })], true],
  ["a list is reordered", [row({ tags: ["a", "b"] })], [row({ tags: ["b", "a"] })], true],
  ["1 vs '1': a number is not its text", [row({ n: 1 })], [row({ n: "1" })], true],
  ["only the SECOND row's content differs", [row({ t: 1 }), row({ t: 2 }, { id: "b".repeat(32) })], [row({ t: 1 }), row({ t: 3 }, { id: "b".repeat(32) })], true],
  ["`created` differs", [row({ t: 1 })], [row({ t: 1 }, { created: 2 })], true],
  ["`state` differs (sdk#96, kept)", [row({ t: 1 }, { state: "pending" })], [row({ t: 1 })], true],
  ["`updated` differs (kept)", [row({ t: 1 })], [row({ t: 1 }, { updated: 6 })], true],
  ["CONTROL equal content in fresh objects", [row({ title: "x", tags: ["a"], at: { x: 1 } })], [row({ title: "x", tags: ["a"], at: { x: 1 } })], false],
  ["CONTROL equal content, fields in another ORDER", [row({ a: 1, b: 2 })], [row({ b: 2, a: 1 })], false],
  ["CONTROL two equal rows", [row({ t: 1 }), row({ t: 2 }, { id: "b".repeat(32) })], [row({ t: 1 }), row({ t: 2 }, { id: "b".repeat(32) })], false],
];
const verdicts = {};
for (const [backend, make] of Object.entries(backends)) {
  verdicts[backend] = [];
  for (const [name, before, after, must] of TABLE) {
    await t(`${backend}: ${name} -> ${must ? "CHANGES" : "does not change"}`, async () => {
      const raw = fakeRaw(); raw.set(before);
      const db = await make(raw);
      const b = db.bind("tasks"); await b.reload(); await settle();
      assert.deepEqual(b.getSnapshot(), before, "the fixture did not load — nothing below would mean anything");
      const held = b.getSnapshot(); let signals = 0; b.subscribe(() => { signals += 1; });
      raw.set(after);
      const changed = await b.reload();
      verdicts[backend].push(changed);
      assert.equal(changed, must, `reload() answered ${changed}`);
      assert.equal(signals, must ? 1 : 0, `${signals} signals`);
      if (must) assert.deepEqual(b.getSnapshot(), after); else assert.equal(b.getSnapshot(), held, "the snapshot array was replaced");
    });
  }
}
await t("**PARITY: the two backends give the same verdict on every row of the table**", async () => {
  const [x, y] = Object.values(verdicts);
  assert.equal(x.length, TABLE.length); assert.equal(y.length, TABLE.length);
  assert.deepEqual(x, y);
});

if (failures) { process.stdout.write(`\n${failures} failed\n`); process.exit(1); }
