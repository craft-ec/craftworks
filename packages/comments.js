// COMMENTS, a component: the ONE comment thread under anything posted — a Board post, a video, an audio. Reddit-style:
// each comment with its replies, a rail to fold it, ▲ score ▼, Reply opening the editor under it, Edit and Delete for
// one's own (Remove where a moderator may). Written with the one editor (`md-editor`: rich text or Markdown, images,
// videos and audio inline), shown as Markdown (`markdown`). The comments are `items`' (`thread`, `comment`).
// WHERE its files are kept: the item's space (public as the item is), or the profile (public unless only its author's).
// WHO may comment and vote: the space's roles for `app` (a profile's: anyone); from outside (Discover): read only.
//
//   const cm = await ctx.require("comments");
//   const t = await cm.create({ item, outside, app: "board" })   // item: from items.get
//   host.append(t.el)          // the box to comment, then the thread
//   await t.refresh()          // read again
export async function start(ctx) {
  const [items, directory, person, theme, space, roles, attachments, markdown, mdEditor] = await Promise.all(
    ["items", "directory", "person", "theme", "space", "roles", "attachments", "markdown", "md-editor"].map(n => ctx.require(n)),
  );
  const style = document.createElement("style");
  style.textContent = `
    .cw-cm { display: grid; gap: var(--cw-space-2); }
    .cw-cm button { font: inherit; cursor: pointer; }
    .cw-cm .go { border: 0; border-radius: var(--cw-radius-pill); padding: 6px var(--cw-space-4); background: var(--cw-accent); color: var(--cw-accent-fg); font-weight: 600; }
    .cw-cm .ghost { border: 1px solid var(--cw-accent); border-radius: var(--cw-radius-pill); padding: 6px var(--cw-space-4); background: none; color: var(--cw-accent); font-weight: 600; }
    .cw-cm .reply { display: grid; gap: var(--cw-space-2); }
    .cw-cm .reply .row { display: flex; justify-content: flex-end; align-items: center; gap: var(--cw-space-2); }
    .cw-cm .tree { display: grid; gap: var(--cw-space-2); }
    .cw-cm .meta { display: flex; flex-wrap: wrap; gap: 4px; align-items: baseline; color: var(--cw-muted); font-size: var(--cw-text-xs); }
    .cw-cm .meta .by:hover { text-decoration: underline; cursor: pointer; }
    .cw-cm .acts { display: flex; flex-wrap: wrap; gap: 2px; align-items: center; }
    .cw-cm .acts button { border: 0; background: none; color: var(--cw-muted); font-size: var(--cw-text-xs); font-weight: 600; padding: 4px var(--cw-space-2); border-radius: var(--cw-radius-sm); }
    .cw-cm .acts button:hover { background: var(--cw-hover); color: var(--cw-fg); }
    .cw-cm .votes { display: flex; align-items: center; gap: 2px; }
    .cw-cm .votes button.up[aria-pressed="true"] { color: #ff4500; }
    .cw-cm .votes button.down[aria-pressed="true"] { color: #7193ff; }
    .cw-cm .votes .n { font-weight: 700; font-size: var(--cw-text-xs); }
    .cw-cm .c { display: grid; grid-template-columns: 20px minmax(0, 1fr); column-gap: var(--cw-space-2); }
    .cw-cm .c > .rail { display: flex; justify-content: center; cursor: pointer; }
    .cw-cm .c > .rail::before { content: ""; width: 2px; background: var(--cw-line); border-radius: 1px; }
    .cw-cm .c > .rail:hover::before { background: var(--cw-accent); }
    .cw-cm .c > .body { display: grid; gap: 4px; min-width: 0; }
    .cw-cm .c .text { overflow-wrap: anywhere; line-height: 1.5; margin: 0; font-size: var(--cw-text-sm); }
    .cw-cm .c .kids { display: grid; gap: var(--cw-space-2); margin-top: var(--cw-space-1); }
    .cw-cm .c.folded .text, .cw-cm .c.folded .acts, .cw-cm .c.folded .kids, .cw-cm .c.folded form { display: none; }
    .cw-cm .c .fold { border: 0; background: none; color: var(--cw-muted); font-size: var(--cw-text-xs); padding: 0; }
    .cw-cm .none { color: var(--cw-muted); text-align: center; padding: var(--cw-space-4); margin: 0; }
    .cw-cm .said { color: var(--cw-danger); font-size: var(--cw-text-sm); margin: 0; }`;
  document.head.append(style);
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  const ago = t => {
    const s = Math.max(0, Math.floor((Date.now() - t) / 1000));
    if (s < 60) return "just now";
    for (const [n, u] of [[31536000, "y"], [2592000, "mo"], [86400, "d"], [3600, "h"], [60, "m"]]) if (s >= n) return `${Math.floor(s / n)}${u} ago`;
  };
  const errorTo = said => e => ((said.textContent = e?.message ?? String(e)), (said.hidden = false));

  async function create({ item, outside = null, app = "board" }) {
    const ref = item.ref;
    const me = (await space.account()).id;
    // Where it is: its space (from outside: none here), and where its comments' files are kept.
    const sp = !outside && item.board ? await items.boardOf(item.board.id) : null;
    const pub = sp ? !!item.pub : !item.private;
    const r = sp ? await roles.of(sp).catch(() => null) : null;
    const mayComment = !outside && (!r || r.allows("comment", me, app));
    const mayVote = !outside && (!r || r.allows("vote", me, app));

    const who = did => {
      const n = h("span", { className: "by", textContent: directory.shown(did), onclick: e => (e.stopPropagation(), person.open(e.currentTarget, did, sp ? { space: sp } : {})) });
      directory.name(did).then(t => (n.textContent = t), () => {});
      return n;
    };
    // What someone wrote, as Markdown: its media where it was written, its other files below it.
    const bodyOf = c =>
      h("div", { className: "text" }, c.body ? markdown.render(c.body, c.files, { item: c.ref }) : null, attachments.show((c.files ?? []).filter(f => !markdown.inlined(c.body).has(markdown.keyOf(f)))));
    const editorFor = ({ value = "", files = [], placeholder = "", label = "" } = {}) => {
      const pick = attachments.picker({ space: sp, from: { app }, public: () => pub, media: true });
      pick.preset(files);
      return mdEditor.create({ value, pick, placeholder, label });
    };

    function votes(c) {
      const n = h("span", { className: "n", textContent: String(c.score ?? 0) });
      const up = h("button", { type: "button", className: "up", textContent: "▲", title: "Upvote", ariaPressed: String(c.mine === 1), disabled: !mayVote });
      const down = h("button", { type: "button", className: "down", textContent: "▼", title: "Downvote", ariaPressed: String(c.mine === -1), disabled: !mayVote });
      const cast = v => async e => {
        e.stopPropagation();
        const next = c.mine === v ? 0 : v;
        c.score = (c.score ?? 0) + next - (c.mine ?? 0);
        c.mine = next;
        n.textContent = String(c.score);
        up.ariaPressed = String(next === 1);
        down.ariaPressed = String(next === -1);
        await items.vote(c.ref, next, ref).catch(() => {});
      };
      up.onclick = cast(1);
      down.onclick = cast(-1);
      return h("div", { className: "votes" }, up, n, down);
    }

    // EDIT one's own comment in place: saved as a new version (shown "(edited)").
    function editIn(host, c) {
      const ed = editorFor({ value: c.body ?? "", files: c.files ?? [], label: "Edit" });
      const said = h("p", { className: "said", hidden: true });
      const save = h("button", { type: "button", className: "go", textContent: "Save" });
      save.onclick = async () => {
        if (ed.busy()) return errorTo(said)(new Error("Still sending the files: a moment…"));
        save.disabled = true;
        await items.editItem(c.ref, ed.value().trim(), { files: ed.files() }).then(refresh, errorTo(said));
        save.disabled = false;
      };
      host.replaceChildren(h("div", { className: "reply" }, ed.el, h("div", { className: "row" }, said, h("button", { type: "button", className: "ghost", textContent: "Cancel", onclick: refresh }), save)));
      ed.focus();
    }

    // THE BOX to comment (on the item) or reply (to a comment).
    function replyForm(re, label, cancel = null) {
      const said = h("p", { className: "said", hidden: true });
      const ed = editorFor({ placeholder: re === ref ? "What are your thoughts?" : "Write a reply", label });
      const f = h("form", { className: "reply" }, ed.el, h("div", { className: "row" }, said, cancel ? h("button", { type: "button", className: "ghost", textContent: "Cancel", onclick: cancel }) : null, h("button", { className: "go", textContent: label })));
      f.onsubmit = async e => {
        e.preventDefault();
        said.hidden = true;
        const btn = f.querySelector("button.go");
        if (ed.busy()) return errorTo(said)(new Error("Still sending the files: a moment…"));
        btn.disabled = true;
        try {
          await items.comment(ref, re, ed.value(), { files: ed.files() });
          ed.clear();
          cancel?.();
          await refresh();
        } catch (err) {
          errorTo(said)(err);
        } finally {
          btn.disabled = false;
        }
      };
      return f;
    }

    // A COMMENT and its replies.
    function commentTree(c) {
      const said = h("p", { className: "said", hidden: true });
      const box = h("div", { className: "c" });
      const kids = h("div", { className: "kids" }, ...(c.replies ?? []).map(commentTree));
      const fold = () => box.classList.toggle("folded");
      const count = (function all(x) {
        return (x.replies ?? []).reduce((n, r) => n + 1 + all(r), 0);
      })(c);
      const replyAt = h("div", {});
      const text = h("div", {}, bodyOf(c));
      const acts = h(
        "div",
        { className: "acts" },
        votes(c),
        mayComment
          ? h("button", {
              type: "button",
              textContent: "Reply",
              onclick: () => (replyAt.firstChild ? replyAt.replaceChildren() : replyAt.replaceChildren(replyForm(c.ref, "Reply", () => replyAt.replaceChildren()))),
            })
          : null,
        c.by === me && !outside ? h("button", { type: "button", textContent: "Edit", onclick: () => editIn(text, c) }) : null,
        c.mayRemove ? h("button", { type: "button", textContent: c.by === me ? "Delete" : "Remove", onclick: () => items.remove(c.ref).then(refresh, errorTo(said)) }) : null,
      );
      box.append(
        h("div", { className: "rail", title: "Fold", onclick: fold }),
        h(
          "div",
          { className: "body" },
          h("div", { className: "meta" }, who(c.by), h("time", { textContent: ago(c.at), title: new Date(c.at).toLocaleString() }), c.edited ? h("span", { textContent: "(edited)" }) : null, h("button", { type: "button", className: "fold", textContent: count ? `[–] ${count} more` : "[–]", onclick: fold })),
          text,
          acts,
          said,
          replyAt,
          kids,
        ),
      );
      return box;
    }

    const tree = h("div", { className: "tree" }, theme.loading("Reading the comments…"));
    async function refresh() {
      const cs = await items.thread(ref, { outside }).catch(() => []);
      tree.replaceChildren(...(cs.length ? cs.map(commentTree) : [h("p", { className: "none", textContent: "No comments yet." })]));
    }
    const el = h(
      "div",
      { className: "cw-cm" },
      outside ? h("p", { className: "none", textContent: "Only the space's members comment and vote." }) : mayComment ? replyForm(ref, "Comment") : h("p", { className: "none", textContent: "Comments are closed to you here." }),
      tree,
    );
    refresh().catch(() => {});
    return { el, refresh };
  }
  return { create };
}
