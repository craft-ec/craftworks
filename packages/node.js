// NODE, a service: the connection to this machine's freenet node, started once, the first time a package asks for it.
// It gives callers `{ core, ask, listen, url }`: the wasm core (framing), one way to send frames and wait for an answer,
// and `listen(fn)` for what the node sends unasked (a followed contract that changed).
// It needs `core-glue`, `core-wasm` and `identity-wasm` (the core is built around the identity delegate's code).
export async function start(ctx) {
  const glue = await ctx.require("core-glue");
  await glue.default({ module_or_path: await ctx.require("core-wasm") });
  ctx.log("core started", { what: "wire framing (wasm)" });

  // The node that served this page — at its own origin, so a gateway on https (try.freenet.org) is reached over wss —
  // unless `?node=` names another on this machine.
  const other = ctx.params.get("node");
  const core = new glue.CraftworksCore(await ctx.require("identity-wasm"));
  const url = other
    ? glue.ws_url(location.hostname, Number(other))
    : `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/v1/contract/command?encodingProtocol=native`;

  // THE CONNECTION, and its RECOVERY: a socket that closes (the machine slept, the network changed, the node
  // restarted) is opened again, every 2 s until it is; a question asked meanwhile waits for it (its own deadline still
  // runs). Once back, the page is told (`craftworks:node-back`): what it followed was the old socket's, so the loader
  // loads the page again as soon as nothing would be lost.
  const waiters = [];
  const listeners = [];
  let ws = null;
  let up = null; // a promise: the socket open now
  const onmessage = ev => {
    const said = JSON.parse(core.take(new Uint8Array(ev.data)));
    if (said.kind === "partial") return;
    const i = waiters.findIndex(w => w.match(said));
    if (i >= 0) waiters.splice(i, 1)[0].resolve(said);
    else if (listeners.length) for (const f of listeners) f(said);
    else ctx.log("node said", { what: JSON.stringify(said).slice(0, 160) });
  };
  const open = () =>
    new Promise((resolve, reject) => {
      const s = new WebSocket(url);
      s.binaryType = "arraybuffer";
      s.onmessage = onmessage;
      s.onopen = () => resolve(s);
      s.onerror = () => reject(new Error(`could not connect to ${url}`));
    });
  let lost = false;
  function watch(s) {
    s.onclose = () => {
      if (s !== ws) return;
      ctx.log("node lost", { what: "the connection to the node closed: opening it again" });
      lost = true;
      up = (async () => {
        for (;;) {
          await new Promise(r => setTimeout(r, 2000));
          try {
            ws = await open();
            watch(ws);
            ctx.log("connected", { what: `${url} (again)` });
            dispatchEvent(new CustomEvent("craftworks:node-back"));
            return ws;
          } catch {}
        }
      })();
    };
  }
  const listen = f => listeners.push(f);
  ws = await open();
  watch(ws);
  up = Promise.resolve(ws);
  ctx.log("connected", { what: url });

  // THE WAITS — one policy for every read here: `ask`, an answer a page needs now (a block, a piece, a tail it reads);
  // `answer`, an answer that will be KEPT (a table's place noted, a bag made — "not there" on a real network takes a
  // minute or more, and silence is never kept); `show`, how long a page waits on what it can show without (another
  // writer's feed, a place in a list) before showing what it has, the rest merged as it comes; `hint`, a fact that only
  // spares work (a space's writers bag), never waited on longer.
  const WAIT = { hint: 1500, show: 5000, ask: 30000, answer: 120000 };
  // BACKOFF: `task` again and again — 5 s, doubling, to every 5 min — until it says it is done (`true`).
  const backoff = task => {
    const again = (n = 0) => setTimeout(async () => ((await task().catch(() => false)) ? null : again(n + 1)), Math.min(5000 * 2 ** n, 300000));
    again();
  };
  // Send frames and wait for the first answer `match` accepts. Never silent: a timeout is an error with its reason.
  const ask = (frames, match, what, ms = 15000) =>
    new Promise((resolve, reject) => {
      const t = setTimeout(() => reject(new Error(`${what}: no answer from the node in ${ms / 1000} s${lost ? " (reconnecting to it)" : ""}`)), ms);
      waiters.push({ match, resolve: v => (clearTimeout(t), resolve(v)) });
      up.then(s => {
        for (const f of frames) s.send(f);
      });
    });

  // `drop()`: close the connection as a sleep or a network change does (to see the recovery work).
  return { core, glue, ask, listen, url, drop: () => ws.close(), WAIT, backoff };
}
