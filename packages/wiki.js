// WIKI, an app (`#/wiki`): pages by TITLE, written in Markdown, linked by [[Title]] (a link to a page not written yet
// shows as one to CREATE), each with what links to it, an index as a tree ("Recipes/Bread" under "Recipes"), found by
// the header's search (titles, as every item) and the index's own filter (titles and text). A LENS on `wiki-pages`
// (pages as sections over `items`): no store, no mechanism of its own.
// - YOUR wiki (`#/wiki`): only you edit; who sees each page is the one choice every app has (`audience`).
// - A SPACE's wiki (`#/s/<space>/wiki`, an app the space adds): its pages are the space's; whoever its `edit` policy
//   on the wiki allows edits, everyone else SUGGESTS (an editor accepts or declines). Two editors of one section at
//   once: both versions kept and shown as a CONFLICT to settle.
// - A PERSON's (`#/wiki/u/<did>`): read, copy, suggest.
// Every page: its HISTORY (who wrote which version of which section, when; restore), COPY TO MY WIKI (with its
// lineage), and on a copy, PROPOSE back to where it came from. Discover (the global wiki, an ownerless commons) is
// not built: said where it would be.
export async function mount(ctx, el) {
  const [items, theme, markdown, mdEditor, directory, W, where, spaceCap] = await Promise.all(["items", "theme", "markdown", "md-editor", "directory", "wiki-pages", "where", "space"].map(n => ctx.require(n)));
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  el.innerHTML = `
    <style>@layer apps {
      .wk { display: grid; gap: var(--cw-space-3); padding: var(--cw-space-3) var(--cw-gutter); min-width: 0; }
      .wk h2, .wk h1 { margin: 0; }
      .wk .tools { display: flex; flex-wrap: wrap; gap: var(--cw-space-2); align-items: center; }
      .wk .tools .end { margin-left: auto; }
      .wk .meta { color: var(--cw-muted); font-size: var(--cw-text-sm); display: flex; flex-wrap: wrap; gap: 6px; }
      .wk .said { color: var(--cw-danger); margin: 0; }
      .wk .said.ok { color: var(--cw-muted); }
      .wk .none { color: var(--cw-muted); }
      .wk .find { width: 100%; box-sizing: border-box; }
      .wk ul.tree { list-style: none; margin: 0; padding-left: 0; display: grid; gap: 2px; }
      .wk ul.tree ul.tree { padding-left: 1.2em; border-left: 1px solid var(--cw-line); margin-left: .4em; }
      .wk ul.tree .row { display: flex; gap: 8px; align-items: baseline; flex-wrap: wrap; }
      .wk ul.tree .group { color: var(--cw-muted); }
      .wk .badge { font-size: var(--cw-text-xs); color: var(--cw-muted); }
      .wk .badge.warn { color: var(--cw-danger); }
      .wk .hit { display: grid; gap: 2px; padding: var(--cw-space-2) 0; border-bottom: 1px solid var(--cw-line); }
      .wk .hit p { margin: 0; color: var(--cw-muted); font-size: var(--cw-text-sm); }
      .wk .sec { position: relative; padding: 2px 0; }
      .wk .sec > .edit { position: absolute; top: 0; right: 0; border: 0; background: none; cursor: pointer; color: var(--cw-muted); padding: 2px 6px; border-radius: var(--cw-radius-sm); }
      .wk .sec > .edit:hover, .wk .sec > .edit:focus-visible { background: var(--cw-hover); color: var(--cw-fg); }
      .wk .sec > .text { padding-right: 2em; min-width: 0; }
      .wk a.wk-missing { color: var(--cw-danger); text-decoration: underline dotted; }
      .wk .box { border: 1px solid var(--cw-line); border-radius: var(--cw-radius); padding: var(--cw-space-2) var(--cw-space-3); display: grid; gap: var(--cw-space-2); background: var(--cw-surface); }
      .wk .box.conflict { border-color: var(--cw-danger); }
      .wk .box h3, .wk .box h4 { margin: 0; font-size: 1rem; }
      .wk .heads { display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: var(--cw-space-2); }
      .wk .head { border: 1px solid var(--cw-line); border-radius: var(--cw-radius-sm); padding: var(--cw-space-2); display: grid; gap: 6px; align-content: start; min-width: 0; }
      .wk .editing { display: grid; gap: var(--cw-space-2); }
      .wk .editing input.title { font: inherit; font-size: 1.2rem; }
      .wk .links { display: flex; flex-wrap: wrap; gap: 6px 12px; }
      .wk details > summary { cursor: pointer; }
      dialog.wk-history { border: 0; border-radius: var(--cw-radius); padding: var(--cw-space-3) var(--cw-space-4); width: min(640px, calc(100vw - 32px)); max-height: calc(100vh - 32px);
        background: var(--cw-surface); color: var(--cw-fg); box-shadow: var(--cw-shadow); display: grid; grid-template-rows: auto minmax(0, 1fr) auto; gap: var(--cw-space-2); }
      dialog.wk-history:not([open]) { display: none; }
      dialog.wk-history h3 { margin: 0; font-size: 1rem; }
      dialog.wk-history .list { overflow-y: auto; display: grid; gap: var(--cw-space-2); }
      dialog.wk-history .v { border: 1px solid var(--cw-line); border-radius: var(--cw-radius); padding: var(--cw-space-2) var(--cw-space-3); display: grid; gap: 4px; }
      dialog.wk-history .v .who { font-size: var(--cw-text-xs); color: var(--cw-muted); display: flex; flex-wrap: wrap; justify-content: space-between; gap: var(--cw-space-2); align-items: center; }
      dialog.wk-history .v .b { white-space: pre-wrap; overflow-wrap: anywhere; max-height: 9em; overflow: hidden; font-size: var(--cw-text-sm); }
    }</style>
    <div class="wk"></div>`;
  const outer = el.querySelector(".wk");
  // Where a page draws: the view's box being built (`ctx.view` render), or the page itself for the maker.
  let root = outer;
  const said = (text, ok = false) => h("p", { className: ok ? "said ok" : "said", textContent: text });
  const when = t => (t ? new Date(t).toLocaleString() : "");
  const errText = e => e?.message ?? String(e);
  let editing = false; // an editor open: what changes underneath does not redraw it away
  let drawing = 0;

  async function draw() {
    const n = ++drawing;
    let at;
    try {
      at = await where.of({ kind: "wiki", app: "wiki", yours: "Your wiki" });
    } catch (e) {
      return outer.replaceChildren(said(errText(e)));
    }
    const sub = at.sub;
    // A CIRCLE of yours opened as a space (an item's link): your own wiki.
    if (at.space?.circle && at.space.governance?.owner === at.me) return location.replace(`#/wiki${sub ? `/${sub}` : ""}`);
    at.tabs([], { yoursOn: !sub, create: at.discover || at.saved ? null : { label: "New page", href: at.href("new"), on: sub === "new" || sub.startsWith("new/") } });
    // DISCOVER: every public wiki page (people's, public spaces'), each opened in its own wiki. The global wiki — one
    // for everyone, owned by nobody — comes after Vote.
    view?.stop();
    view = null;
    // A NEW PAGE (the maker): drawn once — never again under its writer.
    if (sub === "new" || sub.startsWith("new/")) {
      root = outer;
      outer.replaceChildren(theme.loading("Opening the wiki…"));
      const wiki = await W.of(at).catch(e => (outer.replaceChildren(said(errText(e))), null));
      if (wiki && n === drawing) await create({ at, wiki, n }, decodeURIComponent(sub.slice(4))).catch(e => outer.replaceChildren(said(errText(e))));
      return;
    }
    // EVERY OTHER PAGE: a VIEW (`ctx.view`, render form) — built again when pages change (another editor, another
    // device), put in place only when it differs; never while a section's editor is open (`hold`).
    view = ctx.view(outer, {
      render: async () => {
        const box = h("div", { style: "display:contents" });
        root = box;
        // DISCOVER: every public wiki page (people's, public spaces'), each opened in its own wiki.
        if (at.discover) await pages(at, "📖 Public wiki pages", "No public wiki pages yet.");
        else if (at.saved) await pages(at, "📖 Saved pages", "Nothing saved: ☆ on a page keeps it here.");
        else {
          const wiki = await W.of(at);
          const v = { at, wiki, n };
          if (sub.startsWith("t/")) await show(v, decodeURIComponent(sub.slice(2)));
          else if (sub.startsWith("p/")) await show(v, decodeURIComponent(sub.slice(2)));
          else await index(v);
        }
        return box;
      },
      hold: () => editing,
      looks: { loading: () => theme.loading("Opening the wiki…") },
    });
  }
  let view = null;
  // Where a title is read in this wiki.
  const titleHref = (at, title) => at.href(`t/${encodeURIComponent(W.cleanTitle(title))}`);
  // Whose wiki this is, for its heading.
  const placeName = (at, wiki) => (wiki.space ? `${spaceCap.shown(wiki.space)}'s wiki` : at.who === "person" ? h("span", {}, directory.nameEl(at.person), "'s wiki") : "Your wiki");

  // A page's TEXT as it reads: Markdown, its [[links]] made links within this wiki — one not written yet in red, to
  // create it.
  function rendered(at, text, keys) {
    const md = markdown.render(W.linkify(text, t => titleHref(at, t)), []);
    const base = `${at.href("t/")}`;
    for (const a of md.querySelectorAll("a[href]")) {
      const href = a.getAttribute("href");
      if (!href.startsWith(base)) continue;
      // Within the app: opened here, never in a new tab (`markdown` opens other links apart).
      a.removeAttribute("target");
      a.removeAttribute("rel");
      const key = W.normTitle(decodeURIComponent(href.slice(base.length)));
      if (keys.has(key)) continue;
      a.classList.add("wk-missing");
      a.title = "Not written yet";
    }
    return md;
  }

  // ── THE INDEX ──
  async function index({ at, wiki, n }) {
    const pages = await wiki.pages();
    if (n !== drawing) return;
    const find = h("input", { type: "search", className: "find", placeholder: "Find a page (its title or text)", ariaLabel: "Find a page" });
    const list = h("div", {});
    const badges = p => [
      p.conflicts ? h("span", { className: "badge warn", textContent: `⚠ ${p.conflicts} conflict${p.conflicts > 1 ? "s" : ""}` }) : null,
      p.pending.length ? h("span", { className: "badge", textContent: `✉ ${p.pending.length} suggested` }) : null,
      p.private ? h("span", { className: "badge", textContent: "🔒" }) : p.pub ? h("span", { className: "badge", textContent: "🌐" }) : null,
      h("span", { className: "badge", textContent: when(p.updated) }),
    ];
    const branch = nodes =>
      h(
        "ul",
        { className: "tree" },
        ...nodes.map(nd =>
          h(
            "li",
            {},
            h("div", { className: "row" }, nd.page ? h("a", { href: titleHref(at, nd.page.title), textContent: nd.name }) : h("span", { className: "group", textContent: nd.name }), ...(nd.page ? badges(nd.page) : [])),
            nd.children.length ? branch(nd.children) : null,
          ),
        ),
      );
    const fill = () => {
      const q = find.value.trim().toLowerCase();
      if (!pages.length) return list.replaceChildren(h("p", { className: "none", textContent: wiki.may.edit ? "No pages yet: ＋ New page above, or link one as [[Its title]] and follow the link." : "No pages yet." }));
      if (!q) return list.replaceChildren(branch(W.tree(pages)));
      const hits = pages.filter(p => p.title.toLowerCase().includes(q) || p.text.toLowerCase().includes(q));
      list.replaceChildren(
        ...(hits.length
          ? hits.map(p => {
              const i = p.text.toLowerCase().indexOf(q);
              return h("div", { className: "hit" }, h("a", { href: titleHref(at, p.title), textContent: p.title }), i >= 0 ? h("p", { textContent: `…${markdown.plain(p.text.slice(Math.max(0, i - 60), i + 100))}…` }) : null);
            })
          : [h("p", { className: "none", textContent: `No page has “${find.value.trim()}”.` })]),
      );
    };
    find.oninput = fill;
    fill();
    root.replaceChildren(h("h2", {}, "📖 ", placeName(at, wiki)), find, list);
  }

  // PAGES LISTED from many wikis (your Saved; Discover's public ones), each a link into its own wiki.
  async function pages(at, title, none) {
    const list = (await at.read()).filter(p => p.kind === "wiki");
    root.replaceChildren(
      h("h2", { textContent: title }),
      list.length ? h("ul", {}, ...list.map(p => h("li", {}, h("a", { href: items.pageOf(p.ref, p.kind), textContent: p.title || "Untitled" }), at.discover ? h("span", { className: "s" }, " · ", directory.nameEl(p.by)) : null))) : h("p", { className: "none", textContent: none }),
    );
  }

  // An EDITOR (the one `md-editor`) with Save and Cancel: `save(text)` writes; the page drawn again after.
  function editorBox({ value = "", placeholder = "Write in Markdown; link pages as [[Their title]]", label, saveLabel = "Save", save, cancel, extra = [] }) {
    editing = true;
    const ed = mdEditor.create({ value, placeholder, label });
    const msg = h("span", {});
    const btn = h("button", { type: "button", className: "cw-btn primary", textContent: saveLabel });
    const run = theme.action(btn, async () => {
      msg.replaceChildren();
      try {
        await save(ed.value());
        editing = false;
      } catch (e) {
        msg.replaceChildren(said(errText(e)));
        throw e;
      }
    }, { busy: "Saving…", done: "Saved ✓" });
    btn.onclick = () => ctx.background("wiki: run", run());
    const no = h("button", { type: "button", className: "cw-btn", textContent: "Cancel", onclick: () => ((editing = false), cancel()) });
    return h("div", { className: "editing box" }, ...extra, ed.el, h("div", { className: "tools" }, btn, no, msg));
  }

  // ── A NEW PAGE ──
  async function create({ at, wiki }, title) {
    if (!wiki.may.edit) return root.replaceChildren(h("a", { href: at.href(""), textContent: "← Index" }), said(wiki.owner ? "Only its owner writes pages in this wiki." : "This space's rules do not let you write pages in its wiki."));
    const t = h("input", { type: "text", className: "title", placeholder: "Title", maxLength: 200, value: W.cleanTitle(title) });
    // Who sees it: yours, the one choice (a space's: as its wiki reads).
    const who = wiki.scope === "personal" ? await (await ctx.require("audience")).picker({ space: null, kind: "wiki", initial: "private" }) : null;
    root.replaceChildren(
      h("div", { className: "tools" }, h("a", { href: at.href(""), textContent: "← Index" })),
      h("h2", { textContent: "📖 New page" }),
      editorBox({
        label: "Page",
        saveLabel: "Create",
        extra: [t, who?.el ?? null],
        cancel: () => history.back(),
        save: async text => {
          const ref = await wiki.create(t.value, text, { audience: who?.value() ?? null, write: who?.write() ?? null });
          location.hash = titleHref(at, t.value);
          return ref;
        },
      }),
    );
    if (!t.value) t.focus();
  }

  // ── A PAGE ──
  async function show(view, keyOrRef) {
    const { at, wiki, n } = view;
    const p = await wiki.page(keyOrRef);
    if (n !== drawing) return;
    const all = await wiki.pages({ fresh: false });
    const keys = new Set(all.map(x => x.key));
    const back = h("a", { href: at.href(""), textContent: "← Index" });
    if (!p) {
      const title = /^(space:|did:)/.test(keyOrRef) ? "" : W.cleanTitle(keyOrRef);
      // (Optional parts filtered: `replaceChildren` makes a null child the TEXT "null".)
      return root.replaceChildren(
        ...[
          h("div", { className: "tools" }, back),
          h("h1", { textContent: title || "Not here" }),
          h("p", { className: "none", textContent: title ? "There is no page with this title in this wiki yet." : "This page is not here, or not yours to read." }),
          title && wiki.may.edit ? h("a", { className: "cw-btn primary", href: at.href(`new/${encodeURIComponent(title)}`), textContent: "Create it" }) : null,
        ].filter(Boolean),
      );
    }
    const me = at.me;
    const editor = wiki.may.edit;
    const writer = wiki.may.write;
    const msg = h("div", {});
    const redraw = () => show(view, p.ref).catch(e => msg.replaceChildren(said(errText(e))));
    const say = (t, ok) => msg.replaceChildren(said(t, ok));
    const sectionsEl = h("div", {});
    // ITS SECTIONS: each as it reads, ✎ to edit it (an editor) or suggest a change (anyone else who may write).
    const drawSections = () => {
      const live = p.sections.filter(s => !s.gone || s.conflict);
      if (!live.length) return sectionsEl.replaceChildren(h("p", { className: "none", textContent: "This page is empty." }));
      sectionsEl.replaceChildren(...live.map(s => (s.conflict ? conflictBox(s) : sectionEl(s))));
    };
    const sectionEl = s => {
      const box = h("div", { className: "sec" }, h("div", { className: "text" }, rendered(at, s.body, keys)));
      if (writer)
        box.prepend(
          h("button", {
            type: "button",
            className: "edit",
            title: editor ? "Edit this section" : "Suggest a change to this section",
            textContent: "✎",
            onclick: () =>
              box.replaceWith(
                editorBox({
                  value: s.body,
                  label: "Section",
                  saveLabel: editor ? "Save" : "Suggest",
                  cancel: drawSections,
                  save: async text => {
                    await wiki.apply(p, W.planSection(p.sections, s.sid, text));
                    await redraw();
                  },
                }),
              ),
          }),
        );
      return box;
    };
    // A CONFLICT: every version kept, side by side — an editor keeps one, or merges them by hand.
    const conflictBox = s => {
      const box = h("div", { className: "box conflict" });
      const settle = (base, text) => wiki.apply(p, W.planSection(p.sections, s.sid, text, { base: base.h, merges: s.heads.map(x => x.h) })).then(redraw);
      box.append(
        h("h4", { textContent: `⚠ This section was changed at once by ${s.heads.length} people: every version is kept until an editor settles it.` }),
        h(
          "div",
          { className: "heads" },
          ...s.heads.map(v =>
            h(
              "div",
              { className: "head" },
              h("div", { className: "meta" }, directory.nameEl(v.by), h("span", { textContent: `· ${when(v.at)}` })),
              v.gone ? h("p", { className: "none", textContent: "(deleted this section)" }) : rendered(at, v.body, keys),
              editor ? h("button", { type: "button", className: "cw-btn", textContent: v.gone ? "Delete it" : "Keep this one", onclick: e => ((e.currentTarget.disabled = true), settle(v, v.gone ? "" : v.body).catch(err => say(errText(err)))) }) : null,
            ),
          ),
        ),
        editor
          ? h("button", {
              type: "button",
              className: "cw-btn",
              textContent: "Merge them by hand…",
              onclick: () =>
                box.replaceWith(
                  editorBox({
                    value: s.heads.filter(v => !v.gone).map(v => v.body.trim()).join("\n\n"),
                    label: "Merged section",
                    cancel: drawSections,
                    save: text => settle(s.heads.at(-1), text),
                  }),
                ),
            })
          : null,
      );
      return box;
    };
    drawSections();

    // THE WHOLE PAGE edited (or a suggestion made of it) — not while a section is in conflict (settled first).
    const editPage = h("button", {
      type: "button",
      className: "cw-btn",
      textContent: editor ? "✎ Edit page" : "✎ Suggest an edit",
      hidden: !writer,
      onclick: () => {
        if (p.conflicts) return say("Settle the conflicts below first: the whole page is edited once every section has one version.");
        sectionsEl.replaceChildren(
          editorBox({
            value: W.joinSections(p.sections.filter(s => !s.gone)),
            label: "Page",
            saveLabel: editor ? "Save" : "Suggest",
            cancel: drawSections,
            save: async text => {
              const ops = W.planWhole(p.sections, text);
              if (ops.length) await wiki.apply(p, ops);
              await redraw();
            },
          }),
        );
      },
    });
    const historyBtn = h("button", { type: "button", className: "cw-btn", textContent: "🕘 History", onclick: () => openHistory(view, p, editor, redraw) });
    // COPY to your wiki: a page that is not yours (a space's, a person's) — with its lineage.
    const mineAlready = wiki.scope === "personal" && wiki.owner === me;
    const copy = mineAlready
      ? null
      : h("button", {
          type: "button",
          className: "cw-btn",
          textContent: "📥 Copy to my wiki",
          onclick: async e => {
            e.currentTarget.disabled = true;
            try {
              await W.copyToMine(p);
              location.hash = `#/wiki/t/${encodeURIComponent(p.title)}`;
            } catch (err) {
              say(`Not copied: ${errText(err)}`);
              e.currentTarget.disabled = false;
            }
          },
        });
    // PROPOSE: your copy's changes back to where it came from.
    const lin = p.meta?.lineage;
    const propose =
      mineAlready && lin?.from
        ? h("button", {
            type: "button",
            className: "cw-btn",
            textContent: "📤 Propose to its origin",
            title: "Your changes, as edits where you may edit there, else as suggestions",
            onclick: async e => {
              e.currentTarget.disabled = true;
              try {
                const r = await W.propose(p);
                say(r.written ? `${r.written} change${r.written > 1 ? "s" : ""} ${r.suggest ? "suggested — its editors accept or decline them" : "written there"}.` : "Nothing to propose: it reads the same there.", true);
              } catch (err) {
                say(`Not proposed: ${errText(err)}`);
              }
              e.currentTarget.disabled = false;
            },
          })
        : null;
    const save = p.ref ? (await ctx.require("actions")).save({ ...p, kind: "wiki" }) : null;
    // WHERE IT CAME FROM (a copy).
    const lineage = lin?.from
      ? h("span", {}, "· copied from ", h("a", { href: items.pageOf(lin.from, "wiki"), textContent: lin.title || "its original" }), lin.space ? "" : h("span", {}, " by ", directory.nameEl(lin.by)), ` · ${when(lin.at)}`)
      : null;
    // SUGGESTIONS pending: each to accept or decline (an editor); everyone sees them, and whose they are.
    const pendingEl = p.pending.length
      ? h(
          "details",
          { className: "box", open: editor },
          h("summary", { textContent: `✉ ${p.pending.length} suggestion${p.pending.length > 1 ? "s" : ""}` }),
          ...p.pending.map(s => {
            const cur = p.sections.find(x => x.sid === s.sid);
            const what = s.gone ? "deleting a section" : cur ? `a change to “${cur.heading ?? "the lead"}”` : "a new section";
            return h(
              "div",
              { className: "head" },
              h("div", { className: "meta" }, directory.nameEl(s.by), h("span", { textContent: `· ${what} · ${when(s.at)}${s.fresh ? "" : " · on an older version"}` })),
              s.gone ? null : rendered(at, s.body, keys),
              editor
                ? h(
                    "div",
                    { className: "tools" },
                    h("button", { type: "button", className: "cw-btn primary", textContent: "Accept", onclick: e => ((e.currentTarget.disabled = true), wiki.accept(p, s).then(redraw, err => say(errText(err)))) }),
                    h("button", { type: "button", className: "cw-btn", textContent: "Decline", onclick: e => ((e.currentTarget.disabled = true), wiki.decline(p, s).then(redraw, err => say(errText(err)))) }),
                  )
                : null,
            );
          }),
        )
      : null;
    // WHAT LINKS HERE.
    const links = W.backlinks(all, p.key);
    // (Optional parts filtered: `replaceChildren` makes a null child the TEXT "null" — measured on fx: "nullnull".)
    root.replaceChildren(
      ...[
      h("div", { className: "tools" }, back, editPage, historyBtn, copy, propose, h("span", { className: "end" }, save)),
      h("h1", { textContent: p.title }),
      h(
        "div",
        { className: "meta" },
        h("span", {}, "📖 ", placeName(at, wiki)),
        p.private ? h("span", { textContent: "· 🔒 only you" }) : p.pub ? h("span", { textContent: "· 🌐 public" }) : null,
        h("span", { textContent: `· updated ${when(p.updated)}` }),
        lineage,
        editor ? null : h("span", { textContent: writer ? "· you suggest changes here" : "· read only" }),
      ),
      p.conflicts ? said(`⚠ ${p.conflicts} section${p.conflicts > 1 ? "s are" : " is"} in conflict below: nothing is lost — settle each.`) : null,
      msg,
      pendingEl,
      sectionsEl,
      h("details", { className: "box" }, h("summary", { textContent: `What links here (${links.length})` }), links.length ? h("div", { className: "links" }, ...links.map(x => h("a", { href: titleHref(at, x.title), textContent: x.title }))) : h("p", { className: "none", textContent: "No page links here yet." })),
      ].filter(Boolean),
    );
  }

  // ── HISTORY: every version of every section, who and when; RESTORE one (an editor; else a suggestion of it) ──
  async function openHistory({ wiki }, p, editor, redraw) {
    const list = h("div", { className: "list" }, theme.loading("Reading its history…"));
    const close = h("button", { type: "button", textContent: "Close" });
    const note = h("p", { className: "none" });
    const d = h("dialog", { className: "wk-history" }, h("h3", { textContent: `History of “${p.title}”` }), list, h("div", { className: "tools" }, note, h("span", { className: "end" }, close)));
    close.onclick = () => d.close();
    d.addEventListener("close", () => d.remove());
    root.append(d);
    d.showModal();
    let vs = [];
    try {
      vs = await wiki.history(p);
    } catch (e) {
      note.textContent = `Its history did not read: ${errText(e)}`;
    }
    const current = new Set(p.sections.flatMap(s => s.heads.map(x => x.h)));
    if (!vs.length) return list.replaceChildren(h("p", { className: "none", textContent: "No versions read." }));
    list.replaceChildren(
      ...vs.map(v => {
        const kind = v.gone ? "deleted a section" : !v.editor ? "suggested" : "wrote";
        const restore = editor && !current.has(v.h)
          ? h("button", {
              type: "button",
              className: "cw-btn",
              textContent: "Restore",
              onclick: async e => {
                e.currentTarget.disabled = true;
                note.textContent = "Restoring…";
                try {
                  await wiki.restore(p, v);
                  d.close();
                  redraw();
                } catch (err) {
                  note.textContent = `Not restored: ${errText(err)}`;
                }
              },
            })
          : current.has(v.h)
            ? h("span", { textContent: "current" })
            : null;
        return h(
          "div",
          { className: "v" },
          h("div", { className: "who" }, h("span", {}, directory.nameEl(v.by), ` ${kind} “${v.heading ?? "the lead"}” · ${when(v.at)}`), restore),
          v.gone ? null : h("div", { className: "b", textContent: v.body }),
        );
      }),
    );
  }

  await draw();
  // A route within the app (another page, the index, another place): drawn again.
  addEventListener("craftworks:route", () => el.isConnected && ctx.route === "/wiki" && ((editing = false), draw()));
}
