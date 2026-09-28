// ACTIVITY, a capability: what is NEW for this person — unread items per conversation and channel, and a NOTIFICATION
// (the shell's: the app, sandboxed, asks it) for a new message not on screen. READ MARKS are the account table `reads`
// (`<container id>` → { at }: the newest item read), so every device of the account agrees. No UI: pages show the
// counts; the room says what is on screen (read as it arrives, never notified).
//
//   const activity = await ctx.require("activity");
//   activity.unread(id)          // unread items in a container (a conversation, a channel)
//   activity.total("messages")   // across direct and group conversations; "chat": across servers' channels
//   activity.of(serverId)        // across one server's channels
//   activity.showing(container)  // on screen now (null: nothing)
//   activity.onChange(fn)
export async function start(ctx) {
  const [space, storage, conversation, content, directory] = await Promise.all(["space", "storage", "conversation", "content", "directory"].map(n => ctx.require(n)));
  const watched = new Map(); // container id → { container, kind, route, serverId, serverName, room, seen }
  const changed = [];
  const fire = () => changed.forEach(f => f());
  const since = Date.now(); // what arrived before this page never notifies
  let me = null;
  let reads = null;
  let onScreen = null;

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
      w.kind === "chat" ? `${who} in #${w.container.name} · ${w.serverName}` : w.container.kind === "direct" ? who : `${who} in ${w.container.name}`;
    parent.postMessage({ __freenet_shell__: true, type: "notification", title, body: m.body.slice(0, 140), tag: w.route }, "*");
  }

  async function watch(container, kind, extra) {
    if (watched.has(container.id)) return;
    const w = { container, kind, ...extra, seen: new Set() };
    watched.set(container.id, w);
    w.room = await content.in(container);
    for (const m of w.room.list()) w.seen.add(m.id);
    if (onScreen === container.id && visible()) markRead(container.id);
    w.room.onChange(() => {
      for (const m of w.room.list()) {
        if (w.seen.has(m.id)) continue;
        w.seen.add(m.id);
        if (m.by === me.id || m.kind === "system" || m.at < since) continue;
        if (onScreen === container.id && visible()) markRead(container.id);
        else if (m.at > readAt(container.id)) notify(w, m).catch(() => {});
      }
      fire();
    });
    fire();
  }

  // WHAT TO WATCH: this person's conversations, and every channel of their servers — looked at again every minute (a
  // conversation joined, a channel added).
  async function scan() {
    for (const sp of await space.mine().catch(() => [])) {
      if (sp.kind === "direct" || sp.kind === "group") watch(sp, "messages", { route: `#/messages/${sp.id}` }).catch(() => {});
      else if (sp.kind === "server") {
        const chs = await conversation.channels(sp).catch(() => null);
        if (!chs) continue;
        const add = () =>
          chs.list().forEach(c =>
            watch(c, "chat", { serverId: sp.id, serverName: sp.name, route: `#/chat/${sp.id}/${c.id.split("/").pop()}` }).catch(() => {}),
          );
        if (!chs.watched) {
          chs.watched = true;
          chs.onChange(add);
        }
        add();
      }
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
  document.addEventListener("visibilitychange", () => visible() && onScreen && markRead(onScreen));

  return {
    unread,
    total: kind => [...watched.values()].filter(w => w.kind === kind).reduce((n, w) => n + unread(w.container.id), 0),
    of: serverId => [...watched.values()].filter(w => w.serverId === serverId).reduce((n, w) => n + unread(w.container.id), 0),
    showing(container) {
      onScreen = container?.id ?? null;
      if (onScreen && visible()) markRead(onScreen);
    },
    onChange: f => changed.push(f),
  };
}
