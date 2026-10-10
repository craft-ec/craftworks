// VOUCH, a capability: an account's public VOUCHES (docs/REPUTATION.md) — "I vouch for B" — a row per person in its
// public tail `vouches` (only it writes; anyone reads), stamped with the beacon's height (the order the trust tree is
// built in). An account's earliest 10 count. Reputation is COMPUTED from everyone's vouches (`reputation/`), never
// issued; vouching is a bet — a proven cheat costs its vouchers too.
//
//   const vouch = await ctx.require("vouch");
//   vouch.has(did)  await vouch.set(did, on)  await vouch.of(did) → [{ did, at }]  vouch.BUDGET
export async function start(ctx) {
  const [directory, space] = await Promise.all(["directory", "space"].map(n => ctx.require(n)));
  const BUDGET = 10;
  const TABLE = "vouches";
  let mineOpen = null;
  const mine = () => (mineOpen ??= space.account().then(me => directory.publicOf(me.id, TABLE)));
  const read = t =>
    (t?.rows() ?? []).map(r => {
      let v = {};
      try {
        v = JSON.parse(r.value) ?? {};
      } catch {}
      return { did: r.key, at: Number(v.at) || 0 };
    });

  // The beacon's height now (the vouch's stamp).
  async function height() {
    const b = await ctx.require("beacon");
    const s = b.status();
    if (s.height != null) return s.height;
    throw new Error("the beacon is not read yet — try again in a moment");
  }

  async function set(did, on) {
    const t = await mine();
    (await ctx.require("reputation")).refresh();
    if (!on) return t.remove(did);
    if (read(t).some(v => v.did === did)) return;
    if (read(t).length >= BUDGET) throw new Error(`you vouch for ${BUDGET} people at most — withdraw one first`);
    await t.put(did, JSON.stringify({ at: await height() }));
  }
  async function of(did) {
    const t = await directory.publicOf(did, TABLE);
    await t?.answer?.();
    return t && !t.absent ? read(t) : [];
  }
  let held = [];
  mine().then(t => {
    held = read(t);
    t.onChange?.(() => (held = read(t)));
  }, () => {});
  const has = did => held.some(v => v.did === did);

  return { BUDGET, has, set, of, mine: async () => read(await mine()) };
}
