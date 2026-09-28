// DIRECTORY, a capability: every person's public CARD — found from their DID alone: DID → their key log (its address
// comes from the DID) → their account's data key → their card, a public tail under it. Nobody else can write it; anyone
// reads it. A card holds a HANDLE (a name to show — not unique: a person is their handle AND their id), the DID's KEY
// PACKAGES (MLS: with one, anyone adds this person to a conversation while they are away — the person, never a device),
// the account's INBOX key (what is sealed to it only the account's nodes open: `index`'s inbox), and its DEVICES'
// credentials (each signed by an owner key of the DID's key log: where a device writes on the DID's behalf in a space,
// readers learn it is that DID's).
//
//   const directory = await ctx.require("directory");
//   await directory.card(did)                 // { did, handle, inbox, keyPackage }, or null (keyPackage: one, at random)
//   await directory.publish({ handle })       // this person's handle, and their key packages
//   await directory.handle(did)               // their handle, or null (each card read once per page)
//   directory.shown(did, handle)              // how a person is SHOWN everywhere: `pat#8r4orC`
//   await directory.name(did)                 // the same, their handle looked up
//   await directory.publicOf(did, name)       // any public tail of theirs (`card`, `mail`: only their account writes it)
//   await directory.dataKey(did)              // their account's data key (hex) as their key log names it, or null
//   await directory.devices(did, fresh?)      // their devices' keys (hex), each credential checked against their key log
export async function start(ctx) {
  const auth = await ctx.require("auth");
  const storage = await ctx.require("storage");
  const space = await ctx.require("space");
  const { core, glue } = await ctx.require("node");
  const idlogCode = await ctx.require("idlog-wasm");
  const CARD = "card";

  const didBytes = did => (typeof did === "string" ? glue.did_bytes(did) : did);
  // An account's public keys, from its key log (read first).
  async function keysOf(did) {
    const b = didBytes(did);
    if (!(await auth.identity.readKeyLog(b))) return null;
    return core.idlog_keys(idlogCode, b);
  }

  const read = t => {
    const rows = t.rows();
    return {
      handle: rows.find(r => r.key === "handle")?.value ?? null,
      inbox: rows.find(r => r.key === "inbox")?.value ?? null,
      // The account's devices' credentials (hex), as its key group has them.
      nodes: (() => {
        try {
          const list = JSON.parse(rows.find(r => r.key === "nodes")?.value ?? "[]");
          return Array.isArray(list) ? list.filter(x => typeof x === "string") : [];
        } catch {
          return [];
        }
      })(),
      // The DID's key packages (a list; one picked at random for each conversation started).
      keyPackage: (() => {
        let list = [];
        try {
          list = JSON.parse(rows.find(r => r.key === "kp")?.value ?? "[]");
        } catch {}
        return Array.isArray(list) && list.length ? list[Math.floor(Math.random() * list.length)] : null;
      })(),
    };
  };

  // A person's public tail `name`, under their account's data key: whatever is in it, their account wrote.
  async function publicOf(did, name) {
    const me = await space.account();
    const id = typeof did === "string" ? did : glue.did_of(did);
    if (me?.id === id) return storage.publicTail(name, me.shared);
    const k = await keysOf(did);
    return k ? storage.publicTail(name, k.data) : null;
  }

  async function card(did, { fresh = false } = {}) {
    const t = await publicOf(did, CARD);
    if (fresh) await t?.reread?.().catch(() => {});
    if (!t || t.absent) return null;
    return { did: typeof did === "string" ? did : glue.did_of(did), ...read(t) };
  }

  // THIS person's card: its handle, and this node's key package (made by `keys`, its secrets kept with this node).
  async function publish({ handle } = {}) {
    const sp = await space.account();
    if (!sp?.shared) throw new Error("this node does not hold the account's data key");
    const t = await storage.publicTail(CARD, sp.shared);
    if (handle != null && read(t).handle !== handle) await t.put("handle", handle);
    // The inbox key, and the (empty) inbox made now: nobody ever waits on an inbox that does not exist yet.
    if (!read(t).inbox) {
      const k = await auth.identity.inboxKey();
      if (!k.inboxKey) throw new Error(`no inbox key here: ${k.refused ?? JSON.stringify(k)}`);
      await (await ctx.require("index")).makeInbox();
      await t.put("inbox", k.inboxKey);
    }
    if (!read(t).keyPackage) await putKeyPackages(t);
    await putNodes(t);
    return { did: sp.id, ...read(t) };
  }

  // The account's DEVICES on the card: the credentials its key group holds now (rewritten when they change: a device
  // joined or was removed).
  async function putNodes(t) {
    const st = await (await ctx.require("keys")).ready().catch(() => null);
    const creds = (st?.members ?? []).map(m => m.cred).filter(Boolean).sort();
    if (!creds.length || JSON.stringify(creds) === JSON.stringify(read(t).nodes.slice().sort())) return;
    await t.put("nodes", JSON.stringify(creds));
  }

  // NEW key packages (one was used to add this person somewhere: each works once).
  async function renew() {
    const sp = await space.account();
    await putKeyPackages(await storage.publicTail(CARD, sp.shared));
  }

  // The DID's key packages on the card: a fresh set (the ones before stay usable: their secrets are the account's).
  async function putKeyPackages(t) {
    const kps = await (await ctx.require("keys")).keyPackages();
    if (kps?.length) await t.put("kp", JSON.stringify(kps));
  }

  // A person's HANDLE, read once per page (a name to show; the id is what makes them them).
  const handles = new Map();
  function handle(did) {
    if (!handles.has(did)) handles.set(did, card(did).then(c => c?.handle ?? null, () => null));
    return handles.get(did);
  }

  // How a person is SHOWN, everywhere: their handle and the start of their id — `pat#8r4orC` (handles are not unique;
  // the id is). Without a handle: `#8r4orC`.
  const shown = (did, handle) => `${handle ?? ""}#${String(did).replace(/^did:craftec:/, "").slice(0, 6)}`;
  const name = async did => shown(did, await handle(did));

  // A person's DATA key, from their key log (once per page): what signs their member's credential in a space.
  const dataKeys = new Map();
  function dataKey(did) {
    if (!dataKeys.has(did)) dataKeys.set(did, keysOf(did).then(k => k?.data ?? null, () => null));
    return dataKeys.get(did);
  }

  // A person's DEVICES: the keys their card's credentials name, each checked against their key log (a credential signed
  // by a key the log never named counts for nothing). Once per page, or again when asked (`fresh`: a device may have
  // joined since).
  const deviceSets = new Map();
  function devices(did, fresh = false) {
    if (fresh || !deviceSets.has(did))
      deviceSets.set(
        did,
        (async () => {
          const c = await card(did, { fresh });
          if (!c?.nodes.length || !(await keysOf(did))) return [];
          const creds = c.nodes.map(h => new Uint8Array(h.match(/../g).map(x => parseInt(x, 16))));
          return Array.from(core.account_members(idlogCode, didBytes(did), creds, []));
        })().catch(() => []),
      );
    return deviceSets.get(did);
  }

  return { card, publish, renew, handle, shown, name, publicOf, dataKey, devices };
}
