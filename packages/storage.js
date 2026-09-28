// STORAGE, a capability: the logged-in ACCOUNT's tables (notes, desktop, …). A table is a TAIL under the account's data key
// (every node of the account holds it, in its identity delegate), labelled `t/<table>`: the account's, not a site's or
// a node's. Any node of the account, and any SITE the person allows, reads and writes the same table: another
// developer's front end, or a second address for the same app, shows the same data. Writing needs the person's GRANT
// for this site and table; the first time, the node itself asks them. The table is followed, so a row written
// elsewhere arrives here as it lands.
//
//   const notes = await (await ctx.require("storage")).table("notes");
//   notes.rows()                [{ key, value }]
//   await notes.put(key, value)   await notes.remove(key)   notes.onChange(fn)
export async function start(ctx) {
  const auth = await ctx.require("auth");
  // Who may read and write a table here: the person's grant for this site, and the table's key that comes with it.
  const access = await ctx.require("access");
  const { core, glue, ask, listen } = await ctx.require("node");
  const tailCode = await ctx.require("tail-wasm");
  // A table's tree lives in Block contracts: the core names them from this code.
  core.set_block_code(await ctx.require("block-wasm"));
  const Core = glue.CraftworksCore;
  const bytes = hex => new Uint8Array(hex.match(/../g).map(b => parseInt(b, 16)));

  // Tree blocks go through the ONE door for them (fetch raced against parity, put).
  const blocks = await ctx.require("blocks");

  // A table's VIEW, walked to the end: the tail names the tree's root; a view that needs tree blocks names their
  // Block contracts, fetched (raced, rebuilt if missing) until the rows are all there. Returns the last view:
  // `{ kind: "tail", tail: { rows, … } }`, or `tail-unreadable`.
  async function settle(view, app) {
    for (let round = 0; view.kind === "tail-need"; round++) {
      if (round >= 16) throw new Error(`${app}: the tree did not finish loading`);
      await blocks.fetch(view.id, view.blocks, app);
      view = JSON.parse(core.tail_view(bytes(view.id)));
    }
    if (view.kind === "tail-unreadable") throw new Error(`${app}: ${view.said}`);
    return view;
  }

  const tables = new Map(); // id hex -> Promise<table> (opened once per page)
  const live = new Map(); // id hex -> table, for what the node pushes
  listen(said => {
    const t = (said.kind === "tail" || said.kind === "tail-need") && live.get(said.id);
    if (t)
      settle(said, t.app).then(
        v => {
          ctx.log("table changed", { what: `${t.app}: seq ${v.tail.seq}, ${v.tail.rows.length} row(s), pushed by the node` });
          t.took(v.tail.rows);
        },
        e => ctx.log("table changed", { what: `${t.app}: ${e.message}` }),
      );
  });

  // The account's CATALOG (table `tables`): one row per table that exists. A page reads the catalog, then fetches only
  // the tables it lists; a table it does not list is new, opened empty WITHOUT a read (nothing to find), and listed on
  // its first write. A new account's catalog is created at once; an account from before the catalog has none, so its
  // tables are read as before and listed as they are found.
  let catalogOpen = null;
  function catalog() {
    return (catalogOpen ??= open("tables", { catalog: true }));
  }
  // Listed: true. Not listed in a COMPLETE catalog (one made with the account, so every table since went through
  // it): false, nothing to read. Otherwise (no catalog, or one made later for an account that already had tables):
  // null, unknown, read the table.
  async function listed(name) {
    const c = await catalog();
    if (c.rows().some(r => r.key === name)) return true;
    const self = c.rows().find(r => r.key === "tables");
    const complete = !!self && (() => { try { return JSON.parse(self.value).complete === true; } catch { return false; } })();
    return complete ? false : null;
  }
  async function list(name) {
    const c = await catalog();
    if (c.rows().some(r => r.key === name)) return;
    // No catalog yet (an account from before it): this write creates it, NOT complete, since tables may exist that it
    // does not name.
    if (c.absent && !c.rows().length) await c.put("tables", JSON.stringify({ at: Date.now(), complete: false }));
    await c.put(name, JSON.stringify({ at: Date.now() }));
  }

  const openedNames = new Set(); // the tables this page asked for
  function table(name) {
    openedNames.add(name);
    return open(name, {});
  }

  async function open(app, { catalog: isCatalog = false }) {
    const s = await auth.check();
    if (!s) throw new Error("nobody is logged in");
    if (!s.data) throw new Error("this node does not hold the account's data key: log in once with the recovery words");
    const idHex = core.tail_open(tailCode, bytes(s.data), app);
    if (tables.has(idHex)) return tables.get(idHex);
    const id = bytes(idHex);
    const name = Core.id_name(id);
    let rows = [];
    const changed = [];
    let queue = Promise.resolve(); // one write at a time: each builds on the step before it
    const t = {
      app,
      absent: false,
      took: r => {
        rows = r;
        for (const f of changed) f();
      },
      rows: () => rows,
      onChange: f => changed.push(f),
      put: (key, value) => (queue = queue.catch(() => {}).then(() => write(key, value))),
      remove: key => (queue = queue.catch(() => {}).then(() => write(key, ""))),
    };
    const ready = (async () => {
      // THE TABLE'S KEY: the table is sealed, so reading it needs its key, and the key comes only with the person's
      // grant for this site (asked NOW: the node prompts the first time). No grant: nothing of the table reads here.
      // The catalog's key comes with any grant: listing a table is part of using it.
      const k = await access.key(app, { catalog: isCatalog });
      t.sealed = !!k.key;
      if (k.key) core.tail_seal(id, bytes(k.key));
      else ctx.log("table sealed", { what: `${app}: no key here (${k.why}): nothing of it reads` });
      const known = isCatalog ? (s.fresh ? false : null) : await listed(app);
      if (known === false) {
        core.tail_absent(id);
        t.absent = true;
        ctx.log("table open", { what: `${app}: new, nothing to read` });
      } else {
        await read();
        ctx.log("table open", { what: `${app}: ${rows.length} row(s)` });
        // Found but not listed (an account from before the catalog): list it.
        // Once this site may write it (listing is part of writing).
        if (!isCatalog && known === null && !t.absent)
          allowed()
            .then(ok => ok && list(app))
            .catch(e => ctx.log("not listed", { what: `${app}: ${e.message}` }));
      }
      // A new account's catalog is created now, so the next page finds it.
      if (isCatalog && t.absent && s.fresh) await t.put("tables", JSON.stringify({ at: Date.now(), complete: true }));
      // Rows from before tables were sealed: sealed over now, a batch per step (in the write queue).
      if (legacy > 0) t.sealOld = queue = queue.catch(() => {}).then(sealOld);
      return t;
    })();
    tables.set(idHex, ready);
    live.set(idHex, t);

    let writable = isCatalog ? true : null;
    t.writable = () => writable === true;
    function allowed() {
      if (isCatalog) return Promise.resolve(true);
      return access.allowed(app).then(ok => {
        writable = ok;
        return ok;
      });
    }

    // Read it and follow it. None on the network: its first write is a PUT. No answer at all: the same as none, so
    // the page opens empty; a first write the network then refuses resets the table and reads it again.
    async function read() {
      const [, frames] = core.tail_get(id);
      let said;
      try {
        said = await ask(frames, x => (x.kind === "tail" || x.kind === "tail-need" || x.kind === "get-failed") && x.id === idHex, `reading ${app}`, 30000);
      } catch (e) {
        ctx.log("table not found yet", { what: `${app}: ${e.message}; opened empty` });
        said = { kind: "get-failed" };
      }
      // The tail is there: its tree must load too. A tree that does not is an error, never an empty table (that
      // would write over rows it could not see).
      if (said.kind !== "get-failed") said = await settle(said, app);
      t.absent = said.kind !== "tail";
      if (said.kind === "tail") {
        legacy = said.tail.legacy ?? 0;
        t.info = { rows: said.tail.rows.length, pending: said.tail.pending, flushed: !!said.tail.root, legacy, unreadable: said.tail.unreadable ?? 0 };
        t.took(said.tail.rows);
      }
      else core.tail_absent(id);
    }

    // One write: prepared by the core, signed by the identity delegate with the data key, sent as one delta.
    async function write(key, value) {
      if (!(await allowed())) throw new Error(`this app may not change your “${app}”: allow it when your node asks`);
      // Listed BEFORE it is created: a failure between the two leaves a listed table that is empty, never data the
      // catalog does not name.
      if (t.absent && !isCatalog) await list(app);
      const enc = new TextEncoder();
      try {
        await step(core.tail_prepare(id, enc.encode(key), enc.encode(value)), "saving to");
      } catch (e) {
        // It was there after all (made on another page before it was listed): list it now.
        if (!isCatalog && !t.absent) list(app).catch(err => ctx.log("not listed", { what: `${app}: ${err.message}` }));
        throw e;
      }
      // FLUSH once the tail is long: its rows into the tree, the tail emptied (in this write's turn of the queue).
      if (core.tail_pending(id) >= Core.flush_at()) await flush().catch(e => ctx.log("flush failed", { what: `${app}: ${e.message}` }));
      t.absent = false;
    }
    // ONE STEP of this table, the one way a table moves: the prepared step signed by the identity delegate (the data
    // key), sent (the first as a PUT, then one delta each), and confirmed. Refused anywhere — by the identity or by the
    // node — it never landed: what this page holds is dropped and the table read again, so the next step builds on
    // what the network has.
    async function step(p, doing) {
      const t0 = performance.now();
      const r = await auth.identity.sign(p.params, p.seq, p.valueHash);
      let refused = r.signed ? null : `the identity said ${r.refused ?? JSON.stringify(r)}`;
      let kind = null;
      if (!refused) {
        const [k, frames] = core.tail_commit(id, r.signed);
        kind = k;
        const ok = k === "put" ? "put" : "updated";
        const said = await ask(frames, x => (x.kind === ok && x.key === name) || x.kind === "refused", `${doing} ${app}`, 60000);
        if (said.kind === "refused") refused = `the node said ${said.said}`;
      }
      ctx.log(refused ? "write refused" : "written", {
        what: `${app} seq ${p.seq}${refused ? `: ${refused}` : ` as ${kind === "put" ? "a PUT" : "an UPDATE (one delta)"}`}`,
        ms: Math.round(performance.now() - t0),
      });
      if (refused) {
        core.tail_reset(id);
        await read();
        throw new Error(`the write was refused: ${refused}`);
      }
      await view();
    }

    let legacy = 0;
    async function view() {
      const v = await settle(JSON.parse(core.tail_view(id)), app);
      legacy = v.tail.legacy ?? 0;
      t.info = { rows: v.tail.rows.length, pending: v.tail.pending, flushed: !!v.tail.root, legacy, unreadable: v.tail.unreadable ?? 0 };
      t.took(v.tail.rows);
    }

    // SEAL OVER the plaintext rows from before sealing: 16 rows a step, each sealed and its plaintext deleted in that
    // same step, signed like any write.
    async function sealOld() {
      let n = 0;
      for (let round = 0; round < 64; round++) {
        const p = core.tail_migrate(id, 16);
        if (!p) break;
        await step(p, "sealing");
        n += 1;
      }
      if (n) ctx.log("sealed", { what: `${app}: rows from before sealing, sealed over in ${n} step(s)` });
      if (core.tail_pending(id) >= Core.flush_at()) await flush();
    }

    // FLUSH: the tree's new blocks PUT first (a tail must never name a root whose blocks are not there), then the step
    // naming the new root, signed like any write.
    async function flush() {
      const t0 = performance.now();
      for (let round = 0; round < 16; round++) {
        const f = core.tail_flush(id);
        if (f.need) {
          await settle({ kind: "tail-need", id: idHex, blocks: f.need }, app);
          continue;
        }
        try {
          await blocks.put(f.puts, app);
        } catch (e) {
          core.tail_reset(id);
          await read();
          throw e;
        }
        await step(f, "flushing");
        ctx.log("flushed", { what: `${app}: ${f.puts.length} tree block(s) put, the tail emptied (seq ${f.seq})`, ms: Math.round(performance.now() - t0) });
        return;
      }
      throw new Error("the tree did not finish loading");
    }

    return ready;
  }

  // Every table of the account the catalog lists, as `{ name, rows, pending, flushed, sealed, legacy, unreadable }`
  // (for the Account page's Storage section). Only the tables this site uses, or this page already opened, are opened:
  // another app's table is listed by its name (`{ name, closed: true }`), never asked for — no prompt from a list.
  async function describe() {
    const c = await catalog();
    const names = c.rows().map(r => r.key).filter(n => n !== "tables");
    const out = [];
    for (const name of names) {
      if (!ctx.uses.includes(name) && !openedNames.has(name)) {
        out.push({ name, closed: true });
        continue;
      }
      const t = await table(name).catch(() => null);
      if (t) out.push({ name, sealed: !!t.sealed, ...(t.info ?? { rows: t.rows().length, pending: 0, flushed: false, legacy: 0, unreadable: 0 }) });
    }
    return out;
  }

  return { table, describe };
}
