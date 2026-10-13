// APP HOST, a service (FOUNDATION.md §3, "Apps are installed"): runs an app in ITS OWN FRAME — a sandboxed `srcdoc`
// frame (an opaque origin of its own: it cannot reach this page, its DOM, its keys or its storage), with a content
// security policy set before any of the app's code (no network) — and gives it the SYSTEM API by messages, this page
// enforcing every call. Every app moves here in turn; an app not ours runs only this way.
//
// IN THE FRAME, the app's module exports `mount(api, el)`; `api`:
//   where()                       → { who, sub, space, saved, others, base } (where the app is open)
//   read({ saved })               → the place's items of the app's kinds (plain data)
//   get(ref) / submit(item) / edit(ref, body, meta) / remove(ref) — items OF THE APP'S KINDS only (manifest `kinds`)
//   pageOf(ref) / linkOf(ref)     → its page here / its link to share
//   tabs([{ label, href, on, end }]) — the app's menu; go(hash); copy(text); say(error) → the words for a failure
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
      loading: text => h("p", { className: "cw-loading", textContent: text }),
      card: (it, { href } = {}) => h("a", { className: "cw-card cw-box", href: href ?? "#" },
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
      tabs: t => call("tabs", t), go: h => call("go", h), copy: t => call("copy", String(t)), say: e => call("say", String(e?.message ?? e)),
      rows: () => call("rows"), put: (k, v) => call("put", k, v), removeRow: k => call("remove", k), lib: n => call("lib", n),
      onRoute: f => routeFns.push(f), onChange: f => changeFns.push(f), ui,
    };
    // ITS HEIGHT, told (the page fits the frame to it).
    new ResizeObserver(() => parent.postMessage({ event: "height", h: document.documentElement.scrollHeight }, "*")).observe(document.body);
    import(document.currentScript.dataset.src).then(m => m.mount(api, document.body)).catch(e => (document.body.textContent = "This app did not start: " + (e?.message ?? e)));
  `;

  function mount(el, { app, bytes, kinds: own = [] }) {
    if (!KEY.test(app)) throw new Error(`an app's name is a-z, 0-9 and "-": ${app}`);
    const prefix = `${app}/`;
    const mine = new Set(own);
    const frame = document.createElement("iframe");
    frame.sandbox = "allow-scripts allow-forms allow-popups allow-modals";
    frame.title = app;
    frame.style.cssText = "border:0;width:100%;min-height:120px;display:block;background:transparent";
    const csp = "default-src 'none'; script-src 'unsafe-inline' data:; style-src 'unsafe-inline'; img-src data:; font-src data:; connect-src 'none'; frame-src 'none'; form-action 'none'";
    const theme = document.querySelector("style[data-theme]")?.textContent ?? "";
    frame.srcdoc = `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${csp}">
      <style>${theme.replace(/<\/style/gi, "")}</style>
      <style>body{margin:0;font-family:var(--cw-font);color:var(--cw-fg);background:transparent}.cw-card{display:grid;gap:4px;padding:var(--cw-space-3);text-decoration:none;color:inherit}
      .cw-excerpt{margin:0;white-space:pre-wrap;font:12px/1.4 var(--cw-font-mono);color:var(--cw-muted);max-height:6em;overflow:hidden}.cw-loading{color:var(--cw-muted)}</style></head>
      <body><script data-src="data:text/javascript;base64,${b64(bytes)}">${RUNTIME}</script></body></html>`;
    // The SYSTEM's item page, under the frame (for `…/p/<ref>`).
    const system = document.createElement("div");
    el.replaceChildren(frame, system);

    const where = async () => {
      const at = await (await ctx.require("where")).of({ kinds: own, app, yours: "Yours" });
      return { at, plain: { who: at.who, sub: at.sub, space: at.space ? { id: at.space.id, name: at.space.name ?? null } : null, saved: at.saved, others: at.others, base: at.base } };
    };
    const ofMine = async ref => {
      const it = await items.get(ref);
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
        if (!mine.has(it?.kind)) throw Object.assign(new Error(`this app makes ${[...mine].join(", ") || "nothing"}`), { kind: "refused" });
        const { kind, title, body, audience, meta } = it;
        return items.submit({ kind, title, body, audience, meta, board: ctx.space && ctx.space !== "discover" ? ctx.space : null });
      },
      edit: async (ref, body, meta) => (await ofMine(ref), await items.editItem(ref, body, { meta }), true),
      removeItem: async ref => (await ofMine(ref), await items.remove(ref), true),
      pageOf: async ref => items.pageOf(ref, [...mine][0]),
      linkOf: async ref => items.linkOf(ref, app),
      tabs: async entries => {
        ctx.actions[ctx.route] = (Array.isArray(entries) ? entries : []).slice(0, 12).map(e => ({ label: String(e.label ?? "").slice(0, 40), href: /^#\//.test(e.href) ? e.href : undefined, on: !!e.on, end: !!e.end }));
        dispatchEvent(new CustomEvent("craftworks:actions"));
        return true;
      },
      go: async hash => {
        if (!/^#\/[^\s]*$/.test(String(hash))) throw new Error("only a place in this site (#/…)");
        location.hash = hash;
        return true;
      },
      copy: async text => (await navigator.clipboard.writeText(String(text).slice(0, 100000)), true),
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
      put: async (key, value) => (await (await tableOf()).put(own_(key), String(value)), true),
      remove: async key => (await (await tableOf()).remove(own_(key)), true),
    };
    const onMessage = async e => {
      if (!frame.isConnected) return (removeEventListener("message", onMessage), removeEventListener("craftworks:route", onRoute));
      if (e.source !== frame.contentWindow) return;
      const m = e.data ?? {};
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

  return { mount };
}
