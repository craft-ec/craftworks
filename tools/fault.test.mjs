// FAULTS (`packages/fault.js`, FOUNDATION.md §4): kinds, the one retry/report table, `attempt`, `absentOnly`, what a
// person reads — run with no node and no page:   node --test tools/fault.test.mjs
// `FAULT_JS`: another copy of the file to test (the negative control: one whose `attempt` retries every kind, or
// reports a `never` kind, fails).
import test from "node:test";
import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";

const src = process.env.FAULT_JS ?? new URL("../packages/fault.js", import.meta.url).href;
const mod = await import(src.startsWith("file:") ? src : pathToFileURL(src).href);
const logs = [];
const F = await mod.start({ log: (step, d) => logs.push([step, d?.what]) });
const reports = [];
F.reportTo((k, source, what, d) => reports.push({ k, source, what, d }));
const fast = { transient: { wait: () => 0 }, "not-yet": { wait: () => 0 } };

test("anything thrown is classified: unknown errors are bugs; older shapes keep their meaning", () => {
  assert.equal(F.classify(new Error("boom")).kind, "bug");
  assert.equal(F.classify("a string").kind, "bug");
  assert.equal(F.classify(Object.assign(new Error("members only"), { expected: true })).kind, "refused-expected");
  assert.equal(F.classify(Object.assign(new Error("x"), { name: "AbortError" })).kind, "cancelled");
  const ext = new Error("wallet");
  ext.stack = "Error: wallet\n    at x (chrome-extension://abc/inpage.js:1:1)";
  assert.equal(F.classify(ext).kind, "extension");
  const f = new F.Fault("sealed", "reading notes", { d: { epochs: [3] } });
  assert.equal(F.classify(f), f, "a fault is itself");
  assert.throws(() => new F.Fault("nonsense", "x"), /no kind/);
});

test("attempt: a transient fault is tried again by the policy, reported once at the end with its tries", async () => {
  reports.length = 0;
  let n = 0;
  await assert.rejects(
    F.attempt("files", "putting a piece", async () => {
      n++;
      throw new F.Fault("transient", "putting a piece", { message: "no answer in 30 s", d: { ms: 30000 } });
    }, { policy: fast }),
    e => e.kind === "transient",
  );
  assert.equal(n, F.POLICY.transient.tries, "tried the policy's number of times");
  assert.equal(reports.length, 1, "reported once, not per try");
  assert.equal(reports[0].k, "transient");
  assert.equal(reports[0].what, "putting a piece", "keyed by the stable label, not the message");
  assert.equal(reports[0].d.tries, F.POLICY.transient.tries);
  assert.equal(reports[0].d.ms, 30000);
});

test("attempt: a transient fault that clears is not reported; a bug is never tried again", async () => {
  reports.length = 0;
  let n = 0;
  const got = await F.attempt("files", "putting a piece", async () => {
    if (++n < 3) throw new F.Fault("transient", "putting a piece");
    return "ok";
  }, { policy: fast });
  assert.equal(got, "ok");
  assert.equal(reports.length, 0);
  // CONTROL: a bug is not retried (the same attempt with a transient was, above).
  let b = 0;
  await assert.rejects(F.attempt("x", "doing", async () => (b++, Promise.reject(new Error("boom")))), e => e.kind === "bug");
  assert.equal(b, 1);
  assert.equal(reports.length, 1);
  assert.equal(reports[0].k, "bug");
});

test("never-reported kinds stay out of the reports; met reports by kind", () => {
  reports.length = 0;
  for (const k of ["refused-expected", "login-needed", "extension", "absent", "cancelled"]) F.met("page", new F.Fault(k, "x"));
  assert.equal(reports.length, 0);
  F.met("page", new F.Fault("refused", "putting a tree block"));
  assert.equal(reports.length, 1);
});

test("one failure met in many places is reported once", () => {
  reports.length = 0;
  const e = new Error("tables: its tree is sealed");
  F.met("page", e, { via: "page-rejection" });
  F.met("cards", e, { via: "package-failed" });
  F.met("label-menu", e, { via: "package-failed" });
  assert.equal(reports.length, 1);
  assert.equal(reports[0].d.via, "page-rejection");
  F.met("page", new Error("tables: its tree is sealed"));
  assert.equal(reports.length, 2, "another failure (another object) is reported");
});

test("absentOnly: null for absent or not yet; every other failure thrown on", () => {
  assert.equal(F.absentOnly(new F.Fault("absent", "x")), null);
  assert.equal(F.absentOnly(new F.Fault("not-yet", "x")), null);
  assert.throws(() => F.absentOnly(new F.Fault("sealed", "x")), e => e.kind === "sealed");
  assert.throws(() => F.absentOnly(new Error("boom")), e => e.kind === "bug");
});

test("say: one wording per kind; a refusal says its own words", () => {
  assert.equal(F.say(new F.Fault("login-needed", "nobody is logged in")), "Log in to do that.");
  assert.equal(F.say(new F.Fault("refused-expected", "members only", { message: "Chat is for its members" })), "Chat is for its members");
  assert.match(F.say(new Error("boom")), /Something went wrong: boom/);
});
