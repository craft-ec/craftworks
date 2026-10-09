// WATCH A DEFINITION (§19 P3, `watchDefinition`) on the engine-backed surface, over a recording session.
//
// What the session decides (which watch key a head move changed) is tested natively (tests/definition_watch.rs);
// this is the JavaScript half: the watch is bound through the session's door (never by spelling the reserved name),
// a key `take_stale` names re-reads `definition(which)` and records `rendered_definition` IN THE SAME SYNCHRONOUS
// CALL as that read, every watcher gets the rows, unwatch unbinds, and a door that refuses is thrown by name.
import assert from "node:assert/strict";
import { engineDb } from "../../js/engine-db.js";

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.stack}\n`); }
};

/** A session with a draft, recording each call in order. `bind_definition` answers an opaque key. */
const recordingSession = () => {
  const s = {
    calls: [], stale: [], draft: [{ key: "meta", body: { name: "A" } }],
    bind_definition: w => { s.calls.push(["bind", w]); return `opaque-${w}`; },
    unbind_definition: w => s.calls.push(["unbind", w]),
    rendered_definition: w => s.calls.push(["rendered", w]),
    definition: w => { s.calls.push(["read", w]); return w === "draft" ? s.draft : []; },
    take_stale: () => { const out = s.stale; s.stale = []; return JSON.stringify(out); },
    take_loads: () => "[]", root: () => "r",
  };
  return s;
};
const settle = () => new Promise(r => setTimeout(r, 0));

await t("**a head move that changed the draft re-reads it, records rendered in the same call, and tells every watcher**", async () => {
  const s = recordingSession();
  const db = engineDb(s);
  const seen = [[], []];
  db.watchDefinition("draft", rows => seen[0].push(rows));
  db.watchDefinition("draft", rows => seen[1].push(rows));
  await settle();
  for (const w of seen) w.length = 0;          // each watcher's first read (tested on its own below)
  s.calls.length = 0;
  s.draft = [{ key: "meta", body: { name: "B, from the other device" } }];
  s.stale = ["opaque-draft"];
  db.drain();
  await settle();
  assert.deepEqual(s.calls, [["read", "draft"], ["rendered", "draft"]], "the read and its rendered were not one call, in order");
  for (const w of seen) assert.deepEqual(w, [[{ key: "meta", body: { name: "B, from the other device" } }]]);
});

await t("**a re-read that FAILS reaches every watcher as `cb(null, { error })`**, named; it is not rendered", async () => {
  const s = recordingSession();
  s.definition = w => { s.calls.push(["read", w]); throw Object.assign(new Error("the draft's block could not be loaded"), { code: "UNAVAILABLE" }); };
  const db = engineDb(s);
  const told = [];
  db.watchDefinition("draft", (rows, fail) => told.push([rows, fail?.error?.message]));
  db.watchDefinition("draft", (rows, fail) => told.push([rows, fail?.error?.message]));
  await settle();
  told.length = 0;                             // the first reads failed too; the head move's re-read is the subject
  s.stale = ["opaque-draft"];
  db.drain();
  await settle();
  assert.deepEqual(told, [[null, "the draft's block could not be loaded"], [null, "the draft's block could not be loaded"]], "a failed re-read was swallowed");
  assert.ok(!s.calls.some(c => c[0] === "rendered"), "a failed re-read was recorded as rendered: its change would never be named again");
});

await t("**each watcher gets the CURRENT rows once at the start**, with no head move, and rendered is recorded", async () => {
  const s = recordingSession();
  const db = engineDb(s);
  const first = [], second = [];
  db.watchDefinition("draft", rows => first.push(rows));
  await settle();
  assert.deepEqual(first, [[{ key: "meta", body: { name: "A" } }]], "no first read");
  assert.deepEqual(s.calls, [["bind", "draft"], ["read", "draft"], ["rendered", "draft"]]);
  db.watchDefinition("draft", rows => second.push(rows));
  await settle();
  assert.equal(first.length, 1, "a later watcher's first read told the earlier one again");
  assert.equal(second.length, 1);
});

await t("THE CONTROL: a head move that named another key re-reads nothing", async () => {
  const s = recordingSession();
  const db = engineDb(s);
  let told = 0;
  db.watchDefinition("draft", () => { told += 1; });
  await settle();
  told = 0; s.calls.length = 0;               // the first read, not the subject here
  s.stale = ["opaque-app", "tasks"];
  db.drain();
  await settle();
  assert.equal(told, 0);
  assert.ok(!s.calls.some(c => c[0] === "read"), "a definition was read on another key's move");
});

await t("**unwatch unbinds once the last watcher of that definition is gone**", async () => {
  const s = recordingSession();
  const db = engineDb(s);
  const a = db.watchDefinition("draft", () => {});
  const b = db.watchDefinition("draft", () => {});
  await settle();
  s.calls.length = 0;                          // the first reads, not the subject here
  a();
  assert.ok(!s.calls.some(c => c[0] === "unbind"), "unbound while a watcher remained");
  b();
  assert.deepEqual(s.calls.at(-1), ["unbind", "draft"]);
  s.calls.length = 0;
  s.stale = ["opaque-draft"];
  db.drain();
  await settle();
  assert.ok(!s.calls.some(c => c[0] === "read"), "an unwatched definition was re-read");
});

await t("**a door that cannot watch is thrown by name**, never a watch that silently binds nothing", async () => {
  const s = recordingSession();
  s.bind_definition = () => { throw new Error("the `draft` definition of this session's app cannot be watched"); };
  const db = engineDb(s);
  assert.throws(() => db.watchDefinition("draft", () => {}), /cannot be watched/);
});

await t("**engine: a write door handed an app is refused by name**, and nothing reaches the session", async () => {
  const s = recordingSession();
  let wrote = 0;
  s.draft_put = () => { wrote += 1; };
  const db = engineDb(s);
  await assert.rejects(() => db.draftPut("meta", {}, "someone-else"), e => e.code === "REFUSED" && /`draftPut` takes no app/.test(e.message));
  assert.equal(wrote, 0, "the refused write reached the session");
});

if (failures) { process.stdout.write(`\n${failures} failing\n`); process.exit(1); }
process.stdout.write("\nall passing\n");
