// THE LOADER, a package: the wrapper fetches it from the loader's own SITE (signed, versioned), so it can change without
// touching the wrapper or any app.
//
// It works like a website: the app's MANIFEST names its packages (by sha256, with the packages each one needs), its
// LAYOUT (what fills the header and the footer) and its PAGES (a route -> what fills the body, and any header/footer
// of its own). The loader only makes the three SLOTS; what fills them is the app's, so a header, a body or a footer is
// edited by publishing the app, never the loader. Only the current page's packages are fetched; everything else
// loads the first time something asks for it (`ctx.require(name)`), once. A page that needs no node never loads the
// node's code at all.
const VERSION = "17";

export async function run(boot) {
  const status = document.getElementById("status");
  const root = document.getElementById("app");
  const loaded = new Map(); // name -> Promise of what `require` gives
  let manifest;

  const ctx = { params: boot.params, log: boot.log, on: boot.on, require, route: "/", app: "", apps: [], uses: [], identityPrior: [], actions: {} };
  globalThis.app = ctx;

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
      loaded.set(name, load(name));
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

  async function fetchPackage(p, name) {
    const { raceK, openPieces, dec } = await racing();
    const t = performance.now();
    const spec = {
      k: p.k,
      m: p.m,
      pieces: p.pieces.map(x => ({ url: new URL(`/v1/contract/web/${x.address}/piece`, location.href).href, sha256: x.sha256 })),
    };
    const raced = await raceK(spec, {
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
    if (got !== p.sha256) throw new Error(`${name}: decoded to bytes hashing ${got.slice(0, 12)}…, not ${p.sha256.slice(0, 12)}…`);
    return { bytes, ms: Math.round(performance.now() - t), used: raced.pieces.filter(Boolean).length, asked: raced.asked.length };
  }

  // A package's ENTRY (its pieces), in its own file of the site named by the package's hash: read once, and only when
  // it is needed — the manifest itself names each package by kind and hash alone. (A manifest from before: inline.)
  const entries = new Map();
  function entry(name) {
    const head = manifest.packages[name];
    if (!head) return Promise.reject(new Error(`package "${name}" is not in this app's manifest`));
    if (head.pieces) return Promise.resolve(head);
    if (!entries.has(name))
      entries.set(
        name,
        boot.get(new URL(`p/${head.sha256.slice(0, 16)}.json`, location.href), `${name}'s entry`).then(r => ({ ...head, ...JSON.parse(new TextDecoder().decode(r.bytes)) })),
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
  for (const [name, tag] of [["header", "header"], ["body", "main"], ["footer", "footer"]]) {
    slots[name] = document.createElement(tag);
    slots[name].className = `slot-${name}`;
    root.append(slots[name]);
  }
  const filled = { header: null, footer: null };

  // A component still on its way shows the theme's placeholder in its place, until it has drawn itself (it replaces
  // its element's contents) or its mount is over. Every component of a slot gets one at once: what is coming, shown.
  // It stays until the component is SETTLED: mounted, and everything it said it is waiting on (`ctx.pending(promise)`:
  // storage says it for each table it opens — every writer's feed tried) is in, or 20 s have passed. A component that
  // draws itself at once (replacing its element's contents) keeps the placeholder over it meanwhile.
  const placeholders = new Map(); // element -> placeholder
  const pendingOf = new Map(); // component name -> Set of promises it waits on
  ctx.pending = p => pendingOf.get(working.at(-1))?.add(Promise.resolve(p).catch(() => {}));
  async function settle(name) {
    const waits = pendingOf.get(name) ?? new Set();
    const cap = new Promise(r => setTimeout(r, 20000));
    for (let seen = -1; seen !== waits.size; ) {
      seen = waits.size;
      await Promise.race([Promise.all(waits), cap]);
    }
    pendingOf.delete(name);
  }
  async function fill(slot, names) {
    slots[slot].replaceChildren();
    const theme = manifest.theme ? await require(manifest.theme) : null;
    const els = names.map(name => {
      const el = document.createElement("section");
      el.dataset.component = name;
      if (theme?.loading && slot === "body") {
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
      pendingOf.set(name, new Set());
      const p = placeholders.get(el);
      await Promise.resolve(mod.mount(ctx, el)).finally(() => working.pop());
      // Not awaited: the next component mounts meanwhile; this one's placeholder goes when it is settled.
      settle(name).finally(() => p?.done());
      ctx.log("mounted", { what: `${slot}: ${name}`, ms: Math.round(performance.now() - t) });
    }
  }

  // What a page shows: `["a", "b"]` is its body; `{ body, header?, footer? }` overrides the layout's slots.
  // A route names its page by the LONGEST page route it starts with; the rest is the page's SUB-PAGE (`#/account/nodes`:
  // the page `/account`, `ctx.sub` "nodes").
  async function show(full) {
    const route = Object.keys(manifest.pages).filter(r => full === r || full.startsWith(r === "/" ? "/" : `${r}/`)).sort((a, b) => b.length - a.length)[0] ?? "/";
    const raw = manifest.pages[route];
    const page = Array.isArray(raw) ? { body: raw } : raw;
    const t0 = performance.now();
    ctx.log("page", { what: full });
    // The wrapper's "Loading…" line: gone once a page starts (its components show their own placeholders, and how long
    // it took is in the trace); it comes back for a failure.
    status.hidden = status.className !== "bad";
    ctx.route = route;
    ctx.sub = full.slice(route.length).replace(/^\//, "");
    // Every entry this page will need, asked for at once (the manifest lists them): no chain of one-by-one reads.
    for (const n of manifest.needs?.[route] ?? []) entry(n).catch(() => {});
    for (const slot of ["header", "footer"]) {
      const names = page[slot] ?? manifest.layout?.[slot] ?? [];
      const key = JSON.stringify(names);
      // A header or footer the new page shares is kept, not rebuilt; each shown component hears the new route.
      if (filled[slot] !== key) {
        await fill(slot, names);
        filled[slot] = key;
      }
    }
    await fill("body", page.body ?? []);
    dispatchEvent(new CustomEvent("craftworks:route", { detail: route }));
    ctx.log("page ready", { what: route, ms: Math.round(performance.now() - t0) });
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
    askedBy: name => askedBy.get(name),
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
    ctx.app = manifest.app;
    // The site's APPS, as its manifest lists them ({ name, icon, route }): what a desktop shows.
    ctx.apps = manifest.apps ?? [];
    // The kinds of the person's data the site USES (["notes", "pins"]): asked for together, in one prompt.
    ctx.uses = manifest.uses ?? [];
    // Earlier builds of the identity delegate (`<key>:<code hash>`, newest first): where a member made before this
    // build still is, until its next login moves it here.
    ctx.identityPrior = manifest.identity_prior ?? [];
    document.title = manifest.app;
    // The app's THEME (its manifest names it): applied before anything mounts, so nothing shows unstyled.
    if (manifest.theme) await require(manifest.theme);
    ctx.log("manifest read", {
      what: `${manifest.app}: ${Object.keys(manifest.packages).length} packages, ${Object.keys(manifest.pages).length} pages`,
      bytes: m.bytes.length,
      ms: m.ms,
    });
    traceToggle();
    addEventListener("hashchange", () => show(route()).catch(fail));
    await show(route());
  } catch (e) {
    fail(e);
  }

  function fail(e) {
    ctx.log("FAILED", { what: e.message });
    status.textContent = `Could not load: ${e.message}`;
    status.className = "bad";
    status.hidden = false;
  }
}
