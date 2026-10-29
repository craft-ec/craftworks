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
// one. WHO DOES A ROW: every member who sees it due, at once (`rules::rekey::plan`, the node's too) — two doing one
// row make the same fragments (key and secret derived from the file and the salt): no turns, no timer (10-29: a
// 2-minute takeover held a node behind a closed page). Triggers: the page's start, each space's table and roles
// changing, a new epoch (`craftworks:keys`). A row that failed is tried again on the next trigger, not in a loop.
export async function start(ctx) {
  const [space, files, roles, storage, driveStore] = await Promise.all(["space", "files", "roles", "storage", "drive-store"].map(n => ctx.require(n)));
  const shared = space.shared;
  const label = sp => (shared(sp) ? space.shown(sp) : "your files");
  const failed = new Map(); // row → the trigger count it failed at
  let triggers = 0;
  const { glue } = await ctx.require("node");

  // A NEW SALT, when the plan says one is due (`rules::rekey::plan`): written once NOBODY PUT OUT still holds a place
  // in the key group (their removal committed — 10-21: a circle's salt rotated while the unfriended still held the
  // epoch it was sealed under) and this table seals with the newest epoch — else the next trigger (`craftworks:keys`).
  async function rotate(sp, removals) {
    const r = await roles.of.strict(sp);
    await r.settled;
    // (K8.2: the BeeKEM tree — the keys the content is sealed with — no longer holds anyone put out, and this table
    // seals with its newest generation.)
    const tree = (await ctx.require("members")).group(sp);
    for (const did of [...r.goneList(), ...r.bannedList()]) if (await tree.holds(did)) return false;
    // (The generation WRITTEN with — a clean tree's current one, marked: `currentGeneration` — and this table on it.)
    const w = await ctx.optional("file keys: the generation to write with", tree.currentGeneration(), () => null);
    if (!w) return false;
    await storage.sealNewest();
    const t = await storage.table(space.tableOf(sp, "files"), sp);
    if (!t.sealing || t.sealing.epoch !== Number(w[0])) return false;
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
    // What the policy may be asked about for these rows (each row's SUBJECT is chosen by the one rule,
    // `rules::rekey::subject`: its item's domain, else its app) — every name they carry, asked here (the IO).
    const apps = [...new Set(rows.flatMap(x => [x.dom, x.app ?? "drive"].filter(Boolean)).concat("file"))];
    const plan = JSON.parse(glue.rekey_plan(JSON.stringify({
      salt: r ? { n: st.n, removals: st.removals } : null,
      out: r ? r.outActs().map(a => a.did ?? a.by) : [],
      members: r ? r.members().map(m => m.did) : [me],
      gone: r ? r.goneList() : [],
      banned: r ? r.bannedList() : [],
      rows: await Promise.all(rows.map(async x => ({ id: x.id, n: x.n ?? -2, pub: !!x.pub, app: x.app ?? "drive", ...(x.dom ? { dom: x.dom } : {}), moved: await files.lastMoved(sp, x) }))),
      anyone: apps.filter(a => !shared(sp) || !!r?.readsAnyone(a)),
      me,
      now: Date.now(),
    })));
    // (A rotation due and not written yet: the rows wait for it — they re-key under the new salt.)
    if (plan.rotate != null && !(await rotate(sp, plan.rotate))) return true;
    for (const { id, pub } of plan.due) {
      const row = rows.find(x => x.id === id);
      const k = `${sp?.id ?? ""}/${id}`;
      if (!row || failed.get(k) === triggers) continue;
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
