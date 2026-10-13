// APP HOST, a service (FOUNDATION.md §3, "Apps are installed"): runs an app in ITS OWN FRAME — a sandboxed `srcdoc`
// frame (an opaque origin of its own: it cannot reach this page, its DOM or its storage), with a content security
// policy set before any of the app's code (no network: `connect-src 'none'` — it never reaches the node or anywhere
// else) — and gives it the SYSTEM API by messages, the page enforcing every call. An app not ours runs only this way.
//
// The API (each call `{ id, op, args }` → `{ id, ok, value | error }`; events pushed as `{ event, … }`):
//   rows()                       → [{ key, value }] its OWN rows (the account's `appdata` table, keys `<app>/…`)
//   put(key, value) / remove(key) → its own rows only; a key is the app's, never another app's
//   places()                     → where it can be ([{ label, href, on }])
//   where()                      → { route, sub, space }
//   go(hash)                     → the page moves (only inside the site: `#/…`)
//   say(error)                   → the words a person reads for a failure
// and the event `changed` when its rows change.
//
//   const host = await ctx.require("apphost");
//   host.mount(el, { app: "sample", bytes })   // bytes: the app's module (it exports `mount(api, el)`)
export async function start(ctx) {
  const storage = await ctx.require("storage");
  const APPDATA = "appdata";
  const KEY = /^[a-z0-9][a-z0-9-]{0,31}$/;
  const b64 = bytes => {
    let s = "";
    for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return btoa(s);
  };

  // The frame's own runtime: the API as calls by message, then the app's module imported (a data: URL — a srcdoc
  // frame refuses blob: URLs) and mounted on its body.
  const RUNTIME = `
    const waiting = new Map(); let n = 0; const listeners = [];
    addEventListener("message", e => {
      if (e.source !== parent) return;
      const m = e.data ?? {};
      if (m.event) return listeners.forEach(f => f(m));
      const w = waiting.get(m.id); if (!w) return; waiting.delete(m.id);
      m.ok ? w.resolve(m.value) : w.reject(Object.assign(new Error(m.error), { kind: m.kind }));
    });
    const call = (op, ...args) => new Promise((resolve, reject) => { const id = ++n; waiting.set(id, { resolve, reject }); parent.postMessage({ id, op, args }, "*"); });
    const api = {
      rows: () => call("rows"), put: (k, v) => call("put", k, v), remove: k => call("remove", k),
      places: () => call("places"), where: () => call("where"), go: h => call("go", h), say: e => call("say", String(e?.message ?? e)),
      onChange: f => listeners.push(m => m.event === "changed" && f()),
    };
    import(document.currentScript.dataset.src).then(m => m.mount(api, document.body)).catch(e => (document.body.textContent = "This app did not start: " + (e?.message ?? e)));
  `;

  function mount(el, { app, bytes }) {
    if (!KEY.test(app)) throw new Error(`an app's name is a-z, 0-9 and "-": ${app}`);
    const prefix = `${app}/`;
    const frame = document.createElement("iframe");
    frame.sandbox = "allow-scripts allow-forms allow-popups";
    frame.title = app;
    frame.style.cssText = "border:0;width:100%;min-height:70vh;display:block;background:transparent";
    const csp = "default-src 'none'; script-src 'unsafe-inline' data:; style-src 'unsafe-inline'; img-src data:; font-src data:; connect-src 'none'; frame-src 'none'; form-action 'none'";
    frame.srcdoc = `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${csp}">
      <style>body{font:15px/1.5 system-ui,sans-serif;margin:0;padding:16px;color:CanvasText;background:Canvas}</style></head>
      <body><script data-src="data:text/javascript;base64,${b64(bytes)}">${RUNTIME}</script></body></html>`;
    el.replaceChildren(frame);

    let table = null;
    const tableOf = async () => (table ??= storage.table(APPDATA).catch(e => ((table = null), Promise.reject(e))));
    const own = key => {
      if (typeof key !== "string" || !key || key.length > 200) throw new Error("a key is text, 1 to 200 characters");
      return prefix + key;
    };
    const ops = {
      rows: async () => {
        const t = await tableOf();
        await t.decided?.();
        return t.rows().filter(r => r.key.startsWith(prefix) && r.value).map(r => ({ key: r.key.slice(prefix.length), value: r.value }));
      },
      put: async (key, value) => (await (await tableOf()).put(own(key), String(value)), true),
      remove: async key => (await (await tableOf()).remove(own(key)), true),
      places: async () => (await (await ctx.require("places")).list()).map(({ label, href, on }) => ({ label, href, on })),
      where: async () => ({ route: ctx.route, sub: ctx.sub ?? "", space: ctx.space ?? null }),
      go: async hash => {
        if (!/^#\/[^\s]*$/.test(String(hash))) throw new Error("only a place in this site (#/…)");
        location.hash = hash;
        return true;
      },
      say: async message => ctx.say(new Error(message)),
    };
    const onMessage = async e => {
      // Gone with its page (the app closed): no more messages taken.
      if (!frame.isConnected) return removeEventListener("message", onMessage);
      if (e.source !== frame.contentWindow) return;
      const { id, op, args = [] } = e.data ?? {};
      if (typeof id !== "number") return;
      const reply = m => frame.contentWindow?.postMessage({ id, ...m }, "*");
      if (!Object.hasOwn(ops, op)) return reply({ ok: false, error: `no such call: ${op}`, kind: "refused" });
      try {
        reply({ ok: true, value: await ops[op](...args) });
      } catch (err) {
        reply({ ok: false, error: String(err?.message ?? err), kind: err?.kind ?? "refused" });
      }
    };
    addEventListener("message", onMessage);
    // Its rows changed (here or on another device): told.
    tableOf().then(t => t.onChange(() => frame.isConnected && frame.contentWindow?.postMessage({ event: "changed" }, "*")), () => {});
    return frame;
  }

  return { mount };
}
