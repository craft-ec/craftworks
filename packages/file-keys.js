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
  const shared = space.shared;
  const label = sp => (shared(sp) ? space.shown(sp) : "your files");
  const failed = new Map(); // row → the trigger count it failed at
  let triggers = 0;
  // (This person's place for a row comes in the plan: 0 does it; k takes over after k × TAKEOVER of no movement.)
  const { glue } = await ctx.require("node");
  const later = new Map(); // row → its takeover timer
  const seen = new Map(); // row → when this page first saw it due

  // A NEW SALT, when the plan says one is due (`rules::rekey::plan`): written once this table seals with the newest
  // epoch (what the removed cannot read) — else the next trigger (`craftworks:keys`).
  async function rotate(sp, removals) {
    const g = await keys.group(sp).ready();
    await storage.sealNewest();
    const t = await storage.table(space.tableOf(sp, "files"), sp);
    if (!g || (t.sealing && t.sealing.epoch < g.epoch)) return false;
    await files.rotate(sp, removals);
    ctx.log("file keys", { what: `${label(sp)}: a member removed — a new salt; its files re-key` });
    return true;
  }

  // THIS PERSON OUT of a space (removed, or left: not in their list): what they uploaded there adopted into their own
  // space (copied while the old key reads) — an uploader keeps their files.
  async function keepOwn(inside) {
    for (const row of await driveStore.list(null)) {
      if (!row.ref?.in || inside.has(row.ref.in)) continue;
      const ref = await files.adopt(row.ref, null, { app: "drive" });
      await ctx.background("file-keys: driveStore.add", driveStore.add(ref, { folder: row.folder, from: row.from }));
    }
  }

  async function pass(sp) {
    let r = null;
    if (shared(sp)) {
      r = await roles.of.strict(sp);
      await r.settled;
      if (r.left) return false;
    }
    const me = await space.me.strict();
    // THE ONE PLAN (`rules::rekey::plan`, the node's too): a new salt due, and the rows due with this page's turn.
    const st = await files.salt(sp);
    const rows = await files.rows.strict(sp);
    const apps = [...new Set(rows.map(x => (!x.app || x.app === "drive" ? "file" : x.app)))];
    const plan = JSON.parse(glue.rekey_plan(JSON.stringify({
      salt: r ? { n: st.n, removals: st.removals } : null,
      out: r ? r.outActs().map(a => a.did ?? a.by) : [],
      members: r ? r.members().map(m => m.did) : [me],
      gone: r ? r.goneList() : [],
      banned: r ? r.bannedList() : [],
      rows: await Promise.all(rows.map(async x => ({ id: x.id, n: x.n ?? -2, pub: !!x.pub, app: x.app ?? "drive", moved: await files.lastMoved(sp, x) }))),
      anyone: apps.filter(a => !shared(sp) || !!r?.readsAnyone(a)),
      me,
      now: Date.now(),
    })));
    // (A rotation due and not written yet: the rows wait for it — they re-key under the new salt.)
    if (plan.rotate != null && !(await rotate(sp, plan.rotate))) return true;
    for (const { id, pub, rank, moved } of plan.due) {
      const row = rows.find(x => x.id === id);
      const k = `${sp?.id ?? ""}/${id}`;
      if (!row || failed.get(k) === triggers) continue;
      if (rank > 0) {
        // From when it last moved, or when this page first saw it due (a row due since a removal carries an old time).
        if (!seen.has(k)) seen.set(k, Date.now());
        // (The one turn wait: `guards::turn_wait`.)
        const wait = glue.turn_wait(rank, Math.max(seen.get(k), moved), Date.now());
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
        ctx.failed("file keys", e, `${label(sp)}: ${row.id.slice(0, 8)}… not re-keyed yet — ${e.message ?? e}`);
      }
    }
    return true;
  }

  // ONE PASS AT A TIME, every space; a trigger during a pass runs one more after it.
  let running = null;
  let again = false;
  async function all() {
    const list = await space.mine.strict();
    const mine = list.filter(s => s.kind === "server");
    const inside = new Set();
    for (const sp of [null, ...mine]) if (await pass(sp).catch(e => ctx.failed("file keys", e, `${label(sp)}: ${e.message ?? e}`)) !== false && sp) inside.add(sp.id);
    // "Left that space" only from a COMPLETE list: a partial one would move every file of a space not read yet.
    if (list.complete !== false) await keepOwn(inside);
  }
  function kick() {
    triggers += 1;
    if (running) return void (again = true);
    // (A pass whose spaces could not be read stops — reported by its kind — and the next trigger runs it again.)
    running = ctx.background("file-keys: pass", (async () => {
      do {
        again = false;
        await all();
      } while (again);
    })()).finally(() => (running = null));
  }

  // TRIGGERS: each space's table and roles changing; a new epoch; the start (or the login, when nobody is yet).
  const watched = new Set();
  async function watch() {
    const mine = (await space.mine()).filter(s => s.kind === "server");
    for (const sp of [null, ...mine]) {
      const k = sp?.id ?? "";
      if (watched.has(k)) continue;
      watched.add(k);
      const t = shared(sp) ? await storage.table(space.tableOf(sp, "files"), sp) : await storage.table("files");
      t.onChange(kick);
      if (shared(sp)) (await roles.of.strict(sp)).onChange(kick);
    }
  }
  let started = false;
  async function begin() {
    if (started || !(await space.account())) return;
    started = true;
    await watch();
    addEventListener("craftworks:keys", kick);
    (await storage.table("spaces")).onChange(() => watch().then(kick));
    kick();
  }
  addEventListener("craftworks:auth", () => ctx.background("file-keys: begin", begin()));
  await begin().catch(e => ctx.log("file keys", { what: e.message ?? String(e) }));
  return { kick };
}
