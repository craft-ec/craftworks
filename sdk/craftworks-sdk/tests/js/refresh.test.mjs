// THE LIVE PATH: what makes a binding show data it did not have.
//
// A binding's `reload` used to read the LOCAL COPY and return early when its
// root had not moved — and nothing moved the copy's root, so a tab that made
// no write could never see another's. There is no copy now (READ-STATE): a
// read walks the engine's tree, and on a head move the SESSION says which
// LIVE ranges changed (the tree's diff from each binding's `RenderedAt`,
// `LiveBindings`, tested natively in testkit/tests/read_state_model.rs). What
// this pins is the JavaScript half: the page asks the session on every
// message, and re-runs exactly the bindings it names.
//
// Driven through the ENTRY with a fake socket playing the engine, because
// that is the path an app takes and the one every previous gate missed.

import assert from "node:assert/strict";
import { wrap } from "../../js/wrap.js";

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.message}\n`); }
};
const settle = () => new Promise(r => setTimeout(r, 10));

/**
 * A session whose tree can MOVE, and that names a bound domain as changed
 * only when it did — as `take_stale` does over the tree's diff.
 */
function engineRaw() {
  let rows = [], changed = [], bound = new Set(), foreign = 0, told = 0;
  const unrendered = new Set();
  const session = {
    url: () => "ws://127.0.0.1:17509/",
    set_app() {}, // the app a session is (the forest ruling); a fake needs no namespace
    adopt_loader() {}, // the loader's recording (sdk#386's instrument work); a fake records nothing
    outbound: () => [], sent() {}, reconnected() {}, provision() {},
    take_progress: () => "[]", take_loads: () => "[]", resume() {},
    provisioned: () => true, refused: () => "", exhausted: () => false, unusable: () => "[]",
    tick: () => JSON.stringify({ rolledBack: 0, stalled: null, loadsInFlight: 0 }),
    tick_ms: () => 7777, unsaved_writes: () => 0, cold_due_ms: () => -1, cold_tick() {},
    flush() {},
    bind: d => bound.add(d),
    unbind: d => bound.delete(d),
    // The session reports a changed binding until its re-read COMPLETES and
    // says so (READ-STATE: RenderedAt is set on completion).
    rendered: d => { unrendered.delete(d); },
    scan: () => JSON.stringify(rows),
    root: () => `root-${rows.length}`,
    put: () => { rows = [...rows, { id: `p${rows.length}`, updated: 1 }]; return "{}"; },
    live_mode: () => JSON.stringify({ mode: "HeadSubscribed", why: "", foreignNotifications: foreign }),
    take_stale() {
      told += 1;
      for (const d of changed) unrendered.add(d);
      changed = [];
      return JSON.stringify([...unrendered].filter(d => bound.has(d)));
    },
    unrendered: () => [...unrendered],
    take_state_changed: () => "[]",
    /// Another tab's head is adopted: `domain`'s rows are now `newRows`.
    moved(domain, newRows) { rows = newRows; changed.push(domain); },
    /// A head move that changed nothing this page shows.
    movedElsewhere() {},
    told: () => told,
  };
  return {
    version: () => "0", buildInfo: () => "{}", blockId: () => "", parseBlockId: () => "{}",
    Db: function () { return { define() {}, domains: () => "[]" }; },
    Session: function () { return session; },
    __s: session,
  };
}

async function openDb(raw) {
  const sdk = wrap(raw);
  let deliver;
  const { db } = await sdk.open({ app: "test-app",
    port: 17509,
    fetch: async () => ({ ok: true, arrayBuffer: async () => new ArrayBuffer(4) }),
    connect: (_s, { onEvent }) => {
      deliver = () => onEvent({ kind: "message" });
      return { close() {}, pump() {} };
    },
    setInterval: () => 1, clearInterval: () => {},
  });
  return { db, deliver };
}

await t("**a head move the session names re-runs a live binding, and the new rows show**", async () => {
  const raw = engineRaw();
  const { db, deliver } = await openDb(raw);
  const b = db.bind("tasks", { live: true });
  await settle();
  const toldBefore = raw.__s.told();

  // Another tab's head is adopted, and it changed `tasks`.
  raw.__s.moved("tasks", [{ id: "from-A", updated: 2 }]);
  deliver();
  await settle();

  assert.ok(raw.__s.told() > toldBefore, "the page never asked the session which LIVE ranges changed");
  assert.deepEqual(b.getSnapshot(), [{ id: "from-A", updated: 2 }],
    "the binding did not show the rows the head move carried — with no app call, which is the point");
  assert.deepEqual(raw.__s.unrendered(), [],
    "the binding re-read and never told the session it had RENDERED, so it is reported again on every message");
});

await t("THE CONTROL: a head move that changed nothing shown re-renders nothing, and liveMode says so", async () => {
  // Without this, a binding that re-rendered on every notification would
  // pass the test above whether or not its range changed.
  const raw = engineRaw();
  const { db, deliver } = await openDb(raw);
  const b = db.bind("tasks", { live: true });
  await settle();
  const before = b.getSnapshot();

  raw.__s.movedElsewhere();
  deliver();
  await settle();

  assert.strictEqual(b.getSnapshot(), before,
    "a head move that changed nothing re-rendered the binding; every screen would re-render on every notification about anything");
  assert.equal(typeof db.liveMode().why, "string");
});

await t("a PLAIN binding takes out no watch and is not re-run by a head move", async () => {
  const raw = engineRaw();
  const { db, deliver } = await openDb(raw);
  const b = db.bind("tasks", { live: false });
  await settle();
  const before = b.getSnapshot();

  raw.__s.moved("tasks", [{ id: "from-A", updated: 2 }]);
  deliver();
  await settle();

  assert.strictEqual(b.getSnapshot(), before,
    "a plain binding was refreshed by a head move; that is what `live` is for");
});

await t("a person's OWN write shows at once, without waiting for the network", async () => {
  const raw = engineRaw();
  const { db } = await openDb(raw);
  const b = db.bind("tasks", { live: false });
  await settle();
  let fired = 0;
  b.subscribe(() => { fired += 1; });

  await db.put("tasks", { title: "mine" });
  await settle();

  assert.equal(b.getSnapshot().length, 1,
    "the component never re-rendered on its own write");
  assert.ok(fired > 0, "no listener was called for the client's own write");
});

await t("the snapshot is referentially stable when nothing changed", async () => {
  const raw = engineRaw();
  const { db } = await openDb(raw);
  const b = db.bind("tasks", { live: true });
  await settle();
  const a1 = b.getSnapshot();
  await b.reload();
  assert.strictEqual(b.getSnapshot(), a1,
    "a reload with nothing to show handed back a new array; a caller re-rendering on identity would re-render for ever");
});

process.stdout.write(failures ? `\n${failures} failing\n` : "\nall passing\n");
process.exit(failures ? 1 : 0);
