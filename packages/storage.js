// STORAGE, a capability: the logged-in ACCOUNT's tables (notes, pins, …). It composes: the core (a tail under one key:
// read, sealed, written, flushed into its tree), `feed` (a row as a version, and the merge), `membership` (whose feeds
// count), `blocks` (tree blocks) and `access` (the grant and the keys that come with it).
//
// Every node writes its OWN FEED of a table — a tail under its node key, labelled `t/<table>` — and reads the table as
// the MERGE of the feeds of the account's nodes (ARCHITECTURE: every writer its own feed; versions are causal). Tables
// from before feeds, under the account's shared data key, are read as the oldest writer. Any SITE the person allows
// reads and writes the same table (the grant is per site and table; the first time, the node asks them). Every feed is
// followed, so a row written elsewhere arrives here as it lands.
//
// Nothing is asked of the network that is not known to exist (a GET of a tail nobody wrote gets no answer): each
// writer's CATALOG feed lists the tables it has a feed of, listed BEFORE the feed is made; the account's DIRECTORY (its
// shared catalog) lists the nodes that have feeds, and the tables from before feeds.
//
//   const notes = await (await ctx.require("storage")).table("notes");
//   notes.rows()                [{ key, value, id }]
//   await notes.put(key, value)   await notes.remove(key)   notes.onChange(fn)
export async function start(ctx) {
  const auth = await ctx.require("auth");
  // What a space is: its keys and its own tables' names (the account, here).
  const space = await ctx.require("space");
  // Who may read and write a table here: the person's grant for this site, and the table's key that comes with it.
  const access = await ctx.require("access");
  const { core, glue, ask, listen } = await ctx.require("node");
  const tailCode = await ctx.require("tail-wasm");
  // A table's tree lives in Sealed contracts, sealed whole (a tree from before, in Block contracts): the core names
  // them from this code.
  core.set_block_code(await ctx.require("block-wasm"));
  core.set_sealed_code(await ctx.require("sealed-wasm"));
  const Core = glue.CraftworksCore;
  // Tree blocks go through the ONE door for them (fetch raced against parity, put).
  const blocks = await ctx.require("blocks");
  // A row as a VERSION, and the MERGE of feeds: its own package.
  const feed = await ctx.require("feed-glue");
  await feed.default({ module_or_path: await ctx.require("feed-wasm") });
  const bytes = hex => new Uint8Array(hex.match(/../g).map(b => parseInt(b, 16)));
  const enc = new TextEncoder();
  const dec = new TextDecoder();
  // The account's own CHANNELS: one shared tail each, outside the catalog's feeds (the MLS group's, which `membership`
  // reads to say whose feeds count).
  const { catalog: CATALOG, members: MEMBERS, channel: CHANNEL } = space.tables;
  const CHANNELS = new Set([CHANNEL]);
  // Tables whose key comes with ANY grant of the site: the catalog, and the space's members.
  const ANY_GRANT = new Set([CATALOG, MEMBERS]);

  // A tail's VIEW, walked to the end: the tail names the tree's root; a view that needs tree blocks names their
  // contracts, fetched (raced, rebuilt if missing) until the rows are all there; one that names EPOCHS gets their keys
  // for this table from the identity (none, for an epoch this node has no key of: what is sealed with it stays
  // unread). Returns the last view: `{ kind: "tail", tail: { rows, … } }`, or `tail-unreadable`.
  async function settle(view, app) {
    for (let round = 0; view.kind === "tail-need" || view.kind === "tail-keys"; round++) {
      if (round >= 24) throw new Error(`${app}: the tree did not finish loading`);
      if (view.kind === "tail-keys") await epochKeys(view.id, view.epochs, app);
      else await blocks.fetch(view.id, view.blocks, app);
      view = JSON.parse(core.tail_view(bytes(view.id)));
    }
    if (view.kind === "tail-unreadable") throw new Error(`${app}: ${view.said}`);
    return view;
  }

  // REFUSING: set by `keys` when the group says this node was REMOVED from the account. From then on it writes
  // nothing — not even sealing rows over, which would take them out of reach of the nodes that remain.
  let refusing = null;

  // Tails holding rows sealed with an epoch this node had no key of when read: `id` → { app, epochs }. When a key
  // arrives (`craftworks:keys`: the identity kept an epoch — this node caught up, or joined, or another device of the
  // account moved the group), they are given it and read again: a device added while this page is open is read.
  const unkeyed = new Map();
  async function epochKeys(idHex, epochs, app) {
    const spaceId = live.get(idHex)?.spaceId ?? null;
    for (const e of epochs) {
      const k = await access.keyAt(app, e, { catalog: app === CATALOG, space: spaceId });
      core.tail_epoch_key(bytes(idHex), e, k.key ? bytes(k.key) : new Uint8Array(0), false);
      const u = unkeyed.get(idHex) ?? { app, epochs: new Set() };
      if (k.key) u.epochs.delete(e);
      else {
        u.epochs.add(e);
        ctx.log("table sealed", { what: `${app}: no key here for epoch ${e} (${k.why})` });
      }
      if (u.epochs.size) unkeyed.set(idHex, u);
      else unkeyed.delete(idHex);
    }
  }
  let rekeying = null;
  addEventListener("craftworks:keys", () => {
    if (!unkeyed.size || rekeying) return;
    rekeying = (async () => {
      for (const [idHex, u] of [...unkeyed]) {
        await epochKeys(idHex, [...u.epochs], u.app).catch(() => {});
        if (unkeyed.has(idHex)) continue;
        const t = live.get(idHex);
        const v = t && (await settle(JSON.parse(core.tail_view(bytes(idHex))), u.app).catch(() => null));
        if (v?.tail) {
          ctx.log("table opened", { what: `${u.app}: the epoch key arrived` });
          t.took(v.tail);
        }
      }
    })().finally(() => (rekeying = null));
  });

  const tails = new Map(); // id hex -> Promise<tail> (opened once per page)
  const live = new Map(); // id hex -> tail, for what the node pushes
  listen(said => {
    const t = (said.kind === "tail" || said.kind === "tail-need" || said.kind === "tail-keys") && live.get(said.id);
    if (t)
      settle(said, t.app).then(
        v => {
          ctx.log("table changed", { what: `${t.app}: seq ${v.tail.seq}, pushed by the node` });
          t.took(v.tail);
        },
        e => ctx.log("table changed", { what: `${t.app}: ${e.message}` }),
      );
  });

  // ONE TAIL: writer `owner`'s (a key, hex) tail of `app`. `known`: whether it exists (true), is new (false: opened
  // empty, no read), or is unknown (null: read). `beforeCreate`: run once before its first write makes it (its
  // listing). `sealWith` (hex): its sealing key, given (a space's epoch log: its epoch's own), instead of the table's.
  // `space` (id bytes): the space it belongs to, not the account — signed in that space, with no site grant. Its rows AS
  // STORED are `raw()` (bytes); `rows()` are its view's.
  function tail(owner, app, { known = null, catalogKey = false, beforeCreate = null, sealWith = null, space: inSpace = null, public: open = false } = {}) {
    // The space: an object (a table of a space: its group keeps it current) or its id's bytes (an epoch log).
    const spaceId = inSpace?.idBytes ?? inSpace;
    const idHex = core.tail_open(tailCode, bytes(owner), app);
    if (tails.has(idHex)) return tails.get(idHex);
    const id = bytes(idHex);
    const name = Core.id_name(id);
    let rows = [];
    const changed = [];
    let queue = Promise.resolve(); // one write at a time: each builds on the step before it
    const t = {
      app,
      owner,
      id,
      spaceId,
      absent: false,
      took: tail => took(tail),
      rows: () => rows,
      raw: () => core.tail_raw(id) ?? [],
      onChange: f => changed.push(f),
      // `value`: bytes, text, or a function giving the bytes at the moment of writing (a version names the feed's
      // next sequence, so it is made in the write's own turn).
      put: (key, value) => (queue = queue.catch(() => {}).then(() => write(key, value))),
      remove: key => (queue = queue.catch(() => {}).then(() => write(key, ""))),
      tailNext: () => core.tail_next(id),
      // Ask again (a tail that was not there: its writer may have made it since).
      reread: () => read(),
    };
    const ready = (async () => {
      // THE TABLE'S KEY: the table is sealed, so reading it needs its key, and the key comes only with the person's
      // grant for this site (asked NOW: the node prompts the first time). No grant: nothing of the table reads here.
      // The catalog's key comes with any grant: listing a table is part of using it.
      // A PUBLIC tail (a person's card): in the clear, no key.
      if (open) core.tail_public(id);
      const k = open ? {} : sealWith ? { key: sealWith } : await access.key(app, { catalog: catalogKey, space: spaceId });
      t.sealed = !!k.key;
      if (k.key) core.tail_seal(id, bytes(k.key));
      else if (!open) ctx.log("table sealed", { what: `${app}: no key here (${k.why}): nothing of it reads` });
      // WRITES are sealed with the account's newest EPOCH (a node removed from the account then reads nothing written
      // after): the group brought current first. Not a channel (a node reads the MLS channel before it has any epoch),
      // and the catalog not after the group (the group's channel is found through it). No group here: the table's key.
      if (k.key && !sealWith && !CHANNELS.has(app)) {
        const keys = await ctx.require("keys");
        if (inSpace?.id) await keys.group(inSpace).ready().catch(() => null);
        else if (!catalogKey) await keys.ready().catch(() => null);
        const e = await access.keyAt(app, -1, { catalog: catalogKey, space: spaceId });
        if (e.key) core.tail_epoch_key(id, e.epoch, bytes(e.key), true);
        else ctx.log("table sealed", { what: `${app}: no epoch here (${e.why}): written with the table's key` });
      }
      if (known === false) {
        core.tail_absent(id);
        t.absent = true;
      } else {
        await read();
      }
      // Rows under an older key: sealed over now, a batch per step (in the write queue), where this node signs the tail.
      const sp = await space.account();
      if ((legacy > 0 || reseal) && (owner === sp?.self || owner === sp?.shared)) queue = queue.catch(() => {}).then(sealOld);
      return t;
    })();
    tails.set(idHex, ready);
    live.set(idHex, t);

    const allowed = () => (catalogKey || spaceId ? Promise.resolve(true) : access.allowed(app));

    // Read it and follow it. None on the network: its first write is a PUT. No answer at all: the same as none, so
    // it opens empty; a first write the network then refuses resets it and reads it again.
    async function read() {
      const [, frames] = core.tail_get(id);
      let said;
      try {
        said = await ask(frames, x => (x.kind === "tail" || x.kind === "tail-need" || x.kind === "tail-keys" || x.kind === "get-failed") && x.id === idHex, `reading ${app}`, 30000);
      } catch (e) {
        ctx.log("table not found yet", { what: `${app}: ${e.message}; opened empty` });
        said = { kind: "get-failed" };
      }
      // The tail is there: its tree must load too. A tree that does not is an error, never an empty table (that
      // would write over rows it could not see).
      if (said.kind !== "get-failed") said = await settle(said, app);
      t.absent = said.kind !== "tail";
      if (said.kind === "tail") took(said.tail);
      else core.tail_absent(id);
    }

    // One write: prepared by the core, signed by the identity delegate with this tail's key, sent as one delta.
    async function write(key, value) {
      if (!(await allowed())) throw new Error(`this app may not change your “${app}”: allow it when your node asks`);
      if (refusing) throw new Error(refusing);
      // Listed BEFORE it is created: a failure between the two leaves a listed tail that is empty, never one nobody
      // can find.
      if (t.absent && beforeCreate) await beforeCreate();
      const v = typeof value === "function" ? value() : typeof value === "string" ? enc.encode(value) : value;
      await step(core.tail_prepare(id, enc.encode(key), v), "saving to");
      // FLUSH once the tail is long: its rows into the tree, the tail emptied (in this write's turn of the queue).
      if (core.tail_pending(id) >= Core.flush_at()) await flush().catch(e => ctx.log("flush failed", { what: `${app}: ${e.message}` }));
      t.absent = false;
    }
    // ONE STEP of this tail, the one way a tail moves: the prepared step signed by the identity delegate, sent (the
    // first as a PUT, then one delta each), and confirmed. Refused anywhere — by the identity or by the node — it never
    // landed: what this page holds is dropped and the tail read again, so the next step builds on what the network has.
    async function step(p, doing) {
      const t0 = performance.now();
      const r = await auth.identity.sign(p.params, p.seq, p.valueHash, spaceId ?? undefined);
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
    let reseal = false;
    function took(tail) {
      legacy = tail.legacy ?? 0;
      reseal = !!tail.resealTree;
      t.info = { rows: tail.rows.length, pending: tail.pending, flushed: !!tail.root, legacy, unreadable: tail.unreadable ?? 0, writes: tail.writes };
      rows = tail.rows;
      for (const f of changed) f();
    }
    async function view() {
      const v = await settle(JSON.parse(core.tail_view(id)), app);
      took(v.tail);
    }

    // SEAL OVER the rows under an older key (plaintext from before sealing, the table's key, an earlier epoch): 16
    // rows a step, each sealed with the newest key and its old copy deleted in that same step, signed like any write.
    // A tree from before sealing whole is built again, sealed, by a flush.
    async function sealOld() {
      if (refusing) return;
      let n = 0;
      for (let round = 0; round < 64; round++) {
        const p = core.tail_migrate(id, 16);
        if (!p) break;
        await step(p, "sealing");
        n += 1;
      }
      if (n) ctx.log("sealed", { what: `${app}: rows under an older key, sealed over in ${n} step(s)` });
      if (reseal || core.tail_pending(id) >= Core.flush_at()) await flush();
    }

    // FLUSH: the tree's new blocks PUT first (a tail must never name a root whose blocks are not there), then the step
    // naming the new root, signed like any write.
    async function flush() {
      const t0 = performance.now();
      for (let round = 0; round < 16; round++) {
        const f = core.tail_flush(id);
        if (f.need || f.keys) {
          await settle(f.need ? { kind: "tail-need", id: idHex, blocks: f.need } : { kind: "tail-keys", id: idHex, epochs: f.keys }, app);
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

  // A feed's own current rows, merged alone: `[{ key, value, id }]` (text; its deletes left out).
  const decoded = rows => rows.map(r => ({ key: dec.decode(r.key), value: dec.decode(r.value), id: r.id }));
  const own = t => decoded(Array.from(feed.merge_feeds([[bytes(t.owner), t.raw()]])));
  // A write to a feed, as a VERSION replacing `after` (the id of the version current where it is written).
  const versioned = (t, key, value, after) =>
    t.put(key, () => feed.version(bytes(t.owner), t.tailNext(), after ?? "", value === null ? undefined : enc.encode(value)));

  // THE DIRECTORY: the account's shared catalog (its data key). Its rows: the tables from before feeds (and whether
  // it was made COMPLETE, with the account), and `node:<key>` for every node that has feeds. A new account's is made at
  // once; an account from before the catalog has none, so its old tables are read as before.
  let directoryOpen = null;
  function directory() {
    return (directoryOpen ??= (async () => {
      const sp = await space.account();
      const d = await tail(sp.shared, CATALOG, { known: sp.fresh ? false : null, catalogKey: true });
      if (d.absent && sp.fresh) await d.put(CATALOG, JSON.stringify({ at: Date.now(), complete: true }));
      return d;
    })());
  }
  // A table from before feeds: listed (true), not listed in a COMPLETE directory (false), or unknown (null: read).
  function legacyListed(d, name) {
    if (d.rows().some(r => r.key === name)) return true;
    const self = d.rows().find(r => r.key === CATALOG);
    const complete = !!self && (() => { try { return JSON.parse(self.value).complete === true; } catch { return false; } })();
    return complete ? false : null;
  }
  async function listInDirectory(key) {
    const d = await directory();
    if (d.rows().some(r => r.key === key)) return;
    if (d.absent && !d.rows().length) await d.put(CATALOG, JSON.stringify({ at: Date.now(), complete: false }));
    await d.put(key, JSON.stringify({ at: Date.now() }));
  }

  // A WRITER's CATALOG feed: the tables it has a feed of. Known to exist once the directory lists its node.
  async function catalogOf(writer) {
    const d = await directory();
    const listed = d.rows().some(r => r.key === `node:${writer}`);
    return tail(writer, CATALOG, { known: listed ? null : false, catalogKey: true, beforeCreate: () => listInDirectory(`node:${writer}`) });
  }
  // The other writers whose feeds count: the nodes the directory lists, those the account's group holds (where it is
  // kept; elsewhere every listed node, until membership is gossiped in the feeds).
  async function writers(me) {
    const d = await directory();
    const listed = d.rows().filter(r => r.key.startsWith("node:")).map(r => r.key.slice(5));
    const members = await (await ctx.require("membership")).writers().catch(() => null);
    return listed.filter(k => k !== me && (!members || members.includes(k)));
  }

  // A CHANNEL: one shared tail, listed in the directory before it is made.
  async function channel(name) {
    const sp = await space.account();
    const d = await directory();
    const listed = legacyListed(d, name);
    return tail(sp.shared, name, { known: listed === false ? false : null, beforeCreate: () => listInDirectory(name) });
  }

  // A SCOPE: a space seen from this node — who writes in it, where each writer lists its tables, how its tails open.
  // The account: the directory names its nodes, their catalogs are account tables, and its tables from before feeds
  // are the oldest writer. Another space: its GROUP's members write, each listing the space's tables in its own catalog
  // feed of the space; every tail of it is the space's (sealed with its epoch keys, signed in it).
  function scopeOf(sp) {
    if (sp.kind === "account")
      return {
        key: "account",
        self: sp.self,
        opts: name => ({ catalogKey: ANY_GRANT.has(name) }),
        old: async name => {
          const o = legacyListed(await directory(), name);
          return o === false ? null : { owner: sp.shared, known: o };
        },
        writers: () => writers(sp.self),
        catalogOf,
      };
    return {
      key: sp.id,
      self: sp.self,
      opts: () => ({ space: sp }),
      old: async () => null,
      // Its WRITERS: every device of every member (the group's members are DIDs; each DID's devices, from its card,
      // checked against its key log) — this device's own siblings too.
      writers: async () => {
        const [keys, directory] = await Promise.all(["keys", "directory"].map(n => ctx.require(n)));
        const members = (await keys.group(sp).ready())?.members ?? [];
        const dids = [...new Set(members.filter(m => m.cred).map(m => glue.did_of(new Uint8Array(m.cred.match(/../g).slice(4, 36).map(x => parseInt(x, 16))))))];
        const all = (await Promise.all(dids.map(d => directory.devices(d)))).flat();
        return [...new Set(all)].filter(k => k !== sp.self);
      },
      catalogOf: w => tail(w, sp.tables.catalog, { space: sp }),
    };
  }

  // A TABLE: the merge of its writers' feeds, and this node's feed to write.
  const tables = new Map(); // scope + name -> Promise<table>
  function merged(name, sp) {
    const scope = scopeOf(sp);
    const at = `${scope.key}/${name}`;
    if (tables.has(at)) return tables.get(at);
    const ready = (async () => {
      if (sp.kind === "account" && !sp.shared) throw new Error("this node does not hold the account's data key: log in once with the recovery words");
      const [mine, others] = await Promise.all([scope.catalogOf(scope.self), scope.writers()]);
      const lists = (cat, n) => own(cat).some(r => r.key === n);
      const all = [];
      // Another writer's feed that does not open here (sealed under a key this node lacks — a node that has not
      // recovered its epochs) is left out and COUNTED, never fatal: this node writes only its own feed, so nothing it
      // cannot see is written over. Its own feed must open.
      let unopened = 0;
      const opts = scope.opts(name);
      const theirs = (owner, known) =>
        tail(owner, name, { known, ...opts }).catch(e => {
          unopened += 1;
          ctx.log("feed not read", { what: `${name}: ${owner.slice(0, 12)}…: ${e.message}` });
          return null;
        });
      let rows = [];
      const changed = [];
      const remerge = () => {
        rows = decoded(Array.from(feed.merge_feeds(all.map(f => [bytes(f.owner), f.raw()]))));
        for (const f of changed) f();
      };
      const take = f => {
        if (!f || all.includes(f)) return;
        all.push(f);
        f.onChange(remerge);
        remerge();
      };
      // The table from before feeds: the oldest writer.
      const old = await scope.old(name);
      if (old) take(await theirs(old.owner, old.known));
      // This node's: listed in its catalog BEFORE it is made. The table opens once it is open.
      const listMine = async () => {
        if (!lists(mine, name)) await versioned(mine, name, JSON.stringify({ at: Date.now() }), own(mine).find(r => r.key === name)?.id);
      };
      const me = await tail(scope.self, name, { known: lists(mine, name) ? null : false, ...opts, beforeCreate: listMine });
      take(me);
      // The OTHER writers' feeds, in the background: each merged in as it arrives, never holding the table up. A writer
      // whose catalog is not there yet (a node that has not written here), or does not list this table yet, is asked
      // again — its catalog is followed once it exists, and a missing one is asked every 30 s — so what it writes later
      // arrives without a reload.
      const opened = new Set();
      const gather = async o => {
        const c = await scope.catalogOf(o).catch(() => null);
        if (!c) return;
        const open = async () => {
          if (opened.has(o) || !lists(c, name)) return;
          opened.add(o);
          take(await theirs(o, true));
        };
        c.onChange(() => open().catch(() => {}));
        if (c.absent) {
          const again = () =>
            setTimeout(async () => {
              await c.reread().catch(() => {});
              if (c.absent) again();
              else await open();
            }, 30000);
          again();
          return;
        }
        await open();
      };
      const settled = Promise.all(others.map(o => gather(o).catch(() => {})));
      ctx.log("table open", { what: `${name}: ${rows.length} row(s) from ${all.filter(f => !f.absent).length} feed(s)` });
      const write = (key, value) => versioned(me, key, value, rows.find(r => r.key === key)?.id);
      const t = {
        app: name,
        // Every writer tried once (some may still arrive later): for what needs the whole table now (adopting a node's
        // rows before its removal).
        settled,
        rows: () => rows,
        onChange: f => changed.push(f),
        put: (key, value) => write(key, value),
        remove: key => write(key, null),
        get sealed() {
          return me.sealed;
        },
        get info() {
          const i = all.map(f => f.info).filter(Boolean);
          return {
            rows: rows.length,
            pending: me.info?.pending ?? 0,
            flushed: i.some(x => x.flushed),
            legacy: i.reduce((n, x) => n + x.legacy, 0),
            unreadable: i.reduce((n, x) => n + x.unreadable, 0),
            writes: me.info?.writes,
            feeds: all.filter(f => !f.absent).length,
            unopened,
          };
        },
      };
      return t;
    })();
    tables.set(at, ready);
    return ready;
  }

  const openedNames = new Set(); // the account's tables this page asked for
  // A table of the account, or (`sp`) of another space.
  async function table(name, sp = null) {
    if (sp && sp.kind !== "account") return merged(name, sp);
    if (CHANNELS.has(name)) return channel(name);
    openedNames.add(name);
    const acc = await space.account();
    if (!acc) throw new Error("nobody is logged in");
    return merged(name, acc);
  }

  // Every table of the account, as `{ name, rows, pending, flushed, sealed, legacy, unreadable, feeds }` (for the
  // Account page's Storage section): the directory's tables from before feeds and every writer's catalog. Only the
  // tables this site uses, or this page already opened, are opened: another app's table is listed by its name
  // (`{ name, closed: true }`), never asked for — no prompt from a list.
  async function describe() {
    const sp = await space.account();
    const d = await directory();
    const cats = await Promise.all([sp.self, ...(await writers(sp.self))].map(catalogOf));
    const names = new Set(d.rows().map(r => r.key).filter(n => n !== CATALOG && !n.startsWith("node:")));
    for (const c of cats) for (const r of own(c)) names.add(r.key);
    const out = [];
    for (const name of names) {
      if (CHANNELS.has(name)) {
        const c = await channel(name);
        out.push({ name, sealed: !!c.sealed, ...(c.info ?? { rows: c.rows().length, pending: 0, flushed: false, legacy: 0, unreadable: 0 }), feeds: 1 });
        continue;
      }
      if (!ctx.uses.includes(name) && !openedNames.has(name)) {
        out.push({ name, closed: true });
        continue;
      }
      const t = await table(name).catch(() => null);
      if (t) out.push({ name, sealed: !!t.sealed, ...t.info });
    }
    return out;
  }

  // A LOG: one shared tail under `owner`'s key (an epoch's, for the account's MLS commits). Read if it is there; made
  // by its first write if not.
  function log(name, owner, { known = null, sealWith = null, space = null } = {}) {
    return tail(owner, name, { known, sealWith, space });
  }

  // A PUBLIC tail under `owner`'s key (a person's card, under their account's data key): anyone reads it. This
  // account's own is listed in its directory before it is made (so it is never asked for while it does not exist).
  async function publicTail(name, owner) {
    const sp = await space.account();
    if (sp && owner === sp.shared) {
      const listed = legacyListed(await directory(), name);
      return tail(owner, name, { public: true, known: listed === false ? false : null, beforeCreate: () => listInDirectory(name) });
    }
    return tail(owner, name, { public: true });
  }

  // The nodes the directory lists as having feeds (hex keys): where `membership` starts gathering.
  async function nodes() {
    return (await directory()).rows().filter(r => r.key.startsWith("node:")).map(r => r.key.slice(5));
  }

  // The OWN ROWS of table `name` in each of `owners`' feeds (where its catalog lists one): `[{ owner, rows }]`. A
  // union, not a merge — for what must not be overwritten by one writer (a removal stays found in its remover's feed).
  async function feedsOf(name, owners) {
    const out = [];
    await Promise.all(
      owners.map(async owner => {
        const cat = await catalogOf(owner).catch(() => null);
        if (!cat || !own(cat).some(r => r.key === name)) return;
        const t = await tail(owner, name, { known: true, catalogKey: ANY_GRANT.has(name) }).catch(() => null);
        if (t) out.push({ owner, rows: own(t) });
      }),
    );
    return out;
  }

  // ADOPT a node's current rows (before it is removed): every row whose current version is `node`'s is written again
  // in this node's own feed, replacing it — so when its feed stops counting, nothing it wrote is lost.
  async function adopt(node) {
    const sp = await space.account();
    const d = await directory();
    const cats = await Promise.all([sp.self, node].map(o => catalogOf(o).catch(() => null)));
    const names = new Set(cats.filter(Boolean).flatMap(c => own(c).map(r => r.key)));
    for (const n of d.rows().map(r => r.key)) if (n !== CATALOG && !n.startsWith("node:")) names.add(n);
    let moved = 0;
    for (const name of names) {
      if (CHANNELS.has(name)) continue;
      const t = await table(name).catch(() => null);
      if (!t) continue;
      await t.settled;
      for (const r of t.rows().filter(r => r.id?.startsWith(node))) {
        await t.put(r.key, r.value);
        moved += 1;
      }
    }
    ctx.log("adopted", { what: `${moved} row(s) of ${node.slice(0, 12)}… now in this node's feeds` });
    return moved;
  }

  return { table, log, publicTail, describe, nodes, feedsOf, adopt, refuse: why => (refusing = why) };
}
