// ACTIVITY, a capability: what is NEW for this person — unread items per conversation and channel, and a NOTIFICATION
// (the shell's: the app, sandboxed, asks it) for a new message not on screen. READ MARKS are the account table `reads`
// (`<container id>` → { at }: the newest item read), so every device of the account agrees. No UI: pages show the
// counts; the room says what is on screen (read as it arrives, never notified).
//
//   const activity = await ctx.require("activity");
//   activity.unread(id)          // unread items in a container (a conversation, a channel)
//   activity.total("chat")       // across this person's own conversations (direct, group): personal Chat's
//   activity.of(serverId, kind?) // across one space's channels and board (kind "chat" | "board": only those)
//   activity.showing(container)  // on screen now: a container, or several (a board's two tables); null: nothing
// A space's BOARD is watched too (its members' and its public table): a new post or comment by someone else counts,
// and notifies ("sam posted in b/Makers#30fe18: …").
//   activity.onChange(fn)
//   activity.recent()            // NOTIFICATIONS: each place with something unread — [{ id, where, count, at, by, route }]
//   activity.mentions()          // MENTIONS of this person (a message, a post, a comment): [{ id, where, at, by, text,
//                                // open() }] newest first; `open()` goes to the very item (a message: shown in its room)
//   activity.mentionsNew() / await activity.mentionsSeen()   // how many since last looked / looked now
export async function start(ctx) {
  const [space, storage, conversation, content, directory, roles] = await Promise.all(["space", "storage", "conversation", "content", "directory", "roles"].map(n => ctx.require(n)));
  const watched = new Map(); // container id → { container, kind, route, serverId, serverName, room, seen }
  const changed = [];
  const fire = () => changed.forEach(f => f());
  const since = Date.now(); // what arrived before this page never notifies
  let me = null;
  let reads = null;
  let onScreen = new Set(); // container ids on screen now

  const readAt = id => {
    try {
      return Number(JSON.parse(reads?.rows().find(r => r.key === id)?.value ?? "{}").at) || 0;
    } catch {
      return 0;
    }
  };
  // Unread: newer than the read mark, by someone else, not a system line.
  const unread = id => {
    const w = watched.get(id);
    if (!w?.room || !me) return 0;
    const at = readAt(id);
    return w.room.list().filter(m => m.at > at && m.by !== me.id && m.kind !== "system").length;
  };
  async function markRead(id) {
    const w = watched.get(id);
    if (!w?.room || !reads) return;
    const last = Math.max(0, ...w.room.list().map(m => m.at));
    if (last > readAt(id)) await reads.put(id, JSON.stringify({ at: last })).catch(() => {});
  }
  const visible = () => document.visibilityState === "visible";

  async function notify(w, m) {
    const who = await directory.name(m.by);
    const title =
      w.kind === "board"
        ? `${who} ${m.kind === "comment" ? "commented" : "posted"} in b/${w.serverName}`
        : w.kind === "chat"
          ? `${who} in #${w.container.name} · ${w.serverName}`
          : w.container.kind === "direct"
            ? who
            : `${who} in ${w.container.name}`;
    const body = w.kind === "board" && m.title ? m.title : m.body;
    parent.postMessage({ __freenet_shell__: true, type: "notification", title, body: body.slice(0, 140), tag: w.route }, "*");
  }

  async function watch(container, kind, extra) {
    if (watched.has(container.id)) return;
    const w = { container, kind, ...extra, seen: new Set() };
    watched.set(container.id, w);
    // Only the NEWEST page (phase 3, Reads): what is new is all this counts — never a whole history read at startup.
    w.room = await content.in(container, { paged: true });
    for (const m of w.room.list()) w.seen.add(m.id);
    if (onScreen.has(container.id) && visible()) markRead(container.id);
    w.room.onChange(() => {
      for (const m of w.room.list()) {
        if (w.seen.has(m.id)) continue;
        w.seen.add(m.id);
        if (m.by === me.id || m.kind === "system" || m.at < since) continue;
        if (onScreen.has(container.id) && visible()) markRead(container.id);
        else if (m.at > readAt(container.id)) notify(w, m).catch(() => {});
      }
      fire();
    });
    fire();
  }

  // WHAT TO WATCH: this person's conversations, and every channel of their servers — looked at again every minute (a
  // conversation joined, a channel added).
  // One space AT A TIME, each once the node is idle — BACKGROUND work: a page's own reads never wait behind it (a slow
  // space holds the next one's count only, never the page).
  async function scan() {
    const node = await ctx.require("node");
    readTold().catch(() => {});
    // The space open first (moved to another: it goes to the front of the next pass).
    const all = await space.mine().catch(() => []);
    for (const sp of [...all.filter(s => s.id === ctx.space), ...all.filter(s => s.id !== ctx.space)]) {
      await node.idle();
      await scanOne(sp).catch(() => {});
    }
  }
  async function scanOne(sp) {
    if (sp.kind === "direct" || sp.kind === "group") watch(sp, "chat", { route: `#/chat/${sp.id}` }).catch(() => {});
    else if (sp.kind === "server") {
      // Its BOARD (both tables), when it uses Board.
      const r = await roles.of(sp).catch(() => null);
      if (r?.apps().includes("board")) {
        const extra = { serverId: sp.id, serverName: space.shown(sp), route: `#/s/${sp.id}/board` };
        watch(space.board(sp), "board", extra).catch(() => {});
        watch(space.board(sp, { pub: true }), "board", extra).catch(() => {});
      }
      const chs = await conversation.channels(sp).catch(() => null);
      if (!chs) return;
      const add = () =>
        chs.list().forEach(c =>
          watch(c, "chat", { serverId: sp.id, serverName: sp.name, route: `#/s/${sp.id}/chat/${c.id.split("/").pop()}` }).catch(() => {}),
        );
      if (!chs.watched) {
        chs.watched = true;
        chs.onChange(add);
      }
      add();
    }
  }

  // Started once someone is logged in (and again for whoever logs in next).
  async function begin() {
    me = await space.account();
    watched.clear();
    if (!me) return fire();
    reads = await storage.table("reads");
    reads.onChange(fire);
    // Notifications: offered once — the shell asks the person, in its own bar.
    parent.postMessage({ __freenet_shell__: true, type: "notification_enable_prompt" }, "*");
    await scan();
  }
  begin().catch(e => ctx.log("activity", { what: e?.message ?? String(e) }));
  addEventListener("craftworks:auth", () => begin().catch(() => {}));
  setInterval(() => me && scan().catch(() => {}), 60000);
  // A notification clicked: its conversation or channel.
  addEventListener("message", e => {
    const d = e.data;
    if (d?.__freenet_shell__ && d.type === "notification_click" && typeof d.tag === "string" && d.tag.startsWith("#/")) location.hash = d.tag;
  });
  // Back in view: what is on screen is read.
  document.addEventListener("visibilitychange", () => visible() && onScreen.forEach(id => markRead(id)));
  // Another page: nothing on screen until it says what it shows.
  addEventListener("hashchange", () => (onScreen = new Set()));

  // WHERE a place is, in words (as a notification names it).
  const whereOf = w =>
    w.kind === "board" ? `b/${w.serverName}` : w.kind === "chat" && w.serverId ? `#${w.container.name} · ${w.serverName}` : w.container.kind === "direct" ? "a direct message" : w.container.name;
  const MENTIONS = "~mentions"; // the reads row: when this person last looked at their mentions
  const mentionOf = text => {
    if (!me) return false;
    const t = String(text ?? "");
    return t.includes(`(person:${me.id})`) || new RegExp(`@[^\\s@#]*#${me.id.replace(/^did:craftec:/, "").slice(0, 6)}(?![1-9A-HJ-NP-Za-km-z])`).test(t);
  };
  // The very ITEM a mention is in, opened: a board's post (a comment: its post's page), or a message in its room.
  const opener = (w, m) => async () => {
    if (w.kind === "board") {
      const items = await ctx.require("items");
      const root = m.kind === "comment" && m.in ? (w.room.list().find(x => x.id === m.in) ?? { id: m.in, kind: "post" }) : m;
      location.hash = items.pageOf(`space:${w.serverId}/${root.id}`, root.kind);
      return;
    }
    (await ctx.require("room")).focus(m.id);
    location.hash = w.route;
  };
  function mentions() {
    const out = [];
    const seen = new Set();
    for (const w of watched.values())
      for (const m of w.room?.list() ?? [])
        if (m.by !== me?.id && m.kind !== "system" && !seen.has(m.id) && mentionOf(m.body) && seen.add(m.id))
          out.push({ id: m.id, ref: w.kind === "board" ? `space:${w.serverId}/${m.kind === "comment" && m.in ? m.in : m.id}` : null, where: whereOf(w), at: m.at, by: m.by, text: m.title || m.body, open: opener(w, m) });
    // TOLD: the notices sent to this person's inbox (`items`: whoever mentions them, wherever) — what no place watched
    // here holds (another's personal post, a space this person is not in), each opened at its item.
    const held = new Set(out.map(x => x.ref).filter(Boolean));
    for (const t of told)
      if (!held.has(t.ref) && !seen.has(`${t.ref}|${t.at}`) && seen.add(`${t.ref}|${t.at}`))
        out.push({
          id: `${t.ref}|${t.at}`,
          ref: t.ref,
          where: t.ref.startsWith("space:") ? "a space's post" : "a post",
          at: t.at,
          by: t.from,
          text: t.text,
          open: async () => {
            const items = await ctx.require("items");
            location.hash = items.pageOf(t.ref, t.itemKind ?? (await items.get(t.ref).catch(() => null))?.kind ?? "post");
          },
        });
    return out.sort((a, b) => b.at - a.at);
  }
  // The MENTION NOTICES in this person's inbox (read with every scan).
  // A notice is anyone's to send: each is CHECKED once against its item — read as this person may — kept only where the
  // item (or a comment on it) is there, by who the notice names, and mentions this person; its text then the item's.
  let told = [];
  const checked = new Map(); // `${ref}|${from}|${at}` → the mention as checked, or null
  async function readTold() {
    const [index, items] = await Promise.all([ctx.require("index"), ctx.require("items")]);
    const got = (await index.inbox().catch(() => [])).filter(x => x?.kind === "mention" && typeof x.ref === "string" && typeof x.from === "string" && x.from !== me?.id && Number(x.at));
    let changedHere = false;
    for (const x of got) {
      const k = `${x.ref}|${x.from}|${x.at}`;
      if (checked.has(k)) continue;
      const it = await items.get(x.ref).catch(() => null);
      const thread = it ? await items.thread(x.ref).catch(() => []) : [];
      const flat = cs => (cs ?? []).flatMap(c => [c, ...flat(c.replies)]);
      const where = [it, ...flat(thread)].filter(Boolean).find(y => y.by === x.from && mentionOf(y.body));
      checked.set(k, where ? { ref: x.ref, itemKind: it.kind, from: x.from, at: where.at ?? x.at, text: where.title || where.body } : null);
      changedHere = true;
    }
    if (changedHere) ((told = [...checked.values()].filter(Boolean)), fire());
  }
  return {
    recent: () =>
      [...watched.values()]
        .map(w => {
          const count = unread(w.container.id);
          if (!count) return null;
          const last = w.room.list().filter(m => m.by !== me?.id && m.kind !== "system").sort((a, b) => b.at - a.at)[0];
          return { id: w.container.id, where: whereOf(w), count, at: last?.at ?? 0, by: last?.by ?? null, text: last ? last.title || last.body : "", route: w.route };
        })
        .filter(Boolean)
        .sort((a, b) => b.at - a.at),
    mentions,
    mentionsNew: () => mentions().filter(x => x.at > readAt(MENTIONS)).length,
    mentionsSeen: async () => {
      const last = Math.max(0, ...mentions().map(x => x.at));
      if (reads && last > readAt(MENTIONS)) await reads.put(MENTIONS, JSON.stringify({ at: last })).catch(() => {});
      fire();
    },
    unread,
    // PERSONAL only (direct and group conversations): a space's channels count in that space (`of`), never here.
    total: kind => [...watched.values()].filter(w => w.kind === kind && !w.serverId).reduce((n, w) => n + unread(w.container.id), 0),
    of: (serverId, kind = null) => [...watched.values()].filter(w => w.serverId === serverId && (!kind || w.kind === kind)).reduce((n, w) => n + unread(w.container.id), 0),
    showing(container) {
      const list = Array.isArray(container) ? container : container ? [container] : [];
      onScreen = new Set(list.map(c => c.id));
      if (visible()) for (const id of onScreen) markRead(id);
    },
    onChange: f => changed.push(f),
  };
}
