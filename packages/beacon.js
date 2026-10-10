// BEACON, a service: this page EXTENDS the network's beacon (TOKEN §5a: one per network, anyone extends it) — the clock
// every token chain is stamped by. It reads the beacon now and then; when no step has landed for a while (STALE), it
// waits a random moment (so open pages do not all compute at once), reads again, and if still nobody has extended it
// computes the next step in a worker (`beacon-step`: the contract's own sequential work, off this page's thread) and
// puts it, with the outputs log's record of its final heights. Two pages extending at once compute different steps for
// one height: the contract keeps one, nothing is lost. So the clock runs while anyone has Craftworks open.
//
// SET UP by `beacon-config` (`{ params }`: the beacon's params, hex); empty: nothing runs.
//   (await ctx.require("beacon")).status()   // { id, height, extended, last } — what this page did
//   beacon.onHeight(fn)                         // fn(height) each time this page sees it advance (returns: stop)
export async function start(ctx) {
  let cfg = {};
  try {
    cfg = JSON.parse(new TextDecoder().decode(await ctx.require("beacon-config")));
  } catch {}
  if (!cfg.params) return { status: () => ({ off: true }), onHeight: () => () => {} };
  const [node, code, glueJs, stepWasm] = await Promise.all(["node", "beacon-wasm", "beacon-step-glue", "beacon-step-wasm"].map(n => ctx.require(n)));
  const { core, glue, ask } = node;
  const bytes = h => new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
  const params = bytes(cfg.params);
  const id = glue.CraftworksCore.beacon_id(code, params);
  const idBytes = bytes(id);
  const TICK = 5000; // the extender's pace (ms)
  const STALE = 3 * TICK; // no step for this long: someone should extend it
  const state = { id, height: null, extended: 0, last: null };

  // The WORKER: classic (the frame's opaque origin refuses a module worker from a blob), the step's glue inlined.
  const src = `${new TextDecoder().decode(glueJs)}
let ready = null;
self.onmessage = async e => {
  const { wasm, params, state, mix } = e.data;
  ready ??= wasm_bindgen({ module_or_path: wasm });
  await ready;
  const out = wasm_bindgen.next(params, state, mix);
  self.postMessage(out, [out.buffer]);
};`;
  const worker = new Worker(URL.createObjectURL(new Blob([src], { type: "text/javascript" })));
  const step = (st, mix) =>
    new Promise((ok, no) => {
      worker.onmessage = e => ok(e.data);
      worker.onerror = e => no(new Error(e.message || "the beacon's worker failed"));
      worker.postMessage({ wasm: stepWasm, params, state: st, mix });
    });

  // READ it (`node.read`): its state (bytes), or null (not there yet).
  async function read() {
    core.take_got(id);
    return (await node.read(idBytes, "reading the beacon").catch(() => false)) ? core.take_got(id) : null;
  }
  const heightOf = st => (st ? glue.CraftworksCore.beacon_state_height(st) : null);

  let seen = { height: null, at: Date.now() };
  const watchers = new Set();
  const advanced = h => watchers.forEach(fn => fn(h));
  async function tick() {
    const st = await read();
    const h = heightOf(st);
    state.height = h;
    if (h !== seen.height) {
      seen = { height: h, at: Date.now() };
      if (h != null) advanced(h);
      return;
    }
    if (Date.now() - seen.at < STALE) return;
    // Stalled. A random moment first, then read again: another page may have done it.
    await new Promise(r => setTimeout(r, Math.random() * 2 * TICK));
    const again = await read();
    if (heightOf(again) !== h) return;
    const next = await step(again ?? new Uint8Array(0), crypto.getRandomValues(new Uint8Array(32)));
    if (!next.length) return;
    for (const p of core.beacon_puts(code, params, next)) {
      const said = await ask(p.frames, s => (s.kind === "put" && s.key === p.id) || s.kind === "refused", "extending the beacon", 60000).catch(() => null);
      if (said?.kind !== "put") break;
    }
    state.extended += 1;
    state.last = Date.now();
    seen = { height: glue.CraftworksCore.beacon_state_height(next), at: Date.now() };
    state.height = seen.height;
    advanced(seen.height);
  }
  (async function loop() {
    for (;;) {
      await tick().catch(e => ctx.log("beacon", { what: e.message ?? String(e) }));
      await new Promise(r => setTimeout(r, TICK));
    }
  })();
  ctx.log("beacon", { what: `extending ${id.slice(0, 12)}… when it stalls` });
  return {
    status: () => ({ ...state }),
    onHeight(fn) {
      watchers.add(fn);
      return () => watchers.delete(fn);
    },
  };
}
