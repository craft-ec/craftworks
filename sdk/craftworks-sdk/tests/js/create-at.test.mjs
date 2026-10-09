// `createAt` and `slotFrom` through the JavaScript surface an app holds
// (craftworks-sdk#149). `tests/create_at.rs` proves the behaviour in Rust;
// this proves it reaches JS, and that the wasm build derives the SAME slot
// as the native one — the pinned vector, on a second platform.
import assert from "node:assert";
import { createRequire } from "node:module";
import { wrap } from "../../js/wrap.js";
const sdk = wrap(createRequire(import.meta.url)("../../pkg/node/craftworks_sdk.js"));

// Two leading spaces: gate.sh counts `^  ok `.
const t = async (name, fn) => { await fn(); process.stdout.write(`  ok  ${name}\n`); };

const fresh = async () => {
  const db = new sdk.Db();
  await db.define("tasks", { type: "Task", fields: [{ name: "title", kind: "text", required: true }] });
  return db;
};

await t("slotFrom in wasm is the native pinned vector", () => {
  assert.strictEqual(
    sdk.slotFrom(1_750_000_000_000, "craftworks.handoff", "0190aaaa0000000000000000000000aa"),
    "000001977420dc00cd196ed3a1c3f054");
});

await t("slotFrom refuses a time it would have to round", () => {
  for (const bad of [1.5, -1, NaN, 2 ** 60]) {
    assert.throws(() => sdk.slotFrom(bad, "n", "s"), /whole number of milliseconds/, String(bad));
  }
});

await t("**a second createAt at one slot answers `exists` with the stored record; the root does not move**", async () => {
  const db = await fresh();
  const slot = sdk.slotFrom(Date.now() - 1000, "preview", "row-1");
  const first = await db.createAt("tasks", slot, { title: "first" });
  assert.strictEqual(first.outcome, "created");
  assert.strictEqual(first.record.id, slot);
  const root = db.root();
  const again = await db.createAt("tasks", slot, { title: "other" });
  assert.strictEqual(again.outcome, "exists");
  assert.strictEqual(again.record.fields.title, "first");
  assert.strictEqual(db.root(), root);
  assert.strictEqual(await db.count("tasks"), 1);
});

await t("THE CONTROL: a different source gets a different slot, and a second record", async () => {
  const db = await fresh();
  const now = Date.now() - 1000;
  await db.createAt("tasks", sdk.slotFrom(now, "preview", "row-1"), { title: "a" });
  const b = await db.createAt("tasks", sdk.slotFrom(now, "preview", "row-2"), { title: "b" });
  assert.strictEqual(b.outcome, "created");
  assert.strictEqual(await db.count("tasks"), 2);
});

console.log("\ncreate-at (js): all ok");
