// ACTIONS, a capability: what a person can DO with an item — the same in every app, on every kind: vote (▲ score ▼),
// its comments (how many; opening its page), share (its reference: pasted anywhere, it embeds), save (yours, any kind),
// hide (yours: `moderation`), flag (your public list, in Discover), edit, remove. Each shown only where this person
// may, by THE ONE CHECK (`items.mayWriteOn`: a space's member, from outside by its public policy, a personal item's own
// rule with its credential) — never an app's own.
//
//   const actions = await ctx.require("actions");
//   el.append(actions.votes(item, { outside, row, post, may }))   // post: where its votes are kept (a comment's post)
//   el.append(actions.bar(item, { outside, open, edit, removed, changed, discover }))
//     // open: its page (Comments); edit: fn (its editor); removed / changed: after a removal, a hide, a flag;
//     // extra: the app's own buttons, first
//   actions.save(item)      // Save alone (☆/★: a card's tool)
//   actions.saved()         // the refs this person saved, newest first (any kind) — every app's Saved (`where`)
export async function start(ctx) {
  const [items, edge, space] = await Promise.all(["items", "edge", "space"].map(n => ctx.require(n)));
  const pins = await edge.pins();
  const me = async () => (await space.account()).id;
  const style = document.createElement("style");
  style.textContent = `
    .cw-votes { display: flex; flex-direction: column; align-items: center; gap: 2px; min-width: 32px; }
    .cw-votes.row { flex-direction: row; }
    .cw-votes button { border: 0; background: none; cursor: pointer; color: var(--cw-muted); font-size: 1rem; padding: 2px 6px; border-radius: var(--cw-radius-sm); }
    .cw-votes button:hover { background: var(--cw-hover); }
    .cw-votes button[aria-pressed="true"].up { color: var(--cw-accent); }
    .cw-votes button[aria-pressed="true"].down { color: var(--cw-danger); }
    .cw-votes button:disabled { opacity: .4; cursor: default; }
    .cw-votes .n { font-weight: 700; font-size: var(--cw-text-sm); }
    .cw-acts { display: flex; flex-wrap: wrap; gap: 2px; align-items: center; }
    .cw-acts button { border: 0; background: none; cursor: pointer; color: var(--cw-muted); font: inherit; font-size: var(--cw-text-sm); font-weight: 600;
      padding: 4px 8px; border-radius: var(--cw-radius-sm); }
    .cw-acts button:hover { background: var(--cw-hover); color: var(--cw-fg); }
    .cw-acts button.on { color: var(--cw-accent); }
    .cw-acts .said { color: var(--cw-danger); font-size: var(--cw-text-sm); }`;
  document.head.append(style);
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };

  // `outside`: the public description of the space it is in, when this person is not (a value, or a function that
  // finds it).
  const outsideOf = o => Promise.resolve(typeof o === "function" ? o() : o).catch(() => null);
  // VOTES: ▲ score ▼ — changed here at once, then written; disabled where this person may not vote.
  // `post`: where its votes are kept (a comment's: its post's); `may`: already known (a thread's), else asked.
  function votes(it, { outside = null, row = false, post = it.ref, may = null } = {}) {
    const n = h("span", { className: "n", textContent: String(it.score ?? 0) });
    const up = h("button", { type: "button", className: "up", textContent: "▲", title: "Upvote", ariaPressed: String(it.mine === 1) });
    const down = h("button", { type: "button", className: "down", textContent: "▼", title: "Downvote", ariaPressed: String(it.mine === -1) });
    (may != null ? Promise.resolve(may) : outsideOf(outside).then(o => items.mayWriteOn(it, "vote", { outside: o }))).then(ok => !ok && ((up.disabled = down.disabled = true), (up.title = down.title = "Not yours to vote on")), () => {});
    const cast = v => async e => {
      e.stopPropagation();
      e.preventDefault();
      if (up.disabled) return;
      const next = (it.mine ?? 0) === v ? 0 : v;
      it.score = (it.score ?? 0) + next - (it.mine ?? 0);
      it.mine = next;
      n.textContent = String(it.score);
      up.ariaPressed = String(next === 1);
      down.ariaPressed = String(next === -1);
      await items.vote(it.ref, next, post, { outside: await outsideOf(outside) }).catch(() => {});
    };
    up.onclick = cast(1);
    down.onclick = cast(-1);
    return h("div", { className: `cw-votes${row ? " row" : ""}` }, up, n, down);
  }

  // SAVED: any item, one key (`saved:<ref>`); a video's or a track's saved before (`video:`, `videos:`, `audio:`) too.
  const SAVED = ref => `saved:${ref}`;
  const OLD = ["video:", "videos:", "audio:"];
  const isSaved = ref => pins.has(SAVED(ref)) || OLD.some(p => pins.has(`${p}${ref}`));
  const saved = () => [...pins.refs("saved:").map(k => k.slice(6)), ...OLD.flatMap(p => pins.refs(p).map(k => k.slice(p.length)))];
  async function setSaved(ref, on) {
    await pins.set(SAVED(ref), on);
    if (!on) for (const p of OLD) if (pins.has(`${p}${ref}`)) await pins.set(`${p}${ref}`, false);
  }

  // SAVE alone (a card's: a note, a file, a track).
  function save(it) {
    const b = h("button", { type: "button", title: "Save: kept in this app's Saved", textContent: isSaved(it.ref) ? "★" : "☆" });
    b.onclick = async e => {
      e.stopPropagation();
      e.preventDefault();
      const on = !isSaved(it.ref);
      await setSaved(it.ref, on).catch(() => {});
      b.textContent = on ? "★" : "☆";
    };
    return b;
  }

  // THE BAR: Comments · Share · Save · Hide · Flag · Edit · Delete/Remove — each where it applies.
  // `extra`: the app's own, first (a comment's Reply).
  function bar(it, { outside = null, open = null, edit = null, removed = null, changed = null, discover = false, comments = true, extra = [] } = {}) {
    const said = h("span", { className: "said", hidden: true });
    const fail = e => ((said.textContent = e?.message ?? String(e)), (said.hidden = false));
    const btn = (textContent, run, extra = {}) => h("button", { type: "button", textContent, ...extra, onclick: e => (e.stopPropagation(), e.preventDefault(), run(e)) });
    const el = h("div", { className: "cw-acts" });
    me().then(self => {
      const mine = it.by === self;
      const save = btn(isSaved(it.ref) ? "Saved ✓" : "Save", async () => {
        const on = !isSaved(it.ref);
        await setSaved(it.ref, on).catch(fail);
        save.textContent = on ? "Saved ✓" : "Save";
        save.className = on ? "on" : "";
      }, { className: isSaved(it.ref) ? "on" : "" });
      el.replaceChildren(
        ...[
          ...extra,
          comments && open ? btn(`💬 ${it.comments ?? 0} Comment${it.comments === 1 ? "" : "s"}`, open) : null,
          btn("Share", e => navigator.clipboard.writeText(it.ref).then(() => (e.target.textContent = "Copied — paste it to embed"), fail)),
          save,
          // HIDE: for you only (`moderation`: the one check every read asks).
          !mine ? btn("Hide", () => ctx.require("moderation").then(m => m.lists()).then(l => l.hide({ by: it.by, id: it.id ?? String(it.ref).split("/").pop() })).then(() => changed?.(), fail)) : null,
          // FLAG (Discover): on your public moderation list — for you and whoever applies it.
          discover && !mine ? btn("Flag post", () => ctx.require("moderation").then(m => m.lists()).then(l => l.flag("post", it.ref)).then(() => changed?.(), fail)) : null,
          discover && !mine ? btn("Flag author", () => ctx.require("moderation").then(m => m.lists()).then(l => l.flag("person", it.by)).then(() => changed?.(), fail)) : null,
          mine && edit && !discover ? btn("Edit", edit) : null,
          it.mayRemove
            ? btn(mine ? "Delete" : "Remove", async e => {
                if (e.target.dataset.armed !== "1") return ((e.target.dataset.armed = "1"), (e.target.textContent = `Confirm: ${mine ? "delete" : "remove"}`));
                await items.remove(it.ref).then(() => removed?.(), fail);
              })
            : null,
          said,
        ].filter(Boolean),
      );
    });
    return el;
  }

  return { votes, bar, save, saved, isSaved, setSaved };
}
