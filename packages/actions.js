// ACTIONS, a capability: what a person can DO with an item — the same in every app, on every kind: vote (▲ score ▼),
// react (an emoji, counted — as Chat's), its comments (how many; opening its page), share (its reference: pasted
// anywhere, it embeds), save (yours, any kind; counted), label (your private tags, any kind: `label-menu`), hide (yours:
// `moderation`), flag (your public list, in Discover), edit, remove. Every interaction is a SIGNAL (`signals`). Each shown only where this person
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
  const [items, edge, space, signals, labelUI] = await Promise.all(["items", "edge", "space", "signals", "label-menu"].map(n => ctx.require(n)));
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
    .cw-acts .said { color: var(--cw-danger); font-size: var(--cw-text-sm); }
    .cw-acts .n { color: var(--cw-muted); font-size: var(--cw-text-sm); font-weight: 600; padding: 4px 8px; }
    .cw-reacts { display: inline-flex; flex-wrap: wrap; gap: 4px; align-items: center; position: relative; }
    .cw-reacts .chip { border: 1px solid var(--cw-line); border-radius: var(--cw-radius-pill); padding: 1px 8px; background: none; cursor: pointer; font: inherit; font-size: var(--cw-text-sm); }
    .cw-reacts .chip.mine { border-color: var(--cw-accent); background: var(--cw-hover); }
    .cw-reacts .pick { position: absolute; bottom: 100%; left: 0; z-index: 20; display: flex; gap: 2px; background: var(--cw-surface); border: 1px solid var(--cw-line);
      border-radius: var(--cw-radius-pill); padding: 2px 6px; box-shadow: var(--cw-shadow-lg); }
    .cw-reacts .pick[hidden] { display: none; }
    .cw-reacts .pick button { border: 0; background: none; cursor: pointer; font-size: 1.1rem; padding: 2px 4px; }`;
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

  // REACTIONS (the react signal: `signals`): each emoji with how many gave it — this person's own marked, a click
  // taking it back — and React (the catalog's choices). The same on every kind as on Chat's messages.
  function reacts(it, { outside = null } = {}) {
    const counts = () => it.counts?.react ?? {};
    const mine = () => new Set(it.signaled?.react ?? []);
    const el = h("span", { className: "cw-reacts", onclick: e => e.stopPropagation() });
    const toggle = async emoji => {
      const on = !mine().has(emoji);
      const c = { ...counts(), [emoji]: Math.max(0, (counts()[emoji] ?? 0) + (on ? 1 : -1)) };
      if (!c[emoji]) delete c[emoji];
      it.counts = { ...(it.counts ?? {}), react: c };
      it.signaled = { ...(it.signaled ?? {}), react: on ? [...mine(), emoji] : [...mine()].filter(x => x !== emoji) };
      draw();
      await items.signal(it.ref, "react", emoji, { on, outside: await outsideOf(outside) }).catch(e => ctx.log("actions", { what: `not reacted: ${e.message ?? e}` }));
    };
    const pick = h("span", { className: "pick", hidden: true }, ...signals.of("react").choices.map(emoji => h("button", { type: "button", textContent: emoji, onclick: e => (e.preventDefault(), (pick.hidden = true), toggle(emoji)) })));
    const draw = () =>
      el.replaceChildren(
        ...Object.entries(counts()).map(([emoji, n]) => h("button", { type: "button", className: `chip${mine().has(emoji) ? " mine" : ""}`, textContent: `${emoji} ${short(n)}`, onclick: e => (e.preventDefault(), toggle(emoji)) })),
        h("button", { type: "button", className: "chip", title: "React", textContent: "☺︎+", onclick: e => (e.preventDefault(), (pick.hidden = !pick.hidden)) }),
        pick,
      );
    draw();
    return el;
  }

  // SAVED: any item, one key (`saved:<ref>`).
  const SAVED = ref => `saved:${ref}`;
  const isSaved = ref => pins.has(SAVED(ref));
  const saved = () => pins.refs("saved:").map(k => k.slice(6));
  // A SAVE is two things, written together: this person's own list of what they saved (`pins`: every app's Saved,
  // across every place) and the item's save SIGNAL (`items.signal`: counted where it is). The signal is best effort.
  const setSaved = async (ref, on) => {
    await pins.set(SAVED(ref), on);
    items.signal(ref, "save", on).catch(e => ctx.log("actions", { what: `save not counted: ${e.message ?? e}` }));
  };

  // A COUNT on its button, as TikTok's (`signals`' counted ones: how many people saved, shared): 999, 1.2K, 3.4M.
  const short = n => (n < 1000 ? String(n) : n < 1e6 ? `${(n / 1e3).toFixed(n < 1e4 ? 1 : 0).replace(/\.0$/, "")}K` : `${(n / 1e6).toFixed(1).replace(/\.0$/, "")}M`);
  const countOf = (it, id) => it.counts?.[id] ?? 0;
  // Its count moved at once with this person's own signal (written after).
  const bump = (it, id, on) => {
    it.counts = { ...(it.counts ?? {}), [id]: Math.max(0, countOf(it, id) + (on ? 1 : -1)) };
    it.signaled = { ...(it.signaled ?? {}), [id]: on || undefined };
  };

  // SAVE alone (a card's: a note, a file, a track): ☆ / ★ and how many saved it.
  function save(it) {
    const draw = (on = isSaved(it.ref)) => `${on ? "★" : "☆"}${countOf(it, "save") ? ` ${short(countOf(it, "save"))}` : ""}`;
    const b = h("button", { type: "button", title: "Save: kept in this app's Saved", textContent: draw() });
    b.onclick = async e => {
      e.stopPropagation();
      e.preventDefault();
      const on = !isSaved(it.ref);
      bump(it, "save", on);
      b.textContent = draw(on);
      await setSaved(it.ref, on).catch(() => {});
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
      const n = id => (countOf(it, id) ? ` ${short(countOf(it, id))}` : "");
      const saveLabel = (on = isSaved(it.ref)) => `${on ? "★ Saved" : "☆ Save"}${n("save")}`;
      const save = btn(saveLabel(), async () => {
        const on = !isSaved(it.ref);
        bump(it, "save", on);
        save.textContent = saveLabel(on);
        save.className = on ? "on" : "";
        await setSaved(it.ref, on).catch(fail);
      }, { className: isSaved(it.ref) ? "on" : "" });
      el.replaceChildren(
        ...[
          ...extra,
          reacts(it, { outside }),
          comments && open ? btn(`💬 ${it.comments ?? 0} Comment${it.comments === 1 ? "" : "s"}`, open) : null,
          // SHARE: its reference copied (pasted anywhere, it embeds) — and the share signal, counted where it is.
          btn(`↗ Share${n("share")}`, e =>
            navigator.clipboard.writeText(it.ref).then(() => {
              if (!it.signaled?.share) bump(it, "share", true), items.signal(it.ref, "share", true).catch(() => {});
              e.target.textContent = `Copied — paste it to embed${n("share")}`;
            }, fail),
          ),
          // VIEWS: how many people opened it (counted, never pressed).
          countOf(it, "view") ? h("span", { className: "n", title: "People who opened it", textContent: `👁 ${short(countOf(it, "view"))}` }) : null,
          save,
          // LABEL: this person's private tags on it (any kind, as Notes' — `label-menu`).
          btn("🏷 Label", e => labelUI.menu(e.currentTarget, labelUI.key(it.ref), { title: "Label" })),
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

  return { votes, reacts, bar, save, saved, isSaved, setSaved };
}
