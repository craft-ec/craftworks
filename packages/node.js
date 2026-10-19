// NODE, a service: the connection to this machine's freenet node, started once, the first time a package asks for it.
// It gives callers `{ core, ask, listen, url }`: the wasm core (framing), one way to send frames and wait for an answer,
// and `listen(fn)` for what the node sends unasked (a followed contract that changed).
// It needs `core-glue`, `core-wasm` and `identity-wasm` (the core is built around the identity delegate's code).
export async function start(ctx) {
  const { Fault } = await ctx.require("fault");
  const glue = await ctx.require("core-glue");
  await glue.default({ module_or_path: await ctx.require("core-wasm") });
  ctx.log("core started", { what: "wire framing (wasm)" });

  // HEX (`identity::hex`, every crate's and every package's): bytes ↔ lower-case hex.
  const hex = b => glue.hex_of(b instanceof Uint8Array ? b : new Uint8Array(b));
  const bytes = h => glue.bytes_of(String(h));
  // A fresh random id: `n` random bytes as hex (every package's new ids).
  const newHex = n => hex(crypto.getRandomValues(new Uint8Array(n)));
  // A DID as bytes (a DID string, or bytes already).
  const didBytes = did => (typeof did === "string" ? glue.did_bytes(did) : did);

  // The node that served this page — at its own origin, so a gateway on https (try.freenet.org) is reached over wss —
  // unless `?node=` names another on this machine.
  const other = ctx.params.get("node");
  const core = new glue.CraftworksCore(await ctx.require("identity-wasm"));
  const url = other
    ? glue.ws_url(location.hostname, Number(other))
    : `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/v1/contract/command?encodingProtocol=native`;

  // THE CONNECTION, and its RECOVERY: a socket that closes (the machine slept, the network changed, the node
  // restarted) is opened again, every 2 s until it is; a question asked meanwhile waits for it (its own deadline still
  // runs). Once back, the page is told (`craftworks:node-back`): what it followed was the old socket's — the door
  // (below) follows each contract again, read once; the page stays.
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
            // What was asked on the lost socket, asked AGAIN on this one (its answer would never come on the old one —
            // each question waited out its whole deadline, audit 10-13). A get, a put of a signed state, an update:
            // each the same again.
            let again = 0;
            for (const w of waiters) for (const f of w.frames ?? []) (ws.send(f), (again += 1));
            if (again) ctx.log("connected", { what: `${again} frame(s) asked on the lost connection sent again` });
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
  // WITHIN a wait (one of `WAIT`'s): `p`'s value, or `dflt` once `ms` passed first — `p` itself goes on (what comes
  // later is the caller's to merge). The one race against a wait.
  const within = (p, ms, dflt = undefined) => Promise.race([p, new Promise(r => setTimeout(r, ms, dflt))]);
  // Send frames and wait for the first answer `match` accepts. Never silent: a timeout is a `transient` fault with its
  // reason (`ms`, and whether the node was being reconnected to). `ms`: WAIT.ask unless said.
  // `signal` (an AbortSignal): no longer wanted — taken off the list at once (a race won by another: its losers), a
  // `cancelled` fault.
  // WAITS RECORDED (owner 10-12: "those long waits also need to be recorded, so we know what stalls, what blocks and
  // what can be optimized"): every question's time to its answer, by what it was (its label with ids taken out) —
  // how many, how many SLOW (over SLOW_MS), the longest, the total, how many never answered. The visit report carries
  // the worst (`waits()`); never content.
  const SLOW_MS = 3000;
  const stats = new Map();
  const label = what => String(what).replace(/[0-9a-f]{8,}/gi, "…").replace(/\d{2,}/g, "#").slice(0, 60);
  const note = (what, ms, timedOut = false) => {
    const k = label(what);
    const x = stats.get(k) ?? { n: 0, slow: 0, max: 0, total: 0, timeouts: 0 };
    x.n += 1;
    x.total += ms;
    x.max = Math.max(x.max, ms);
    if (ms >= SLOW_MS) x.slow += 1;
    if (timedOut) x.timeouts += 1;
    stats.set(k, x);
  };
  const waits = (top = 8) =>
    [...stats]
      .filter(([, x]) => x.slow || x.timeouts)
      .sort((a, b) => b[1].total - a[1].total)
      .slice(0, top)
      .map(([what, x]) => ({ what, ...x, total: Math.round(x.total), max: Math.round(x.max) }));
  const ask = (frames, match, what, ms = WAIT.ask, signal = null) =>
    new Promise((resolve, reject) => {
      const w = { match, what, frames, at: performance.now(), resolve: v => (clearTimeout(t), note(what, performance.now() - w.at), resolve(v)) };
      // Timed out: no longer waited for — taken off the list (left on it, it would match a late answer meant for
      // another, and count as in flight for ever: `idle` would never come).
      const t = setTimeout(() => {
        const i = waiters.indexOf(w);
        if (i >= 0) waiters.splice(i, 1);
        note(what, ms, true);
        reject(new Fault("transient", what, { message: `${what}: no answer from the node in ${ms / 1000} s${lost ? " (reconnecting to it)" : ""}`, d: { ms, reconnecting: lost } }));
      }, ms);
      waiters.push(w);
      signal?.addEventListener("abort", () => {
        clearTimeout(t);
        const i = waiters.indexOf(w);
        if (i >= 0) waiters.splice(i, 1);
        reject(new Fault("cancelled", what, { message: `${what}: no longer wanted` }));
      });
      up.then(s => {
        for (const f of frames) s.send(f);
      });
    });

  // GETS AT ONCE, capped — ONE limit for every get any package makes (files' pieces, tables' blocks): racing asks many
  // together, and past what the node answers they come back refused or late (measured 2026-10-06: a file read with
  // ~144 gets in flight left 294 of 433 pieces unanswered; capped at 32, 68). `slot(f, { urgent })`: f run when a place
  // is free. URGENT (what a page needs NOW: the fragment playback reaches next) goes first and may use every place;
  // the rest (prefetch, keeping) at most half — so what is needed never waits behind a queue of what is merely wanted.
  const MAX_GETS = 32;
  const MAX_BACKGROUND = 16;
  let running = 0;
  let background = 0;
  const urgentQ = [];
  const backgroundQ = [];
  const can = urgent => running < MAX_GETS && (urgent || background < MAX_BACKGROUND);
  const take = urgent => (running++, urgent || background++);
  const grant = () => {
    while (urgentQ.length && can(true)) (take(true), urgentQ.shift()());
    while (backgroundQ.length && can(false)) (take(false), backgroundQ.shift()());
  };
  const slot = async (f, { urgent = false } = {}) => {
    if (can(urgent) && !(urgent ? urgentQ : backgroundQ).length) take(urgent);
    else {
      // QUEUED behind the cap: how long, recorded as its own wait (what blocks, not what the node is slow at).
      const q0 = performance.now();
      await new Promise(r => (urgent ? urgentQ : backgroundQ).push(r)); // `grant` took the place for it
      note(urgent ? "queued for a get (urgent)" : "queued for a get (background)", performance.now() - q0);
    }
    try {
      return await f();
    } finally {
      running--;
      if (!urgent) background--;
      grant();
    }
  };

  // IDLE: nothing asked of the node for WAIT.hint (the one wait policy's) — what BACKGROUND work waits for before each
  // step (unread counts, upkeep, keeping), so a page's own reads never queue behind it.
  // At most `cap` (10-07, measured on Wren's page on B: never 1.5 s with nothing asked — upkeep never started, so its node
  // never learned which spaces to keep): background work then goes ahead, and says so.
  const idle = (quiet = WAIT.hint, cap = 30000) =>
    new Promise(resolve => {
      const t0 = performance.now();
      let since = waiters.length ? 0 : performance.now();
      const check = () => {
        if (waiters.length) since = 0;
        else if (!since) since = performance.now();
        else if (performance.now() - since >= quiet) return resolve();
        if (performance.now() - t0 >= cap) {
          ctx.log("node", { what: `never idle for ${Math.round(cap / 1000)} s (${waiters.length} asked, the oldest ${inFlight()[0]?.what ?? "?"}): background work goes ahead` });
          return resolve();
        }
        setTimeout(check, 100);
      };
      check();
    });

  // THE DOOR (FOUNDATION §2 R6): every contract that can change is read here, ONCE, and then FOLLOWED — the first GET
  // subscribes, the node pushes each change, the core keeps the copy (`held_state`, a tail's view). Its read state, per
  // contract: UNKNOWN (not asked, or silent) → ASKED → HELD; ABSENT (the node answered not found); STALE (the
  // connection was lost: what it followed may have moved). A read of a HELD contract answers `{ held: true }` with no
  // GET — whatever the caller calls the read. ABSENT is asked again only here, on the one backoff (5 s doubling to
  // 5 min), and only while someone follows it (`onContract`). Every GET is said in the trace with its reason (`first`,
  // `stale`, `absent again`): a page's reads are counted where they are made.
  //   contract(hexId, what, { frames, match, ms }) → `{ held: true }` | the node's answer (`get-failed`: absent)
  //   onContract(hexId, fn)   fn(said) on each push, and when an absent one appears
  //   changed(hexId)          this page wrote it: read again on the next ask (its own write's answer is not a push)
  const reads = new Map(); // hexId → { s, p, said, n, at, fns, timer, frames, match, what }
  const AGAIN = n => Math.min(5000 * 2 ** n, 300000);
  const isAbsent = said => said.kind === "get-failed";
  function contract(hexId, what, { frames, match, ms = WAIT.ask }) {
    let r = reads.get(hexId);
    if (!r) reads.set(hexId, (r = { s: "UNKNOWN", n: 0, fns: new Set() }));
    Object.assign(r, { frames, match, what });
    if (r.s === "HELD") return Promise.resolve({ held: true });
    if (r.s === "ASKED") return r.p;
    if (r.s === "ABSENT" && Date.now() - r.at < AGAIN(r.n)) return Promise.resolve(r.said);
    const why = r.s === "UNKNOWN" ? "first" : r.s === "STALE" ? "stale" : "absent again";
    r.n = r.s === "ABSENT" ? r.n + 1 : 0;
    r.s = "ASKED";
    r.gets = (r.gets ?? 0) + 1;
    if (why !== "first") ctx.log("read", { what: `${what} (${why})` });
    r.p = ask(frames(), match, what, ms).then(
      said => {
        r.said = said;
        r.at = Date.now();
        r.s = isAbsent(said) ? "ABSENT" : "HELD";
        if (r.s === "ABSENT" && r.fns.size) again(hexId);
        return said;
      },
      e => ((r.s = "UNKNOWN"), Promise.reject(e)),
    );
    return r.p;
  }
  // ABSENT and followed: asked again on the backoff until it appears (then its followers are told), or nobody follows.
  function again(hexId) {
    const r = reads.get(hexId);
    if (!r || r.timer) return;
    r.timer = setTimeout(async () => {
      r.timer = null;
      if (!r.fns.size || r.s !== "ABSENT") return;
      const said = await ctx.optional("node: contract again", contract(hexId, r.what, r), () => null);
      if (said && !isAbsent(said)) for (const f of r.fns) f(said);
    }, AGAIN(r.n));
  }
  function onContract(hexId, fn) {
    let r = reads.get(hexId);
    if (!r) reads.set(hexId, (r = { s: "UNKNOWN", n: 0, fns: new Set() }));
    r.fns.add(fn);
    if (r.s === "ABSENT") again(hexId);
    return () => r.fns.delete(fn);
  }
  // (Its copy dropped too: the core's steps then want it read, and the door reads it — never a step on the old state.)
  // DATA CHANGED (FOUNDATION §3 `view`): one signal for every view — a followed contract pushed, or this page wrote
  // one — at most once per 150 ms (`craftworks:data`).
  let dataT = null;
  const dataChanged = () => (dataT ??= setTimeout(() => ((dataT = null), dispatchEvent(new CustomEvent("craftworks:data"))), 150));
  const changed = hexId => {
    dataChanged();
    core.take_got(hexId);
    const r = reads.get(hexId);
    if (r && r.s !== "ASKED") r.s = "STALE";
  };
  // A PUSH: the core holds the new state (a tail's view, or `held_state`); the contract is HELD, its followers told.
  listen(said => {
    const hexId = said.kind === "changed" ? said.id : said.kind?.startsWith("tail") ? said.id : null;
    const r = hexId && reads.get(hexId);
    if (!r) return;
    if (r.s !== "ASKED") r.s = "HELD";
    for (const f of r.fns) f(said);
    dataChanged();
  });
  // The connection lost and back: its subscriptions were the old socket's, and what was followed may have moved — each
  // HELD contract read again ONCE (followed again), its answer handed on as a push is (its tables, bags, followers
  // take it). No page reload (10-15).
  addEventListener("craftworks:node-back", () => {
    const back = [...reads].filter(([, r]) => r.s === "HELD");
    for (const [, r] of back) r.s = "STALE";
    if (back.length) ctx.log("read", { what: `the node is back: ${back.length} followed contract(s) read again once` });
    for (const [hexId, r] of back)
      ctx.background("node: follow again", contract(hexId, r.what, r).then(said => {
        if (said?.held || isAbsent(said)) return;
        for (const f of listeners) f(said);
      }));
  });

  // READ a contract (its 32-byte id) so the core holds its state (`held_state`): true when the node has it. Through the
  // door: read once, then followed. `absent: true`: a contract the node does not have is marked "read, not there"
  // (`core.not_there`), so the core's steps tell it from "not read yet".
  // ONE GET of an IMMUTABLE contract (a tree block, a file's piece) — the one door: ONE request per id however many ask
  // at once, through the one cap on gets (`slot`); a "not there" remembered `forget` ms (nothing looked up twice that
  // is known not to be there — a wait that ran out is not remembered). `match`: which answer is its (default: its id's).
  const oneGets = new Map();
  const notThere = new Map();
  function get(idHex, what, { match = null, forget = 0, urgent = false } = {}) {
    const no = notThere.get(idHex);
    if (no && Date.now() - no < forget) return Promise.resolve({ kind: "get-failed", id: idHex });
    if (!oneGets.has(idHex)) {
      const [, frames] = core.frames_get(bytes(idHex));
      const p = ctx
        .optional("node: get", slot(() => ask(frames, match ?? (x => (x.kind === "got" || x.kind === "get-failed") && x.id === idHex), what, WAIT.ask), { urgent }), () => ({ kind: "get-failed", id: idHex, waited: true }))
        .then(a => (a.kind === "get-failed" && !a.waited && notThere.set(idHex, Date.now()), a))
        .finally(() => oneGets.delete(idHex));
      oneGets.set(idHex, p);
    }
    return oneGets.get(idHex);
  }

  async function read(id, what, { absent = false, ms = 30000 } = {}) {
    const hexId = hex(id);
    const said = await contract(hexId, what, { frames: () => core.frames_follow(id)[1], match: s => (s.kind === "got" || s.kind === "get-failed") && s.id === hexId, ms });
    if (said.held) return true;
    if (said.kind !== "got" && absent) core.not_there(id);
    return said.kind === "got";
  }

  // `drop()`: close the connection as a sleep or a network change does (to see the recovery work).
  // WHAT IS IN FLIGHT now — each request's label and how long it has waited (ms): to see what a page waits on.
  const inFlight = () => waiters.map(w => ({ what: w.what, ms: Math.round(performance.now() - w.at) })).sort((a, b) => b.ms - a.ms);
  return { hex, bytes, newHex, didBytes, core, glue, ask, within, get, read, contract, onContract, changed, dataChanged, gets: () => new Map([...reads].map(([k, r]) => [k, { s: r.s, gets: r.gets ?? 0, what: r.what }])), slot, listen, url, drop: () => ws.close(), WAIT, idle, busy: () => waiters.length, inFlight, waits };
}
