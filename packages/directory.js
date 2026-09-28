// DIRECTORY, a capability: every person's public CARD — found from their DID alone: DID → their key log (its address
// comes from the DID) → their account's data key → their card, a public tail under it. Nobody else can write it; anyone
// reads it. A card holds a HANDLE (a name to show — not unique: a person is their handle AND their id) and a KEY
// PACKAGE per node (MLS: with it, anyone adds that node to a conversation while it is away).
//
//   const directory = await ctx.require("directory");
//   await directory.card(did)                 // { did, handle, keyPackages: [{ node, keyPackage }] }, or null
//   await directory.publish({ handle })       // this person's handle, and this node's key package
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
      keyPackages: rows.filter(r => r.key.startsWith("kp/")).map(r => ({ node: r.key.slice(3), keyPackage: r.value })),
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
    if (!read(t).keyPackages.some(k => k.node === sp.self)) {
      const kp = await (await ctx.require("keys")).keyPackage();
      if (kp) await t.put(`kp/${sp.self}`, kp);
    }
    return { did: sp.id, ...read(t) };
  }

  return { card, publish };
}
