// DOCS, an app (`#/docs`): documents written together, live — a LENS on `codoc` (each document an item of kind `doc`,
// its text the merge of every editor's changes). Yours (`#/docs`), a space's (`#/s/<space>/docs`), shared with you
// (Shared), saved, Discover — `where`'s tabs, as every app. Who may edit: the document's maker, a space's `edit`
// policy, or its editors when shared (`share`, the bar's "👥 Share with…"); everyone else reads. Edits from others
// arrive as their changes do (read again while it is open).
export async function mount(ctx, el) {
  const [items, where, codoc, actionsCap] = await Promise.all(["items", "where", "codoc", "actions"].map(n => ctx.require(n)));
  const h = (await ctx.require("theme")).h;
  el.innerHTML = `
    <style>@layer apps {
      .dc { display: grid; gap: var(--cw-space-3); padding: var(--cw-space-3) var(--cw-gutter); min-width: 0; }
      .dc h2 { margin: 0; }
      .dc .list { display: grid; gap: var(--cw-space-2); }
      .dc .list a { display: block; padding: var(--cw-space-2) var(--cw-space-3); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); }
      .dc .none, .dc .meta { color: var(--cw-muted); }
      .dc .said { color: var(--cw-danger); margin: 0; }
      .dc textarea.body { box-sizing: border-box; width: 100%; min-height: 60vh; font: inherit; line-height: 1.6; padding: var(--cw-space-3); resize: vertical; }
      .dc .new { display: grid; gap: var(--cw-space-2); max-width: 640px; }
    }</style>
    <div class="dc"></div>`;
  const root = el.querySelector(".dc");
  const at = await where.of({ kind: "doc", app: "docs", yours: "Your docs" });
  const sub = at.sub;
  at.tabs([], { yoursOn: !sub, create: at.discover || at.saved || at.shared ? null : { label: "New document", href: at.href("new"), on: sub === "new" } });
  const said = h("p", { className: "said" });
  // (In an app's frame `ctx.say` answers through the host: awaited, or a refusal would be shown as nothing.)
  const fail = async e => (said.textContent = await ctx.say(e));

  // NEW: its title and who sees it (the one picker); made, it opens.
  async function create(box) {
    const title = h("input", { className: "field", placeholder: "Title", maxLength: 300 });
    const who = await (await ctx.require("audience")).picker({ space: at.space, kind: "doc", initial: at.space ? "members" : "private" });
    const go = h("button", { type: "button", className: "cw-btn", textContent: "Create", onclick: async () => {
      go.disabled = true;
      const ref = await codoc.create({ board: at.space?.id ?? null, title: title.value, audience: who.value() }).catch(e => (fail(e), null));
      go.disabled = false;
      if (ref) location.hash = at.href(`p/${encodeURIComponent(ref)}`);
    } });
    box.replaceChildren(h("h2", { textContent: "New document" }), h("div", { className: "new" }, title, who.el, go), said);
  }

  // A DOCUMENT open: its text, edited here; what others write arrives as their changes do.
  async function edit(box, ref) {
    // (Strict: waited for, a failure said — a total read gives its fallback while it is still arriving.)
    const it = await items.get.strict(ref).catch(e => (fail(e), null));
    if (!it) return box.replaceChildren(h("p", { className: "none", textContent: "This document is not there, or not yours to read." }), said);
    const area = h("textarea", { className: "body", spellcheck: true });
    const meta = h("p", { className: "meta" });
    let shown = "";
    let v = 0;
    let sending = Promise.resolve();
    let queued = 0;
    const draw = s => {
      area.readOnly = !s.mayEdit;
      meta.textContent = s.ready ? (s.mayEdit ? "Everyone with edit rights writes here at once." : "You may read this document.") : "Opening its first change…";
      if (s.text === shown) return;
      // ANOTHER's CHANGE: the cursor kept where it was, moved by what was put before it.
      const [a, b] = [area.selectionStart, area.selectionEnd];
      let i = 0;
      while (i < shown.length && i < s.text.length && shown[i] === s.text[i]) i++;
      const d = s.text.length - shown.length;
      area.value = shown = s.text;
      area.setSelectionRange(a > i ? a + d : a, b > i ? b + d : b);
    };
    // AN EDIT here: what changed from the text last shown (its common start and end), sent in order.
    area.oninput = () => {
      const now = area.value;
      let i = 0;
      while (i < shown.length && i < now.length && shown[i] === now[i]) i++;
      let j = 0;
      while (j < shown.length - i && j < now.length - i && shown[shown.length - 1 - j] === now[now.length - 1 - j]) j++;
      const del = shown.length - i - j;
      const ins = now.slice(i, now.length - j);
      shown = now;
      queued += 1;
      sending = sending.then(() => codoc.splice(ref, i, del, ins)).then(() => (queued -= 1), e => ((queued -= 1), fail(e)));
    };
    openArea = area;
    box.replaceChildren(h("a", { href: at.base, textContent: "← Docs" }), h("h2", { textContent: it.title || "Untitled document" }), meta, area, actionsCap.bar(it, { comments: false }), said);
    // READ AGAIN while it is open: another's change drawn when nothing of this page's is still on its way.
    const tick = async () => {
      if (!area.isConnected) return;
      if (!queued) {
        const s = await codoc.state(ref).catch(e => (fail(e), null));
        if (s && (s.v !== v || s.text !== shown) && !queued && area.value === shown) ((v = s.v), draw(s));
      }
      setTimeout(tick, 700);
    };
    const first = await codoc.state(ref).catch(e => (fail(e), null));
    if (first) (v = first.v), draw(first);
    setTimeout(tick, 700);
  }

  async function home(box) {
    const list = (await at.read({ whole: true }).catch(e => (fail(e), []))).filter(x => x.kind === "doc").sort((a, b) => (b.edited || b.at) - (a.edited || a.at));
    box.replaceChildren(
      h("h2", { textContent: at.shared ? "Shared documents" : at.saved ? "Saved documents" : "Documents" }),
      list.length
        ? h("div", { className: "list" }, ...list.map(d => h("a", { href: at.href(`p/${encodeURIComponent(d.ref)}`), textContent: d.title || "Untitled document" })))
        : h("p", { className: "none", textContent: "No documents here yet." }),
      said,
    );
  }

  // ONE VIEW (`ctx.view`, render form) for every screen of it; an open editor is never drawn again under the person
  // (`hold`) — what others write reaches it through the document's own reads.
  // (Held while its editor is ON SCREEN — never before its first drawing: held then, it would never be drawn.)
  let openArea = null;
  ctx.view(root, {
    render: async () => {
      const box = h("div", { style: "display:contents" });
      if (sub === "new") await create(box);
      else if (sub.startsWith("p/")) await edit(box, decodeURIComponent(sub.slice(2)));
      else await home(box);
      return box;
    },
    hold: () => !!openArea?.isConnected,
  });
}
