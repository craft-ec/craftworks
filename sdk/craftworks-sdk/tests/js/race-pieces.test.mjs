// THE LOAD-PIECE RACE (sdk#347): any k of k + m, through served(), paced by the page's window, and a stalled piece
// never holds the page. Every "did not wait" claim has its count beside it: requests started, in flight at once,
// and which pieces were asked.
import assert from "node:assert/strict";
import { webcrypto } from "node:crypto";
import { raceK } from "../../js/served.js";
import { WINDOW_AFTER_ANSWERS } from "../../js/rto.js";

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.message}\n`); }
};
const subtle = webcrypto.subtle;
const hex = b => [...new Uint8Array(b)].map(x => x.toString(16).padStart(2, "0")).join("");
const sha = async b => hex(await subtle.digest("SHA-256", b));

/** k + m pieces of random bytes, and a node whose answer per piece `how(i)` decides: "ok", "silent", "wrong",
 * "notfound" (404 every time: the node does not hold it), "notready" (503 every time: a node still joining). */
async function rig(k, m, how, { delayMs = 1 } = {}) {
  const bytes = Array.from({ length: k + m }, (_, i) => webcrypto.getRandomValues(new Uint8Array(64 + i)));
  const pieces = await Promise.all(bytes.map(async (b, i) => ({ url: `http://node/p${i}`, sha256: await sha(b) })));
  const stats = { started: 0, inFlight: 0, maxInFlight: 0, byPiece: new Map() };
  const fetch = (url, init) => {
    const i = Number(url.slice(url.lastIndexOf("p") + 1));
    stats.started += 1;
    stats.byPiece.set(i, (stats.byPiece.get(i) ?? 0) + 1);
    stats.inFlight += 1;
    stats.maxInFlight = Math.max(stats.maxInFlight, stats.inFlight);
    return new Promise((resolve, reject) => {
      let over = false;
      const done = () => { if (!over) { over = true; stats.inFlight -= 1; } };
      init?.signal?.addEventListener?.("abort", () => { done(); reject(new Error("aborted")); }, { once: true });
      if (how(i) === "silent") return; // never answers: only the abort ends it
      setTimeout(() => {
        if (init?.signal?.aborted) return;
        done();
        if (how(i) === "notfound" || how(i) === "notready") {
          resolve({ ok: false, status: how(i) === "notfound" ? 404 : 503, arrayBuffer: async () => new ArrayBuffer(0) });
          return;
        }
        const body = how(i) === "wrong" ? new Uint8Array(bytes[i].length) : bytes[i];
        resolve({ ok: true, status: 200, arrayBuffer: async () => body.buffer.slice(body.byteOffset, body.byteOffset + body.length) });
      }, delayMs);
    });
  };
  return { bytes, spec: { k, m, pieces }, fetch, stats };
}

const fastSleep = (ms, sig) => new Promise(ok => { const t = setTimeout(ok, 1); sig?.addEventListener?.("abort", () => { clearTimeout(t); ok(); }, { once: true }); });

await t("every piece answers: resolves at the first k, and NOTHING is asked after it resolved", async () => {
  const r = await rig(23, 8, () => "ok", { delayMs: 5 });
  const out = await raceK(r.spec, { fetch: r.fetch, subtle, sleep: fastSleep });
  assert.equal(out.verified, 23);
  assert.equal(out.pieces.filter(Boolean).length, 23);
  for (const [i, b] of out.pieces.entries()) if (b) assert.deepEqual(b, r.bytes[i]);
  const atResolve = r.stats.started;
  await new Promise(ok => setTimeout(ok, 40));
  process.stdout.write(`    asked ${out.asked.length} of 31 (answers open the window), ${atResolve} requests, max in flight ${r.stats.maxInFlight}\n`);
  assert.equal(r.stats.started, atResolve, "a request started after the race had resolved");
  // Pieces CANCELLED at k were not missing: none is marked for repair.
  assert.ok(out.asked.length > 23, `THE SETUP: nothing was cancelled at k (asked ${out.asked.length})`);
  assert.deepEqual(out.notHeld, [], "a piece the race cancelled at k was marked not-held (it would be re-PUT)");
  assert.equal(r.stats.inFlight, 0, "a request still in flight after the race resolved");
});

await t("m pieces SILENT for ever: resolves from the others, and the silent ones are aborted", async () => {
  const silent = new Set([0, 3, 7, 11, 15, 19, 22, 30]);
  const r = await rig(23, 8, i => (silent.has(i) ? "silent" : "ok"));
  const out = await raceK(r.spec, { fetch: r.fetch, subtle, sleep: fastSleep });
  assert.equal(out.verified, 23);
  for (const i of silent) assert.equal(out.pieces[i], null);
  assert.deepEqual(out.notHeld, [], "SILENCE was taken for not-held (rule 8: silence is re-asked, never missing)");
  await new Promise(ok => setTimeout(ok, 20));
  assert.equal(r.stats.inFlight, 0, `${r.stats.inFlight} requests still in flight after the race resolved`);
});

await t("the window paces it: never more in flight than the page's window after that many answers", async () => {
  const silent = new Set([0, 1, 2]);
  const r = await rig(23, 8, i => (silent.has(i) ? "silent" : "ok"), { delayMs: 3 });
  await raceK(r.spec, { fetch: r.fetch, subtle, sleep: fastSleep });
  process.stdout.write(`    max in flight ${r.stats.maxInFlight}; window starts at ${WINDOW_AFTER_ANSWERS[0]}\n`);
  assert.ok(r.stats.maxInFlight <= WINDOW_AFTER_ANSWERS[23], "more in flight than the window allows");
  assert.ok(r.stats.maxInFlight > WINDOW_AFTER_ANSWERS[0], "the window never opened past its start: not paced by answers");
});

await t("a piece with the WRONG bytes does not count, and the race still finishes from the rest", async () => {
  const wrong = new Set([1, 2]);
  const r = await rig(10, 4, i => (wrong.has(i) ? "wrong" : "ok"));
  const out = await raceK(r.spec, { fetch: r.fetch, subtle, sleep: fastSleep });
  assert.equal(out.verified, 10);
  for (const i of wrong) assert.equal(out.pieces[i], null, `wrong bytes counted for piece ${i}`);
});

await t("more than m pieces refuse: rejected in words, not waited on for ever", async () => {
  const r = await rig(4, 2, i => (i < 3 ? "wrong" : "ok"));
  // Bounded HERE (the test's own deadline, not the race's): a race that stopped counting refusals would wait for ever.
  const within = (p, ms) => Promise.race([p, new Promise((_, no) => setTimeout(() => no(new Error(`no answer in ${ms} ms: waited on refused pieces`)), ms))]);
  await assert.rejects(within(raceK(r.spec, { fetch: r.fetch, subtle, sleep: fastSleep }), 5000), /refused: 3 of 6/);
});

await t("fewer than k can ever arrive: it WAITS (no time cut-off) and says so, until the person cancels", async () => {
  const r = await rig(4, 2, i => (i < 3 ? "silent" : "ok"));
  const cancel = new AbortController();
  const waits = [];
  const p = raceK(r.spec, { fetch: r.fetch, subtle, sleep: fastSleep, signal: cancel.signal, onWait: w => waits.push(w) });
  await new Promise(ok => setTimeout(ok, 50));
  cancel.abort();
  await assert.rejects(p, /cancelled with 3 of 4 verified/);
});

await t("**a piece the node answers NOT FOUND is marked not-held (the repair's set); a 503 (not ready) is not**", async () => {
  const gone = new Set([2, 9, 17]);
  const busy = new Set([4, 25]);
  const r = await rig(23, 8, i => (gone.has(i) ? "notfound" : busy.has(i) ? "notready" : "ok"), { delayMs: 3 });
  const out = await raceK(r.spec, { fetch: r.fetch, subtle, sleep: fastSleep });
  assert.equal(out.verified, 23);
  process.stdout.write(`    asked ${out.asked.length}, not held ${JSON.stringify(out.notHeld)}\n`);
  assert.deepEqual(out.notHeld, [...gone].filter(i => out.asked.includes(i)).sort((a, b) => a - b), "not exactly the 404s");
  assert.ok(out.notHeld.length >= 1, "THE SETUP: no 404 piece was asked");
  for (const i of busy) assert.ok(!out.notHeld.includes(i), `a 503 (not ready) was taken for not-held: ${i}`);
});

if (failures) { process.stdout.write(`race-pieces: ${failures} FAILED\n`); process.exit(1); }
process.stdout.write("race-pieces: all ok\n");
