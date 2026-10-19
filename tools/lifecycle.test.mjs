// LIFECYCLES (`packages/lifecycle.js`, FOUNDATION.md §1, §3): moves only from the states the table allows, guards
// that say no refuse, one report per move — run with no node and no page:   node --test tools/lifecycle.test.mjs
// `LIFECYCLE_JS`: another copy of the file to test (the negative control: one whose `create` guard is removed lets a
// words login make a second group, and fails).
import test from "node:test";
import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";

const load = async p => import(p.startsWith("file:") ? p : pathToFileURL(p).href);
const faultMod = await load(new URL("../packages/fault.js", import.meta.url).href);
const lifeMod = await load(process.env.LIFECYCLE_JS ?? new URL("../packages/lifecycle.js", import.meta.url).href);
globalThis.dispatchEvent ??= () => true;
globalThis.CustomEvent ??= class CustomEvent {
  constructor(type, init) {
    this.type = type;
    this.detail = init?.detail;
  }
};
// THE GROUP's STEPS as the one list says them (`rules/src/steps.rs` — what `core.group_steps` returns): read from the Rust
// source itself, so this test runs on the real list.
import { readFileSync } from "node:fs";
const stepsRs = readFileSync(new URL("../rules/src/steps.rs", import.meta.url), "utf8");
const stepConsts = Object.fromEntries([...stepsRs.matchAll(/pub const ([A-Z_]+): &str = "([a-z-]+)";/g)].map(m => [m[1], m[2]]));
const groupSteps = [...stepsRs.matchAll(/\(("?[A-Za-z_-]+"?), (true|false)\)/g)].map(m => [m[1].startsWith('"') ? m[1].slice(1, -1) : stepConsts[m[1]], m[2] === "true"]);
const healths = [];
const ctx = {
  log: () => {},
  background: (w, p) => Promise.resolve(p).catch(() => {}),
  optional: (w, f, fb) => Promise.resolve().then(() => (typeof f === "function" ? f() : f)).catch(() => (typeof fb === "function" ? fb() : fb)),
  require: async name => {
    if (name === "fault") return fault;
    if (name === "index") return { health: async item => (healths.push(item), true) };
    if (name === "node") return { glue: { group_steps: () => JSON.stringify(groupSteps) } };
    throw new Error(`no ${name}`);
  },
};
const fault = await faultMod.start(ctx);
fault.reportTo(() => {});
const L = await lifeMod.start(ctx);
const tick = () => new Promise(r => setTimeout(r, 0));

test("a registration makes the group only on a channel answered absent", async () => {
  L.transition("account", "a", "login", { fresh: true });
  assert.throws(() => L.transition("account", "a", "create", { fresh: true, channel: "unknown" }), e => e.kind === "refused");
  assert.equal(L.state("account", "a").s, "CONFIRMING_CHANNEL", "a refused move leaves the state as it was");
  L.transition("account", "a", "create", { fresh: true, channel: "absent" });
  L.transition("account", "a", "made", { epoch: 0 });
  assert.equal(L.state("account", "a").s, "MEMBER");
});

test("a words login never makes a group — it walks a channel read with its info, or fails", () => {
  L.transition("account", "b", "login", { fresh: false });
  assert.throws(() => L.transition("account", "b", "create", { fresh: false, channel: "absent" }), e => e.kind === "refused");
  assert.throws(() => L.transition("account", "b", "walk", { fresh: false, channel: "present", info: false }), e => e.kind === "refused");
  L.transition("account", "b", "walk", { fresh: false, channel: "present", info: true });
  L.transition("account", "b", "join", { epoch: 3 });
  L.transition("account", "b", "joined", { epoch: 4 });
  assert.equal(L.state("account", "b").s, "MEMBER");
});

test("a move the table does not allow from here is a bug, thrown", () => {
  assert.throws(() => L.transition("account", "c", "joined", {}), e => e.kind === "bug");
  assert.throws(() => L.transition("account", "c", "nonsense", {}), e => e.kind === "bug");
  assert.equal(L.state("account", "c").s, "NONE");
});

test("one health report per move, with its from and to; listeners told; a snapshot of where each stands", async () => {
  healths.length = 0;
  const heard = [];
  L.on("session", (key, to, from) => heard.push(`${key}:${from}>${to}`));
  L.transition("session", "s", "keyless", { why: "keys not held" });
  L.transition("session", "s", "open", {});
  await tick();
  assert.deepEqual(healths.map(h => [h.k, h.to]), [["lifecycle-session", "GUEST>OPEN_KEYLESS"], ["lifecycle-session", "OPEN_KEYLESS>OPEN"]]);
  assert.deepEqual(heard, ["s:GUEST>OPEN_KEYLESS", "s:OPEN_KEYLESS>OPEN"]);
  assert.equal(L.snapshot().session.s, "OPEN");
});

test("a job runs only while its needs hold; paused (once) with why, resumed when they hold again", async () => {
  healths.length = 0;
  let ok = "nobody is logged in";
  let runs = 0;
  const j = L.job("t", { every: 10, needs: async () => ok, run: async () => runs++ });
  await new Promise(r => setTimeout(r, 40));
  assert.equal(runs, 0, "not run while its needs do not hold");
  assert.equal(L.state("job", "t").s, "PAUSED");
  ok = true;
  await new Promise(r => setTimeout(r, 40));
  assert.ok(runs >= 2, `run once its needs hold (${runs} runs)`);
  j.stop();
  const n = runs;
  await new Promise(r => setTimeout(r, 30));
  assert.equal(runs, n, "stopped: never run again");
  const moves = healths.filter(h => h.k === "lifecycle-job").map(h => h.to);
  assert.deepEqual(moves, ["IDLE>STOPPED"], "runs, pauses and resumes are quiet (the visit's snapshot has them); a stop is reported");
});

test("become moves only on a change", async () => {
  healths.length = 0;
  L.become("group", "x", "held");
  L.become("group", "x", "held");
  L.become("group", "x", "behind", { e: 2, oe: 3 });
  await tick();
  assert.deepEqual(healths.map(h => h.to), ["UNKNOWN>HELD", "HELD>BEHIND"]);
});

test("two ticks at once (a timer and now()) never move the job from a state they did not see", async () => {
  healths.length = 0;
  let runs = 0;
  const j = L.job("race", { every: 1000, needs: () => new Promise(r => setTimeout(() => r(true), 20)), run: async () => (runs++, new Promise(r => setTimeout(r, 30))) });
  await Promise.all([j.now(), j.now(), j.now()]);
  await new Promise(r => setTimeout(r, 80));
  j.stop();
  assert.ok(runs >= 1);
  assert.equal(healths.filter(h => /bug|refused/.test(h.k)).length, 0);
});

test("progress is the lifecycle's: one line per process in progress, its end said, then gone", async () => {
  const seen = [];
  const was = globalThis.dispatchEvent;
  globalThis.dispatchEvent = e => (e.type === "craftworks:progress" && seen.push(e.detail.lines), true);
  const has = re => seen.at(-1).some(l => re.test(l));
  try {
    // (Other tests' processes may still show an end line: counted from here.)
    L.transition("account", "p", "login", {});
    const base = seen.at(-1).length - 1;
    assert.ok(has(/1 of 4/), "the first stage is said");
    L.transition("account", "p", "walk", { channel: "present", info: true });
    assert.ok(has(/2 of 4/) && !has(/1 of 4/), "the next stage replaces it");
    L.transition("delete", "x", "request", { what: "a post" });
    assert.equal(seen.at(-1).length, base + 2, "two processes in progress: two lines");
    assert.ok(has(/Deleting a post/));
    L.transition("delete", "x", "gone", {});
    assert.equal(seen.at(-1).length, base + 1, "a process done leaves the pill");
    L.transition("account", "p", "join", {});
    L.transition("account", "p", "joined", { epoch: 1 });
    assert.ok(has(/4 of 4/), "how it ended is said");
    // A machine with no progress (a space's membership) never touches the pill.
    const n = seen.length;
    L.transition("group", "s", "held", {});
    assert.equal(seen.length, n);
  } finally {
    globalThis.dispatchEvent = was;
  }
});

// A REQUEST TO JOIN never hangs (owner 10-17): asked; then CANCELLED by the asker, REJECTED by one who may invite, or
// MEMBER (let in — or in by a code, the open door, a level — entered by itself). Every move reported, keyed by the space.
test("a request ends: asked, then cancelled, rejected or in — every move reported", async () => {
  const from = healths.length;
  L.transition("join", "s-cancel", "ask", { s: "5a5a5a" });
  L.transition("join", "s-cancel", "cancelled", {});
  L.transition("join", "s-reject", "ask", {});
  L.transition("join", "s-reject", "rejected", { why: "not now" });
  L.transition("join", "s-in", "ask", {});
  L.transition("join", "s-in", "joined", { why: "told" });
  // In by a code with no request first: straight to MEMBER.
  L.transition("join", "s-code", "joined", { why: "code" });
  await tick();
  assert.deepEqual(healths.slice(from).map(h => [h.s, h.to]), [
    ["s-cancel", "NONE>REQUESTED"],
    ["s-cancel", "REQUESTED>CANCELLED"],
    ["s-reject", "NONE>REQUESTED"],
    ["s-reject", "REQUESTED>REJECTED"],
    ["s-in", "NONE>REQUESTED"],
    ["s-in", "REQUESTED>MEMBER"],
    ["s-code", "NONE>MEMBER"],
  ]);
  // A request after its end is a new one.
  assert.equal(L.transition("join", "s-reject", "ask", {}), "REQUESTED");
  // The retired deciders' machine is gone: a move on it is a bug, never silent.
  assert.throws(() => L.transition("member", "q:x", "admit", {}), e => e.kind === "bug");
});

// A GROUP's STEP (a commit, a join — 10-14: an epoch moved twice and nothing said by what): kept and reported, the
// state unchanged; a routine step kept only (not reported); a step the table does not list is a bug, never silent.
test("a group's commit is a step: reported, the state kept; routine steps quiet", async () => {
  const g = "5a".repeat(32);
  const from = healths.length;
  L.transition("group", g, "held", { e: 4 });
  assert.equal(L.transition("group", g, "commit", { space: g, from: 4, to: 5, intent: "add" }), "HELD");
  L.transition("group", g, "log-open-written", { space: g, epoch: 5 });
  await tick();
  assert.deepEqual(healths.slice(from).map(h => [h.k, h.to]), [["lifecycle-group", "UNKNOWN>HELD"], ["lifecycle-group", "HELD>HELD"]]);
  assert.match(healths.at(-1).why, /^commit:/);
  assert.throws(() => L.transition("group", g, "made-up-step", {}), e => e.kind === "bug");
});

test("the group's steps are the one list's: the node's names known, an unknown one refused as a bug", () => {
  assert.ok(groupSteps.length > 40, `read ${groupSteps.length} steps`);
  for (const [name] of groupSteps) assert.ok(L.MACHINES.group.events[name], `${name} in the table`);
  assert.ok(L.MACHINES.group.quiet.includes("keys-in-step") && !L.MACHINES.group.quiet.includes("keys-reconciled"));
  L.transition("group", "s", "out-plan", { by: "node" });
  assert.throws(() => L.transition("group", "s", "follow-plan", {}), e => e.kind === "bug", "the old prefixed names are gone");
});

test("a step repeated with the same reason is recorded once; a different one, or another step between, again", async () => {
  const seen = [];
  L.on("group", (key, to, from, event, d) => key === "once" && seen.push(`${event}:${d.why}`));
  L.transition("group", "once", "keys-short", { why: "not whole" });
  L.transition("group", "once", "keys-short", { why: "not whole" });
  L.transition("group", "once", "keys-short", { why: "still not whole" });
  L.transition("group", "once", "keys-in-step", { why: "placed" });
  L.transition("group", "once", "keys-short", { why: "still not whole" });
  await tick();
  assert.deepEqual(seen, ["keys-short:not whole", "keys-short:still not whole", "keys-in-step:placed", "keys-short:still not whole"]);
});

test("a journal line is logged once per change (logOnce)", () => {
  assert.equal(L.logOnce("t", "k", "a"), true);
  assert.equal(L.logOnce("t", "k", "a"), false);
  assert.equal(L.logOnce("t", "k", "b"), true);
  assert.equal(L.logOnce("t", "other", "b"), true, "per key");
});
