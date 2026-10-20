// OFFICE APP, a module: the ONE way Docs, Sheets and Slides are an app — their list (yours, a space's, shared with you,
// saved), New (a title and who sees it), and an item OPEN: its editor (each app's own component) inside the office shell
// (`office-shell`: title, menus, toolbar), over a document written together (`codoc`: its text and marks, its cells).
// Every edit sent in order ("Saving…" / "All changes saved"); what others write drawn once nothing of this page's is on
// its way and the person is not typing. Each app's mount gives its spec:
//
//   (await ctx.require("office-app")).open(ctx, el, { kind: "doc", route: "docs", name: "Docs", icon: "📝", noun: "document" })
// ITS EDITOR, by its kind (`EDITORS`, each a component): `create({ text, marks, cells, readOnly, onSplice, onMark,
// onSet })` → `{ canvas, tools, menus, undo, redo, set(state), busy(), readOnly(on) }`.
export async function open(ctx, el, spec) {
  const { kind, route, name, icon, noun } = spec;
  // (Named, so the app check sees each: an editor loads only when its kind opens.)
  const EDITORS = { doc: () => ctx.require("doc-editor"), sheet: () => ctx.require("sheet-grid"), deck: () => ctx.require("slide-deck") };
  const nouns = spec.nouns ?? `${noun}s`;
  const Noun = noun[0].toUpperCase() + noun.slice(1);
  const [items, where, codoc, actionsCap] = await Promise.all(["items", "where", "codoc", "actions"].map(n => ctx.require(n)));
  const h = (await ctx.require("theme")).h;
  el.innerHTML = `
    <style>@layer apps {
      .oa { display: grid; gap: var(--cw-space-3); padding: var(--cw-space-3) var(--cw-gutter); min-width: 0; }
      .oa h2 { margin: 0; }
      .oa .list { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: var(--cw-space-3); }
      .oa .list a { display: grid; gap: 6px; padding: var(--cw-space-3); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); background: var(--cw-surface); color: var(--cw-fg); text-decoration: none; }
      .oa .list a:hover { box-shadow: var(--cw-shadow); }
      .oa .list .ic { font-size: 28px; }
      .oa .list .when { color: var(--cw-muted); font-size: var(--cw-text-xs); }
      .oa .none { color: var(--cw-muted); }
      .oa .said { color: var(--cw-danger); margin: 0; }
      .oa .new { display: grid; gap: var(--cw-space-2); max-width: 640px; }
    }</style>
    <div class="oa"></div>`;
  const root = el.querySelector(".oa");
  const at = await where.of({ kind, app: route, yours: `Your ${nouns}` });
  // ANOTHER ADDRESS in the app (an item opened from the list, one just made): drawn again for it (`where.onMove`).
  const stop = where.onMove(el, () => (stop(), el.replaceChildren(), open(ctx, el, spec)));
  const sub = at.sub;
  at.tabs([], { yoursOn: !sub, create: at.discover || at.saved || at.shared ? null : { label: `New ${noun}`, href: at.href("new"), on: sub === "new" } });
  const said = h("p", { className: "said" });
  // (In an app's frame `ctx.say` answers through the host: awaited, or a refusal would be shown as nothing.)
  // (A quiet kind is said as nothing: never the word "null".)
  const fail = async e => (said.textContent = (await ctx.say(e)) ?? "");

  // NEW: its title and who sees it (the one picker); made, it opens.
  async function create(box) {
    const title = h("input", { className: "field", placeholder: "Title", maxLength: 300 });
    const who = await (await ctx.require("audience")).picker({ space: at.space, kind, initial: at.space ? "members" : "private" });
    const go = h("button", { type: "button", className: "cw-btn primary", textContent: "Create", onclick: async () => {
      go.disabled = true;
      const ref = await codoc.create({ board: at.space?.id ?? null, title: title.value, audience: who.value(), kind }).catch(e => (fail(e), null));
      go.disabled = false;
      if (ref) location.hash = at.href(`p/${encodeURIComponent(ref)}`);
    } });
    box.replaceChildren(h("h2", { textContent: `New ${noun}` }), h("div", { className: "new" }, title, who.el, go), said);
  }

  // AN ITEM OPEN: its editor in the shell; every edit sent in order; others' changes drawn when this page is quiet.
  async function edit(box, ref) {
    // (Strict: waited for, a failure said — a total read gives its fallback while it is still arriving.)
    const it = await items.get.strict(ref).catch(e => (fail(e), null));
    if (!it) return box.replaceChildren(h("p", { className: "none", textContent: `This ${noun} is not there, or not yours to read.` }), said);
    const first = await codoc.state(ref).catch(e => (fail(e), null));
    let v = first?.v ?? 0;
    let queued = 0;
    let sending = Promise.resolve();
    let shell = null;
    const send = f => {
      queued += 1;
      shell?.status("saving");
      sending = sending.then(f).then(() => ((queued -= 1), queued || shell?.status("saved")), e => ((queued -= 1), fail(e), shell?.status("Not saved")));
    };
    const ed = (await EDITORS[kind]()).create({
      text: first?.text ?? "",
      marks: first?.marks ?? [],
      cells: first?.cells ?? {},
      readOnly: !first?.mayEdit,
      onSplice: (pos, del, ins) => send(() => codoc.splice(ref, pos, del, ins)),
      onMark: (start, end, name_, value, on) => send(() => codoc.mark(ref, start, end, name_, value, on)),
      onSet: (key, value) => send(() => codoc.set(ref, key, value)),
    });
    shell = (await ctx.require("office-shell")).create({
      app: { name, icon, home: at.base },
      title: it.title || `Untitled ${noun}`,
      readOnly: !first?.mayEdit,
      onRename: first?.mayEdit ? title => items.editItem(ref, it.body ?? "", { title }).catch(fail) : null,
      actions: actionsCap.bar(it, { comments: false }),
      menus: ed.menus,
      tools: ed.tools,
      edit: { undo: ed.undo, redo: ed.redo },
      canvas: ed.canvas,
    });
    shell.status(first?.ready === false ? "Opening its first change…" : first?.mayEdit ? "saved" : "View only");
    openArea = shell.el;
    box.replaceChildren(said, shell.el);
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

  // THE LIST: its items as cards, newest first; ＋ New in the page itself (the side panel's is not shown on a narrow screen).
  async function home(box) {
    const list = (await at.read({ whole: true }).catch(e => (fail(e), []))).filter(x => x.kind === kind).sort((a, b) => (b.edited || b.at) - (a.edited || a.at));
    const mayMake = !at.guest && !at.discover && !at.saved && !at.shared && !at.others;
    // (Nothing in its place where nothing is made here: `replaceChildren` writes a null as the word "null".)
    box.replaceChildren(
      ...[h("h2", { textContent: at.shared ? `Shared ${nouns}` : at.saved ? `Saved ${nouns}` : `${Noun}s` }),
      mayMake ? h("a", { className: "cw-btn primary", href: at.href("new"), textContent: `＋ New ${noun}`, style: "justify-self:start" }) : null,
      list.length
        ? h("div", { className: "list" }, ...list.map(d => h("a", { href: at.href(`p/${encodeURIComponent(d.ref)}`) }, h("span", { className: "ic", textContent: icon }), h("b", { textContent: d.title || `Untitled ${noun}` }), h("span", { className: "when", textContent: new Date(d.edited || d.at).toLocaleString() }))))
        : h("p", { className: "none", textContent: `No ${nouns} here yet.` }),
      said].filter(Boolean),
    );
  }

  // ONE VIEW (`ctx.view`, render form); an open editor never drawn again under the person (`hold`, while on screen).
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
