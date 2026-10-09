// The engine-backed database wrapper: promises, stable codes, and a read that
// actually waits for the data.
//
// No wasm and no socket. The fake session behaves the way the real one does —
// a NOT_LOADED carries a TICKET, the data arrives on a LATER TASK, and the
// ticket is only reported ended once it has. That last part is the whole
// point: the first version of this wrapper awaited a resolved promise and
// asked again in a microtask, before any message could have arrived, and its
// test passed because the fake scripted the second call to succeed.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { engineDb, DbError } from "../../js/engine-db.js";

let failures = 0;

/**
 * Every test gets a DEADLINE.
 *
 * The failure this exists for is not slowness. A wrapper that retries without
 * a bound does not fail — it HANGS, and a suite with no deadline cannot tell
 * that apart from a slow machine: it simply never finishes, and in CI that
 * reads as a stuck runner rather than as a broken retry.
 *
 * Found by mutation: removing the ticket check from `once` (so a NOT_LOADED
 * with nothing to wait on is retried for ever) wedged this file instead of
 * failing it. Retries and bounds are most of what this file tests, so a
 * missing bound has to be a FAILING test.
 */
const DEADLINE_MS = 5_000;
const t = async (name, fn) => {
  let timer;
  try {
    await Promise.race([
      fn(),
      new Promise((_, bad) => {
        timer = setTimeout(() => bad(new Error(
          `did not finish within ${DEADLINE_MS}ms — a retry with no bound hangs rather than failing`)), DEADLINE_MS);
      }),
    ]);
    process.stdout.write(`  ok  ${name}\n`);
  }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.stack?.split("\n")[0] ?? e}\n`); }
  finally { clearTimeout(timer); }
};

/**
 * A session whose data ARRIVES LATER.
 *
 * `afterMs` is when the load completes. Until then the read throws
 * NOT_LOADED with a ticket, exactly as the Rust side does, and `take_loads`
 * reports nothing. A wrapper that re-asks without waiting sees the second
 * NOT_LOADED and fails — which is the behaviour the old test could not see.
 */
const lateSession = ({ afterMs = 20, rows = [{ id: "a" }], fail = false, failCode = "UNAVAILABLE" } = {}) => {
  let loaded = false;
  let asks = 0;
  let requests = 0;
  let ended = [];
  let nextTicket = 1;
  const byRange = new Map();
  const s = {
    asks: () => asks,
    requests: () => requests,
    scan(domain = "tasks") {
      asks += 1;
      if (loaded) return JSON.stringify(rows);
      // ONE request per range, as the session does.
      let ticket = byRange.get(domain);
      if (ticket === undefined) {
        ticket = nextTicket++;
        byRange.set(domain, ticket);
        requests += 1;
        setTimeout(() => {
          loaded = !fail;
          ended.push({ id: ticket, ok: !fail, code: fail ? failCode : "LOADED" });
          s.pump();               // the page, woken by a message arriving
        }, afterMs);
      }
      const e = new Error("not loaded");
      e.code = "NOT_LOADED";
      e.transient = true;
      e.wait = ticket;
      throw e;
    },
    put() { return s.scan(); },
    create_at(domain = "tasks") { return s.scan(domain); },
    // A COLD DEFINE. It reads the existing schema before it can check the new
    // one against it, so until that range is loaded it fails exactly as a
    // read does — with a ticket.
    define(domain = "tasks") { s.scan(domain); return undefined; },
    update(domain = "tasks") { return s.scan(domain); },
    delete(domain = "tasks") { s.scan(domain); return true; },
    take_loads() { const out = ended; ended = []; return JSON.stringify(out); },
    // A woken read RESUMES its ticket before it asks again (READ-STATE: the
    // next walk is pinned to that ticket's root).
    resumed: [],
    resume(ticket) { s.resumed.push(ticket); },
    pump: () => {},
  };
  return s;
};

await t("a read waits for the data and then answers — TWO asks, one request", async () => {
  const s = lateSession({ afterMs: 20 });
  const db = engineDb(s);
  s.pump = db.drain;
  const rows = await db.scan("tasks");
  assert.deepEqual(rows, [{ id: "a" }]);
  assert.equal(s.asks(), 2, "the read did not ask exactly twice");
  assert.equal(s.requests(), 1, "the read issued more than one request for one range");
  assert.deepEqual(s.resumed, [1], "the woken read did not resume the ticket it waited on, so its next walk chases the head");
});

await t("CONCURRENT reads of one unloaded range issue ONE request", async () => {
  const s = lateSession({ afterMs: 20 });
  const db = engineDb(s);
  s.pump = db.drain;
  const all = await Promise.all([db.scan("tasks"), db.scan("tasks"), db.scan("tasks")]);
  for (const rows of all) assert.deepEqual(rows, [{ id: "a" }]);
  assert.equal(s.requests(), 1,
    "three components reading one range asked the node three times; that multiplies every first paint");
});

await t("THE CONTROL: a loaded read asks ONCE and requests nothing", async () => {
  const s = lateSession({ afterMs: 0 });
  const db = engineDb(s);
  s.pump = db.drain;
  await db.scan("tasks");                     // warm it
  const asksBefore = s.asks(), reqBefore = s.requests();
  const rows = await db.scan("tasks");
  assert.deepEqual(rows, [{ id: "a" }]);
  assert.equal(s.asks() - asksBefore, 1, "a loaded read asked twice");
  assert.equal(s.requests() - reqBefore, 0,
    "a loaded read requested a range it already had; without this the test above proves nothing");
});

await t("a load that ENDS without delivering rejects — it is not 'not yet'", async () => {
  const s = lateSession({ afterMs: 10, fail: true });
  const db = engineDb(s);
  s.pump = db.drain;
  await assert.rejects(() => db.scan("tasks"), e => {
    assert.ok(e instanceof DbError);
    assert.equal(e.code, "UNAVAILABLE");
    return true;
  });
});


await t("**a SUPERSEDED load restarts the whole chain, UNPINNED**: nothing resumed, asked again at the head (#330 ruling)", async () => {
  // The first ticket ends SUPERSEDED (a newer head while it waited on a block nobody serves); the chain asks again
  // from its first call, gets a NEW ticket at the head, and that one loads.
  let asks = 0, ticket = 0, loaded = false, ended = [];
  const s = {
    scan() {
      asks += 1;
      if (loaded) return JSON.stringify([{ id: "at the head" }]);
      ticket += 1;
      const mine = ticket;
      setTimeout(() => {
        if (mine === 1) ended.push({ id: 1, ok: false, code: "SUPERSEDED" });
        else { loaded = true; ended.push({ id: mine, ok: true, code: "LOADED" }); }
        s.pump();
      }, 5);
      throw Object.assign(new Error("not loaded"), { code: "NOT_LOADED", transient: true, wait: mine });
    },
    take_loads() { const out = ended; ended = []; return JSON.stringify(out); },
    resumed: [],
    resume(tk) { s.resumed.push(tk); },
    pump: () => {},
  };
  const db = engineDb(s);
  s.pump = db.drain;
  assert.deepEqual(await db.scan("tasks"), [{ id: "at the head" }], "a superseded read did not answer at the head");
  assert.equal(asks, 3, `asks ${asks}: the first, the restart, and the one after its load`);
  assert.deepEqual(s.resumed, [2], "the SUPERSEDED ticket was resumed (pinned to the old root) — or the new one was not");
});

await t("a NOT_LOADED with no ticket rejects rather than hanging", async () => {
  const s = {
    scan() { const e = new Error("x"); e.code = "NOT_LOADED"; throw e; },
    take_loads: () => "[]",
  };
  const db = engineDb(s);
  await assert.rejects(() => db.scan("tasks"), e => e.code === "NOT_LOADED");
});

await t("an error a load cannot fix is NOT waited on", async () => {
  let asks = 0;
  const s = {
    scan() { asks += 1; const e = new Error("too big"); e.code = "TOO_LARGE"; e.transient = false; throw e; },
    take_loads: () => "[]",
  };
  const db = engineDb(s);
  await assert.rejects(() => db.scan("tasks"), e => e.code === "TOO_LARGE");
  assert.equal(asks, 1, "a TOO_LARGE was retried; no load makes a record smaller");
});

// ---------------------------------------------------------------------------
// A WRITE THAT COULD NOT READ IS NOT AN INVALID WRITE.
//
// This block replaces a test that asserted the opposite — "a WRITE is never
// retried, whatever it says" — whose stated fear was that a retried write can
// be applied twice. That fear is answered rather than ignored: in `Db` every
// read a write makes happens BEFORE the single write that mutates, and that
// write applies the whole edit or none of it. A NOT_LOADED therefore leaves
// the tree untouched, so asking again repeats the attempt and not the effect.
//
// What the old rule cost (sdk#89): a cold `define` returned a ticketless
// NOT_LOADED that nothing could retry, and a published app kept a form on
// screen, a button saying "Published", and refused every write with `domain
// has no schema; define it first` — permanently, because the next mount ran
// the same cold define. Measured against a real node: 120 of 120 refused.
// ---------------------------------------------------------------------------

await t("**a COLD DEFINE succeeds, with no preload**", async () => {
  // The user-visible bug, as a test. Nothing warms the range first: the
  // define itself queues the load, waits for it, and asks once more.
  const s = lateSession({ afterMs: 20 });
  const db = engineDb(s);
  s.pump = db.drain;
  await db.define("tasks", { type: "Task", fields: [] });
  assert.equal(s.asks(), 2, "the define did not ask exactly twice");
  assert.equal(s.requests(), 1, "it issued more than one request for one range");
});

await t("a cold PUT, CREATE_AT, UPDATE and DELETE do the same — they all read first", async () => {
  // `need_schema` is on all three paths, so none of them is a special case.
  for (const [name, call] of [
    ["put", db => db.put("tasks", {})],
    // sdk#149: a cold slot is NOT absent — createAt waits to read it too.
    ["createAt", db => db.createAt("tasks", "a", {})],
    ["update", db => db.update("tasks", "a", {})],
    ["delete", db => db.delete("tasks", "a")],
  ]) {
    const s = lateSession({ afterMs: 10 });
    const db = engineDb(s);
    s.pump = db.drain;
    await call(db);
    assert.equal(s.asks(), 2, `a cold ${name} did not wait for the range it needed`);
  }
});

await t("a load that never answers ends the write UNAVAILABLE, bounded", async () => {
  // Not a hang and not an infinite retry: the load ENDS, having failed, and
  // the write is told so.
  const s = lateSession({ afterMs: 10, fail: true });
  const db = engineDb(s);
  s.pump = db.drain;
  await assert.rejects(
    () => db.define("tasks", { type: "Task", fields: [] }),
    e => e.code === "UNAVAILABLE",
    "a define whose range never loaded did not end UNAVAILABLE");
  assert.ok(s.asks() <= 2, `it asked ${s.asks()} times for a range that was never going to load`);
});

await t("THE CONTROL: an INVALID write is still refused, and NOT retried", async () => {
  // The half of the old rule that was right, kept. Without this, routing
  // writes through the retry would look correct while quietly re-sending a
  // write that no load can ever make valid.
  // IT CARRIES A TICKET. That is the whole point of the control: with no
  // ticket the retry stops for a second reason, and the test would pass
  // whether or not the wrapper still checks the code at all. Found by
  // mutation — deleting the code check left this green.
  let asks = 0;
  const s = {
    define() {
      asks += 1;
      const e = new Error("field `title` must be text");
      e.code = "REFUSED";
      e.transient = false;
      e.wait = 1;              // something TO wait on, deliberately
      throw e;
    },
    take_loads: () => JSON.stringify([{ id: 1, ok: true, code: "LOADED" }]),
  };
  const db = engineDb(s);
  s.pump = db.drain;
  await assert.rejects(() => db.define("tasks", { type: "Task", fields: [] }), e => e.code === "REFUSED");
  assert.equal(asks, 1,
    "an invalid write was re-sent although its code says no load can help; " +
    "the wrapper is deciding on the ticket rather than on what went wrong");
});

await t("and the retry is BOUNDED even when every attempt hands back a ticket", async () => {
  // A session that always says "not loaded, wait for this" and always
  // completes the load — progress on every hop, and never an answer. Nothing
  // but MAX_HOPS stops it.
  //
  // Without this, removing the hop bound was caught by no test at all: the
  // other fakes stop issuing NOT_LOADED once loaded, so the loop ends for a
  // reason that has nothing to do with the bound.
  let asks = 0, ticket = 0;
  const ended = [];
  const s = {
    define() {
      asks += 1;
      ticket += 1;
      const t = ticket;
      setTimeout(() => { ended.push({ id: t, ok: true, code: "LOADED" }); s.pump(); }, 1);
      const e = new Error("not loaded");
      e.code = "NOT_LOADED"; e.transient = true; e.wait = t;
      throw e;
    },
    take_loads() { const out = ended.splice(0); return JSON.stringify(out); },
    resume() {},
    pump: () => {},
  };
  const db = engineDb(s);
  s.pump = db.drain;
  await assert.rejects(
    () => db.define("tasks", { type: "Task", fields: [] }),
    e => e.code === "NOT_LOADED",
    "a write that is always told to wait never gave up");
  assert.ok(asks <= 10,
    `it asked ${asks} times; the hop bound is what stops a chain that makes ` +
    "progress for ever, and an unbounded one hangs rather than fails");
});

await t("THE CONTROL: a ticketless NOT_LOADED on a write still surfaces", async () => {
  // `park` hands back no ticket when the span was already loaded and the read
  // still cannot be answered — asking again would answer identically. The
  // write must be told, not sent round.
  let asks = 0;
  const s = {
    define() { asks += 1; const e = new Error("not loaded"); e.code = "NOT_LOADED"; e.transient = true; throw e; },
    take_loads: () => "[]",
  };
  const db = engineDb(s);
  await assert.rejects(() => db.define("tasks", { type: "Task", fields: [] }), e => e.code === "NOT_LOADED");
  assert.equal(asks, 1, "a write with no ticket to wait on was retried anyway");
});

await t("a write the store REFUSED rejects with its code, whether it may be retried, and the bound it met — never resolves as made (sdk#180)", async () => {
  // Rust built this: `{ code, message, transient, retryable, cap }`. The
  // wrapper must carry every field a caller acts on.
  let asks = 0;
  const refusal = { code: "TOO_LARGE_TO_SEND", message: "this write is 9 bytes as sent and the limit is 8", transient: false, retryable: false };
  const s = { create_at() { asks += 1; throw { ...refusal }; }, put() { asks += 1; throw { ...refusal }; },
              update() { asks += 1; throw { ...refusal }; }, delete() { asks += 1; throw { ...refusal }; } };
  const db = engineDb(s);
  for (const [name, call] of [["createAt", () => db.createAt("tasks", "a".repeat(32), {})], ["put", () => db.put("tasks", {})],
                              ["update", () => db.update("tasks", "a".repeat(32), {})], ["delete", () => db.delete("tasks", "a".repeat(32))]]) {
    await assert.rejects(call, e => {
      assert.equal(e.name, "DbError", `${name}: not a DbError`);
      assert.equal(e.code, "TOO_LARGE_TO_SEND", `${name}: the code`);
      assert.equal(e.retryable, false, `${name}: retryable was lost`);
      assert.equal(e.transient, false, `${name}: a reload does not make it smaller`);
      return true;
    }, `${name} resolved for a write the store refused`);
  }
  assert.equal(asks, 4, "a refused write was re-sent by the wrapper");
});

await t("a FULL QUEUE is backpressure: the write waits for the session's wake and is made again (R-b; COMMIT-LIFE K1)", async () => {
  // QUEUE_FULL is not a verdict on the write: nothing was written, and the
  // same write made again once a commit publishes is taken. The wrapper waits
  // on the session's own wake -- never a timer -- and asks again.
  let asks = 0, wake;
  const full = { code: "QUEUE_FULL", message: "4000 of 4096 bytes of writes are already waiting to be saved; wait for them, then try again", transient: false, retryable: true, cap: 4096 };
  const s = { put() { asks += 1; if (asks < 3) throw { ...full }; return JSON.stringify({ id: "x" }); }, take_loads: () => "[]" };
  const db = engineDb({ session: s, onReadsWake: fn => { wake = fn; } });
  let done = false;
  const p = db.put("tasks", {}).then(r => { done = true; return r; });
  await new Promise(r => setImmediate(r));
  assert.equal(asks, 1, "the first ask was not made");
  assert.equal(done, false, "a write met a full queue and resolved without waiting");
  wake();
  await new Promise(r => setImmediate(r));
  assert.equal(asks, 2, "the wake did not make the write again");
  wake();
  const r = await p;
  assert.equal(asks, 3, "the write was not made again once the queue had room");
  assert.deepEqual(r, { id: "x" });
});

await t("**NOT DECIDED YET waits for the signer's answer -- no deadline, not even the app's -- then takes the answered branch** (#342; rule 8)", async () => {
  // The node's signer has not said whose node this is (asking, or silent):
  // a define (or a write) is NOT DECIDED, not refused. The wrapper waits on
  // the session's own wake, past any deadline, and makes the SAME call again;
  // when the answer comes, the call takes its branch.
  let clock = 0, wake, answered = null, asks = 0;
  const undecided = { code: "NOT_DECIDED", message: "not decided yet: asking this node's signer whose node it is", transient: true };
  const s = {
    define() {
      asks += 1;
      if (answered === null) throw { ...undecided };
      if (answered === "view, different") throw { code: "REFUSED", message: "read-only: `notes` is not defined like that here", transient: false };
      return undefined;
    },
    take_loads: () => "[]",
  };
  const db = engineDb({ session: s, onReadsWake: fn => { wake = fn; } }, { writeDeadlineMs: 5_000, now: () => clock });
  let failed, done = false;
  const p = db.define("notes", { type: "Note", fields: [] }).then(() => { done = true; }, e => { failed = e; });
  for (const at of [4_999, 5_000, 60_000, 3_600_000]) {
    await new Promise(r => setImmediate(r));
    clock = at;
    wake();
  }
  await new Promise(r => setImmediate(r));
  assert.equal(failed, undefined, `an undecided define ended after ${clock} ms: ${failed?.code}`);
  assert.equal(done, false, "an undecided define resolved before any answer");
  assert.ok(asks >= 4, `it was not asked again on each wake (${asks})`);
  // The answer: the user's own tree -- the define is taken.
  answered = "own";
  wake();
  await p;
  assert.equal(failed, undefined);
  assert.equal(done, true, "the answered define did not resolve");

  // A VIEW whose published schema is different: the answered branch refuses, by name.
  answered = null;
  const q = db.define("notes", { type: "Note", fields: [] });
  await new Promise(r => setImmediate(r));
  answered = "view, different";
  wake();
  await assert.rejects(q, e => e.code === "REFUSED" && /not defined like that/.test(e.message));
});

await t("past the app's deadline a full queue fails NAMED, with the limit and how long it waited", async () => {
  let clock = 0, wake;
  const full = { code: "QUEUE_FULL", message: "4000 of 4096 bytes of writes are already waiting to be saved; wait for them, then try again", transient: false, retryable: true, cap: 4096 };
  const s = { put() { throw { ...full }; }, take_loads: () => "[]" };
  const db = engineDb({ session: s, onReadsWake: fn => { wake = fn; } }, { writeDeadlineMs: 5_000, now: () => clock });
  const p = assert.rejects(() => db.put("tasks", {}), e => {
    assert.equal(e.code, "QUEUE_FULL");
    assert.equal(e.retryable, true, "the app cannot tell to try again later");
    assert.equal(e.cap, 4096, "the limit was lost");
    assert.ok(e.message.includes("waited"), `not told it waited: ${e.message}`);
    return true;
  });
  await new Promise(r => setImmediate(r));
  clock = 4_999;
  wake();
  await new Promise(r => setImmediate(r));
  clock = 5_000;
  wake();
  await p;
});

await t("with NO deadline passed a full queue waits for room however long it takes, and says it is waiting (the owner: no limit, managed by the core)", async () => {
  let clock = 0, wake, room = false;
  const full = { code: "QUEUE_FULL", message: "full", transient: false, retryable: true, cap: 4096 };
  const s = { put() { if (!room) throw { ...full }; return JSON.stringify({ id: "x" }); }, take_loads: () => "[]" };
  const db = engineDb({ session: s, onReadsWake: fn => { wake = fn; } }, { now: () => clock });
  let failed;
  const p = db.put("tasks", {}).catch(e => { failed = e; });
  for (const at of [59_999, 60_000, 3_600_000, 86_400_000]) {
    await new Promise(r => setImmediate(r));
    clock = at;
    wake();
  }
  await new Promise(r => setImmediate(r));
  assert.equal(failed, undefined, `the SDK chose a deadline of its own: failed after ${clock} ms`);
  assert.equal(db.waitingForRoom(), 1, "a write waiting for room is not shown as waiting");
  room = true;
  wake();
  await p;
  assert.equal(failed, undefined);
  assert.equal(db.waitingForRoom(), 0, "a taken write is still shown as waiting");
});

await t("writes waiting for room are made again in the ORDER they waited (review sec. 2 on sdk#295)", async () => {
  // The engine holds a session told QUEUE_FULL until its queue is empty;
  // then the FIRST write made again is taken. That must be the oldest.
  let wake, room = false;
  const made = [];
  const full = { code: "QUEUE_FULL", message: "full", transient: false, retryable: true, cap: 4096 };
  const s = { put(_d, row) { if (!room) throw { ...full }; made.push(JSON.parse(row).v); return JSON.stringify({ id: JSON.parse(row).v }); }, take_loads: () => "[]" };
  const db = engineDb({ session: s, onReadsWake: fn => { wake = fn; } });
  const ps = ["a", "b", "c"].map(v => db.put("tasks", { v }));
  await new Promise(r => setImmediate(r));
  assert.equal(db.waitingForRoom(), 3);
  room = true;
  wake();
  await Promise.all(ps);
  assert.deepEqual(made, ["a", "b", "c"], "a later write was made before an older one that waited");
});

await t("nothing in the wrapper branches on a message, or waits on a timer", () => {
  const src = readFileSync(new URL("../../js/engine-db.js", import.meta.url), "utf8");
  const code = src.split("\n").filter(l => !l.trim().startsWith("//"));
  const onMessage = code.filter(l =>
    /\.message\s*(===|==|!==|!=)/.test(l) || /\.message\.(includes|startsWith|match|indexOf)/.test(l));
  assert.deepEqual(onMessage, [], `these branch on a message:\n${onMessage.join("\n")}`);
  // A timer here would either spin or answer late, and neither is a fact
  // about the data. The wake-up is an event.
  const timers = code.filter(l => /setTimeout|setInterval|Promise\.resolve\(\)/.test(l));
  assert.deepEqual(timers, [], `these wait on a clock instead of on the answer:\n${timers.join("\n")}`);
  assert.ok(/\.code\s*!==\s*"NOT_LOADED"/.test(src), "the wrapper does not branch on a code at all");
});

// ---- REPAIR (sdk#479): the whole tree through the normal read, then the put-backs' answers ----------------------
// A session whose whole-tree pages ARRIVE LATER (the first a NOT_LOADED with a ticket, as the Rust side answers), and
// whose repair PUTs are answered on a later wake. `outcome` comes from the session (the SDK's one derivation), never
// from the wrapper.
const repairSession = ({ pages = 3, pending = 2 } = {}) => {
  let ended = [];
  let loaded = false;
  let owed = pending;
  const s = {
    asked: [],
    putBack: [],
    scan_all(after, limit, putBack) {
      s.asked.push(after);
      s.putBack.push(putBack);
      if (!loaded) {
        setTimeout(() => { loaded = true; ended.push({ id: 1, ok: true, code: "LOADED" }); s.wake(); }, 5);
        const e = new Error("not loaded");
        e.code = "NOT_LOADED"; e.transient = true; e.wait = 1;
        throw e;
      }
      const n = after === "" ? 0 : Number(after);
      return JSON.stringify({ rows: 10, next: n + 1 < pages ? String(n + 1) : null });
    },
    repair_report(cancelled, check) {
      return JSON.stringify({ outcome: cancelled ? "cancelled" : check ? "DEGRADED" : owed ? "partial" : "repaired", missing: 2, putBack: pending - owed, rejected: 0, givenUp: 0, parityMismatched: 0, pending: owed, damaged: [], why: null });
    },
    answerPuts() { owed = 0; s.wake(); },
    take_loads() { const out = ended; ended = []; return JSON.stringify(out); },
    resume() {},
    onReadsWake(cb) { s.wake = cb; },
    wake: () => {},
  };
  return s;
};

await t("repairAll reads every page of the tree, waits for its put-backs' answers, and returns the session's report", async () => {
  const s = repairSession({ pages: 3, pending: 2 });
  const db = engineDb(s);
  let done = false;
  const run = db.repairAll({ limit: 10 }).then(r => { done = true; return r; });
  await new Promise(r => setTimeout(r, 30));
  assert.deepEqual(s.asked, ["", "", "1", "2"], "the pages were not read in order, the first after its load");
  assert.equal(done, false, "repairAll resolved before its put-backs were answered");
  s.answerPuts();
  const r = await run;
  assert.equal(r.rows, 30);
  assert.equal(r.outcome, "repaired", "the outcome is not the session's word");
  assert.equal(r.putBack, 2);
});

await t("checkAll reads every page with putBack false, waits for no put-back, and returns the session's check word", async () => {
  const s = repairSession({ pages: 2, pending: 2 });
  const db = engineDb(s);
  const r = await db.checkAll({ limit: 10 });
  assert.deepEqual(s.putBack.filter((_, i) => i > 0), [false, false], "a check page asked the session to put back");
  assert.equal(r.rows, 20);
  assert.equal(r.outcome, "DEGRADED", "the outcome is not the session's check word");
});

await t("repairAllCancel stops a running pass: it resolves CANCELLED with the counts so far", async () => {
  const s = repairSession({ pages: 2, pending: 2 });
  const db = engineDb(s);
  const run = db.repairAll({ limit: 10 });
  await new Promise(r => setTimeout(r, 30));
  db.repairAllCancel();
  const r = await run;
  assert.equal(r.outcome, "cancelled");
  assert.equal(r.pending, 2, "the counts so far were not reported");
});

process.stdout.write(failures ? `\n${failures} failing\n` : "\nall passing\n");
process.exit(failures ? 1 : 0);
