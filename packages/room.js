// ROOM, a component: one conversation's MESSAGES and COMPOSER, the same wherever talk is shown — a server's channel in
// Chat, a direct conversation in Messages. What is shown is `content`'s (the items of the conversation, each in its
// author's feed, live); who wrote it is named by `directory` (their handle, else a short id).
//
//   const room = await ctx.require("room");
//   const r = await room.show(el, conversation, "# general")   // renders into `el`
//   r.close()                                                 // stops following it
export async function start(ctx) {
  const [content, directory, space] = await Promise.all(["content", "directory", "space"].map(n => ctx.require(n)));
  const style = document.createElement("style");
  style.textContent = `
    .cw-room { display: flex; flex-direction: column; min-width: 0; min-height: 0; height: 100%; }
    .cw-room h2 { margin: 0; font-size: 1rem; padding: 12px var(--cw-space-4); border-bottom: 1px solid var(--cw-line);
      overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .cw-room .msgs { flex: 1; overflow-y: auto; margin: 0; padding: var(--cw-space-3) var(--cw-space-4); list-style: none; display: grid;
      align-content: end; gap: var(--cw-space-3); }
    .cw-room .msg .who { font-weight: 600; margin-right: var(--cw-space-2); cursor: pointer; }
    .cw-room .msg .who:hover { text-decoration: underline; }
    .cw-room .msg time { color: var(--cw-muted); font-size: var(--cw-text-xs); }
    .cw-room .msg .text { white-space: pre-wrap; overflow-wrap: anywhere; }
    .cw-room .msg.system { color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .cw-room .msg { position: relative; }
    .cw-room .msg .del { position: absolute; top: 0; right: 0; visibility: hidden; border: 0; background: none; cursor: pointer;
      color: var(--cw-muted); font-size: var(--cw-text-xs); padding: 2px var(--cw-space-1); border-radius: var(--cw-radius-sm); }
    .cw-room .msg:hover .del { visibility: visible; }
    .cw-room .msg .del:hover { background: var(--cw-hover); color: var(--cw-danger); }
    .cw-room .empty { color: var(--cw-muted); text-align: center; margin: auto; padding: var(--cw-space-5); }
    .cw-room .said { color: var(--cw-danger); font-size: var(--cw-text-sm); padding: 0 var(--cw-space-4); margin: 0; }
    .cw-room .compose { padding: var(--cw-space-3) var(--cw-space-4) var(--cw-space-4); }
    .cw-room .compose input { width: 100%; box-sizing: border-box; padding: 10px var(--cw-space-3); border-radius: var(--cw-radius); }`;
  document.head.append(style);
  const el = (tag, props) => Object.assign(document.createElement(tag), props);
  // A name clicked: what can be done with that person (asked for the first time a name is clicked).
  let personing = null;
  const person = { then: f => (personing ??= ctx.require("person")).then(f) };

  async function show(host, conversation, title) {
    const me = (await space.account()).id;
    const box = el("div", { className: "cw-room" });
    const msgs = el("ol", { className: "msgs" });
    const said = el("p", { className: "said", hidden: true });
    const form = el("form", { className: "compose" });
    const input = el("input", { name: "text", autocomplete: "off", placeholder: `Message ${title}`, disabled: true });
    form.append(input);
    box.append(el("h2", { textContent: title }), msgs, said, form);
    host.replaceChildren(box);
    // Until every author's feed has been tried, an empty room is not known to be empty: the theme's placeholder.
    const theme = await ctx.require("theme");
    const waiting = el("li", {});
    waiting.append(theme.loading("Loading messages…"));
    msgs.replaceChildren(waiting);
    let open = true;
    let settled = false;
    const room = await content.in(conversation);
    room.settled.finally(() => {
      settled = true;
      if (open) draw();
    });
    const names = new Map();
    const nameOf = did => {
      if (!did) return "?";
      if (!names.has(did)) {
        names.set(did, null);
        directory.handle(did).then(h => h && open && (names.set(did, h), draw()));
      }
      return directory.shown(did, names.get(did));
    };
    function draw() {
      const items = room.list();
      if (!items.length) {
        if (!settled) return;
        msgs.replaceChildren(el("li", { className: "empty", textContent: `This is the start of ${title}.` }));
        return;
      }
      msgs.replaceChildren(
        ...items.map(m => {
          const li = el("li", { className: m.kind === "system" ? "msg system" : "msg" });
          li.append(
            el("span", { className: "who", textContent: nameOf(m.by), title: m.by ?? "", onclick: e => m.by && person.then(p => p.open(e.currentTarget, m.by, { space: conversation })) }),
            el("time", { textContent: new Date(m.at).toLocaleString() }),
            el("div", { className: "text", textContent: m.body }),
          );
          // Its author removes it; a moderator hides someone else's (content decides which).
          if (m.kind !== "system" && room.mayRemove(m))
            li.append(
              el("button", {
                type: "button",
                className: "del",
                textContent: m.by === me ? "Delete" : "Remove",
                onclick: () =>
                  room.remove(m.id).catch(err => {
                    said.textContent = `Not removed: ${err?.message ?? err}`;
                    said.hidden = false;
                  }),
              }),
            );
          return li;
        }),
      );
      msgs.scrollTop = msgs.scrollHeight;
    }
    room.onChange(() => open && draw());
    draw();
    form.onsubmit = async e => {
      e.preventDefault();
      const text = input.value.trim();
      if (!text) return;
      input.value = "";
      said.hidden = true;
      await room.post("message", text).catch(err => {
        input.value = text;
        said.textContent = `Not sent: ${err?.message ?? err}`;
        said.hidden = false;
      });
    };
    input.disabled = false;
    input.focus();
    // Its messages still arriving (every author's feed tried): whoever shows the room waits on it with it.
    return { close: () => (open = false), settled: room.settled };
  }

  return { show };
}
