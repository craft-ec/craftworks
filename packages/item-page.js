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
  style.textContent = `@layer components {

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
    .cw-page .editing .row { display: flex; gap: var(--cw-space-2); justify-content: flex-end; align-items: center; }
}`;
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
    const sp = it.board ? await items.boardOf.strict(it.board.id) : null;
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
      await items.editItem(it.ref, ed.value().trim(), { files: ed.files(), meta: { ...(it.meta ?? {}), tags } }).then(done, e => ((said.textContent = ctx.say(e)), (said.hidden = false)));
      save.disabled = false;
    };
    // ITS HISTORY where it is edited (`history`): an earlier version restored instead of saving this.
    const hist = it.edited ? (await ctx.require("history")).button(it.ref, { current: it, restored: () => done() }) : null;
    host.replaceChildren(h("div", { className: "editing" }, ed.el, tg.el, h("div", { className: "row" }, said, hist, h("button", { type: "button", textContent: "Cancel", onclick: () => done() }), save)));
    ed.focus();
  }

  async function show(ref, { outside = null, app = null, back = null, discover = false, item = null, shown = false } = {}) {
    const it = item ?? (await items.get.strict(ref, { outside }));
    if (!it) return h("p", { className: "none", textContent: "Not found" });
    // ADULT CONTENT (tagged NSFW) for someone who did not choose to see it: said, and shown only on asking.
    if (!shown && items.isNsfw(it) && !(await items.nsfwShown())) {
      const space = await ctx.require("space");
      if (it.by !== (await space.me.strict())) {
        const gate = h(
          "div",
          { className: "cw-page" },
          h("div", { className: "about" }, h("p", { textContent: "🔞 NSFW · hidden by your setting" }), h("button", { type: "button", textContent: "Show anyway", onclick: async () => gate.replaceWith(await show(ref, { outside, app, back, discover, item: it, shown: true })) })),
        );
        return gate;
      }
    }
    // A SEALED LEVEL'S ITEM (rewards §5a: a teaser naming where it is sealed): a member of its level group reads it
    // there; one at the level ASKS in (once: any member, or the creator's node with no page open, lets them in); one
    // below it is told where to give.
    const gate = it.meta?.gate;
    if (gate?.space && gate?.ref && (it.meta.level === "pro" || it.meta.level === "vip")) {
      const space = await ctx.require("space");
      if ((await space.mine.strict()).some(s => s.id === gate.space)) return show(gate.ref, { app, back, discover });
      const usage = await ctx.require("usage");
      const mine = await ctx.optional("item-page: usage.level", usage.level(), () => ({ id: "free", name: "Free" }));
      const name = it.meta.level === "vip" ? "VIP" : "PRO and VIP";
      const about = h("div", { className: "about" });
      if (await usage.reaches(mine.id, it.meta.level)) {
        // AT THE LEVEL: in by itself (its level the proof — `conversation.joinLevel`), then the item opens.
        const conversation = await ctx.require("conversation");
        if (gate.owner && gate.nonce) {
          const inNow = await conversation.joinLevel(gate).then(() => true, e => (ctx.failed("item", e, `entering ${gate.name ?? gate.space.slice(0, 8)}: ${e.message}`), false));
          if (inNow) return show(gate.ref, { app, back, discover });
        }
        about.append(h("p", { textContent: `🔒 ${name} · you: ${mine.name} · ${gate.owner ? "could not enter yet — try again shortly" : "made before levels could be entered: ask its author"}` }));
      } else about.append(h("p", { textContent: `🔒 ${name} · you: ${mine.name}` }), h("a", { href: "#/wallet/contribute", textContent: "Contribute (Wallet)" }));
      return h("div", { className: "cw-page" }, h("h1", { textContent: it.title || "" }), about);
    }
    // A LEVEL'S ITEM from before sealing (rewards §5a) for someone below it: said, with where to contribute — its
    // author always opens it. Enforced here, in Craftworks' pages only.
    const lv = it.meta?.level;
    if (lv === "pro" || lv === "vip") {
      const usage = await ctx.require("usage");
      if (!(await usage.mayOpen(lv, it.by))) {
        const mine = await ctx.optional("item-page: usage.level", usage.level(), () => ({ name: "Free" }));
        return h(
          "div",
          { className: "cw-page" },
          h("h1", { textContent: it.title || "" }),
          h("div", { className: "about" }, h("p", { textContent: `🔒 ${lv === "vip" ? "VIP" : "PRO · VIP"} · you: ${mine.name}` }), h("a", { href: "#/wallet/contribute", textContent: "Contribute (Wallet)" })),
        );
      }
    }
    const parts = kinds.parts(it.kind);
    // VIEWED: its page opened counts once for this reader (`items.view`; never one's own), where votes are kept.
    if (parts.votes) items.view(ref, { outside });
    const page = h("div", { className: "cw-page" });
    // OPENED: counted in this person's month (`usage`).
    // Its TIME: what played, for a video or an audio (`media-look`); else the time on this page.
    ctx.background("item-page: ctx.require", ctx.require("usage").then(u => {
      const info = { kind: it.kind, title: it.title ?? null };
      u.opened(it.ref ?? ref, info);
      if (kinds.parts(it.kind).look !== "player") u.reading(page, it.ref ?? ref, info);
    }));
    const redraw = async () => page.replaceWith(await show(ref, { outside, app, back, discover }));
    const removed = () => (location.hash = back ?? items.pageOf(ref, it.kind).replace(/\/(p|w)\/.*$/, ""));
    let host = null; // where its text is, to edit in place
    // THE SAME ROW as in a list (Grid's): 💬 its count — here it goes down to the comments.
    let threadAt = null;
    const bar = actions.bar(it, { outside, vote: parts.votes, comments: parts.comments, open: () => threadAt?.scrollIntoView({ behavior: "smooth", block: "start" }), discover, removed, changed: redraw, edit: parts.look === "card" && parts.votes ? () => editIn(host, it, redraw) : null });
    // ITS LOOK, WHOLE.
    if (parts.look === "player" || parts.look === "picture" || parts.look === "reader") {
      const m = (await ctx.require("media-look")).full(it, { outside });
      const k = kinds.of(it.kind);
      const fields = (k?.fields ?? []).filter(x => it.meta?.[x]).map(x => h("span", { textContent: `${kinds.fieldLabel(x)}: ${it.meta[x]}` }));
      page.append(
        ...[
          m.el,
          h("h1", { textContent: it.title }),
          h("div", { className: "line" }, h("span", { className: "s" }, cards.author(it.by, items.appOf(it.kind)), ` · ${cards.ago(it.at)}${k ? ` · ${k.label}` : ""}${cards.counted(it)}${it.private ? " · only you" : ""}`), bar),
          fields.length ? h("div", { className: "fields" }, ...fields) : null,
          cards.marks(it),
          parts.about && it.body ? h("div", { className: "about" }, written(it)) : null,
        ].filter(Boolean),
      );
      queueMicrotask(() => m.start());
    } else {
      host = h("div", {}, written(it));
      page.append(cards.card(it, { href: null, body: host, actions: [bar], by: true }));
    }
    // ITS EDITIONS (TOKEN §1c): bought and sold here; the author makes them (`editions`, null when none to show).
    // In an APP's FRAME: a SYSTEM SHEET the host draws (money moves there — never in an app's frame; FOUNDATION §3).
    // (Shown where it has editions, and to its author — who makes them there — as the page's own section is.)
    const nEds = Array.isArray(it.meta?.editions) ? it.meta.editions.length : 0;
    const eds = ctx.system
      ? nEds || it.by === (await (await ctx.require("space")).me())
        ? h("p", {}, h("button", { type: "button", className: "cw-btn", textContent: nEds ? `🪙 Editions (${nEds})` : "🪙 Editions", onclick: () => ctx.system("editions", { ref: it.ref, outside }).then(redraw, e => ctx.failed("editions", e)) }))
        : null
      : await (await ctx.require("editions")).section(it, { changed: redraw }).catch(e => (ctx.log("editions", { what: e.message ?? String(e) }), null));
    if (eds) page.append(eds);
    // ITS COMMENTS: the one thread.
    if (parts.comments) {
      const thread = await (await ctx.require("comments")).create({ item: it, outside, app: app ?? items.appOf(it.kind) });
      // Its COUNT, from the very comments drawn (replies included) — the same rule as a list's count.
      const head = h("h3", { textContent: "Comments" });
      const label = n => `💬 ${n} Comment${n === 1 ? "" : "s"}`;
      thread.onCount?.(n => {
        head.textContent = n ? label(n) : "Comments";
        // The row's count is the thread's: the very comments drawn.
        it.comments = n;
        const b = [...bar.querySelectorAll("button")].find(x => x.textContent.startsWith("💬"));
        if (b) b.textContent = label(n);
      });
      threadAt = h("div", { className: "thread" }, head, thread.el);
      page.append(threadAt);
    }
    return page;
  }

  // ADD TO A BUNDLE, on every item's page (one place, every app): your bundles, and those of the space the item is in
  // (when you are in it) — a bundle holds items of any kind. Not on a bundle's own page, not from outside.
  async function withBundles(ref, o = {}) {
    const page = await show(ref, o);
    if (o.outside || o.discover || !page?.classList?.contains("cw-page")) return page;
    // The page returned AT ONCE; the picker put in when its reads answer (audit 10-13: a slow node held every item page
    // behind them).
    ctx.background("item page: bundles", offerBundles(ref, o, page));
    return page;
  }
  async function offerBundles(ref, o, page) {
    try {
      const space = await ctx.require("space");
      if (!(await space.me.strict())) return;
      const it = o.item ?? (await items.get(ref));
      if (!it || kinds.isBundle?.(it.kind)) return;
      const bundles = await ctx.require("bundles");
      // In an APP's FRAME: the host's picker lists your bundles — the app is given the one chosen, never the list.
      if (ctx.system) {
        const add = document.createElement("button");
        Object.assign(add, { type: "button", className: "cw-btn", textContent: "Add to a bundle…" });
        add.onclick = async () => {
          const bref = await ctx.system("pick-bundle", { ref }).catch(async e => ((await ctx.require("theme")).ask(add, { text: ctx.say(e) }), null));
          if (bref) await bundles.add(bref, ref).then(() => (add.textContent = "Added to a bundle ✓"), async e => (await ctx.require("theme")).ask(add, { text: ctx.say(e) }));
        };
        if (page.isConnected) page.prepend(add);
        return;
      }
      const all = await bundles.mineFor(ref);
      if (!all.length) return;
      const sel = document.createElement("select");
      sel.ariaLabel = "Add to a bundle";
      sel.append(Object.assign(document.createElement("option"), { value: "", textContent: "Add to a bundle…" }), ...all.map(b => Object.assign(document.createElement("option"), { value: b.ref, textContent: `${b.title}${b.has ? " ✓" : ""}` })));
      sel.onchange = () => sel.value && bundles.add(sel.value, ref).then(() => (sel.selectedOptions[0].textContent += " ✓", (sel.value = "")), async e => (await ctx.require("theme")).ask(sel, { text: ctx.say(e) }));
      if (page.isConnected) page.prepend(sel);
    } catch (e) {
      ctx.failed("item page", e, "add to a bundle not offered");
    }
  }
  return { show: withBundles };
}
