// BOARD, a page: posts, Reddit-style, CONFINED to the space open on the rail. In a SHARED space
// (`#/s/<space>/board`): its own posts only — its members post, its roles and moderation apply. In the PERSONAL space
// (`#/board`): your own posts, your profile (public: what your followers read); another person's profile is
// `#/board/u/<did>`; and the FEED (`#/board/feed`: the personal view gathers — the boards of every space you are in,
// and the profiles you follow). A POST with its comment tree: `…/board/p/<ref>`; CREATE: `…/board/submit`. Sorted Hot, New or Top.
// UI only: posts are `posts`', names `directory`'s, and a name opens the one `person` menu (follow; in a space, its
// role and removal). The space itself (its members, settings, invites) is its Home's.
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  const [posts, directory, person, theme, space, roles, appSettings] = await Promise.all(["posts", "directory", "person", "theme", "space", "roles", "app-settings"].map(n => ctx.require(n)));
  const me = (await space.account()).id;
  let sort = "hot";
  // The route: what is shown.
  // Where Board is: the space open (`ctx.space`), or the personal space.
  const discovering = () => ctx.space === "discover";
  const base = () => (discovering() ? "#/discover/board" : ctx.space ? `#/s/${ctx.space}/board` : "#/board");
  const where = () => {
    const s = ctx.sub || "";
    // DISCOVER (the public view): every public board, one space's (`b/<id>`), or a public post (`p/<ref>`).
    if (discovering()) {
      if (s.startsWith("b/")) return { pub: s.slice(2) };
      if (s.startsWith("p/")) return { post: s.slice(2), pub: s.slice(2).match(/^space:([0-9a-f]{64})\//)?.[1] };
      return { discover: true };
    }
    const at = ctx.space ? { board: ctx.space } : {};
    if (s.startsWith("p/")) return { ...at, post: s.slice(2) };
    if (s === "submit") return { ...at, submit: true };
    if (!ctx.space && s === "feed") return { feed: true };
    if (!ctx.space && s.startsWith("u/")) return { by: s.slice(2) };
    return ctx.space ? at : { by: me };
  };
  // The top bar: its sub-pages — the posts, and Create post.
  const setActions = async w => {
    if (discovering()) {
      ctx.actions["/board"] = [{ label: "All public boards", href: base(), on: !!w.discover }];
      return dispatchEvent(new CustomEvent("craftworks:actions"));
    }
    ctx.actions["/board"] = [
      ...(ctx.space ? [] : [{ label: "Feed", href: "#/board/feed", on: !!w.feed }]),
      { label: w.board ? "Posts" : w.by === me ? "Your posts" : "Posts", href: base(), on: !w.post && !w.submit && (!!w.board || w.by === me) },
      ...(w.board || w.by === me || w.feed || w.submit ? [{ label: "Create post", href: `${base()}/submit`, on: !!w.submit }] : []),
    ];
    dispatchEvent(new CustomEvent("craftworks:actions"));
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
      .bd .sorts { display: flex; gap: var(--cw-space-1); padding: var(--cw-space-2); }
      .bd .sorts button { border: 0; background: none; color: var(--cw-muted); border-radius: var(--cw-radius-pill); padding: 4px var(--cw-space-3); font-weight: 600; }
      .bd .sorts button:hover { background: var(--cw-hover); }
      .bd .sorts button[aria-pressed="true"] { background: var(--cw-pressed); color: var(--cw-fg); }
      .bd .post { display: grid; grid-template-columns: 40px minmax(0, 1fr); background: var(--cw-surface); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); overflow: hidden; }
      .bd .post.link:hover { border-color: var(--cw-muted); cursor: pointer; }
      .bd .post > .votes { background: var(--cw-bg); padding: var(--cw-space-2) 0; }
      .bd .votes { display: grid; justify-items: center; align-content: start; gap: 0; }
      .bd .votes button { border: 0; background: none; color: var(--cw-muted); padding: 0 4px; line-height: 1.3; border-radius: var(--cw-radius-sm); }
      .bd .votes button:hover { background: var(--cw-hover); }
      .bd .votes button.up[aria-pressed="true"] { color: #ff4500; }
      .bd .votes button.down[aria-pressed="true"] { color: #7193ff; }
      .bd .votes .n { font-weight: 700; font-size: var(--cw-text-xs); }
      .bd .post .in { padding: var(--cw-space-2) var(--cw-space-3); display: grid; gap: 4px; min-width: 0; }
      .bd .meta { display: flex; flex-wrap: wrap; gap: 4px; align-items: baseline; color: var(--cw-muted); font-size: var(--cw-text-xs); }
      .bd .meta .b { color: var(--cw-fg); font-weight: 700; }
      .bd .meta .b:hover, .bd .meta .by:hover { text-decoration: underline; cursor: pointer; }
      .bd .post h3 { margin: 0; font-size: 1.1rem; font-weight: 600; overflow-wrap: anywhere; }
      .bd .post .text { white-space: pre-wrap; overflow-wrap: anywhere; line-height: 1.5; margin: 0; font-size: var(--cw-text-sm); }
      .bd .post.link .text { display: -webkit-box; -webkit-line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; color: var(--cw-muted); }
      .bd .acts { display: flex; flex-wrap: wrap; gap: 2px; align-items: center; }
      .bd .acts button { border: 0; background: none; color: var(--cw-muted); font-size: var(--cw-text-xs); font-weight: 600; padding: 4px var(--cw-space-2); border-radius: var(--cw-radius-sm); }
      .bd .acts button:hover { background: var(--cw-hover); color: var(--cw-fg); }
      .bd .acts .votes { display: flex; align-items: center; gap: 2px; }
      .bd textarea, .bd .field { font: inherit; padding: var(--cw-space-2); border-radius: var(--cw-radius-sm); width: 100%; box-sizing: border-box; }
      .bd textarea { resize: vertical; min-height: 90px; }
      .bd form.reply { display: grid; gap: var(--cw-space-2); }
      .bd form.reply .row { display: flex; justify-content: flex-end; gap: var(--cw-space-2); }
      .bd .comments { display: grid; gap: var(--cw-space-2); }
      .bd .c { display: grid; grid-template-columns: 20px minmax(0, 1fr); column-gap: var(--cw-space-2); }
      .bd .c > .rail { display: flex; justify-content: center; cursor: pointer; }
      .bd .c > .rail::before { content: ""; width: 2px; background: var(--cw-line); border-radius: 1px; }
      .bd .c > .rail:hover::before { background: var(--cw-accent); }
      .bd .c > .body { display: grid; gap: 4px; min-width: 0; }
      .bd .c .text { white-space: pre-wrap; overflow-wrap: anywhere; line-height: 1.5; margin: 0; }
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
    const n = h("span", { className: "by", textContent: directory.shown(did), onclick: e => (e.stopPropagation(), person.open(e.currentTarget, did, here ? { space: here } : {})) });
    directory.name(did).then(t => (n.textContent = t), () => {});
    return n;
  };
  const boardLink = b => h("span", { className: "b", textContent: `b/${space.shown(b)}` });
  const errorTo = said => e => ((said.textContent = e?.message ?? String(e)), (said.hidden = false));

  const flag = async (kind, ref) => {
    await (await (await ctx.require("moderation")).lists()).flag(kind, ref);
    await draw();
  };
  // VOTES on a post or a comment: ▲ score ▼, changed here at once, then written.
  function votes(it, post) {
    const n = h("span", { className: "n", textContent: String(it.score) });
    const up = h("button", { type: "button", className: "up", textContent: "▲", title: "Upvote", ariaPressed: String(it.mine === 1) });
    const down = h("button", { type: "button", className: "down", textContent: "▼", title: "Downvote", ariaPressed: String(it.mine === -1) });
    const cast = v => async e => {
      e.stopPropagation();
      // From outside (Discover): scores only — members vote; nor where the space's policy says you may not.
      if (discovering() || (here && !(await roles.of(here)).allows("vote", me, "board"))) return;
      const next = it.mine === v ? 0 : v;
      it.score += next - it.mine;
      it.mine = next;
      n.textContent = String(it.score);
      up.ariaPressed = String(next === 1);
      down.ariaPressed = String(next === -1);
      await posts.vote(it.ref, next, post).catch(() => {});
    };
    up.onclick = cast(1);
    down.onclick = cast(-1);
    return h("div", { className: "votes" }, up, n, down);
  }

  // A POST: in a list (a link to its page, the text cut short) or on its own page.
  function postCard(p, full = false) {
    const said = h("p", { className: "said", hidden: true });
    // A space's post opens in its space (the rail follows); a profile's in the personal space.
    // A post opens where it lives: in Discover (read from outside), in its space, or in the personal space.
    const open = () => (location.hash = discovering() ? `#/discover/board/p/${p.ref}` : p.board ? `#/s/${p.board.id}/board/p/${p.ref}` : `#/board/p/${p.ref}`);
    const acts = h(
      "div",
      { className: "acts" },
      h("button", { type: "button", textContent: `💬 ${p.comments} Comment${p.comments === 1 ? "" : "s"}`, onclick: e => (e.stopPropagation(), open()) }),
      h("button", {
        type: "button",
        textContent: "Share",
        onclick: e => {
          e.stopPropagation();
          navigator.clipboard.writeText(p.ref).then(() => (e.target.textContent = "Copied"), () => {});
        },
      }),
      // DISCOVER: flag it on your moderation list (what you, and whoever applies your list, no longer see there).
      ...(discovering() && p.by !== me
        ? [
            h("button", { type: "button", textContent: "Flag post", onclick: e => (e.stopPropagation(), flag("post", p.ref)) }),
            h("button", { type: "button", textContent: "Flag author", onclick: e => (e.stopPropagation(), flag("person", p.by)) }),
          ]
        : []),
      p.mayRemove
        ? h("button", {
            type: "button",
            textContent: p.by === me ? "Delete" : "Remove",
            onclick: async e => {
              e.stopPropagation();
              if (e.target.dataset.armed !== "1") return ((e.target.dataset.armed = "1"), (e.target.textContent = `Confirm: ${p.by === me ? "delete" : "remove"}`));
              await posts.remove(p.ref).then(() => (full ? (location.hash = base()) : draw()), errorTo(said));
            },
          })
        : null,
    );
    return h(
      "article",
      { className: `post${full ? "" : " link"}`, onclick: full ? null : open },
      votes(p, p.ref),
      h(
        "div",
        { className: "in" },
        h("div", { className: "meta" }, p.board ? boardLink(p.board) : h("span", { textContent: "profile" }), p.pub ? h("span", { textContent: "· 🌐 public" }) : null, p.private ? h("span", { textContent: "· 🔒 only you" }) : null, h("span", { textContent: "·" }), h("span", { textContent: "Posted by" }), who(p.by), h("time", { textContent: ago(p.at), title: new Date(p.at).toLocaleString() }), p.edited ? h("span", { textContent: "(edited)" }) : null),
        h("h3", { textContent: p.title }),
        p.body ? h("p", { className: "text", textContent: p.body }) : null,
        acts,
        said,
      ),
    );
  }

  // A COMMENT and its replies: a rail to fold it, Reply opening a box under it.
  function commentTree(c, post, again) {
    const said = h("p", { className: "said", hidden: true });
    const box = h("div", { className: "c" });
    const kids = h("div", { className: "kids" }, ...c.replies.map(r => commentTree(r, post, again)));
    const fold = () => box.classList.toggle("folded");
    const count = (x => x(c))(function all(x) {
      return x.replies.reduce((n, r) => n + 1 + all(r), 0);
    });
    const replyAt = h("div", {});
    const acts = h(
      "div",
      { className: "acts" },
      votes(c, post),
      h("button", {
        type: "button",
        textContent: "Reply",
        onclick: () => {
          if (replyAt.firstChild) return replyAt.replaceChildren();
          replyAt.replaceChildren(replyForm(post, c.ref, "Reply", again, () => replyAt.replaceChildren()));
          replyAt.querySelector("textarea").focus();
        },
      }),
      c.mayRemove ? h("button", { type: "button", textContent: c.by === me ? "Delete" : "Remove", onclick: () => posts.remove(c.ref).then(again, errorTo(said)) }) : null,
    );
    box.append(
      h("div", { className: "rail", title: "Fold", onclick: fold }),
      h(
        "div",
        { className: "body" },
        h("div", { className: "meta" }, who(c.by), h("time", { textContent: ago(c.at), title: new Date(c.at).toLocaleString() }), c.edited ? h("span", { textContent: "(edited)" }) : null, h("button", { type: "button", className: "fold", textContent: count ? `[–] ${count} more` : "[–]", onclick: fold })),
        h("p", { className: "text", textContent: c.body }),
        acts,
        said,
        replyAt,
        kids,
      ),
    );
    return box;
  }
  function replyForm(post, re, label, again, cancel = null) {
    const said = h("p", { className: "said", hidden: true });
    const f = h(
      "form",
      { className: "reply" },
      h("textarea", { name: "body", placeholder: re === post ? "What are your thoughts?" : "Write a reply", ariaLabel: label }),
      h("div", { className: "row" }, said, cancel ? h("button", { type: "button", className: "ghost", textContent: "Cancel", onclick: cancel }) : null, h("button", { className: "go", textContent: label })),
    );
    f.onsubmit = async e => {
      e.preventDefault();
      said.hidden = true;
      const btn = f.querySelector("button.go");
      btn.disabled = true;
      try {
        await posts.comment(post, re, f.elements.body.value);
        f.elements.body.value = "";
        await again();
      } catch (err) {
        errorTo(said)(err);
      } finally {
        btn.disabled = false;
      }
    };
    return f;
  }

  // THE SIDE PANEL: the space's board (its name, members, Create post), or a profile.
  // A PUBLIC space seen from Discover: what it is, and Join when it is open (or you are in: open it).
  async function publicPanel(d) {
    const pr = await roles.ofPublic(d);
    const mine = (await space.mine()).some(s => s.id === d.id);
    const said = h("p", { hidden: true });
    const join =
      !mine && pr.policy("", "join") === "anyone"
        ? h("button", {
            type: "button",
            className: "go",
            textContent: "Join",
            onclick: async e => {
              e.target.disabled = true;
              await (await ctx.require("conversation")).joinOpen(d).then(
                () => ((said.textContent = "Asked to join: you are in once a member who may invite is online."), (said.hidden = false)),
                err => ((said.textContent = err.message), (said.hidden = false), (e.target.disabled = false)),
              );
            },
          })
        : null;
    return h(
      "div",
      { className: "panel" },
      h("h3", { textContent: `b/${space.shown(d)}` }),
      h("p", { textContent: `🌐 A public board: anyone reads it; its ${pr.members().length} member${pr.members().length === 1 ? "" : "s"} post, comment and vote.` }),
      mine ? h("a", { className: "go", href: `#/s/${d.id}/board`, textContent: "Open in your space" }) : join ?? h("p", { textContent: "Joining is by invite." }),
      said,
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
      await r.refresh().catch(() => {});
      const n = r.members().length;
      // BOARD'S OWN SETTINGS (its owner and admins): who may post, and its rules (shown here).
      const rules = r.config("board", "rules", "");
      const settingsBtn = r.can(me, "apps")
        ? h("button", {
            type: "button",
            className: "ghost",
            textContent: "Board settings",
            onclick: () =>
              appSettings.open(
                sp,
                "Board settings",
                [
                  // Who may read: the owner's (anyone makes the space's members, roles and moderation public too).
                  { action: "read", path: "board", label: "Who may read (Anyone: public — its members and moderation too; posts made before stay as they were)" },
                  { action: "post", path: "board", label: "Who may post" },
                  { action: "comment", path: "board", label: "Who may comment" },
                  { action: "vote", path: "board", label: "Who may vote" },
                  { key: "rules", app: "board", label: "Rules (shown beside the board)" },
                ],
                // Made public: its acts published, and the space listed in Discover.
                { saved: async changed => changed["board|read"] === "anyone" && (await r.publish(), await (await ctx.require("index")).listSpace(sp)) },
              ),
          })
        : null;
      const mayPost = r.allows("post", me, "board");
      return [
        h(
          "div",
          { className: "panel" },
          h("h3", { textContent: `b/${space.shown(sp)}` }),
          h("p", { textContent: `${n} member${n === 1 ? "" : "s"} · you: ${r.role(me) ?? "member"}` }),
          r.policy("board", "read") === "anyone" ? h("p", { textContent: "🌐 Public: anyone reads new posts." }) : null,
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

  const sortBar = () =>
    h(
      "div",
      { className: "panel sorts" },
      ...[["hot", "🔥 Hot"], ["new", "✨ New"], ["top", "📈 Top"]].map(([k, label]) =>
        h("button", { type: "button", textContent: label, ariaPressed: String(sort === k), onclick: () => ((sort = k), draw()) }),
      ),
    );

  async function listPage(w) {
    const outside = w.pub ? await descOf(w.pub) : null;
    const list = await posts.list(w.discover ? { discover: true } : outside ? { outside } : w.board ? { board: w.board } : w.feed ? { feed: true } : { by: w.by }, sort);
    if (w.discover || w.pub) {
      const head = h("div", { className: "panel banner" }, h("h2", { textContent: w.pub ? `b/${outside ? space.shown(outside) : "?"} · 🌐 public` : "🧭 Public boards" }));
      return [head, sortBar(), ...(list.length ? list.map(p => postCard(p)) : [h("p", { className: "none", textContent: "No public posts yet." })])];
    }
    const sp = w.board ? await posts.boardOf(w.board) : null;
    const head = w.board
      ? h("div", { className: "panel banner" }, h("h2", { textContent: `b/${sp ? space.shown(sp) : "?"}` }))
      : w.by
        ? h("div", { className: "panel banner" }, h("h2", {}, who(w.by)))
        : null;
    const empty = w.board ? "No posts here yet. Be the first." : w.feed ? "Nothing yet: your spaces' boards, the people you follow and your friends post here." : w.by === me ? "You have not posted yet." : "No posts yet.";
    return [head, sortBar(), ...(list.length ? list.map(p => postCard(p)) : [h("p", { className: "none", textContent: empty })])];
  }

  // A public space's description (Discover): from the public list.
  const descOf = async id => (await posts.publicSpaces()).find(d => d.id === id) ?? null;
  let shownPost = null;
  async function postPage(ref) {
    const w = where();
    const outside = w.pub ? await descOf(w.pub) : null;
    const p = (shownPost = await posts.get(ref, { outside }));
    if (!p) return [h("p", { className: "none", textContent: "This post is not there (removed, or not found yet)." })];
    const tree = h("div", { className: "comments" });
    const again = async () => {
      const cs = await posts.thread(ref, { outside });
      tree.replaceChildren(...(cs.length ? cs.map(c => commentTree(c, ref, again)) : [h("p", { className: "none", textContent: "No comments yet." })]));
    };
    tree.append(theme.loading("Reading the comments…"));
    again().catch(() => {});
    // From outside (Discover): read only — members comment.
    // Who may comment here: the space's policy (a profile's: open).
    const sp = !outside && p.board ? await posts.boardOf(p.board.id) : null;
    const mayComment = !sp || (await roles.of(sp)).allows("comment", me, "board");
    return [
      postCard(p, true),
      h(
        "div",
        { className: "panel" },
        outside ? h("p", { className: "none", textContent: "Only the space's members comment and vote." }) : mayComment ? replyForm(ref, ref, "Comment", again) : h("p", { className: "none", textContent: "Comments are closed to you here." }),
        tree,
      ),
    ];
  }

  async function submitPage(board) {
    const sp = board ? await posts.boardOf(board) : null;
    const said = h("p", { className: "said", hidden: true });
    const f = h(
      "form",
      { className: "panel reply" },
      h("h3", { textContent: "Create a post" }),
      sp
        ? h("p", {}, `To b/${space.shown(sp)}: its members read it.`)
        : // YOUR PROFILE: who sees this post — everyone (public: what your followers read) or only you (private).
          h(
            "label",
            {},
            "Who sees it",
            h("select", { className: "field", name: "audience" }, h("option", { value: "public", textContent: "🌐 Everyone (public, your followers read it)" }), h("option", { value: "private", textContent: "🔒 Only you" })),
          ),
      h("label", {}, "Title", h("input", { className: "field", name: "title", maxLength: 300, autocomplete: "off", required: true })),
      h("label", {}, "Text (optional)", h("textarea", { name: "body" })),
      h("div", { className: "row" }, said, h("button", { className: "go", textContent: "Post" })),
    );
    f.onsubmit = async e => {
      e.preventDefault();
      said.hidden = true;
      const btn = f.querySelector("button.go");
      btn.disabled = true;
      try {
        const ref = await posts.submit({ board: sp?.id ?? null, title: f.elements.title.value, body: f.elements.body.value, private: f.elements.audience?.value === "private" });
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
  const routeKey = () => `${ctx.space ?? ""}|${ctx.sub ?? ""}`;
  async function draw() {
    drawnFor = routeKey();
    const w = where();
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
    if (!el.isConnected || drawing || where().submit || where().post) return;
    drawing = draw().finally(() => (drawing = null));
  };
  main.replaceChildren(theme.loading("Reading the posts…"));
  await draw();
  posts.onChange(redraw);
  addEventListener("craftworks:route", () => el.isConnected && ctx.route === "/board" && routeKey() !== drawnFor && draw());
}
