// CO-EDITING, a capability (ARCHITECTURE §7 "Live co-editing"): an item of kind `doc` whose text is the MERGE of every
// editor's changes (`doc-glue`: Automerge — any order, duplicates harmless, every reader reaches the same document).
// Each editor's device writes its changes as rows of its OWN feed in the item's table `doc-<id>` (`storage`: every
// writer its own feed), a burst of keystrokes batched into one row; each row keyed by its bytes' hash (written twice:
// one row). WHO MAY WRITE: the item's maker, or whoever its place's `edit` policy allowed at the row's time — the one
// check every collaborative edit has (`roles.mayWrite`); a row anyone else wrote is passed over by every reader
// (`storage`'s admission). Shared with chosen people as any item is (`share`: editors, viewers).
//
//   const codoc = await ctx.require("codoc");
//   const ref = await codoc.create({ board, title, audience })   // the item, and its maker's first change
//   await codoc.state(ref)                     // { text, marks, mayEdit, v, title } — `v` grows with every change seen
//   await codoc.splice(ref, pos, del, insert)  // an edit (positions: UTF-16 units, a text field's) → { text, v }
//   await codoc.mark(ref, start, end, name, value, on)
//   await codoc.set(ref, key, value)   // a cell (a sheet's `c:A1`, a slide's `s:<id>:title`; null: cleared); state's `cells`
//   await codoc.snapshot(ref)   await codoc.seed(ref, snapshot)   // the document moved with its item (`share.moveTo`)
// Every call takes and gives plain values: an app in its own frame reaches it through the host.
export async function start(ctx) {
  const [items, storage, space, roles, node] = await Promise.all(["items", "storage", "space", "roles", "node"].map(n => ctx.require(n)));
  const glue = await ctx.require("doc-glue");
  await glue.default({ module_or_path: await ctx.require("doc-wasm") });
  // THIS PAGE's ACTOR: its own (every concurrent writer one of its own — two devices of one account too).
  const actor = crypto.getRandomValues(new Uint8Array(16));
  // ITS KINDS: the catalog's co-edited ones (`kinds.coedited`: a document — its text and marks —, a spreadsheet and a
  // presentation — their CELLS).
  const K = await ctx.require("kinds");
  const KINDS = { has: kind => K.coedited(kind) };
  const spaceOf = ref => (String(ref).startsWith("space:") ? String(ref).slice(6, String(ref).indexOf("/")) : null);
  const b64 = bytes => btoa(Array.from(bytes, c => String.fromCharCode(c)).join(""));
  const unb64 = s => Uint8Array.from(atob(s), c => c.charCodeAt(0));
  const keyOf = async bytes => node.hex(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))).slice(0, 32);
  const parse = s => {
    try {
      return JSON.parse(s);
    } catch {
      return null;
    }
  };

  const open_ = new Map(); // ref → Promise<{ doc, t, seen, v, it, r }>
  function open(ref) {
    if (!open_.has(ref))
      open_.set(
        ref,
        (async () => {
          const it = await items.get(ref);
          if (!it || !KINDS.has(it.kind)) throw new Error("not a document, a spreadsheet or a presentation");
          const sid = spaceOf(ref);
          const sp = sid ? ((await space.mine.strict()).find(s => s.id === sid) ?? null) : null;
          if (sid && !sp) throw new Error("not in that space");
          // WHERE ITS CHANGES ARE: a space's table (its members'); a PUBLIC one of a person's own space, their public
          // tail (`directory.publicOf`: anyone reads it, only their account writes it — 10-21, a doc made public read
          // empty by everyone else: each reader opened a table of their OWN); a private one, its maker's own table.
          const pub = !sid && !it.private;
          const t = pub ? await (await ctx.require("directory")).publicOf(it.by, `doc-${it.id}`) : await storage.table(`doc-${it.id}`, sp);
          if (!t) throw new Error("its maker's public tables are not known here yet");
          await (pub ? t.answer() : t.settled);
          const r = sp ? await roles.of.strict(sp) : null;
          if (r) await r.settled;
          const R = await ctx.require("roles");
          // A ROW STANDS by its writer: the maker's, or an editor's as of the row's time; an unknown writer's not yet.
          const admitted = row => {
            const w = r.author(row);
            if (!w) return false;
            if (w === it.by) return true;
            return R.mayWrite({ action: "edit", item: { kind: it.kind, by: it.by, meta: it.meta }, writer: w, r, at: Number(parse(row.value)?.at) || Infinity }) === true;
          };
          if (r && t.admit) {
            t.admit(admitted);
            r.onChange?.(() => t.admit(admitted));
            r.onWriters?.(() => t.admit(admitted));
          }
          const rowsNow = () => t.rows().map(x => ({ key: x.key, c: parse(x.value)?.c })).filter(x => typeof x.c === "string");
          const all = rowsNow();
          const o = { doc: glue.Doc.load(actor, all.map(x => unb64(x.c))), t, seen: new Set(all.map(x => x.key)), v: 1, it, r, sp, pending: [], timer: null };
          t.onChange(() => {
            const fresh = rowsNow().filter(x => !o.seen.has(x.key));
            if (!fresh.length) return;
            fresh.forEach(x => o.seen.add(x.key));
            if (o.doc.apply(fresh.map(x => unb64(x.c)))) o.v += 1;
          });
          return o;
        })().catch(e => (open_.delete(ref), Promise.reject(e))),
      );
    return open_.get(ref);
  }

  // A BURST of changes written as ONE row (its bytes concatenated: `doc` loads them as each alone): 400 ms after the
  // last, or at once when the page goes.
  async function flush(o) {
    clearTimeout(o.timer);
    o.timer = null;
    if (!o.pending.length) return;
    const parts = o.pending.splice(0);
    const bytes = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
    let at = 0;
    for (const p of parts) bytes.set(p, at), (at += p.length);
    const key = await keyOf(bytes);
    o.seen.add(key);
    await o.t.put(key, JSON.stringify({ at: Date.now(), c: b64(bytes) }));
  }
  const write = (o, change) => {
    o.pending.push(change);
    o.v += 1;
    clearTimeout(o.timer);
    o.timer = setTimeout(() => ctx.background("codoc: write", flush(o)), 400);
  };
  addEventListener("pagehide", () => open_.forEach(p => ctx.background("codoc: flush on leaving", p.then(o => flush(o)))));

  async function mayEdit(o) {
    const me = await space.me.strict();
    if (o.it.by === me) return true;
    if (!o.r) return false;
    return (await ctx.require("roles")).mayWrite({ action: "edit", item: { kind: o.it.kind, by: o.it.by, meta: o.it.meta }, writer: me, r: o.r }) === true;
  }

  async function create({ board = null, title = "", audience = null, kind = "doc" } = {}) {
    if (!KINDS.has(kind)) throw new Error(`not a kind written together: ${kind}`);
    const untitled = { doc: "Untitled document", sheet: "Untitled spreadsheet", deck: "Untitled presentation" }[kind];
    const ref = await items.submit({ board, title: String(title).trim() || untitled, body: "", kind, audience: audience ?? (board ? "members" : "private") });
    const o = await open(ref);
    // ITS MAKER's FIRST CHANGE: the text object everyone else starts from (the only one: a second would be a second text).
    if (!o.seen.size) {
      const [doc, first] = glue.Doc.create(actor);
      o.doc = doc;
      write(o, first);
      await flush(o);
    }
    return ref;
  }

  async function state(ref) {
    const o = await open(ref);
    let text = "";
    let marks = [];
    let cells = {};
    try {
      text = o.doc.text();
      marks = JSON.parse(o.doc.marks());
      cells = JSON.parse(o.doc.cells());
    } catch {
      // (Its maker's first change not read yet: empty, said by `ready`.)
    }
    return { text, marks, cells, v: o.v, ready: o.seen.size > 0, mayEdit: await mayEdit(o), title: o.it.title, kind: o.it.kind };
  }

  async function splice(ref, pos, del, insert = "") {
    const o = await open(ref);
    if (!(await mayEdit(o))) throw new Error("you may read this document, not edit it");
    write(o, o.doc.splice(pos, del, insert));
    return { text: o.doc.text(), v: o.v };
  }

  async function mark(ref, start, end, name, value = null, on = true) {
    const o = await open(ref);
    if (!(await mayEdit(o))) throw new Error("you may read this document, not edit it");
    write(o, o.doc.mark(start, end, name, value ?? undefined, on));
    return { marks: JSON.parse(o.doc.marks()), v: o.v };
  }

  // A CELL set (a sheet's cell, a slide's field; `value` null: cleared) → its cells now.
  async function set(ref, key, value = null) {
    const o = await open(ref);
    if (!(await mayEdit(o))) throw new Error("you may read this, not edit it");
    write(o, o.doc.set(String(key), value == null ? undefined : String(value)));
    return { v: o.v };
  }

  // ITS WHOLE DOCUMENT, to move with it (`share.moveTo`): a snapshot (every change, compacted) — and, in its new place,
  // that snapshot as the first row of its table (its mover's).
  async function snapshot(ref) {
    const o = await open(ref);
    await flush(o);
    return b64(o.doc.save());
  }
  async function seed(ref, snap) {
    const o = await open(ref);
    const bytes = unb64(snap);
    const key = await keyOf(bytes);
    if (o.seen.has(key)) return;
    o.seen.add(key);
    o.doc.apply([bytes]);
    o.v += 1;
    await o.t.put(key, JSON.stringify({ at: Date.now(), c: snap }));
  }

  return { create, state, splice, mark, set, snapshot, seed };
}
