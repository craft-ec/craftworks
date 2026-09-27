// NODE, a service: the connection to this machine's freenet node, started once, the first time a package asks for it.
// It gives callers `{ core, ask, listen, url }`: the wasm core (framing), one way to send frames and wait for an answer,
// and `listen(fn)` for what the node sends unasked (a followed contract that changed).
// It needs `core-glue`, `core-wasm` and `identity-wasm` (the core is built around the identity delegate's code).
export async function start(ctx) {
  const glue = await ctx.require("core-glue");
  await glue.default({ module_or_path: await ctx.require("core-wasm") });
  ctx.log("core started", { what: "wire framing (wasm)" });

  // The node that served this page, unless `?node=` names another on this machine.
  const port = Number(ctx.params.get("node") ?? location.port);
  if (!port) throw new Error("no node port: this page was not served by a node, and no ?node= names one");
  const core = new glue.CraftworksCore(await ctx.require("identity-wasm"));
  const url = glue.ws_url(location.hostname, port);

  const ws = new WebSocket(url);
  ws.binaryType = "arraybuffer";
  const waiters = [];
  const listeners = [];
  ws.onmessage = ev => {
    const said = JSON.parse(core.take(new Uint8Array(ev.data)));
    if (said.kind === "partial") return;
    const i = waiters.findIndex(w => w.match(said));
    if (i >= 0) waiters.splice(i, 1)[0].resolve(said);
    else if (listeners.length) for (const f of listeners) f(said);
    else ctx.log("node said", { what: JSON.stringify(said).slice(0, 160) });
  };
  const listen = f => listeners.push(f);
  await new Promise((resolve, reject) => {
    ws.onopen = resolve;
    ws.onerror = () => reject(new Error(`could not connect to ${url}`));
  });
  ctx.log("connected", { what: url });

  // Send frames and wait for the first answer `match` accepts. Never silent: a timeout is an error with its reason.
  const ask = (frames, match, what, ms = 15000) =>
    new Promise((resolve, reject) => {
      const t = setTimeout(() => reject(new Error(`${what}: no answer from the node in ${ms / 1000} s`)), ms);
      waiters.push({ match, resolve: v => (clearTimeout(t), resolve(v)) });
      for (const f of frames) ws.send(f);
    });

  return { core, glue, ask, listen, url };
}
