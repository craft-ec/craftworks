// PASTE, an app IN ITS OWN FRAME (FOUNDATION.md §3: every app runs framed, reaching the system only through its API):
// a pastebin. A paste is an item of kind `paste` — the note family (its policy, search, votes, comments, history are a
// note's) — with its LANGUAGE and an EXPIRY in `meta`. Write one (a large monospace editor), choose who sees it —
// Public, UNLISTED (anyone with its link, never listed in Discover) or Private — and when it expires; Save opens its
// page (the system's item page, drawn under this frame); its RAW view (`r/<ref>`: the text alone). Its menu: Your
// pastes, Saved, Discover, ＋ New paste. Everything through `api` — no capability here, no page DOM.
export async function mount(api, el) {
  const { h, loading, card, action } = api.ui;
  const style = h("style", {
    textContent: `
      .pb { display: grid; gap: var(--cw-space-3); padding: var(--cw-space-3) var(--cw-space-4, 16px); }
      .pb h2 { margin: 0; }
      .pb .new { display: grid; gap: var(--cw-space-2); }
      .pb .opts { display: flex; flex-wrap: wrap; gap: var(--cw-space-2); align-items: center; }
      .pb .opts label { display: inline-flex; gap: 6px; align-items: center; font-size: var(--cw-text-sm); color: var(--cw-muted); }
      .pb textarea.code { box-sizing: border-box; min-height: 45vh; width: 100%; font: 13px/1.5 var(--cw-font-mono); tab-size: 2; resize: vertical; white-space: pre; }
      .pb .list { display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: var(--cw-space-3); align-items: start; }
      .pb .said { color: var(--cw-danger); margin: 0; }
      .pb .none { color: var(--cw-muted); }
      .pb .tools { display: flex; flex-wrap: wrap; gap: var(--cw-space-2); align-items: center; }
      .pb pre.raw { margin: 0; padding: var(--cw-space-3); overflow: auto; font: 13px/1.5 var(--cw-font-mono); white-space: pre; }`,
  });
  const root = h("div", { className: "pb" });
  el.replaceChildren(style, root);
  const LANGUAGES = (await api.lib("highlight").catch(() => null))?.LANGUAGES ?? [["plain", "Plain text"]];
  const EXPIRY = [
    ["", "Never"],
    [String(3600e3), "1 hour"],
    [String(86400e3), "1 day"],
    [String(7 * 86400e3), "1 week"],
    [String(30 * 86400e3), "1 month"],
  ];
  const select = (opts, value) => h("select", {}, ...opts.map(([v, t]) => h("option", { value: v, textContent: t, selected: v === value })));
  const expired = it => !!it.meta?.expires && it.meta.expires < Date.now();

  // A PASTE's page: Raw and Copy link here; the item page itself is the system's, under this frame.
  async function page(ref, base) {
    const said = h("span", { className: "said" });
    const copy = h("button", {
      type: "button",
      className: "cw-btn",
      textContent: "Copy link",
      onclick: async () => api.copy(await api.linkOf(ref)).then(() => (copy.textContent = "Copied"), async e => (said.textContent = await api.say(e))),
    });
    root.replaceChildren(h("div", { className: "tools" }, h("a", { href: base, textContent: "← Paste" }), h("a", { className: "cw-btn", href: `#/paste/r/${encodeURIComponent(ref)}`, textContent: "Raw" }), copy, said));
  }

  // THE RAW VIEW: the text alone (to read, select, copy).
  async function raw(ref) {
    root.replaceChildren(loading("Opening…"));
    const it = await api.get(ref).catch(() => null);
    if (!it) return root.replaceChildren(h("p", { className: "none", textContent: "This paste is not there, or not yours to read." }));
    if (expired(it)) return root.replaceChildren(h("p", { className: "none", textContent: "This paste has expired." }));
    const copy = h("button", { type: "button", className: "cw-btn", textContent: "Copy", onclick: () => api.copy(it.body ?? "").then(() => (copy.textContent = "Copied")) });
    root.replaceChildren(h("div", { className: "tools" }, h("a", { href: await api.pageOf(ref), textContent: "← Back" }), copy), h("pre", { className: "raw cw-box", textContent: it.body ?? "" }));
  }

  // NEW (`new`, the ＋ tab): the editor and its options.
  function home() {
    const title = h("input", { type: "text", placeholder: "Title (optional)", maxLength: 300 });
    const lang = select(LANGUAGES, "plain");
    const who = select(
      [
        ["public", "Public (listed in Discover)"],
        ["unlisted", "Unlisted (anyone with the link)"],
        ["private", "Private (only you)"],
      ],
      "unlisted",
    );
    const expiry = select(EXPIRY, "");
    const text = h("textarea", { className: "code", placeholder: "Paste or type here…", spellcheck: false, maxLength: 200000 });
    // A Tab types a tab (not leaving the editor).
    text.addEventListener("keydown", e => {
      if (e.key !== "Tab" || e.shiftKey) return;
      e.preventDefault();
      text.setRangeText("\t", text.selectionStart, text.selectionEnd, "end");
    });
    const said = h("p", { className: "said" });
    const save = h("button", { type: "submit", className: "cw-btn primary", textContent: "Save" });
    const form = h("form", { className: "new cw-box" }, title, text, h("div", { className: "opts" }, h("label", {}, "Language ", lang), h("label", {}, "Who sees it ", who), h("label", {}, "Expires ", expiry), save), said);
    const saveOnce = action(save, async () => {
      said.textContent = "";
      try {
        const ttl = Number(expiry.value) || 0;
        const ref = await api.submit({
          kind: "paste",
          title: title.value,
          body: text.value,
          audience: who.value === "private" ? "private" : "public",
          meta: { language: lang.value, ...(ttl ? { expires: Date.now() + ttl } : {}), ...(who.value === "unlisted" ? { unlisted: true } : {}) },
        });
        await api.go(await api.pageOf(ref));
      } catch (err) {
        said.textContent = await api.say(err);
        throw err;
      }
    }, { busy: "Saving…", done: "Saved ✓" });
    form.onsubmit = e => {
      e.preventDefault();
      if (!text.value.trim()) return (said.textContent = "Nothing to save yet.");
      saveOnce().catch(() => {});
    };
    root.replaceChildren(h("h2", { textContent: "📄 New paste" }), form);
  }

  // A PLACE's pastes (yours, Saved, Discover, a person's) — expired ones, and elsewhere unlisted ones, left out.
  async function place(w) {
    const list = h("div", { className: "list" }, loading("Pastes…"));
    const title = w.saved ? "Saved pastes" : w.who === "discover" ? "Public pastes" : w.who === "person" ? "Their pastes" : "Your pastes";
    root.replaceChildren(h("h2", { textContent: `📄 ${title}` }), list);
    const all = (await api.read({ saved: w.saved }).catch(() => [])).filter(p => !expired(p) && !(w.others && p.meta?.unlisted));
    const none = w.saved ? "Nothing saved: “Save” on a paste keeps it here." : w.who === "discover" ? "No public pastes yet." : w.who === "person" ? "No pastes." : "Nothing yet: ＋ New paste above.";
    list.replaceChildren(...(all.length ? all.map(p => card(p, { href: `${w.base}/p/${encodeURIComponent(p.ref)}` })) : [h("p", { className: "none", textContent: none })]));
  }

  async function draw(w) {
    w ??= await api.where();
    const s = w.sub;
    const home_ = w.who === "mine" || w.who === "discover";
    await api.tabs([
      ...(home_ ? [{ label: "Your pastes", href: "#/paste", on: w.who === "mine" && !s }] : []),
      ...(home_ ? [{ label: "Saved", href: "#/paste/saved", on: w.saved }] : []),
      ...(home_ ? [{ label: "Discover", href: "#/discover/paste", on: w.who === "discover" }] : []),
      { label: "＋ New paste", href: "#/paste/new", on: s === "new", end: true },
    ]);
    if (s.startsWith("r/")) return raw(decodeURIComponent(s.slice(2)));
    const m = /(?:^|\/)p\/(.+)$/.exec(s);
    if (m) return page(decodeURIComponent(m[1]), w.base);
    if (s === "new") return home();
    return place(w);
  }
  api.onRoute(w => draw(w));
  await draw();
}
