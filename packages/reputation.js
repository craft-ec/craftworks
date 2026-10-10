// REPUTATION, a service (docs/REPUTATION.md): computed, never issued — the vouches CRAWLED outward from the starting
// roots (`vouch.of`: each account's public vouches), then `core.reputation_compute` (the `reputation` crate: the trust
// tree, kinship, diversity, promotion, the flow). An account nobody reachable vouched for has none. The crawl is
// capped (MAX accounts) and kept for a while; every page computes the same numbers from the same vouches.
//
//   const reputation = await ctx.require("reputation");
//   await reputation.of(did) → { rep, vouchedBy, family, admitted }   // rep: per-mille of 1.0; null: not reached
export async function start(ctx) {
  const [{ glue }, vouch] = await Promise.all(["node", "vouch"].map(n => ctx.require(n)));
  const Core = glue.CraftworksCore;
  const MAX = 400; // accounts read at most
  const KEEP = 10 * 60 * 1000; // a crawl kept this long
  let last = null; // { at, vouches, result }

  async function crawl() {
    const roots = [...Core.reputation_roots()];
    const seen = new Set(roots);
    let frontier = [...roots];
    const vouches = [];
    while (frontier.length && seen.size <= MAX) {
      const next = [];
      const read = await Promise.all(frontier.map(d => vouch.of(d).then(vs => [d, vs], () => [d, []])));
      for (const [from, vs] of read) {
        for (const v of vs) {
          vouches.push([from, v.did, v.at]);
          if (!seen.has(v.did) && seen.size < MAX) {
            seen.add(v.did);
            next.push(v.did);
          }
        }
      }
      frontier = next;
    }
    return { at: Date.now(), vouches, result: Core.reputation_compute(vouches) };
  }
  async function current() {
    if (!last || Date.now() - last.at > KEEP) last = await crawl();
    return last;
  }
  async function of(did) {
    const { vouches, result } = await current();
    return {
      rep: result.rep[did] ?? null,
      vouchedBy: vouches.filter(v => v[1] === did).length,
      family: result.promoted.includes(did),
      admitted: result.admitted,
    };
  }
  return { of, refresh: () => (last = null) };
}
