// An id a read cannot address answers `null`, through the surface an app holds.
//
// This is where the defect was actually felt: a stale `lastOpened` in browser
// storage, pointing at a project deleted on another device, would have failed
// the builder ON LOAD rather than showing an empty canvas — so the builder
// wrapped the call in a `try/catch`. Every caller holding an outside id needed
// that wrapper, and the one that forgets is the one that crashes
// (craftworks-sdk#118).
//
// The Rust tests prove `Db::get`. They do not prove the wasm boundary, which
// is where the parse happens and where the throw came from.
import assert from "node:assert";
import { createRequire } from "node:module";
import { wrap } from "../../js/wrap.js";
const sdk = wrap(createRequire(import.meta.url)("../../pkg/node/craftworks_sdk.js"));

const t = async (name, fn) => { await fn(); process.stdout.write(`  ok  ${name}\n`); };

const db = new sdk.Db();
await db.define("tasks", { type: "Task", fields: [{ name: "title", kind: "text", required: true }] });
await db.define("component", {
  type: "Component",
  parent: "pid",
  fields: [{ name: "pid", kind: "text", required: true }],
});
const real = await db.put("tasks", { title: "here" });

// Everything that arrives from outside and is not a usable id here.
//
// NOTE: the issue listed a 64-character id among these. Since #122 that is a
// VALID id — a record under a parent — so it is tested below as
// valid-but-absent instead, and 48 and 65 characters take its place.
const NONSENSE = {
  "the empty string": "",
  "a truncated id": "0123456789abcdef0123456789abcde",
  "one character too many": "0123456789abcdef0123456789abcdef0",
  "not hex at all": "nope",
  "right length, wrong alphabet": "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
  "half a parented id": "0123456789abcdef0123456789abcdef0123456789abcdef",
  "a whole pasted sentence": "have you seen this project",
};

await t("**an id that cannot be parsed reads as absent, not as a throw**", async () => {
  for (const [what, id] of Object.entries(NONSENSE)) {
    assert.strictEqual(await db.get("tasks", id), null, `${what} (${JSON.stringify(id)})`);
  }
});

await t("a well-formed id that was never written is absent, as it always was", async () => {
  assert.strictEqual(await db.get("tasks", "f".repeat(32)), null);
});

await t("a 64-hex id in a domain with no parent is absent, not a refusal", async () => {
  // Valid as an id since #122; it simply cannot address anything here.
  assert.strictEqual(await db.get("tasks", "0123456789abcdef".repeat(4)), null);
});

await t("**a pre-#122 bare id does not crash the app on load**", async () => {
  // The case #122 actually created: a component id stored before that change
  // is 32 hex where the domain now wants 64. This is the stale-`lastOpened`
  // scenario, and it is no longer hypothetical.
  assert.strictEqual(await db.get("component", "0123456789abcdef0123456789abcdef"), null);
});

await t("THE CONTROL: a real id still reads, and a missing domain still throws", async () => {
  // Without the first half, everything above would pass on a `get` that
  // returned null for absolutely everything.
  assert.deepStrictEqual(await db.get("tasks", real.id), real);
  // And "no such domain" is a programming error, not an outside id: it must
  // not be hidden behind `null`.
  await assert.rejects(() => db.get("nope", real.id), /no schema/);
});

await t("a WRITE against an unaddressable id still refuses", async () => {
  // The asymmetry is deliberate: answering "nothing to do" to a delete or an
  // update aimed at an id nobody can resolve would hide the caller's mistake
  // instead of reporting it.
  await assert.rejects(() => db.delete("tasks", "nope"));
  await assert.rejects(() => db.update("tasks", "nope", { title: "x" }));
});

console.log("\nunparseable id: all ok");
