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
const healths = [];
const ctx = {
  log: () => {},
  background: (w, p) => Promise.resolve(p).catch(() => {}),
  optional: (w, f, fb) => Promise.resolve().then(() => (typeof f === "function" ? f() : f)).catch(() => (typeof fb === "function" ? fb() : fb)),
  require: async name => {
    if (name === "fault") return fault;
    if (name === "index") return { health: async item => (healths.push(item), true) };
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
  L.become("space", "x", "held");
  L.become("space", "x", "held");
  L.become("space", "x", "behind", { e: 2, oe: 3 });
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
