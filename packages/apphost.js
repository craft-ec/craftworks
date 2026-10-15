// APP HOST, a service (FOUNDATION.md §3, "Apps are installed"): runs an app in ITS OWN FRAME — a sandboxed `srcdoc`
// frame (an opaque origin of its own: it cannot reach this page, its DOM, its keys or its storage; scripts only — no
// popups, forms or modals), with a content security policy set before any of the app's code (no connections; the
// page's own frame policy keeps it from navigating out) — and gives it the SYSTEM API by messages, this page enforcing
// every call: writes and copying only on the person's own click or key, an app's own items and rows only, a budget of
// calls a second. Every app moves here in turn; an app not ours runs only this way.
//
// IN THE FRAME, the app's module exports `mount(api, el)`; `api`:
//   where()                       → { who, sub, space, saved, others, base } (where the app is open)
//   read({ saved })               → the place's items of the app's kinds (plain data)
//   get(ref) / submit(item) / edit(ref, body, meta) / remove(ref) — items OF THE APP'S KINDS only (manifest `kinds`)
//   pageOf(ref) / linkOf(ref)     → its page here / its link to share
//   tabs(extra, { create, yoursOn }) — its own entries and its create (Yours, Saved, Discover: the system's); go(hash); copy(text); say(error) → the words for a failure
//   rows() / put(key, value) / remove… its OWN rows (`appdata`, keys `<app>/…`)
//   lib(name)                     → a pure library's DATA (an allowlist: `highlight`'s languages …)
//   onRoute(fn), onChange(fn)
// and `api.ui`: h(tag, props, …kids), loading(text), card(item, { href }), action(button, fn, { busy, done }).
// The ITEM PAGE (votes, comments, actions) is the SYSTEM's — composed per kind, never per app: for `…/p/<ref>` this page
// draws it under the app's frame.
export async function start(ctx) {
  const [storage, items, kinds] = await Promise.all(["storage", "items", "kinds"].map(n => ctx.require(n)));
  const APPDATA = "appdata";
  const KEY = /^[a-z0-9][a-z0-9-]{0,31}$/;
  const LIBS = new Set(["highlight"]);
  const b64 = bytes => {
    let s = "";
    for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return btoa(s);
  };
  // A plain copy (no functions, no elements): what crosses into a frame.
  const plain = v => JSON.parse(JSON.stringify(v ?? null));

  const RUNTIME = `
    const waiting = new Map(); let n = 0; const routeFns = [], changeFns = [];
    addEventListener("message", e => {
      if (e.source !== parent) return;
      const m = e.data ?? {};
      if (m.event === "route") return routeFns.forEach(f => f(m.where));
      if (m.event === "changed") return changeFns.forEach(f => f());
      const w = waiting.get(m.id); if (!w) return; waiting.delete(m.id);
      m.ok ? w.resolve(m.value) : w.reject(Object.assign(new Error(m.error), { kind: m.kind }));
    });
    const call = (op, ...args) => new Promise((resolve, reject) => { const id = ++n; waiting.set(id, { resolve, reject }); parent.postMessage({ id, op, args }, "*"); });
    const h = (tag, props = {}, ...kids) => { const e = Object.assign(document.createElement(tag), props); e.append(...kids.filter(k => k != null && k !== false)); return e; };
    const ui = {
      h,
      loading: text => h("p", { className: "cw-app-loading", textContent: text }),
      card: (it, { href } = {}) => h("a", { className: "cw-app-card cw-box", href: href ?? "#" },
        h("b", { textContent: it.title || String(it.body ?? "").split("\\n")[0].slice(0, 80) || "untitled" }),
        h("pre", { className: "cw-excerpt", textContent: String(it.body ?? "").slice(0, 240) }),
        h("small", { textContent: new Date(it.at ?? Date.now()).toLocaleString() })),
      action: (btn, fn, { busy = "Working…", done = "Done ✓" } = {}) => async () => {
        if (btn.disabled) return; const was = btn.textContent; btn.disabled = true; btn.textContent = busy;
        try { const r = await fn(); btn.textContent = done; return r; } catch (e) { btn.textContent = was; btn.disabled = false; throw e; }
      },
    };
    // A link in the frame moves the PAGE (the frame has no address of its own).
    addEventListener("click", e => { const a = e.target.closest?.("a[href^='#']"); if (!a) return; e.preventDefault(); call("go", a.getAttribute("href")); }, true);
    const api = {
      where: () => call("where"), read: o => call("read", o ?? {}), get: r => call("get", r), submit: it => call("submit", it),
      edit: (r, b, m) => call("edit", r, b, m), remove: r => call("removeItem", r), pageOf: r => call("pageOf", r), linkOf: r => call("linkOf", r),
      tabs: (t, o) => call("tabs", t, o ?? {}), go: h => call("go", h), copy: t => call("copy", String(t)), say: e => call("say", String(e?.message ?? e)),
      rows: () => call("rows"), put: (k, v) => call("put", k, v), removeRow: k => call("remove", k), lib: n => call("lib", n),
      onRoute: f => routeFns.push(f), onChange: f => changeFns.push(f), ui,
    };
    // ITS HEIGHT, told (the page fits the frame to it).
    // (The APP's own element is measured — never the body or the document, which the theme sizes to the frame: a
    // feedback loop.)
    const appEl = document.getElementById("cw-app");
    const tell = () => parent.postMessage({ event: "height", h: Math.ceil(appEl.getBoundingClientRect().height) + 8 }, "*");
    new ResizeObserver(tell).observe(appEl);
    // (A failure in its kind's words: the host says them.)
    const words = e => call("say", String(e && e.message ? e.message : e));
    import(document.currentScript.dataset.src).then(m => m.mount(api, appEl)).catch(e => words(e).then(t => (appEl.textContent = "This app did not start: " + t)));
  `;

  // A WRITE ONLY ON THE PERSON's GESTURE — one rule for every framed app, either mode: this page's user activation (a
  // click or a key inside the frame activates its ancestors too, so the app cannot fake it). A GESTURE'S CREDIT: a call
  // that arrived while the person had just clicked or typed leaves ONE write for the next 2 minutes — a click that saves
  // after a slow read is still the person's (audit 10-13). A second write needs another gesture.
  function gestures() {
    let credit = 0;
    return {
      clicked: () => navigator.userActivation?.isActive && (credit = Date.now()),
      gesture: what => {
        if (navigator.userActivation?.isActive) return void (credit = 0);
        if (credit && Date.now() - credit < 120000) return void (credit = 0);
        throw Object.assign(new Error(`${what}: only when you click or type in the app`), { kind: "refused" });
      },
    };
  }
  function mount(el, { app, bytes, kinds: own = [] }) {
    if (!KEY.test(app)) throw new Error(`an app's name is a-z, 0-9 and "-": ${app}`);
    const prefix = `${app}/`;
    const mine = new Set(own);
    const frame = document.createElement("iframe");
    // Scripts and forms (a sandbox without forms never fires a form's submit — the app's own handler; posting a form
    // anywhere is refused by its CSP, `form-action 'none'`); no popups (a popup is a request CSP does not stop — data
    // out), no modal loops freezing the page. (Navigating itself away: refused by the page's own frame policy, and the
    // frame removed.)
    frame.sandbox = "allow-scripts allow-forms";
    frame.title = app;
    frame.style.cssText = "border:0;width:100%;min-height:120px;display:block;background:transparent";
    const csp = "default-src 'none'; script-src 'unsafe-inline' data:; style-src 'unsafe-inline'; img-src data:; font-src data:; connect-src 'none'; frame-src 'none'; form-action 'none'";
    const theme = document.querySelector("style[data-theme]")?.textContent ?? "";
    frame.srcdoc = `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${csp}">
      <style>${theme.replace(/</g, "\\3c ")}</style>
      <style>body{margin:0;font-family:var(--cw-font);color:var(--cw-fg);background:transparent}.cw-app-card{display:grid;gap:4px;padding:var(--cw-space-3);text-decoration:none;color:inherit}
      .cw-excerpt{margin:0;white-space:pre-wrap;font:12px/1.4 var(--cw-font-mono);color:var(--cw-muted);max-height:6em;overflow:hidden}.cw-app-loading{color:var(--cw-muted)}</style></head>
      <style>html,body{height:auto!important;min-height:0!important}#cw-app{display:flow-root}</style>
      <body><div id="cw-app"></div><script data-src="data:text/javascript;base64,${b64(bytes)}">${RUNTIME}</script></body></html>`;
    // The SYSTEM's item page, under the frame (for `…/p/<ref>`).
    const system = document.createElement("div");
    el.replaceChildren(frame, system);
    // An app that NAVIGATES its frame away (a second load) is not this app any more: removed.
    let loads = 0;
    frame.addEventListener("load", () => {
      if (++loads > 1) frame.replaceWith(Object.assign(document.createElement("p"), { textContent: `${app} left its page and was closed.` }));
    });

    const where = async () => {
      const at = await (await ctx.require("where")).of({ kinds: own, app, yours: "Yours" });
      return { at, plain: { who: at.who, sub: at.sub, space: at.space ? { id: at.space.id, name: at.space.name ?? null } : null, saved: at.saved, others: at.others, base: at.base } };
    };
    // A WRITE (make, edit, remove, copy) only on the person's own gesture: this page's user activation — a click or a key
    // inside the frame activates its ancestors too, so the app cannot fake it.
    // A GESTURE'S CREDIT: a call that arrived while the person had just clicked or typed (`clicked`, below) leaves ONE
    // write for the next 2 minutes — a click that saves after a slow read (a slow node: 30-60 s) is still the person's
    // (audit 10-13: the 5 s activation had run out). Used by the write; a second write needs another gesture.
    const { clicked, gesture } = gestures();
    // What an app may set on an item: its own fields — never a level, a gate or write rules (the system's).
    const META_DENY = new Set(["level", "gate", "write", "editions", "members"]);
    const cleanMeta = m => Object.fromEntries(Object.entries(m && typeof m === "object" ? m : {}).filter(([k]) => !META_DENY.has(k)).slice(0, 32));
    const AUDIENCES = new Set(["private", "public", "members"]);
    const ofMine = async ref => {
      const it = await items.get.strict(ref);
      if (!it || !mine.has(it.kind)) throw Object.assign(new Error("not one of this app's items"), { kind: "refused" });
      return it;
    };
    let table = null;
    const tableOf = async () => (table ??= storage.table(APPDATA).catch(e => ((table = null), Promise.reject(e))));
    const own_ = key => {
      if (typeof key !== "string" || !key || key.length > 200) throw new Error("a key is text, 1 to 200 characters");
      return prefix + key;
    };
    const ops = {
      where: async () => (await where()).plain,
      read: async ({ saved = false } = {}) => plain((await (await where()).at.read({ kinds: own })).filter(it => mine.has(it.kind))),
      get: async ref => plain(await ofMine(ref)),
      submit: async it => {
        gesture("making an item");
        if (!mine.has(it?.kind)) throw Object.assign(new Error(`this app makes ${[...mine].join(", ") || "nothing"}`), { kind: "refused" });
        const { kind, title, body, audience, meta } = it;
        if (audience != null && !AUDIENCES.has(audience)) throw Object.assign(new Error(`who sees it: ${[...AUDIENCES].join(", ")}`), { kind: "refused" });
        return items.submit({ kind, title, body, audience, meta: cleanMeta(meta), board: ctx.space && ctx.space !== "discover" ? ctx.space : null });
      },
      edit: async (ref, body, meta) => {
        gesture("changing an item");
        const it = await ofMine(ref);
        await items.editItem(ref, body, { meta: { ...cleanMeta(meta), ...Object.fromEntries(Object.entries(it.meta ?? {}).filter(([k]) => META_DENY.has(k))) } });
        return true;
      },
      // Only the person's OWN item (never another's, even where they may moderate).
      removeItem: async ref => {
        gesture("removing an item");
        const it = await ofMine(ref);
        const me = await (await ctx.require("space")).me();
        if (it.by !== me) throw Object.assign(new Error("only your own items"), { kind: "refused" });
        await items.remove(ref);
        return true;
      },
      pageOf: async ref => items.pageOf(ref, [...mine][0]),
      linkOf: async ref => items.linkOf(ref, app),
      // THE APP's ENTRIES: through `where.tabs` — the ONE builder (yours, Saved, Discover and the roles the side panel
      // places them by are the system's); the app gives only its extras and its create.
      tabs: async (extra, opts = {}) => {
        const link = e => ({ label: String(e?.label ?? "").slice(0, 40), href: /^#\//.test(e?.href) ? e.href : undefined, on: !!e?.on });
        const { at } = await where();
        at.tabs((Array.isArray(extra) ? extra : []).slice(0, 10).map(link), { yoursOn: opts.yoursOn !== false, create: opts.create ? link(opts.create) : null });
        return true;
      },
      go: async hash => {
        if (!/^#\/[^\s]*$/.test(String(hash))) throw new Error("only a place in this site (#/…)");
        location.hash = hash;
        return true;
      },
      copy: async text => (gesture("copying"), await navigator.clipboard.writeText(String(text).slice(0, 100000)), true),
      say: async message => ctx.say(new Error(message)),
      lib: async name => {
        if (!LIBS.has(name)) throw Object.assign(new Error(`no library "${name}" for apps`), { kind: "refused" });
        return plain(await ctx.require(name));
      },
      rows: async () => {
        const t = await tableOf();
        await t.decided?.();
        return t.rows().filter(r => r.key.startsWith(prefix) && r.value).map(r => ({ key: r.key.slice(prefix.length), value: r.value }));
      },
      // (Its rows: at most 500, each at most 16 KiB.)
      put: async (key, value) => {
        const v = String(value);
        if (v.length > 16384) throw Object.assign(new Error("a row of at most 16 KiB"), { kind: "refused" });
        const t = await tableOf();
        if (!t.rows().some(r => r.key === own_(key)) && t.rows().filter(r => r.key.startsWith(prefix) && r.value).length >= 500) throw Object.assign(new Error("at most 500 rows an app"), { kind: "refused" });
        await t.put(own_(key), v);
        return true;
      },
      remove: async key => (await (await tableOf()).remove(own_(key)), true),
    };
    // CALLS a second, at most (a flood starves the page): past it, refused.
    let budget = 30;
    ctx.every(frame, 1000, () => (budget = 30));
    const onMessage = async e => {
      if (!frame.isConnected) return (removeEventListener("message", onMessage), removeEventListener("craftworks:route", onRoute));
      if (e.source !== frame.contentWindow) return;
      const m = e.data ?? {};
      clicked();
      if (typeof m.id === "number" && budget-- <= 0) return frame.contentWindow?.postMessage({ id: m.id, ok: false, error: "too many calls", kind: "refused" }, "*");
      if (m.event === "height") return void (frame.style.height = `${Math.max(120, Math.min(Number(m.h) || 0, 20000))}px`);
      const { id, op, args = [] } = m;
      if (typeof id !== "number") return;
      const reply = r => frame.contentWindow?.postMessage({ id, ...r }, "*");
      if (!Object.hasOwn(ops, op)) return reply({ ok: false, error: `no such call: ${op}`, kind: "refused" });
      try {
        reply({ ok: true, value: await ops[op](...args) });
      } catch (err) {
        reply({ ok: false, error: String(err?.message ?? err), kind: err?.kind ?? "refused" });
      }
    };
    // THE ROUTE: told to the app; `…/p/<ref>` — the system's item page drawn under it.
    const showSystem = async () => {
      const { plain: w } = await where();
      const m = /(?:^|\/)p\/(.+)$/.exec(w.sub);
      if (!m) return system.replaceChildren();
      const ref = decodeURIComponent(m[1]);
      system.replaceChildren(await (await ctx.require("item-page")).show(ref, { app, back: w.base }).catch(err => Object.assign(document.createElement("p"), { textContent: ctx.say(err) })));
    };
    const onRoute = async () => {
      if (!frame.isConnected || ctx.route !== `/${app}`) return;
      frame.contentWindow?.postMessage({ event: "route", where: (await where()).plain }, "*");
      showSystem();
    };
    addEventListener("message", onMessage);
    addEventListener("craftworks:route", onRoute);
    showSystem();
    items.onChange(() => frame.isConnected && frame.contentWindow?.postMessage({ event: "changed" }, "*"));
    return frame;
  }

  // THE FRAME RUNTIME (FOUNDATION §3): an app whose module mounts with a `ctx` — `mount(ctx, el)`, as every page here —
  // run in ITS OWN FRAME with the page's own runtime: the same `runtime.js` installed on the frame's ctx (view, cache,
  // total, background, optional, every); `require` gives the SYSTEM's pure and UI packages loaded into the frame by
  // their hash (the same bytes as the page's: `FRAME_LOCAL`), and every other capability as a PROXY whose calls cross
  // here, each refused unless the app's grants name it (`uses`). The page's data signal is relayed in.
  // THE TRUST CORE stays here (the node, the keys, the tables, the identity, the spaces and their items): a frame gets
  // it only as proxies, each call checked. EVERYTHING ELSE — UI, pure libraries, an app's own logic — is loaded into the
  // frame by its hash (as a phone's frameworks into an app's process), its own requires resolved the same way there.
  const HOST = new Set(["node", "storage", "identity", "auth", "keys", "access", "index", "files", "membership", "conversation", "space", "items", "roles", "directory", "lifecycle", "activity", "usage", "upkeep", "edge", "drafts", "installed", "apphost", "places", "login", "blocks", "account", "ordering", "statement", "tokens", "beacon", "keeper", "witness", "signals"]);
  const FRAME_LOCAL = { has: name => !HOST.has(name) };
  const { SOURCE: RUNTIME_CTX } = await ctx.require("frame-runtime");
  // (`bytes`: a module given as it is — a probe's — in place of a package by name.)
  function mountCtx(el, { app, module = null, bytes = null, uses = [] }) {
    if (!KEY.test(app)) throw new Error(`an app's name is a-z, 0-9 and "-": ${app}`);
    const granted = new Set(uses);
    const { clicked, gesture } = gestures();
    // A call that WRITES (by its name): only on the person's gesture.
    const WRITES = /^(submit|edit|remove|put|set|create|send|join|leave|delete|add|move|rename|revoke|publish|vote|act|make|drop|write|post|accept|ban|invite|grant)/i;
    const frame = document.createElement("iframe");
    frame.sandbox = "allow-scripts allow-forms";
    frame.title = app;
    frame.style.cssText = "border:0;width:100%;min-height:120px;display:block;background:transparent";
    const csp = "default-src 'none'; script-src 'unsafe-inline' data:; style-src 'unsafe-inline'; img-src data: blob:; media-src data: blob:; font-src data:; connect-src 'none'; frame-src 'none'; form-action 'none'";
    frame.srcdoc = `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${csp}">
      <style>html,body{height:auto!important;min-height:0!important;margin:0;background:transparent}#cw-app{display:flow-root}</style></head>
      <body><div id="cw-app"></div><script>${RUNTIME_CTX}</script></body></html>`;
    el.replaceChildren(frame);
    const whereNow = () => ({ route: ctx.route, sub: ctx.sub ?? "", space: ctx.space ?? null });
    const ops = {
      // (The site's apps, as data: what `where` reads to know an app's places — its public side, its views.)
      boot: async () => ({ where: { ...whereNow(), apps: plain(ctx.apps ?? []) }, runtime: b64(await ctx.runtimeBytes()), app: b64(bytes ?? (await ctx.packageOf(module)).bytes) }),
      // A SYSTEM package (the same bytes as the page's) loaded in the frame; a capability the app is granted: a proxy.
      pkg: async name => {
        if (FRAME_LOCAL.has(name)) {
          const p = await ctx.packageOf(name);
          return { local: true, kind: p.kind, b64: b64(p.bytes) };
        }
        if (!granted.has(name)) throw Object.assign(new Error(`${app} is not granted "${name}"`), { kind: "refused" });
        return { local: false };
      },
      cap: async (name, method, args, strict) => {
        if (!granted.has(name)) throw Object.assign(new Error(`${app} is not granted "${name}"`), { kind: "refused" });
        if (WRITES.test(method)) gesture(`${name}.${method}`);
        const c = await ctx.require(name);
        const f = c?.[method];
        if (typeof f !== "function") throw Object.assign(new Error(`${name} has no call "${method}"`), { kind: "refused" });
        return plain(await (strict && typeof f.strict === "function" ? f.strict : f).apply(c, Array.isArray(args) ? args : []));
      },
      log: async (step, detail) => (ctx.log(`${app}: ${String(step).slice(0, 40)}`, detail), true),
      say: async (message, kind) => ctx.say(Object.assign(new Error(message), { kind })),
      // ITS MENU (the frame's `where`): its entries, to the side panel through the one door (`where.menu`); an entry
      // that runs something is run in the frame (`run`: its number) when chosen here.
      actions: async entries => {
        const list = (Array.isArray(entries) ? entries : []).slice(0, 16).map(e => ({
          label: String(e?.label ?? "").slice(0, 40),
          href: /^#\//.test(e?.href) ? e.href : undefined,
          on: !!e?.on,
          end: !!e?.end,
          role: ["discover", "feed", "you", "create"].includes(e?.role) ? e.role : undefined,
          ...(Number.isInteger(e?.run) ? { run: () => frame.contentWindow?.postMessage({ event: "run", i: e.run }, "*") } : {}),
        }));
        (await ctx.require("where")).menu(list);
        return true;
      },
      failed: async (step, message, kind, what) => (ctx.failed(`${app}: ${String(step).slice(0, 40)}`, Object.assign(new Error(message), { kind }), what), true),
    };
    let budget = 60;
    ctx.every(frame, 1000, () => (budget = 60));
    const onMessage = async e => {
      if (!frame.isConnected) return (removeEventListener("message", onMessage), removeEventListener("craftworks:data", onData), removeEventListener("craftworks:route", onRoute));
      if (e.source !== frame.contentWindow) return;
      const m = e.data ?? {};
      clicked();
      if (m.event === "height") return void (frame.style.height = `${Math.max(120, Math.min(Number(m.h) || 0, 20000))}px`);
      const { id, op, args = [] } = m;
      if (typeof id !== "number") return;
      const reply = r => frame.contentWindow?.postMessage({ id, ...r }, "*");
      if (budget-- <= 0) return reply({ ok: false, error: "too many calls", kind: "refused" });
      if (!Object.hasOwn(ops, op)) return reply({ ok: false, error: `no such call: ${op}`, kind: "refused" });
      try {
        reply({ ok: true, value: await ops[op](...args) });
      } catch (err) {
        reply({ ok: false, error: String(err?.message ?? err), kind: err?.kind ?? "refused" });
      }
    };
    const onData = () => frame.contentWindow?.postMessage({ event: "data" }, "*");
    const onRoute = () => frame.contentWindow?.postMessage({ event: "route", where: whereNow() }, "*");
    addEventListener("message", onMessage);
    addEventListener("craftworks:data", onData);
    addEventListener("craftworks:route", onRoute);
    return frame;
  }

  return { mount, mountCtx };
}
