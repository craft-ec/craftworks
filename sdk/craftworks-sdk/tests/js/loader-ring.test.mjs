// THE LOADER'S RECORDING (js/served.js `LoaderRing`, sdk#386's instrument work): every fetch round the loader makes
// before the SDK exists, as numbers from the GENERATED vocabulary (js/instrument-vocab.js), recorded at ONE site
// inside served()'s round, bounded, and handed to the page once. The page validates every number (page::loader).
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { LoaderRing, coarsenMs, loaderTrace, served, statusClassOf } from "../../js/served.js";
import { EVENT, KEY, OUTCOME, STATUS, STRIDE } from "../../js/instrument-vocab.js";

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.message}\n`); }
};

/** A node answering `answers` in order: a status, "throw" (no HTTP answer), or bytes. */
const node = answers => {
  let n = 0;
  return async () => {
    const a = answers[Math.min(n, answers.length - 1)];
    n += 1;
    if (a === "throw") throw new Error("connection refused");
    return typeof a === "number" ? new Response("", { status: a }) : new Response(a);
  };
};

/** A clock the test drives, and a sleep that moves it. */
const clock = (start = 1_790_253_181_367) => {
  let at = start;
  return { now: () => at, sleep: async ms => { at += ms; }, step: ms => { at += ms; } };
};

/** The ring's events as objects, for asserting. */
const decoded = ring => {
  const ev = ring.events();
  const out = [];
  for (let i = 0; i < ev.length; i += STRIDE) out.push(ev.slice(i, i + STRIDE));
  return out;
};
const counters = (ring, o, key) => decoded(ring).filter(([k, , a, b]) => k === EVENT.COUNTER && a === o && b === key).map(e => e[4]);

await t("**404, 404, 200: three rounds, each ASKED and ANSWERED with its class, its attempt, and a coarsened offset**", async () => {
  const c = clock();
  const ring = new LoaderRing(64, c.now);
  const got = await served({ url: "/a/secret-app-name/piece" }, { fetch: node([404, 404, new Uint8Array([1])]), sleep: c.sleep, now: c.now, rec: ring });
  assert.deepEqual([...got], [1]);
  const asked = decoded(ring).filter(([k, , a]) => k === EVENT.EDGE && a === 0).map(e => e[3]);
  assert.deepEqual(asked, [1, 2, 3], "not one Request per round, by the ring's own order");
  assert.deepEqual([1, 2, 3].map(o => counters(ring, o, KEY.StatusClass)[0]), [STATUS.NotFound, STATUS.NotFound, STATUS.Ok]);
  assert.deepEqual([1, 2, 3].map(o => counters(ring, o, KEY.Attempts)[0]), [1, 2, 3]);
  // The waits between rounds are the RTO schedule (1 s, then 2 s): offsets 0, 1000, 3000 -- coarsened.
  assert.deepEqual([1, 2, 3].map(o => counters(ring, o, KEY.OffsetMs)[0]), [0, 1000, 3000]);
});

await t("**no HTTP answer is Timeout + NetworkError; a cancel is Withdrawn + Abort**", async () => {
  const c = clock();
  const ring = new LoaderRing(64, c.now);
  await served({ url: "/x" }, { fetch: node(["throw", new Uint8Array([2])]), sleep: c.sleep, now: c.now, rec: ring });
  assert.ok(decoded(ring).some(e => e[0] === EVENT.EXIT && e[2] === 1 && e[3] === OUTCOME.Timeout), "a network error was not closed Timeout");
  assert.deepEqual(counters(ring, 1, KEY.StatusClass), [STATUS.NetworkError]);
  const stop = new AbortController();
  const ring2 = new LoaderRing(64, c.now);
  const cancelled = served({ url: "/y" }, {
    fetch: async (_u, init) => { stop.abort(); throw new Error("aborted"); },
    signal: stop.signal, sleep: c.sleep, now: c.now, rec: ring2,
  });
  await assert.rejects(cancelled, /cancelled/);
  assert.ok(decoded(ring2).some(e => e[0] === EVENT.EXIT && e[2] === 1 && e[3] === OUTCOME.Withdrawn), "a cancel was not closed Withdrawn");
  assert.deepEqual(counters(ring2, 1, KEY.StatusClass), [STATUS.Abort]);
});

await t("**NO USER CONTENT: every recorded value is a whole number, whatever the URL said**", async () => {
  const c = clock();
  const ring = new LoaderRing(64, c.now);
  await served({ url: "https://node/v1/contract/web/SECRET-ADDRESS/piece?who=alice" }, { fetch: node([404, new Uint8Array([3])]), sleep: c.sleep, now: c.now, rec: ring });
  const ev = ring.events();
  assert.ok(ev.length > 0, "THE SETUP: nothing was recorded");
  for (const v of ev) assert.ok(Number.isInteger(v) && v >= 0, `a recorded value is not a code: ${JSON.stringify(v)}`);
  const seg = JSON.stringify(ring.take());
  assert.ok(!/SECRET|alice|contract|piece/.test(seg), `the handover carries content: ${seg}`);
});

await t("**BOUNDED: the last N events kept, older ones overwritten and COUNTED; handed over once, then silent**", async () => {
  const c = clock();
  const ring = new LoaderRing(4, c.now);
  for (let r = 0; r < 3; r += 1) ring.asked(r); // 9 events into 4 places
  assert.equal(ring.events().length, 4 * STRIDE);
  assert.equal(ring.dropped, 5, "the overwritten events were not counted");
  assert.deepEqual(decoded(ring).map(e => e[2] === 0 ? `edge#${e[3]}` : `${e[3]}@${e[2]}`), [`${KEY.OffsetMs}@2`, "edge#3", `${KEY.Attempts}@3`, `${KEY.OffsetMs}@3`], "not the LAST four, oldest first");
  const seg = ring.take();
  assert.equal(seg.dropped, 5);
  ring.asked(9);
  assert.equal(ring.events().length, seg.events.length, "the ring recorded after it was handed over");
});

await t("**RECORDING IS NEVER AN INPUT: a full ring, a taken ring or no ring -- served() returns the same, after the same waits**", async () => {
  // engineer4's condition (js/served.js's owner): what served() fetches, waits and returns cannot depend on the ring.
  const run = async rec => {
    const c = clock();
    let asked = 0;
    const fetch = async () => { asked += 1; return asked < 3 ? new Response("", { status: 404 }) : new Response(new Uint8Array([7, 7])); };
    const waits = [];
    const bytes = await served({ url: "/f" }, { fetch, sleep: async ms => { waits.push(ms); await c.sleep(ms); }, now: c.now, rec });
    return { bytes: [...bytes], asked, waits };
  };
  const full = new LoaderRing(1, clock().now);
  for (let r = 0; r < 5; r += 1) full.asked(r); // full, dropping
  const taken = new LoaderRing(8, clock().now);
  taken.take();
  const none = await run(null);
  assert.deepEqual(await run(full), none, "a FULL ring changed what served() did");
  assert.deepEqual(await run(taken), none, "a TAKEN ring changed what served() did");
  assert.ok(full.dropped > 0, "THE SETUP: the ring was not full");
});

await t("the generated rules: coarsening and the HTTP classes are instrument's own", async () => {
  assert.deepEqual([0, 999, 1_000, 59_999, 60_000, 1_790_253_181_367].map(coarsenMs), [0, 990, 1_000, 59_900, 60_000, 1_790_253_181_000]);
  assert.deepEqual([200, 206, 404, 503, 403].map(statusClassOf), [STATUS.Ok, STATUS.Ok, STATUS.NotFound, STATUS.ServerError, STATUS.OtherHttp]);
});

await t("loaderTrace names the rounds from the vocabulary (for a load where the SDK never came)", async () => {
  const c = clock();
  const ring = new LoaderRing(64, c.now);
  await served({ url: "/z" }, { fetch: node([503, new Uint8Array([4])]), sleep: c.sleep, now: c.now, rec: ring });
  const text = loaderTrace(ring);
  assert.match(text, /fetch#1 asked/);
  assert.match(text, /fetch#1 StatusClass=ServerError/);
  assert.match(text, /fetch#2 StatusClass=Ok/);
});

await t("**js/instrument-vocab.js IS what page/src/loader.rs and craftworks-instrument produce, not a copy**", async () => {
  // Generated by build.sh; re-run here, so a hand edit (or a stale file) is caught.
  const { execFileSync } = await import("node:child_process");
  const out = execFileSync("cargo", ["run", "-q", "-p", "page", "--example", "instrument_js"], { encoding: "utf8" });
  assert.equal(await readFile(new URL("../../js/instrument-vocab.js", import.meta.url), "utf8"), out, "js/instrument-vocab.js is not what the generator produces");
});

if (failures) { process.stdout.write(`${failures} failed\n`); process.exit(1); }
