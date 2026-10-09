// A DAMAGED GROUP NAMED THROUGH THE REAL `Session` (sdk#524; Codex on #542: a fake session, or a test of the method's
// NAME, stays green with `Session::damaged()` answering "[]"). A reader `Session` from pkg/node opens a tree whose node
// has lost the root's whole group -- the root and every parity block its head names (a group of ONE, k = 1) -- and a
// read of it waits. The node's answers are `page-io/examples/damaged_node.rs`'s, in the stdlib's own encoding.
// `damaged()` then names the root's group DAMAGED at j = 0 of k = 1, on the Session and through the engine db alike.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { engineDb } from "../../js/engine-db.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const { Session } = createRequire(import.meta.url)("../../pkg/node/craftworks_sdk.js");

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.stack}\n`); }
};
const enc = s => new TextEncoder().encode(s);
const hex = b => Buffer.from(b).toString("hex");

/** The node's answers to `frames` (and the Register it serves). */
function node(frames = []) {
  const r = spawnSync("cargo", ["run", "-q", "-p", "page-io", "--example", "damaged_node", "--", hex(enc("register code")), ...frames.map(hex)],
    { cwd: root, encoding: "utf8", maxBuffer: 1 << 26 });
  assert.equal(r.status, 0, r.stderr);
  return JSON.parse(r.stdout);
}

await t("**a read whose root group is lost past its reach: the REAL Session's damaged() names it, and the db forwards it**", async () => {
  const { register, root: rootId } = node();
  const s = new Session(7999);
  s.set_app("notes");
  s.open_named(enc("block code"), register, 1, 0);
  let wake = () => {};
  const db = engineDb({ session: s, onReadsWake: fn => { wake = fn; } });
  db.scan("notes").catch(() => {}); // the read: it waits, and never settles here
  const asked = [];
  for (let round = 0; round < 12 && JSON.parse(s.damaged()).length === 0; round += 1) {
    const out = s.outbound();
    s.sent(out.length);
    if (out.length > 0) {
      const r = node(out);
      asked.push(...r.asked);
      for (const a of r.answers) s.on_inbound(new Uint8Array(Buffer.from(a, "hex")));
    }
    wake();
    s.cold_tick();
    await new Promise(r => setTimeout(r, 0));
  }
  assert.ok(asked.some(x => x.head), `THE SETUP: the reader never asked the node for its head: ${JSON.stringify(asked)}`);
  assert.ok(asked.filter(x => !x.head).length >= 2, `THE SETUP: the reader never asked the root's group: ${JSON.stringify(asked)}`);
  const damaged = JSON.parse(s.damaged());
  assert.equal(damaged.length, 1, `the Session named no damaged group: ${s.damaged()}`);
  assert.equal(damaged[0].block, rootId);
  assert.deepEqual([damaged[0].health, damaged[0].j, damaged[0].k], ["DAMAGED", 0, 1]);
  assert.deepEqual(db.damaged(), damaged, "the engine db did not forward the Session's answer");
});

if (failures) { process.stdout.write(`\n${failures} failing\n`); process.exit(1); }
process.stdout.write("\nall passing\n");
