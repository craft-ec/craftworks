// SOCIAL, a page: posts in public — the FEED (this person's posts and the posts of whom they follow), one PERSON's
// posts (`#/social/<did>`), a post's thread (comments) and votes. UI only: posts, comments and votes are `posts`',
// names `directory`'s, and a name opens the one `person` menu (follow is there). The top bar: Feed · Your posts.
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  const [posts, directory, person, theme, space] = await Promise.all(["posts", "directory", "person", "theme", "space"].map(n => ctx.require(n)));
  const me = (await space.account()).id;
  const who = () => (ctx.sub && ctx.sub.startsWith("did:craftec:") ? ctx.sub : null);
  const setActions = () => {
    ctx.actions["/social"] = [
      { label: "Feed", href: "#/social", on: !who() },
      { label: "Your posts", href: `#/social/${me}`, on: who() === me },
    ];
    dispatchEvent(new CustomEvent("craftworks:actions"));
  };
  el.innerHTML = `
    <style>
      .so { max-width: 640px; margin: 0 auto; display: grid; gap: var(--cw-space-3); }
      .so button { font: inherit; cursor: pointer; }
      .so form.new { display: grid; gap: var(--cw-space-2); background: var(--cw-surface); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); padding: var(--cw-space-3); }
      .so textarea { font: inherit; resize: vertical; min-height: 64px; padding: var(--cw-space-2); border-radius: var(--cw-radius-sm); }
      .so form.new .row { display: flex; justify-content: flex-end; gap: var(--cw-space-2); align-items: center; }
      .so .go { border: 0; border-radius: var(--cw-radius); padding: 6px var(--cw-space-4); background: var(--cw-accent); color: var(--cw-accent-fg); }
      .so .head { display: flex; align-items: center; gap: var(--cw-space-2); padding: 0 var(--cw-space-1); }
      .so .head h2 { margin: 0; font-size: 1.1rem; flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .so article { display: grid; grid-template-columns: 40px 1fr; gap: var(--cw-space-2); background: var(--cw-surface); border: 1px solid var(--cw-line);
        border-radius: var(--cw-radius); padding: var(--cw-space-3); }
      .so .votes { display: grid; justify-items: center; align-content: start; gap: 2px; }
      .so .votes button { border: 0; background: none; color: var(--cw-muted); padding: 0 4px; border-radius: var(--cw-radius-sm); }
      .so .votes button:hover { background: var(--cw-hover); }
      .so .votes button[aria-pressed="true"] { color: var(--cw-accent); }
      .so .votes .n { font-weight: 600; font-size: var(--cw-text-sm); }
      .so .meta { display: flex; gap: var(--cw-space-2); align-items: baseline; font-size: var(--cw-text-sm); min-width: 0; }
      .so .meta .by { font-weight: 600; cursor: pointer; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .so .meta .by:hover { text-decoration: underline; }
      .so .meta time, .so .meta .edited { color: var(--cw-muted); font-size: var(--cw-text-xs); }
      .so .body { white-space: pre-wrap; overflow-wrap: anywhere; line-height: 1.5; margin: var(--cw-space-1) 0; }
      .so .foot { display: flex; gap: var(--cw-space-2); }
      .so .foot button { border: 0; background: none; color: var(--cw-muted); font-size: var(--cw-text-sm); padding: 2px var(--cw-space-2); border-radius: var(--cw-radius-sm); }
      .so .foot button:hover { background: var(--cw-hover); color: var(--cw-fg); }
      .so .thread { grid-column: 2; display: grid; gap: var(--cw-space-2); border-top: 1px solid var(--cw-line); padding-top: var(--cw-space-2); }
      .so .comment { display: grid; gap: 2px; }
      .so .comment .body { margin: 0; }
      .so .thread form { display: flex; gap: var(--cw-space-2); }
      .so .thread input { flex: 1; min-width: 0; padding: 6px var(--cw-space-2); border-radius: var(--cw-radius-sm); }
      .so .none { color: var(--cw-muted); text-align: center; padding: var(--cw-space-5); margin: 0; }
      .so .said { color: var(--cw-danger); font-size: var(--cw-text-sm); margin: 0; }
    </style>
    <div class="so"></div>`;
  const root = el.querySelector(".so");
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null));
    return e;
  };
  const byline = (did, at, edited) =>
    h(
      "div",
      { className: "meta" },
      Object.assign(h("span", { className: "by", textContent: directory.shown(did), onclick: e => person.open(e.currentTarget, did) }), { did }),
      h("time", { textContent: new Date(at).toLocaleString() }),
      edited ? h("span", { className: "edited", textContent: "(edited)" }) : null,
    );
  // Names: shown at once as known, filled in as each card is read.
  const named = () => {
    for (const n of root.querySelectorAll(".by")) directory.name(n.did).then(t => (n.textContent = t), () => {});
  };
  const open = new Set(); // refs whose thread is shown

  function card(p) {
    const said = h("p", { className: "said", hidden: true });
    const fail = e => ((said.textContent = e?.message ?? String(e)), (said.hidden = false));
    const n = h("span", { className: "n", textContent: String(p.score) });
    const up = h("button", { type: "button", textContent: "▲", title: "Vote up", ariaPressed: String(p.mine === 1) });
    const down = h("button", { type: "button", textContent: "▼", title: "Vote down", ariaPressed: String(p.mine === -1) });
    const cast = v => async () => {
      const next = p.mine === v ? 0 : v;
      p.score += next - p.mine;
      p.mine = next;
      n.textContent = String(p.score);
      up.ariaPressed = String(next === 1);
      down.ariaPressed = String(next === -1);
      await posts.vote(p.ref, next).catch(fail);
    };
    up.onclick = cast(1);
    down.onclick = cast(-1);
    const thread = h("div", { className: "thread", hidden: true });
    const count = h("button", { type: "button", textContent: `💬 ${p.comments} comment${p.comments === 1 ? "" : "s"}` });
    async function showThread() {
      thread.hidden = false;
      if (!thread.firstChild) thread.append(theme.loading("Reading the comments…"));
      const t = await posts.thread(p.ref);
      count.textContent = `💬 ${t.comments.length} comment${t.comments.length === 1 ? "" : "s"}`;
      n.textContent = String((p.score = t.score));
      const f = h("form", {}, h("input", { name: "body", placeholder: "Write a comment", autocomplete: "off", ariaLabel: "Comment" }), h("button", { className: "go", textContent: "Comment" }));
      f.onsubmit = async e => {
        e.preventDefault();
        const body = f.elements.body.value;
        f.elements.body.value = "";
        await posts.comment(p.ref, body).then(showThread, fail);
      };
      thread.replaceChildren(
        ...t.comments.map(c =>
          h(
            "div",
            { className: "comment" },
            byline(c.by, c.at, c.edited),
            h("p", { className: "body", textContent: c.body }),
            c.by === me ? h("div", { className: "foot" }, h("button", { type: "button", textContent: "Delete", onclick: () => posts.remove(c.ref).then(showThread, fail) })) : null,
          ),
        ),
        f,
      );
      named();
    }
    count.onclick = () => {
      if (open.has(p.ref)) {
        open.delete(p.ref);
        thread.hidden = true;
      } else {
        open.add(p.ref);
        showThread().catch(fail);
      }
    };
    const foot = h("div", { className: "foot" }, count);
    if (p.by === me) foot.append(h("button", { type: "button", textContent: "Delete", onclick: () => posts.remove(p.ref).then(draw, fail) }));
    const a = h(
      "article",
      {},
      h("div", { className: "votes" }, up, n, down),
      h("div", {}, byline(p.by, p.at, p.edited), h("p", { className: "body", textContent: p.body }), foot, said),
      thread,
    );
    if (open.has(p.ref)) showThread().catch(fail);
    return a;
  }

  let drawing = null;
  async function draw() {
    const did = who();
    setActions();
    const list = did ? await posts.of(did) : await posts.feed();
    const parts = [];
    if (!did || did === me) {
      const f = h(
        "form",
        { className: "new" },
        h("textarea", { name: "body", placeholder: "Say something in public", ariaLabel: "New post" }),
        h("div", { className: "row" }, h("p", { className: "said", hidden: true }), h("button", { className: "go", textContent: "Post" })),
      );
      f.onsubmit = async e => {
        e.preventDefault();
        const s = f.querySelector(".said");
        s.hidden = true;
        try {
          await posts.post(f.elements.body.value);
          f.elements.body.value = "";
          await draw();
        } catch (err) {
          s.textContent = err?.message ?? String(err);
          s.hidden = false;
        }
      };
      parts.push(f);
    }
    if (did) {
      const name = h("h2", { textContent: directory.shown(did) });
      directory.name(did).then(t => (name.textContent = did === me ? `Your posts` : `${t}'s posts`));
      parts.push(h("div", { className: "head" }, name, did === me ? null : h("button", { className: "go", textContent: "…", title: "Follow, message and more", onclick: e => person.open(e.currentTarget, did) })));
    }
    parts.push(
      ...(list.length
        ? list.map(card)
        : [h("p", { className: "none", textContent: did ? "No posts yet." : "Nothing here yet. Post something, or follow people (from their name) to see theirs." })]),
    );
    root.replaceChildren(...parts);
    named();
  }
  const redraw = () => {
    if (!el.isConnected || drawing) return;
    // Not while typing: a redraw would drop what is being written.
    if (root.contains(document.activeElement) && /INPUT|TEXTAREA/.test(document.activeElement.tagName)) return;
    drawing = draw().finally(() => (drawing = null));
  };
  root.replaceChildren(theme.loading("Reading the posts…"));
  await draw();
  posts.onChange(redraw);
  addEventListener("craftworks:route", () => el.isConnected && location.hash.startsWith("#/social") && draw());
}
