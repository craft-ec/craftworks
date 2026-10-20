// DOCS, an app (`#/docs`): documents written together, live — a LENS on `codoc` (each document an item of kind `doc`,
// its text the merge of every editor's changes). Yours (`#/docs`), a space's (`#/s/<space>/docs`), shared with you
// (Shared), saved, Discover — `where`'s tabs, as every app. Who may edit: the document's maker, a space's `edit`
// policy, or its editors when shared (`share`, the bar's "👥 Share with…"); everyone else reads. Edits from others
// arrive as their changes do (read again while it is open).
export async function mount(ctx, el) {
  const [items, where, codoc, actionsCap] = await Promise.all(["items", "where", "codoc", "actions"].map(n => ctx.require(n)));
  // (`doc-editor`: the page and its toolbar — what a word processor shows.)
  const h = (await ctx.require("theme")).h;
  el.innerHTML = `
    <style>@layer apps {
      .dc { display: grid; gap: var(--cw-space-3); padding: var(--cw-space-3) var(--cw-gutter); min-width: 0; }
      .dc h2 { margin: 0; }
      .dc .list { display: grid; gap: var(--cw-space-2); }
      .dc .list a { display: block; padding: var(--cw-space-2) var(--cw-space-3); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); }
      .dc .none, .dc .meta { color: var(--cw-muted); }
      .dc .said { color: var(--cw-danger); margin: 0; }
      .dc .new { display: grid; gap: var(--cw-space-2); max-width: 640px; }
    }</style>
    <div class="dc"></div>`;
  const root = el.querySelector(".dc");
  const at = await where.of({ kind: "doc", app: "docs", yours: "Your docs" });
  // ANOTHER ADDRESS in Docs (a document opened from the list, one just created): the app drawn again for it — read once
  // at mount, a link inside it changed nothing (10-20: Create and every document in the list did nothing).
  const stop = where.onMove(el, () => (stop(), el.replaceChildren(), mount(ctx, el)));
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

  // A DOCUMENT open: a page and its toolbar (`doc-editor`) over its text and marks (`codoc`); every edit sent in order,
  // what others write drawn once nothing of this page's is on its way and the person is not typing.
  async function edit(box, ref) {
    // (Strict: waited for, a failure said — a total read gives its fallback while it is still arriving.)
    const it = await items.get.strict(ref).catch(e => (fail(e), null));
    if (!it) return box.replaceChildren(h("p", { className: "none", textContent: "This document is not there, or not yours to read." }), said);
    const first = await codoc.state(ref).catch(e => (fail(e), null));
    let v = first?.v ?? 0;
    let queued = 0;
    let sending = Promise.resolve();
    // WHERE ITS CHANGES ARE (the shell's "Saving…" / "All changes saved"): every edit sent in order.
    let shell = null;
    const send = f => {
      queued += 1;
      shell?.status("saving");
      sending = sending.then(f).then(() => ((queued -= 1), queued || shell?.status("saved")), e => ((queued -= 1), fail(e), shell?.status("Not saved")));
    };
    const ed = (await ctx.require("doc-editor")).create({
      text: first?.text ?? "",
      marks: first?.marks ?? [],
      readOnly: !first?.mayEdit,
      onSplice: (pos, del, ins) => send(() => codoc.splice(ref, pos, del, ins)),
      onMark: (start, end, name, value, on) => send(() => codoc.mark(ref, start, end, name, value, on)),
    });
    // THE OFFICE SHELL (`office-shell`: the one top panel of Docs, Sheets and Slides) around its page.
    shell = (await ctx.require("office-shell")).create({
      app: { name: "Docs", icon: "📝", home: at.base },
      title: it.title || "Untitled document",
      readOnly: !first?.mayEdit,
      onRename: first?.mayEdit && it.mine !== false ? name => items.editItem(ref, it.body ?? "", { title: name }).catch(fail) : null,
      actions: actionsCap.bar(it, { comments: false }),
      menus: ed.menus,
      tools: ed.tools,
      edit: { undo: ed.undo, redo: ed.redo },
      canvas: ed.canvas,
    });
    shell.status(first?.ready === false ? "Opening its first change…" : first?.mayEdit ? "saved" : "View only");
    openArea = shell.el;
    box.replaceChildren(said, shell.el);
    // READ AGAIN while it is open: another's change drawn when nothing of this page's is still on its way.
    const tick = async () => {
      if (!shell.el.isConnected) return;
      if (!queued && !ed.busy()) {
        const s = await codoc.state(ref).catch(e => (fail(e), null));
        if (s && s.v !== v && !queued && !ed.busy()) {
          v = s.v;
          ed.set(s);
          ed.readOnly(!s.mayEdit);
          shell.readOnly(!s.mayEdit);
          if (!s.mayEdit) shell.status("View only");
        }
      }
      setTimeout(tick, 700);
    };
    setTimeout(tick, 700);
  }

  async function home(box) {
    const list = (await at.read({ whole: true }).catch(e => (fail(e), []))).filter(x => x.kind === "doc").sort((a, b) => (b.edited || b.at) - (a.edited || a.at));
    // ＋ NEW in the page itself (the menu's own "＋ New document" is the side panel's — not shown on a narrow screen).
    const mayMake = !at.guest && !at.discover && !at.saved && !at.shared && !at.others;
    box.replaceChildren(
      h("h2", { textContent: at.shared ? "Shared documents" : at.saved ? "Saved documents" : "Documents" }),
      mayMake ? h("a", { className: "cw-btn primary", href: at.href("new"), textContent: "＋ New document", style: "justify-self:start" }) : null,
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
