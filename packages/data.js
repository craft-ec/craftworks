// DATA, a service: the logged-in ACCOUNT's tables (notes, desktop, …). A table is a TAIL under the account's data key
// (every node of the account holds it, in its identity delegate), labelled `t/<table>`: the account's, not a site's or
// a node's. Any node of the account, and any SITE the person allows, reads and writes the same table: another
// developer's front end, or a second address for the same app, shows the same data. Writing needs the person's GRANT
// for this site and table; the first time, the node itself asks them. The table is followed, so a row written
// elsewhere arrives here as it lands.
//
//   const notes = await (await ctx.require("data")).table("notes");
//   notes.rows()                [{ key, value }]
//   const pins = await data.pins();   pins.has("notes:<id>")   pins.set(ref, on)   pins.onChange(fn)
//   await notes.put(key, value)   await notes.remove(key)   notes.onChange(fn)
export async function start(ctx) {
  const auth = await ctx.require("auth");
  const { core, glue, ask, listen } = await ctx.require("node");
  const tailCode = await ctx.require("tail-wasm");
  const Core = glue.CraftworksCore;
  const bytes = hex => new Uint8Array(hex.match(/../g).map(b => parseInt(b, 16)));

  const tables = new Map(); // id hex -> table
  listen(said => {
    const t = said.kind === "tail" && tables.get(said.id);
    if (t) {
      ctx.log("table changed", { what: `${t.app}: seq ${said.tail.seq}, ${said.tail.rows.length} row(s), pushed by the node` });
      t.took(said.tail.rows);
    }
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

  async function table(app) {
    const s = await auth.session();
    if (!s) throw new Error("nobody is logged in");
    if (!s.data) throw new Error("this node does not hold the account's data key: log in once with the recovery words");
    const idHex = core.tail_open(tailCode, bytes(s.data), app);
    if (tables.has(idHex)) return tables.get(idHex);
    const id = bytes(idHex);
    const name = Core.id_name(id);
    let rows = [];
    const changed = [];
    const t = {
      app,
      took: r => {
        rows = r;
        for (const f of changed) f();
      },
      rows: () => rows,
      onChange: f => changed.push(f),
      put: (key, value) => write(key, value),
      remove: key => write(key, ""),
    };
    tables.set(idHex, t);

    // Leave to write it: the person's grant for this site (the node asks them the first time). Reading needs none;
    // without it the table is read-only here, and a write says why. Asked NOW, beside the read, so a slow read never
    // holds the prompt back.
    let writable = null;
    t.writable = () => writable === true;
    const allowed = () =>
      grant(app).then(ok => {
        writable = ok;
        return ok;
      });
    allowed().catch(() => {});

    // Read it and follow it. None on the network yet: its first write is a PUT. No answer at all (a table nobody has
    // written can take the network longer than the wait): the same as none, so the page opens empty; a first write
    // that the network then refuses resets the table and reads it again.
    async function read() {
      const [, frames] = core.tail_get(id);
      let said;
      try {
        said = await ask(frames, x => (x.kind === "tail" || x.kind === "get-failed") && x.id === idHex, `reading ${app}`, 30000);
      } catch (e) {
        ctx.log("table not found yet", { what: `${app}: ${e.message}; opened empty` });
        said = { kind: "get-failed" };
      }
      if (said.kind === "tail") t.took(said.tail.rows);
      else core.tail_absent(id);
    }
    await read();
    ctx.log("table open", { what: `${app}: ${rows.length} row(s)` });

    // One write: prepared by the core, signed by the identity delegate with the data key, sent as one delta.
    async function write(key, value) {
      if (!(await allowed())) throw new Error(`this app may not change your “${app}”: allow it when your node asks`);
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
        throw new Error(`the node refused the write: ${said.said}`);
      }
      t.took(JSON.parse(core.tail_rows(id)).rows);
    }
    return t;
  }

  // PINS: one table of the account for every pin in every app, keyed by what is pinned — `app:/notes` (an app on
  // the home screen), `notes:<id>` (a note), … — so pinning is one mechanism, not one per app.
  let pinsHandle;
  function pins() {
    pinsHandle ??= table("pins").then(t => ({
      has: ref => t.rows().some(r => r.key === ref),
      refs: prefix => t.rows().map(r => r.key).filter(k => !prefix || k.startsWith(prefix)),
      set: (ref, on) => (on ? t.put(ref, JSON.stringify({ at: Date.now() })) : t.remove(ref)),
      onChange: t.onChange,
    }));
    return pinsHandle;
  }

  return { table, pins };
}
