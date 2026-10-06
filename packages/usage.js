// USAGE, a capability: what this person used, per MONTH and per ITEM — the WATCH TIME of what played (a video, an
// audio), the DATA fetched over the network for it (its files' pieces, as the node delivered them), and how often it
// was OPENED. Kept in the account's own table `usage` (sealed like every table: theirs alone), the first step of the
// rewards (docs/REWARDS.md §3: a subscriber's month splits their fee by it). No money here: "your month", shown in
// Settings → Usage.
//
//   const usage = await ctx.require("usage");
//   usage.track(itemRef, { kind, title }, [fileRef | fileRoot, …])   // whose files are these (their bytes count for the
//                                                         // item); a file ref that brings bytes is also KEPT (`keep.watched`)
//   usage.watch(mediaElement, itemRef, { kind, title })      // its playing time counted
//   usage.opened(itemRef, { kind, title })                   // an item's page opened
//   usage.reading(pageElement, itemRef, { kind, title })     // its time on the page counted (an item that does not play)
//   await usage.month("2026-10")                             // [{ ref, kind, title, s, b, n }], most used first
//   await usage.flush()
export async function start(ctx) {
  const storage = await ctx.require("storage");
  const monthOf = (t = Date.now()) => new Date(t).toISOString().slice(0, 7);
  // In memory until written (every 30 s, and when the page goes away): `m:<month>:<ref>` → { kind, title, s, b, n }.
  const pending = new Map();
  const owners = new Map(); // a file's root → { ref, kind, title, file }
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
    // WHAT IT PLAYED, kept for others (rewards step 2): a file of an item that brought bytes here, never one's own.
    if (o?.file && !o.mine) ctx.require("keep").then(k => k.watched(o.file, o.ref, { title: o.title }), () => {});
  });
  const track = (ref, info, roots) =>
    roots.filter(Boolean).forEach(r => (typeof r === "string" ? owners.set(r, { ref, ...info }) : r.root && owners.set(r.root, { ref, ...info, file: r })));
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
  // TIME ON THE PAGE: each second its page is shown, the window focused and the person active (an input in the last
  // 2 minutes: a tab left open is not reading), until the page goes.
  let lastInput = Date.now();
  for (const e of ["pointermove", "pointerdown", "keydown", "wheel", "scroll", "touchstart"]) addEventListener(e, () => (lastInput = Date.now()), { passive: true, capture: true });
  function reading(el, ref, info) {
    let shown = false; // attached after it is made: gone only once it was there
    const t = setInterval(() => {
      if (!el.isConnected) return shown ? clearInterval(t) : undefined;
      shown = true;
      if (document.visibilityState === "visible" && document.hasFocus() && Date.now() - lastInput < 120e3) add(ref, info, { s: 1 });
    }, 1000);
  }
  const parse = v => {
    try {
      return JSON.parse(v ?? "null");
    } catch {
      return null;
    }
  };
  // THE PUBLIC COUNTS (`items.used`: an item's time and data, as its views — owner 10-06, a confidential roll-up
  // later): this person's running total per item (`t:<ref>`, all months), given again once it grew by a minute or
  // 5 MB since it was last given, or when the page goes.
  const STEP = { s: 60, b: 5e6 };
  let writing = Promise.resolve();
  const flush = (leaving = false) =>
    (writing = writing.then(async () => {
      const now = [...pending];
      pending.clear();
      const t = await storage.table("usage");
      await t.settled;
      const totals = new Map();
      for (const [k, a] of now) {
        const was = parse(t.rows().find(r => r.key === k)?.value) ?? {};
        const s = Math.round(((was.s ?? 0) + a.s) * 10) / 10;
        await t.put(k, JSON.stringify({ kind: a.kind ?? was.kind ?? null, title: a.title ?? was.title ?? null, s, b: (was.b ?? 0) + a.b, n: (was.n ?? 0) + a.n, at: Date.now() }));
        const ref = k.slice(k.indexOf(":", 2) + 1);
        if (!ref.startsWith("file:") && (a.s || a.b)) totals.set(ref, { s: (totals.get(ref)?.s ?? 0) + a.s, b: (totals.get(ref)?.b ?? 0) + a.b });
      }
      // Every item's running total; given when it grew enough (or on leaving), in the background.
      const items = await ctx.require("items");
      for (const ref of new Set([...totals.keys(), ...(leaving ? [...given.keys()] : [])])) {
        const k = `t:${ref}`;
        const was = parse(t.rows().find(r => r.key === k)?.value) ?? { s: 0, b: 0, gs: 0, gb: 0 };
        const add = totals.get(ref) ?? { s: 0, b: 0 };
        const cur = { s: Math.round(was.s + add.s), b: was.b + add.b };
        const due = cur.s - was.gs >= STEP.s || cur.b - was.gb >= STEP.b || (leaving && (cur.s > was.gs || cur.b > was.gb));
        await t.put(k, JSON.stringify({ ...cur, gs: due ? cur.s : was.gs, gb: due ? cur.b : was.gb }));
        given.set(ref, true);
        if (due) items.used(ref, cur);
      }
    }).catch(e => ctx.log("usage", { what: `not written: ${e?.message ?? e}` })));
  const given = new Map(); // the items this page counted (given on leaving)
  setInterval(() => pending.size && flush(), 30000);
  addEventListener("pagehide", () => flush(true));
  addEventListener("visibilitychange", () => document.visibilityState === "hidden" && flush(true));
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
  return { track, watch, opened, reading, month, flush, monthOf };
}
