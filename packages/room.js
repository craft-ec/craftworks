// ROOM, a component: one conversation's MESSAGES and COMPOSER, the same wherever talk is shown — a server's channel in
// Chat, a direct or group conversation in Messages. What is shown is `content`'s (the items of the conversation, each
// in its author's feed, live); who wrote it is named by `directory` (their handle, and the start of their id).
// On a message: Reply (a quote above the composer, the reply keeps `re`), React (a few emoji; a chip per emoji, yours
// marked — click to take it back), Edit (your own), Delete (your own) or Remove (a moderator's). Mentions — `@name#abc123`,
// suggested while typing `@` — are marked, and a message that mentions this person stands out. The newest messages
// are shown; earlier ones on asking.
//
//   const room = await ctx.require("room");
//   const r = await room.show(el, conversation, "# general")   // renders into `el`
//   r.close()                                                 // stops following it
export async function start(ctx) {
  const [content, directory, space, attachments, markdown, mdEditor] = await Promise.all(["content", "directory", "space", "attachments", "markdown", "md-editor"].map(n => ctx.require(n)));
  const EMOJI = ["👍", "❤️", "😂", "🎉", "😮", "🙏"];
  const PAGE = 60;
  const MENTION = /@[^\s@#]*#[1-9A-HJ-NP-Za-km-z]{6}/g;
  const style = document.createElement("style");
  style.textContent = `
    .cw-room { display: flex; flex-direction: column; min-width: 0; min-height: 0; height: 100%; }
    .cw-room h2 { margin: 0; font-size: 1rem; padding: 12px var(--cw-space-4); border-bottom: 1px solid var(--cw-line);
      overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .cw-room .msgs { flex: 1; overflow-y: auto; margin: 0; padding: var(--cw-space-3) var(--cw-space-4); list-style: none; display: flex;
      flex-direction: column; gap: var(--cw-space-3); }
    /* Few messages sit at the bottom; many scroll all the way up (an end-aligned grid cut the top off, unreachable). */
    .cw-room .msgs > :first-child { margin-top: auto; }
    .cw-room .earlier { align-self: center; display: block; margin: 0 auto; border: 1px solid var(--cw-line); background: none; color: var(--cw-fg); border-radius: 999px;
      padding: 2px var(--cw-space-3); font-size: var(--cw-text-sm); cursor: pointer; }
    .cw-room .msg { position: relative; padding: 2px var(--cw-space-2); border-radius: var(--cw-radius-sm); }
    .cw-room .msg:hover { background: var(--cw-hover); }
    .cw-room .msg.me-mentioned { background: color-mix(in srgb, var(--cw-accent) 12%, transparent); box-shadow: inset 3px 0 var(--cw-accent); }
    .cw-room .msg .who { font-weight: 600; margin-right: var(--cw-space-2); cursor: pointer; }
    .cw-room .msg .who:hover { text-decoration: underline; }
    .cw-room .msg time, .cw-room .msg .edited { color: var(--cw-muted); font-size: var(--cw-text-xs); }
    .cw-room .msg .edited { margin-left: var(--cw-space-1); }
    .cw-room .msg .text { overflow-wrap: anywhere; }
    .cw-room .msg .text .cw-md p:last-child { margin-bottom: 0; }
    .cw-room .msg .editing .hint { color: var(--cw-muted); font-size: var(--cw-text-xs); }
    .cw-room .msg .text .mention { color: var(--cw-accent); font-weight: 600; }
    .cw-room .msg .quote { color: var(--cw-muted); font-size: var(--cw-text-sm); border-left: 2px solid var(--cw-line); padding-left: var(--cw-space-2);
      margin-bottom: 2px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .cw-room .msg.system { color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .cw-room .msg .tools { position: absolute; top: -10px; right: 4px; display: none; gap: 2px; background: var(--cw-surface);
      border: 1px solid var(--cw-line); border-radius: var(--cw-radius-sm); padding: 2px; box-shadow: var(--cw-shadow); }
    .cw-room .msg:hover .tools, .cw-room .msg .tools.open { display: flex; }
    .cw-room .msg .tools button { border: 0; background: none; cursor: pointer; font-size: var(--cw-text-xs); padding: 2px 6px; border-radius: var(--cw-radius-sm); color: var(--cw-fg); }
    .cw-room .msg .tools button:hover { background: var(--cw-hover); }
    .cw-room .msg .tools button.danger { color: var(--cw-danger); }
    .cw-room .msg .pick { display: flex; gap: 2px; }
    .cw-room .msg .chips { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 4px; }
    .cw-room .msg .chips button { border: 1px solid var(--cw-line); background: var(--cw-surface); border-radius: 999px; padding: 0 8px; cursor: pointer;
      font-size: var(--cw-text-sm); color: var(--cw-fg); }
    .cw-room .msg .chips button.mine { border-color: var(--cw-accent); background: color-mix(in srgb, var(--cw-accent) 15%, var(--cw-surface)); }
    .cw-room .msg .editing { width: 100%; box-sizing: border-box; }
    .cw-room .empty { color: var(--cw-muted); text-align: center; margin: auto; padding: var(--cw-space-5); }
    .cw-room .said { color: var(--cw-danger); font-size: var(--cw-text-sm); padding: 0 var(--cw-space-4); margin: 0; }
    .cw-room .compose { position: relative; padding: var(--cw-space-3) var(--cw-space-4) var(--cw-space-4); }
    .cw-room .compose input { width: 100%; box-sizing: border-box; padding: 10px var(--cw-space-3); border-radius: var(--cw-radius); }
    .cw-room .compose .line { display: flex; align-items: center; gap: 6px; }
    .cw-room .compose .line > .cw-mde { flex: 1; min-width: 0; }
    .cw-room .compose .line > input { flex: 1; min-width: 0; }
    .cw-room .replying { display: flex; gap: var(--cw-space-2); align-items: center; font-size: var(--cw-text-sm); color: var(--cw-muted);
      margin-bottom: var(--cw-space-1); }
    .cw-room .replying span { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .cw-room .replying button { border: 0; background: none; cursor: pointer; color: var(--cw-muted); }
    .cw-room .suggest { position: absolute; bottom: calc(100% - var(--cw-space-2)); left: var(--cw-space-4); margin: 0; padding: 4px; list-style: none;
      background: var(--cw-surface); border: 1px solid var(--cw-line); border-radius: var(--cw-radius-sm); box-shadow: var(--cw-shadow-lg); min-width: 200px; }
    .cw-room .suggest li { padding: 4px 8px; border-radius: var(--cw-radius-sm); cursor: pointer; }
    .cw-room .suggest li:hover, .cw-room .suggest li.on { background: var(--cw-hover); }`;
  document.head.append(style);
  const el = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  // A name clicked: what can be done with that person (asked for the first time a name is clicked).
  let personing = null;
  const person = { then: f => (personing ??= ctx.require("person")).then(f) };

  // `title`: text, or an element (a person's name: `directory.nameEl`, filled in when their card is read).
  async function show(host, conversation, title) {
    const titleEl = typeof title === "string" ? null : title;
    title = titleEl ? titleEl.textContent : title;
    const me = (await space.account()).id;
    const box = el("div", { className: "cw-room" });
    const msgs = el("ol", { className: "msgs" });
    const said = el("p", { className: "said", hidden: true });
    const form = el("form", { className: "compose" });
    const replying = el("div", { className: "replying", hidden: true });
    // THE COMPOSER (`md-editor`, compact): rich text or Markdown, Enter sends, "@" suggests the room's people; FILES
    // with a message (sealed for this conversation's members) — media inline, others under it.
    const pick = attachments.picker({ space: conversation.scope ?? null, from: { app: "chat" }, media: true });
    const suggestPeople = async q => {
      q = q.toLowerCase();
      return (await candidates()).filter(p => p.shown.toLowerCase().startsWith(q) || p.shown.toLowerCase().includes(q)).map(p => p.shown);
    };
    const ed = mdEditor.create({ compact: true, pick, placeholder: `Message ${title}`, label: `Message ${title}`, onSubmit: () => form.requestSubmit(), suggest: suggestPeople });
    ed.disable(true, `Message ${title}`);
    const line = el("div", { className: "line" }, ed.el);
    form.append(replying, line);
    box.append(titleEl ? el("h2", {}, titleEl) : el("h2", { textContent: title }), msgs, said, form);
    host.replaceChildren(box);
    const say = m => ((said.textContent = m ?? ""), (said.hidden = !m));
    // Until every author's feed has been tried, an empty room is not known to be empty: the theme's placeholder.
    const theme = await ctx.require("theme");
    msgs.replaceChildren(el("li", {}, theme.loading("Loading messages…")));
    let open = true;
    let settled = false;
    let limit = PAGE;
    let replyTo = null;
    let editing = null;
    // Read by PAGES: the newest messages first, older ones as asked (phase 3, Reads).
    const room = await content.in(conversation, { paged: true });
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
    // The body, as Markdown (`markdown`: its media where written), its mentions marked.
    const bodyOf = (text, files = []) => {
      const out = el("div", { className: "text" }, markdown.render(text ?? "", files));
      const walker = document.createTreeWalker(out, NodeFilter.SHOW_TEXT);
      const hits = [];
      while (walker.nextNode()) if (MENTION.test(walker.currentNode.textContent) && !walker.currentNode.parentElement.closest("code, a")) hits.push(walker.currentNode);
      for (const n of hits) {
        const parts = [];
        let last = 0;
        for (const m of n.textContent.matchAll(MENTION)) {
          parts.push(n.textContent.slice(last, m.index), el("span", { className: "mention", textContent: m[0] }));
          last = m.index + m[0].length;
        }
        parts.push(n.textContent.slice(last));
        n.replaceWith(...parts);
      }
      return out;
    };
    const mentionsMe = text => [...text.matchAll(MENTION)].some(m => m[0].endsWith(`#${me.replace(/^did:craftec:/, "").slice(0, 6)}`));
    const fail = what => err => say(`${what}: ${err?.message ?? err}`);

    function message(m, byId) {
      const li = el("li", { className: m.kind === "system" ? "msg system" : `msg${m.by !== me && mentionsMe(m.body) ? " me-mentioned" : ""}` });
      if (m.re) {
        const q = byId.get(m.re);
        li.append(el("div", { className: "quote", textContent: q ? `↪ ${nameOf(q.by)}: ${markdown.plain(q.body)}` : "↪ a message not shown" }));
      }
      li.append(
        el("span", { className: "who", textContent: nameOf(m.by), title: m.by ?? "", onclick: e => m.by && person.then(p => p.open(e.currentTarget, m.by, { space: conversation })) }),
        el("time", { textContent: new Date(m.at).toLocaleString() }),
      );
      if (m.edited) li.append(el("span", { className: "edited", textContent: "(edited)" }));
      if (editing === m.id) {
        // EDITING: the same editor, its text and files; Enter saves, Escape leaves it as it was.
        const epick = attachments.picker({ space: conversation.scope ?? null, from: { app: "chat" }, media: true });
        epick.preset(m.files ?? []);
        const save = () => {
          const body = fed.value();
          editing = null;
          if ((body || fed.files().length) && (body !== m.body || fed.files().length !== (m.files ?? []).length)) room.edit(m.id, body, { files: fed.files() }).catch(fail("Not edited"));
          draw();
        };
        const fed = mdEditor.create({ compact: true, pick: epick, value: m.body, label: "Edit the message", onSubmit: save });
        fed.el.addEventListener("keydown", e => e.key === "Escape" && ((editing = null), draw()));
        li.append(el("div", { className: "editing" }, fed.el, el("div", { className: "hint", textContent: "Enter saves · Escape cancels" })));
        queueMicrotask(() => fed.focus());
      } else li.append(bodyOf(m.body, m.files));
      const att = attachments.show((m.files ?? []).filter(f => !markdown.inlined(m.body).has(markdown.keyOf(f))));
      if (att && editing !== m.id) li.append(att);
      // Reactions: a chip per emoji, yours marked; a click takes yours back or adds it.
      const chips = Object.entries(m.reactions ?? {}).filter(([, who]) => who.length);
      if (chips.length)
        li.append(
          el(
            "div",
            { className: "chips" },
            ...chips.map(([emoji, who]) =>
              el("button", {
                type: "button",
                className: who.includes(me) ? "mine" : "",
                textContent: `${emoji} ${who.length}`,
                title: who.map(nameOf).join(", "),
                onclick: () => room.react(m.id, emoji, !who.includes(me)).catch(fail("Not reacted")),
              }),
            ),
          ),
        );
      if (m.kind === "system") return li;
      const tools = el("div", { className: "tools" });
      const pick = el("div", { className: "pick", hidden: true }, ...EMOJI.map(emoji => el("button", { type: "button", textContent: emoji, onclick: () => room.react(m.id, emoji, !(m.reactions?.[emoji] ?? []).includes(me)).catch(fail("Not reacted")) })));
      tools.append(
        el("button", { type: "button", textContent: "Reply", onclick: () => ((replyTo = m), drawReplying(), ed.focus()) }),
        el("button", { type: "button", textContent: "React", onclick: () => ((pick.hidden = !pick.hidden), tools.classList.toggle("open", !pick.hidden)) }),
        pick,
      );
      if (m.by === me) tools.append(el("button", { type: "button", textContent: "Edit", onclick: () => ((editing = m.id), draw()) }));
      // Hidden for you only (`moderation`: the one check); its author removes it; a moderator hides it for everyone.
      if (m.by !== me) tools.append(el("button", { type: "button", textContent: "Hide", onclick: async () => await (await ctx.require("moderation")).lists().then(l => l.hide({ by: m.by, id: m.id })).catch(fail("Not hidden")) }));
      if (room.mayRemove(m)) tools.append(el("button", { type: "button", className: "danger", textContent: m.by === me ? "Delete" : "Remove", onclick: () => room.remove(m.id).catch(fail("Not removed")) }));
      li.append(tools);
      return li;
    }

    function draw() {
      // A conversation (Messages) shows what people said, never lines about it ("started the conversation", "joined").
      const items = room.list().filter(m => !(m.kind === "system" && (conversation.kind === "direct" || conversation.kind === "group")));
      if (!items.length) {
        if (!settled) return;
        msgs.replaceChildren(el("li", { className: "empty", textContent: `This is the start of ${title}.` }));
        return;
      }
      const atEnd = msgs.scrollHeight - msgs.scrollTop - msgs.clientHeight < 40;
      const byId = new Map(items.map(m => [m.id, m]));
      const shown = items.slice(-limit);
      const earlier = items.length - shown.length;
      // EARLIER: those held but not shown, else the next page read.
      const readMore = !earlier && room.hasMore?.();
      msgs.replaceChildren(
        ...(earlier || readMore
          ? [el("li", {}, el("button", { type: "button", className: "earlier", textContent: earlier ? `Show earlier messages (${earlier})` : "Load earlier messages", onclick: async e => {
              if (earlier) return (limit += PAGE), draw();
              e.target.disabled = true;
              e.target.textContent = "Loading…";
              await room.older(PAGE).catch(() => 0);
              limit += PAGE;
              draw();
            } }))]
          : []),
        ...shown.map(m => message(m, byId)),
      );
      if (atEnd || !editing) msgs.scrollTop = msgs.scrollHeight;
    }
    function drawReplying() {
      replying.hidden = !replyTo;
      if (!replyTo) return;
      replying.replaceChildren(
        el("span", { textContent: `Replying to ${nameOf(replyTo.by)}: ${markdown.plain(replyTo.body)}` }),
        el("button", { type: "button", title: "Cancel the reply", textContent: "✕", onclick: () => ((replyTo = null), drawReplying()) }),
      );
    }

    // MENTIONS while typing: `@` and the start of a name → the people of this room (who wrote here, and its members).
    let people = null;
    const candidates = async () => {
      if (!people) {
        const dids = new Set(room.list().map(m => m.by).filter(Boolean));
        const sp = conversation.scope && conversation.scope.kind !== "account" ? conversation.scope : null;
        if (sp) for (const d of await (await ctx.require("conversation")).members(sp).catch(() => [])) dids.add(d);
        dids.delete(me);
        people = await Promise.all([...dids].map(async d => ({ did: d, shown: directory.shown(d, await directory.handle(d)) })));
      }
      return people;
    };

    room.onChange(() => open && editing === null && draw());
    draw();
    form.onsubmit = async e => {
      e.preventDefault();
      const text = ed.value();
      if (pick.busy()) return say("Still sending the files: a moment…");
      const withFiles = pick.files();
      if (!text && !withFiles.length) return;
      const re = replyTo?.id ?? null;
      ed.clear();
      replyTo = null;
      drawReplying();
      say(null);
      await room.post("message", text, { re, files: withFiles }).catch(err => {
        ed.set(text);
        fail("Not sent")(err);
      });
    };
    // Who may post here (the app's setting in the space): the composer says so when this person may not.
    let mayPostNow = false;
    const gate = () => {
      const ok = room.mayPost();
      ed.disable(!ok, ok ? `Message ${title}` : ({ admins: "Only admins post here", owner: "Only the owner posts here", nobody: "Nobody posts here" }[room.postingRule()] ?? "You may not post here"));
      mayPostNow = ok;
    };
    gate();
    room.onChange(gate);
    if (mayPostNow) ed.focus();
    // On screen: read as it arrives (`activity`), until closed.
    const activity = await ctx.require("activity");
    activity.showing(conversation);
    // Its messages still arriving (every author's feed tried): whoever shows the room waits on it with it.
    return {
      close: () => {
        open = false;
        activity.showing(null);
      },
      settled: room.settled,
    };
  }

  return { show };
}
