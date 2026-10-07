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
//   await usage.contribute(amount, days = 30)                // a contribution (SHADOW: a preview, nothing given)
//   await usage.contributions()                              // [{ at, amount, days }]
//   await usage.level()                                      // { id: "free"|"pro"|"vip", name, perMonth, next }
//   await usage.levelPlan()                                  // [{ id, name, until: "YYYY-MM-DD" }] — the level ahead
//   await usage.mayOpen(level, by)                           // this person may open an item of that level
//   await usage.statement("2026-10")                         // the month so far, each day's share of the
//                                                            // contributions split by that day's use: { month, amount,
//                                                            // days, creators, carriers, unclaimed, network } ({did: n})
export async function start(ctx) {
  const storage = await ctx.require("storage");
  const monthOf = (t = Date.now()) => new Date(t).toISOString().slice(0, 7);
  // In memory until written (every 30 s, and when the page goes away): `m:<month>:<ref>` → { kind, title, s, b, n }.
  const pending = new Map();
  const owners = new Map(); // a file's root → { ref, kind, title, file }
  const dayOf = (t = Date.now()) => new Date(t).toISOString().slice(0, 10);
  // Per MONTH (`m:`, the Usage table) and per DAY (`d:`, what a day's share of a contribution is split by).
  const add = (ref, info, d) => {
    for (const k of [`m:${monthOf()}:${ref}`, `d:${dayOf()}:${ref}`]) {
      const a = pending.get(k) ?? { kind: info?.kind ?? null, title: info?.title ?? null, s: 0, b: 0, n: 0 };
      a.s += d.s ?? 0;
      a.b += d.b ?? 0;
      a.n += d.n ?? 0;
      if (info?.title && !a.title) a.title = info.title;
      pending.set(k, a);
    }
  };
  // DATA: every piece `files` received over the network, by its file's root — to the item that file is (else the file).
  addEventListener("craftworks:bytes", e => {
    const { root, bytes } = e.detail ?? {};
    if (!root || !bytes) return;
    const o = owners.get(root);
    o ? add(o.ref, o, { b: bytes }) : add(`file:${root}`, { kind: "file" }, { b: bytes });
    // PER FILE and DAY too (`r:<day>:<root>`): what pays its keepers (rewards §2: carriers by the data each file brought).
    const rk = `r:${dayOf()}:${root}`;
    const r = pending.get(rk) ?? { item: o?.ref ?? null, s: 0, b: 0, n: 0 };
    r.b += bytes;
    pending.set(rk, r);
    // WHAT IT PLAYED, kept for others (rewards step 2): a file of an item that brought bytes here, never one's own.
    if (o?.file && !o.mine) ctx.require("keep").then(k => k.watched(o.file, o.ref, { title: o.title, kind: o.kind }), () => {});
  });
  // AN ITEM's FILES, from wherever they are opened (a post's attachment, an inline image, Drive's Open, a picture, a
  // book): counted for the item, and KEPT for others once they bring bytes here (owner 10-07: books, Drive and Board
  // too) — not one's own (its author read from the ref, else the item read once).
  const byItem = new Map();
  async function trackItem(itemRef, fileRefs, info = {}) {
    if (!itemRef || !fileRefs?.length) return;
    const ref = String(itemRef);
    let by = info.by ?? (ref.startsWith("did:") ? ref.slice(0, ref.lastIndexOf("/")) : null);
    let it = null;
    if (!by || !info.kind || !info.title) {
      if (!byItem.has(ref)) byItem.set(ref, ctx.require("items").then(i => i.get(ref), () => null).catch(() => null));
      it = await byItem.get(ref);
      by = by ?? it?.by ?? null;
    }
    const me = (await (await ctx.require("space")).account())?.id;
    track(ref, { kind: info.kind ?? it?.kind ?? null, title: info.title ?? it?.title ?? null, mine: !!by && by === me }, fileRefs.filter(f => f && !f.inline && f.root));
  }
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
        await t.put(k, JSON.stringify({ kind: a.kind ?? was.kind ?? null, title: a.title ?? was.title ?? null, ...(a.item ?? was.item ? { item: a.item ?? was.item } : {}), s, b: (was.b ?? 0) + a.b, n: (was.n ?? 0) + a.n, at: Date.now() }));
        const ref = k.slice(k.indexOf(":", 2) + 1);
        if (k.startsWith("m:") && !ref.startsWith("file:") && (a.s || a.b)) totals.set(ref, { s: (totals.get(ref)?.s ?? 0) + a.s, b: (totals.get(ref)?.b ?? 0) + a.b });
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
  // CONTRIBUTIONS (owner 10-07: contribution, not subscription — any amount, whenever): each SPREAD over its days
  // (default 30) and SPLIT PER DAY by that day's use; contributions that overlap add up on the days they share.
  // SHADOW MODE: a contribution here is a preview (table `contrib`), nothing is given or paid.
  const DAYS = 30;
  // THE TOKEN has 8 DECIMALS (owner 10-07): 0.00000001 is its smallest amount; every share is rounded to it.
  const DECIMALS = 8;
  const SPLIT = { creators: 0.6, carriers: 0.3, network: 0.1 };
  async function contribute(amount, days = DAYS) {
    const t = await storage.table("contrib");
    await t.settled;
    const at = Date.now();
    await t.put(`c:${at}`, JSON.stringify({ at, amount: Math.max(0, Number(amount) || 0), days: Math.max(1, Math.round(Number(days) || DAYS)) }));
    // PUBLIC (owner 10-07): the ones still running, on this person's card — anyone computes their level (a badge).
    const running = (await contributions()).filter(c => c.at + c.days * 86400000 > Date.now());
    await (await ctx.require("directory")).setContributions(running);
  }
  async function contributions() {
    const t = await storage.table("contrib");
    await t.settled;
    return t.rows().filter(r => r.key.startsWith("c:") && r.value).map(r => parse(r.value)).filter(c => c?.amount > 0).sort((a, b) => a.at - b.at);
  }
  // ON THE CARD, kept in step (contributions made before they were public, or on another device): the running ones
  // written there when they differ — once, a little after the page is up.
  setTimeout(async () => {
    try {
      const space = await ctx.require("space");
      const me = (await space.account())?.id;
      if (!me) return;
      const directory = await ctx.require("directory");
      const running = (await contributions()).filter(c => c.at + c.days * 86400000 > Date.now());
      const on = (await directory.card(me))?.contrib ?? [];
      const key = l => JSON.stringify(l.map(c => [c.at, c.amount, c.days]).sort());
      if (running.length && key(running) !== key(on)) await directory.setContributions(running);
    } catch (e) {
      ctx.log("usage", { what: `contributions not on the card: ${e?.message ?? e}` });
    }
  }, 15000);
  // THE LEVEL (rewards §5a): FREE, PRO or VIP by what this person gives NOW — today's share of their active
  // contributions, as a month. Thresholds in one place (placeholders, owner 10-07).
  // ONE RULE (`craftworks_gov::level_from`, through the core): the identity delegate admits into a level group by the
  // same. Its thresholds too; only the names shown are here.
  const { glue } = await ctx.require("node");
  const NAMES = { vip: "VIP", pro: "PRO", free: "Free" };
  const LEVELS = JSON.parse(glue.levels()).map(([id, min]) => [id, NAMES[id] ?? id, min]);
  const noon = day => Date.parse(`${day}T12:00:00Z`);
  async function level(day = dayOf()) {
    return levelFrom(await contributions(), day);
  }
  // ANYONE's level from their (public) contributions — what a name's badge shows (`directory.levelOf`).
  function levelFrom(cs, day = dayOf()) {
    const { id, perMonth } = JSON.parse(glue.level_from(JSON.stringify(cs ?? []), noon(day)));
    return { id, name: NAMES[id] ?? id, perMonth, next: LEVELS.slice().reverse().find(([, , min]) => perMonth < min) ?? null };
  }
  // A DAY's share of every contribution active on it (from its day, for its days).
  const shareOn = (cs, day) =>
    cs.reduce((n, c) => {
      const from = dayOf(c.at);
      const until = dayOf(c.at + (c.days - 1) * 86400000);
      return day >= from && day <= until ? n + c.amount / c.days : n;
    }, 0);
  // THE STATEMENT (rewards §2-§3): the month so far, day by day — each day's share split by that day's use: creators by
  // time on others' items, carriers by the data each file brought (among its keepers, never this person), the network
  // 10%. A file with no keeper but this person: its share to its item's CREATOR (owner 10-07), else the day's creators
  // by time; a day with no use at all: its share to the network.
  async function statement(m = monthOf()) {
    await flush();
    const [items, space, keep] = await Promise.all(["items", "space", "keep"].map(n => ctx.require(n)));
    const me = (await space.account())?.id;
    const cs = await contributions();
    const t = await storage.table("usage");
    await t.settled;
    const rows = p => t.rows().filter(r => r.key.startsWith(p) && r.value).map(r => ({ key: r.key.slice(p.length), ...parse(r.value) }));
    const out = { month: m, amount: 0, days: 0, creators: {}, carriers: {}, unclaimed: {}, network: 0, at: Date.now() };
    // Added up EXACTLY (no rounding along the way); each share rounded once at the end, to the token's 8 decimals, and
    // the network takes what is left — so the rows always add up to the amount, to the last 0.00000001.
    const give = (to, who, n) => (to[who] = (to[who] ?? 0) + n);
    const bys = new Map();
    const byOf = ref => {
      if (!bys.has(ref)) bys.set(ref, String(ref).startsWith("did:") ? Promise.resolve(String(ref).slice(0, String(ref).lastIndexOf("/"))) : items.get(ref).then(it => it?.by ?? null, () => null));
      return bys.get(ref);
    };
    const keepersOf = new Map();
    const keepers = root => (keepersOf.has(root) || keepersOf.set(root, keep.keepers(root).catch(() => [])), keepersOf.get(root));
    const today = dayOf();
    const [y, mo] = m.split("-").map(Number);
    for (let d = 1; d <= 31; d++) {
      const day = `${m}-${String(d).padStart(2, "0")}`;
      if (new Date(Date.UTC(y, mo - 1, d)).getUTCMonth() !== mo - 1 || day > today) break;
      const fee = shareOn(cs, day);
      if (!fee) continue;
      out.amount += fee;
      out.days += 1;
      const owned = (await Promise.all(rows(`d:${day}:`).filter(r => !r.key.startsWith("file:") && r.s > 0).map(async r => ({ ...r, by: await byOf(r.key) })))).filter(r => r.by && r.by !== me);
      const time = owned.reduce((n, r) => n + r.s, 0);
      const files = rows(`r:${day}:`).filter(r => r.b > 0);
      const data = files.reduce((n, r) => n + r.b, 0);
      if (!time && !data) {
        out.network += fee;
        continue;
      }
      let unpaid = 0;
      if (time) for (const r of owned) give(out.creators, r.by, (fee * SPLIT.creators * r.s) / time);
      else unpaid += fee * SPLIT.creators;
      const toCreators = async (share, item) => {
        const by = item ? await byOf(item) : null;
        if (by && by !== me) return give(out.unclaimed, by, share);
        if (!time) return (unpaid += share);
        for (const r of owned) give(out.unclaimed, r.by, (share * r.s) / time);
      };
      for (const f of files) {
        const share = (fee * SPLIT.carriers * f.b) / data;
        const ks = (await keepers(f.key)).filter(x => x !== me);
        if (!ks.length) await toCreators(share, f.item);
        else for (const x of ks) give(out.carriers, x, share / ks.length);
      }
      if (!data) await toCreators(fee * SPLIT.carriers, null);
      out.network += fee * SPLIT.network + unpaid;
    }
    const unit = 10 ** DECIMALS;
    const units = n => Math.round(n * unit);
    const total = units(out.amount);
    let given = 0;
    for (const to of [out.creators, out.carriers, out.unclaimed])
      for (const k of Object.keys(to)) {
        const u = units(to[k]);
        given += u;
        to[k] = u / unit;
      }
    out.amount = total / unit;
    out.network = (total - given) / unit;
    return out;
  }
  // THE LEVEL AHEAD: each level and its LAST day, from today, as the contributions run out — [{ id, name, until }].
  async function levelPlan() {
    const cs = await contributions();
    const out = [];
    for (let i = 0; i < 400; i++) {
      const day = dayOf(Date.now() + i * 86400000);
      const { id, name } = levelFrom(cs, day);
      if (out.length && out.at(-1).id === id) out.at(-1).until = day;
      else if (id === "free" && out.length) break;
      else if (id !== "free") out.push({ id, name, until: day });
      else break;
    }
    return out;
  }
  // May this person open an item of LEVEL `lv` ("" / "pro" / "vip")? Its author always may.
  const reaches = (have, need) => glue.level_reaches(have, need);
  async function mayOpen(lv, by = null) {
    if (!lv || !LEVELS.some(([id]) => id === lv)) return true;
    if (by && by === (await (await ctx.require("space")).account())?.id) return true;
    return reaches((await level()).id, lv);
  }
  return { track, trackItem, watch, opened, reading, month, flush, monthOf, dayOf, statement, contribute, contributions, level, levelFrom, levelPlan, mayOpen, reaches, LEVELS };
}
