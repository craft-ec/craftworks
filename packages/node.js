// NODE, a service: the connection to this machine's freenet node, started once, the first time a package asks for it.
// It registers the signer (the node needs its code to run it) and gives callers `{ core, ask }`.
// It needs `core-glue`, `core-wasm` and `signer`, fetched when it starts.
export async function start(ctx) {
  const glue = await ctx.require("core-glue");
  await glue.default({ module_or_path: await ctx.require("core-wasm") });
  ctx.log("core started", { what: "wire framing (wasm)" });

  // The node that served this page, unless `?node=` names another on this machine.
  const port = Number(ctx.params.get("node") ?? location.port);
  if (!port) throw new Error("no node port: this page was not served by a node, and no ?node= names one");
  const core = new glue.CraftworksCore(await ctx.require("signer"));
  const url = glue.ws_url(port);

  const ws = new WebSocket(url);
  ws.binaryType = "arraybuffer";
  const waiters = [];
  ws.onmessage = ev => {
    const said = JSON.parse(core.take(new Uint8Array(ev.data)));
    if (said.kind === "partial") return;
    const i = waiters.findIndex(w => w.match(said));
    if (i >= 0) waiters.splice(i, 1)[0].resolve(said);
    else ctx.log("node said", { what: JSON.stringify(said).slice(0, 160) });
  };
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

  const reg = await ask(core.frames_register_signer(), s => s.kind === "registered" || s.kind === "refused", "registering the signer");
  if (reg.kind !== "registered") throw new Error(`the node refused the signer: ${reg.said}`);
  ctx.log("signer ready", { what: core.signer_key().slice(0, 16) + "…" });
  return { core, ask, url };
}
