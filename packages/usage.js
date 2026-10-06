// USAGE, a capability: what this person used, per MONTH and per ITEM — the WATCH TIME of what played (a video, an
// audio), the DATA fetched over the network for it (its files' pieces, as the node delivered them), and how often it
// was OPENED. Kept in the account's own table `usage` (sealed like every table: theirs alone), the first step of the
// rewards (docs/REWARDS.md §3: a subscriber's month splits their fee by it). No money here: "your month", shown in
// Settings → Usage.
//
//   const usage = await ctx.require("usage");
//   usage.track(itemRef, { kind, title }, [fileRoot, …])   // whose files are these (their bytes count for the item)
//   usage.watch(mediaElement, itemRef, { kind, title })      // its playing time counted
//   usage.opened(itemRef, { kind, title })                   // an item's page opened
//   await usage.month("2026-10")                             // [{ ref, kind, title, s, b, n }], most used first
//   await usage.flush()
export async function start(ctx) {
  const storage = await ctx.require("storage");
  const monthOf = (t = Date.now()) => new Date(t).toISOString().slice(0, 7);
  // In memory until written (every 30 s, and when the page goes away): `m:<month>:<ref>` → { kind, title, s, b, n }.
  const pending = new Map();
  const owners = new Map(); // a file's root → { ref, kind, title }
  const add = (ref, info, d) => {
    const k = `m:${monthOf()}:${ref}`;
    const a = pending.get(k) ?? { kind: info?.kind ?? null, title: info?.title ?? null, s: 0, b: 0, n: 0 };
    a.s += d.s ?? 0;
    a.b += d.b ?? 0;
    a.n += d.n ?? 0;
    if (info?.title && !a.title) a.title = info.title;
    pending.set(k, a);
  };
  // DATA: every piece `files` received over the network, by its file's root — to the item that file is (else the file).
  addEventListener("craftworks:bytes", e => {
    const { root, bytes } = e.detail ?? {};
    if (!root || !bytes) return;
    const o = owners.get(root);
    o ? add(o.ref, o, { b: bytes }) : add(`file:${root}`, { kind: "file" }, { b: bytes });
  });
  const track = (ref, info, roots) => roots.filter(Boolean).forEach(r => owners.set(r, { ref, ...info }));
  // WATCH TIME: while it plays, the time it moved (a seek or a stall is not watching: at most 2 s a tick, at its speed).
  const watched = new WeakSet();
  function watch(media, ref, info) {
    if (!media || watched.has(media)) return;
    watched.add(media);
    let last = null;
    media.addEventListener("timeupdate", () => {
      const t = media.currentTime;
      if (last != null && !media.paused && !media.seeking) {
        const d = t - last;
        if (d > 0 && d < 2 * Math.max(1, media.playbackRate)) add(ref, info, { s: d / Math.max(1, media.playbackRate) });
      }
      last = t;
    });
    media.addEventListener("seeking", () => (last = null));
  }
  const opened = (ref, info) => add(ref, info, { n: 1 });
  const parse = v => {
    try {
      return JSON.parse(v ?? "null");
    } catch {
      return null;
    }
  };
  let writing = Promise.resolve();
  const flush = () =>
    (writing = writing.then(async () => {
      if (!pending.size) return;
      const now = [...pending];
      pending.clear();
      const t = await storage.table("usage");
      await t.settled;
      for (const [k, a] of now) {
        const was = parse(t.rows().find(r => r.key === k)?.value) ?? {};
        const s = Math.round(((was.s ?? 0) + a.s) * 10) / 10;
        await t.put(k, JSON.stringify({ kind: a.kind ?? was.kind ?? null, title: a.title ?? was.title ?? null, s, b: (was.b ?? 0) + a.b, n: (was.n ?? 0) + a.n, at: Date.now() }));
      }
    }).catch(e => ctx.log("usage", { what: `not written: ${e?.message ?? e}` })));
  setInterval(flush, 30000);
  addEventListener("pagehide", () => flush());
  addEventListener("visibilitychange", () => document.visibilityState === "hidden" && flush());
  // A MONTH: what was used, most first (watch time, then data), what is still in memory counted too.
  async function month(m = monthOf()) {
    await flush();
    const t = await storage.table("usage");
    await t.settled;
    const p = `m:${m}:`;
    return t
      .rows()
      .filter(r => r.key.startsWith(p) && r.value)
      .map(r => ({ ref: r.key.slice(p.length), ...parse(r.value) }))
      .sort((a, b) => b.s - a.s || b.b - a.b);
  }
  return { track, watch, opened, month, flush, monthOf };
}
