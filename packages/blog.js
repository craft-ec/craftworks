// BLOG, an app (`#/blog`): a LENS over Board's posts — the same posts, read as ARTICLES; no store of its own (§1, §7).
// Its states (a draft, a schedule, a review) are `blog-press`'s, kept on the posts themselves.
//   YOURS        `#/blog`                your articles (your public profile posts), newest first; `…/t/<tag>` by tag
//   A PERSON's   `#/blog/u/<did>`        theirs — SUBSCRIBE is the one follow (`person`): new posts reach your feed
//   A SPACE's    `#/s/<space>/blog`      a PUBLICATION: its members' articles; where its editors approve (its setting,
//                                        `app-settings`), a member's goes to them first (`…/review`, the editors')
//   DISCOVER     `#/discover/blog`       public articles
//   AN ARTICLE   `…/blog/p/<ref>`        cover, a wide reading column, reading time, contents, byline, tags, comments
//   WRITE        `…/blog/write[/<draft>]`   save a draft, schedule it, publish (or send to the editors)
//   DRAFTS       `#/blog/drafts`         your drafts, scheduled and in review, every place's
//   EDIT         `…/blog/edit/<ref>`     a published article changed (a new version: its history kept)
// UI only. Its look is `theme`'s tokens; what an item is, `items`'; who may, `roles`'.
export async function mount(ctx, el) {
  // WHERE it is — its spaces — shown as this app chooses (`places`: a subtle group line).
  ctx.background("blog: ctx.require", ctx.require("places").then(p => p.attach(el, "group")));
  const [items, press, theme, space, cards, markdown, attachments, directory, person] = await Promise.all(
    ["items", "blog-press", "theme", "space", "cards", "markdown", "attachments", "directory", "person"].map(n => ctx.require(n)),
  );
  const me = await space.me();
  const whereCap = await ctx.require("where");
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  el.innerHTML = `
    <style>@layer apps {
      .bl { display: grid; gap: var(--cw-space-3); padding: var(--cw-space-3) var(--cw-gutter); }
      .bl h2 { margin: 0; }
      .bl a { color: inherit; }
      .bl .s { color: var(--cw-muted); font-size: var(--cw-text-sm); }
      .bl .said { color: var(--cw-danger); margin: 0; }
      .bl .none { color: var(--cw-muted); text-align: center; padding: var(--cw-space-5); margin: 0; }
      .bl .head { display: flex; gap: var(--cw-space-3); align-items: center; flex-wrap: wrap; }
      .bl .head h2 { flex: 1; min-width: 0; overflow-wrap: anywhere; }
      .bl .chips { display: flex; gap: 6px; flex-wrap: wrap; }
      .bl .chips a { text-decoration: none; font-size: var(--cw-text-sm); padding: 2px 10px; border: 1px solid var(--cw-line); border-radius: var(--cw-radius-pill); }
      .bl .chips a.on { background: var(--cw-accent); color: var(--cw-accent-fg); border-color: transparent; }
      /* THE LIST: one article a row — its cover, its title, its first lines, who, when, how long, its tags. */
      .bl .list { display: grid; gap: var(--cw-space-3); max-width: 960px; width: 100%; margin: 0 auto; }
      .bl .row { display: grid; grid-template-columns: minmax(0, 1fr) 160px; gap: var(--cw-space-3); padding: var(--cw-space-3); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); background: var(--cw-surface); text-decoration: none; }
      .bl .row:not(:has(.thumb)) { grid-template-columns: minmax(0, 1fr); }
      .bl .row h3 { margin: 0 0 4px; font-size: 1.15rem; overflow-wrap: anywhere; }
      .bl .row p.ex { margin: 0 0 6px; color: var(--cw-muted); display: -webkit-box; -webkit-line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; }
      .bl .row .thumb { aspect-ratio: 4 / 3; border-radius: var(--cw-radius-sm); overflow: hidden; background: var(--cw-hover); }
      .bl .row .thumb img { width: 100%; height: 100%; object-fit: cover; display: block; }
      @media (max-width: 600px) { .bl .row { grid-template-columns: minmax(0, 1fr); } .bl .row .thumb { order: -1; aspect-ratio: 16 / 9; } }
      /* THE ARTICLE: a wide reading column, its contents beside it where there is room. */
      .bl .art { display: grid; grid-template-columns: minmax(0, 46rem) 14rem; gap: var(--cw-space-5); justify-content: center; }
      .bl .art > .col { display: grid; gap: var(--cw-space-3); min-width: 0; }
      .bl .art .cover { border-radius: var(--cw-radius); overflow: hidden; }
      .bl .art .cover > * { display: block; width: 100%; max-width: none; max-height: 60vh; margin: 0; }
      .bl .art .cover img { object-fit: cover; }
      .bl .art h1 { margin: 0; font-size: clamp(1.6rem, 4vw, 2.3rem); line-height: 1.2; overflow-wrap: anywhere; }
      .bl .art .by { display: flex; gap: 6px; align-items: baseline; flex-wrap: wrap; color: var(--cw-muted); font-size: var(--cw-text-sm); }
      .bl .art .body { font-size: 1.0625rem; }
      .bl .art .body .cw-md h1, .bl .art .body .cw-md h2, .bl .art .body .cw-md h3 { scroll-margin-top: var(--cw-space-4); }
      .bl .art .flag { padding: var(--cw-space-2) var(--cw-space-3); border-radius: var(--cw-radius-sm); background: var(--cw-hover); }
      .bl .art .thread { border-top: 1px solid var(--cw-line); padding-top: var(--cw-space-3); }
      .bl .art .thread h3 { margin: 0 0 var(--cw-space-2); }
      .bl .toc { position: sticky; top: var(--cw-space-3); align-self: start; font-size: var(--cw-text-sm); }
      .bl .toc h4 { margin: 0 0 var(--cw-space-2); font-size: var(--cw-text-xs); letter-spacing: .08em; text-transform: uppercase; color: var(--cw-muted); }
      .bl .toc ol { list-style: none; margin: 0; padding: 0; display: grid; gap: 4px; }
      .bl .toc button { font: inherit; text-align: left; background: none; border: 0; padding: 2px 0; color: var(--cw-fg); cursor: pointer; }
      .bl .toc button:hover { color: var(--cw-accent); }
      @media (max-width: 1000px) { .bl .art { grid-template-columns: minmax(0, 46rem); } .bl .toc { position: static; order: -1; border: 1px solid var(--cw-line); border-radius: var(--cw-radius); padding: var(--cw-space-3); } }
      /* WRITE */
      .bl form.write { display: grid; gap: var(--cw-space-2); max-width: 46rem; width: 100%; margin: 0 auto; }
      .bl form.write input:not([type=checkbox]), .bl form.write select { font: inherit; padding: 6px 8px; border: 1px solid var(--cw-line); border-radius: var(--cw-radius-sm); background: var(--cw-surface); color: var(--cw-fg); }
      .bl form.write input[name=title] { font-size: 1.4rem; font-weight: 600; }
      .bl form.write .line { display: flex; gap: var(--cw-space-2); align-items: center; flex-wrap: wrap; }
      .bl .tools { display: flex; gap: var(--cw-space-2); flex-wrap: wrap; align-items: center; }
    }</style>
    <div class="bl"></div>`;
  const root = el.querySelector(".bl");
  const ago = cards.ago;
  const day = at => new Date(at).toLocaleDateString(undefined, { year: "numeric", month: "long", day: "numeric" });
  const fail = (host, e) => host.replaceChildren(h("p", { className: "said", textContent: e?.message ?? String(e) }));
  const blogOf = it => (it?.meta?.blog && typeof it.meta.blog === "object" ? it.meta.blog : {});
  const minutes = it => press.readingTime(it.body).minutes;
  const thumbOf = it => {
    const f = press.coverOf(it, { kindOf: markdown.kindOf, keyOf: markdown.keyOf, inlined: markdown.inlined(it.body) }) ?? it.files?.find(x => markdown.kindOf(x) === "image");
    const src = f?.preview ?? (f?.inline ? `data:${f.type};base64,${f.inline}` : null);
    return src ? h("div", { className: "thumb" }, h("img", { src, alt: "", loading: "lazy" })) : null;
  };

  let at = null;
  let months = 1; // how far back a list reads (30 days each; "Older" reaches further)
  const base = () => at.base;

  // AN ARTICLE in a list.
  function row(it, { href }) {
    const tags = items.tagsOf(it).filter(t => t !== "nsfw");
    return h(
      "a",
      { className: "row", href },
      h(
        "div",
        {},
        h("h3", { textContent: it.title || "Untitled" }),
        it.body ? h("p", { className: "ex", textContent: markdown.plain(it.body).slice(0, 320) }) : null,
        h("div", { className: "s" }, cards.author(it.by, "blog"), ` · ${day(it.at)} · ${minutes(it) || 1} min read`, it.board ? ` · ${space.shown(it.board)}` : "", it.aud === "members" ? " · 👥 members" : "", tags.length ? ` · ${tags.map(t => `#${t}`).join(" ")}` : ""),
      ),
      thumbOf(it),
    );
  }

  // A PLACE's ARTICLES: newest first, by tag where one is chosen; what is read is its last `months` × 30 days.
  async function home(tag = null) {
    const list = h("div", { className: "list" }, theme.loading("Reading the articles…"));
    const sp = at.space;
    const r = sp ? await (await ctx.require("roles")).of(sp) : null;
    const title = at.who === "discover" ? "🧭 Public articles" : sp ? `📰 ${space.shown(sp)}` : at.who === "person" ? null : "✍️ Your blog";
    const head = h("div", { className: "head" }, title ? h("h2", { textContent: title }) : h("h2", {}, directory.nameEl(at.person, "span")));
    // SUBSCRIBE: the one follow — a person's card (Follow), a space's (Follow, Join).
    if (at.who === "person") head.append(h("button", { type: "button", className: "cw-btn primary", textContent: "Subscribe…", onclick: e => person.open(e.currentTarget, at.person) }));
    if (sp) {
      const editors = r.members().filter(m => r.can(m.did, "moderate"));
      head.append(h("span", { className: "s", textContent: `${editors.length} editor${editors.length === 1 ? "" : "s"} · ${r.config("blog", "approval", "") === "editors" ? "editors approve members' articles" : "members publish directly"}` }));
    }
    const chips = h("div", { className: "chips" });
    root.replaceChildren(head, chips, list);
    const where = at.who === "discover" ? { discover: true } : sp ? { board: sp.id } : { by: at.person ?? me };
    const all = (await items.list(where, "new", "post", { window: 30 * months })).filter(it => press.isArticle(it) && !it.private);
    // Its TAGS (what the read articles carry): a filter, newest first within it.
    const counts = new Map();
    for (const it of all) for (const t of items.tagsOf(it)) if (t !== "nsfw") counts.set(t, (counts.get(t) ?? 0) + 1);
    const shown = tag ? all.filter(it => items.tagsOf(it).includes(tag)) : all;
    chips.replaceChildren(
      ...(counts.size ? [h("a", { href: base(), className: tag ? "" : "on", textContent: "All" }), ...[...counts].sort((a, b) => b[1] - a[1]).slice(0, 30).map(([t, n]) => h("a", { href: at.href(`t/${encodeURIComponent(t)}`), className: t === tag ? "on" : "", textContent: `#${t} ${n}` }))] : []),
    );
    const older = h("button", { type: "button", className: "cw-btn", textContent: `Older (before ${30 * months} days ago)`, onclick: () => ((months += 1), draw()) });
    const none = at.who === "discover" ? "No public articles yet." : sp ? "No articles yet." : at.who === "person" ? "No articles." : "Nothing published yet: ＋ Write above.";
    list.replaceChildren(...(shown.length ? shown.map(it => row(it, { href: at.who === "discover" ? at.href(`p/${it.ref}`) : `${placeBase(it)}/p/${it.ref}` })) : [h("p", { className: "none", textContent: tag ? `Nothing tagged #${tag} in the last ${30 * months} days.` : none })]), older);
  }
  // Where an article opens: its space's blog, else its author's (yours: yours).
  const placeBase = it => (it.board?.id ? `#/s/${it.board.id}/blog` : it.by === me ? "#/blog" : `#/blog/u/${it.by}`);

  // AN ARTICLE whole.
  async function article(ref) {
    root.replaceChildren(theme.loading("Opening the article…"));
    const outside = await items.outsideOf(ref);
    const it = await items.get(ref, { outside });
    if (!it) return root.replaceChildren(h("p", { className: "none", textContent: "This article is not there (removed, or not found yet)." }));
    // GATED (adult content not chosen, a level's item): the one item page decides what shows.
    if (it.meta?.gate || it.meta?.level || (items.isNsfw(it) && it.by !== me && !(await items.nsfwShown())))
      return root.replaceChildren(await (await ctx.require("item-page")).show(ref, { outside, app: "blog", back: base(), discover: at.who === "discover" }));
    const b = blogOf(it);
    const inlined = markdown.inlined(it.body);
    const cover = press.coverOf(it, { kindOf: markdown.kindOf, keyOf: markdown.keyOf, inlined });
    const toc = press.headingsOf(it.body);
    const { minutes: min, words } = press.readingTime(it.body);
    items.view(ref, { outside });
    const col = h("div", { className: "col" });
    // REVIEW COPY: said, and — an editor's — approved or declined here.
    if (b.review === "pending") {
      const sp = it.board?.id ? await items.boardOf(it.board.id) : null;
      const editor = sp ? await press.mayEdit(sp) : false;
      col.append(h("div", { className: "flag" }, h("b", { textContent: "In review · " }), "members only until an editor approves it.", editor && it.by !== me ? verdict(ref, () => draw()) : null));
    }
    if (it.private) col.append(h("div", { className: "flag", textContent: b.draft ? "📝 A draft · only you" : "🔒 Only you" }));
    if (cover) col.append(h("div", { className: "cover" }, markdown.fileView(cover, { item: it.ref, alt: it.title })));
    col.append(h("h1", { textContent: it.title || "Untitled" }));
    const subscribe = it.by !== me ? h("button", { type: "button", className: "cw-btn", textContent: "Subscribe…", onclick: e => person.open(e.currentTarget, it.by) }) : null;
    col.append(
      h(
        "div",
        { className: "by" },
        cards.author(it.by, "blog"),
        h("span", { textContent: `· ${day(it.at)}` }),
        it.edited ? h("span", { textContent: `· updated ${ago(it.edited)}` }) : null,
        h("span", { textContent: `· ${min || 1} min read (${words} words)` }),
        it.board ? h("span", {}, "· in ", h("a", { href: `#/s/${it.board.id}/blog`, textContent: space.shown(it.board) })) : null,
        subscribe,
      ),
    );
    const tags = items.tagsOf(it).filter(t => t !== "nsfw");
    if (tags.length) col.append(h("div", { className: "chips" }, ...tags.map(t => h("a", { href: `${placeBase(it)}/t/${encodeURIComponent(t)}`, textContent: `#${t}` }))));
    // ITS TEXT whole (`markdown`), its other files below (its cover shown above).
    const md = markdown.render(it.body, it.files ?? [], { item: it.ref });
    const rest = (it.files ?? []).filter(f => !inlined.has(markdown.keyOf(f)) && f !== cover);
    col.append(h("div", { className: "body" }, md, rest.length ? attachments.show(rest, { item: it.ref }) : null));
    // ITS CONTENTS: each heading drawn (top level, in order) given its slug; a click scrolls there (the address is the app's).
    const drawn = [...md.querySelectorAll(":scope > h1, :scope > h2, :scope > h3, :scope > h4, :scope > h5, :scope > h6")];
    toc.forEach((x, i) => {
      const hd = drawn[i]?.textContent.trim() === x.text ? drawn[i] : drawn.find(d => d.textContent.trim() === x.text && !d.id);
      if (hd) hd.id = `h-${x.slug}`;
    });
    const nav =
      toc.length >= 2
        ? h(
            "nav",
            { className: "toc", ariaLabel: "Contents" },
            h("h4", { textContent: "Contents" }),
            h("ol", {}, ...toc.map(x => h("li", { style: `padding-left:${(x.level - Math.min(...toc.map(y => y.level))) * 12}px` }, h("button", { type: "button", textContent: x.text, onclick: () => md.querySelector(`#h-${CSS.escape(x.slug)}`)?.scrollIntoView({ behavior: "smooth", block: "start" }) })))),
          )
        : null;
    // ITS ACTIONS (`actions`: votes, share, save … edit, delete) and ITS COMMENTS (`comments`: the one thread).
    let threadAt = null;
    const actionsCap = await ctx.require("actions");
    const bar = actionsCap.bar(it, {
      outside,
      vote: true,
      open: () => threadAt?.scrollIntoView({ behavior: "smooth", block: "start" }),
      discover: at.who === "discover",
      removed: () => (location.hash = base()),
      changed: () => draw(),
      edit: it.by === me && !it.private ? () => (location.hash = `${placeBase(it)}/edit/${it.ref}`) : null,
      // Shared as the ARTICLE it is read as here (this page's own address), not its Board page.
      link: () => `${location.origin}${location.pathname}${location.hash}`,
    });
    col.append(bar);
    const thread = await (await ctx.require("comments")).create({ item: it, outside, app: "blog" });
    const head = h("h3", { textContent: "Comments" });
    thread.onCount?.(n => (head.textContent = n ? `💬 ${n} Comment${n === 1 ? "" : "s"}` : "Comments"));
    threadAt = h("div", { className: "thread" }, head, thread.el);
    col.append(threadAt);
    const page = h("div", { className: "art" }, col, nav);
    root.replaceChildren(page);
    // ITS TIME read (`usage`), as every item page counts it.
    ctx.background("blog: ctx.require", ctx.require("usage").then(u => (u.opened(it.ref, { kind: it.kind, title: it.title ?? null }), u.reading(page, it.ref, { kind: it.kind, title: it.title ?? null }))));
  }

  // AN EDITOR's WORD on a review copy: approve, or decline with a note.
  function verdict(ref, done) {
    const note = h("input", { type: "text", placeholder: "Note (optional)", maxLength: 280 });
    const said = h("span", { className: "said" });
    const go = yes => press.approve(ref, yes, note.value.trim()).then(done, e => (said.textContent = ctx.say(e)));
    return h("div", { className: "tools", style: "margin-top:6px" }, note, h("button", { type: "button", className: "cw-btn primary", textContent: "✅ Approve", onclick: () => go(true) }), h("button", { type: "button", className: "cw-btn", textContent: "❌ Decline", onclick: () => go(false) }), said);
  }

  // A PUBLICATION's REVIEW QUEUE (its editors).
  async function reviewPage() {
    const sp = at.space;
    root.replaceChildren(h("h2", { textContent: `📰 ${space.shown(sp)} · review` }), theme.loading("Reading what waits…"));
    if (!(await press.mayEdit(sp))) return root.replaceChildren(h("p", { className: "none", textContent: "Only this publication's editors (who may moderate here) review." }));
    const list = await press.review(sp);
    root.replaceChildren(
      h("h2", { textContent: `📰 ${space.shown(sp)} · review` }),
      h("p", { className: "s", textContent: "Members' articles waiting for an editor (the last 90 days). Approved: their author's page publishes it." }),
      ...(list.length
        ? list.map(it =>
            h(
              "div",
              { className: "row", style: "grid-template-columns:minmax(0,1fr)" },
              h("h3", {}, h("a", { href: at.href(`p/${it.ref}`), textContent: it.title || "Untitled" })),
              h("div", { className: "s" }, cards.author(it.by, "blog"), ` · sent ${ago(it.at)} · ${minutes(it) || 1} min read`, it.word ? ` · ${it.word.word === "approved" ? "✅ approved" : "❌ declined"} by ` : "", it.word ? directory.nameEl(it.word.by, "span") : null),
              it.by !== me ? verdict(it.ref, () => draw()) : h("p", { className: "s", textContent: "Yours: another editor approves it." }),
            ),
          )
        : [h("p", { className: "none", textContent: "Nothing waits for review." })]),
    );
  }

  // YOUR DRAFTS (every place's): scheduled, in review, declined — edit, publish now, delete.
  async function draftsPage() {
    root.replaceChildren(h("h2", { textContent: "📝 Drafts" }), theme.loading("Reading your drafts…"));
    const list = await press.drafts();
    const spaces = new Map((await space.mine()).map(s => [s.id, s]));
    const line = d => {
      const b = blogOf(d);
      const said = h("span", { className: "said" });
      const publish = h("button", { type: "button", className: "cw-btn primary", textContent: b.review ? "Send again" : "Publish now" });
      const go = theme.action(publish, () => press.publish(d).then(x => (x.review ? draw() : (location.hash = `${b.place ? `#/s/${b.place}/blog` : "#/blog"}/p/${x.ref}`)), e => ((said.textContent = ctx.say(e)), Promise.reject(e))), { busy: "Publishing…", done: "Done ✓" });
      publish.onclick = () => ctx.background("blog: go", go());
      const del = h("button", { type: "button", className: "cw-btn", textContent: "Delete" });
      del.onclick = () => (del.dataset.armed === "1" ? items.remove(d.ref).then(() => draw(), e => (said.textContent = ctx.say(e))) : ((del.dataset.armed = "1"), (del.textContent = "Confirm: delete")));
      const where = b.place ? `for ${spaces.get(b.place) ? space.shown(spaces.get(b.place)) : "a space you left"}` : "for your blog";
      const state = b.review ? (b.declined ? "❌ declined — change it and send again" : `in review since ${ago(b.review.at)}`) : Number.isFinite(b.due) ? `⏰ publishes ${new Date(b.due).toLocaleString()}${b.due <= Date.now() ? " (due: the next page open publishes it)" : ""}` : "not scheduled";
      return h("div", { className: "row", style: "grid-template-columns:minmax(0,1fr)" }, h("h3", {}, h("a", { href: `#/blog/write/${d.ref}`, textContent: d.title || "Untitled draft" })), h("div", { className: "s", textContent: `${where} · ${state} · saved ${ago(d.edited || d.at)}` }), h("div", { className: "tools" }, h("a", { className: "cw-btn", href: `#/blog/write/${d.ref}`, textContent: "Edit" }), publish, del, said));
    };
    root.replaceChildren(
      h("h2", { textContent: "📝 Drafts" }),
      h("p", { className: "s", textContent: "Only you read these (your account's private posts). A scheduled one publishes when it is due while any Craftworks page of yours is open; with none open then, it publishes the next time one opens." }),
      h("div", { className: "list" }, ...(list.length ? list.map(line) : [h("p", { className: "none", textContent: "No drafts." })])),
    );
  }

  // WRITE: a new draft (here: your blog, or the space open), an existing draft (`write/<ref>`), or a published article
  // changed (`edit/<ref>`).
  async function writePage({ draftRef = null, editRef = null } = {}) {
    root.replaceChildren(theme.loading("Opening the editor…"));
    const it = draftRef || editRef ? await items.get(draftRef ?? editRef) : null;
    if ((draftRef || editRef) && (!it || it.by !== me)) return root.replaceChildren(h("p", { className: "none", textContent: "Not yours to change, or not there." }));
    const b = blogOf(it);
    const placeId = editRef ? (it.board?.id ?? null) : draftRef ? (b.place ?? null) : (at.space?.id ?? null);
    const sp = placeId ? await items.boardOf(placeId) : null;
    if (placeId && !sp) return root.replaceChildren(h("p", { className: "none", textContent: "You are not in the space this is for." }));
    const [mdEditor, publisher, audience] = await Promise.all(["md-editor", "publisher", "audience"].map(n => ctx.require(n)));
    // Its FILES: kept as private as a draft is (its space's, or yours); made public with what is published public.
    const pick = attachments.picker({ space: sp, from: { app: "blog" }, media: true, publish: true, public: () => (editRef ? !!it.pub || (!it.board && !it.private) : false) });
    if (it) pick.preset(it.files ?? []);
    const ed = mdEditor.create({ value: it?.body ?? "", pick, label: "Article" });
    const title = h("input", { name: "title", placeholder: "Title", maxLength: 300, value: it && it.title !== "Untitled draft" ? (it.title ?? "") : "", autocomplete: "off" });
    const tg = publisher.tagFields(it ? items.tagsOf(it) : []);
    // ITS COVER: one of its images (none chosen: its first image not in its text).
    const coverSel = h("select", { name: "cover" });
    const covers = () => {
      const now = coverSel.value || it?.meta?.cover || "";
      const imgs = ed.files().filter(f => markdown.kindOf(f) === "image");
      coverSel.replaceChildren(h("option", { value: "", textContent: "Cover: its first image" }), ...imgs.map(f => h("option", { value: markdown.keyOf(f), textContent: `Cover: ${f.name ?? markdown.keyOf(f)}`, selected: markdown.keyOf(f) === now })));
    };
    covers();
    coverSel.onfocus = covers;
    pick.onChange?.(covers);
    // WHOM IT IS FOR (`audience`, the one choice), once published — a published article keeps its own.
    const who = editRef ? null : await audience.picker({ space: sp, kind: "post", initial: b.audience ?? (sp ? null : "public") });
    const when = h("input", { type: "datetime-local", name: "due" });
    if (Number.isFinite(b.due)) when.value = new Date(b.due - new Date(b.due).getTimezoneOffset() * 60e3).toISOString().slice(0, 16);
    const review = sp && !editRef ? await press.needsReview(sp) : false;
    const said = h("p", { className: "said" });
    const save = h("button", { type: "button", className: "cw-btn", textContent: "Save draft" });
    const schedule = h("button", { type: "button", className: "cw-btn", textContent: "Schedule" });
    const publish = h("button", { type: "button", className: "cw-btn primary", textContent: editRef ? "Save" : review ? "Send to the editors" : "Publish" });
    const fields = () => ({ title: title.value.trim(), body: ed.value().trim(), files: ed.files(), tags: tg.value(), cover: coverSel.value || null });
    const draftOf = async due => {
      return press.saveDraft({ ref: draftRef, ...fields(), place: sp?.id ?? null, audience: who?.value() ?? "public", write: who?.write() ?? null, level: who?.level() ?? "", due });
    };
    const guard = () => {
      if (ed.busy()) throw new Error("Still sending the files: a moment…");
      if (!title.value.trim()) throw new Error("An article needs a title.");
    };
    // Each button RUNS ONCE at a time (`theme.action`: locked and saying so), its error said under the form.
    const run = (btn, fn, words) => {
      const act = theme.action(btn, async () => {
        said.textContent = "";
        try {
          guard();
          await fn();
        } catch (e) {
          said.textContent = e?.message ?? String(e);
          throw e;
        }
      }, words);
      btn.onclick = () => ctx.background("blog: act", act());
    };
    if (editRef) {
      run(publish, async () => {
        const f = fields();
        await items.editItem(it.ref, f.body, { files: f.files, title: f.title, meta: { ...(it.meta ?? {}), tags: f.tags, ...(f.cover ? { cover: f.cover } : { cover: undefined }) } });
        location.hash = `${placeBase(it)}/p/${it.ref}`;
      }, { busy: "Saving…", done: "Saved ✓" });
    } else {
      run(save, async () => {
        const ref = await draftOf(null);
        if (!draftRef) location.hash = `#/blog/write/${ref}`;
      }, { busy: "Saving…", done: "Saved ✓" });
      run(schedule, async () => {
        const due = when.value ? new Date(when.value).getTime() : NaN;
        if (!Number.isFinite(due) || due <= Date.now()) throw new Error("Choose a time in the future to schedule it.");
        await draftOf(due);
        location.hash = "#/blog/drafts";
      }, { busy: "Scheduling…", done: "Scheduled ✓" });
      run(publish, async () => {
        const ref = await draftOf(null);
        const d = await items.get(ref);
        if (!d) throw new Error("the draft was saved but is not read back yet: publish it from Drafts");
        const x = await press.publish(d);
        location.hash = x.review ? "#/blog/drafts" : `${sp ? `#/s/${sp.id}/blog` : "#/blog"}/p/${x.ref}`;
      }, { busy: review ? "Sending…" : "Publishing…", done: "Done ✓" });
    }
    const where = sp ? `In ${space.shown(sp)}${review ? " — its editors approve before it is public" : ""}.` : "On your blog.";
    root.replaceChildren(
      h(
        "form",
        { className: "write", onsubmit: e => e.preventDefault() },
        h("h2", { textContent: editRef ? "✍️ Edit article" : draftRef ? "✍️ Draft" : "✍️ Write" }),
        h("p", { className: "s", textContent: where }),
        title,
        ed.el,
        h("div", { className: "line" }, coverSel),
        tg.el,
        who?.el ?? null,
        editRef ? null : h("label", { className: "s" }, "Publish at ", when, " (Schedule: published by your page when due — or, with none open then, the next time one opens)"),
        h("div", { className: "line" }, editRef ? null : save, editRef ? null : schedule, publish, h("a", { className: "cw-btn", href: editRef ? `${placeBase(it)}/p/${it.ref}` : draftRef ? "#/blog/drafts" : base(), textContent: "Cancel" })),
        said,
      ),
    );
    ed.focus?.();
  }

  let drawn = null;
  async function draw() {
    at = await whereCap.of({ kind: "post", app: "blog", yours: "Your blog" });
    drawn = whereCap.key();
    // ITS OWN SETTINGS (a space's, for who may manage its apps): drawn here, in the app.
    if (at.sub === "settings") return void (at.tabs([]), await at.settings(root));
    const s = at.sub;
    const mine = at.who === "mine";
    const editor = at.space ? await ctx.optional("blog: press.mayEdit", press.mayEdit(at.space), () => false) : false;
    at.tabs(
      [
        ...(at.who === "space" ? [{ label: "Articles", href: base(), on: !s }] : []),
        ...(at.who === "space" && editor ? [{ label: "Review", href: at.href("review"), on: s === "review" }] : []),
        ...(mine || at.who === "discover" || at.who === "space" ? [{ label: "Drafts", href: "#/blog/drafts", on: mine && s === "drafts" }] : []),
      ],
      { yoursOn: mine && (!s || s.startsWith("t/")), create: { label: "Write", href: at.space ? at.href("write") : "#/blog/write", on: s === "write" || s.startsWith("write/") } },
    );
    try {
      if (at.saved) {
        const list = (await at.read()).filter(press.isArticle);
        return root.replaceChildren(h("h2", { textContent: "★ Saved articles" }), h("div", { className: "list" }, ...(list.length ? list.map(it => row(it, { href: `${placeBase(it)}/p/${it.ref}` })) : [h("p", { className: "none", textContent: "Nothing saved: “Save” on an article keeps it here." })])));
      }
      if (s.startsWith("p/")) return await article(decodeURIComponent(s.slice(2)));
      if (s.startsWith("t/")) return await home(decodeURIComponent(s.slice(2)));
      if (s === "drafts" && mine) return await draftsPage();
      if (s === "review" && at.space) return await reviewPage();
      if (s === "write" && !at.others) return await writePage();
      if (s.startsWith("write/") && mine) return await writePage({ draftRef: decodeURIComponent(s.slice(6)) });
      if (s.startsWith("edit/") && !at.others) return await writePage({ editRef: decodeURIComponent(s.slice(5)) });
      return await home();
    } catch (e) {
      fail(root, e);
    }
  }
  await draw();
  // Drawn again when what it reads changes (your private posts arrive after the public ones) — never under someone writing.
  items.onChange(() => el.isConnected && !/^(write|edit)/.test(at?.sub ?? "") && whereCap.key() === drawn && !/^p\//.test(at?.sub ?? "") && draw());
  addEventListener("craftworks:route", () => {
    if (!el.isConnected || ctx.route !== "/blog" || whereCap.key() === drawn) return;
    months = 1;
    draw();
  });
}
