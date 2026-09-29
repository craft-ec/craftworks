// BOARD, a page: posts, Reddit-style. A BOARD is a server's (`#/board/b/<server id>`: the server Chat shows — the same
// members, roles, invites and moderation); a PERSON's profile posts (`#/board/u/<did>`: what their followers see); HOME
// (`#/board`: your boards' and the profiles you follow, and your own); a POST with its comment tree (`#/board/p/<ref>`);
// CREATE (`#/board/submit[/<server id>]`). Sorted Hot, New or Top; a side panel for the board (your role, create a post,
// invite, settings, its chat) or your boards (join by an invite code, a new board). UI only: posts are `posts`', servers
// `space`'s and `conversation`'s, settings the one `server-settings`, names `directory`'s, and a name opens the one
// `person` menu (follow; on a board, its role and removal).
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  const [posts, directory, person, theme, space, conversation, roles, settings, places] = await Promise.all(
    ["posts", "directory", "person", "theme", "space", "conversation", "roles", "server-settings", "places"].map(n => ctx.require(n)),
  );
  const me = (await space.account()).id;
  let sort = "hot";
  // The route: what is shown.
  const where = () => {
    const s = ctx.sub || "";
    if (s.startsWith("b/")) return { board: s.slice(2) };
    if (s.startsWith("u/")) return { by: s.slice(2) };
    if (s.startsWith("p/")) return { post: s.slice(2) };
    if (s === "submit" || s.startsWith("submit/")) return { submit: s.slice(7) };
    return {};
  };
  // The top bar: on a space's board (or your profile, the personal space's), that space's places; then Home and
  // Create post.
  const account = await space.account();
  const setActions = async w => {
    const sp = w.board ? await posts.boardOf(w.board) : w.by === me ? account : null;
    ctx.actions["/board"] = [
      ...(sp ? places.of(sp, "board") : []),
      { label: "Home", href: "#/board", on: !w.board && !w.by && !w.post && w.submit == null },
      { label: "Create post", href: `#/board/submit${w.board ? `/${w.board}` : ""}`, on: w.submit != null },
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
  const boardLink = b => h("a", { className: "b", href: `#/board/b/${b.id}`, textContent: `b/${b.name}`, onclick: e => e.stopPropagation() });
  const errorTo = said => e => ((said.textContent = e?.message ?? String(e)), (said.hidden = false));

  // VOTES on a post or a comment: ▲ score ▼, changed here at once, then written.
  function votes(it, post) {
    const n = h("span", { className: "n", textContent: String(it.score) });
    const up = h("button", { type: "button", className: "up", textContent: "▲", title: "Upvote", ariaPressed: String(it.mine === 1) });
    const down = h("button", { type: "button", className: "down", textContent: "▼", title: "Downvote", ariaPressed: String(it.mine === -1) });
    const cast = v => async e => {
      e.stopPropagation();
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
    const open = () => (location.hash = `#/board/p/${p.ref}`);
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
      p.mayRemove
        ? h("button", {
            type: "button",
            textContent: p.by === me ? "Delete" : "Remove",
            onclick: async e => {
              e.stopPropagation();
              if (e.target.dataset.armed !== "1") return ((e.target.dataset.armed = "1"), (e.target.textContent = `Confirm: ${p.by === me ? "delete" : "remove"}`));
              await posts.remove(p.ref).then(() => (full ? (location.hash = p.board ? `#/board/b/${p.board.id}` : "#/board") : draw()), errorTo(said));
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
        h("div", { className: "meta" }, p.board ? boardLink(p.board) : h("span", { textContent: "profile" }), h("span", { textContent: "·" }), h("span", { textContent: "Posted by" }), who(p.by), h("time", { textContent: ago(p.at), title: new Date(p.at).toLocaleString() }), p.edited ? h("span", { textContent: "(edited)" }) : null),
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

  // THE SIDE PANEL: the board (its server: your role, create a post, invite, settings, its chat), a person, or home
  // (your boards; join one by an invite code; a new board — a new server).
  async function yourBoards() {
    const bs = await posts.boards();
    const join = h("form", {}, h("input", { name: "code", placeholder: "Invite code: xxxx-xxxx-xxxx-xxxx", autocomplete: "off", ariaLabel: "Invite code" }), h("button", { className: "go", textContent: "Join" }));
    const joinSaid = h("p", { hidden: true });
    join.onsubmit = async e => {
      e.preventDefault();
      joinSaid.hidden = true;
      try {
        await conversation.join(join.elements.code.value);
        joinSaid.textContent = "Asked to join. You are in once a member who may invite is online (looked at every 30 s).";
        joinSaid.hidden = false;
        join.elements.code.value = "";
      } catch (err) {
        errorTo(joinSaid)(err);
      }
    };
    const make = h("form", {}, h("input", { name: "name", placeholder: "A new board's name", autocomplete: "off", ariaLabel: "New board" }), h("button", { className: "go", textContent: "Create" }));
    const makeSaid = h("p", { className: "said", hidden: true });
    make.onsubmit = async e => {
      e.preventDefault();
      const name = make.elements.name.value.trim();
      if (!name) return;
      try {
        // A new board is a new server: made as Chat makes one (with its first channel), so both show it.
        const s = await space.create("server", name);
        await (await conversation.channels(s)).add("general");
        location.hash = `#/board/b/${s.id}`;
      } catch (err) {
        errorTo(makeSaid)(err);
      }
    };
    return h(
      "div",
      { className: "panel" },
      h("h3", { textContent: "Your boards" }),
      // Your personal space's board (your profile) first, then every space's.
      h("ul", {}, h("li", {}, h("a", { href: `#/board/u/${me}`, textContent: "Your profile" })), ...bs.map(b => h("li", {}, h("a", { href: `#/board/b/${b.id}`, textContent: `b/${b.name}` })))),
      bs.length ? null : h("p", { textContent: "Every space has a board beside its messages: a server, a group, a conversation." }),
      join,
      joinSaid,
      make,
      makeSaid,
    );
  }
  async function sidePanel(w) {
    const yours = await yourBoards();
    const create = h("a", { className: "go", href: `#/board/submit${w.board ? `/${w.board}` : ""}`, textContent: "Create post" });
    if (w.board) {
      const sp = await posts.boardOf(w.board);
      if (!sp) return [yours];
      const r = await roles.of(sp);
      await r.refresh().catch(() => {});
      const mine = r.role(me) ?? "member";
      const server = sp.kind === "server";
      const chs = server ? await conversation.channels(sp) : null;
      const ops = chs && { list: () => chs.list(), add: n => chs.add(n), rename: (c, n) => chs.rename(c, n), remove: c => chs.remove(c) };
      const open = tab => settings.open(sp, { tab, channels: ops, left: () => (location.hash = "#/board") });
      return [
        h(
          "div",
          { className: "panel" },
          h("h3", { textContent: `b/${sp.name}` }),
          h("p", { textContent: `${r.members().length} member${r.members().length === 1 ? "" : "s"} · you: ${mine}. The same space as its ${server ? "chat" : "conversation"}.` }),
          create,
          server && r.can(me, "invite") ? h("button", { type: "button", className: "ghost", textContent: "Invite", onclick: () => open("invites") }) : null,
          server ? h("button", { type: "button", className: "ghost", textContent: "Settings", onclick: () => open("overview") }) : null,

        ),
        yours,
      ];
    }
    if (w.by) return [h("div", { className: "panel" }, h("h3", {}, who(w.by)), w.by === me ? h("p", { textContent: "Your profile: posts your followers see." }) : h("button", { type: "button", className: "ghost", textContent: "Follow, message…", onclick: e => person.open(e.currentTarget, w.by) }), w.by === me ? create : null), yours];
    return [h("div", { className: "panel" }, h("h3", { textContent: "Home" }), h("p", { textContent: "Posts from your boards and the people you follow." }), create), yours];
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
    const list = await posts.list(w.board ? { board: w.board } : w.by ? { by: w.by } : {}, sort);
    const sp = w.board ? await posts.boardOf(w.board) : null;
    const head = w.board
      ? h("div", { className: "panel banner" }, h("h2", { textContent: `b/${sp?.name ?? "?"}` }))
      : w.by
        ? h("div", { className: "panel banner" }, h("h2", {}, who(w.by)))
        : null;
    const empty = w.board ? "No posts here yet. Be the first." : w.by ? "No posts yet." : "Nothing here yet. Join a board, follow people (from their name), or create a post.";
    return [head, sortBar(), ...(list.length ? list.map(p => postCard(p)) : [h("p", { className: "none", textContent: empty })])];
  }

  let shownPost = null;
  async function postPage(ref) {
    const p = (shownPost = await posts.get(ref));
    if (!p) return [h("p", { className: "none", textContent: "This post is not there (removed, or not found yet)." })];
    const tree = h("div", { className: "comments" });
    const again = async () => {
      const cs = await posts.thread(ref);
      tree.replaceChildren(...(cs.length ? cs.map(c => commentTree(c, ref, again)) : [h("p", { className: "none", textContent: "No comments yet." })]));
    };
    tree.append(theme.loading("Reading the comments…"));
    again().catch(() => {});
    return [postCard(p, true), h("div", { className: "panel" }, replyForm(ref, ref, "Comment", again), tree)];
  }

  async function submitPage(board) {
    const bs = await posts.boards();
    const said = h("p", { className: "said", hidden: true });
    const f = h(
      "form",
      { className: "panel reply" },
      h("h3", { textContent: "Create a post" }),
      h("label", {}, "Post to", h("select", { className: "field", name: "board" }, h("option", { value: "", textContent: "Your profile (your followers)" }), ...bs.map(b => h("option", { value: b.id, textContent: `b/${b.name}`, selected: b.id === board })))),
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
        const ref = await posts.submit({ board: f.elements.board.value || null, title: f.elements.title.value, body: f.elements.body.value });
        location.hash = `#/board/p/${ref}`;
      } catch (err) {
        errorTo(said)(err);
      } finally {
        btn.disabled = false;
      }
    };
    return [f];
  }

  async function draw() {
    const w = where();
    await setActions(w);
    here = w.board ? await posts.boardOf(w.board) : w.post?.startsWith("space:") ? await posts.boardOf(w.post) : null;
    const parts = await (w.post ? postPage(w.post) : w.submit != null ? submitPage(w.submit) : listPage(w));
    // A post's page: its board's panel.
    const sidebar = await sidePanel(w.post ? { board: shownPost?.board?.id } : w);
    main.replaceChildren(...parts.filter(Boolean));
    side.replaceChildren(...sidebar);
  }
  let drawing = null;
  const redraw = () => {
    if (!el.isConnected || drawing || where().submit != null || where().post) return;
    drawing = draw().finally(() => (drawing = null));
  };
  main.replaceChildren(theme.loading("Reading the posts…"));
  await draw();
  posts.onChange(redraw);
  // Welcomes waiting (a board joined by a code: its server), and — where this person may invite — who asked by a code
  // of the board shown, let in: now and every 30 s while Board is open (as Chat does).
  const tick = async () => {
    const joined = await conversation.accept().catch(() => []);
    if (joined.length) redraw();
    if (here && (await roles.of(here)).can(me, "invite")) await conversation.admit(here).catch(() => []);
  };
  tick();
  const every = setInterval(() => (el.isConnected ? tick() : clearInterval(every)), 30000);
  addEventListener("craftworks:route", () => el.isConnected && location.hash.startsWith("#/board") && draw());
}
