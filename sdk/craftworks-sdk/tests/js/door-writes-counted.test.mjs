// A DEFINITION DOOR'S WRITE MOVES THE UNSAVED COUNT WITH IT, as a row write does (`touched` → `handle.wrote()`,
// craftworks-sdk#163). Measured before: four door writes had returned and `saving` still read 0 for ~800 ms, until
// the next tick or message ran the session's guard -- a page, or a tool, reading it right after writing saw
// "nothing unsaved". Here: each door write arms the guard (`handle.wrote`) by the time it resolves; a refused one
// arms nothing; a row write is the control, which always did.
import assert from "node:assert/strict";
import { engineDb } from "../../js/engine-db.js";

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.stack}\n`); }
};

/** A session answering every door, and a handle counting `wrote()` -- the session's guard. */
const handleOf = () => {
  const h = { wrote: 0 };
  h.session = {
    draft_put() {}, draft_file() {}, draft_delete: () => true, publish_definition: () => 1,
    publish_definition_site: () => "{}", mark_published: () => "{}",
    put: () => JSON.stringify({ id: "r1", fields: {} }),
    take_stale: () => "[]", take_loads: () => "[]", root: () => "r",
  };
  return { handle: { session: h.session, wrote: () => { h.wrote += 1; } }, h };
};

await t("**every door write arms the unsaved guard when it resolves**, once each", async () => {
  const { handle, h } = handleOf();
  const db = engineDb(handle);
  const writes = [
    ["draftPut", () => db.draftPut("meta", { name: "A" })],
    ["draftFile", () => db.draftFile("app.js", new Uint8Array([1]))],
    ["draftDelete", () => db.draftDelete("c/x")],
    ["publishDefinition", () => db.publishDefinition()],
    ["publishDefinition({ site })", () => db.publishDefinition({ site: { set: {}, webappCode: new Uint8Array(), pieceStates: "[]", siteCode: new Uint8Array(), starter: {} } })],
    ["markPublished", () => db.markPublished("rows")],
  ];
  for (const [door, write] of writes) {
    const before = h.wrote;
    await write();
    assert.equal(h.wrote - before, 1, `${door} did not arm the guard as it resolved (the count would lag to the next tick)`);
  }
});

await t("THE CONTROL: a row write arms it too (it always did), and a REFUSED door write arms nothing", async () => {
  const { handle, h } = handleOf();
  const db = engineDb(handle);
  await db.put("rows", { n: 1 });
  assert.equal(h.wrote, 1, "a row write no longer arms the guard");
  await assert.rejects(() => db.draftPut("meta", {}, "someone-else"), /takes no app/);
  assert.equal(h.wrote, 1, "a refused door write armed the guard");
});

if (failures) { process.stdout.write(`\n${failures} failing\n`); process.exit(1); }
process.stdout.write("\nall passing\n");
