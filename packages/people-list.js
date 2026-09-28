// PEOPLE LIST, a component: a person's RELATIONS to others — friend requests waiting (answered here), friends,
// following, hidden, blocked — each name opening the one `person` menu (which changes them). Any page shows it: Messages
// (Friends), Account (People). UI only: the relations are `edge.people`'s, requests `conversation`'s, names
// `directory`'s.
//
//   const list = await ctx.require("people-list");
//   const shown = await list.show(host)   // { close }
export async function start(ctx) {
  const [edge, conversation, directory, person, theme] = await Promise.all(["edge", "conversation", "directory", "person", "theme"].map(n => ctx.require(n)));
  const style = document.createElement("style");
  style.textContent = `
    .cw-people { display: grid; gap: var(--cw-space-4); padding: var(--cw-space-3); align-content: start; }
    .cw-people h4 { margin: 0 0 var(--cw-space-2); font-size: var(--cw-text-xs); letter-spacing: .08em; text-transform: uppercase; color: var(--cw-muted); }
    .cw-people ul { list-style: none; margin: 0; padding: 0; display: grid; gap: 2px; }
    .cw-people li { display: flex; align-items: center; gap: var(--cw-space-2); padding: 6px var(--cw-space-2); border-radius: var(--cw-radius-sm); }
    .cw-people li .n { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; cursor: pointer; }
    .cw-people li .n:hover { text-decoration: underline; }
    .cw-people li:hover { background: var(--cw-hover); }
    .cw-people button { font: inherit; border: 1px solid var(--cw-line); background: none; color: var(--cw-fg); border-radius: var(--cw-radius-sm); padding: 2px var(--cw-space-2); cursor: pointer; }
    .cw-people button.yes { background: var(--cw-accent); color: var(--cw-accent-fg); border-color: transparent; }
    .cw-people .none { color: var(--cw-muted); font-size: var(--cw-text-sm); margin: 0; }`;
  document.head.append(style);
  const el = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids);
    return e;
  };
  const SECTIONS = [
    ["friend", "Friends", "No friends yet — add one from anyone's name."],
    ["asked", "Requests you sent", null],
    ["follow", "Following", "You follow nobody yet."],
    ["hide", "Hidden", null],
    ["block", "Blocked", null],
  ];

  async function show(host) {
    const people = await edge.people();
    const box = el("div", { className: "cw-people" }, theme.loading("Loading people…"));
    host.replaceChildren(box);
    let open = true;
    const row = (did, name, ...extra) =>
      el("li", { title: did }, el("span", { className: "n", textContent: name, onclick: e => person.open(e.currentTarget, did) }), ...extra);
    async function draw() {
      const asking = await conversation.friendRequests().catch(() => []);
      const all = [...new Set([...asking, ...SECTIONS.flatMap(([rel]) => people.list(rel))])];
      const names = new Map(await Promise.all(all.map(async d => [d, await directory.name(d)])));
      if (!open) return;
      const parts = [];
      if (asking.length)
        parts.push(
          el("section", {}, el("h4", { textContent: "Friend requests" }), el("ul", {}, ...asking.map(d => {
            const yes = el("button", { type: "button", className: "yes", textContent: "Accept", onclick: () => conversation.answerFriend(d, true).then(draw) });
            const no = el("button", { type: "button", textContent: "Decline", onclick: () => conversation.answerFriend(d, false).then(draw) });
            return row(d, names.get(d), yes, no);
          }))),
        );
      for (const [rel, title, none] of SECTIONS) {
        const list = people.list(rel);
        if (!list.length && !none) continue;
        parts.push(el("section", {}, el("h4", { textContent: title }), list.length ? el("ul", {}, ...list.map(d => row(d, names.get(d)))) : el("p", { className: "none", textContent: none })));
      }
      box.replaceChildren(...parts);
    }
    people.onChange(() => open && draw());
    await draw();
    return { close: () => (open = false) };
  }

  return { show };
}
