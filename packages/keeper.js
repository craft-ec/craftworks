// KEEPER, a service (TOKEN §7a: keepers are paid on PROVEN keeping): while a page is open, the STANDING RECORD of this
// device's keeper — named by its MEMBER KEY (hex): the node refuses node queries from web apps, so a page never learns
// its peer id or what its node hosts, and proves only what CRAFTWORKS keeps — in its standing contract. Each standing
// epoch its HOLDINGS: the pieces of the files `keep` keeps whole (`keep.holdings`, fixed for the epoch); at each beacon
// height SAMPLED for it, its ANSWER to the beacon's challenge: the asked pieces read (a read is keeping: Freenet evicts
// what is read least), sealed in the page's worker (`beacon.sealed`), signed. Every record is signed by the member key
// through its identity delegate (`tokenSign`, the standing domain). (Answering with the page CLOSED is the delegate's.)
//
// THE FILES REGISTRY (`wallet::registry`): each epoch it also LISTS every file it keeps whole — its own, watched,
// assigned (address key and size: what lets other keepers find their pieces) — so a file stays listed while anyone
// keeps it. And it lists ITSELF in the keepers registry each epoch it is open (holding anything yet or not; not when
// its owner keeps nothing: limit 0), so a new account is assigned files.
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

  // THE ASSIGNMENT of epoch `epoch` (`craftworks_standing::assign`): the registry's live files, the keepers listed the
  // epoch before (complete: every page computes the same), this keeper's limit (Usage) — handed to `keep`.
  async function assignment(epoch, keep) {
    if (epoch < 1) return;
    const [index, tokens] = await Promise.all(["index", "tokens"].map(n => ctx.require(n)));
    const me = await auth.check();
    const [now, before, ks] = await Promise.all([
      index.bag.chain(Core.registry_address, epoch, "reading the files registry"),
      index.bag.chain(Core.registry_address, epoch - 1, "reading the files registry"),
      index.bag.chain(Core.keepers_address, epoch - 1, "reading the keepers registry"),
    ]);
    const keepers = await tokens.run(() => core.keepers_listed(tokens.idlogCode, ks));
    const { limit } = await keep.kept();
    const mine = Core.keeper_assigned(tokens.bytes(me.member), Core.registry_live(now, before), keepers.map(k => k.key), limit);
    await keep.assign(epoch, mine);
    state.assigned = mine.length;
  }

  // The epoch's HOLDINGS: every piece of what `keep` keeps — its own and watched files whole, its assigned ones' pieces
  // found — fixed for the epoch.
  async function holdingsOf(epoch) {
    // (Like the token service, `keep` and `files` need an account: asked for once one is logged in.)
    const [keep, files, { bytes }] = await Promise.all(["keep", "files", "tokens"].map(n => ctx.require(n)));
    await assignment(epoch, keep).catch(e => ctx.log("keeper", { what: `no assignment this epoch (${e?.message ?? e})` }));
    const fs = await keep.holdings(epoch);
    // EVERY file it holds whole listed again in this epoch's registry — its own, those it watched, those it is assigned
    // (none missing): a file stays listed while anyone keeps it, its uploader online or not.
    const index = await ctx.require("index");
    let listed = 0;
    for (const f of fs) {
      if (f.assigned && f.assigned.missing.length) continue;
      const item = f.assigned ? Promise.resolve(Core.registry_item(bytes(f.assigned.key), f.assigned.size)) : files.listing(f.sp, f.row);
      await item
        .then(i => index.bag.chained(Core.registry_address, epoch, i, "listing a file in the registry"))
        .then(() => (listed += 1))
        .catch(e => ctx.log("keeper", { what: `${(f.assigned?.key ?? f.row.id).slice(0, 8)}…: not listed (${e?.message ?? e})` }));
    }
    state.listed = listed;
    state.own = fs.filter(f => !f.assigned).length;
    const hosted = [];
    for (const f of fs) {
      if (f.assigned) {
        const gone = new Set(f.assigned.missing);
        hosted.push(...core.registry_pieces(bytes(f.assigned.key), f.assigned.size).filter((_, i) => !gone.has(i)));
        continue;
      }
      const { sp, row } = f;
      hosted.push(...(await files.pieces(sp, row).catch(e => (ctx.log("keeper", { what: `${row.id.slice(0, 8)}…: its pieces not listed (${e?.message ?? e})` }), []))));
    }
    state.files = fs.length;
    state.pieces = hosted.length;
    return hosted;
  }

  // THE SEAT'S NODE (owner 10-11): an account's witness seat names ONE node's key — the one most likely online, the
  // node whose standing record answered the most challenges last epoch (ties: the lowest key). Every node of the
  // account computes the same from the public records; only that one claims.
  async function seatIsMine(epoch, me) {
    const [membership, tokens] = await Promise.all(["membership", "tokens"].map(n => ctx.require(n)));
    const list = (await membership.nodes().catch(() => null)) ?? [{ key: me.member }];
    if (list.length < 2) return true;
    let best = null;
    for (const n of list) {
      const key = n.key.toLowerCase();
      const id = tokens.hexOf(Core.keeper_record_id(standingCode, beaconCode, tokens.beacon, tokens.bytes(key)));
      await tokens.read(id);
      const rec = core.take_got(id);
      const answered = (rec ? Core.standing_read(rec) : []).find(x => x.epoch === epoch - 1)?.answers ?? 0;
      if (!best || answered > best.answered || (answered === best.answered && key < best.key)) best = { key, answered };
    }
    return best.key === me.member.toLowerCase();
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
      Object.assign(state, { key: me.member, epoch });
      // FINDABLE, holding anything yet or not: this keeper (its account and the credential naming its record's key) in
      // the epoch's keepers registry — so a new account is ASSIGNED files — unless its owner keeps nothing (limit 0).
      if (self.registered !== epoch && (await (await ctx.require("keep")).kept()).limit > 0) {
        const { creds } = await tokens.device();
        const item = await run(() => core.keeper_listing(tokens.idlogCode, me.didBytes, public_, creds));
        await (await ctx.require("index")).bag.chained(Core.keepers_address, epoch, item, "listing this keeper");
        self.registered = epoch;
        state.registered = epoch;
      }
      // STAKE IN THE LOT (TOKEN §7a): its earned $CRAFT claimed for the next open claims epoch, its member key the
      // witness key (made its $CRAFT chain's writer first) — once a standing epoch, by the account's SEAT NODE only;
      // nothing staked: nothing claimed.
      if (self.claimed !== epoch && (await seatIsMine(epoch, me))) {
        self.claimed = epoch;
        const { creds } = await tokens.device();
        const c = await run(sigs => core.keeper_claim(tokens.code, beacon, tokens.idlogCode, me.didBytes, h, public_, creds, sigs)).catch(e => (ctx.log("keeper", { what: `stake not claimed: ${e?.message ?? e}` }), null));
        if (c?.puts) {
          await putAll(c.puts);
          ctx.log("keeper", { what: `staked in the witness lot: ${c.amount} units of $CRAFT, claims epoch ${c.epoch}` });
        }
      }
      if (!self.hosted.length) return;
      // OPEN the epoch: binding and holdings.
      if (!self.open) {
        const r = await run(sigs => core.keeper_open(standingCode, beaconCode, beacon, public_, epoch, self.hosted, sigs));
        if (r?.puts) {
          await putAll(r.puts);
          state.opened++;
          ctx.log("keeper", { what: `epoch ${epoch}: holdings of ${state.files} file(s) (${state.assigned ?? 0} assigned), ${self.hosted.length} piece(s) committed; ${state.listed} file(s) listed in the registry` });
        }
        self.open = true;
      }
      // HANDED OVER (once an epoch): the identity delegate answers this epoch's challenges — and opens the next ones on
      // the same holdings — while this page is closed.
      if (self.mandated !== epoch) {
        const a = await auth.identity.keeperMandate(standingCode, beaconCode, beacon, epoch, self.hosted).catch(e => ({ refused: e?.message ?? e }));
        if (a?.keeperMandated) {
          self.mandated = epoch;
          ctx.log("keeper", { what: `epoch ${epoch}: the delegate keeps with no page open too` });
        } else ctx.log("keeper", { what: `not handed to the delegate (${a?.refused ?? "no answer"})` });
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
