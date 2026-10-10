// KEEPER, a service (TOKEN §7a: keepers are paid on PROVEN keeping): while a page is open, the STANDING RECORD of this
// device's keeper — named by its MEMBER KEY (hex): the node refuses node queries from web apps, so a page never learns
// its peer id or what its node hosts, and proves only what CRAFTWORKS keeps — in its standing contract. Each standing
// epoch its HOLDINGS: the pieces of the files `keep` keeps whole (`keep.holdings`, fixed for the epoch); at each beacon
// height SAMPLED for it, its ANSWER to the beacon's challenge: the asked pieces read (a read is keeping: Freenet evicts
// what is read least), sealed in the page's worker (`beacon.sealed`), signed. Every record is signed by the member key
// through its identity delegate (`tokenSign`, the standing domain). (Answering with the page CLOSED is the delegate's.)
//
// THE FILES REGISTRY (`wallet::registry`): each epoch it also LISTS the account's own files it keeps whole (address key
// and size: what lets other keepers find their pieces), so a file stays listed while its owner keeps it.
//
//   (await ctx.require("keeper")).status()   // { key, epoch, files, pieces, listed, opened, answered, last } — what this page did
//   await keeper.network(epoch)              // { files, bytes, keepers }: the registries at standing epoch `epoch`
export async function start(ctx) {
  const beaconSvc = await ctx.require("beacon");
  if (beaconSvc.status().off) return { status: () => ({ off: true }) };
  const [node, auth, standingCode, beaconCode] = await Promise.all(["node", "auth", "standing-wasm", "beacon-wasm"].map(n => ctx.require(n)));
  const { core } = node;
  const Core = node.glue.CraftworksCore;
  const state = { key: null, epoch: null, files: 0, pieces: 0, listed: 0, opened: 0, answered: 0, last: null };
  let self = null; // { epoch, hosted, open } — this epoch's holdings ([id hex, size]) and whether its record is open
  let busy = false;
  let upTo = -1; // the last height handled
  // How far back a catch-up looks (the beacon's state keeps its latest steps' outputs; older ones are null).
  const BACK = 1024;

  // The epoch's HOLDINGS: every piece of the files `keep` keeps whole, fixed for the epoch.
  async function holdingsOf(epoch) {
    // (Like the token service, `keep` and `files` need an account: asked for once one is logged in.)
    const [keep, files] = await Promise.all(["keep", "files"].map(n => ctx.require(n)));
    const fs = await keep.holdings(epoch);
    // ITS OWN files listed in this epoch's registry (a watched file is its uploader's to list).
    const index = await ctx.require("index");
    let listed = 0;
    for (const { sp, row } of fs.filter(f => !f.watched))
      await files
        .listing(sp, row)
        .then(item => index.bag.chained(Core.registry_address, epoch, item, "listing a file in the registry"))
        .then(() => (listed += 1))
        .catch(e => ctx.log("keeper", { what: `${row.id.slice(0, 8)}…: not listed (${e?.message ?? e})` }));
    state.listed = listed;
    const hosted = [];
    for (const { sp, row } of fs) hosted.push(...(await files.pieces(sp, row).catch(e => (ctx.log("keeper", { what: `${row.id.slice(0, 8)}…: its pieces not listed (${e?.message ?? e})` }), []))));
    state.files = fs.length;
    state.pieces = hosted.length;
    return hosted;
  }

  async function tick(h) {
    if (busy) return;
    busy = true;
    try {
      const me = await auth.check().catch(() => null);
      if (!me) return;
      // (The token service needs an account: asked for once one is logged in.)
      const tokens = await ctx.require("tokens");
      if (!tokens.on) return;
      const { beacon, run, putAll, bytes, hexOf } = tokens;
      const public_ = bytes(me.member);
      const epoch = Core.keeper_epoch(h);
      // (None yet — `keep`'s passes still running: asked again 64 heights on.)
      if (!self || self.epoch !== epoch || (!self.hosted.length && h - self.at >= 64)) self = { epoch, at: h, hosted: await holdingsOf(epoch), open: false };
      if (!self.hosted.length) return;
      Object.assign(state, { key: me.member, epoch });
      // OPEN the epoch: binding and holdings.
      if (!self.open) {
        const r = await run(sigs => core.keeper_open(standingCode, beaconCode, beacon, public_, epoch, self.hosted, sigs));
        if (r?.puts) {
          await putAll(r.puts);
          state.opened++;
          ctx.log("keeper", { what: `epoch ${epoch}: holdings of ${state.files} file(s), ${self.hosted.length} piece(s) committed; ${state.listed} own file(s) listed in the registry` });
        }
        self.open = true;
        // FINDABLE: this keeper (its account and the credential naming its record's key) in the epoch's keepers registry.
        const { creds } = await tokens.device();
        const item = await run(() => core.keeper_listing(tokens.idlogCode, me.didBytes, public_, creds));
        await (await ctx.require("index")).bag.chained(Core.keepers_address, epoch, item, "listing this keeper");
        state.registered = epoch;
      }
      // ANSWER every height of this epoch since the last one handled that is sampled for this keeper (one that came
      // while it was sealing is not missed), while the beacon's state still holds its output. The beacon read into the
      // core first (the beacon service takes its own reads out of it).
      await tokens.read(hexOf(beacon));
      for (let x = Math.max(upTo + 1, h - BACK); x <= h; x++) {
        if (Core.keeper_epoch(x) !== epoch) continue;
        const out = core.beacon_out(beacon, x);
        if (!out) continue;
        const asked = await run(() => core.keeper_asked(standingCode, beaconCode, beacon, public_, x, out, self.hosted));
        if (!asked?.length) continue;
        const hashes = [];
        for (const id of asked) {
          await tokens.read(id);
          const piece = core.take_got(id) ?? new Uint8Array(0);
          hashes.push([id, await beaconSvc.sealed(public_, bytes(id), piece, out)]);
        }
        const r = await run(sigs => core.keeper_answer(standingCode, beaconCode, beacon, public_, x, out, self.hosted, hashes, sigs));
        if (r?.puts) {
          await putAll(r.puts);
          state.answered++;
          state.last = x;
          ctx.log("keeper", { what: `height ${x}: answered the beacon's challenge (${asked.length} piece(s))` });
        }
      }
      upTo = h;
    } catch (e) {
      ctx.log("keeper", { what: e.message ?? String(e) });
    } finally {
      busy = false;
    }
  }
  beaconSvc.onHeight(tick);
  // The NETWORK: the files live in the registry at `epoch` (listed then or the epoch before) and their bytes; the
  // keepers listed that epoch (verified against their accounts' key logs).
  async function network(epoch) {
    const [index, tokens] = await Promise.all(["index", "tokens"].map(n => ctx.require(n)));
    const [now, before, ks] = await Promise.all([
      ...[epoch, epoch - 1].map(e => (e < 0 ? [] : index.bag.chain(Core.registry_address, e, "reading the files registry"))),
      index.bag.chain(Core.keepers_address, epoch, "reading the keepers registry"),
    ]);
    const live = Core.registry_live(now, before);
    const keepers = await tokens.run(() => core.keepers_listed(tokens.idlogCode, ks));
    return { files: live.length, bytes: live.reduce((n, l) => n + l.size, 0), keepers: keepers.length };
  }
  return { status: () => ({ ...state }), network };
}
