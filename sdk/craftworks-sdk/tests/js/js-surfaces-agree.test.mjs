// THE TWO JAVASCRIPT SURFACES MUST AGREE TOO.
//
// `web/tests/surfaces_agree.rs` compares the two WASM objects. That is half
// the boundary: an app does not hold a wasm object, it holds what `wrap()`
// and `engineDb()` return — and `bind` lives there, on neither wasm side.
//
// So `engineDb` had no `bind` at all, and a published project threw
// `db.bind is not a function` the moment it rendered. Every gate was green:
// the Rust one compared the wrong pair of objects, and no JavaScript one
// existed. "Publish switches the backend and the app code does not change"
// has to be true of the object a component actually holds.
//
// AND THE NAME IS NOT THE SURFACE. This file then compared names only, and
// the very next instance of the same failure walked straight through it: the
// engine-backed `schema` and `count` are ASYNC — they read over the network —
// and the in-memory ones answered values. An app got a Promise where it
// expected an object, and a Promise is truthy, so it passed an
// `if (!schema)` guard and arrived at `schema.fields.map` as `undefined.map`.
//
// Measured against a real node (sdk#87): four defects in the builder, three
// of which replaced the whole app with an exception string while the button
// said "Published", and one of which printed "— [object Promise] records" to
// a person — a wrong number, no crash, nothing would have reported it.
//
// So the second gate below compares what each method ANSWERS WITH: a promise
// or a value. An app must not be able to tell which backend it has from the
// shape of an answer.

import assert from "node:assert/strict";
import { engineDb } from "../../js/engine-db.js";
import { wrap } from "../../js/wrap.js";

let failures = 0;
/**
 * AWAITED. It was not, and an async test's rejection was swallowed: the
 * helper wrote `ok` and moved on while the assertions were still pending.
 *
 * Found by mutation — the state-comparison test below reported `ok` against
 * code with the fix REMOVED, which is a test that cannot fail. Every call is
 * awaited now, so a synchronous test is unaffected and an async one is
 * actually run.
 */
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.message}\n`); }
};

/** A wasm stand-in: enough for `wrap` and `engineDb` to be constructed. */
const fakeRaw = () => ({
  version: () => "0", buildInfo: () => "{}", blockId: () => "", parseBlockId: () => "{}",
  Db: function () {
    return {
      define() {}, schema: () => "null", domains: () => "[]",
      put: () => "{}", create_at: () => "{}", update: () => "{}", get: () => "null", delete: () => false,
      scan: () => "[]", count: () => 0, root: () => "", stats: () => "{}",
      draft_put() {}, draft_file() {}, draft_delete: () => false, publish_definition: () => 0, definition: () => [], definition_apps: () => "[]",
      mark_published: () => "{}", is_published: () => false, published_state: () => "null",
    };
  },
  Session: function () { return {}; },
});

const fakeSession = () => ({
  session: {
    schema: () => "null", domains: () => "[]", put: () => "{}", create_at: () => "{}", update: () => "{}",
    get: () => "null", delete: () => false, scan: () => "[]", count: () => 0,
    root: () => "", stats: () => "{}", define: () => undefined,
    draft_put: () => undefined, draft_file: () => undefined, draft_delete: () => false, publish_definition: () => 0, definition: () => [], definition_apps: () => "[]",
    mark_published: () => "{}", is_published: () => false, published_state: () => "null",
    preload: () => 0, trace: () => "null", trace_on: () => undefined,
    take_loads: () => "[]", take_stale: () => "[]", live_mode: () => "{}",
    bind: () => undefined, unbind: () => undefined, rendered: () => undefined,
    take_stale: () => "[]",
  },
});

/** Methods an object offers, own and inherited, excluding the constructor. */
const surfaceOf = o => {
  const names = new Set();
  for (let p = o; p && p !== Object.prototype; p = Object.getPrototypeOf(p)) {
    for (const k of Object.getOwnPropertyNames(p)) {
      if (k === "constructor") continue;
      const d = Object.getOwnPropertyDescriptor(p, k);
      if (typeof d?.value === "function") names.add(k);
    }
  }
  return names;
};

/** On the engine side only: there is no engine behind the in-memory one. */
// `waitingForRoom`: writes waiting on the page queue's byte bound (QUEUE_FULL),
// which only the engine-backed db has -- the in-memory one never fills.
// repairAll / repairAllCancel / checkAll (sdk#479): REPAIR puts back (CHECK counts) what the NODE lost; an
// unpublished project has no node. `watchDefinition`, like `watch`: a head move is a session's; the in-tab db's tree
// moves only by its own writes.
const ENGINE_ONLY = new Set(["preload", "trace", "traceOn", "watch", "watchDefinition", "liveMode", "drain", "waitingForRoom", "damaged", "repairAll", "repairAllCancel", "checkAll"]);

await t("**the engine-backed db offers every method the in-memory one does**", () => {
  const memory = surfaceOf(wrap(fakeRaw()).Db.prototype ?? new (wrap(fakeRaw()).Db)());
  const engine = surfaceOf(engineDb(fakeSession()));

  const missing = [...memory].filter(m => !engine.has(m));
  assert.deepEqual(
    missing, [],
    `an app that switched backend would break on exactly these: ${missing.join(", ")}.\n` +
    "This is the gate that was missing when `bind` was absent and a published " +
    "project threw `db.bind is not a function` on its first render.");
});

await t("THE CONTROL: the reader really does find methods", () => {
  // Without this, a `surfaceOf` that returned an empty set would make the
  // test above pass against any two objects at all.
  const engine = surfaceOf(engineDb(fakeSession()));
  for (const m of ["put", "get", "scan", "bind"]) {
    assert.ok(engine.has(m), `the reader did not find ${m}, so it finds nothing`);
  }
});

await t("and the engine side's extras are declared, not accidental", () => {
  const memory = surfaceOf(wrap(fakeRaw()).Db.prototype ?? new (wrap(fakeRaw()).Db)());
  const engine = surfaceOf(engineDb(fakeSession()));
  const extra = [...engine].filter(m => !memory.has(m) && !ENGINE_ONLY.has(m));
  assert.deepEqual(
    extra, [],
    `the engine surface grew ${extra.join(", ")}, which an app on an unpublished ` +
    "project would not have. Add them to the in-memory one, or to ENGINE_ONLY with a reason.");
});

await t("**every ENGINE_ONLY method EXISTS on the engine db** -- declared is not offered (Codex on sdk#542)", () => {
  const engine = surfaceOf(engineDb(fakeSession()));
  const absent = [...ENGINE_ONLY].filter(m => !engine.has(m));
  assert.deepEqual(absent, [], `declared engine-only, but the engine db does not offer: ${absent.join(", ")}`);
});

await t("**damaged() forwards what the session names** (sdk#524): a DAMAGED group, not an empty list", () => {
  const named = [{ block: "ab".repeat(32), j: 20, k: 21, health: "DAMAGED", why: "lost past the group's reach" }];
  const fake = fakeSession();
  const db = engineDb({ ...fake, session: { ...fake.session, damaged: () => JSON.stringify(named) } });
  assert.deepEqual(db.damaged(), named);
});

// ---------------------------------------------------------------------------
// SHAPE, not just name.
// ---------------------------------------------------------------------------

/** Plausible arguments, so every method can actually be called. */
const ARGS = {
  define: ["d", { type: "T", fields: [] }],
  schema: ["d"], domains: [], count: ["d"], get: ["d", "x"], delete: ["d", "x"],
  put: ["d", {}], createAt: ["d", "x", {}], update: ["d", "x", {}], scan: ["d"],
  // The parent is a record id both surfaces will refuse to parse; that is
  // fine, because what this gate compares is the SHAPE of the answer (promise
  // or value), and a refusal that differs in shape is exactly the drift it
  // exists to catch.
  children: ["d", "x"],
  root: [], stats: [], bind: ["d"],
  // The definition doors (app-as-data P2).
  draftPut: ["meta", {}], draftFile: ["app.js", new Uint8Array([1])], draftDelete: ["meta"], publishDefinition: [], definition: ["draft"], definitionApps: [],
  markPublished: ["d"], isPublished: ["d"], publishedState: ["d"],
  // Another app's data, read-only: the handle itself is a VALUE on both surfaces.
  other: ["some-app"],
};

/**
 * Calls whose OPTIONS change what they read (sdk#137): each is asked of both
 * surfaces too, so an option one surface ignores cannot hide behind the other
 * answering the same shape without it.
 */
const OPTION_CALLS = [
  ["bind", ["d", { parent: "0000000000000000000000000000000a", limit: 5, reverse: true }]],
];

/** Is this a promise? The only question the two surfaces must agree on. */
const isThenable = v => typeof v?.then === "function";

/**
 * What a method answers with, or why it could not be asked.
 *
 * A method that THROWS is reported as such rather than skipped: "could not
 * check" is a third outcome, and in a log it is indistinguishable from a pass.
 */
const shapeOf = (obj, name) => {
  if (!(name in ARGS)) return { kind: "no-args-known" };
  try {
    const v = obj[name](...ARGS[name]);
    if (isThenable(v)) v.catch(() => {});      // nothing here awaits; do not warn
    return { kind: isThenable(v) ? "promise" : "value" };
  } catch (e) {
    return { kind: "threw", why: String(e?.message ?? e) };
  }
};

await t("**both surfaces answer the same SHAPE, not just the same names**", () => {
  const memory = new (wrap(fakeRaw()).Db)();
  const engine = engineDb(fakeSession());
  const shared = [...surfaceOf(memory)].filter(m => surfaceOf(engine).has(m)).sort();

  const differ = [], unchecked = [];
  for (const name of shared) {
    const a = shapeOf(memory, name), b = shapeOf(engine, name);
    if (a.kind === "no-args-known") { unchecked.push(name); continue; }
    if (a.kind === "threw" || b.kind === "threw") {
      unchecked.push(`${name} (threw: ${a.why ?? b.why})`);
      continue;
    }
    if (a.kind !== b.kind) differ.push(`${name}: in-memory ${a.kind}, engine ${b.kind}`);
  }

  assert.deepEqual(differ, [],
    "an app can tell which backend it has from the shape of an answer:\n  " +
    differ.join("\n  ") +
    "\nA caller written against one breaks on the other, and a Promise is " +
    "truthy — so it passes a falsy guard and fails later, somewhere else.");
  assert.deepEqual(unchecked, [],
    `these could not be compared at all: ${unchecked.join(", ")}. ` +
    "A method this gate cannot ask is a method it does not check, and in a " +
    "green log that is indistinguishable from one it checked. Give it " +
    "arguments in ARGS, or say why it is exempt.");
  console.log(`      ${shared.length} shared methods, all compared by shape`);
});

await t("**a binding over ONE PARENT reports it on both surfaces** (sdk#137)", () => {
  const memory = new (wrap(fakeRaw()).Db)();
  const engine = engineDb({ ...fakeSession(), session: { ...fakeSession().session, watch_key: (d, p) => `${d}#${p}`, children: () => "[]" } });
  for (const [name, args] of OPTION_CALLS) {
    const a = memory[name](...args), b = engine[name](...args);
    for (const [k, want] of Object.entries(args[1])) {
      assert.deepEqual([a[k], b[k]], [want, want], `${name}: option \`${k}\` not reported by both surfaces`);
    }
  }
});

// ---------------------------------------------------------------------------
// EVERY RUST-BOUNDARY DATA METHOD HAS A JS COUNTERPART, ON BOTH SURFACES.
// ---------------------------------------------------------------------------
//
// The gate above compares the two JS surfaces with EACH OTHER, so when
// `children` was missing from both it was green: they agreed, on its
// absence (sdk#137). This compares each against the BUILT wasm — the
// in-memory `Db`'s methods, which are the data surface by definition.

const snakeToCamel = s => s.replace(/_([a-z])/g, (_, c) => c.toUpperCase());
/** Methods on a wasm class that are not data: wasm-bindgen's own. */
const BINDGEN = new Set(["constructor", "free", "__destroy_into_raw"]);
const dataMethodsOf = cls => Object.getOwnPropertyNames(cls.prototype).filter(n => !BINDGEN.has(n) && typeof n === "string");
/** What a JS surface is missing of a wasm data surface. */
const missingFrom = (wasmMethods, jsSurface) => wasmMethods.map(snakeToCamel).filter(m => !jsSurface.has(m));

await t("**every data method of the BUILT wasm Db exists on BOTH JavaScript surfaces**", async () => {
  const { createRequire } = await import("node:module");
  const raw = createRequire(import.meta.url)("../../pkg/node/craftworks_sdk.js");
  const wasm = dataMethodsOf(raw.Db);
  assert.ok(wasm.includes("children") && wasm.length > 8, `the reader found only ${wasm}, so it checks nothing`);
  const memory = surfaceOf(new (wrap(fakeRaw()).Db)());
  const engine = surfaceOf(engineDb(fakeSession()));
  assert.deepEqual(missingFrom(wasm, memory), [], "the in-memory JS surface lacks a wasm data method");
  assert.deepEqual(missingFrom(wasm, engine), [], "the engine-backed JS surface lacks a wasm data method");
});

await t("THE CONTROL: a wasm method with no JS counterpart is FOUND missing", () => {
  class Fake { children() {} frobnicate_rows() {} }
  const js = new Set(["children"]);
  assert.deepEqual(missingFrom(dataMethodsOf(Fake), js), ["frobnicateRows"]);
});

await t("THE CONTROL: a method that is sync on one side and async on the other FAILS", () => {
  // Without this, a `shapeOf` that returned the same kind for everything —
  // a broken thenable check, an early return — would report perfect agreement
  // over two surfaces that agree about nothing.
  const memory = new (wrap(fakeRaw()).Db)();
  const engine = engineDb(fakeSession());
  // `count` is async on both. Make the in-memory one answer a value.
  const patched = Object.create(memory);
  patched.count = () => 0;

  const a = shapeOf(patched, "count"), b = shapeOf(engine, "count");
  assert.equal(a.kind, "value", "the control did not actually make it synchronous");
  assert.equal(b.kind, "promise", "the engine side is not async, so there is nothing to detect");
  assert.notEqual(a.kind, b.kind, "a sync-vs-async pair was not detected as a difference");
});

await t("THE CONTROL: the shape reader is not simply calling everything a promise", () => {
  // `root` and `stats` are synchronous on BOTH: a root is a value this client
  // already holds. If the reader called them promises, the gate above would
  // pass by agreeing on the wrong answer everywhere.
  const memory = new (wrap(fakeRaw()).Db)();
  for (const name of ["root", "stats"]) {
    assert.equal(shapeOf(memory, name).kind, "value",
      `${name} was read as a promise; the reader cannot tell the two apart`);
  }
});

// ---------------------------------------------------------------------------
// A WRITE REACHING THE NETWORK IS A CHANGE THE SCREEN MUST SEE.
// ---------------------------------------------------------------------------

/**
 * A row whose STATE moved, and nothing else.
 *
 * `PENDING -> CLEAN` is what happens when a write reaches the network. It
 * changes neither `id` nor `updated` — `updated` is the record's own
 * timestamp, and a write state is not a content change — so a snapshot
 * comparison of those two says "the same rows" and the component never
 * re-renders.
 *
 * MEASURED against a real node: a row sat on screen saying "saving" for 70
 * seconds while `db.scan()` returned it CLEAN the whole time. The data was
 * published within a second. Only the screen was wrong, which is the worst
 * version of this: a person is told their data is unsaved when it is safely
 * on the network, and closing the tab then feels like losing it.
 */
const ROW = (state) => ({ id: "a", updated: 7, fields: { t: "x" }, state });

await t("**a row whose write STATE changed is a new snapshot, on the engine side**", async () => {
  let state = "PENDING";
  const session = {
    ...fakeSession().session,
    scan: () => JSON.stringify([ROW(state)]),
    root: () => "node:same",
  };
  const db = engineDb({ session });
  const b = db.bind("notes");
  await b.reload();
  const first = b.getSnapshot();
  assert.equal(first[0].state, "PENDING");

  let told = 0;
  b.subscribe(() => { told += 1; });
  state = "CLEAN";                      // the write reached the network
  await b.reload();

  assert.notEqual(b.getSnapshot(), first,
    "the snapshot was not replaced when the row's write state changed, so a " +
    "component holding it renders `saving` for ever over data that is safely " +
    "on the network");
  assert.equal(b.getSnapshot()[0].state, "CLEAN", "the new snapshot has the old state");
  assert.equal(told, 1, "nobody was told, so nothing re-renders");
});

await t("THE CONTROL: a reload that changes NOTHING keeps the same snapshot", () => {
  // The referential stability this comparison exists for. Without this, a
  // `same` that always returned false would pass the test above and make
  // useSyncExternalStore re-render on every render, for ever.
  const session = {
    ...fakeSession().session,
    scan: () => JSON.stringify([ROW("CLEAN")]),
    root: () => "node:same",
  };
  const db = engineDb({ session });
  const b = db.bind("notes");
  return b.reload().then(async () => {
    const first = b.getSnapshot();
    let told = 0;
    b.subscribe(() => { told += 1; });
    await b.reload();
    assert.equal(b.getSnapshot(), first, "an unchanged reload replaced the snapshot");
    assert.equal(told, 0, "an unchanged reload told a listener");
  });
});

// ---------------------------------------------------------------------------
// UNREACHABLE IS NOT EMPTY, and a component must be able to tell.
//
// A component holds a binding and reads `getSnapshot()`. An empty array meant
// BOTH "the range was read and holds nothing" and "the range could not be
// reached" — and the second is not a smaller version of the first. Absence is
// never provable (§3); the layers underneath were built at real cost to keep
// `NotLoaded` apart from empty (sdk#54, #63); and a binding that flattens
// them is the LAST place that distinction can be lost.
// ---------------------------------------------------------------------------

const scanning = answer => ({
  ...fakeSession().session,
  scan: answer,
  root: () => "node:r",
});

await t("**a range that cannot be read is `unreachable`, not empty**", async () => {
  const db = engineDb({
    session: scanning(() => {
      const e = new Error("the range this read needed could not be loaded");
      e.code = "UNAVAILABLE";
      throw e;
    }),
  });
  const b = db.bind("notes");
  await b.reload();

  assert.deepEqual(b.getSnapshot(), [], "there are no rows either way — that is the problem");
  assert.equal(b.status().state, "unreachable",
    "a range nobody could read reports the same state as one that is empty, so a " +
    "component renders `No records yet` over a tree it merely could not reach");
  assert.equal(b.status().code, "UNAVAILABLE", "the code a caller would branch on is gone");
  assert.match(b.status().why, /could not be loaded/, "the sentence a person would read is gone");
});

await t("THE CONTROL: a range that IS empty says so, and is not unreachable", async () => {
  // Without this, a binding that reported `unreachable` whenever it had no
  // rows would pass the test above — and an app would claim it could not
  // reach a domain that is simply empty, which is the same lie reversed.
  const db = engineDb({ session: scanning(() => "[]") });
  const b = db.bind("notes");
  await b.reload();
  assert.deepEqual(b.getSnapshot(), []);
  assert.equal(b.status().state, "ready",
    "an empty range was reported unreachable; emptiness IS provable once the range was read");
});

await t("a binding is `loading` until its first reload finishes", async () => {
  const db = engineDb({ session: scanning(() => "[]") });
  const b = db.bind("notes");
  assert.equal(b.status().state, "loading",
    "a binding claims to know something before it has read anything");
  await b.reload();
  assert.equal(b.status().state, "ready");
});

await t("**the state changing is itself a change, even when the rows do not**", async () => {
  // A range that was unreachable and is now readable-and-empty is a different
  // screen with the same rows. A binding that only told its listeners when
  // the ROWS changed would leave the component showing the error for ever.
  let fail = true;
  const db = engineDb({
    session: scanning(() => {
      if (fail) {
        const e = new Error("no"); e.code = "UNAVAILABLE"; throw e;
      }
      return "[]";
    }),
  });
  const b = db.bind("notes");
  await b.reload();
  assert.equal(b.status().state, "unreachable");

  let told = 0;
  b.subscribe(() => { told += 1; });
  fail = false;
  await b.reload();

  assert.equal(b.status().state, "ready", "it never recovered");
  assert.deepEqual(b.getSnapshot(), [], "the rows are the same — no rows, either way");
  assert.equal(told, 1,
    "nobody was told the range became readable, because only the rows are watched. " +
    "The component would show `unreachable` over a range it can now read.");
});

await t("**a DIFFERENT reason is a change, even with the same state**", async () => {
  // `state` alone is not the whole of status. Two failures with different
  // reasons share a state, so a comparison on `state` replaced `why` and
  // `code` and told nobody — and the doc tells callers to SHOW `why`, so a
  // component displayed the first sentence for ever while the real reason
  // changed underneath it.
  //
  // THE ERROR DIFFERS PER CALL, and the two are asserted distinct before
  // anything is concluded: `bind()` itself triggers a scan, so a probe
  // drawing from a fixed list consumes one before the first `reload()` and
  // would compare an error against itself — a gap manufactured by its own
  // setup.
  let n = 0;
  const db = engineDb({
    session: scanning(() => {
      n += 1;
      const e = new Error(`reason ${n}`);
      e.code = `CODE_${n}`;
      throw e;
    }),
  });
  const b = db.bind("notes");
  await b.reload();
  const first = { ...b.status() };

  let told = 0;
  b.subscribe(() => { told += 1; });
  await b.reload();
  const second = { ...b.status() };

  assert.notDeepEqual(first, second,
    "the two failures are identical, so this test cannot detect anything — " +
    "the error must differ per call");
  assert.equal(first.state, second.state, "they must share a state, or the state alone would catch it");
  assert.equal(second.code, `CODE_${n}`, "the latest reason is not the one held");
  assert.equal(told, 1,
    `the reason changed from ${first.code} to ${second.code} and nobody was told. ` +
    "A component showing status().why displays a stale sentence for ever.");
});

await t("THE CONTROL: the SAME failure twice tells nobody", async () => {
  // Without this, a binding that notified on every reload would pass the
  // test above and re-render for ever on an unchanging error.
  const db = engineDb({
    session: scanning(() => {
      const e = new Error("the same every time");
      e.code = "SAME";
      throw e;
    }),
  });
  const b = db.bind("notes");
  await b.reload();
  let told = 0;
  b.subscribe(() => { told += 1; });
  await b.reload();
  assert.equal(told, 0, "an unchanged failure notified anyway, so nothing is stable");
});

await t("THE CONTROL: both surfaces answer `status`, so an app cannot tell them apart", async () => {
  // The in-memory database is in the tab and a range there is never
  // unreachable — but a component that branched on `status()` EXISTING would
  // work in a preview and throw the moment the project was published.
  const memory = new (wrap(fakeRaw()).Db)();
  const b = memory.bind("notes");
  assert.equal(typeof b.status, "function", "the in-memory binding has no status()");
  assert.equal(b.status().state, "loading", "it claims to know something before reading");
  await b.reload();
  assert.equal(b.status().state, "ready");
});

// ---------------------------------------------------------------------------
// A READ SHOULD COST WHAT THE SCREEN COSTS.
//
// `reload` scanned the whole domain, every time, for every binding — so a
// component showing twenty rows read twenty thousand if the domain held them.
// `Scan { limit, after }` existed unused the whole time.
// ---------------------------------------------------------------------------

await t("**a binding with a limit asks for a PAGE, not the domain**", async () => {
  const asked = [];
  const db = engineDb({
    session: {
      ...fakeSession().session,
      scan: (domain, reverse, limit, after) => {
        asked.push({ domain, limit, after });
        return "[]";
      },
      root: () => "node:r",
    },
  });
  const b = db.bind("notes", { limit: 20 });
  await b.reload();
  assert.ok(asked.length > 0, "the binding never scanned at all — this test measures nothing");
  assert.strictEqual(asked.at(-1).limit, 20,
    `the scan asked for limit ${asked.at(-1).limit}, so the read costs the DOMAIN and not the screen`);
  assert.strictEqual(b.limit, 20, "the binding does not report the page size it was given");
});

await t("THE CONTROL: no limit still asks for everything", async () => {
  // Unbounded is the default rather than a hidden 50, because a caller that
  // asked for everything and silently got a page would draw a partial list
  // and call it complete — the same confusion between "all of it" and "what
  // I could get" that NotLoaded exists to prevent one layer down.
  const asked = [];
  const db = engineDb({
    session: {
      ...fakeSession().session,
      scan: (domain, reverse, limit) => { asked.push(limit); return "[]"; },
      root: () => "node:r",
    },
  });
  await db.bind("notes").reload();
  assert.strictEqual(asked.at(-1), 0, "a binding with no limit quietly paginated");
});

await t("THE CONTROL: both surfaces take a limit, so an app cannot tell them apart", async () => {
  const memory = new (wrap(fakeRaw()).Db)();
  const b = memory.bind("notes", { limit: 20 });
  assert.strictEqual(b.limit, 20, "the in-memory binding ignores the page size");
  assert.strictEqual(memory.bind("notes").limit, 0, "its default is not unbounded");
});

// ---------------------------------------------------------------------------
// A LIMIT WITHOUT A DIRECTION IS HALF A PAGE.
//
// Ids are time-ordered, so the first `limit` rows of a FORWARD scan are the
// OLDEST. A caller rendering newest-first by reversing the snapshot then shows
// the oldest page in reverse, and the newest records never reach the screen —
// measured on the builder: past 50 records, note 51 onward never appeared
// though the form had accepted them (craftworks-builder#51).
//
// The page bounded the read and silently changed WHICH rows it was a page of.
// ---------------------------------------------------------------------------

await t("**a reverse binding pages from the NEWEST end**", async () => {
  // The fake answers the way the real scan does: forward gives the head of
  // the range, reverse gives the tail.
  const all = Array.from({ length: 60 }, (_, i) => ({ id: `n${String(i).padStart(3, "0")}`, updated: i }));
  const db = engineDb({
    session: {
      ...fakeSession().session,
      scan: (_d, reverse, limit) => {
        const rows = reverse ? [...all].reverse() : all;
        return JSON.stringify(limit ? rows.slice(0, limit) : rows);
      },
      root: () => "node:r",
    },
  });

  const forward = db.bind("notes", { limit: 50 });
  await forward.reload();
  const back = db.bind("notes", { limit: 50, reverse: true });
  await back.reload();

  assert.strictEqual(forward.getSnapshot().length, 50, "the forward page is not a page");
  assert.strictEqual(forward.getSnapshot().at(-1).id, "n049",
    "the forward page does not end where a forward page should");
  assert.strictEqual(back.getSnapshot()[0].id, "n059",
    "a reverse binding did not start at the NEWEST row, so a newest-first list " +
    "paged from the wrong end and the latest records never reach the screen");
  assert.strictEqual(back.reverse, true, "the binding does not report its direction");
});

await t("THE CONTROL: the default is still FORWARD", async () => {
  // Without this, a binding that always reversed would pass the test above
  // and silently flip every existing caller's order.
  const asked = [];
  const db = engineDb({
    session: {
      ...fakeSession().session,
      scan: (_d, reverse) => { asked.push(reverse); return "[]"; },
      root: () => "node:r",
    },
  });
  await db.bind("notes").reload();
  assert.strictEqual(asked.at(-1), false, "a binding with no direction asked for a reverse scan");
  assert.strictEqual(db.bind("notes").reverse, false, "its default direction is not forward");
});

await t("THE CONTROL: both surfaces take a direction", async () => {
  const memory = new (wrap(fakeRaw()).Db)();
  assert.strictEqual(memory.bind("notes", { reverse: true }).reverse, true,
    "the in-memory binding ignores the direction, so an app could tell the backends apart");
  assert.strictEqual(memory.bind("notes").reverse, false, "its default is not forward");
});

await t("a BINDING from the engine has the shape a component holds", () => {
  const b = engineDb(fakeSession()).bind("tasks", { live: false });
  for (const m of ["getSnapshot", "subscribe", "reload"]) {
    assert.equal(typeof b[m], "function", `a binding has no ${m}`);
  }
  // Referentially stable: a caller re-rendering on identity change would
  // otherwise re-render for ever, whatever the data did.
  assert.strictEqual(b.getSnapshot(), b.getSnapshot(), "the snapshot is a new object each call");
  assert.equal(b.live, false);
  assert.equal(engineDb(fakeSession()).bind("tasks", { live: true }).live, true);
});

process.stdout.write(failures ? `\n${failures} failing\n` : "\nall passing\n");
process.exit(failures ? 1 : 0);
