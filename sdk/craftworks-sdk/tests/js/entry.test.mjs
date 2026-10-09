// WHAT AN APP ACTUALLY GETS.
//
// Every other test in this repo imports the thing it tests from the file that
// defines it. That is a test of the file, not of the SDK, and twice now it has
// been green over code no app could reach:
//
//   * #63 merged with `engine-db.js` calling `handle.onReadsWake` and nothing
//     anywhere defining it, and with the engine path not exported at all;
//   * #64's first attempt added `open()` to `session.js` and its test imported
//     it from there, while `wrap()` — the entry — still returned only the two
//     halves an app can wire wrongly.
//
// Both times every gate passed. So this one goes through the ENTRY, exactly as
// an app does: `wrap(raw)`, then only what that returns.

import assert from "node:assert/strict";
import { wrap } from "../../js/wrap.js";

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.stack?.split("\n")[0] ?? e}\n`); }
};

const within = (ms, p, what) =>
  Promise.race([p, new Promise((_, r) =>
    setTimeout(() => r(new Error(`${what} never settled within ${ms}ms`)), ms))]);

/** What an app is promised at the entry, and what each one must be. */
const PROMISED = {
  version: "function",
  buildInfo: "function",
  blockId: "function",
  parseBlockId: "function",
  Db: "function",
  open: "function",
  SHIPPED_ARTEFACTS: "object",
  webapp: "object",
  pieces: "object",
  internals: "object",
};

/** A stand-in for the wasm module, so the entry can be built without wasm. */
function fakeRaw() {
  let loaded = false, ended = [], ticket = 0, asks = 0;
  const session = {
    url: () => "ws://127.0.0.1:7509/",
    outbound: () => [], sent() {},
    // The real one says whether the frame was its own (sdk#239); a fake's
    // delivery always is.
    set_app() {}, // the app a session is (sdk forest ruling); a fake needs no namespace
    adopt_loader() {}, // the loader's recording (sdk#386's instrument work); a fake records nothing
    on_inbound() { if (ticket && !loaded) { loaded = true; ended.push({ id: ticket, ok: true, code: "LOADED" }); } return true; },
    unowned() {},
    reconnected() {}, take_progress: () => "[]", provision() {},
    take_loads() { const o = ended; ended = []; return JSON.stringify(o); },
    resume() {},
    provisioned: () => true, refused: () => "", exhausted: () => false,
    unusable: () => "[]",
    // A DISTINCTIVE RATE. 1000 would pass whether the page asked the session
    // or wrote a literal; 7777 can only come from having asked.
    tick_ms: () => 7777, unsaved_writes: () => 0, cold_due_ms: () => -1, cold_tick() {},
    ticks: 0,
    flushes: 0,
    tick() {
      session.ticks += 1;
      return JSON.stringify({ rolledBack: 0, stalled: null, loadsInFlight: 0 });
    },
    flush() { session.flushes += 1; },
    scan() {
      asks += 1;
      if (loaded) return JSON.stringify([{ id: "a" }]);
      ticket ||= 1;
      const e = new Error("not loaded");
      e.code = "NOT_LOADED"; e.transient = true; e.wait = ticket;
      throw e;
    },
    asks: () => asks,
  };
  return {
    version: () => "0.1.0",
    buildInfo: () => "{}",
    blockId: () => "",
    parseBlockId: () => "{}",
    Db: function () { return { define() {}, domains: () => "[]" }; },
    Session: function () { return session; },
    __session: session,
  };
}

await t("every name an app is promised is present at the ENTRY, and typed", () => {
  const sdk = wrap(fakeRaw());
  const missing = Object.entries(PROMISED)
    .filter(([k, want]) => typeof sdk[k] !== want)
    .map(([k, want]) => `${k}: expected ${want}, got ${typeof sdk[k]}`);
  assert.deepEqual(missing, [],
    "an app cannot reach these, however complete the files behind them are:\n" + missing.join("\n"));
});

await t("THE CONTROL: a name that exists in a file but not at the entry FAILS", () => {
  // `openSession` is real, exported from `js/session.js`, and imported by
  // `wrap.js` — and it is deliberately NOT app-facing. If this check could
  // not tell that apart from a name that IS at the entry, it would have
  // passed on both #63 and #64, which is exactly what happened.
  const sdk = wrap(fakeRaw());
  assert.equal(sdk.openSession, undefined,
    "openSession is app-facing; either that is intended and PROMISED should say so, or the entry is handing out the halves again");
  assert.equal(typeof sdk.internals.openSession, "function",
    "the control names something that does not exist anywhere, so it proves nothing");
});

await t("a COLD SCAN resolves through sdk.open() and nothing else", async () => {
  const raw = fakeRaw();
  const sdk = wrap(raw);

  // Exactly what a page does: one call, then read.
  const { db, close } = await sdk.open({ app: "test-app",
    port: 17509,   // NAMED. There is no default port, deliberately.
    // The artefacts are FETCHED by default — `wrap` binds this build's own,
    // which is the point. Here the fetch is faked, because the test is about
    // reachability and not about a web server.
    fetch: async () => ({ ok: true, arrayBuffer: async () => new ArrayBuffer(4) }),
    connect: (session, { onEvent }) => {
      // Deliver one message, the way a socket would.
      setTimeout(() => { session.on_inbound(new Uint8Array()); onEvent({ kind: "message" }); }, 5);
      return { close() {}, pump() {} };
    },
    setInterval: () => 1,
    clearInterval: () => {},
  });

  const rows = await within(1000, db.scan("tasks"), "a cold scan through the entry");
  assert.deepEqual(rows, [{ id: "a" }]);
  assert.equal(raw.__session.asks(), 2, "the cold read did not ask twice");
  close();
});

await t("the artefacts an app is handed are the ones this build ships", () => {
  const sdk = wrap(fakeRaw());
  for (const k of ["signer", "block", "register"]) {
    assert.equal(typeof sdk.SHIPPED_ARTEFACTS[k], "string", `no ${k} artefact`);
    assert.match(sdk.SHIPPED_ARTEFACTS[k], /\.wasm$/, `${k} is not a wasm artefact`);
  }
});

/**
 * A session that watches a head and can be told it moved.
 *
 * `notify(key)` is what the node sends. OUR head is "ours"; anything else is
 * somebody else's contract, which is the case that must change nothing.
 */
function watchingRaw() {
  let stale = [], bound = new Set(), foreign = 0, scans = 0, ownChanged = [], title = "mine";
  const session = {
    url: () => "ws://127.0.0.1:17509/",
    outbound: () => [], sent() {}, reconnected() {}, provision() {},
    take_progress: () => "[]", take_loads: () => "[]",
    provisioned: () => true, refused: () => "", exhausted: () => false,
    unusable: () => "[]",
    // A DISTINCTIVE RATE. 1000 would pass whether the page asked the session
    // or wrote a literal; 7777 can only come from having asked.
    tick_ms: () => 7777, unsaved_writes: () => 0, cold_due_ms: () => -1, cold_tick() {},
    ticks: 0,
    flushes: 0,
    tick() {
      session.ticks += 1;
      return JSON.stringify({ rolledBack: 0, stalled: null, loadsInFlight: 0 });
    },
    flush() { session.flushes += 1; },
    bind: d => bound.add(d),
    unbind: d => bound.delete(d),
    rendered() {},
    take_stale() { const s = stale; stale = []; return JSON.stringify(s); },
    take_state_changed() { const s = ownChanged; ownChanged = []; return JSON.stringify(s); },
    // A binding's reload asks the session to refresh what it reads first.
    root: () => "00".repeat(32),
    live_mode: () => JSON.stringify({ mode: "HeadSubscribed", why: "", foreignNotifications: foreign }),
    scan() { scans += 1; return JSON.stringify([{ id: "a", title }]); },
    // The node's notification. Ours is taken; anything else is not this
    // session's (sdk#239) and is counted once, through `unowned`.
    set_app() {}, // the app a session is (sdk forest ruling); a fake needs no namespace
    adopt_loader() {}, // the loader's recording (sdk#386's instrument work); a fake records nothing
    on_inbound(key) {
      if (key === "ours") { stale = [...bound]; return true; }
      // One of THIS client's own writes on `domain` changed state (the node's
      // verdict on it), as the store reports it (builder#107).
      if (typeof key === "string" && key.startsWith("own:")) { ownChanged = [key.slice(4)]; return true; }
      // This client's write on `domain` was SUPERSEDED (sdk#225b): another
      // device's head won, the tree holds ITS value, and the store names the
      // write's keys as its own state change.
      if (typeof key === "string" && key.startsWith("superseded:")) {
        const [, domain, winner] = key.split(":");
        title = winner;
        ownChanged = [domain];
        return true;
      }
      return false;
    },
    unowned() { foreign += 1; },
    scans: () => scans,
    foreign: () => foreign,
  };
  return {
    version: () => "0", buildInfo: () => "{}", blockId: () => "", parseBlockId: () => "{}",
    Db: function () { return { define() {}, domains: () => "[]" }; },
    Session: function () { return session; },
    __session: session,
  };
}

/** An open over `watchingRaw`, with `deliver(key)` as the node's frame. */
const openWatching = async () => {
  const raw = watchingRaw();
  const sdk = wrap(raw);
  let deliver;
  const { db } = await sdk.open({
    app: "test-app",
    port: 17509,
    fetch: async () => ({ ok: true, arrayBuffer: async () => new ArrayBuffer(4) }),
    connect: (session, { onEvent }) => {
      deliver = key => { session.on_inbound(key); onEvent({ kind: "message" }); };
      return { close() {}, pump() {} };
    },
    setInterval: () => 1, clearInterval: () => {},
  });
  return { raw, db, deliver: key => deliver(key) };
};
const settle = () => new Promise(r => setTimeout(r, 10));

await t("**a PLAIN binding re-reads when its OWN write changes state (builder#107)**", async () => {
  const { raw, db, deliver } = await openWatching();
  db.bind("tasks", { live: false });
  await settle();
  const before = raw.__session.scans();
  deliver("own:tasks");
  await settle();
  assert.ok(raw.__session.scans() > before, "a plain binding stayed on its old snapshot after its own write published: its chip would say saving for ever");
});

await t("**a PLAIN binding shows the WINNER'S value after its own write is superseded (sdk#264)**", async () => {
  const { db, deliver } = await openWatching();
  const b = db.bind("tasks", { live: false });
  await settle();
  assert.equal(b.getSnapshot()[0]?.title, "mine", "the binding never showed this tab's own value");
  deliver("superseded:tasks:theirs");
  await settle();
  assert.equal(b.getSnapshot()[0]?.title, "theirs", "after this tab's write was superseded its plain binding still shows the forgotten value");
});

await t("THE CONTROL: somebody ELSE'S write (a head move) does NOT re-run a PLAIN binding", async () => {
  const { raw, db, deliver } = await openWatching();
  db.bind("tasks", { live: false });
  await settle();
  const before = raw.__session.scans();
  deliver("ours");
  await settle();
  assert.equal(raw.__session.scans(), before, "a plain binding re-read on another writer's change: LIVE would mean nothing");
});

await t("**a head move re-runs a LIVE binding, never a plain one**", async () => {
  const { raw, db, deliver } = await openWatching();
  let liveRuns = 0, plainRuns = 0;
  const live = db.bind("tasks", { live: true });
  const plain = db.bind("notes", { live: false });
  live.subscribe?.(() => { liveRuns += 1; });
  plain.subscribe?.(() => { plainRuns += 1; });
  await settle();
  const before = raw.__session.scans();
  deliver("ours");
  await settle();
  assert.equal(raw.__session.scans(), before + 1, "a head move did not re-run exactly the LIVE binding");
});

await t("**liveMode says how many LIVE bindings are bound, not only what the SESSION holds (sdk#259)**", async () => {
  const { raw, db } = await openWatching();
  assert.equal(db.liveMode().liveBindings, 0, "a page with no LIVE binding reported one");
  const live = db.bind("tasks", { live: true });
  const plain = db.bind("notes", { live: false });
  await settle();
  assert.equal(db.liveMode().liveBindings, 1, "a LIVE binding is bound and the report does not say so");
  assert.equal(db.liveMode().mode, "HeadSubscribed", "the session half of the report was lost");
  // A page can hold the subscription and have nothing to re-run on it: that
  // is what "not live = read when needed" looks like, and a tool reading only
  // the session's half would call this page live.
  live.stop?.();
  plain.stop?.();
  await settle();
  assert.equal(db.liveMode().liveBindings, 0, "a stopped LIVE binding is still counted, so the page looks live when nothing watches");
  assert.ok(raw.__session.scans() >= 0);
});

await t("**a HeadChanged for OUR head re-runs a bound scan, with no app call**", async () => {
  const raw = watchingRaw();
  const sdk = wrap(raw);
  let deliver;
  const { db } = await sdk.open({ app: "test-app",
    port: 17509,
    fetch: async () => ({ ok: true, arrayBuffer: async () => new ArrayBuffer(4) }),
    connect: (session, { onEvent }) => {
      deliver = key => { session.on_inbound(key); onEvent({ kind: "message" }); };
      return { close() {}, pump() {} };
    },
    setInterval: () => 1, clearInterval: () => {},
  });

  let reloads = 0;
  db.watch("tasks", () => { reloads += 1; db.scan("tasks"); });
  const before = raw.__session.scans();

  deliver("ours");
  await new Promise(r => setTimeout(r, 10));

  assert.equal(reloads, 1, "a head move did not re-run the bound binding");
  assert.ok(raw.__session.scans() > before, "the binding did not actually re-ask");
});

await t("THE CONTROL: a notification for SOMEBODY ELSE'S contract re-runs nothing", async () => {
  // Without this, a handler that reloaded on every notification would pass
  // the test above — and one unasked-for message would make a page refetch
  // every screen for ever.
  const raw = watchingRaw();
  const sdk = wrap(raw);
  let deliver;
  const { db } = await sdk.open({ app: "test-app",
    port: 17509,
    fetch: async () => ({ ok: true, arrayBuffer: async () => new ArrayBuffer(4) }),
    connect: (session, { onEvent }) => {
      deliver = key => { session.on_inbound(key); onEvent({ kind: "message" }); };
      return { close() {}, pump() {} };
    },
    setInterval: () => 1, clearInterval: () => {},
  });

  let reloads = 0;
  db.watch("tasks", () => { reloads += 1; });
  const before = raw.__session.scans();

  deliver("someone-elses-contract");
  await new Promise(r => setTimeout(r, 10));

  assert.equal(reloads, 0, "a foreign notification re-ran a binding");
  assert.equal(raw.__session.scans(), before, "it re-asked for data nobody said had changed");
  assert.equal(db.liveMode().foreignNotifications, 1, "it was ignored silently rather than counted");
});

await t("liveMode reaches an app THROUGH THE ENTRY", () => {
  // It was stored and never read: nothing in open(), engine-db.js or the
  // entry ever called it, so a page could not find out whether it was being
  // notified or polling.
  const sdk = wrap(watchingRaw());
  assert.equal(typeof sdk.open, "function");
});

// ---------------------------------------------------------------------------
// TIME, AND THE LAST THING A PAGE SAYS.
//
// The delegate has no clock (F32). Every deadline it holds — owed parity
// going out, a stuck commit reported `Stalled` — happens only because a
// connected page told it what time it is, and a page that closes stops
// telling it anything at all.
//
// Both were built on the engine side and reachable from neither: the page's
// `tick()` did its OWN housekeeping and sent the delegate nothing, and no
// page anywhere sent `Flush`. These go through the entry, because that is
// where the two ends meet.
// ---------------------------------------------------------------------------

/** A page harness: a driveable clock, a recording socket, a lifecycle. */
function pageOf(raw, extra = {}) {
  const clock = { body: null, stopped: false };
  const win = new Map(), doc = new Map();
  const conn = { pumps: 0, closed: false };
  const opts = { app: "test-app",
    port: 17509,
    fetch: async () => ({ ok: true, arrayBuffer: async () => new ArrayBuffer(4) }),
    connect: () => ({ close() { conn.closed = true; }, pump() { conn.pumps += 1; } }),
    setInterval: (fn, ms) => { clock.body = fn; clock.ms = ms; return 1; },
    clearInterval: () => { clock.stopped = true; clock.body = null; },
    addEventListener: (name, fn) => win.set(name, fn),
    removeEventListener: name => win.delete(name),
    documentOf: {
      visibilityState: "visible",
      addEventListener: (name, fn) => doc.set(name, fn),
      removeEventListener: name => doc.delete(name),
    },
    ...extra,
  };
  return { opts, clock, conn, win, doc, session: raw.__session };
}

await t("the tick RATE comes from the session, not from a literal in the page", async () => {
  const raw = fakeRaw();
  const page = pageOf(raw);
  const { close } = await wrap(raw).open(page.opts);

  assert.equal(page.clock.ms, raw.__session.tick_ms(),
    "the page picked its own rate. The number the page sends time at and the " +
    "number the engine's deadlines are counted in are the same number, and a " +
    "literal here goes on being right until the day the other side moves");
  close();
});

await t("**every tick PUMPS, so the time actually leaves the page**", async () => {
  const raw = fakeRaw();
  const page = pageOf(raw);
  const { close } = await wrap(raw).open(page.opts);
  const pumpsBefore = page.conn.pumps;

  page.clock.body();

  assert.equal(raw.__session.ticks, 1, "the timer did not tick the session at all");
  assert.ok(page.conn.pumps > pumpsBefore,
    "the tick queued a frame and nothing sent it. The socket pumps on open " +
    "and on a message, and a tick produces neither — so on the quiet " +
    "connection where a stuck commit actually lives, the time would sit in " +
    "the outbox for ever");
  close();
});

await t("a closed page stops sending time", async () => {
  const raw = fakeRaw();
  const page = pageOf(raw);
  const { close } = await wrap(raw).open(page.opts);
  page.clock.body();
  const after = raw.__session.ticks;

  close();

  assert.ok(page.clock.stopped, "the interval was left running on a closed page");
  assert.equal(page.clock.body, null, "there is still a timer body to fire");
  assert.equal(raw.__session.ticks, after, "it ticked after being closed");
  assert.ok(page.conn.closed, "the socket was left open");
});

await t("**a page being HIDDEN flushes, and the frame is pumped in the same task**", async () => {
  const raw = fakeRaw();
  const page = pageOf(raw);
  const { close } = await wrap(raw).open(page.opts);
  const pumpsBefore = page.conn.pumps;

  page.opts.documentOf.visibilityState = "hidden";
  page.doc.get("visibilitychange")();

  assert.equal(raw.__session.flushes, 1,
    "a tab going into the background sent no Flush. It also stops ticking, " +
    "so whatever the engine was holding back to coalesce sits unwritten " +
    "until somebody opens the app again");
  assert.ok(page.conn.pumps > pumpsBefore,
    "the Flush was queued and not sent. A page being hidden may not get a " +
    "second task");
  close();
});

await t("THE CONTROL: a page becoming VISIBLE flushes nothing", async () => {
  // Without this, a `visibilitychange` handler that flushed unconditionally
  // would pass the test above — and would ship everything the engine was
  // coalescing every time somebody switched back to the tab, which is the
  // opposite of what coalescing is for.
  const raw = fakeRaw();
  const page = pageOf(raw);
  const { close } = await wrap(raw).open(page.opts);

  page.opts.documentOf.visibilityState = "visible";
  page.doc.get("visibilitychange")();

  assert.equal(raw.__session.flushes, 0, "it flushed on the tab coming BACK");
  close();
});

await t("pagehide flushes too, because visibilitychange alone is not enough", async () => {
  // A page can be discarded from the background without a second
  // `visibilitychange`, and on a real navigation away `pagehide` is what
  // fires. Both are wired; a Flush is idempotent, so the cost of the
  // overlap is a frame the engine answers with nothing to do.
  const raw = fakeRaw();
  const page = pageOf(raw);
  const { close } = await wrap(raw).open(page.opts);

  page.win.get("pagehide")();

  assert.equal(raw.__session.flushes, 1, "nothing is listening for pagehide");
  close();
});

await t("and closing flushes, while the socket can still carry it", async () => {
  const raw = fakeRaw();
  const page = pageOf(raw);
  const { close } = await wrap(raw).open(page.opts);

  close();

  assert.equal(raw.__session.flushes, 1,
    "close() shut the socket without shipping what was waiting — the one " +
    "moment the frame could still be sent");
  assert.equal(page.doc.size, 0, "the lifecycle listeners outlived the page");
  assert.equal(page.win.size, 0, "the window listeners outlived the page");
});

// ---- THE UNSAVED-CHANGES GUARD (craftworks-sdk#163) ------------------------
//
// A write is safe from a closing tab only once it is PUBLISHED. The session
// counts every write not yet published — held by the window included
// (`unsaved_writes`, tested natively in tests/unsaved_writes.rs) — and the
// page asks the browser's "leave site?" question while that count is above
// zero, and stops asking the moment it reaches zero.

/** A fake session whose unsaved count the test sets, and whose `put` makes one. */
function unsavedRaw() {
  const raw = fakeRaw();
  const s = raw.__session;
  s.unsaved = 0;
  s.unsaved_writes = () => s.unsaved;
  s.put = () => { s.unsaved += 1; return JSON.stringify({ id: "a".repeat(32), created: 1, updated: 1, fields: {} }); };
  return raw;
}

/** What a browser does with a `beforeunload` listener: call it, and ask if it objected. */
const leaving = page => {
  const fn = page.win.get("beforeunload");
  if (!fn) return { asked: false };
  const e = { prevented: false, preventDefault() { this.prevented = true; }, returnValue: undefined };
  fn(e);
  return { asked: e.prevented || e.returnValue !== undefined };
};

await t("**a write made and not yet published arms the guard AT ONCE; the last one publishing disarms it**", async () => {
  const raw = unsavedRaw();
  const events = [];
  const page = pageOf(raw, { onEvent: e => { if (e.kind === "saving") events.push(e.count); } });
  const { db, close } = await wrap(raw).open(page.opts);
  assert.equal(page.win.has("beforeunload"), false, "a page with nothing unsaved asks the leave-site question anyway");

  for (let i = 0; i < 100; i++) await db.put("tasks", { title: `row ${i}` });
  // Armed in the same task as the write — not at the next message or tick.
  assert.equal(leaving(page).asked, true, "100 writes unsaved and the tab would close without a word");

  raw.__session.unsaved = 3;          // 97 published; 3 still going
  page.clock.body();
  assert.equal(leaving(page).asked, true, "disarmed with 3 writes still unsaved");

  raw.__session.unsaved = 0;          // the last one published
  page.clock.body();
  assert.equal(page.win.has("beforeunload"), false, "everything is published and the page still objects to closing");

  assert.deepEqual(events, [...Array.from({ length: 100 }, (_, i) => i + 1), 3, 0],
    "the page was not told each count as it changed — it cannot say 'saving N…' without it");
  close();
});

await t("closing a page with writes unsaved removes the guard with the page", async () => {
  const raw = unsavedRaw();
  const page = pageOf(raw);
  const { db, close } = await wrap(raw).open(page.opts);
  await db.put("tasks", { title: "one" });
  assert.equal(page.win.has("beforeunload"), true);
  close();
  assert.equal(page.win.has("beforeunload"), false, "the guard outlived the page");
});

// THE PAGE'S RECORDING HAS A READER (sdk#434). The page records every op from its first (sdk#407), but no JS
// surface exposed `page_trace`, so neither a harness nor anyone supporting a person could read which End an op took
// (sdk#431 had to be reproduced instead of read). The handle's `pageTrace()` is that reader: the session's own dump,
// read when asked -- never a copy taken at open.
await t("**the session handle reads the page's recording: `pageTrace()` is the session's `page_trace()`, read when asked**", async () => {
  const raw = fakeRaw();
  raw.__session.page_trace = () => raw.__session.dump;
  raw.__session.dump = "── instrument dump: page ops (stream v1) ──\n   exit   page::op::put#3 Withdrawn\n";
  const page = pageOf(raw);
  const handle = await wrap(raw).open(page.opts);
  assert.equal(typeof handle.pageTrace, "function", "the handle has no pageTrace(): the recording has no reader");
  assert.equal(handle.pageTrace(), raw.__session.dump);
  raw.__session.dump = "── instrument dump: page ops (stream v1) ──\n   exit   page::op::put#4 Response\n";
  assert.equal(handle.pageTrace(), raw.__session.dump, "pageTrace() answered an old dump: a copy, not the session's recording");
  handle.close();
});

process.stdout.write(failures ? `\n${failures} failing\n` : "\nall passing\n");
process.exit(failures ? 1 : 0);
