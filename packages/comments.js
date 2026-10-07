// COMMENTS, a component: the ONE comment thread under anything posted — a Board post, a video, an audio. Reddit-style:
// each comment with its replies, a rail to fold it, ▲ score ▼, Reply opening the editor under it, Edit and Delete for
// one's own (Remove where a moderator may). Written with the one editor (`md-editor`: rich text or Markdown, images,
// videos and audio inline), shown as Markdown (`markdown`). The comments are `items`' (`thread`, `comment`).
// WHERE its files are kept: the item's space (public as the item is), or the profile (public unless only its author's).
// WHO may comment and vote: the one check (`items.mayWriteOn`) — from outside a space too, where its policy says anyone.
//
//   const cm = await ctx.require("comments");
//   const t = await cm.create({ item, outside, app: "board" })   // item: from items.get
//   host.append(t.el)          // the box to comment, then the thread
//   await t.refresh()          // read again
export async function start(ctx) {
  const [items, directory, person, theme, space, roles, attachments, markdown, mdEditor] = await Promise.all(
    ["items", "directory", "person", "theme", "space", "roles", "attachments", "markdown", "md-editor"].map(n => ctx.require(n)),
  );
  const actionsCap = await ctx.require("actions");
  const style = document.createElement("style");
  style.textContent = `@layer components {

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
    .cw-cm .said { color: var(--cw-danger); font-size: var(--cw-text-sm); margin: 0; }
}`;
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
    // THE ONE CHECK (`roles.mayWrite`, through `items`): the item's own rule, else its space's policy — the same for a
    // personal item (its author's friends or followers, by the credential this person holds).
    // From outside (not in its space): as its public policy says (`items.mayWriteOn`: anyone may, or nobody outside).
    const [mayComment, mayVote] = await Promise.all(["comment", "vote"].map(a => items.mayWriteOn(item, a, { outside }).catch(() => false)));
    // The rule that closed it, to say (a personal item: its own, else its author's policy for what it is).
    const closedBy = item.meta?.write?.comment ?? (!sp && !outside && !String(ref).startsWith("space:") ? roles.personal(item.by).policyIn((await ctx.require("kinds")).policyDomain(item.kind), "comment") : null);

    const who = did => {
      const n = directory.nameEl(did, "span", { className: "by", onclick: e => (e.stopPropagation(), person.open(e.currentTarget, did, sp ? { space: sp } : {})) });
      return n;
    };
    // What someone wrote, as Markdown: its media where it was written, its other files below it.
    const bodyOf = c =>
      h("div", { className: "text" }, c.body ? markdown.render(c.body, c.files, { item: c.ref }) : null, attachments.show((c.files ?? []).filter(f => !markdown.inlined(c.body).has(markdown.keyOf(f)))));
    const editorFor = ({ value = "", files = [], placeholder = "", label = "" } = {}) => {
      const pick = attachments.picker({ space: sp, from: { app }, public: () => pub, media: true, publish: true });
      pick.preset(files);
      return mdEditor.create({ value, pick, placeholder, label });
    };

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
      // SENT ONCE (`theme.action`): the button locked and saying "Sending…" from the first press.
      const send = theme.action(
        f.querySelector("button.go"),
        async () => {
          await items.comment(ref, re, ed.value(), { files: ed.files(), outside });
          ed.clear();
          cancel?.();
          await refresh();
        },
        { busy: "Sending…", done: "Sent ✓" },
      );
      f.onsubmit = e => {
        e.preventDefault();
        said.hidden = true;
        if (ed.busy()) return errorTo(said)(new Error("Still sending the files: a moment…"));
        send().catch(err => errorTo(said)(err));
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
      // Its actions (`actions`: the same as every item's — vote, share, save, hide, edit, remove), Reply first.
      const acts = h(
        "div",
        { className: "acts" },
        actionsCap.bar(c, {
          outside,
          vote: { post: ref, may: mayVote },
          comments: false,
          edit: () => editIn(text, c),
          removed: refresh,
          changed: refresh,
          extra: mayComment
            ? [h("button", { type: "button", textContent: "Reply", onclick: () => (replyAt.firstChild ? replyAt.replaceChildren() : replyAt.replaceChildren(replyForm(c.ref, "Reply", () => replyAt.replaceChildren()))) })]
            : [],
        }),
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
    // HOW MANY: every comment drawn, replies included (`onCount`: the page's heading shows it).
    const counted = [];
    let lastCount = null;
    async function refresh() {
      const cs = await items.thread(ref, { outside }).catch(() => []);
      tree.replaceChildren(...(cs.length ? cs.map(commentTree) : [h("p", { className: "none", textContent: "No comments yet." })]));
      lastCount = all_(cs).length;
      counted.forEach(f => f(lastCount));
    }
    const all_ = xs => (xs ?? []).flatMap(c => [c, ...all_(c.replies)]);
    const el = h(
      "div",
      { className: "cw-cm" },
      mayComment ? replyForm(ref, "Comment") : outside ? h("p", { className: "none", textContent: "Only the space's members comment here." }) : h("p", { className: "none", textContent: ({ friends: `Only ${directory.shown(item.by)}'s friends comment here.`, followers: `Only ${directory.shown(item.by)}'s followers comment here.`, author: "Only its author comments here." })[closedBy] ?? "Comments are closed to you here." }),
      tree,
    );
    refresh().catch(() => {});
    // Read again as what it is read from ARRIVES (each profile's read is bounded: shown within a wait, the rest merged
    // later) — a comment read late shows then. Never under a reply or an edit being written (drawn once it is done).
    let again = null;
    items.onChange(() => {
      if (!el.isConnected || again) return;
      again = setTimeout(() => ((again = null), tree.querySelector("form") ? null : refresh().catch(() => {})), 300);
    });
    return { el, refresh, onCount: f => (counted.push(f), lastCount !== null && f(lastCount)) };
  }
  return { create };
}
