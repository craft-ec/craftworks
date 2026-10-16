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
  const HOST = new Set(["node", "storage", "identity", "auth", "keys", "access", "index", "files", "membership", "conversation", "space", "items", "roles", "directory", "lifecycle", "activity", "usage", "upkeep", "edge", "drafts", "installed", "apphost", "places", "login", "blocks", "account", "ordering", "statement", "tokens", "beacon", "keeper", "witness", "vote-tally", "person", "keep", "video-studio", "moderation", "wiki-pages", "blog-press", "editions", "app-settings"]);
  const FRAME_LOCAL = { has: name => !HOST.has(name) };
  const { SOURCE: RUNTIME_CTX } = await ctx.require("frame-runtime");
  // (`bytes`: a module given as it is — a probe's — in place of a package by name.)
  function mountCtx(el, { app, module = null, bytes = null, uses = [] }) {
    if (!KEY.test(app)) throw new Error(`an app's name is a-z, 0-9 and "-": ${app}`);
    const granted = new Set(uses);
    const { clicked, gesture } = gestures();
    // WHAT CROSSES THE BRIDGE (as a phone's IPC parcel): DATA BY VALUE — structured clone, so a Map, a Set, bytes stay
    // what they are; a read's state (a total read's \`.state\`) kept with it — and a FUNCTION BY REFERENCE: a handle
    // whose calls come back here (a place's steps, an unsubscribe), each a write only on a gesture, as any call. A live
    // object (an element, a binding's handle) is not sent. The frame's own functions (a callback it passes) come here
    // as handles too, called back in the frame.
    const handles = new Map(); // id → { f, name }
    let handleN = 0;
    const OPEN = v => ArrayBuffer.isView(v) || v instanceof ArrayBuffer || v instanceof Date || v instanceof Blob;
    const marshal = (v, name, depth = 0) => {
      if (typeof v === "function") {
        handles.set(++handleN, { f: v, name });
        if (handles.size > 4000) handles.delete(handles.keys().next().value);
        return { __cwfn: handleN };
      }
      if (v == null || typeof v !== "object") return v;
      // A PROMISE (a place's `settled`, a person's `ready`): a handle the frame awaits.
      if (v instanceof Promise) return marshal(() => v, name);
      if (depth > 16) return null;
      if (OPEN(v)) return v;
      let out;
      if (Array.isArray(v)) out = v.map(x => marshal(x, name, depth + 1));
      else if (v instanceof Map) out = new Map([...v].map(([k, x]) => [marshal(k, name, depth + 1), marshal(x, name, depth + 1)]));
      else if (v instanceof Set) out = new Set([...v].map(x => marshal(x, name, depth + 1)));
      else if ([Object.prototype, null].includes(Object.getPrototypeOf(v))) out = Object.fromEntries(Object.entries(v).map(([k, x]) => [k, marshal(x, k, depth + 1)]));
      else return null;
      return Object.getOwnPropertyDescriptor(v, "state")?.enumerable === false ? { __cwstate: plain(v.state), v: out } : out;
    };
    // A SYNC ANSWER across the bridge (the host's function gave a value, not a promise): in the frame it is a promise
    // all the same — said once per name in the trace, where an app may still use it as a value (fx 10-16: a place's
    // `mayEdit()` read as true).
    const syncNames = new Set();
    const syncSaid = (name, v) => {
      if (!(v && typeof v.then === "function") && !syncNames.has(name)) syncNames.add(name), ctx.log("bridge: a sync call", { what: `${app}: ${name}` });
      return v;
    };
    const callback = id => (...args) => void frame.contentWindow?.postMessage({ event: "cb", id, args: marshal(args, "callback") }, "*");
    const unmarshal = v => {
      if (v == null || typeof v !== "object" || OPEN(v)) return v;
      if (Array.isArray(v)) return v.map(unmarshal);
      if (v instanceof Map) return new Map([...v].map(([k, x]) => [unmarshal(k), unmarshal(x)]));
      if (v instanceof Set) return new Set([...v].map(unmarshal));
      if (typeof v.__cwcb === "number") return callback(v.__cwcb);
      // A place on the app's screen (an element it passed): an anchor at the same place on this page.
      if (v.__cwrect === 1) {
        const f = frame.getBoundingClientRect();
        const n = k => Number(v[k]) || 0;
        const r = { left: f.left + n("left"), top: f.top + n("top"), right: f.left + n("right"), bottom: f.top + n("bottom"), width: n("width"), height: n("height") };
        // (On the page while the app's frame is: what a clock on it — `ctx.every` — runs by.)
        return { getBoundingClientRect: () => ({ ...r, x: r.left, y: r.top }), get isConnected() { return frame.isConnected; } };
      }
      return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, unmarshal(x)]));
    };
    // A READ (by its exact name) needs no gesture; EVERY OTHER call — a capability's or a handle's (a step a call gave
    // back, named by its key) — only on the person's gesture. Fails closed: a read not named here is refused until named
    // (a visible refusal), never a write let through because its name was not on a list (fx 10-15: \`delegate\` was not).
    const READS = new Set(["get", "list", "read", "of", "ofPublic", "find", "mine", "me", "account", "shown", "name", "nameEl", "handle", "card", "levelOf", "publicOf", "devices", "isListed", "view", "attached", "inPlaces", "mayWriteOn", "mayWrite", "spaceOf", "pollView", "proposalView", "inSpace", "people", "personal", "tagsOf", "pageOf", "appOf", "graphOf", "participation", "delegationChains", "onChange", "onChecked", "onDevices", "followable", "credToCite", "policy", "policyIn", "policiesAt", "members", "role", "allowsIn", "can", "config", "enabled", "capabilities", "apps", "tableOf", "owner", "answer", "rows", "held", "loaded", "isNsfw", "spaceIdOf", "linkOf", "refOf", "loginNeeded", "level", "settled", "ready",
      // (10-16, from what the framed apps' code calls — and the methods of what those calls return.)
      "allows", "asked", "askedCodes", "boardOf", "boards", "following", "history", "keepers", "mayOpen", "nsfwShown", "outsideOf", "personalPolicy",
      "pins", "labels", "people", "prefs", "progress", "publicIn", "publicSpace", "publicSpaces", "range", "reaches", "visible", "waiting", "page", "pages",
      "has", "is", "at", "about", "refs", "lists", "roles", "domains", "acts", "mayEdit", "needsReview", "drafts", "thread", "probe", "editorAt",
      "opened", "reading", "played", "showing", "settings", "policyIn", "config"]);
    const WRITES = { test: name => !READS.has(name) };
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
      boot: async () => ({ where: { ...whereNow(), apps: plain(ctx.apps ?? []), site: `${location.origin}${location.pathname}` }, runtime: b64(await ctx.runtimeBytes()), app: b64(bytes ?? (await ctx.packageOf(module)).bytes) }),
      // A SYSTEM package (the same bytes as the page's) loaded in the frame; a capability the app is granted: a proxy.
      pkg: async name => {
        if (FRAME_LOCAL.has(name)) {
          const p = await ctx.packageOf(name);
          return { local: true, kind: p.kind, b64: b64(p.bytes) };
        }
        if (!granted.has(name)) throw Object.assign(new Error(`${app} is not granted "${name}"`), { kind: "refused" });
        // A capability with SYNC HELPERS (its module's `frame` export): its bytes, so the frame runs the same helpers.
        const p = await ctx.packageOf(name);
        // Its CONSTANTS (what it gives that is not a call — a type's name, a limit): with the proxy, as values.
        const c = await ctx.require(name);
        const consts = c && typeof c === "object" ? marshal(Object.fromEntries(Object.entries(c).filter(([, v]) => typeof v !== "function" && !(v instanceof Promise))), name) : {};
        return { local: false, consts, ...(/\bexport (async )?function frame\(/.test(new TextDecoder().decode(p.bytes)) ? { b64: b64(p.bytes) } : {}) };
      },
      cap: async (name, method, args, strict) => {
        if (!granted.has(name)) throw Object.assign(new Error(`${app} is not granted "${name}"`), { kind: "refused" });
        if (WRITES.test(method)) gesture(`${name}.${method}`);
        const c = await ctx.require(name);
        const f = c && Object.hasOwn(c, method) ? c[method] : undefined; // (its own calls only: never its prototype's)
        if (typeof f !== "function") throw Object.assign(new Error(`${name} has no call "${method}"`), { kind: "refused" });
        return marshal(await syncSaid(`${name}.${method}`, (strict && typeof f.strict === "function" ? f.strict : f).apply(c, unmarshal(Array.isArray(args) ? args : []))), method);
      },
      // A HANDLE called (a function a call gave back).
      fn: async (id, args) => {
        const h = handles.get(id);
        if (!h) throw Object.assign(new Error("this step is gone: open it again"), { kind: "refused" });
        if (WRITES.test(h.name)) gesture(h.name);
        return marshal(await syncSaid(h.name, h.f(...unmarshal(Array.isArray(args) ? args : []))), h.name);
      },
      log: async (step, detail) => (ctx.log(`${app}: ${String(step).slice(0, 40)}`, detail), true),
      say: async (message, kind) => ctx.say(Object.assign(new Error(message), { kind })),
      // A SYSTEM PAGE the host draws in the app's place: its SETTINGS in a space (`where.settings`, the same page an
      // app in the page shows) — the frame hidden while the address stays there.
      system: async (page, { kind, yours, ref, outside = null } = {}) => {
        // AN ITEM's EDITIONS (bought and sold): a sheet over the app, the host's (`editions.section`).
        if (page === "editions") {
          const it = await (await ctx.require("items")).get.strict(String(ref), { outside: unmarshal(outside) });
          if (!it) throw Object.assign(new Error("that item is not here"), { kind: "absent" });
          const sheet = document.createElement("dialog");
          sheet.className = "cw-sheet";
          const close = Object.assign(document.createElement("button"), { type: "button", className: "cw-btn", textContent: "Close", onclick: () => sheet.close() });
          const body = document.createElement("div");
          sheet.append(body, close);
          document.body.append(sheet);
          const draw = async () => body.replaceChildren((await (await ctx.require("editions")).section(it, { changed: draw })) ?? Object.assign(document.createElement("p"), { textContent: "No editions." }));
          await draw();
          sheet.showModal();
          await new Promise(done => sheet.addEventListener("close", done, { once: true }));
          sheet.remove();
          return true;
        }
        if (page !== "settings") throw Object.assign(new Error(`no such system page: ${page}`), { kind: "refused" });
        const at = await (await ctx.require("where")).of({ kind: String(kind ?? ""), app: String(ctx.route).replace(/^\//, ""), yours: String(yours ?? "Yours") });
        const box = document.createElement("div");
        box.className = "cw-system-page";
        frame.after(box);
        frame.hidden = true;
        const back = () => (box.remove(), (frame.hidden = false), removeEventListener("craftworks:route", back));
        addEventListener("craftworks:route", back);
        await at.settings(box);
        return true;
      },
      // ITS NAVIGATION: a place in this site (#/…), routed by the page.
      go: async href => (/^#\/[^\s]*$/.test(href) && (location.hash = href), true),
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
    // A RUNAWAY FRAME is slowed, never a list broken: past 400 calls in a second, a call WAITS for the next second
    // (a long list's cards each ask a few things — fx 10-16: 60 a second refused Discover's).
    const PER_SECOND = 400;
    let budget = PER_SECOND;
    let refill = [];
    ctx.every(frame, 1000, () => ((budget = PER_SECOND), refill.splice(0).forEach(go => go())));
    const turn = () => (budget-- > 0 ? null : new Promise(go => refill.push(go)).then(turn));
    const onMessage = async e => {
      if (!frame.isConnected) return (removeEventListener("message", onMessage), removeEventListener("craftworks:data", onData), removeEventListener("craftworks:route", onRoute));
      if (e.source !== frame.contentWindow) return;
      const m = e.data ?? {};
      clicked();
      if (m.event === "height") return void (!el.classList.contains("cw-fill") && (frame.style.height = `${Math.max(120, Math.min(Number(m.h) || 0, 20000))}px`));
      // An app with panes of its own: the frame fills the area (the theme's `cw-fill`), each pane scrolling itself.
      if (m.event === "fill") return void (el.classList.add("cw-fill"), (frame.style.height = ""), (frame.style.minHeight = "0"));
      const { id, op, args = [] } = m;
      if (typeof id !== "number") return;
      const reply = r => frame.contentWindow?.postMessage({ id, ...r }, "*");
      await turn();
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
