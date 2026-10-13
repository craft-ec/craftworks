// THE LOADER, a package: the wrapper fetches it from the loader's own SITE (signed, versioned), so it can change without
// touching the wrapper or any app.
//
// It works like a website: the app's MANIFEST names its packages (by sha256, with the packages each one needs), its
// LAYOUT (what fills the header and the footer) and its PAGES (a route -> what fills the body, and any header/footer
// of its own). The loader only makes the three SLOTS; what fills them is the app's, so a header, a body or a footer is
// edited by publishing the app, never the loader. Only the current page's packages are fetched; everything else
// loads the first time something asks for it (`ctx.require(name)`), once. The node's code is loaded after the first
// page is up even where the page needs none: to FOLLOW the app's and the loader's sites (below).
const VERSION = "30";

// THE APPS, as a phone lists them: by NAME (A–Z), each with its CATEGORY (the manifest's) — every list of apps (Home,
// the header's menus, the spaces panel) in this one order.
const appsOf = manifest => [...(manifest.apps ?? [])].sort((a, b) => String(a.name).localeCompare(String(b.name)));
export async function run(boot) {
  const status = document.getElementById("status");
  const root = document.getElementById("app");
  const loaded = new Map(); // name -> Promise of what `require` gives
  let manifest;

  const ctx = { params: boot.params, log: boot.log, on: boot.on, require, route: "/", app: "", apps: [], uses: [], identityPrior: [], actions: {} };
  globalThis.app = ctx;

  // TELEMETRY, ONE FOR EVERY PACKAGE AND APP (owner 10-11: "one implementation where it collects telemetry, as the one
  // foundation under all apps"): what fails anywhere in the page goes into the day's health bag (`index.health`, sealed
  // to support, once a kind per source per message per hour), tagged with what the loader knows — the route, the app,
  // the service starting — so no app writes its own reports: an uncaught error or rejected promise, a package that does
  // not load, and every log line that says something failed. The apps opened are counted in the heartbeat
  // (`ctx.appsOpened`, `upkeep`'s `seen`). The mechanisms' own reports (keys, joins, reads) stay: they carry diagnosis
  // only they know.
  ctx.appsOpened = new Set();
  let healthIndex = null;
  const tag = s => [...String(s)].reduce((h, c) => (Math.imul(h, 31) + c.charCodeAt(0)) | 0, 7).toString(16).slice(-6);
  const where = () => ({ route: ctx.route, sub: ctx.sub || null, starting: working.at(-1) ?? null });
  function report(k, source, why, d = {}) {
    if (!manifest) return;
    const msg = String(why ?? "").slice(0, 160);
    (healthIndex ??= require("index").catch(() => null)).then(ix => ix?.health?.({ k, s: String(source ?? "page").slice(0, 24), to: tag(msg), why: msg, d: { ...where(), ...d } })).catch(() => {});
  }
  // HOW EACH VISIT WENT (owner 10-12: "not only an error collector" — a page that never finishes says nothing by
  // itself, and a person rarely knows what to report): every page load reports its OUTCOME, success included — how
  // long the page took to start, each page opened and whether it SETTLED (drawn, its work done), was STUCK (still
  // working after STUCK_MS), was SAID (a refusal shown) or FAILED; the faults met, by kind; reconnects to the node; the
  // node questions that were SLOW or never answered and the time spent queued behind the get cap (`node.waits`);
  // guest or member, and for a member how many spaces read. Sent once the first page settles (`visit #1`) and again
  // when the page is hidden with something new (`#2`, …, at most every VISIT_EVERY_MS). A page STUCK reports at once
  // (`stuck`), naming what it waits on. Never content; sealed to support like every report.
  const STUCK_MS = 20000;
  const VISIT_EVERY_MS = 5 * 60000;
  const t00 = performance.now();
  const visit = { pid: [...crypto.getRandomValues(new Uint8Array(3))].map(x => x.toString(16).padStart(2, "0")).join(""), n: 0, boot: null, routes: [], faults: {}, reconnects: 0, sentAt: 0, sentRoutes: 0 };
  addEventListener("craftworks:node-back", () => visit.reconnects++);
  async function sendVisit() {
    if (!manifest || visit.routes.length === visit.sentRoutes) return;
    visit.n += 1;
    visit.sentAt = performance.now();
    visit.sentRoutes = visit.routes.length;
    const sp = await require("space").catch(() => null);
    const me = sp ? await sp.me().catch(() => undefined) : undefined;
    // (Never waited on: a report must go before the page is closed — the spaces as their lifecycles stand now.)
    const waits = loaded.has("node") ? await require("node").then(n => n.waits?.() ?? [], () => []) : [];
    // Where each lifecycle ENDED UP (FOUNDATION.md §3): an account group JOIN_FAILED, a session OPEN_KEYLESS — outcomes
    // even when nothing threw.
    const life = loaded.has("lifecycle") ? await require("lifecycle").then(l => l.snapshot(), () => null) : null;
    const spaces = me ? Object.keys(life?.space ?? {}).length : null;
    const ix = await (healthIndex ??= require("index").catch(() => null));
    await ix?.health?.({ k: "visit", s: "page", to: `${visit.pid}#${visit.n}`, why: "", d: { member: me === undefined ? "unknown" : !!me, spaces, boot: visit.boot, up: Math.round(performance.now() - t00), routes: visit.routes.slice(-12), faults: visit.faults, reconnects: visit.reconnects, waits, life } }).catch(() => {});
  }
  document.addEventListener("visibilitychange", () => {
    // (Hidden before its first page settled — a page that never finished: sent now, it is the visit that matters most.)
    if (document.visibilityState === "hidden" && (!visit.n || performance.now() - visit.sentAt >= VISIT_EVERY_MS)) sendVisit();
  });
  // A PAGE's outcome: watched until its body has no work left and no placeholder (SETTLED), or STUCK_MS passed.
  // Recorded when the page STARTS (`loading` until its components are mounted), and WATCHED from then: a page that
  // never gets past mounting is still in the visit, and still reported stuck.
  function beginPage(route, t0) {
    // (`t0` moves on while the person is asked to log in.)
    const rec = { r: route, ready: null, settled: null, out: "loading" };
    visit.routes.push(rec);
    const busy = () => [...slots.body.querySelectorAll("[aria-busy=true], .cw-cover")].map(el => el.closest("[data-component]")?.dataset.component ?? "?");
    let reportedStuck = false;
    const born = performance.now();
    const check = async () => {
      if (ctx.route !== route && rec.settled === null) return; // left before it settled: its record says so (settled null)
      if (rec.out === "said" || rec.out === "failed") return;
      // Waiting on the person (logging in): not timed — its clock starts again once they are in.
      if (rec.out === "login") return void (performance.now() - born < 600000 && ((t0 = performance.now()), setTimeout(check, 500)));
      const now = rec.ready === null ? ["(mounting)", ...busy()] : busy();
      if (!now.length) {
        rec.settled = Math.round(performance.now() - t0);
        if (rec.out === "stuck") rec.out = "late";
        if (visit.n === 0) setTimeout(sendVisit, 2000);
        return;
      }
      if (!reportedStuck && performance.now() - t0 >= STUCK_MS) {
        reportedStuck = true;
        rec.out = "stuck";
        const waits = await require("node").then(n => n.inFlight().slice(0, 3).map(w => ({ what: String(w.what).replace(/[0-9a-f]{8,}/gi, "…").slice(0, 60), ms: w.ms })), () => []);
        report("stuck", route, `${route} still working after ${STUCK_MS / 1000} s`, { on: [...new Set(now)].slice(0, 6), waits, pid: visit.pid });
        if (visit.n === 0) sendVisit();
      }
      if (performance.now() - t0 < 120000) setTimeout(check, 500);
    };
    setTimeout(check, 300);
    return rec;
  }
  // MOUNTED: its components drawn (what is still arriving is watched by `beginPage`).
  function watchPage(rec, t0) {
    rec.ready = Math.round(performance.now() - t0);
    if (rec.out === "loading") rec.out = "drawn";
    if (visit.boot === null) visit.boot = Math.round(performance.now() - t00);
  }
  const stackTop = e => String(e?.stack ?? "").split("\n").slice(1, 3).map(l => l.trim().replace(/blob:[^ )]+/g, "blob")).join(" | ").slice(0, 160);
  // FAULTS (FOUNDATION.md §4, `fault`): anything thrown is classified by KIND and reported by the kind's policy — an
  // extension's own error, an expected refusal, a guest asked to log in, an abort are never reported. The service
  // loads once the manifest is read; what fails before is not reported (`report` needs the manifest anyway).
  let faults = null;
  // (Asked before the manifest is read: nothing yet, and nothing kept — asked again later.)
  const fault = () => (manifest ? (faults ??= manifest.packages?.fault ? require("fault").then(f => (f.reportTo(report), f)).catch(() => null) : Promise.resolve(null)) : Promise.resolve(null));
  let faultNow = null; // the service, once loaded: `fail` classifies at once
  // The LIFECYCLES (`lifecycle`), once the manifest names it: each body component's app machine moved through it.
  const lifeOf = () => (manifest?.packages?.lifecycle ? require("lifecycle").catch(() => null) : null);
  const met = (source, e, via, extra = {}) =>
    fault().then(f => {
      if (f) {
        const c = f.met(source, e, { via, stack: stackTop(e) || undefined, ...extra });
        visit.faults[c.kind] = (visit.faults[c.kind] ?? 0) + 1;
        return c;
      }
      // (A site without the service: as before.)
      if (!extensions(e) && !e?.expected && !/nobody is logged in/.test(e?.message ?? "")) report(via, source, e?.message ?? e, { stack: stackTop(e) });
    });
  // A BROWSER EXTENSION's own error (its stack wholly in the extension: a wallet extension failing to connect) is not
  // this site's: not reported.
  const extensions = e => {
    const lines = String(e?.stack ?? "").split("\n").slice(1).filter(l => l.trim());
    return lines.length > 0 && lines.every(l => /(chrome|moz|safari(-web)?)-extension:\/\//.test(l));
  };
  addEventListener("error", e => met(working.at(-1) || ctx.route, e.error ?? new Error(e.message), "page-error"));
  addEventListener("unhandledrejection", e => met(working.at(-1) || ctx.route, e.reason, "page-rejection"));
  // A FAILURE MET (`ctx.failed(step, error, what)`): said in the trace as any line, and REPORTED by its fault's kind
  // (FOUNDATION.md §4) — the one way a package says something failed. (Log lines are no longer read for words like
  // "failed": a failure is reported because it IS one, not because of how it is worded.)
  ctx.log = (step, detail = {}) => boot.log(step, detail);
  ctx.failed = (step, e, what = "") => {
    boot.log(step, { what: what || String(e?.message ?? e) });
    met(step, e, "failed", what ? { doing: String(what).slice(0, 120) } : {});
  };
  // WHAT A PERSON READS for a failure: its kind's words (`fault.say`), never a stack.
  ctx.say = e => faultNow?.say(e) ?? String(e?.message ?? e);

  // A package, fetched and checked by the wrapper, the first time anything asks for it. What it gives depends on its
  // kind: `bytes` (wasm, data), `module` (an imported JS module), `service` (a module whose `start(ctx)` runs once and
  // whose result is shared).
  // WHO ASKED FOR WHAT: the component mounting or the service starting when a package is first asked for. What the
  // Stack tab shows for a package no slot names (a service, a component's own parts).
  const working = [];
  const askedBy = new Map();
  function require(name) {
    if (!loaded.has(name)) {
      askedBy.set(name, working.at(-1) ?? null);
      // (A service that needs an account, asked before login, is not a failure: its fault's kind says so.)
      loaded.set(name, load(name).catch(e => (name !== "fault" && met(name, e, "package-failed"), Promise.reject(e))));
    }
    return loaded.get(name);
  }

  // RACING (the loader's own parts, from its site, loaded once): every package is a PIECE SET, k data + m parity
  // pieces, each its own container. All are asked at once; the first k that verify (each against its sha256) are
  // decoded back into the package, which is checked against the package's own sha256. One slow or missing piece holds
  // nothing up. The fetch is the SDK's (served.js: a node's 503 is "not yet", asked again), the decoding its decoder.
  let racingParts;
  function racing() {
    racingParts ??= (async () => {
      const t = performance.now();
      const served = await import(new URL("served.js", boot.loaderBase).href);
      const pieces = await import(new URL("pieces.js", boot.loaderBase).href);
      const d = await boot.get(new URL("decoder.wasm", boot.loaderBase), "decoder");
      const dec = await pieces.decoder(d.bytes);
      ctx.log("loaded", { what: "racing (the loader's: served.js, pieces.js, decoder.wasm)", bytes: d.bytes.length, ms: Math.round(performance.now() - t) });
      return { raceK: served.raceK, openPieces: pieces.openPieces, dec };
    })();
    return racingParts;
  }

  const hex = buf => [...new Uint8Array(buf)].map(x => x.toString(16).padStart(2, "0")).join("");

  // A PIECE never changes (its address is its content's): the browser's copy is used when it has one — a reload asks
  // the node for nothing it already holds. The package's hash is checked after decoding anyway; a mismatch asks the
  // node again, bypassing the copy.
  async function fetchPackage(p, name, fresh = false) {
    const { raceK, openPieces, dec } = await racing();
    const t = performance.now();
    const cache = fresh ? "reload" : "force-cache";
    const spec = {
      k: p.k,
      m: p.m,
      pieces: p.pieces.map(x => ({ url: new URL(`/v1/contract/web/${x.address}/piece`, location.href).href, sha256: x.sha256 })),
    };
    const raced = await raceK(spec, {
      // The browser's copy only when it is the piece: a FAILED answer it kept (a "not found" from before the node
      // served it) is asked for again from the node, which replaces it — never replayed on every load.
      fetch: async (u, init) => {
        const r = await fetch(u, { ...(init ?? {}), cache });
        return r.ok || cache === "reload" ? r : fetch(u, { ...(init ?? {}), cache: "reload" });
      },
      onWait: w => {
        const what = `${w.verified} of ${w.k} pieces`;
        ctx.log("waiting", { what: `${name}: ${what}` });
        for (const p of placeholders.values()) p.say(`Loading ${p.dataset.name}: ${name}, ${what}…`);
      },
    });
    const { files } = openPieces(dec, { k: p.k, m: p.m, payload: p.payload, bundle_len: p.bundle_len }, raced.pieces);
    const bytes = files.get(p.file);
    if (!bytes) throw new Error(`${name}: its pieces decode to no file "${p.file}"`);
    const got = hex(await crypto.subtle.digest("SHA-256", bytes));
    if (got !== p.sha256) {
      if (!fresh) return fetchPackage(p, name, true);
      throw new Error(`${name}: decoded to bytes hashing ${got.slice(0, 12)}…, not ${p.sha256.slice(0, 12)}…`);
    }
    return { bytes, ms: Math.round(performance.now() - t), used: raced.pieces.filter(Boolean).length, asked: raced.asked.length };
  }

  // A package's ENTRY (its pieces), in its own file of the site named by the package's hash: read once, and only when
  // it is needed — the manifest itself names each package by kind and hash alone. (A manifest from before: inline.)
  const entries = new Map();
  const read = new Map(); // name → its entry, once read (what the trace's Stack shows: its pieces)
  function entry(name) {
    const head = manifest.packages[name];
    if (!head) return Promise.reject(new Error(`package "${name}" is not in this app's manifest`));
    if (head.pieces) return (read.set(name, head), Promise.resolve(head));
    if (!entries.has(name))
      entries.set(
        name,
        boot
          .get(new URL(`p/${head.sha256.slice(0, 16)}.json`, location.href), `${name}'s entry`)
          .then(r => ({ ...head, ...JSON.parse(new TextDecoder().decode(r.bytes)) }))
          .then(e => (read.set(name, e), e)),
      );
    return entries.get(name);
  }

  async function load(name) {
    const p = await entry(name);
    for (const d of p.needs ?? []) await require(d);
    const { bytes, ms, used, asked } = await fetchPackage(p, name);
    ctx.log("loaded", { what: `${name} (${used} of ${p.k}+${p.m} pieces; asked ${asked})`, bytes: bytes.length, hash: p.sha256.slice(0, 12), ms });
    if (p.kind === "bytes") return bytes;
    const mod = await boot.importBytes(bytes);
    if (p.kind === "service") {
      const t = performance.now();
      working.push(name);
      const given = await mod.start(ctx).finally(() => working.pop());
      ctx.log("started", { what: name, ms: Math.round(performance.now() - t) });
      return given;
    }
    return mod;
  }

  // THE SLOTS: made once. Each holds the components the layout or the page names, mounted in order.
  const slots = {};
  // The SIDE slot (the layout's `side`: a rail of spaces) sits beside the body, the whole height between the bars.
  for (const [name, tag] of [["header", "header"], ["side", "aside"], ["body", "main"], ["footer", "footer"]]) {
    slots[name] = document.createElement(tag);
    slots[name].className = `slot-${name}`;
    root.append(slots[name]);
  }
  const filled = { header: null, side: null, footer: null };

  // A component still on its way shows the theme's placeholder in its place, until it has drawn itself (it replaces
  // its element's contents) or its mount is over. Every component of a slot gets one at once: what is coming, shown.
  // A component's LOADING is INHERITED from the capabilities it uses: each component gets its own view of them
  // (`ctx.require` in the ctx it is mounted with), and every call through that view still running — and every result
  // that says it is still arriving (a `settled` promise: a table, a room) — is that component's work, and marks its panel
  // busy (`aria-busy`: the theme's line). The placeholder covers it only until its MOUNT is over (it drew itself then):
  // work it starts after — a background tick, a table still arriving, a subscription — never covers what it drew.
  // `ctx.pending(promise)`: anything else a component waits on.
  const CAP = 20000;
  function tracker(el) {
    let n = 0;
    const work = p => {
      const w = Promise.race([Promise.resolve(p), new Promise(r => setTimeout(r, CAP))]).catch(() => {});
      n += 1;
      el.setAttribute("aria-busy", "true");
      w.finally(() => {
        n -= 1;
        if (n) return;
        el.removeAttribute("aria-busy");
      });
      return p;
    };
    return { work };
  }
  // A capability as a component sees it: its functions, each call counted as the component's work. What a call gives
  // back is seen the same way (a table's own calls count too), and a result's `settled` is waited on. Only plain
  // objects are viewed (a capability, a table); data, bytes and DOM pass as they are.
  const plain = v => v && typeof v === "object" && [Object.prototype, null].includes(Object.getPrototypeOf(v));
  function view(v, work) {
    if (!plain(v) || !Object.values(v).some(x => typeof x === "function")) return v;
    return new Proxy(v, {
      get(t, k) {
        const x = Reflect.get(t, k, t);
        if (typeof x !== "function") return x;
        return (...args) => {
          const r = x.apply(t, args);
          if (!(r instanceof Promise)) return plain(r) ? view(r, work) : r;
          return work(
            r.then(got => {
              if (got?.settled instanceof Promise) work(got.settled);
              return view(got, work);
            }),
          );
        };
      },
    });
  }
  const placeholders = new Map(); // element -> placeholder
  // CLOSING a component (its slot filled anew): its `ctx.signal` aborted — listeners added with it, fetches and timers
  // it tied to it end with it (FOUNDATION.md §3: an app's work ends when it closes).
  const mountedIn = { header: [], side: [], body: [], footer: [] }; // slot → [{ name, ac }]
  async function fill(slot, names) {
    for (const m of mountedIn[slot].splice(0)) {
      m.ac.abort();
      if (slot === "body") lifeOf()?.then(l => l?.become("app", m.name, "closed", {})).catch(() => {});
    }
    slots[slot].replaceChildren();
    const theme = manifest.theme ? await require(manifest.theme) : null;
    const els = names.map(name => {
      const el = document.createElement("section");
      el.dataset.component = name;
      // The body's components and the side's (the rail) show the theme's placeholder until drawn.
      if (theme?.loading && (slot === "body" || slot === "side")) {
        const p = theme.loading(`Loading ${name}…`);
        p.dataset.name = name;
        p.classList.add("cw-cover");
        placeholders.set(el, p);
        el.append(p);
        // Put back if the component's drawing took it out, until it is settled.
        const keep = new MutationObserver(() => placeholders.has(el) && !el.contains(p) && el.append(p));
        keep.observe(el, { childList: true });
        p.done = () => {
          keep.disconnect();
          placeholders.delete(el);
          p.remove();
        };
      }
      slots[slot].append(el);
      return el;
    });
    for (const [i, name] of names.entries()) {
      const el = els[i];
      const mod = await require(name);
      const t = performance.now();
      working.push(name);
      const p = placeholders.get(el);
      const track = tracker(el);
      const own = Object.create(ctx);
      own.require = n => require(n).then(v => view(v, track.work));
      own.pending = track.work;
      const ac = new AbortController();
      own.signal = ac.signal;
      mountedIn[slot].push({ name, ac });
      if (slot === "body") lifeOf()?.then(l => l?.become("app", name, "opening", {})).catch(() => {});
      const mounted = Promise.resolve(mod.mount(own, el)).finally(() => working.pop());
      await mounted.then(
        () => slot === "body" && lifeOf()?.then(l => l?.become("app", name, "open", {})).catch(() => {}),
        e => slot === "body" && lifeOf()?.then(l => l?.become("app", name, "failed", { why: String(e?.message ?? e).slice(0, 80) })).catch(() => {}),
      );
      // Its placeholder goes once its mount is over; the busy line stays while its own work runs.
      p?.done();
      await mounted;
      ctx.log("mounted", { what: `${slot}: ${name}`, ms: Math.round(performance.now() - t) });
    }
  }

  // What a page shows: `["a", "b"]` is its body; `{ body, header?, side?, footer? }` overrides the layout's slots.
  // A route names its page by the LONGEST page route it starts with; the rest is the page's SUB-PAGE (`#/account/nodes`:
  // the page `/account`, `ctx.sub` "nodes").
  // A SPACE: `/s/<space id>/…` is the rest's page IN that space (`ctx.space`: its id; none: the personal space), and
  // `/s/<space id>` alone the space's own page, `/space` (its Home). DISCOVER (the public network, nobody's space):
  // `/discover/…`, `ctx.space` "discover".
  const shell = msg => {
    try {
      if (parent !== window) parent.postMessage({ __freenet_shell__: true, ...msg }, "*");
    } catch {}
  };
  const shellAddress = () => location.hash && shell({ type: "hash", hash: location.hash });
  async function show(at) {
    const inSpace = at.match(/^\/s\/([0-9a-f]{64})(\/.*)?$/) ?? at.match(/^\/(discover)(\/.*)?$/);
    ctx.space = inSpace ? inSpace[1] : null;
    const full = inSpace ? (inSpace[2] && inSpace[2] !== "/" ? inSpace[2] : "/space") : at;
    // A page MOVED (the manifest's `moved`: an app renamed): its old links open the new page, the address made the new.
    const moved = Object.entries(manifest.moved ?? {}).find(([from]) => full === from || full.startsWith(`${from}/`));
    if (moved) {
      history.replaceState(history.state, "", `#${inSpace ? at.slice(0, at.length - (inSpace[2] ?? "").length) : ""}${moved[1]}${full.slice(moved[0].length)}`);
      return show(location.hash.replace(/^#/, ""));
    }
    const route = Object.keys(manifest.pages).filter(r => full === r || full.startsWith(r === "/" ? "/" : `${r}/`)).sort((a, b) => b.length - a.length)[0] ?? "/";
    const raw = manifest.pages[route];
    const page = Array.isArray(raw) ? { body: raw } : raw;
    const t0 = performance.now();
    const rec = beginPage(route, t0);
    ctx.log("page", { what: full });
    // The wrapper's "Loading…" line: gone once a page starts (its components show their own placeholders, and how long
    // it took is in the trace); it comes back for a failure.
    status.hidden = status.className !== "bad";
    ctx.route = route;
    ctx.sub = full.slice(route.length).replace(/^\//, "");
    // Every entry this page will need, asked for at once (the manifest lists them): no chain of one-by-one reads.
    for (const n of manifest.needs?.[route] ?? []) entry(n).catch(() => {});
    for (const slot of ["header", "side", "footer"]) {
      const names = page[slot] ?? manifest.layout?.[slot] ?? [];
      const key = JSON.stringify(names);
      // A header or footer the new page shares is kept, not rebuilt; each shown component hears the new route.
      if (filled[slot] !== key) {
        await fill(slot, names);
        filled[slot] = key;
      }
    }
    // ACCESS (the manifest's, per page, as any website's routes): a `login` page opens only for someone logged in — the
    // login dialog if nobody is, home if it is closed. Every other page is public: what it shows is the access rules'.
    // (Waiting on the PERSON — the login dialog — is not the page stuck: `login` while asked.)
    if (page.access === "login") rec.out = "login";
    if (page.access === "login" && !(await (await require("login")).session())) {
      if (location.hash.replace(/^#/, "") === at) location.hash = "#/";
      return;
    }
    if (rec.out === "login") rec.out = "loading";
    await fill("body", page.body ?? []);
    dispatchEvent(new CustomEvent("craftworks:route", { detail: route }));
    shellAddress();
    ctx.appsOpened.add(route);
    ctx.log("page ready", { what: route, ms: Math.round(performance.now() - t0) });
    watchPage(rec, t0);
  }

  // SAVING, said in ONE place (FOUNDATION.md §3): "Saving…" while any write is on its way, "Not saved" with why when
  // one fails (the fault's words), nothing once all are held. No app says it on its own.
  {
    let n = 0;
    let line = null;
    let hide = null;
    const say = (text, bad) => {
      line ??= Object.assign(document.createElement("div"), { role: "status" });
      line.style.cssText = `position:fixed;left:16px;bottom:8px;z-index:2147483001;font-size:12px;padding:3px 10px;border-radius:999px;background:Canvas;color:${bad ? "#b3261e" : "CanvasText"};border:1px solid #8886;opacity:.85`;
      line.textContent = text;
      if (!line.isConnected) document.body.append(line);
      clearTimeout(hide);
      hide = text ? (bad ? setTimeout(() => line.remove(), 8000) : null) : (line.remove(), null);
    };
    addEventListener("craftworks:save", e => {
      const d = e.detail ?? {};
      if (!d.user) return; // background writes (drafts, upkeep, records): not the person's saves
      if (d.s === "saving") (n += 1), say("Saving…");
      else {
        n = Math.max(0, n - 1);
        if (d.s === "failed") say(`Not saved: ${(() => { try { return faultNow.say(new faultNow.Fault(d.kind, d.why)); } catch { return d.why; } })()}`, true);
        else if (!n) say("");
      }
    });
  }

  // THE TRACE: always recorded (the wrapper's log), shown only on demand. A small toggle; the `trace` component is
  // fetched the first time it is opened.
  // What the Stack tab shows: the app as its manifest names it, where each part comes from, and what is loaded now.
  const stack = {
    manifest: () => manifest,
    appSite: () => location.pathname.split("/")[4] ?? "",
    loaderSite: () => boot.loaderBase.pathname.split("/")[4] ?? "",
    loaderVersion: VERSION,
    loaded: name => loaded.has(name),
    // Its pieces (k data + m parity), once its entry was read (null: not needed yet).
    pieces: name => (read.has(name) ? `${read.get(name).k}+${read.get(name).m}` : null),
    askedBy: name => askedBy.get(name),
    // GET THE NEWEST: this node follows (subscribes to) the app's site and the loader's — a node holding a copy answers
    // a plain GET from it — then the page loads again. Returns what each said.
    async fresh() {
      const { core, glue, ask } = await require("node");
      const out = [];
      for (const site of [stack.appSite(), stack.loaderSite()].filter(Boolean)) {
        const [id, frames] = core.frames_follow(glue.CraftworksCore.id_bytes(site));
        const said = await ask(frames, x => (x.kind === "got" || x.kind === "get-failed") && x.id === id, "getting the newest", 60000).catch(e => ({ kind: "failed", said: e.message }));
        ctx.log("newest", { what: `${site.slice(0, 8)}…: ${said.kind}` });
        out.push({ site, kind: said.kind });
      }
      return out;
    },
  };

  function traceToggle() {
    const b = document.createElement("button");
    b.textContent = "trace";
    b.title = "Show what this page loaded, in order";
    b.style.cssText = "position:fixed;right:16px;bottom:6px;font-size:12px;padding:4px 8px;opacity:.7;z-index:2147483001";
    const panel = document.createElement("section");
    panel.id = "trace";
    panel.hidden = true;
    // An OVERLAY over the app, never in its layout: the page underneath stays as it is.
    panel.style.cssText =
      "position:fixed;right:12px;bottom:44px;width:min(760px,calc(100vw - 24px));max-height:70vh;overflow:auto;" +
      "background:Canvas;color:CanvasText;border:1px solid #8886;border-radius:8px;box-shadow:0 8px 28px #0005;" +
      "padding:10px 12px;font-size:.85rem;z-index:2147483000";
    document.body.append(panel, b);
    let mounted = false;
    b.onclick = async () => {
      panel.hidden = !panel.hidden;
      b.textContent = panel.hidden ? "trace" : "hide trace";
      if (!panel.hidden && !mounted) {
        mounted = true;
        const { bytes, ms } = await boot.get(new URL("trace.js", boot.loaderBase), "trace");
        ctx.log("loaded", { what: "trace (the loader's)", bytes: bytes.length, ms });
        (await boot.importBytes(bytes)).mount(ctx, panel, stack);
      }
    };
  }

  // IN-APP LINKS ARE THE LOADER'S. The node's page shell turns a same-site link click into a "navigate" and, for a hop
  // inside the same app, CLOSES ALL of the app's WebSockets first (freenet-core shell_bridge.js, same-contract
  // navigate), taking the frame to be replaced. A `#` link replaces nothing, so the app would lose its node connection
  // on every page change. A click on a `#` link is taken here, at the window, before the shell's listener on the
  // document, and becomes a plain hash change. Every other link stays the shell's.
  addEventListener(
    "click",
    e => {
      if (e.defaultPrevented || e.button || e.ctrlKey || e.metaKey || e.shiftKey || e.altKey) return;
      const a = e.target instanceof Element ? e.target.closest("a[href^='#']") : null;
      if (!a) return;
      e.preventDefault();
      e.stopImmediatePropagation();
      location.hash = a.getAttribute("href");
    },
    true,
  );

  const route = () => location.hash.replace(/^#/, "") || "/";
  try {
    ctx.log("loader started", { what: `loader v${VERSION}` });
    const m = await boot.manifest();
    manifest = JSON.parse(new TextDecoder().decode(m.bytes));
    // The SITE's version (its manifest's hash): what every health report names — one package's hash cannot tell two
    // releases apart that left it unchanged.
    ctx.siteV = hex(await crypto.subtle.digest("SHA-256", m.bytes)).slice(0, 12);
    // Each package's VERSION (its sha256, as the manifest names it): what a record of a critical action carries.
    ctx.shaOf = n => manifest?.packages?.[n]?.sha256 ?? null;
    ctx.app = manifest.app;
    // The site's APPS, as its manifest lists them ({ name, icon, route }): what a desktop shows.
    ctx.apps = appsOf(manifest);
    // The kinds of the person's data the site USES (["notes", "pins"]): asked for together, in one prompt.
    ctx.uses = manifest.uses ?? [];
    // Earlier builds of the identity delegate (`<key>:<code hash>`, newest first): where a member made before this
    // build still is, until its next login moves it here.
    ctx.identityPrior = manifest.identity_prior ?? [];
    document.title = manifest.app;
    // The browser's TAB and ADDRESS BAR are the node's shell page's (this page is its iframe): told by its own message
    // (`__freenet_shell__`), the address after every move — a reload or a copied link opens where the person is.
    shell({ type: "title", title: manifest.app });
    addEventListener("craftworks:address", shellAddress);
    // The app's THEME (its manifest names it): applied before anything mounts, so nothing shows unstyled.
    // FAULTS first: what fails from here on is classified and reported by kind.
    faultNow = await fault();
    if (manifest.theme) await require(manifest.theme);
    // The SESSION layer (`login`), on every page as any site's: it answers a guest's "log in to do that" anywhere, and
    // guards the `login` pages.
    if (manifest.packages.login) require("login").catch(() => {});
    // DRAFTS (save on change: what is typed outlives the tab — FOUNDATION.md §3, "Close at any time").
    if (manifest.packages.drafts) require("drafts").catch(() => {});
    ctx.log("manifest read", {
      what: `${manifest.app}: ${Object.keys(manifest.packages).length} packages, ${Object.keys(manifest.pages).length} pages, site ${ctx.siteV}`,
      bytes: m.bytes.length,
      ms: m.ms,
    });
    traceToggle();
    addEventListener("hashchange", () => show(route()).catch(fail));
    await show(route());
    setTimeout(() => follow(m.bytes).catch(e => ctx.log("newest", { what: `following: ${e.message}` })), 2000);
  } catch (e) {
    fail(e);
  }

  // FOLLOWING: a node answers a plain GET of a site from the copy it holds, so a node that fetched an app once would
  // keep serving that version. Every page load has this node follow (subscribe to) the app's site and the loader's:
  // the network then sends it each new version. When one lands — the node says the site changed, or its manifest
  // (read again every minute and on focus) differs from the one this page runs — the page loads it by itself.
  async function follow(running) {
    // IN PLACE: the new manifest's changed packages all COMPONENTS (kind `module`), and what the site asks of the
    // person unchanged — the changed ones forgotten, the manifest swapped, every slot mounted again. False: reload.
    const inPlace = bytes => {
      let next;
      try {
        next = JSON.parse(new TextDecoder().decode(bytes));
      } catch {
        return false;
      }
      const same = k => JSON.stringify(next[k] ?? null) === JSON.stringify(manifest[k] ?? null);
      if (!["uses", "identity_prior", "theme", "layout"].every(same)) return false;
      const names = [...new Set([...Object.keys(manifest.packages), ...Object.keys(next.packages)])];
      const changed = names.filter(n => manifest.packages[n]?.sha256 !== next.packages[n]?.sha256);
      if (changed.some(n => (next.packages[n] ?? manifest.packages[n]).kind !== "module")) return false;
      manifest = next;
      ctx.apps = appsOf(manifest);
      for (const n of changed) loaded.delete(n), entries.delete(n), read.delete(n);
      for (const slot of Object.keys(filled)) filled[slot] = null;
      ctx.log("newest", { what: `taken in place (no reload): ${changed.join(", ") || "the manifest"}` });
      show(route()).catch(fail);
      return true;
    };
    const node = await require("node");
    const sites = [stack.appSite(), stack.loaderSite()].filter(Boolean);
    let told = false;
    // A newer version: TAKEN BY ITSELF, at the first moment nothing would be lost — nobody typing, no text waiting in a
    // box, no dialog open; looked at every 2 s, and taken at once when the page changes or goes out of view.
    // IN PLACE when only COMPONENTS changed (`inPlace`): the services — the node, the tables read, the keys — kept,
    // nothing read again. Anything else (a service, data, the loader): the page loaded again.
    const newer = (next = null) => {
      if (told) return;
      told = true;
      ctx.log("newest", { what: "a newer version is on this node: taking it when nothing would be lost" });
      const editable = el => el && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName));
      // With DRAFTS, what is typed is saved (flushed before the reload) and put back after it: AN UPDATE IS A RESUME —
      // only an open dialog (a login: never drafted) holds it. Without them: as before, nothing typed is lost.
      const drafted = loaded.has("drafts");
      const busy = () =>
        !!document.querySelector("dialog[open]") ||
        (!drafted &&
          (editable(document.activeElement) ||
            [...document.querySelectorAll("textarea, input[type=text], input:not([type])")].some(x => x.value && !x.closest("header, [role=search]") && x.type !== "search")));
      let done = false;
      const go = async () => {
        if (done) return;
        // Everything typed written first; a draft not saved holds the update until it is.
        if (drafted && !(await require("drafts").then(d => d.flush(), () => false))) return;
        if (done) return;
        done = true;
        clearInterval(poll);
        removeEventListener("hashchange", onRoute);
        document.removeEventListener("visibilitychange", onHidden);
        if (next && inPlace(next)) {
          running = next;
          told = false;
          return;
        }
        const n = document.createElement("div");
        n.setAttribute("role", "status");
        n.style.cssText =
          "position:fixed;left:50%;bottom:44px;transform:translateX(-50%);z-index:2147483002;background:Canvas;color:CanvasText;" +
          "border:1px solid #8886;border-radius:999px;box-shadow:0 4px 18px #0004;padding:6px 14px;font-size:.9rem";
        n.textContent = "Updating to the newest version…";
        document.body.append(n);
        setTimeout(() => location.reload(), 400);
      };
      const tryNow = () => !busy() && go();
      const onRoute = () => go();
      const onHidden = () => document.visibilityState === "hidden" && go();
      const poll = setInterval(tryNow, 2000);
      addEventListener("hashchange", onRoute);
      document.addEventListener("visibilitychange", onHidden);
      tryNow();
    };
    const check = async () => {
      const r = await fetch(new URL("manifest.json", location.href), { cache: "no-store" }).catch(() => null);
      if (!r?.ok) return;
      const now = new Uint8Array(await r.arrayBuffer());
      if (now.length !== running.length || now.some((x, i) => x !== running[i])) newer(now);
    };
    node.listen(said => {
      if (said.kind !== "changed" || !sites.includes(said.key)) return;
      if (said.key === stack.loaderSite()) newer();
      else check();
    });
    await stack.fresh();
    await check();
    // And the node's copy looked at again now and then, and when the page comes back into view: however it got there.
    const again = () => !told && check().catch(() => {});
    // The node connection came back after a loss: what the page followed was on the old one — loaded again, the same
    // careful way (nothing lost).
    addEventListener("craftworks:node-back", () => {
      ctx.log("newest", { what: "the node is back: loading the page again when nothing would be lost" });
      told = false;
      newer();
    });
    setInterval(again, 60000);
    addEventListener("focus", again);
  }

  // SHOWN AGAIN (FOUNDATION.md §2, R3): a page that FAILED because something was not readable yet or had not arrived
  // is shown again after a pause, a few times.
  let again = null;
  const showAgain = why => {
    clearTimeout(again);
    again = setTimeout(() => {
      ctx.log("page", { what: `shown again: ${why}` });
      filled.header = filled.side = filled.footer = null;
      show(route()).catch(fail);
    }, 300);
  };
  // Tables opened before their key was held (a words login: its keys arrived after the page opened) — every capability
  // built on them (roles, spaces, names) RESUMED: the page saved (drafts) and loaded again where the person is. An
  // unreadable table that opens now changes in place (no resume).
  addEventListener("craftworks:reopen", async () => {
    if (loaded.has("drafts") && !(await require("drafts").then(d => d.flush(), () => false))) return showAgain("tables opened again (a draft not saved: no resume)");
    ctx.log("page", { what: "resumed: tables opened before their key was held, opened again with it" });
    location.reload();
  });
  let retries = 0;
  function fail(e) {
    // Whatever was thrown, said (a rejection with no Error showed "undefined").
    const said = e?.message ?? (typeof e === "string" ? e : (() => { try { return JSON.stringify(e); } catch { return String(e); } })());
    // An EXPECTED refusal (a members-only app opened from outside) or a guest asked to log in is the page's answer:
    // said as it is, never reported as a fault.
    const kind = faultNow?.classify(e).kind ?? (e?.expected ? "refused-expected" : null);
    if (kind === "refused-expected" || kind === "login-needed") {
      const r = visit.routes.at(-1);
      if (r?.out === "loading") r.out = "said";
      else visit.routes.push({ r: ctx.route, out: "said" });
      ctx.log("page", { what: `${location.hash}: ${said}` });
      status.textContent = faultNow?.say(e) ?? said;
      status.className = "said";
      status.hidden = false;
      return;
    }
    const r = visit.routes.at(-1);
    if (r?.out === "loading") r.out = "failed";
    else visit.routes.push({ r: ctx.route, out: "failed" });
    met(location.hash || "/", e, "page-failed");
    // Not readable yet, or not arrived: tried again when keys come (`craftworks:reopen`), or after a pause — 4 times.
    if ((kind === "sealed" || kind === "not-yet" || kind === "transient") && retries < 4) {
      retries += 1;
      setTimeout(() => showAgain(`after a ${kind} failure (try ${retries})`), 5000 * retries);
    }
    if (visit.n === 0) setTimeout(sendVisit, 2000);
    boot.log("page failed", { what: `${location.hash}: ${said}${e?.stack ? ` — ${String(e.stack).split("\n").slice(0, 3).join(" | ")}` : ""}` });
    status.textContent = `Could not load: ${said}`;
    status.className = "bad";
    status.hidden = false;
  }
}
