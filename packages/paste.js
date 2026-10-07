// PASTE, an app (`#/paste`): a pastebin over the shared parts. A paste is an item of kind `paste` — the note family
// (`kinds`: its policy, search, votes, comments, history are a note's) — with its LANGUAGE and an EXPIRY in `meta`.
// Write one (a large monospace editor), choose who sees it — Public, UNLISTED (anyone with its link, never listed in
// Discover) or Private — and when it expires; Save gives its page; the tabs are every app's (`where`: Your pastes, Saved, Discover, ＋ New paste) (`…/paste/…/p/<ref>`: the item page, the code
// coloured by `highlight`, its action row) and its RAW view (`#/paste/r/<ref>`: the text alone). Below: your pastes.
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  const [items, cards, theme, hl] = await Promise.all(["items", "cards", "theme", "highlight"].map(n => ctx.require(n)));
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  el.innerHTML = `
    <style>@layer apps {
      .pb { display: grid; gap: var(--cw-space-3); padding: var(--cw-space-3) var(--cw-gutter); }
      .pb h2 { margin: 0; }
      .pb .new { display: grid; gap: var(--cw-space-2); }
      .pb .opts { display: flex; flex-wrap: wrap; gap: var(--cw-space-2); align-items: center; }
      .pb .opts label { display: inline-flex; gap: 6px; align-items: center; font-size: var(--cw-text-sm); color: var(--cw-muted); }
      .pb textarea.code { box-sizing: border-box; min-height: 45vh; width: 100%; font: 13px/1.5 ui-monospace, Menlo, Consolas, monospace; tab-size: 2; resize: vertical; white-space: pre; }
      .pb .list { display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: var(--cw-space-3); align-items: start; }
      .pb .said { color: var(--cw-danger); margin: 0; }
      .pb .none { color: var(--cw-muted); }
      .pb .tools { display: flex; flex-wrap: wrap; gap: var(--cw-space-2); align-items: center; }
      .pb pre.raw { margin: 0; padding: var(--cw-space-3); overflow: auto; font: 13px/1.5 ui-monospace, Menlo, Consolas, monospace; white-space: pre; }
    }</style>
    <div class="pb"></div>`;
  const root = el.querySelector(".pb");
  const EXPIRY = [
    ["", "Never"],
    [String(3600e3), "1 hour"],
    [String(86400e3), "1 day"],
    [String(7 * 86400e3), "1 week"],
    [String(30 * 86400e3), "1 month"],
  ];
  const select = (opts, value) => h("select", {}, ...opts.map(([v, t]) => h("option", { value: v, textContent: t, selected: v === value })));
    const refIn = s => decodeURIComponent(s.slice(s.indexOf("p/") + 2));

  // A PASTE's page: the one item page, framed here, with Raw and Copy link beside it.
  async function page(ref) {
    const page = await (await ctx.require("item-page")).show(ref, { app: "paste", back: "#/paste" });
    const said = h("span", { className: "said" });
    const copy = h("button", {
      type: "button",
      className: "cw-btn",
      textContent: "Copy link",
      onclick: () => navigator.clipboard?.writeText(items.linkOf(ref, "paste")).then(() => (copy.textContent = "Copied"), e => (said.textContent = e.message)),
    });
    root.replaceChildren(h("div", { className: "tools" }, h("a", { href: "#/paste", textContent: "← Paste" }), h("a", { className: "cw-btn", href: `#/paste/r/${encodeURIComponent(ref)}`, textContent: "Raw" }), copy, said), page);
  }

  // THE RAW VIEW: the text alone (to read, select, copy), and a download.
  async function raw(ref) {
    root.replaceChildren(theme.loading("Opening…"));
    const it = await items.get(ref).catch(() => null);
    if (!it) return root.replaceChildren(h("p", { className: "none", textContent: "This paste is not there, or not yours to read." }));
    if (expired(it)) return root.replaceChildren(h("p", { className: "none", textContent: "This paste has expired." }));
    const blob = URL.createObjectURL(new Blob([it.body ?? ""], { type: "text/plain" }));
    root.replaceChildren(
      h("div", { className: "tools" }, h("a", { href: items.pageOf(ref, "paste"), textContent: "← Back" }), h("a", { className: "cw-btn", href: blob, download: `${(it.title || "paste").replace(/[^\w.-]+/g, "_")}.txt`, textContent: "Download" })),
      h("pre", { className: "raw cw-box", textContent: it.body ?? "" }),
    );
  }
  const expired = it => !!it.meta?.expires && it.meta.expires < Date.now();

  // NEW (`#/paste/new`, the ＋ tab): the editor and its options.
  async function home() {
    const title = h("input", { type: "text", placeholder: "Title (optional)", maxLength: 300 });
    const lang = select(hl.LANGUAGES, "plain");
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
    const save = h("button", {
      type: "submit",
      className: "cw-btn primary",
      textContent: "Save",
    });
    const form = h(
      "form",
      { className: "new cw-box" },
      title,
      text,
      h("div", { className: "opts" }, h("label", {}, "Language ", lang), h("label", {}, "Who sees it ", who), h("label", {}, "Expires ", expiry), save),
      said,
    );
    // SAVED ONCE (`theme.action`): the button locked and saying "Saving…" from the first press.
    const saveOnce = theme.action(save, () => saveNow(), { busy: "Saving…", done: "Saved ✓" });
    form.onsubmit = e => {
      e.preventDefault();
      if (!text.value.trim()) return (said.textContent = "Nothing to save yet.");
      saveOnce().catch(() => {});
    };
    async function saveNow() {
      said.textContent = "";
      try {
        const ttl = Number(expiry.value) || 0;
        const ref = await items.submit({
          kind: "paste",
          title: title.value,
          body: text.value,
          audience: who.value === "private" ? "private" : "public",
          meta: { language: lang.value, ...(ttl ? { expires: Date.now() + ttl } : {}), ...(who.value === "unlisted" ? { unlisted: true } : {}) },
        });
        location.hash = items.pageOf(ref, "paste");
      } catch (err) {
        said.textContent = err?.message ?? String(err);
        throw err;
      }
    }
    root.replaceChildren(h("h2", { textContent: "📄 New paste" }), form);
  }

  // A PLACE's pastes (`where`: yours, Saved, Discover, a person's) — expired ones, and on Discover unlisted ones, left out.
  async function place(at) {
    const list = h("div", { className: "list" }, theme.loading("Pastes…"));
    const title = at.saved ? "Saved pastes" : at.who === "discover" ? "Public pastes" : at.who === "person" ? "Their pastes" : "Your pastes";
    root.replaceChildren(h("h2", { textContent: `📄 ${title}` }), list);
    const all = (await at.read().catch(() => [])).filter(p => !expired(p) && !(at.others && p.meta?.unlisted));
    const none = at.saved ? "Nothing saved: “Save” on a paste keeps it here." : at.who === "discover" ? "No public pastes yet." : at.who === "person" ? "No pastes." : "Nothing yet: ＋ New paste above.";
    list.replaceChildren(...(all.length ? all.map(p => cards.card(p, { href: at.href(`p/${encodeURIComponent(p.ref)}`), actions: [], by: at.others })) : [h("p", { className: "none", textContent: none })]));
  }

  async function draw() {
    const at = await (await ctx.require("where")).of({ kind: "paste", app: "paste", yours: "Your pastes" });
    const s = at.sub;
    at.tabs([], { yoursOn: !s, create: { label: "New paste", href: "#/paste/new", on: s === "new" } });
    if (s.startsWith("r/")) return raw(decodeURIComponent(s.slice(2)));
    if (s.startsWith("p/")) return page(refIn(s));
    if (s === "new") return home();
    return place(at);
  }
  await draw();
  addEventListener("craftworks:route", () => el.isConnected && ctx.route === "/paste" && draw());
}
