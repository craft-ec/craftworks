// open() HANDS OVER A DB THAT CAN BE READ (sdk#234): when it provisions the
// node, it returns only once the node says it is provisioned — or ends on an
// ANSWER (refused; the connection refused twice, never opened) or a person
// CANCELLING. No time ends it (rule 8): a slow node is waited on, and how long
// it has not answered is reported. Measured on the notes site: a read straight
// after an open() that returned early failed as UNAVAILABLE every time.
import assert from "node:assert/strict";
import { open, untilProvisioned } from "../../js/session.js";

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.stack}\n`); }
};
const tick = (r, ms) => setTimeout(r, 0);

/** A session that is provisioned after `after` polls, or refuses, or never is. */
function FakeSession({ after = 3, refuse = "" } = {}) {
  let polls = 0;
  return function () {
    return {
      url: () => "ws://127.0.0.1:1/", outbound: () => [], sent() {}, on_inbound: () => true, unowned() {}, set_app() {}, adopt_loader() {},
      reconnected() {}, take_progress: () => "[]", take_loads: () => "[]", tick_ms: () => 1000, tick: () => "{}",
      unsaved_writes: () => 0, cold_due_ms: () => -1, cold_tick() {}, flush() {}, provision() {},
      provisioned: () => (polls += 1) > after && !refuse, refused: () => refuse,
      not_answering: () => JSON.stringify({ what: "the signer", ms: polls * 100 }),
      polls: () => polls,
    };
  };
}
const deps = extra => ({
  port: 7999, app: "test-app", artefacts: { signer: "s", block: "b", register: "r" },
  fetch: async () => ({ ok: true, arrayBuffer: async () => new Uint8Array([1]).buffer }),
  connect: () => ({ pump() {}, close() { deps.closed = (deps.closed ?? 0) + 1; } }),
  setInterval: () => 0, clearInterval() {}, setTimeout: tick, clearTimeout() {},
  addEventListener: null, removeEventListener: null, documentOf: null, provisionEveryMs: 0, ...extra,
});

await t("**open() returns only once the node says it is provisioned**", async () => {
  const S = FakeSession({ after: 3 });
  const h = await open(S, deps());
  assert.ok(h.session.polls() > 3, `open() returned after ${h.session.polls()} polls, before the node was provisioned`);
  assert.ok(h.db, "no db");
});

await t("**a RE-open re-subscribes (session.reconnected), the FIRST open does not** (sdk#376)", async () => {
  // On the first open there is no old connection whose reassembly or head
  // subscription could be gone; on every re-open there is, and the page must
  // read its head again with subscribe at once (page-io), not at the backstop.
  let reconnects = 0, fire = null;
  const Base = FakeSession({ after: 3 });
  const S = function () { const s = Base(); s.reconnected = () => { reconnects += 1; }; return s; };
  const h = await open(S, deps({ connect: (_engine, o) => { fire = o.onEvent; return { pump() {}, close() {} }; } }));
  assert.ok(fire, "THE SETUP: the socket's events were not captured");
  fire({ kind: "open" });
  assert.equal(reconnects, 0, "the FIRST open was treated as a reconnect");
  fire({ kind: "closed" });
  fire({ kind: "open" });
  assert.equal(reconnects, 1, "a RE-open did not re-subscribe: a live page would poll for up to 120 s");
  fire({ kind: "closed" });
  fire({ kind: "open" });
  assert.equal(reconnects, 2, "a second re-open did not re-subscribe");
  assert.ok(h.db, "no db");
});

await t("a REFUSAL ends it in the node's words, and the session is closed", async () => {
  deps.closed = 0;
  await assert.rejects(() => open(FakeSession({ refuse: "a second install" }), deps()), /the node refused to set up: a second install/);
  assert.equal(deps.closed, 1, "the socket of a failed open() was left open");
});

await t("**NO TIME ENDS IT: a signer silent for five minutes is waited on, says how long, and opens when it answers**", async () => {
  let polls = 0;
  const heard = [];
  const handle = {
    provisioned: () => (polls += 1) > 3_000,
    connectedOnce: () => true,
    notAnswering: () => ({ what: "the signer", ms: polls * 100 }),
  };
  await untilProvisioned(handle, { provisionEveryMs: 0, setTimeout: tick, onWaiting: w => heard.push(w.ms) });
  assert.ok(polls > 3_000, `it stopped waiting after ${polls} polls`);
  assert.ok(Math.max(...heard) >= 299_900, `it never said it had waited five minutes: ${Math.max(...heard)} ms`);
});

await t("**a person CANCELS: the wait ends, named — the one end that is not the node's**", async () => {
  const ac = new AbortController();
  let polls = 0;
  const p = untilProvisioned({ provisioned: () => { if ((polls += 1) === 50) ac.abort(); return false; }, connectedOnce: () => true },
    { provisionEveryMs: 0, setTimeout: tick, signal: ac.signal });
  await assert.rejects(p, /cancelled: setting up was stopped/);
  assert.ok(polls >= 50 && polls < 60, `cancelled at poll ${polls}`);
});

await t("**a node that is not there is named at once: two refused ATTEMPTS, never opened**", async () => {
  // Without the refusal end this would wait for ever: the assertion's own
  // guard is the test runner's.
  const frozen = { provisionEveryMs: 1, setTimeout: r => setTimeout(r, 0) };
  await assert.rejects(() => untilProvisioned({ provisioned: () => false, connectedOnce: () => false, refusedBeforeOpen: () => 2, url: () => "ws://127.0.0.1:7999/" }, frozen),
    /nothing answered at ws:\/\/127\.0\.0\.1:7999\/: the connection was refused and never opened — is the node running\?/);
  // THE CONTROL: one refused attempt is a node still starting — it waits.
  let polls = 0;
  const one = untilProvisioned({ provisioned: () => (polls += 1) > 3, connectedOnce: () => false, refusedBeforeOpen: () => 1 }, frozen);
  await one;
  assert.ok(polls > 3, "one refusal ended the wait");
  // Through open(), with a REAL refused socket's events: each attempt fires
  // `error` THEN `closed` (connection.js). ONE such attempt is a node still
  // starting: open() keeps waiting and succeeds once it is provisioned.
  const attempts = n => ({ connect: (_e, { onEvent } = {}) => { for (let i = 0; i < n; i += 1) { onEvent?.({ kind: "error" }); onEvent?.({ kind: "closed" }); } return { pump() {}, close() {} }; } });
  const h = await open(FakeSession({ after: 5 }), deps(attempts(1)));
  assert.ok(h.db, "one refused attempt (error + closed) ended open(): a node still starting is not a node that is not there");
  // TWO attempts: the node is not there, named.
  await assert.rejects(() => open(FakeSession({ after: 1e9 }), deps(attempts(2))), /the connection was refused and never opened/);
});

await t("THE CONTROL: provision: false does not wait — nothing is being set up", async () => {
  const S = FakeSession({ after: 1e9 });
  const h = await open(S, deps({ provision: false }));
  assert.equal(h.session.polls(), 0, "open() waited on provisioning it was told not to do");
});

if (failures) { process.stdout.write(`${failures} failed\n`); process.exit(1); }
