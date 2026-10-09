// THE ONE FETCH (`served`, js/artefacts.js): every file a page or tool fetches
// over HTTP comes through it. It waits until answered on the page's back-off
// (rto.js, generated from page/src/rto.rs), with no give-up of its own; only a
// person's cancel, or a `check` that every source refuses as final, ends it.
import assert from "node:assert/strict";
import { served, servedText } from "../../js/served.js";
import { NODE_GET_BOUND_MS, NODE_WEB_BOUND_MS, RTO_SCHEDULE_MS, STILL_FETCHING_AFTER_MS, STILL_FETCHING_REASK_MS } from "../../js/rto.js";

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.message}\n`); }
};

/** A node that answers `answers` in order (a status, "throw", or bytes), then the last one for ever. */
function node(answers) {
  let n = 0;
  const fetch = async () => {
    const a = answers[Math.min(n, answers.length - 1)];
    n += 1;
    if (a === "throw") throw new Error("connection refused");
    if (typeof a === "number") return new Response("", { status: a });
    return new Response(a);
  };
  return { fetch, asked: () => n };
}

/** A clock the test drives: `sleep` records each wait and moves time on. */
function clock() {
  let at = 0;
  const waits = [];
  return { now: () => at, sleep: async ms => { waits.push(ms); at += ms; }, waits };
}

await t("**a file the node does not serve yet is WAITED on (404, 500, a refused connection), then fetched**", async () => {
  const net = node([404, 500, "throw", new TextEncoder().encode("hello")]);
  const c = clock();
  const said = [];
  const got = await served({ url: "/f" }, { fetch: net.fetch, sleep: c.sleep, now: c.now, onWait: w => said.push(w) });
  assert.equal(new TextDecoder().decode(got), "hello");
  assert.equal(net.asked(), 4, "not asked until answered");
  assert.deepEqual(c.waits, RTO_SCHEDULE_MS.slice(0, 3), "the waits are not the page's RTO schedule");
  assert.equal(said.length, 3, "a wait was not said");
  assert.ok(said.every(w => w.failures.length === 1), "a wait did not say what the source did");
});

await t("**NO GIVE-UP: 200 rounds of 404 still wait; only a person's cancel ends it, by name**", async () => {
  const net = node([404]);
  const c = clock();
  const ctl = new AbortController();
  let rounds = 0;
  const p = served({ url: "/never" }, { fetch: net.fetch, sleep: c.sleep, now: c.now, signal: ctl.signal, onWait: () => { rounds += 1; if (rounds === 200) ctl.abort(); } });
  await assert.rejects(p, /\/never: cancelled after \d+ s unanswered/);
  assert.ok(net.asked() >= 200, `gave up after ${net.asked()} asks`);
  assert.ok(c.waits.every(ms => ms <= RTO_SCHEDULE_MS.at(-1)), "a wait past the RTO's ceiling");
});

await t("a `check` every source refuses as FINAL ends it, naming the refusal; one refused only once is waited on", async () => {
  const bytes = new TextEncoder().encode("wrong");
  const c = clock();
  let seen = 0;
  // Final only on the second sight from the same source (the torn-read case).
  const check = async () => { seen += 1; return { says: "not it", answer: seen >= 2 }; };
  await assert.rejects(served({ urls: ["/a"] }, { fetch: node([bytes]).fetch, sleep: c.sleep, now: c.now, check, refusal: "a hash mismatch" }), /refused: a hash mismatch from every source/);
  assert.equal(c.waits.length, 1, "a first refusal was taken as final");
});

await t("url AND urls is refused rather than silently dropping one; no url at all is refused", async () => {
  await assert.rejects(served({ url: "/a", urls: ["/b"] }, { fetch: node([404]).fetch }), /both `url` and `urls`/);
  await assert.rejects(served({}, { fetch: node([404]).fetch }), /no url/);
});

await t("fetch OPTIONS go on every request, as given (a file read fresh: no-store)", async () => {
  const c = clock();
  const seen = [];
  const fetch = async (u, init) => { seen.push(init); return seen.length < 2 ? new Response("", { status: 404 }) : new Response("x"); };
  await served({ url: "/b" }, { fetch, sleep: c.sleep, now: c.now, init: { cache: "no-store" } });
  assert.deepEqual(seen, [{ cache: "no-store" }, { cache: "no-store" }], "an option was dropped on a re-ask");
  const plain = [];
  await served({ url: "/b" }, { fetch: async (u, ...rest) => { plain.push(rest.length); return new Response("x"); } });
  assert.deepEqual(plain, [0], "a request with no options was given some");
});

await t("servedText is the same fetch, decoded", async () => {
  const c = clock();
  assert.equal(await servedText({ url: "/j" }, { fetch: node([404, new TextEncoder().encode('{"a":1}')]).fetch, sleep: c.sleep, now: c.now }), '{"a":1}');
});

/** A node whose answers take TIME on the test's clock: `answers` of `{ status | bytes, ms }`, in order, the last for
 *  ever; each ask is recorded with the time it was SENT. */
function slowNode(c, answers) {
  const sent = [];
  const fetch = async () => {
    const a = answers[Math.min(sent.length, answers.length - 1)];
    sent.push(c.now());
    await c.sleep(a.ms);
    return a.bytes ? new Response(a.bytes) : new Response("", { status: a.status });
  };
  return { fetch, sent };
}
const FILE = new TextEncoder().encode("the file");

const SLOW = NODE_WEB_BOUND_MS + 2; // V20's measured 503: 30,002 ms for the 30,000 ms bound
const [FLOOR] = STILL_FETCHING_REASK_MS;

await t("**THE NODE IS STILL FETCHING (sdk#447): a 503 after the node's web bound is re-asked one floor after it -- a node that has FINISHED answers from its store at once**", async () => {
  const c = clock();
  const net = slowNode(c, [{ status: 503, ms: SLOW }, { bytes: FILE, ms: 5 }]);
  const said = [];
  const got = await served({ url: "/p" }, { fetch: net.fetch, sleep: c.sleep, now: c.now, onWait: w => said.push(w) });
  assert.deepEqual(got, FILE);
  assert.deepEqual(net.sent, [0, SLOW + FLOOR], `asked at ${net.sent}: the finished piece was not taken one floor after the 503 (batch 4 step 12: held for B)`);
  assert.ok(said.length === 1 && said[0].stillFetching && /still fetching/.test(said[0].says), `the wait did not say the node is still fetching: ${JSON.stringify(said)}`);
});

await t("**a node STILL fetching is re-asked on the doubling schedule from each slow answer, so a B holds few duplicate GETs, not one per RTO**", async () => {
  const c = clock();
  const slow = Array.from({ length: 6 }, () => ({ status: 503, ms: SLOW }));
  const net = slowNode(c, [...slow, { bytes: FILE, ms: 5 }]);
  await served({ url: "/p" }, { fetch: net.fetch, sleep: c.sleep, now: c.now });
  const gaps = net.sent.slice(1).map((at, i) => at - (net.sent[i] + SLOW));
  const want = Array.from({ length: 6 }, (_, i) => STILL_FETCHING_REASK_MS[Math.min(i, STILL_FETCHING_REASK_MS.length - 1)]);
  assert.deepEqual(gaps, want, `the waits after each slow answer were ${gaps}`);
  const firstAnswer = SLOW;
  const inB = net.sent.filter(at => at > 0 && at < firstAnswer + NODE_GET_BOUND_MS).length;
  assert.ok(inB <= 4, `${inB} re-asks within one B of the first 503: the duplicates are not few`);
});

await t("**(B) while every source is held the wait still TICKS on the RTO: onWait counts the seconds, never silent for a whole hold**", async () => {
  const c = clock();
  const net = slowNode(c, [{ status: 503, ms: SLOW }, { status: 503, ms: SLOW }, { bytes: FILE, ms: 5 }]);
  const said = [];
  await served({ url: "/p" }, { fetch: net.fetch, sleep: c.sleep, now: c.now, onWait: w => said.push({ at: c.now(), ...w }) });
  const secondAnswer = net.sent[1] + SLOW;
  const inHold = said.filter(w => w.at >= secondAnswer && w.at < net.sent[2]);
  assert.ok(inHold.length >= 3 && inHold.every(w => w.stillFetching), `the hold of ${STILL_FETCHING_REASK_MS[1]} ms was told ${inHold.length} times: ${JSON.stringify(inHold)}`);
  for (let i = 1; i < inHold.length; i += 1) {
    assert.ok(inHold[i].waitedMs > inHold[i - 1].waitedMs, "a tick did not count on");
    assert.ok(inHold[i].at - inHold[i - 1].at <= RTO_SCHEDULE_MS.at(-1), "a tick waited past the RTO's ceiling");
  }
  assert.equal(net.sent[2] - secondAnswer, STILL_FETCHING_REASK_MS[1], "the ticks moved the re-ask");
});

await t("**a FAST 503 (a node still joining: no GET of its own left running) is re-asked on the RTO, as before**", async () => {
  const c = clock();
  const net = slowNode(c, [{ status: 503, ms: 3 }, { bytes: FILE, ms: 5 }]);
  const said = [];
  await served({ url: "/p" }, { fetch: net.fetch, sleep: c.sleep, now: c.now, onWait: w => said.push(w) });
  assert.deepEqual(net.sent, [0, 3 + RTO_SCHEDULE_MS[0]], `a fast 503 was not re-asked on the RTO: asked at ${net.sent}`);
  assert.ok(!said[0].stillFetching, "a fast 503 was told as the node still fetching");
});

await t("THE BOUNDARY (the ruling's pair): an answer just UNDER the threshold is re-asked on the RTO; one AT it on the still-fetching schedule", async () => {
  for (const [ms, second] of [[STILL_FETCHING_AFTER_MS - 1, STILL_FETCHING_AFTER_MS - 1 + RTO_SCHEDULE_MS[0]], [STILL_FETCHING_AFTER_MS, STILL_FETCHING_AFTER_MS + FLOOR]]) {
    const c = clock();
    const net = slowNode(c, [{ status: 503, ms }, { bytes: FILE, ms: 1 }]);
    await served({ url: "/p" }, { fetch: net.fetch, sleep: c.sleep, now: c.now });
    assert.deepEqual(net.sent, [0, second], `a 503 after ${ms} ms: asked at ${net.sent}`);
  }
});

await t("a fast answer after slow ones starts the schedule over: that node has no GET left running", async () => {
  const c = clock();
  const net = slowNode(c, [{ status: 503, ms: SLOW }, { status: 503, ms: SLOW }, { status: 404, ms: 1 }, { status: 503, ms: SLOW }, { bytes: FILE, ms: 1 }]);
  await served({ url: "/p" }, { fetch: net.fetch, sleep: c.sleep, now: c.now });
  assert.equal(net.sent[4] - (net.sent[3] + SLOW), FLOOR, `after a fast answer the next slow one waited ${net.sent[4] - (net.sent[3] + SLOW)} ms, not the first wait`);
});

await t("the node's constants, derived (page/src/rto.rs, F64): the threshold just under the web bound; the waits a floor, then doubling from the bound to B", async () => {
  assert.equal(NODE_WEB_BOUND_MS, 30000, "the node's web bound moved: re-read path_handlers.rs at the bump");
  assert.ok(STILL_FETCHING_AFTER_MS < NODE_WEB_BOUND_MS && STILL_FETCHING_AFTER_MS > NODE_WEB_BOUND_MS * 0.9, `threshold ${STILL_FETCHING_AFTER_MS} is not just under the bound`);
  assert.ok(FLOOR < RTO_SCHEDULE_MS[0], `the first wait ${FLOOR} is not under the RTO's first`);
  const rest = STILL_FETCHING_REASK_MS.slice(1);
  assert.equal(rest[0], NODE_WEB_BOUND_MS, "the doubling does not start at the web bound");
  assert.equal(rest.at(-1), NODE_GET_BOUND_MS, "the waits do not end at B");
  for (let i = 1; i < rest.length; i += 1) assert.ok(rest[i] === Math.min(rest[i - 1] * 2, NODE_GET_BOUND_MS), `wait ${rest[i]} is not the doubling of ${rest[i - 1]}, capped at B`);
});

await t("two sources: a held one is skipped while the other is still asked on the RTO", async () => {
  const c = clock();
  const slow = slowNode(c, [{ status: 503, ms: NODE_WEB_BOUND_MS }]);
  const fast = slowNode(c, [{ status: 404, ms: 1 }, { status: 404, ms: 1 }, { bytes: FILE, ms: 1 }]);
  const fetch = u => (u === "/slow" ? slow.fetch() : fast.fetch());
  await served({ urls: ["/slow", "/fast"] }, { fetch, sleep: c.sleep, now: c.now });
  // /slow: asked at 0, its floor ended before the RTO round, asked again, then held for the web bound.
  assert.equal(slow.sent.length, 2, `the held source was asked ${slow.sent.length} times, not skipped in its hold`);
  assert.equal(fast.sent.length, 3, "the free source was not re-asked on the RTO");
});

if (failures) { process.stdout.write(`${failures} failed\n`); process.exit(1); }
