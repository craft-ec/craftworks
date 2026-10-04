// BOARD, a page: posts, Reddit-style, CONFINED to the place open (the spaces panel). In a SHARED space
// (`#/s/<space>/board`): its own posts only — its members post, its roles and moderation apply. In the PERSONAL space
// (`#/board`): your own posts, your profile (public: what your followers read); another person's profile is
// `#/board/u/<did>`; and the FEED (`#/board/feed`: the personal view gathers — the boards of every space you are in,
// and the profiles you follow). A POST with its comment tree: `…/board/p/<ref>`; CREATE: `…/board/submit`. As Grid's:
// a FEED — New (by date, a month at a time, older as the end is scrolled to), Hot, Best, Rising or Top (votes or
// comments) over TODAY, THIS WEEK or THIS MONTH, their counts over the same window — and a SORT reordering what is
// shown. What is read is that window of the board, never all of it.
// UI only: posts are `posts`', names `directory`'s, and a name opens the one `person` menu (follow; in a space, its
// role and removal). The space itself (its members, settings, invites) is its Home's.
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  const [posts, directory, person, theme, space, roles, appSettings, attachments, markdown, mdEditor, cards] = await Promise.all(["items", "directory", "person", "theme", "space", "roles", "app-settings", "attachments", "markdown", "md-editor", "cards"].map(n => ctx.require(n)));
  const me = (await space.account()).id;
  // The FEED BAR (shared by every content app, as Grid's): the feed, its window, and a sort of what is shown.
  const bar = (await ctx.require("feed-bar")).create({ start: "hot", onChange: () => draw() });
  // WHERE Board is (`where`: the one reading of an address — yours, a space's, a person's, Discover), read at each draw.
  const whereCap = await ctx.require("where");
  let at = await whereCap.of({ kind: "post", app: "board", yours: "Your posts" });
  const discovering = () => at.who === "discover";
  const base = () => at.base;
  // The route: what is shown.
  const route = () => {
    const s = at.sub;
    // DISCOVER (the public view): every public board, one space's (`b/<id>`), or a public post (`p/<ref>`).
    if (discovering()) {
      // A public board's composer, for one not in it whose policy lets anyone post.
      const sub = /^b\/([0-9a-f]{64})\/submit$/.exec(s);
      if (sub) return { pub: sub[1], board: sub[1], submit: true };
      if (s.startsWith("b/")) return { pub: s.slice(2) };
      if (s.startsWith("p/")) return { post: s.slice(2), pub: s.slice(2).match(/^space:([0-9a-f]{64})\//)?.[1] };
      return { discover: true };
    }
    const inSpace = at.space ? { board: at.space.id } : {};
    if (s.startsWith("p/")) return { ...inSpace, post: s.slice(2) };
    if (s === "submit" && !at.others) return { ...inSpace, submit: true };
    if (at.who === "mine" && s === "feed") return { feed: true };
    if (at.saved) return { saved: true };
    if (at.who === "person") return { by: at.person };
    return at.space ? inSpace : { by: me };
  };
  // The top bar: its sub-pages — the posts, and Create post.
  const setActions = async w => {
    // THE TABS (`where`'s, in every app's order): Your posts · Feed · Create post · Discover — a space's: Posts and
    // Create post; a person's: none. Each lit only on its own page.
    const page = !w.post && !w.submit && !w.feed && !w.saved;
    at.tabs(
      [
        ...(at.who === "space" ? [{ label: "Posts", href: base(), on: page }] : []),
        ...(at.who === "mine" || at.who === "discover" ? [{ label: "Feed", href: "#/board/feed", on: !!w.feed }] : []),
        ...(at.others ? [] : [{ label: "Create post", href: `${base()}/submit`, on: !!w.submit }]),
      ],
      { yoursOn: page && w.by === me },
    );
  };
  el.innerHTML = `
    <style>
      .bd { max-width: 1080px; margin: 0 auto; display: grid; grid-template-columns: minmax(0, 1fr) 300px; gap: var(--cw-space-4); align-items: start; }
      @media (max-width: 900px) { .bd { grid-template-columns: minmax(0, 1fr); } .bd .side { order: -1; } }
      .bd button { font: inherit; cursor: pointer; }
      .bd a { color: inherit; text-decoration: none; }
      .bd .main { display: grid; gap: var(--cw-space-2); min-width: 0; }
      .bd .panel { background: var(--cw-surface); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); padding: var(--cw-space-3); display: grid; gap: var(--cw-space-2); }
      .bd .side { display: grid; gap: var(--cw-space-3); }
      .bd .side h3 { margin: 0; font-size: 1rem; }
      .bd .side p { margin: 0; color: var(--cw-muted); font-size: var(--cw-text-sm); }
      .bd .side ul { list-style: none; margin: 0; padding: 0; display: grid; gap: 2px; }
      .bd .side li a { display: block; padding: 4px var(--cw-space-2); border-radius: var(--cw-radius-sm); }
      .bd .side li a:hover { background: var(--cw-hover); }
      .bd .side form { display: flex; gap: var(--cw-space-2); }
      .bd .side input { flex: 1; min-width: 0; padding: 6px var(--cw-space-2); border-radius: var(--cw-radius-sm); }
      .bd .go { border: 0; border-radius: var(--cw-radius-pill); padding: 6px var(--cw-space-4); background: var(--cw-accent); color: var(--cw-accent-fg); font-weight: 600; text-align: center; }
      .bd .ghost { border: 1px solid var(--cw-accent); border-radius: var(--cw-radius-pill); padding: 6px var(--cw-space-4); background: none; color: var(--cw-accent); font-weight: 600; text-align: center; }
      .bd .banner { display: flex; align-items: center; gap: var(--cw-space-3); padding: var(--cw-space-3); }
      .bd .banner h2 { margin: 0; font-size: 1.4rem; flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .bd .sorts { display: flex; gap: var(--cw-space-2); padding: var(--cw-space-2); align-items: center; flex-wrap: wrap; }
      .bd .sorts .lbl { color: var(--cw-muted); font-size: var(--cw-text-sm); }
      .bd .sorts select { font: inherit; padding: 3px 6px; border-radius: var(--cw-radius-sm); border: 1px solid var(--cw-line); background: var(--cw-surface); color: var(--cw-fg); }
      .bd .sorts button { border: 0; background: none; color: var(--cw-muted); border-radius: var(--cw-radius-pill); padding: 4px var(--cw-space-3); font-weight: 600; }
      .bd .sorts button:hover { background: var(--cw-hover); }
      .bd .sorts button[aria-pressed="true"] { background: var(--cw-pressed); color: var(--cw-fg); }
      .bd .meta { display: flex; flex-wrap: wrap; gap: 4px; align-items: baseline; color: var(--cw-muted); font-size: var(--cw-text-xs); }
      .bd .meta .b { color: var(--cw-fg); font-weight: 700; }
      .bd .meta .b:hover, .bd .meta .by:hover { text-decoration: underline; cursor: pointer; }
      .bd .acts .votes { display: flex; align-items: center; gap: 2px; }
      .bd textarea, .bd .field { font: inherit; padding: var(--cw-space-2); border-radius: var(--cw-radius-sm); width: 100%; box-sizing: border-box; }
      .bd textarea { resize: vertical; min-height: 90px; }
      .bd form.reply { display: grid; gap: var(--cw-space-2); }
      .bd form.reply .row { display: flex; justify-content: flex-end; gap: var(--cw-space-2); }
      .bd .c { display: grid; grid-template-columns: 20px minmax(0, 1fr); column-gap: var(--cw-space-2); }
      .bd .c > .rail { display: flex; justify-content: center; cursor: pointer; }
      .bd .c > .rail::before { content: ""; width: 2px; background: var(--cw-line); border-radius: 1px; }
      .bd .c > .rail:hover::before { background: var(--cw-accent); }
      .bd .c > .body { display: grid; gap: 4px; min-width: 0; }
      .bd .c .text { overflow-wrap: anywhere; line-height: 1.5; margin: 0; }
      .bd .c .kids { display: grid; gap: var(--cw-space-2); margin-top: var(--cw-space-1); }
      .bd .c.folded .text, .bd .c.folded .acts, .bd .c.folded .kids, .bd .c.folded form { display: none; }
      .bd .c .fold { border: 0; background: none; color: var(--cw-muted); font-size: var(--cw-text-xs); padding: 0; }
      .bd .none { color: var(--cw-muted); text-align: center; padding: var(--cw-space-5); margin: 0; }
      .bd .said { color: var(--cw-danger); font-size: var(--cw-text-sm); margin: 0; }
      .bd label { display: grid; gap: 4px; font-size: var(--cw-text-sm); font-weight: 600; }
    </style>
    <div class="bd"><div class="main"></div><aside class="side"></aside></div>`;
  const main = el.querySelector(".main");
  const side = el.querySelector(".side");
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  const ago = at => {
    const s = Math.max(0, (Date.now() - at) / 1000);
    if (s < 60) return "just now";
    for (const [n, u] of [[31536000, "y"], [2592000, "mo"], [86400, "d"], [3600, "h"], [60, "m"]]) if (s >= n) return `${Math.floor(s / n)}${u} ago`;
  };
  let here = null; // the server of the board shown (a name clicked there offers its role and removal)
  const who = did => {
    const n = directory.nameEl(did, "span", { className: "by", onclick: e => (e.stopPropagation(), person.open(e.currentTarget, did, here ? { space: here } : {})) });
    return n;
  };
  const errorTo = said => e => ((said.textContent = e?.message ?? String(e)), (said.hidden = false));

  // AN ITEM's ACTIONS (`actions`: the same in every app — vote, comments, share, save, hide, flag, edit, remove — by
  // the one check). From OUTSIDE (a space's post this person is not in): its public description.
  const actionsCap = await ctx.require("actions");
  const outsideFor = it => () => (!here && it.board && !discovering() ? posts.boardOf(it.board.id).then(sp => (sp ? null : descOf(it.board.id))) : discovering() && it.board ? descOf(it.board.id) : null);

  // A POST: in a list (a link to its page, the text cut short) or on its own page.
  // WHAT SOMEONE WROTE, as Markdown (`markdown`): its images, videos and audio where they were written; the item's
  // other files below it, as attachments.
  // In a LIST: a plain preview and the files' thumbnails (a list never plays anything).
  const bodyOf = (it, full = true) =>
    full
      ? h("div", { className: "text" }, it.body ? markdown.render(it.body, it.files, { item: it.ref }) : null, attachments.show((it.files ?? []).filter(f => !markdown.inlined(it.body).has(markdown.keyOf(f)))))
      : h("div", { className: "text" }, it.body ? h("p", { className: "preview", textContent: markdown.plain(it.body) }) : null, attachments.show(it.files));
  // WHERE an item's files are kept: its board's space (public while the board reads in public), or the profile
  // (public unless the post is only its author's).
  const placeOf = async it => ({ sp: it.board ? await posts.boardOf(it.board.id) : threadAt.sp, pub: it.board ? !!it.pub : threadAt.sp ? threadAt.pub : !it.private && threadAt.pub !== false });
  // The EDITOR for a post or a comment (`md-editor`, with its files: kept, others added).
  function editorFor({ value = "", files = [], sp = null, pub = false, placeholder = "", label = "" } = {}) {
    const pick = attachments.picker({ space: sp, from: { app: "board" }, public: () => pub, media: true, publish: true });
    pick.preset(files);
    return mdEditor.create({ value, pick, placeholder, label });
  }
  // EDIT one's own post or comment in place: its text and files; saved as a new version (shown "(edited)").
  async function editIn(host, it, done) {
    const at = await placeOf(it);
    const ed = editorFor({ value: it.body ?? "", files: it.files ?? [], ...at, label: "Edit" });
    const said = h("p", { className: "said", hidden: true });
    const save = h("button", { type: "button", className: "go", textContent: "Save" });
    save.onclick = async e => {
      e.stopPropagation();
      if (ed.busy()) return errorTo(said)(new Error("Still sending the files: a moment…"));
      save.disabled = true;
      await posts.editItem(it.ref, ed.value().trim(), { files: ed.files() }).then(done, errorTo(said));
      save.disabled = false;
    };
    host.replaceChildren(h("div", { className: "reply", onclick: e => e.stopPropagation() }, ed.el, h("div", { className: "row" }, said, h("button", { type: "button", className: "ghost", textContent: "Cancel", onclick: e => (e.stopPropagation(), done()) }), save)));
    ed.focus();
  }

  // A POST: its look is the kind's (`cards`), in a list and on its own page; its votes and its tools given.
  function postCard(p, full = false) {
    // A post opens where it lives: in Discover (read from outside), in its space, or in the personal space.
    const open = () => (location.hash = discovering() ? `#/discover/board/p/${p.ref}` : posts.pageOf(p.ref, p.kind ?? "post"));
    const body = full ? h("div", {}, bodyOf(p, true)) : null;
    const acts = actionsCap.bar(p, { outside: outsideFor(p), open, discover: discovering(), edit: full ? () => editIn(body, p, () => draw()) : null, removed: () => (full ? (location.hash = base()) : draw()), changed: () => draw() });
    return cards.card(p, { href: null, open, lead: actionsCap.votes(p, { outside: outsideFor(p) }), actions: [acts], body });
  }

  // THE SIDE PANEL: the space's board (its name, members, Create post), or a profile.
  // A PUBLIC space seen from Discover: what it is, and Join when it is open (or you are in: open it).
  async function publicPanel(d) {
    const pr = await roles.ofPublic(d);
    const mine = (await space.mine()).some(s => s.id === d.id);
    // Join, Requested (while a member lets you in), Open: the one join control.
    // Follow always (its public board in your feed); Join too when joining is open.
    const join = !mine ? await (await ctx.require("join-button")).control(d, { open: `#/s/${d.id}/board`, joinable: pr.policy("", "join") === "anyone" }) : null;
    return h(
      "div",
      { className: "panel" },
      h("h3", { textContent: `b/${space.shown(d)}` }),
      // Who takes part, as its policy says (`roles`: each of post, comment, vote — anyone, or its members).
      h("p", { textContent: `🌐 A public board: anyone reads it. ${["post", "comment", "vote"].map(a => `${a[0].toUpperCase()}${a.slice(1)}: ${pr.policyIn("text", a) === "anyone" ? "anyone" : "its members"}`).join(" · ")}.` }),
      !mine && pr.policyIn("text", "post") === "anyone" ? h("a", { className: "go", href: `#/discover/board/b/${d.id}/submit`, textContent: "Create post" }) : null,
      mine ? h("a", { className: "go", href: `#/s/${d.id}/board`, textContent: "Open in your space" }) : join ?? h("p", { textContent: "Joining is by invite." }),
    );
  }
  async function sidePanel(w) {
    // Discover: the public spaces; one public space: its name (joining is a member's way to post).
    if (discovering()) {
      // Public boards only (a space may be listed for joining, its board still its members').
      const listed = await posts.publicSpaces();
      const spaces = (await Promise.all(listed.map(async x => ((await roles.ofPublic(x).catch(() => null))?.policy("board", "read") === "anyone" ? x : null)))).filter(Boolean);
      const d = w.pub ? spaces.find(x => x.id === w.pub) : null;
      return [
        d ? await publicPanel(d) : null,
        h("div", { className: "panel" }, h("h3", { textContent: "Public spaces" }), spaces.length ? h("ul", {}, ...spaces.map(x => h("li", {}, h("a", { href: `#/discover/board/b/${x.id}`, textContent: `b/${space.shown(x)}` })))) : h("p", { textContent: "None listed yet." })),
      ].filter(Boolean);
    }
    const create = h("a", { className: "go", href: `${base()}/submit`, textContent: "Create post" });
    if (w.feed) return [h("div", { className: "panel" }, h("h3", { textContent: "Feed" }), h("p", { textContent: "The boards of every space you are in, the people you follow, and your friends." }), create)];
    if (w.board) {
      const sp = await posts.boardOf(w.board);
      if (!sp) return [];
      const r = await roles.of(sp);
      const n = r.members().length;
      // BOARD'S OWN SETTINGS (its owner and admins): who may post, and its rules (shown here).
      const rules = r.config("board", "rules", "");
      // Its settings: on the space's Home (`app-settings`: the one place).
      const settingsBtn = r.can(me, "apps") ? h("a", { className: "ghost", href: appSettings.href(sp, "board"), textContent: "Board settings" }) : null;
      const mayPost = r.allows("post", me, "board");
      return [
        h(
          "div",
          { className: "panel" },
          h("h3", { textContent: `b/${space.shown(sp)}` }),
          h("p", { textContent: `${n} member${n === 1 ? "" : "s"} · you: ${r.role(me) ?? "member"}` }),
          r.policy("board", "read") === "anyone" ? h("p", { textContent: "🌐 Public: anyone reads its posts." }) : null,
          mayPost ? create : h("p", { textContent: "Only admins post here; comment and vote on any post." }),
          settingsBtn,
        ),
        rules ? h("div", { className: "panel" }, h("h3", { textContent: "Rules" }), h("p", { className: "text", style: "white-space: pre-wrap", textContent: rules })) : null,
      ].filter(Boolean);
    }
    const by = w.by ?? me;
    return [
      h(
        "div",
        { className: "panel" },
        h("h3", {}, who(by)),
        by === me ? h("p", { textContent: "Your profile: posts your followers read." }) : h("button", { type: "button", className: "ghost", textContent: "Follow, message…", onclick: e => person.open(e.currentTarget, by) }),
        by === me ? create : null,
      ),
    ];
  }

  const sortBar = () => h("div", { className: "panel sorts" }, bar.el());

  async function listPage(w) {
    const outside = w.pub ? await descOf(w.pub) : null;
    const span = bar.span();
    // SAVED: `where`'s (what you saved, of Board's kinds).
    const list = w.saved ? await at.read() : bar.reorder(await posts.list(w.discover ? { discover: true } : outside ? { outside } : w.board ? { board: w.board } : w.feed ? { feed: true } : { by: w.by }, bar.sort(), "post", bar.options()));
    const older = bar.older("Older posts", list.length);
    if (w.discover || w.pub) {
      const head = h("div", { className: "panel banner" }, h("h2", { textContent: w.pub ? `b/${outside ? space.shown(outside) : "?"} · 🌐 public` : "🧭 Public boards" }));
      return [head, sortBar(), ...(list.length ? list.map(p => postCard(p)) : [h("p", { className: "none", textContent: `No public posts ${span}.` })]), older];
    }
    const sp = w.board ? await posts.boardOf(w.board) : null;
    const head = w.board
      ? h("div", { className: "panel banner" }, h("h2", { textContent: `b/${sp ? space.shown(sp) : "?"}` }))
      : w.by
        ? h("div", { className: "panel banner" }, h("h2", {}, who(w.by)))
        : null;
    const empty = w.board ? `No posts ${span}.` : w.feed ? `Nothing ${span}: your spaces' boards, the people you follow and your friends post here.` : w.by === me ? `You have not posted ${span}.` : `No posts ${span}.`;
    return [head, sortBar(), ...(list.length ? list.map(p => postCard(p)) : [h("p", { className: "none", textContent: empty })]), older];
  }

  // A public space's description (Discover): from the public list.
  const descOf = async id => (await posts.publicSpaces()).find(d => d.id === id) ?? null;
  let shownPost = null;
  let threadAt = { sp: null, pub: true }; // the thread shown: where its comments' files are kept
  async function postPage(ref) {
    const w = route();
    const outside = w.pub ? await descOf(w.pub) : null;
    const p = (shownPost = await posts.get(ref, { outside }));
    if (!p) return [h("p", { className: "none", textContent: "This post is not there (removed, or not found yet)." })];
    // Where the post's files go when it is edited: its space (public as the post is), or the profile.
    const sp = !outside && p.board ? await posts.boardOf(p.board.id) : null;
    threadAt = { sp, pub: sp ? !!p.pub : !p.private };
    // Its COMMENTS: the one thread (`comments`), as under a video or an audio.
    const thread = await (await ctx.require("comments")).create({ item: p, outside, app: "board" });
    return [postCard(p, true), h("div", { className: "panel" }, thread.el)];
  }

  async function submitPage(board) {
    const sp = board ? await posts.boardOf(board) : null;
    // A space this person is not in, whose policy lets anyone post: posted from outside (`items.submit`'s `outside`).
    const outside = board && !sp ? await descOf(board) : null;
    const said = h("p", { className: "said", hidden: true });
    // FILES on the post: public where the post is (a public board, a public profile post: keyed by their content, the
    // whole network dedups them), else sealed for the space (or you) — asked as each is picked.
    // WHO SEES IT: the one picker (`audience`) — its files put public exactly when the post is.
    const who = await (await ctx.require("audience")).picker({ space: sp, kind: "post" });
    const pick = attachments.picker({ space: sp, from: { app: "board" }, media: true, publish: true, public: () => who.isPublic() });
    const ed = mdEditor.create({ pick, label: "Text" });
    const f = h(
      "form",
      { className: "panel reply" },
      h("h3", { textContent: "Create a post" }),
      sp || outside ? h("p", {}, `To b/${space.shown(sp ?? outside)}.`) : null,
      // From outside: public, as the board is (who sees it and who answers it: the space's policy).
      outside ? null : who.el,
      h("label", {}, "Title", h("input", { className: "field", name: "title", maxLength: 300, autocomplete: "off", required: true })),
      h("label", {}, "Text (optional) — Markdown; 🖼 puts an image, a video or an audio where you write it"),
      ed.el,
      h("div", { className: "row" }, said, h("button", { className: "go", textContent: "Post" })),
    );
    f.onsubmit = async e => {
      e.preventDefault();
      said.hidden = true;
      const btn = f.querySelector("button.go");
      if (pick.busy()) return errorTo(said)(new Error("Still sending the files: a moment…"));
      btn.disabled = true;
      try {
        const ref = await posts.submit({ board: sp?.id ?? outside?.id ?? null, outside, title: f.elements.title.value, body: ed.value(), audience: outside ? "public" : who.value(), write: who.write(), files: pick.files() });
        location.hash = `${base()}/p/${ref}`;
      } catch (err) {
        errorTo(said)(err);
      } finally {
        btn.disabled = false;
      }
    };
    return [f];
  }

  // What is drawn (its route): the loader's route event right after mounting names the same page — drawing it again
  // would replace the page under someone typing (and a comment sent from the old page would land where nobody looks).
  let drawnFor = null;
  const routeKey = whereCap.key;
  async function draw() {
    drawnFor = routeKey();
    at = await whereCap.of({ kind: "post", app: "board", yours: "Your posts" });
    const w = route();
    // On screen: this space's board (both tables) is read as it arrives (`activity`).
    const onBoard = w.board ? await posts.boardOf(w.board) : null;
    ctx.require("activity").then(a => a.showing(onBoard ? [space.board(onBoard), space.board(onBoard, { pub: true })] : null), () => {});
    await setActions(w);
    here = discovering() ? null : w.board ? await posts.boardOf(w.board) : w.post?.startsWith("space:") ? await posts.boardOf(w.post) : null;
    const parts = await (w.post ? postPage(w.post) : w.submit ? submitPage(w.board) : listPage(w));
    // A post's page: its board's panel (or its author's profile).
    const sidebar = await sidePanel(discovering() ? w : w.post ? (shownPost?.board ? { board: shownPost.board.id } : { by: shownPost?.by }) : w);
    main.replaceChildren(...parts.filter(Boolean));
    side.replaceChildren(...sidebar);
  }
  let drawing = null;
  const redraw = () => {
    if (!el.isConnected || drawing || route().submit || route().post) return;
    drawing = draw().finally(() => (drawing = null));
  };
  main.replaceChildren(theme.loading("Reading the posts…"));
  await draw();
  posts.onChange(redraw);
  addEventListener("craftworks:route", () => el.isConnected && ctx.route === "/board" && routeKey() !== drawnFor && draw());
}
