// DATA, a service: the logged-in ACCOUNT's tables (notes, desktop, …). A table is a TAIL under the account's data key
// (every node of the account holds it, in its identity delegate), labelled `t/<table>`: the account's, not a site's or
// a node's. Any node of the account, and any SITE the person allows, reads and writes the same table: another
// developer's front end, or a second address for the same app, shows the same data. Writing needs the person's GRANT
// for this site and table; the first time, the node itself asks them. The table is followed, so a row written
// elsewhere arrives here as it lands.
//
//   const notes = await (await ctx.require("data")).table("notes");
//   notes.rows()                [{ key, value }]
//   await notes.put(key, value)   await notes.remove(key)   notes.onChange(fn)
export async function start(ctx) {
  const auth = await ctx.require("auth");
  const { core, glue, ask, listen } = await ctx.require("node");
  const tailCode = await ctx.require("tail-wasm");
  // A table's tree lives in Block contracts: the core names them from this code.
  core.set_block_code(await ctx.require("block-wasm"));
  const Core = glue.CraftworksCore;
  const bytes = hex => new Uint8Array(hex.match(/../g).map(b => parseInt(b, 16)));

  // A table's VIEW, walked to the end: the tail names the tree's root; a view that needs tree blocks names their
  // Block contracts, each fetched (its answer is the next view), until the rows are all there. Returns the last view:
  // `{ kind: "tail", tail: { rows, … } }`, or `tail-unreadable`.
  // One tree block, by its Block contract id: its answer is the table's next view, or `get-failed`.
  const fetchBlock = (b, app) => {
    const [, frames] = core.frames_get(bytes(b));
    return ask(frames, x => x.block === b || (x.kind === "get-failed" && x.id === b), `reading ${app}'s tree`, 30000).catch(() => ({ kind: "get-failed", id: b }));
  };

  async function settle(view, app) {
    for (let round = 0; view.kind === "tail-need"; round++) {
      if (round >= 16) throw new Error(`${app}: the tree did not finish loading`);
      const t0 = performance.now();
      const answers = await Promise.all(view.blocks.map(b => fetchBlock(b, app)));
      ctx.log("tree read", { what: `${app}: ${view.blocks.length} block(s)`, ms: Math.round(performance.now() - t0) });
      // A block the network no longer has: rebuilt from its group (any k of its k + 8), verified against its id.
      for (const lost of answers.filter(a => a.kind === "get-failed").map(a => a.id)) {
        const t1 = performance.now();
        const group = Array.from(core.tail_repair(bytes(view.id), lost));
        await Promise.all(group.map(b => fetchBlock(b, app)));
        core.tail_rebuild(lost);
        ctx.log("tree repaired", { what: `${app}: block ${lost.slice(0, 12)}… rebuilt from ${group.length} of its group`, ms: Math.round(performance.now() - t1) });
      }
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

  // GRANTS, asked once per page for every kind of data the site uses (its manifest's `uses`) in ONE prompt; a table
  // outside that list is asked for on its own. A "no" stands until the page is opened again, so a refused site does
  // not re-ask on every write; a failure to ask (no answer from the node) is not a "no" and is asked again.
  const asked = new Map(); // table -> Promise<bool>
  function grant(name) {
    if (asked.has(name)) return asked.get(name);
    const list = ctx.uses.includes(name) ? ctx.uses.filter(t => !asked.has(t)) : [name];
    const p = auth.identity.grant(list).then(
      g => {
        ctx.log(g.granted ? "allowed" : "not allowed", { what: `${list.join(", ")}${g.granted ? "" : `: ${g.refused ?? JSON.stringify(g)}`}` });
        return !!g.granted;
      },
      e => {
        for (const t of list) asked.delete(t);
        ctx.log("not allowed", { what: `${list.join(", ")}: ${e.message}` });
        return false;
      },
    );
    for (const t of list) asked.set(t, p);
    return p;
  }

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

  function table(name) {
    return open(name, {});
  }

  async function open(app, { catalog: isCatalog = false }) {
    const s = await auth.session();
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
      // Leave to write it: the person's grant for this site (the node asks them the first time). Reading needs none;
      // without it the table is read-only here, and a write says why. Asked NOW, beside the read, so a slow read never
      // holds the prompt back. The catalog needs none of its own: listing a table is part of using it.
      if (!isCatalog) allowed().catch(() => {});
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
      return t;
    })();
    tables.set(idHex, ready);
    live.set(idHex, t);

    let writable = isCatalog ? true : null;
    t.writable = () => writable === true;
    function allowed() {
      if (isCatalog) return Promise.resolve(true);
      return grant(app).then(ok => {
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
      if (said.kind === "tail") t.took(said.tail.rows);
      else core.tail_absent(id);
    }

    // One write: prepared by the core, signed by the identity delegate with the data key, sent as one delta.
    async function write(key, value) {
      if (!(await allowed())) throw new Error(`this app may not change your “${app}”: allow it when your node asks`);
      // Listed BEFORE it is created: a failure between the two leaves a listed table that is empty, never data the
      // catalog does not name.
      if (t.absent && !isCatalog) await list(app);
      const enc = new TextEncoder();
      const p = core.tail_prepare(id, enc.encode(key), enc.encode(value));
      const t0 = performance.now();
      const r = await auth.identity.sign(p.params, p.seq, p.valueHash);
      if (!r.signed) {
        ctx.log("write refused", { what: `${app} seq ${p.seq}: the identity said ${r.refused ?? JSON.stringify(r)}` });
        core.tail_reset(id);
        await read();
        throw new Error(`the identity would not sign: ${r.refused ?? JSON.stringify(r)}`);
      }
      const [kind, frames] = core.tail_commit(id, r.signed);
      const ok = kind === "put" ? "put" : "updated";
      const said = await ask(frames, x => (x.kind === ok && x.key === name) || x.kind === "refused", `saving to ${app}`, 60000);
      ctx.log(said.kind === "refused" ? "write refused" : "written", {
        what: `${app} seq ${p.seq} as ${kind === "put" ? "a PUT" : "an UPDATE (one delta)"}${said.kind === "refused" ? `: ${said.said}` : ""}`,
        ms: Math.round(performance.now() - t0),
      });
      if (said.kind === "refused") {
        // The step never landed: drop what this page holds and read the table again, so the next write builds on
        // what the network has.
        core.tail_reset(id);
        await read();
        // It was there after all (made on another page before it was listed): list it now.
        if (!isCatalog && !t.absent) list(app).catch(e => ctx.log("not listed", { what: `${app}: ${e.message}` }));
        throw new Error(`the node refused the write: ${said.said}`);
      }
      await view();
      // FLUSH once the tail is long: its rows into the tree, the tail emptied (in this write's turn of the queue).
      if (core.tail_pending(id) >= Core.flush_at()) await flush().catch(e => ctx.log("flush failed", { what: `${app}: ${e.message}` }));
      t.absent = false;
    }
    async function view() {
      const v = await settle(JSON.parse(core.tail_view(id)), app);
      t.took(v.tail.rows);
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
        const puts = await Promise.all(
          f.puts.map(([key, frames]) => ask(frames, x => (x.kind === "put" && x.key === key) || x.kind === "refused", `putting ${app}'s tree`, 60000)),
        );
        const refused = puts.find(p => p.kind === "refused");
        if (refused) {
          core.tail_reset(id);
          await read();
          throw new Error(`a tree block was refused: ${refused.said}`);
        }
        const r = await auth.identity.sign(f.params, f.seq, f.valueHash);
        if (!r.signed) {
          core.tail_reset(id);
          await read();
          throw new Error(`the identity would not sign the flush: ${r.refused ?? JSON.stringify(r)}`);
        }
        const [, frames] = core.tail_commit(id, r.signed);
        const said = await ask(frames, x => (x.kind === "updated" && x.key === name) || x.kind === "refused", `flushing ${app}`, 60000);
        if (said.kind === "refused") {
          core.tail_reset(id);
          await read();
          throw new Error(`the node refused the flush: ${said.said}`);
        }
        ctx.log("flushed", { what: `${app}: ${f.puts.length} tree block(s) put, the tail emptied (seq ${f.seq})`, ms: Math.round(performance.now() - t0) });
        await view();
        return;
      }
      throw new Error("the tree did not finish loading");
    }

    return ready;
  }

  return { table };
}
