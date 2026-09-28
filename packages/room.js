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
    .cw-room .msg .who { font-weight: 600; margin-right: var(--cw-space-2); }
    .cw-room .msg time { color: var(--cw-muted); font-size: var(--cw-text-xs); }
    .cw-room .msg .text { white-space: pre-wrap; overflow-wrap: anywhere; }
    .cw-room .msg.system { color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .cw-room .empty { color: var(--cw-muted); text-align: center; margin: auto; padding: var(--cw-space-5); }
    .cw-room .said { color: var(--cw-danger); font-size: var(--cw-text-sm); padding: 0 var(--cw-space-4); margin: 0; }
    .cw-room .compose { padding: var(--cw-space-3) var(--cw-space-4) var(--cw-space-4); }
    .cw-room .compose input { width: 100%; box-sizing: border-box; padding: 10px var(--cw-space-3); border-radius: var(--cw-radius); }`;
  document.head.append(style);
  const el = (tag, props) => Object.assign(document.createElement(tag), props);

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
    let open = true;
    const room = await content.in(conversation);
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
        msgs.replaceChildren(el("li", { className: "empty", textContent: `This is the start of ${title}.` }));
        return;
      }
      msgs.replaceChildren(
        ...items.map(m => {
          const li = el("li", { className: m.kind === "system" ? "msg system" : "msg" });
          li.append(
            el("span", { className: "who", textContent: nameOf(m.by), title: m.by ?? "" }),
            el("time", { textContent: new Date(m.at).toLocaleString() }),
            el("div", { className: "text", textContent: m.body }),
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
    return { close: () => (open = false) };
  }

  return { show };
}
