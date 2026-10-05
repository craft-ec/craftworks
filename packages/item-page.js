// ITEM PAGE, a component: THE page of any item — a post, a video, an audio, an image, a note, a file — wherever it is
// opened from. Its KIND decides what shows (`kinds.parts`): its look whole (a video's or an audio's player — `media-look`
// —, else its card's look, `cards`), its votes, its about, its comments; everything around it is the same for every
// kind: who made it, where, when; one action bar (`actions`); one comment thread (`comments`). An app only frames it.
//
//   const page = await ctx.require("item-page");
//   host.append(await page.show(ref, { outside, app, back, discover, item }))
//     // outside: the space's public description when this person is not in it; app: whose page (comments' files);
//     // back: where to go once it is removed; discover: shown from Discover (Flag, not Edit); item: read already
export async function start(ctx) {
  const [items, kinds, cards, actions, markdown, attachments, mdEditor] = await Promise.all(["items", "kinds", "cards", "actions", "markdown", "attachments", "md-editor"].map(n => ctx.require(n)));
  const style = document.createElement("style");
  style.textContent = `
    .cw-page { display: grid; gap: var(--cw-space-3); }
    .cw-page h1 { font-size: 1.25rem; margin: 0; overflow-wrap: anywhere; }
    .cw-page .line { display: flex; gap: var(--cw-space-3); align-items: center; flex-wrap: wrap; }
    .cw-page .s { color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .cw-page .fields { display: flex; gap: var(--cw-space-3); flex-wrap: wrap; font-size: var(--cw-text-sm); color: var(--cw-muted); }
    .cw-page .about { background: var(--cw-hover); border-radius: var(--cw-radius); padding: var(--cw-space-3); overflow-wrap: anywhere; }
    .cw-page .thread { background: var(--cw-surface); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); padding: var(--cw-space-3); }
    .cw-page .thread h3 { margin: 0 0 var(--cw-space-2); font-size: 1rem; }
    .cw-page .said { color: var(--cw-danger); margin: 0; }
    .cw-page .editing { display: grid; gap: var(--cw-space-2); }
    .cw-page .editing .row { display: flex; gap: var(--cw-space-2); justify-content: flex-end; align-items: center; }`;
  document.head.append(style);
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };

  // WHAT SOMEONE WROTE, whole (`markdown`): its media and embedded items where they were written, its other files below.
  const written = it =>
    h("div", { className: "text" }, it.body ? markdown.render(it.body, it.files ?? [], { item: it.ref }) : null, attachments.show((it.files ?? []).filter(f => !markdown.inlined(it.body).has(markdown.keyOf(f)))));

  // EDIT one's own in place: its text and its files (kept where it is — its space's, public as it is; or the profile's),
  // saved as a new version (shown "(edited)").
  async function editIn(host, it, done) {
    const sp = it.board ? await items.boardOf(it.board.id) : null;
    const pub = it.board ? !!it.pub : !it.private;
    const pick = attachments.picker({ space: sp, from: { app: items.appOf(it.kind) }, public: () => pub, media: true, publish: true });
    pick.preset(it.files ?? []);
    const ed = mdEditor.create({ value: it.body ?? "", pick, label: "Edit" });
    // Its TAGS edited with it (the one pair of fields: `publisher.tagFields`).
    const tg = (await ctx.require("publisher")).tagFields(items.tagsOf(it));
    const said = h("p", { className: "said", hidden: true });
    const save = h("button", { type: "button", className: "go", textContent: "Save" });
    save.onclick = async () => {
      if (ed.busy()) return ((said.textContent = "Still sending the files: a moment…"), (said.hidden = false));
      save.disabled = true;
      const tags = tg.value();
      await items.editItem(it.ref, ed.value().trim(), { files: ed.files(), meta: { ...(it.meta ?? {}), tags } }).then(done, e => ((said.textContent = e.message ?? String(e)), (said.hidden = false)));
      save.disabled = false;
    };
    // ITS HISTORY where it is edited (`history`): an earlier version restored instead of saving this.
    const hist = (await ctx.require("history")).button(it.ref, { current: it, restored: () => done() });
    host.replaceChildren(h("div", { className: "editing" }, ed.el, tg.el, h("div", { className: "row" }, said, hist, h("button", { type: "button", textContent: "Cancel", onclick: () => done() }), save)));
    ed.focus();
  }

  async function show(ref, { outside = null, app = null, back = null, discover = false, item = null, shown = false } = {}) {
    const it = item ?? (await items.get(ref, { outside }));
    if (!it) return h("p", { className: "none", textContent: "This is not here (removed, or not shared with you)." });
    // ADULT CONTENT (tagged NSFW) for someone who did not choose to see it: said, and shown only on asking.
    if (!shown && items.isNsfw(it) && !(await items.nsfwShown())) {
      const space = await ctx.require("space");
      if (it.by !== (await space.account())?.id) {
        const gate = h(
          "div",
          { className: "cw-page" },
          h("div", { className: "about" }, h("p", { textContent: "🔞 Adult content (NSFW). You chose not to see it: Settings → Content shows it everywhere." }), h("button", { type: "button", textContent: "Show anyway", onclick: async () => gate.replaceWith(await show(ref, { outside, app, back, discover, item: it, shown: true })) })),
        );
        return gate;
      }
    }
    const parts = kinds.parts(it.kind);
    // VIEWED: its page opened counts once for this reader (`items.view`; never one's own), where votes are kept.
    if (parts.votes) items.view(ref, { outside });
    const page = h("div", { className: "cw-page" });
    const redraw = async () => page.replaceWith(await show(ref, { outside, app, back, discover }));
    const removed = () => (location.hash = back ?? items.pageOf(ref, it.kind).replace(/\/(p|w)\/.*$/, ""));
    const votes = parts.votes ? actions.votes(it, { outside, row: parts.look === "player" }) : null;
    let host = null; // where its text is, to edit in place
    const bar = actions.bar(it, { outside, comments: false, discover, removed, changed: redraw, edit: parts.look === "card" && parts.votes ? () => editIn(host, it, redraw) : null });
    // ITS LOOK, WHOLE.
    if (parts.look === "player" || parts.look === "picture" || parts.look === "reader") {
      const m = (await ctx.require("media-look")).full(it, { outside });
      const k = kinds.of(it.kind);
      const fields = (k?.fields ?? []).filter(x => it.meta?.[x]).map(x => h("span", { textContent: `${kinds.fieldLabel(x)}: ${it.meta[x]}` }));
      page.append(
        ...[
          m.el,
          h("h1", { textContent: it.title }),
          h("div", { className: "line" }, h("span", { className: "s" }, cards.author(it.by, items.appOf(it.kind)), ` · ${cards.ago(it.at)}${k ? ` · ${k.label}` : ""}${cards.counted(it)}${it.private ? " · only you" : ""}`), votes, bar),
          fields.length ? h("div", { className: "fields" }, ...fields) : null,
          cards.marks(it),
          parts.about && it.body ? h("div", { className: "about" }, written(it)) : null,
        ].filter(Boolean),
      );
      queueMicrotask(() => m.start());
    } else {
      host = h("div", {}, written(it));
      page.append(cards.card(it, { href: null, body: host, lead: votes, actions: [bar], by: true }));
    }
    // ITS COMMENTS: the one thread.
    if (parts.comments) {
      const thread = await (await ctx.require("comments")).create({ item: it, outside, app: app ?? items.appOf(it.kind) });
      page.append(h("div", { className: "thread" }, h("h3", { textContent: "Comments" }), thread.el));
    }
    return page;
  }

  return { show };
}
