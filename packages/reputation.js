// REPUTATION, a service (docs/REPUTATION.md): computed, never issued — the SIGNED vouch items crawled outward from the
// starting roots (`vouch.of`: each account's public vouch table), RELAYED into their epochs' registries where missing
// (anyone relays: TOKEN §7d, the two rules), then `core.reputation_from_items` AS OF the beacon's height (each item
// verified against its voucher's key log; the trust tree, kinship, promotion, the ramp). An account nobody reachable
// vouched for has none. The crawl is capped (MAX accounts) and kept a while.
//
//   const reputation = await ctx.require("reputation");
//   await reputation.of(did) → { rep, vouchedBy, family, admitted }   // rep: per-mille of 1.0; null: not reached
export async function start(ctx) {
  const [node, vouch, idlogCode] = await Promise.all(["node", "vouch", "idlog-wasm"].map(n => ctx.require(n)));
  const { core, glue } = node;
  const Core = glue.CraftworksCore;
  const bytes = h => new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
  const MAX = 400; // accounts read at most
  const KEEP = 10 * 60 * 1000; // a crawl kept this long
  let last = null; // { at, items, result }

  async function crawl() {
    const roots = [...Core.reputation_roots()];
    const seen = new Set(roots);
    let frontier = [...roots];
    const items = [];
    while (frontier.length && seen.size <= MAX) {
      const next = [];
      const read = await Promise.all(frontier.map(d => vouch.of(d).catch(() => [])));
      for (const its of read) {
        for (const it of its) {
          items.push(it);
          const v = Core.vouch_item_read(it);
          if (v?.on && !seen.has(v.to) && seen.size < MAX) {
            seen.add(v.to);
            next.push(v.to);
          }
        }
      }
      frontier = next;
    }
    vouch.relay(items).catch(e => ctx.log("reputation", { what: `relaying: ${e.message ?? e}` }));
    const now = (await ctx.require("beacon")).status().height ?? 0;
    let result = null;
    for (let round = 0; round < 6 && !result; round++) {
      const r = core.reputation_from_items(idlogCode, items, now);
      if (r.want) await Promise.all(r.want.map(id => node.read(bytes(id), "reading a voucher's key log", { absent: true }).catch(() => false)));
      else result = r;
    }
    return { at: Date.now(), items, result: result ?? { rep: {}, promoted: [], admitted: 0 } };
  }
  async function current() {
    if (!last || Date.now() - last.at > KEEP) last = await crawl();
    return last;
  }
  async function of(did) {
    const { items, result } = await current();
    return {
      rep: result.rep[did] ?? null,
      vouchedBy: items.filter(it => {
        const v = Core.vouch_item_read(it);
        return v?.on && v.to === did;
      }).length,
      family: result.promoted.includes(did),
      admitted: result.admitted,
    };
  }
  return { of, refresh: () => (last = null) };
}
