// A BINDING OVER ONE PARENT reads that parent's band, not the domain
// (craftworks-sdk#137), through the JavaScript surfaces an app holds.
//
// `Db::children` made "the children of P" a bounded read in #122/#127, and it
// did not reach bindings: a component showing one parent's rows held a
// binding over the whole domain. The cost of the band read itself is #127's
// measurement (`tests/parent_key.rs`); what this pins is that the BINDING
// asks for the band — and, live, is woken by its band and not by a sibling's.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { wrap } from "../../js/wrap.js";
import { engineDb } from "../../js/engine-db.js";
const sdk = wrap(createRequire(import.meta.url)("../../pkg/node/craftworks_sdk.js"));

// `  ok  `, two spaces: gate.sh counts `^  ok `.
const t = async (name, fn) => { await fn(); process.stdout.write(`  ok  ${name}\n`); };
const P = "0000000000000000000000000000000a", Q = "0000000000000000000000000000000b";
const schema = { type: "Item", fields: [{ name: "project", kind: "text", required: true }, { name: "n", kind: "int" }], parent: "project" };

/** An in-memory db: 10 rows under P beside 1,000 under Q, with every read counted. */
const tenBesideAThousand = async () => {
  const db = new sdk.Db();
  await db.define("item", schema);
  for (let i = 0; i < 10; i += 1) await db.put("item", { project: P, n: i });
  for (let i = 0; i < 1000; i += 1) await db.put("item", { project: Q, n: i });
  const calls = [];
  for (const m of ["scan", "children"]) {
    const real = db[m].bind(db);
    db[m] = (...a) => { calls.push([m, a[0], m === "children" ? a[1] : null]); return real(...a); };
  }
  return { db, calls };
};

await t("**in memory: a binding with a parent reads ITS BAND — ten rows beside a thousand, and never a scan**", async () => {
  const { db, calls } = await tenBesideAThousand();
  const b = db.bind("item", { parent: P });
  assert.equal(b.parent, P, "the binding does not report the parent it was given");
  await b.reload();
  assert.equal(b.getSnapshot().length, 10);
  assert.ok(b.getSnapshot().every(r => r.fields.project === P));
  assert.deepEqual(calls, [["children", "item", P]], "the binding read something other than its band");
});

await t("THE CONTROL: without a parent it is the whole domain, as before", async () => {
  const { db, calls } = await tenBesideAThousand();
  const b = db.bind("item");
  assert.equal(b.parent, null);
  await b.reload();
  assert.equal(b.getSnapshot().length, 1010);
  assert.deepEqual(calls.map(c => c[0]), ["scan"]);
});

await t("a limit and a direction apply WITHIN the band", async () => {
  const { db } = await tenBesideAThousand();
  const b = db.bind("item", { parent: P, limit: 3, reverse: true });
  await b.reload();
  assert.deepEqual(b.getSnapshot().map(r => r.fields.n), [9, 8, 7]);
});

await t("a domain with no parent declared REFUSES a parent, as `children` does", async () => {
  const db = new sdk.Db();
  await db.define("flat", { type: "Flat", fields: [{ name: "n", kind: "int" }] });
  await assert.rejects(db.bind("flat", { parent: P }).reload(), /parent/i);
});

// ---- the engine-backed surface: the band is also what it WATCHES -------------

/** A session that answers from a fixed page and records what it was asked. */
const recordingSession = () => {
  const s = {
    asked: [], bound: [], stale: [],
    watch_key: (d, p) => `${d}#${p}`,
    bind: k => s.bound.push(k), unbind: () => {}, rendered: k => s.asked.push(["rendered", k]),
    children: (d, p) => { s.asked.push(["children", d, p]); return "[]"; },
    scan: d => { s.asked.push(["scan", d]); return "[]"; },
    take_stale: () => { const out = s.stale; s.stale = []; return JSON.stringify(out); },
    take_loads: () => "[]", root: () => "r", count: () => 0,
  };
  return s;
};

await t("**engine: a parented binding reads its band and WATCHES its band**", async () => {
  const s = recordingSession();
  const db = engineDb(s);
  const b = db.bind("item", { parent: P, live: true });
  await b.reload();
  assert.equal(b.parent, P);
  assert.ok(s.asked.some(a => a[0] === "children" && a[2] === P), `asked ${JSON.stringify(s.asked)}`);
  assert.ok(!s.asked.some(a => a[0] === "scan"), "the band binding scanned the domain");
  assert.deepEqual(s.bound, [`item#${P}`], "it watches something other than its band");
});

await t("**engine: a live band is re-run for ITS band, and NOT for a sibling parent's**", async () => {
  const s = recordingSession();
  const db = engineDb(s);
  const p = db.bind("item", { parent: P, live: true });
  const q = db.bind("item", { parent: Q, live: true });
  await p.reload(); await q.reload();
  const reads = parent => s.asked.filter(a => a[0] === "children" && a[2] === parent).length;
  const [p0, q0] = [reads(P), reads(Q)];
  s.stale = [`item#${Q}`];              // the session says: Q's band moved
  db.drain();
  await new Promise(r => setTimeout(r, 0));
  assert.equal(reads(Q) - q0, 1, "the band that moved was not re-read");
  assert.equal(reads(P) - p0, 0, "a change under a SIBLING parent re-ran this band");
});
