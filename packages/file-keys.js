// FILE KEYS, a capability (ARCHITECTURE §6 Access): keeps every file of every space this person is in on the key its
// access calls for — in the background, triggered, with no job list: the space's table `files` says which rows are DUE.
//
// - A member REMOVED, banned or LEFT: once the removal is in the group and the table writes with the newest epoch, a new salt
//   (`files.rotate`); every row keyed under an older salt is due.
// - A row keyed PUBLIC whose items are no longer read by anyone (the policy narrowed, a post made private): due.
// - A row ADOPTED from another space (keyed by that one's salt): due until copied. A row with no hash: hashed first.
// - This person REMOVED from a space: the files they uploaded there are adopted into their own (and copied, while the
//   old key still reads) — an uploader keeps their files.
//
// Rows only move forward (salt `n` up, public to salted, adopted to owned), so a member on an older view never undoes
// one. WHO DOES A ROW: every member ranks the members for it the same way (a hash of the row and each DID: no claim,
// no message) — the first does it now; the next takes it over when it has not moved for TAKEOVER (a member away),
// and so on — going on from its progress in the space's table. Two members doing one row anyway make the same
// fragments (the key is derived): wasted work, never a wrong result. Triggers: the page's start, each space's table
// and roles changing, a new epoch (`craftworks:keys`), and a takeover falling due. A row that failed is tried again on
// the next trigger, not in a loop.
export async function start(ctx) {
  const [space, files, roles, keys, storage, driveStore] = await Promise.all(["space", "files", "roles", "keys", "storage", "drive-store"].map(n => ctx.require(n)));
  const shared = sp => sp && sp.kind !== "account";
  const label = sp => (shared(sp) ? space.shown(sp) : "your files");
  const failed = new Map(); // row → the trigger count it failed at
  let triggers = 0;
  const TAKEOVER = 2 * 60 * 1000;
  const hash = s => {
    let h = 2166136261;
    for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619) >>> 0;
    return h;
  };
  // This person's place among the members for a row: 0 does it; k takes over after k × TAKEOVER of no movement.
  const rankFor = (id, dids, me) => [...new Set(dids)].sort((a, b) => hash(`${id}|${b}`) - hash(`${id}|${a}`) || (a < b ? -1 : 1)).indexOf(me);
  const later = new Map(); // row → its takeover timer
  const seen = new Map(); // row → when this page first saw it due

  // WHICH ROWS ARE DUE in a space, and what each is due to become (public or salted).
  async function due(sp, r) {
    const st = await files.salt(sp);
    const readsAnyone = row => !shared(sp) || r?.policy(row.app ?? "drive", "read") === "anyone";
    const out = [];
    for (const row of await files.rows(sp)) {
      const pub = !!row.pub && readsAnyone(row);
      const n = row.n ?? -2;
      if (n === -2 || !row.h) out.push({ row, pub }); // adopted, or from before hashes
      else if (n === -1 ? !pub : n < st.n) out.push({ row, pub: false }); // public no longer; an older salt
    }
    return out;
  }

  // A REMOVAL is in: the removed out of the group, this table writing with the newest epoch — then a new salt.
  async function rotateIfRemoved(sp, r) {
    const acts = r.acts().filter(a => a.act === "remove" || a.act === "ban" || a.act === "leave");
    const st = await files.salt(sp);
    if (acts.length <= st.removals) return;
    const gone = new Set(acts.slice(st.removals).map(a => a.did ?? a.by));
    await r.refresh().catch(() => {});
    if (r.members().some(m => gone.has(m.did))) return; // the group has not moved yet: the next trigger
    const g = await keys.group(sp).ready().catch(() => null);
    await storage.sealNewest();
    const t = await storage.table(space.tableOf(sp, "files"), sp);
    if (!g || (t.sealing && t.sealing.epoch < g.epoch)) return; // the newest epoch not here yet: `craftworks:keys`
    await files.rotate(sp, acts.length);
    ctx.log("file keys", { what: `${label(sp)}: a member removed — a new salt; its files re-key` });
  }

  // THIS PERSON OUT of a space (removed, or left: not in their list): what they uploaded there adopted into their own
  // space (copied while the old key reads) — an uploader keeps their files.
  async function keepOwn(inside) {
    for (const row of await driveStore.list(null).catch(() => [])) {
      if (!row.ref?.in || inside.has(row.ref.in)) continue;
      const ref = await files.adopt(row.ref, null, { app: "drive" });
      await driveStore.add(ref, { folder: row.folder, from: row.from }).catch(() => {});
    }
  }

  async function pass(sp) {
    let r = null;
    if (shared(sp)) {
      r = await roles.of(sp);
      await r.settled;
      if (r.left) return false;
      await rotateIfRemoved(sp, r);
    }
    const me = (await space.account())?.id;
    const dids = r ? r.members().map(m => m.did) : [me];
    for (const { row, pub } of await due(sp, r)) {
      const k = `${sp?.id ?? ""}/${row.id}`;
      if (failed.get(k) === triggers) continue;
      const rank = Math.max(0, rankFor(row.id, dids, me));
      if (rank > 0) {
        // From when it last moved: its progress, or when this page first saw it due (a row due since a removal
        // carries an old time of its own).
        if (!seen.has(k)) seen.set(k, Date.now());
        const wait = rank * TAKEOVER - (Date.now() - Math.max(seen.get(k), await files.lastMoved(sp, row)));
        if (wait > 0) {
          if (!later.has(k)) later.set(k, setTimeout(() => (later.delete(k), kick()), wait + 1000));
          continue;
        }
      }
      try {
        await files.recode(sp, row, { pub });
        failed.delete(k);
        ctx.log("file keys", { what: `${label(sp)}: ${row.id.slice(0, 8)}… re-keyed (${pub ? "public" : "sealed for its members"})` });
      } catch (e) {
        failed.set(k, triggers);
        ctx.log("file keys", { what: `${label(sp)}: ${row.id.slice(0, 8)}… not re-keyed yet — ${e.message ?? e}` });
      }
    }
    return true;
  }

  // ONE PASS AT A TIME, every space; a trigger during a pass runs one more after it.
  let running = null;
  let again = false;
  async function all() {
    const mine = (await space.mine().catch(() => [])).filter(s => s.kind === "server");
    const inside = new Set();
    for (const sp of [null, ...mine]) if (await pass(sp).catch(e => ctx.log("file keys", { what: `${label(sp)}: ${e.message ?? e}` })) !== false && sp) inside.add(sp.id);
    await keepOwn(inside);
  }
  function kick() {
    triggers += 1;
    if (running) return void (again = true);
    running = (async () => {
      do {
        again = false;
        await all();
      } while (again);
    })().finally(() => (running = null));
  }

  // TRIGGERS: each space's table and roles changing; a new epoch; the start (or the login, when nobody is yet).
  const watched = new Set();
  async function watch() {
    const mine = (await space.mine().catch(() => [])).filter(s => s.kind === "server");
    for (const sp of [null, ...mine]) {
      const k = sp?.id ?? "";
      if (watched.has(k)) continue;
      watched.add(k);
      const t = shared(sp) ? await storage.table(space.tableOf(sp, "files"), sp) : await storage.table("files");
      t.onChange(kick);
      if (shared(sp)) (await roles.of(sp)).onChange(kick);
    }
  }
  let started = false;
  async function begin() {
    if (started || !(await space.account().catch(() => null))) return;
    started = true;
    await watch();
    addEventListener("craftworks:keys", kick);
    (await storage.table("spaces")).onChange(() => watch().then(kick));
    kick();
  }
  addEventListener("craftworks:auth", () => begin().catch(() => {}));
  await begin().catch(e => ctx.log("file keys", { what: e.message ?? String(e) }));
  return { kick };
}
