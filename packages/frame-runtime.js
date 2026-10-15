// THE FRAME RUNTIME's BOOTSTRAP (FOUNDATION §3): the script an app's frame starts with — the machinery of the frame as
// the loader is the page's: it asks the host for the RUNTIME (`runtime.js`, installed on the frame's ctx) and the app's
// module, gives `require` (the system's packages loaded by hash, every other capability a proxy to the host), relays
// the host's data and route signals, and mounts the app with `mount(ctx, el)`. Its own calls home (a log line, a report)
// never stop the frame. Run as an inline script of the frame (`apphost.mountCtx`).
export const SOURCE = `
    const waiting = new Map(); let n = 0;
    // WHAT CROSSES (the host's marshal, mirrored): data by value, a function by reference — the host's a call back
    // there, the frame's own (a callback it passes) kept here by number.
    const cbs = new Map(); let cbN = 0;
    const out = v => {
      if (typeof v === "function") { cbs.set(++cbN, v); return { __cwcb: cbN }; }
      // An ELEMENT (a menu's anchor): where it is on the app's screen — the host places what it draws there.
      if (v instanceof Element) { const r = v.getBoundingClientRect(); return { __cwrect: 1, left: r.left, top: r.top, right: r.right, bottom: r.bottom, width: r.width, height: r.height }; }
      if (v == null || typeof v !== "object" || ArrayBuffer.isView(v) || v instanceof ArrayBuffer || v instanceof Date || v instanceof Blob) return v;
      if (Array.isArray(v)) return v.map(out);
      if (v instanceof Map) return new Map([...v].map(([k, x]) => [out(k), out(x)]));
      if (v instanceof Set) return new Set([...v].map(out));
      return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, out(x)]));
    };
    const back = v => {
      if (v == null || typeof v !== "object" || ArrayBuffer.isView(v) || v instanceof ArrayBuffer || v instanceof Date || v instanceof Blob) return v;
      if (Array.isArray(v)) return v.map(back);
      if (v instanceof Map) return new Map([...v].map(([k, x]) => [back(k), back(x)]));
      if (v instanceof Set) return new Set([...v].map(back));
      if (typeof v.__cwfn === "number") { const id = v.__cwfn; return (...args) => call("fn", id, args); }
      if (v.__cwstate && "v" in v) { const x = back(v.v); if (x && typeof x === "object") Object.defineProperty(x, "state", { value: v.__cwstate, enumerable: false }); return x; }
      return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, back(x)]));
    };
    const call = (op, ...args) => new Promise((resolve, reject) => { const id = ++n; waiting.set(id, { resolve: v => resolve(back(v)), reject }); parent.postMessage({ id, op, args: out(args) }, "*"); });
    addEventListener("message", e => {
      if (e.source !== parent) return;
      const m = e.data ?? {};
      if (m.event === "data") return dispatchEvent(new CustomEvent("craftworks:data"));
      if (m.event === "run") return runs[m.i]?.();
      if (m.event === "cb") return cbs.get(m.id)?.(...back(m.args ?? []));
      if (m.event === "route") return Object.assign(ctx, m.where), dispatchEvent(new CustomEvent("craftworks:route"));
      const w = waiting.get(m.id); if (!w) return; waiting.delete(m.id);
      m.ok ? w.resolve(m.value) : w.reject(Object.assign(new Error(m.error), { kind: m.kind }));
    });
    const fromB64 = b => Uint8Array.from(atob(b), c => c.charCodeAt(0));
    const importB64 = b => import("data:text/javascript;base64," + b);
    const loaded = new Map();
    // A CAPABILITY's PROXY: each method a call to the host (its .strict too).
    // Its SYNC HELPERS (the module's own \`frame(cap, ctx)\`, the same bytes the host runs) answer here.
    const proxy = (name, helpers = {}) => new Proxy({}, { get: (_, m) => {
      if (m === "then" || typeof m === "symbol") return undefined;
      if (Object.hasOwn(helpers, m)) return helpers[m];
      const f = (...args) => call("cap", name, m, args, false);
      f.strict = (...args) => call("cap", name, m, args, true);
      return f;
    } });
    const require = name => {
      if (!loaded.has(name)) loaded.set(name, (async () => {
        const p = await call("pkg", name);
        if (!p.local) {
          const helpers = { ...(p.consts || {}) };
          if (!p.b64) return proxy(name, helpers);
          const cap = proxy(name, helpers);
          Object.assign(helpers, await (await importB64(p.b64)).frame(cap, ctx));
          return cap;
        }
        if (p.kind === "bytes") return fromB64(p.b64);
        const mod = await importB64(p.b64);
        return p.kind === "service" ? mod.start(ctx) : mod;
      })().catch(e => (loaded.delete(name), Promise.reject(e))));
      return loaded.get(name);
    };
    const ctx = { params: new URLSearchParams(), route: "", sub: "", space: null, actions: {}, apps: [], require,
      log: (step, detail = {}) => call("log", step, JSON.parse(JSON.stringify(detail ?? {}))).catch(() => {}),
      failed: (step, e, what = "") => call("failed", step, String(e && e.message ? e.message : e), e && e.kind, what).catch(() => {}),
      say: e => String(e && e.message ? e.message : e),
      // A SYSTEM PAGE (an app's settings in a space): drawn by the host in the app's place.
      system: (page, opts) => call("system", page, opts) };
    globalThis.app = ctx;
    // ITS NAVIGATION is the page's: a link to a place (#/…) or a hash set by the app goes to the host, which routes.
    addEventListener("click", e => {
      const a = e.target && e.target.closest ? e.target.closest("a[href]") : null;
      const href = a ? a.getAttribute("href") : "";
      if (href && href.startsWith("#/")) { e.preventDefault(); call("go", href).catch(() => {}); }
    }, true);
    addEventListener("hashchange", () => { if (location.hash.startsWith("#/")) call("go", location.hash).catch(() => {}); });
    // ITS MENU: what its where.tabs / where.menu set, handed to the host (an entry's run kept here by number).
    let runs = [];
    addEventListener("craftworks:actions", () => {
      runs = [];
      const entries = (ctx.actions[ctx.route] ?? []).map(e => { const x = { label: e.label, href: e.href, on: e.on, end: e.end, role: e.role }; if (typeof e.run === "function") (x.run = runs.length), runs.push(e.run); return x; });
      call("actions", entries).catch(() => {});
    });
    const appEl = document.getElementById("cw-app");
    // ITS SIZE: a page as tall as what it shows (the host's area scrolls it) — or, an app with panes of its own
    // (\`cw-fill\`, as Board's), the whole area, each pane scrolling itself.
    let filled = false;
    const tell = () => !filled && parent.postMessage({ event: "height", h: Math.ceil(appEl.getBoundingClientRect().height) + 8 }, "*");
    new ResizeObserver(tell).observe(appEl);
    const fill = () => {
      if (filled || !appEl.classList.contains("cw-fill")) return;
      filled = true;
      for (const x of [document.documentElement, document.body, appEl]) x.style.setProperty("height", "100%", "important");
      parent.postMessage({ event: "fill" }, "*");
    };
    new MutationObserver(fill).observe(appEl, { attributes: true, attributeFilter: ["class"] });
    (async () => {
      const boot = await call("boot");
      Object.assign(ctx, boot.where);
      const rt = await importB64(boot.runtime);
      const QUIET = new Set(["absent", "not-yet", "login-needed", "cancelled", "refused-expected", "extension", "transient"]);
      rt.install(ctx, {
        handled: (source, e) => { const kind = (e && e.kind) || "bug"; if (!QUIET.has(kind)) call("failed", source, String(e && e.message ? e.message : e), kind, "").catch(() => {}); return kind; },
        stateOf: kind => (kind === "sealed" ? "unreadable" : ["absent", "not-yet", "transient", "cancelled", "login-needed", "refused-expected"].includes(kind) ? "unknown" : "failed"),
        theme: () => require("theme"),
      });
      await require("theme");
      const m = await importB64(boot.app);
      await m.mount(ctx, appEl);
    })().catch(e => call("say", String(e && e.message ? e.message : e), e && e.kind).then(t => (appEl.textContent = "This app did not start: " + t)));
  `;
