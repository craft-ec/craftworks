// VOUCH, a capability: an account's public VOUCHES (docs/REPUTATION.md) — "I vouch for B" or its withdrawal — each a
// SIGNED ITEM (`reputation::item`: signed by this node, the account's device, through the identity's `TokenSign`)
// stored as the row for B in the account's public tail `vouches` and dropped into the epoch's VOUCH REGISTRY (a bag).
// Signed, it is anyone's to RELAY into the registry later (`relay`) — its voucher need not be online. An account's
// earliest 10 standing vouches count; reputation is computed from verified items (`reputation`), never issued.
//
//   const vouch = await ctx.require("vouch");
//   vouch.has(did)  await vouch.set(did, on)  await vouch.of(did) → [item bytes]  await vouch.relay(items)  vouch.BUDGET
export async function start(ctx) {
  const [directory, space, auth, node, membership, index, idlogCode] = await Promise.all(["directory", "space", "auth", "node", "membership", "index", "idlog-wasm"].map(n => ctx.require(n)));
  const { core, glue } = node;
  const Core = glue.CraftworksCore;
  const BUDGET = 10;
  const TABLE = "vouches";
  const EPOCH = 1024; // standing epoch (beacon heights): a vouch's registry is its height's epoch
  const bytes = h => new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
  const hexOf = b => [...b].map(x => x.toString(16).padStart(2, "0")).join("");
  let mineOpen = null;
  const mine = () => (mineOpen ??= space.account().then(me => directory.publicOf(me.id, TABLE)));
  // A table's rows as signed items (rows from before signing — not items — are left out).
  const items = t =>
    (t?.rows() ?? []).flatMap(r => {
      try {
        const b = bytes(r.value);
        const v = Core.vouch_item_read(b);
        return v ? [{ to: r.key, item: b, ...v }] : [];
      } catch {
        return [];
      }
    });

  async function height() {
    const s = (await ctx.require("beacon")).status();
    if (s.height != null) return s.height;
    throw new Error("the beacon is not read yet — try again in a moment");
  }

  // SIGN one: this node signs the message, the core builds the item (reading the key log it needs first).
  async function sign(to, on) {
    const me = await auth.check();
    const at = await height();
    const toBytes = glue.did_bytes(to);
    const a = await auth.identity.tokenSign(Core.vouch_message(me.didBytes, toBytes, at, on));
    if (!a?.signed) throw new Error(`this node did not sign (${a?.refused ?? "no answer"})`);
    const creds = await membership.credentials();
    for (let round = 0; round < 4; round++) {
      const r = core.vouch_item(idlogCode, me.didBytes, toBytes, at, on, bytes(me.member), creds, bytes(a.signed));
      if (r.item) return r.item;
      await Promise.all((r.want ?? []).map(id => node.read(bytes(id), "reading a key log", { absent: true })));
    }
    throw new Error("the account's key log could not be read");
  }

  // RELAY signed items into their epochs' registries (each epoch's bag read once; only what it lacks is added).
  async function relay(list) {
    const byEpoch = new Map();
    for (const it of list) {
      const v = Core.vouch_item_read(it);
      if (!v) continue;
      const e = Math.floor(v.at / EPOCH);
      byEpoch.set(e, [...(byEpoch.get(e) ?? []), it]);
    }
    for (const [e, its] of byEpoch) {
      const address = Core.vouch_registry_address(e);
      const held = new Set((await index.bag.read(address, "reading the vouch registry")).map(p => hexOf(p)));
      for (const it of its) if (!held.has(hexOf(it))) await index.bag.add(address, it, "relaying a vouch");
    }
  }

  async function set(did, on) {
    const t = await mine();
    const standing = items(t).filter(v => v.on && v.to !== did).length;
    if (on && standing >= BUDGET) throw new Error(`you vouch for ${BUDGET} people at most — withdraw one first`);
    (await ctx.require("reputation")).refresh();
    const item = await sign(did, on);
    await t.put(did, hexOf(item));
    await relay([item]).catch(e => ctx.log("vouch", { what: `relaying: ${e.message ?? e}` }));
  }
  async function of(did) {
    const t = await directory.publicOf(did, TABLE);
    await t?.answer?.();
    return t && !t.absent ? items(t).map(v => v.item) : [];
  }
  let held = [];
  mine().then(t => {
    held = items(t);
    t.onChange?.(() => (held = items(t)));
  }, () => {});
  const has = did => held.some(v => v.to === did && v.on);

  return { BUDGET, has, set, of, relay };
}
