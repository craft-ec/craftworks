// DIRECTORY, a capability: every person's public CARD — found from their DID alone: DID → their key log (its address
// comes from the DID) → their account's data key → their card, a public tail under it. Nobody else can write it; anyone
// reads it. A card holds a HANDLE (a name to show — not unique: a person is their handle AND their id), a KEY
// PACKAGE per node (MLS: with it, anyone adds that node to a conversation while it is away), and the account's INBOX
// key (what is sealed to it only the account's nodes open: `index`'s inbox).
//
//   const directory = await ctx.require("directory");
//   await directory.card(did)                 // { did, handle, inbox, keyPackages: [{ node, keyPackage }] }, or null
//   await directory.publish({ handle })       // this person's handle, and this node's key package
//   await directory.handle(did)               // their handle, or null (each card read once per page)
//   directory.shown(did, handle)              // how a person is SHOWN everywhere: `pat#8r4orC`
//   await directory.name(did)                 // the same, their handle looked up
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
      // Each node's key packages (a list; one picked at random for each conversation started).
      keyPackages: rows
        .filter(r => r.key.startsWith("kp/"))
        .map(r => {
          let list;
          try {
            list = JSON.parse(r.value);
          } catch {
            list = [r.value];
          }
          if (!Array.isArray(list)) list = [r.value];
          return { node: r.key.slice(3), keyPackage: list[Math.floor(Math.random() * list.length)], count: list.length };
        }),
    };
  };

  async function card(did) {
    const k = await keysOf(did);
    if (!k) return null;
    const t = await storage.publicTail(CARD, k.data);
    if (t.absent) return null;
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
    if (!read(t).keyPackages.some(k => k.node === sp.self)) await putKeyPackages(t, sp);
    return { did: sp.id, ...read(t) };
  }

  // NEW key packages for this node (one of them was used to add it somewhere: each works once).
  async function renew() {
    const sp = await space.account();
    await putKeyPackages(await storage.publicTail(CARD, sp.shared), sp);
  }

  // This node's key packages on the card: a fresh set (the ones before stay usable: their secrets are kept here).
  async function putKeyPackages(t, sp) {
    const kps = await (await ctx.require("keys")).keyPackages();
    if (kps) await t.put(`kp/${sp.self}`, JSON.stringify(kps));
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

  return { card, publish, renew, handle, shown, name };
}
