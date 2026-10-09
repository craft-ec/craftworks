// `children` through the JavaScript surface — the one an app actually holds.
//
// `tests/parent_key.rs` proves the behaviour in Rust. That is not the same
// claim: a wasm method nobody can call from JS is not a feature, and when this
// file was written BOTH JS surfaces were missing `children` entirely while the
// whole Rust suite and the gate were green. `js-surfaces-agree` did not catch
// it either, because the two surfaces agreed — on its absence.
//
// So this drives the entry an app uses, which is the workspace rule: test
// through the ENTRY an app uses, not past it.
import assert from "node:assert";
import { createRequire } from "node:module";
import { wrap } from "../../js/wrap.js";
const sdk = wrap(createRequire(import.meta.url)("../../pkg/node/craftworks_sdk.js"));

// `  ok  ` with TWO leading spaces, which is not cosmetic: gate.sh counts
// `^  ok ` and a flush-left line is invisible to the ratchet. These four
// checks ran green and uncounted before this was fixed, which means deleting
// them later would not have been caught — the exact state the ratchet exists
// to prevent.
const t = async (name, fn) => { await fn(); process.stdout.write(`  ok  ${name}\n`); };

const componentSchema = {
  type: "Component",
  fields: [
    { name: "project", kind: "text", required: true },
    { name: "title", kind: "text" },
  ],
  parent: "project",
};

const fresh = async () => {
  const db = new sdk.Db();
  await db.define("component", componentSchema);
  return db;
};

await t("a record under a parent gets a 64-hex id and is addressable by it", async () => {
  const db = await fresh();
  const pid = "0000000000000000000000000000000a";
  const r = await db.put("component", { project: pid, title: "first" });

  // Self-describing BY LENGTH: an app never has to be told which kind of id
  // it holds, and never takes one apart.
  assert.match(r.id, /^[0-9a-f]{64}$/);
  assert.strictEqual(r.id.slice(0, 32), pid, "the parent leads the id");

  assert.deepStrictEqual(await db.get("component", r.id), r);
  const u = await db.update("component", r.id, { title: "second" });
  assert.strictEqual(u.fields.title, "second");
  assert.strictEqual(u.id, r.id, "an update does not move the record");
  assert.strictEqual(await db.delete("component", r.id), true);
  assert.strictEqual(await db.get("component", r.id), null);
});

await t("children reads one parent's band, and not the other parent's", async () => {
  const db = await fresh();
  const a = "0000000000000000000000000000000a";
  const b = "0000000000000000000000000000000b";
  for (let i = 0; i < 5; i += 1) await db.put("component", { project: a, title: `a${i}` });
  for (let i = 0; i < 40; i += 1) await db.put("component", { project: b, title: `b${i}` });

  const kids = await db.children("component", a);
  assert.strictEqual(kids.length, 5);
  assert.deepStrictEqual(kids.map(r => r.fields.title), ["a0", "a1", "a2", "a3", "a4"]);
  assert.ok(kids.every(r => r.id.startsWith(a)), "every child is in this parent's band");

  // THE CONTROL that gives the number meaning: the domain really does hold
  // both parents' records, so reading 5 was a bounded read and not an empty one.
  assert.strictEqual(await db.count("component"), 45);
  assert.strictEqual((await db.children("component", b)).length, 40);

  // The scan options travel: this is the same `Scan` the Rust side takes.
  assert.deepStrictEqual(
    (await db.children("component", a, { reverse: true, limit: 2 })).map(r => r.fields.title),
    ["a4", "a3"],
  );
  assert.deepStrictEqual(
    (await db.children("component", a, { after: kids[0].id.slice(32) })).map(r => r.fields.title),
    ["a1", "a2", "a3", "a4"],
  );
});

await t("the refusals arrive in JavaScript as thrown Errors with the Rust message", async () => {
  const db = await fresh();
  const pid = "0000000000000000000000000000000a";
  const r = await db.put("component", { project: pid, title: "x" });

  // A bare rkey cannot address a record in a parent-keyed domain. A read of
  // one is ABSENT (craftworks-sdk#118: ids reaching a read come from outside);
  // a write with one refuses, because "nothing to do" would hide the mistake.
  assert.strictEqual(await db.get("component", r.id.slice(32)), null);
  await assert.rejects(() => db.delete("component", r.id.slice(32)), /wants a 64-hex id/);

  // A patch cannot re-parent: the parent decides the key.
  await assert.rejects(
    () => db.update("component", r.id, { project: "0000000000000000000000000000000b" }),
    /decides the record's key/,
  );

  // A missing or malformed parent is refused at the WRITE, not keyed under
  // something invented.
  await assert.rejects(() => db.put("component", { title: "orphan" }), /required/);
  await assert.rejects(() => db.put("component", { project: "nope" }), /32 hex characters/);

  // And the schema's parent declaration cannot be evolved, because changing it
  // re-keys every record in the domain.
  await assert.rejects(
    () => db.define("component", { ...componentSchema, parent: undefined }),
    /re-keys every record/,
  );
});

await t("THE CONTROL: a domain with no parent is untouched", async () => {
  const db = new sdk.Db();
  await db.define("note", { type: "Note", fields: [{ name: "body", kind: "text" }] });
  const r = await db.put("note", { body: "hello" });

  assert.match(r.id, /^[0-9a-f]{32}$/, "no parent, no parent in the id");
  assert.deepStrictEqual(await db.get("note", r.id), r);

  // "None" and "the question does not apply here" are different answers.
  await assert.rejects(
    () => db.children("note", "0000000000000000000000000000000a"),
    /does not declare a parent/,
  );
});

console.log("\nchildren: all ok");
