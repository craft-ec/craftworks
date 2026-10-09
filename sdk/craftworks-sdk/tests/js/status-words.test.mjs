// THE SDK'S STATUS VOCABULARY IS ONE LIST (sdk#518). `sdk.status` is BUILT from the Rust owner (`status_words`, each
// enum's ALL) and the binding's JS owner (`BINDING`): these assertions compare it against those owners, read here
// independently, so a copy typed into wrap.js would fail. A source check keeps the binding words from coming back as
// literals beside their owner.
import assert from "node:assert";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { wrap } from "../../js/wrap.js";
import { BINDING } from "../../js/engine-db.js";

const raw = createRequire(import.meta.url)("../../pkg/node/craftworks_sdk.js");
const sdk = wrap(raw);
const said = JSON.parse(raw.status_words());

// ---- every group is its owner's list, keyed by the word upper-cased ----------
const groups = { row: "rowState", put: "putStatus", site: "siteStatus", appPublish: "appPublishStatus", canWrite: "canWrite", asked: "asked", groupHealth: "groupHealth", repairOutcome: "repairOutcome", failWhy: "failWhy" };
assert.deepStrictEqual(Object.keys(sdk.status).sort(), [...Object.keys(groups), "binding"].sort(), "sdk.status has exactly these groups");
for (const [group, name] of Object.entries(groups)) {
  assert.deepStrictEqual(Object.values(sdk.status[group]), said[name], `status.${group} is the Rust ${name} list`);
  for (const [k, w] of Object.entries(sdk.status[group])) assert.strictEqual(k, w.toUpperCase(), `status.${group}.${k}`);
  assert.ok(Object.isFrozen(sdk.status[group]), `status.${group} is frozen`);
}
assert.deepStrictEqual(Object.keys(said).sort(), Object.values(groups).sort(), "the Rust list carries a group sdk.status drops");
assert.strictEqual(sdk.status.binding, BINDING, "status.binding is engine-db.js's BINDING itself");
assert.ok(Object.isFrozen(sdk.status) && Object.isFrozen(BINDING));
assert.deepStrictEqual(sdk.rowStates(), said.rowState, "rowStates() and status.row agree");

// ---- the words the builder copied, now imported --------------------------------
assert.strictEqual(sdk.status.row.ROLLED_BACK, "ROLLED_BACK");
assert.strictEqual(sdk.status.put.PUT, "put");
assert.ok(!("FAILED" in sdk.status.put), "put_status has no `failed` (builder sweep #14)");

// ---- rowLost: the rolled-back state only ---------------------------------------
for (const code of said.rowState) assert.strictEqual(sdk.rowLost(code), code === sdk.status.row.ROLLED_BACK, code);
assert.strictEqual(sdk.rowLost("NOT_A_STATE"), false);
assert.strictEqual(sdk.rowLost(undefined), false);

// ---- ONE HOME: no binding-status literal beside its owner ----------------------
for (const file of ["wrap.js", "engine-db.js"]) {
  const src = readFileSync(new URL(`../../js/${file}`, import.meta.url), "utf8")
    .split("\n")
    .filter(l => !l.includes("export const BINDING") && !/^\s*(\/\/|\*)/.test(l));
  for (const w of Object.values(BINDING)) {
    const hit = src.find(l => l.includes(`"${w}"`) || l.includes(`'${w}'`));
    assert.strictEqual(hit, undefined, `js/${file} spells the binding word "${w}" instead of BINDING: ${hit}`);
  }
}
console.log("status-words: ok");
