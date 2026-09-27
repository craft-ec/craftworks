// DATA, a service: the logged-in ACCOUNT's tables, one per app. A table is a TAIL under the account's data key (every
// node of the account holds it, in its identity delegate), labelled `site id ‖ app name`: the account's, not a node's,
// so any of its nodes reads and writes it, and losing a node loses nothing. The table is followed, so a row written on
// another node arrives here as it lands.
//
//   const notes = await (await ctx.require("data")).table("notes");
//   notes.rows()                [{ key, value }]
//   await notes.put(key, value)   await notes.remove(key)   notes.onChange(fn)
export async function start(ctx) {
  const auth = await ctx.require("auth");
  const { core, glue, ask, listen } = await ctx.require("node");
  const tailCode = await ctx.require("tail-wasm");
  const Core = glue.CraftworksCore;
  // This site: the one the page was served from (the id the node names as the asking app).
  const site = Core.id_bytes(location.pathname.split("/")[4]);
  const bytes = hex => new Uint8Array(hex.match(/../g).map(b => parseInt(b, 16)));

  const tables = new Map(); // id hex -> table
  listen(said => {
    const t = said.kind === "tail" && tables.get(said.id);
    if (t) t.took(said.tail.rows);
  });

  async function table(app) {
    const s = await auth.session();
    if (!s) throw new Error("nobody is logged in");
    if (!s.data) throw new Error("this node does not hold the account's data key: log in once with the recovery words");
    const idHex = core.tail_open(tailCode, bytes(s.data), site, app);
    if (tables.has(idHex)) return tables.get(idHex);
    const id = bytes(idHex);
    const name = Core.id_name(id);
    let rows = [];
    const changed = [];
    const t = {
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

    // Read it and follow it. None on the network yet: its first write is a PUT.
    async function read() {
      const [, frames] = core.tail_get(id);
      const said = await ask(frames, x => (x.kind === "tail" || x.kind === "get-failed") && x.id === idHex, `reading ${app}`, 30000);
      if (said.kind === "tail") t.took(said.tail.rows);
      else core.tail_absent(id);
    }
    await read();
    ctx.log("table open", { what: `${app}: ${rows.length} row(s)` });

    // One write: prepared by the core, signed by the identity delegate with the data key, sent as one delta.
    async function write(key, value) {
      const enc = new TextEncoder();
      const p = core.tail_prepare(id, enc.encode(key), enc.encode(value));
      const r = await auth.identity.sign(p.params, p.seq, p.valueHash);
      if (!r.signed) throw new Error(`the identity would not sign: ${r.refused ?? JSON.stringify(r)}`);
      const [kind, frames] = core.tail_commit(id, r.signed);
      const ok = kind === "put" ? "put" : "updated";
      const said = await ask(frames, x => (x.kind === ok && x.key === name) || x.kind === "refused", `saving to ${app}`, 60000);
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

  return { table };
}
