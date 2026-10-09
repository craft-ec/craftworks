// THE DEFINITION DOORS THROUGH THE REAL SDK (app-as-data P2, ARCHITECTURE §19): the in-tab `Db` from pkg/node, as an
// app holds it (`wrap`). The reserved names are spelled here only to plant them: an ordinary write of each is REFUSED;
// the doors edit the draft, publish it in one write, and read both definitions back; the live marker is created once.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { wrap } from "../../js/wrap.js";

const sdk = wrap(createRequire(import.meta.url)("../../pkg/node/craftworks_sdk.js"));
let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.stack}\n`); }
};

await t("**an ordinary write of the reserved prefix is refused**, by name, and an ordinary domain is not", async () => {
  const db = new sdk.Db();
  for (const d of ["craftworks.draft", "craftworks.app", "craftworks.published", "craftworks"]) {
    await assert.rejects(() => db.define(d, { type: "T", fields: [] }), /reserved `craftworks\.` prefix/, `define of ${d}`);
  }
  await db.define("rows", { type: "Row", fields: [{ name: "n", kind: "int" }] });
  assert.ok((await db.put("rows", { n: 1 })).id, "THE CONTROL: an ordinary domain writes");
});

await t("**the doors**: draft edited, published in one write, both definitions read back; the marker created once", async () => {
  const db = new sdk.Db();
  await db.draftPut("meta", { name: "Notes" });
  await db.draftPut("c/header", { kind: "Text" });
  await db.draftPut("d/rows", { type: "Row" });
  assert.equal(await db.draftDelete("d/rows"), true);
  const byKey = rs => Object.fromEntries(rs.map(r => [r.key, r.body]));
  assert.deepEqual(byKey(await db.definition("draft")), { meta: { name: "Notes" }, "c/header": { kind: "Text" } });
  assert.deepEqual(await db.definition("app"), [], "published before a publish");
  assert.equal(await db.publishDefinition(), 2);
  assert.deepEqual(byKey(await db.definition("app")), { meta: { name: "Notes" }, "c/header": { kind: "Text" } });
  assert.equal(await db.publishDefinition(), 0, "an unchanged draft published something");
  await assert.rejects(() => db.draftPut("x/y", {}), /not a definition key/);
  assert.equal(await db.isPublished("rows"), false);
  assert.equal(await db.publishedState("rows"), null, "an unmarked domain has a marker record");
  assert.equal((await db.markPublished("rows")).outcome, "created");
  const marker = await db.publishedState("rows");
  assert.ok(marker && typeof marker.state === "string", `the marker's record carries no write state: ${JSON.stringify(marker)}`);
  assert.equal((await db.markPublished("rows")).outcome, "exists");
  assert.equal(await db.isPublished("rows"), true);
});

// §19 P3: a definition is READ by app (the builder's project list), and only read: a write door takes no app, so an
// app argument handed to one does not reach another app's draft.
await t("**another app's definition is read, never written**: the in-tab Db reads by app id; a write door ignores one", async () => {
  const db = new sdk.Db();
  await db.draftPut("meta", { name: "Mine" });
  assert.deepEqual(await db.definition("draft", "someone-else"), [], "another app's (empty) draft read this tab's records");
  // A WRITE DOOR TAKES NO APP (the architect on sdk#551): handed one, it is refused by name -- never dropped, which
  // wrote THIS tab's draft under the caller's belief that it wrote another's.
  for (const [door, call] of [
    ["draftPut", () => db.draftPut("meta", { name: "Planted" }, "someone-else")],
    ["draftFile", () => db.draftFile("app.js", new Uint8Array([1]), {}, "someone-else")],
    ["draftDelete", () => db.draftDelete("meta", "someone-else")],
    ["publishDefinition", () => db.publishDefinition("someone-else")],
    ["markPublished", () => db.markPublished("rows", "someone-else")],
  ]) {
    await assert.rejects(call, e => e.code === "REFUSED" && e.message.includes(`\`${door}\` takes no app`), `${door} took an app`);
  }
  assert.deepEqual(await db.definition("draft", "someone-else"), [], "a refused write door wrote into another app's draft");
  assert.deepEqual((await db.definition("draft")).map(r => r.body.name), ["Mine"], "a refused write door wrote this tab's draft");
  await assert.rejects(() => db.definition("draft", "Not An App"), /app id/);
  assert.deepEqual(await db.definitionApps(), [], "the tab's own (unnamed) draft was listed as an app of a tree");
});

// app-as-data P5: the app's CODE as data -- a file's bytes through `draftFile`, published by the one publish write,
// read back as a Uint8Array byte for byte; `meta.entry` must name a file of the draft.
await t("**a file of the app's code** round-trips as bytes, and the entry must be a file of the draft", async () => {
  const db = new sdk.Db();
  const js = new Uint8Array(38_006).map((_, i) => i % 251);
  await db.draftPut("meta", { entry: "f/app.js" });
  await assert.rejects(() => db.publishDefinition(), /meta\.entry/, "an entry naming no file was published");
  await db.draftFile("app.js", js);
  await db.draftFile("components/header.js", new TextEncoder().encode("export default 1;"), { type: "text/javascript" });
  await assert.rejects(() => db.draftFile("../x.js", new Uint8Array([1])), /not a definition key/);
  assert.equal(await db.publishDefinition(), 3);
  const app = Object.fromEntries((await db.definition("app")).map(r => [r.key, r]));
  assert.ok(app["f/app.js"].bytes instanceof Uint8Array, "a file's bytes are not a Uint8Array");
  assert.deepEqual([...app["f/app.js"].bytes], [...js], "a file did not read back byte for byte");
  assert.deepEqual(app["f/components/header.js"].body, { type: "text/javascript" });
  assert.equal(app.meta.bytes, undefined, "a definition record came with bytes");
});

if (failures) { process.stdout.write(`\n${failures} failing\n`); process.exit(1); }
process.stdout.write("\nall passing\n");
