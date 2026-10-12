//! TOKENS in the page's core: what every token action shares — a step's answer (`Want` read these, `Sign` this, `Done`),
//! the context (token code, beacon, key log code), this node as the account's DEVICE, a chain's params, a put — so the
//! editions (`editions`) and the wallet (`wallet`) build on one set. Pure, over what the page has read.

use craftworks_token_contract as t;
use craftworks_wallet::{Asking, Held, Wallet};
use t::{ChainParams, Definition, State};

/// A contract's state as the page read it: `None` not read yet, `Some(empty)` read and not there.
pub type Got<'a> = &'a dyn Fn(&[u8; 32]) -> Option<Vec<u8>>;

/// One step of an edition action.
#[derive(Debug, PartialEq)]
pub enum Step<T> {
    /// Read these contracts, then ask again.
    Want(Vec<[u8; 32]>),
    /// Have the identity delegate sign this (`TokenSign`), then ask again with its signature.
    Sign(Vec<u8>),
    Done(T),
}

/// What it works with: the token's code (its hash names every token contract), the beacon (the clock every
/// chain is stamped by) and the key log's code hash (a chain's owner's keys).
pub struct Ctx {
    pub code_hash: [u8; 32],
    pub beacon: [u8; 32],
    pub idlog: [u8; 32],
}

/// The account's DEVICE as the page knows it: its node key, the credential its owner key gave it (from the account's
/// devices), the key seq of that owner key, and the signatures the delegate gave so far.
pub struct Device {
    pub public: [u8; 32],
    pub credential: Vec<u8>,
    pub key_seq: u32,
    pub sigs: Vec<(Vec<u8>, [u8; 64])>,
}

/// A PUT: a token contract (by its params) and its state.
#[derive(Debug, Clone, PartialEq)]
pub struct Put {
    pub params: Vec<u8>,
    pub state: Vec<u8>,
}

pub fn short(b: &[u8; 32]) -> String {
    b[..6].iter().map(|x| format!("{x:02x}")).collect()
}

pub fn there(got: Got, id: &[u8; 32]) -> Result<Option<Vec<u8>>, [u8; 32]> {
    match got(id) {
        None => Err(*id),
        Some(b) if b.is_empty() => Ok(None),
        Some(b) => Ok(Some(b)),
    }
}

/// A chain's params: `owner`'s chain of `token` (`item`: a numbered copy's, else zero) — on the TOKEN's own beacon and
/// key log (its definition's), whatever this page's clock.
pub fn chain(ctx: &Ctx, d: &Definition, token: &[u8; 32], owner: &[u8; 32], item: [u8; 32]) -> ChainParams {
    ChainParams { code_hash: ctx.code_hash, definition: *token, owner: *owner, beacon: d.beacon, idlog: d.idlog, item }
}

/// A token this page can WRITE: one stamped by this page's clock (`ctx.beacon` — the heights it reads are that
/// beacon's; a token on another beacon is read here, never written).
pub fn on_our_clock(ctx: &Ctx, d: &Definition) -> Result<(), String> {
    if d.beacon != ctx.beacon {
        return Err(format!("this token runs on another clock (beacon {}), not this site's", short(&d.beacon)));
    }
    Ok(())
}

pub fn device_of(dev: &Device) -> Asking {
    Asking::new(dev.public, dev.credential.clone(), dev.key_seq, dev.sigs.clone())
}

/// This node's DEVICE from the account's credentials as the page holds them (its group's): the one for this node key,
/// its key seq the key log's event whose key signed it (what the token checks a writer by).
pub fn device(public: [u8; 32], creds: &[Vec<u8>], keys: &craftworks_idlog_contract::Log, sigs: Vec<(Vec<u8>, [u8; 64])>) -> Result<Device, &'static str> {
    for c in creds {
        let Some((_, owner, node, _)) = craftworks_identity::read_credential(c) else { continue };
        if node != public {
            continue;
        }
        if let Some(seq) = keys.events.iter().position(|e| e.key == owner) {
            return Ok(Device { public, credential: c.clone(), key_seq: seq as u32, sigs });
        }
    }
    Err("no credential of this node on the account's key log")
}

/// An account's CHAIN of a token as read (`tok`: the token and, for a numbered copy, its item), with the token's
/// definition and the account's key log. A chain not there is EMPTY when the account is the token's creator with a
/// genesis (it begins it: [`begin`]); anyone else's chain not there is `Ok(None)` — it holds none.
pub fn holding(ctx: &Ctx, got: Got, tok: &t::Tok, owner: &[u8; 32]) -> Result<Step<Option<(Held, craftworks_idlog_contract::Log)>>, String> {
    let d = match there(got, &tok.definition) {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(None) => return Err(format!("the token {} is not there", short(&tok.definition))),
        Ok(Some(b)) => Definition::decode(&b).ok_or("not a token definition")?,
    };
    let p = chain(ctx, &d, &tok.definition, owner, tok.item);
    let mut want = Vec::new();
    let mut read = |x: [u8; 32]| match there(got, &x) {
        Err(w) => {
            want.push(w);
            None
        }
        Ok(b) => Some(b),
    };
    let (mine, keys) = (read(p.id()), read(p.keys_id()));
    let (Some(mine), Some(keys)) = (mine, keys) else { return Ok(Step::Want(want)) };
    let state = match mine {
        Some(b) => State::decode(&b).ok_or("the account's chain does not read")?,
        None if d.genesis_owner == *owner && d.genesis_amount > 0 => State::default(),
        None => return Ok(Step::Done(None)),
    };
    let keys = craftworks_idlog_contract::read(owner, &keys.ok_or("the account's key log is not there")?).ok_or("the account's key log does not verify")?;
    Ok(Step::Done(Some((Held { params: p, definition: d, state, blocks: Default::default() }, keys))))
}

/// BEGIN `held` if it is a creator's chain not begun: its genesis appended. `Some(message)`: the delegate signs it first.
pub fn begin(w: &Wallet, asking: &Asking, held: &mut Held, now: u64) -> Result<Option<Vec<u8>>, String> {
    if !held.state.entries.is_empty() || held.definition.genesis_owner != held.params.owner || held.definition.genesis_amount == 0 {
        return Ok(None);
    }
    let g = w.genesis(held, now);
    if let Some(m) = asking.next() {
        return Ok(Some(m));
    }
    held.state.entries.extend(g.map_err(String::from)?);
    Ok(None)
}

/// Where an entry's ACCEPTANCE is read from (TOKEN §7a): the stake POOL's token (whose claims the lot draws from), the
/// BAG and BEACON codes' hashes (the witness queue's and the stake pointers' bags; the beacon's outputs log, for heights
/// its state no longer keeps), and the KEY LOG code's (with the pool, what names a staker's pool chain). A place's
/// signatures are in their own container of the token code ([`witness_at`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lot {
    pub pool: [u8; 32],
    pub bag: [u8; 32],
    pub beacon_code: [u8; 32],
    pub idlog: [u8; 32],
}

/// Is entry `seq` of chain `p` (its state `s`, its token `d`) ACCEPTED — enough of its committee signed it
/// (`witness::accepted`: drawn seats and, while the network is small, the baseline's)? `Done(Ok(n))`: n counted;
/// `Done(Err(why))`: not (yet) — not witnessed enough, or its lot not drawn yet. What a wallet credits money on.
pub fn accepted(lot: &Lot, got: Got, p: &ChainParams, d: &Definition, s: &State, seq: u64) -> Result<Step<Result<usize, String>>, String> {
    let i = s.entries.iter().position(|e| e.seq == seq).ok_or("the entry not held")?;
    let e = &s.entries[i];
    let (epoch, h) = t::wallet::lot_of(e, i.checked_sub(1).map(|j| &s.entries[j]), d.margin).map_err(String::from)?;
    accepted_at(lot, got, p.code_hash, d.beacon, &p.id(), seq, &e.hash(), epoch, h)
}

/// Is the epoch statement of standing epoch `epoch` with ROOT `root` ACCEPTED — its committee (drawn at
/// `statement::lot_height`) signed that root, at its `statement::witness_place`? What a payout by it rests on: a root
/// anyone may post, value only once witnessed (TOKEN §7d).
pub fn statement_accepted(lot: &Lot, got: Got, code_hash: [u8; 32], beacon: [u8; 32], epoch: u64, root: &[u8; 32]) -> Result<Step<Result<usize, String>>, String> {
    use craftworks_reputation::statement::{lot_height, witness_place};
    let h = lot_height(epoch);
    accepted_at(lot, got, code_hash, beacon, &witness_place(epoch), 0, root, h / t::supply::EPOCH, h)
}

/// A PLACE'S WITNESS SIGNATURES' container (TOKEN §7a, `witness::Params`): (`chain`, `seq`) under claims epoch `epoch`
/// — the claims for that epoch in the signers' pool chains (under `beacon`) say who may sign there.
pub fn witness_at(code_hash: [u8; 32], lot: &Lot, beacon: [u8; 32], chain: &[u8; 32], seq: u64, epoch: u64) -> t::witness::Params {
    t::witness::Params { code_hash, pool: lot.pool, beacon, idlog: lot.idlog, epoch, chain: *chain, seq }
}

/// Bag `k` of the STAKE POINTERS of `epoch` (TOKEN §7a, C2b: a kind of the token contract, `pointers::Params` — it
/// freezes at the epoch's freeze), for the stake pool of `lot` on `beacon`.
pub fn pointer_bag(code_hash: [u8; 32], lot: &Lot, beacon: [u8; 32], epoch: u64, k: u32) -> t::pointers::Params {
    t::pointers::Params { code_hash, pool: lot.pool, beacon, epoch, k }
}

/// Every POINTER posted for `epoch`, as `read` reads their bags — the chained rule: read on past a FULL bag, stop at the
/// first with room (a bag not read: the reads stop there — the caller asks for it and evaluates again).
pub fn pointers_of(code_hash: [u8; 32], lot: &Lot, beacon: [u8; 32], read: &dyn Fn([u8; 32]) -> Option<Vec<u8>>, epoch: u64) -> Vec<t::pointers::Pointer> {
    let mut out = Vec::new();
    for k in 0..4096u32 {
        let p = pointer_bag(code_hash, lot, beacon, epoch, k);
        let Some(items) = read(p.id()).and_then(|b| t::pointers::read(&p, &b)) else { break };
        out.extend(items.iter().filter_map(|i| t::pointers::Pointer::decode(craftworks_bag_contract::payload(i))));
        if !t::pointers::full(&items) {
            break;
        }
    }
    out
}

/// Its SIGNATURES as `read` reads them (none if not there) — what every reader here reads a place's signatures by.
pub fn witness_sigs(read: &dyn Fn([u8; 32]) -> Option<Vec<u8>>, p: &t::witness::Params) -> Vec<t::witness::WitnessSig> {
    read(p.id()).map(|b| p.sigs(&b)).unwrap_or_default()
}

/// A lot's memo of `V(E)` per claims epoch, for one evaluation (`stakers`).
pub type StakeMemo = std::cell::RefCell<std::collections::BTreeMap<u64, Vec<t::draw::StakeClaim>>>;

/// The VALID STAKERS of claims epoch `epoch` on `lot` (`wallet::valid_stakers`, TOKEN §7a): every read through `read`
/// (a contract not read: `None` — the caller records it and asks for it, then evaluates again).
pub fn stakers(code_hash: [u8; 32], lot: &Lot, got: Got, beacon: [u8; 32], read: &dyn Fn([u8; 32]) -> Option<Vec<u8>>, epoch: u64, memo: &StakeMemo) -> Vec<t::draw::StakeClaim> {
    let Some(pd) = read(lot.pool).and_then(|b| Definition::decode(&b)) else { return Vec::new() };
    let pointers = |ep: u64| pointers_of(code_hash, lot, beacon, read, ep);
    let chain = |owner: &[u8; 32]| {
        let cp = ChainParams { code_hash, definition: lot.pool, owner: *owner, beacon: pd.beacon, idlog: pd.idlog, item: [0; 32] };
        read(cp.id()).and_then(|b| State::decode(&b)).map(|st| (cp, st))
    };
    let out = |h: u64| match crate::witness::beacon_out(&lot.beacon_code, got, &beacon, h) {
        Err(w) => read(w).and(None),
        Ok(r) => r.ok(),
    };
    let sigs = |c: &[u8; 32], sq: u64, ep: u64| witness_sigs(read, &witness_at(code_hash, lot, beacon, c, sq, ep));
    t::wallet::valid_stakers(&t::wallet::StakeReads { pool: lot.pool, pool_def: &pd, pointers: &pointers, chain: &chain, out: &out, sigs: &sigs }, epoch, memo)
}

/// The VALID STAKERS of claims epoch `epoch` as a step (`stakers`): `Want` what is not read yet.
pub fn valid_of(ctx: &Ctx, lot: &Lot, got: Got, epoch: u64) -> Step<Vec<t::draw::StakeClaim>> {
    let want = std::cell::RefCell::new(Vec::new());
    let read = |x: [u8; 32]| match there(got, &x) {
        Err(w) => {
            want.borrow_mut().push(w);
            None
        }
        Ok(b) => b,
    };
    let v = stakers(ctx.code_hash, lot, got, ctx.beacon, &read, epoch, &StakeMemo::default());
    let mut w = want.into_inner();
    if w.is_empty() {
        return Step::Done(v);
    }
    w.sort();
    w.dedup();
    Step::Want(w)
}

/// ACCEPTANCE at a PLACE (`place`, `seq`: a chain entry's, or a statement's) of `entry` (its hash, or a root), drawn
/// from claims epoch `epoch` at beacon height `h`.
#[allow(clippy::too_many_arguments)]
fn accepted_at(lot: &Lot, got: Got, code_hash: [u8; 32], beacon_id: [u8; 32], place: &[u8; 32], seq: u64, entry: &[u8; 32], epoch: u64, h: u64) -> Result<Step<Result<usize, String>>, String> {
    use t::{wallet, witness};
    // Its lot's OUTPUT (the beacon's state, else its outputs log: `witness::beacon_out`).
    let out = match crate::witness::beacon_out(&lot.beacon_code, got, &beacon_id, h) {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(Err(why)) => return Ok(Step::Done(Err(why.into()))),
        Ok(Ok(o)) => o,
    };
    // The epoch's VALID STAKERS (never the raw registry) and the place's SIGNATURES.
    let want = std::cell::RefCell::new(Vec::new());
    let read = |x: [u8; 32]| match there(got, &x) {
        Err(w) => {
            want.borrow_mut().push(w);
            None
        }
        Ok(b) => b,
    };
    let memo = StakeMemo::default();
    let valid = stakers(code_hash, lot, got, beacon_id, &read, epoch, &memo);
    let sigs = witness_sigs(&read, &witness_at(code_hash, lot, beacon_id, place, seq, epoch));
    let mut w = want.into_inner();
    if !w.is_empty() {
        w.sort();
        w.dedup();
        return Ok(Step::Want(w));
    }
    Ok(Step::Done(witness::accepted(&lot.pool, place, seq, entry, &out, epoch, &valid, &sigs, wallet::COMMITTEE, &|o| wallet::stake_from(&valid, o)).map_err(String::from)))
}

/// A REAL LOT for tests (TOKEN §7a): six stakers (so the baseline has no seat — no private key needed), each a DID whose
/// pool chain stakes 1,000 before epoch 1 (genesis to the first, paid on, staked — every entry signed by its device),
/// their signed claims for epoch 1, a beacon past every lot and its outputs log; and `witness`: an entry signed by its
/// drawn committee into its place's signatures container.
#[cfg(any(test, feature = "testlot"))]
pub mod testlot {
    use super::*;
    use craftworks_wallet::{Held, Sources, Wallet};
    use ed25519_dalek::{Signer as _, SigningKey};
    use std::collections::HashMap;
    use t::{wallet, witness, Body};

    pub const POOL_GEN: u128 = 6_000;
    /// The test beacon's height: past every lot of epoch 1 (`supply::EPOCH` steps an epoch).
    const TOP: u64 = 3 * t::supply::EPOCH;
    pub const E1: u64 = t::supply::EPOCH;

    struct Dev {
        key: SigningKey,
        cred: Vec<u8>,
    }
    impl craftworks_wallet::Device for Dev {
        fn public(&self) -> [u8; 32] {
            self.key.verifying_key().to_bytes()
        }
        fn sign(&self, m: &[u8]) -> [u8; 64] {
            self.key.sign(m).to_bytes()
        }
        fn credential(&self) -> Vec<u8> {
            self.cred.clone()
        }
        fn key_seq(&self) -> u32 {
            0
        }
    }

    pub struct TestLot {
        pub lot: Lot,
        /// Each staker's device key (what its claim names).
        pub keys: Vec<SigningKey>,
        /// Each staker's pool chain (its params and state).
        pub staked: Vec<(ChainParams, State)>,
    }

    impl TestLot {
        /// THE BASELINE signs each staker's last entry before epoch 1 — its CLAIM for epoch 1 (its seat alone accepts:
        /// no stakers before) — at beacon height `at`, and its POINTER is posted for epoch 1 with that signature: before
        /// epoch 1's freeze (`pointers::frozen_at`) their stake is VALID in epoch 1, after it not
        /// (`wallet::valid_stakers`).
        pub fn baseline_signs(&self, ctx: &Ctx, got: &mut HashMap<[u8; 32], Vec<u8>>, at: u64) {
            let baseline = SigningKey::from_bytes(&witness::BASELINE_SEED);
            let margin = Definition::decode(&got[&self.lot.pool]).unwrap().margin;
            let mut pointers = Vec::new();
            for (cp, st) in &self.staked {
                let e = st.entries.last().unwrap();
                let (ep, h) = wallet::lot_of(e, st.entries.len().checked_sub(2).map(|j| &st.entries[j]), margin).unwrap();
                let wp = witness_at(ctx.code_hash, &self.lot, ctx.beacon, &cp.id(), e.seq, ep);
                let sig = witness::WitnessSig { chain: cp.id(), seq: e.seq, entry: e.hash(), at, sig: baseline.sign(&witness::WitnessSig::message(&cp.id(), e.seq, &e.hash(), at)).to_bytes() };
                got.insert(wp.id(), wp.state(&[(witness::BASELINE, [0; 32], sig.clone())]));
                pointers.push(t::pointers::Pointer { owner: cp.owner, seq: e.seq, entry: e.hash(), height: h, at, sigs: vec![(witness::BASELINE, at, sig.sig)] });
            }
            post_pointers(self, ctx, got, 1, &pointers);
        }
    }

    /// POST `pointers` for `epoch` into its pointer bags (`pointers::address`), as writers do: each into the first bag
    /// with room (`chained::full`), the bags' items kept.
    pub fn post_pointers(tl: &TestLot, ctx: &Ctx, got: &mut HashMap<[u8; 32], Vec<u8>>, epoch: u64, pointers: &[t::pointers::Pointer]) {
        let mut p = pointer_bag(ctx.code_hash, &tl.lot, ctx.beacon, epoch, 0);
        let mut items: Vec<Vec<u8>> = got.get(&p.id()).and_then(|b| t::pointers::read(&p, b)).unwrap_or_default();
        for ptr in pointers {
            while t::pointers::full(&items) {
                got.insert(p.id(), craftworks_bag_contract::encode(&p.address(), &items));
                p.k += 1;
                items = got.get(&p.id()).and_then(|b| t::pointers::read(&p, b)).unwrap_or_default();
            }
            items.push(craftworks_bag_contract::grind(&p.address(), &ptr.encode()));
        }
        got.insert(p.id(), craftworks_bag_contract::encode(&p.address(), &items));
    }

    /// The POINTER to entry `seq` of pool chain `cp` (state `st`) for `epoch`: the signatures its container holds.
    pub fn pointer_to(tl: &TestLot, got: &HashMap<[u8; 32], Vec<u8>>, ctx: &Ctx, cp: &ChainParams, st: &State, seq: u64, at: u64) -> t::pointers::Pointer {
        let margin = Definition::decode(&got[&tl.lot.pool]).unwrap().margin;
        let i = st.entries.iter().position(|e| e.seq == seq).unwrap();
        let e = &st.entries[i];
        let (ep, h) = wallet::lot_of(e, i.checked_sub(1).map(|j| &st.entries[j]), margin).unwrap();
        let wp = witness_at(ctx.code_hash, &tl.lot, ctx.beacon, &cp.id(), seq, ep);
        let sigs = got.get(&wp.id()).and_then(|b| witness::Signatures::decode(b)).map_or(vec![], |s| s.items.iter().filter(|x| x.entry == e.hash()).map(|x| (x.key, x.at, x.sig)).collect());
        t::pointers::Pointer { owner: cp.owner, seq, entry: e.hash(), height: h, at, sigs }
    }

    /// `V(epoch)` as `got` reads (`valid_of`).
    pub fn seats(tl: &TestLot, got: &HashMap<[u8; 32], Vec<u8>>, code_hash: [u8; 32], beacon: [u8; 32], epoch: u64) -> Vec<t::draw::StakeClaim> {
        let ctx = Ctx { code_hash, beacon, idlog: tl.lot.idlog };
        done(&|g| Ok(valid_of(&ctx, &tl.lot, g, epoch)), got).unwrap()
    }

    /// Six stakers in `got`, for tokens on `ctx`'s beacon — their stake VALID in epoch 1 (the baseline signed their last
    /// entries long before its freeze).
    pub fn stakers(ctx: &Ctx, got: &mut HashMap<[u8; 32], Vec<u8>>) -> TestLot {
        let tl = stakers_unwitnessed(ctx, got);
        tl.baseline_signs(ctx, got, 0);
        tl
    }

    /// Six stakers in `got`, each pool chain ending in its CLAIM for epoch 1, NO witness has signed yet and no pointer
    /// posted (not valid stake in any epoch until signed: [`TestLot::baseline_signs`]).
    pub fn stakers_unwitnessed(ctx: &Ctx, got: &mut HashMap<[u8; 32], Vec<u8>>) -> TestLot {
        let mut people = Vec::new();
        for n in 0..6u8 {
            let owner_seed = [70 + n; 32];
            let next = SigningKey::from_bytes(&[90 + n; 32]).verifying_key().to_bytes();
            let ev = craftworks_idlog_contract::Event::signed(0, [0; 32], &owner_seed, craftworks_idlog_contract::commit(&next), [1; 32], [1; 32], vec![]).unwrap();
            let log = craftworks_idlog_contract::Log { events: vec![ev] };
            let did = log.events[0].id();
            let key = SigningKey::from_bytes(&[110 + n; 32]);
            let cred = craftworks_identity::credential(&did, &key.verifying_key().to_bytes(), &[7; 32], &owner_seed);
            people.push((did, log, Dev { key, cred }));
        }
        let pd = Definition { code_hash: ctx.code_hash, decimals: 0, min_send: 1, genesis_owner: people[0].0, genesis_amount: POOL_GEN, name: b"POOL".to_vec(), nonce: [3; 16], beacon: ctx.beacon, margin: 2, idlog: ctx.idlog, mint: None, freeze: None, ids: t::Ids::None, royalty: None, cap: 0, item: [0; 32], version: [0; 32], soulbound: false, transfer_fee: None, sale: None, keepers: None };
        let pool = t::id(&ctx.code_hash, &pd.encode());
        got.insert(pool, pd.encode());
        let held = |did: &[u8; 32]| Held { params: chain(ctx, &pd, &pool, did, [0; 32]), definition: pd.clone(), state: State::default(), blocks: Default::default() };
        // The first: genesis, 1,000 to each other, 1,000 staked.
        let (d0, l0, v0) = &people[0];
        let w0 = Wallet { device: v0, keys: l0 };
        let mut h0 = held(d0);
        h0.state.entries = w0.genesis(&h0, 1).unwrap();
        for (d, ..) in &people[1..] {
            h0.state.entries.extend(w0.debit(&h0, Body::Send { to: *d, amount: 1_000 }, 1).unwrap());
        }
        h0.state.entries.extend(w0.debit(&h0, Body::Stake { amount: 1_000 }, 2).unwrap());
        let first = h0.state.clone();
        // Its CLAIM for epoch 1 (stamped in epoch 0, before epoch 1's freeze).
        h0.state.entries.extend(w0.debit(&h0, Body::Claim { from_epoch: 1, amount: 1_000 }, 6).unwrap());
        got.insert(h0.params.id(), h0.state.encode());
        let mut staked = vec![(h0.params.clone(), h0.state.clone())];
        // The others: receive, then stake.
        for (d, l, v) in &people[1..] {
            let w = Wallet { device: v, keys: l };
            let mut h = held(d);
            let chains = [(*d0, first.clone())];
            h.state.entries = w.collect(&h, &Sources { chains: &chains, blocks: &Default::default(), programs: &[], shards: &[] }, 4).unwrap();
            h.state.entries.extend(w.debit(&h, Body::Stake { amount: 1_000 }, 5).unwrap());
            h.state.entries.extend(w.debit(&h, Body::Claim { from_epoch: 1, amount: 1_000 }, 6).unwrap());
            got.insert(h.params.id(), h.state.encode());
            staked.push((h.params.clone(), h.state.clone()));
        }
        for (d, l, _) in &people {
            got.insert(chain(ctx, &pd, &pool, d, [0; 32]).keys_id(), l.encode());
        }
        // A BEACON past every lot (one step, tiny params), and its outputs log for the heights before.
        let bp = craftworks_beacon_contract::Params { k: 16, segs: 1, sample: 1, window: 2, keep: 600, seed: [5; 32] };
        let top = craftworks_beacon_contract::State { steps: vec![craftworks_beacon_contract::Step::compute(&bp, [0; 32], TOP, [1; 32])], alts: vec![], frauds: vec![] };
        got.insert(ctx.beacon, craftworks_beacon_contract::wire::encode(&top));
        let lot = Lot { pool, bag: [41; 32], beacon_code: [42; 32], idlog: ctx.idlog };
        let span = craftworks_beacon_contract::log::SPAN;
        for index in 0..TOP / span + 1 {
            let entries = (index * span..((index + 1) * span).min(TOP)).filter(|h| *h > 0).map(|h| (h, *blake3::hash(&h.to_le_bytes()).as_bytes())).collect();
            got.insert(t::id(&lot.beacon_code, &craftworks_beacon_contract::log::Params { beacon: ctx.beacon, index }.encode()), craftworks_beacon_contract::log::Log { entries }.encode());
        }
        TestLot { lot, keys: people.into_iter().map(|p| p.2.key).collect(), staked }
    }

    /// A NEW TOKEN's first entries, stamped in epoch 1 (its creator staker 1, the test's): the token "G" made, its
    /// chain put in `got` — `(the queue item of its first entry, its chain's id, that entry's hash)`. Its first entry has
    /// nothing before it and cites nothing: what a seated witness may sign by the rule alone.
    pub fn genesis_in_epoch1(ctx: &Ctx, got: &mut HashMap<[u8; 32], Vec<u8>>) -> (craftworks_wallet::queue::Queued, [u8; 32], [u8; 32]) {
        let owner_seed = [71u8; 32];
        let next = SigningKey::from_bytes(&[91; 32]).verifying_key().to_bytes();
        let ev = craftworks_idlog_contract::Event::signed(0, [0; 32], &owner_seed, craftworks_idlog_contract::commit(&next), [1; 32], [1; 32], vec![]).unwrap();
        let log = craftworks_idlog_contract::Log { events: vec![ev] };
        let did = log.events[0].id();
        let key = SigningKey::from_bytes(&[111; 32]);
        let cred = craftworks_identity::credential(&did, &key.verifying_key().to_bytes(), &[7; 32], &owner_seed);
        let dev = Dev { key, cred };
        let d = Definition { code_hash: ctx.code_hash, decimals: 0, min_send: 1, genesis_owner: did, genesis_amount: 100, name: b"G".to_vec(), nonce: [9; 16], beacon: ctx.beacon, margin: 2, idlog: ctx.idlog, mint: None, freeze: None, ids: t::Ids::None, royalty: None, cap: 0, item: [0; 32], version: [0; 32], soulbound: false, transfer_fee: None, sale: None, keepers: None };
        let id = t::id(&ctx.code_hash, &d.encode());
        let mut h = Held { params: chain(ctx, &d, &id, &did, [0; 32]), definition: d.clone(), state: State::default(), blocks: Default::default() };
        h.state.entries = Wallet { device: &dev, keys: &log }.genesis(&h, E1 + 12).unwrap();
        got.insert(id, d.encode());
        got.insert(h.params.id(), h.state.encode());
        got.insert(h.params.keys_id(), log.encode());
        let first = &h.state.entries[0];
        (craftworks_wallet::queue::Queued { token: t::Tok { definition: id, item: [0; 32] }, owner: did, seq: first.seq }, h.params.id(), first.hash())
    }

    /// Entry `seq` of chain `p` (state `s`, token `d`) SIGNED by the first `n` members of its committee, into its
    /// place's signatures container.
    pub fn witness(tl: &TestLot, got: &mut HashMap<[u8; 32], Vec<u8>>, p: &ChainParams, d: &Definition, s: &State, seq: u64, n: usize) {
        let i = s.entries.iter().position(|e| e.seq == seq).unwrap();
        let e = &s.entries[i];
        let (epoch, h) = wallet::lot_of(e, i.checked_sub(1).map(|j| &s.entries[j]), d.margin).unwrap();
        let out = *blake3::hash(&h.to_le_bytes()).as_bytes();
        let claims = seats(tl, got, p.code_hash, d.beacon, epoch);
        let valid = t::draw::valid(&tl.lot.pool, epoch, &claims);
        let (k, _, _) = witness::seats(valid.len(), wallet::COMMITTEE);
        let members = t::draw::committee_of(&out, &p.id(), seq, &valid, k);
        let wp = witness_at(p.code_hash, &tl.lot, d.beacon, &p.id(), seq, epoch);
        let items: Vec<([u8; 32], [u8; 32], witness::WitnessSig)> = members.iter().take(n).map(|m| {
            let key = tl.keys.iter().find(|k| k.verifying_key().to_bytes() == claims[*m].key).unwrap();
            (claims[*m].key, claims[*m].owner, witness::WitnessSig { chain: p.id(), seq, entry: e.hash(), at: 0, sig: key.sign(&witness::WitnessSig::message(&p.id(), seq, &e.hash(), 0)).to_bytes() })
        }).collect();
        got.insert(wp.id(), wp.state(&items));
    }

    /// The epoch STATEMENT `root` signed by the first `n` members of its committee (at its witness place).
    pub fn witness_statement(tl: &TestLot, got: &mut HashMap<[u8; 32], Vec<u8>>, code_hash: [u8; 32], beacon: [u8; 32], epoch: u64, root: &[u8; 32], n: usize) {
        use craftworks_reputation::statement::{lot_height, witness_place};
        let h = lot_height(epoch);
        let ce = h / t::supply::EPOCH;
        let out = *blake3::hash(&h.to_le_bytes()).as_bytes();
        let place = witness_place(epoch);
        let claims = seats(tl, got, code_hash, beacon, ce);
        let valid = t::draw::valid(&tl.lot.pool, ce, &claims);
        let (k, _, _) = witness::seats(valid.len(), wallet::COMMITTEE);
        let members = t::draw::committee_of(&out, &place, 0, &valid, k);
        let wp = witness_at(code_hash, &tl.lot, beacon, &place, 0, ce);
        let items: Vec<([u8; 32], [u8; 32], witness::WitnessSig)> = members.iter().take(n).map(|m| {
            let key = tl.keys.iter().find(|k| k.verifying_key().to_bytes() == claims[*m].key).unwrap();
            (claims[*m].key, claims[*m].owner, witness::WitnessSig { chain: place, seq: 0, entry: *root, at: 0, sig: key.sign(&witness::WitnessSig::message(&place, 0, root, 0)).to_bytes() })
        }).collect();
        got.insert(wp.id(), wp.state(&items));
    }

    /// Resolve a step whose reads are all in `got` (a read not there: read and NOT THERE).
    pub fn done<T>(f: &dyn Fn(Got) -> Result<Step<T>, String>, got: &HashMap<[u8; 32], Vec<u8>>) -> Result<T, String> {
        let mut g = got.clone();
        for _ in 0..16 {
            let gg = g.clone();
            match f(&|x| gg.get(x).cloned())? {
                Step::Done(x) => return Ok(x),
                Step::Want(w) => w.into_iter().for_each(|x| {
                    g.entry(x).or_default();
                }),
                Step::Sign(_) => return Err("a step that signs".into()),
            }
        }
        Err("no end".into())
    }

    #[test]
    fn an_entry_counts_once_its_committee_signed_it() {
        let ctx = Ctx { code_hash: [9; 32], beacon: [7; 32], idlog: [6; 32] };
        let mut got = HashMap::new();
        let tl = stakers(&ctx, &mut got);
        // A token chain's entry in epoch 1 (its first: drawn at epoch 1's first step).
        let (_, pool_def) = (tl.lot.pool, Definition::decode(&got[&tl.lot.pool]).unwrap());
        let first_staker = pool_def.genesis_owner;
        let p = chain(&ctx, &pool_def, &tl.lot.pool, &first_staker, [0; 32]);
        let mut s = State::decode(&got[&p.id()]).unwrap();
        // Re-stamp a copy into epoch 1 for the check (its lot then falls in epoch 1, where the claims are).
        for e in &mut s.entries {
            e.stamp += E1;
        }
        let acc = |got: &HashMap<[u8; 32], Vec<u8>>| done(&|g| accepted(&tl.lot, g, &p, &pool_def, &s, 0), got);
        assert_eq!(acc(&got), Ok(Err("an entry its committee has not witnessed".into())));
        // 6 seats (no baseline at 6 live), 4 needed: 3 short, 4 enough, all 6 counted.
        for (n, want) in [(3, Err("an entry its committee has not witnessed".to_string())), (4, Ok(4)), (6, Ok(6))] {
            let mut g = got.clone();
            witness(&tl, &mut g, &p, &pool_def, &s, 0, n);
            assert_eq!(acc(&g), Ok(want), "{n} signed");
        }
    }
}
#[cfg(test)]
mod baseline_check {
    use super::*;
    use std::collections::HashMap;
    use t::{witness, Body};

    /// VALID STAKE (audit 10-11): a staker whose stake no witness signed is no staker. The six test stakers' chains
    /// are signed by the baseline (the lot's bootstrap); one's signature taken away, that one drops out of `V(1)`.
    #[test]
    fn only_witnessed_stake_is_valid_stake() {
        let ctx = Ctx { code_hash: [9; 32], beacon: [7; 32], idlog: [6; 32] };
        let mut got = HashMap::new();
        let tl = testlot::stakers(&ctx, &mut got);
        let v = |got: &HashMap<[u8; 32], Vec<u8>>| testlot::done(&|g| Ok(valid_of(&ctx, &tl.lot, g, 1)), got).unwrap();
        assert_eq!(v(&got).len(), 6, "every staker's stake witnessed");
        // Staker 2's last entry before epoch 1 (its claim): its signatures container emptied (its pointer still posted).
        let two = tl.staked[2].0.owner;
        let pd = Definition::decode(&got[&tl.lot.pool]).unwrap();
        let cp = ChainParams { code_hash: ctx.code_hash, definition: tl.lot.pool, owner: two, beacon: pd.beacon, idlog: pd.idlog, item: [0; 32] };
        let st = State::decode(&got[&cp.id()]).unwrap();
        let last = st.entries.last().unwrap();
        let (ep, _) = t::wallet::lot_of(last, st.entries.len().checked_sub(2).map(|j| &st.entries[j]), pd.margin).unwrap();
        let wp = witness_at(ctx.code_hash, &tl.lot, ctx.beacon, &cp.id(), last.seq, ep);
        got.insert(wp.id(), wp.state(&[]));
        let after = v(&got);
        assert_eq!(after.len(), 5, "its stake no longer counts");
        assert!(!after.iter().any(|c| c.owner == two));
    }

    /// The BASELINE's signature on standing epoch `epoch`'s statement `root` (claims epoch 1's lot), made at `at`.
    fn baseline_signs_statement(ctx: &Ctx, tl: &testlot::TestLot, got: &mut HashMap<[u8; 32], Vec<u8>>, epoch: u64, root: &[u8; 32], at: u64) {
        use craftworks_reputation::statement::{lot_height, witness_place};
        use ed25519_dalek::Signer as _;
        let (place, ce) = (witness_place(epoch), lot_height(epoch) / t::supply::EPOCH);
        let k = ed25519_dalek::SigningKey::from_bytes(&t::witness::BASELINE_SEED);
        let sig = t::witness::WitnessSig { chain: place, seq: 0, entry: *root, at, sig: k.sign(&t::witness::WitnessSig::message(&place, 0, root, at)).to_bytes() };
        let wp = witness_at(ctx.code_hash, &tl.lot, ctx.beacon, &place, 0, ce);
        got.insert(wp.id(), wp.state(&[(t::witness::BASELINE, [0; 32], sig)]));
    }

    /// THE MEASURED WEDGE (10-11), as a regression: a statement of claims epoch 1 ACCEPTED by the baseline alone while
    /// `V(1)` was empty (the stakers' chains not yet witnessed); a keeper mint cites it. Then the stakers' pre-epoch
    /// entries are accepted LATE — signatures made at or after epoch 1's freeze. `V(1)` must stay empty and the
    /// statement accepted: before the fix `V(1)` grew to 6, the baseline lost its seat, the statement needed 4 of 6 drawn
    /// seats and was no longer accepted — the mint citing it never accepted, its chain wedged for good.
    #[test]
    fn a_late_acceptance_of_stake_never_unaccepts_what_its_epoch_accepted() {
        let ctx = Ctx { code_hash: [9; 32], beacon: [7; 32], idlog: [6; 32] };
        let mut got = HashMap::new();
        let tl = testlot::stakers_unwitnessed(&ctx, &mut got);
        let (epoch, root) = (5u64, [0x5Au8; 32]); // standing epoch 5: its lot in claims epoch 1
        let freeze = t::pointers::frozen_at(1);
        let verdict = |g: &HashMap<[u8; 32], Vec<u8>>| testlot::done(&|gg| statement_accepted(&tl.lot, gg, ctx.code_hash, ctx.beacon, epoch, &root), g).unwrap();
        let valid = |g: &HashMap<[u8; 32], Vec<u8>>| testlot::done(&|gg| Ok(valid_of(&ctx, &tl.lot, gg, 1)), g).unwrap().len();
        assert_eq!(valid(&got), 0, "no staker witnessed yet");
        baseline_signs_statement(&ctx, &tl, &mut got, epoch, &root, craftworks_reputation::statement::lot_height(epoch));
        assert_eq!(verdict(&got), Ok(1), "accepted by the baseline's seat alone");
        // LATE: the stakers' last pre-epoch entries witnessed at the freeze, and hours after it.
        for at in [freeze, freeze + 1, 2 * t::supply::EPOCH] {
            tl.baseline_signs(&ctx, &mut got, at);
            assert_eq!(valid(&got), 0, "stake accepted at {at} (epoch 1 froze at {freeze}) grew V(1)");
            assert_eq!(verdict(&got), Ok(1), "a late stake acceptance (at {at}) unaccepted the statement");
        }
    }

    /// A staker whose pre-epoch entry was accepted BEFORE the freeze is valid stake — up to the last height before it.
    #[test]
    fn stake_accepted_before_the_freeze_is_valid_stake() {
        let ctx = Ctx { code_hash: [9; 32], beacon: [7; 32], idlog: [6; 32] };
        let mut got = HashMap::new();
        let tl = testlot::stakers_unwitnessed(&ctx, &mut got);
        tl.baseline_signs(&ctx, &mut got, t::pointers::frozen_at(1) - 1);
        let v = testlot::done(&|g| Ok(valid_of(&ctx, &tl.lot, g, 1)), &got).unwrap();
        assert_eq!(v.len(), 6, "witnessed one step before the freeze: all six valid");
        // So epoch 1's lots are drawn over six stakers: the baseline has no seat, its signature alone accepts nothing.
        baseline_signs_statement(&ctx, &tl, &mut got, 5, &[0x5A; 32], craftworks_reputation::statement::lot_height(5));
        assert!(testlot::done(&|g| statement_accepted(&tl.lot, g, ctx.code_hash, ctx.beacon, 5, &[0x5A; 32]), &got).unwrap().is_err());
    }

    /// `V(1)` read through `stakers` with a `read` that COUNTS the reads of the chains in `watch` (the network reads a
    /// flood of pointers would cost) — every other read from `got`.
    fn v1_counting(ctx: &Ctx, tl: &testlot::TestLot, got: &HashMap<[u8; 32], Vec<u8>>, watch: &std::collections::HashSet<[u8; 32]>) -> (Vec<t::draw::StakeClaim>, usize) {
        let n = std::cell::Cell::new(0usize);
        let read = |id: [u8; 32]| {
            if watch.contains(&id) {
                n.set(n.get() + 1);
            }
            got.get(&id).cloned()
        };
        let v = stakers(ctx.code_hash, &tl.lot, &|x| got.get(x).cloned(), ctx.beacon, &read, 1, &StakeMemo::default());
        (v, n.get())
    }

    /// THE FILL ATTACK (audit C2b), on the new code: the old registry refused the real claim once 4,096 forged ones
    /// filled it. Now a claim is an entry of the staker's own chain, and the index is POINTERS anyone posts: 4,096 forged
    /// pointers (made-up stakers; signatures that are junk, or real but by keys with no seat) fill the epoch's first
    /// bags, and ONE real staker's pointer lands after them. `V(1)` holds the real staker only, and not one forged
    /// pointer's chain is read (the CPU prefilter, `Pointer::accepted`). NEGATIVE CONTROL: the prefilter off — every
    /// forged pointer's chain is read (4,096 reads; the count assertion fails).
    #[test]
    fn a_flood_of_forged_pointers_seats_nobody_and_reads_no_chain() {
        use ed25519_dalek::Signer as _;
        let ctx = Ctx { code_hash: [9; 32], beacon: [7; 32], idlog: [6; 32] };
        let mut got = HashMap::new();
        let tl = testlot::stakers_unwitnessed(&ctx, &mut got);
        let pd = Definition::decode(&got[&tl.lot.pool]).unwrap();
        // The real staker (0): its claim signed by the baseline before the freeze (`baseline_signs` signs all six and
        // posts their pointers — the others' pointers taken back out: only staker 0 points).
        let mut g = got.clone();
        tl.baseline_signs(&ctx, &mut g, 0);
        let real = testlot::pointer_to(&tl, &g, &ctx, &tl.staked[0].0, &tl.staked[0].1, tl.staked[0].1.entries.last().unwrap().seq, 10);
        let cp0 = &tl.staked[0].0;
        let wp0 = witness_at(ctx.code_hash, &tl.lot, ctx.beacon, &cp0.id(), real.seq, 0);
        got.insert(wp0.id(), g[&wp0.id()].clone());
        // THE FLOOD: 4,096 forged pointers — made-up owners, a lot height in epoch 0, each "signed" by the baseline with
        // junk bytes or by a fresh key that holds no seat.
        let forged: Vec<t::pointers::Pointer> = (0..4096u32).map(|n| {
            let owner = *blake3::hash(&[&n.to_le_bytes()[..], b"forged owner"].concat()).as_bytes();
            let entry = *blake3::hash(&[&n.to_le_bytes()[..], b"forged entry"].concat()).as_bytes();
            let chain = chain(&ctx, &pd, &tl.lot.pool, &owner, [0; 32]).id();
            let sig = if n % 2 == 0 {
                (witness::BASELINE, 0, [(n % 251) as u8; 64])
            } else {
                let k = ed25519_dalek::SigningKey::from_bytes(blake3::hash(&n.to_le_bytes()).as_bytes());
                (k.verifying_key().to_bytes(), 0, k.sign(&witness::WitnessSig::message(&chain, 3, &entry, 0)).to_bytes())
            };
            t::pointers::Pointer { owner, seq: 3, entry, height: 9, at: 10, sigs: vec![sig] }
        }).collect();
        let watch: std::collections::HashSet<[u8; 32]> = forged.iter().map(|p| chain(&ctx, &pd, &tl.lot.pool, &p.owner, [0; 32]).id()).collect();
        testlot::post_pointers(&tl, &ctx, &mut got, 1, &forged);
        testlot::post_pointers(&tl, &ctx, &mut got, 1, &[real.clone()]);
        let bags = (0..).take_while(|k| got.contains_key(&pointer_bag(ctx.code_hash, &tl.lot, ctx.beacon, 1, *k).id())).count();
        assert!(bags >= 5, "the flood fills the first bags: {bags}");
        let (v, reads) = v1_counting(&ctx, &tl, &got, &watch);
        println!("NEW INDEX FILL: {} forged pointers in {bags} bags + 1 real claim; V(1) = {:?}; forged chains read: {reads}", forged.len(), v.iter().map(|c| short(&c.owner)).collect::<Vec<_>>());
        assert_eq!(v.iter().map(|c| c.owner).collect::<Vec<_>>(), vec![cp0.owner], "the real staker, and only it");
        assert_eq!(v[0].key, tl.keys[0].verifying_key().to_bytes());
        assert_eq!(reads, 0, "no forged pointer's chain read");
        // NEGATIVE CONTROL: the prefilter off — still no forged seat (their chains are not there), but every one read.
        t::wallet::switches::PREFILTER.with(|c| c.set(false));
        let (v_off, reads_off) = v1_counting(&ctx, &tl, &got, &watch);
        t::wallet::switches::PREFILTER.with(|c| c.set(true));
        println!("CONTROL (prefilter off): V(1) = {} staker(s); forged chains read: {reads_off} — `reads == 0` fails", v_off.len());
        assert_eq!(reads_off, forged.len(), "without the prefilter each forged pointer costs a chain read");
    }

    /// A POINTER POSTED AFTER THE FREEZE (review 10-12: `V(E)` must be the same for every reader). The six stakers'
    /// claims accepted long before epoch 1's freeze, but their pointers posted only after it (dated then — the bags take
    /// nothing from the freeze on, `pointers::take`; one put whole later is ignored here): `V(1)` is empty, as for a
    /// reader that never saw them. Posted before the freeze: all six. NEGATIVE CONTROL: the freeze check off — the late
    /// pointers count (the `V(1) == 0` assertion fails).
    #[test]
    fn a_pointer_posted_after_the_freeze_is_ignored_by_every_reader() {
        let ctx = Ctx { code_hash: [9; 32], beacon: [7; 32], idlog: [6; 32] };
        let freeze = t::pointers::frozen_at(1);
        let mut signed = HashMap::new();
        let tl = testlot::stakers_unwitnessed(&ctx, &mut signed);
        let base = signed.clone();
        tl.baseline_signs(&ctx, &mut signed, 0);
        // The witness containers as signed, the pointer bags as they were before (none).
        let mut got = base.clone();
        for (k, v) in &signed {
            if !base.contains_key(k) && !(0..8).any(|j| pointer_bag(ctx.code_hash, &tl.lot, ctx.beacon, 1, j).id() == *k) {
                got.insert(*k, v.clone());
            }
        }
        let late: Vec<t::pointers::Pointer> = tl.staked.iter().map(|(cp, st)| testlot::pointer_to(&tl, &got, &ctx, cp, st, st.entries.last().unwrap().seq, freeze + 5)).collect();
        let mut g_late = got.clone();
        testlot::post_pointers(&tl, &ctx, &mut g_late, 1, &late);
        let v = |g: &HashMap<[u8; 32], Vec<u8>>| testlot::seats(&tl, g, ctx.code_hash, ctx.beacon, 1).len();
        assert_eq!(v(&got), 0, "no pointer: nobody found");
        let n_late = v(&g_late);
        println!("LATE POINTERS: 6 claims accepted before the freeze ({freeze}), pointed to at {}: V(1) = {n_late}", freeze + 5);
        assert_eq!(n_late, 0, "a pointer posted after the freeze grew V(1)");
        // The same pointers dated before the freeze: all six.
        let early: Vec<t::pointers::Pointer> = late.iter().map(|p| t::pointers::Pointer { at: freeze - 1, ..p.clone() }).collect();
        let mut g_early = got.clone();
        testlot::post_pointers(&tl, &ctx, &mut g_early, 1, &early);
        assert_eq!(v(&g_early), 6, "pointed to before the freeze");
        // NEGATIVE CONTROL: the freeze check off.
        t::wallet::switches::LATE.with(|c| c.set(false));
        let off = v(&g_late);
        t::wallet::switches::LATE.with(|c| c.set(true));
        println!("CONTROL (freeze check off): V(1) = {off} — `V(1) == 0` fails");
        assert_eq!(off, 6);
    }

    /// A POINTER TO A REAL BUT UNWITNESSED CLAIM (the six stakers' claims, no witness signed them; pointers posted with
    /// junk signatures) seats nobody: the prefilter refuses the pointers, and the chains' acceptance check refuses the
    /// claims. NEGATIVE CONTROL: both checks off — all six counted (the `V(1)` empty assertion fails).
    #[test]
    fn a_pointer_to_an_unwitnessed_claim_seats_nobody() {
        let ctx = Ctx { code_hash: [9; 32], beacon: [7; 32], idlog: [6; 32] };
        let mut got = HashMap::new();
        let tl = testlot::stakers_unwitnessed(&ctx, &mut got);
        let pointers: Vec<t::pointers::Pointer> = tl.staked.iter().map(|(cp, st)| {
            let mut p = testlot::pointer_to(&tl, &got, &ctx, cp, st, st.entries.last().unwrap().seq, 10);
            p.sigs = vec![(witness::BASELINE, 0, [7; 64])];
            p
        }).collect();
        testlot::post_pointers(&tl, &ctx, &mut got, 1, &pointers);
        let v = |g: &HashMap<[u8; 32], Vec<u8>>| testlot::seats(&tl, g, ctx.code_hash, ctx.beacon, 1).len();
        assert_eq!(v(&got), 0, "an unwitnessed claim seats nobody");
        // Prefilter alone off: the chains are read, and the acceptance check refuses them.
        t::wallet::switches::PREFILTER.with(|c| c.set(false));
        assert_eq!(v(&got), 0, "the acceptance check alone refuses them");
        // NEGATIVE CONTROL: both off.
        t::wallet::switches::ACCEPTANCE.with(|c| c.set(false));
        let off = v(&got);
        t::wallet::switches::PREFILTER.with(|c| c.set(true));
        t::wallet::switches::ACCEPTANCE.with(|c| c.set(true));
        println!("CONTROL (prefilter and acceptance off): V(1) = {off} — `V(1) == 0` fails");
        assert_eq!(off, 6);
    }

    /// LIVENESS (owner 10-12: "stakers offline must not stall epochs"): staker 0, valid in epoch 1, claims epoch 2 (its
    /// claim stamped in epoch 1). Its seated witnesses run THE WITNESS RULE (`witness::entry` → `wallet::may_witness`):
    /// with no standing record answering in epoch 1 the claim is REFUSED — never signed, so staker 0 is not in `V(2)`;
    /// with one (an answer at a height in epoch 1, before the claim) it is signed, pointed to, and staker 0 is in `V(2)`.
    /// NEGATIVE CONTROL: the liveness check off — the claim of a node that answered nothing is signed and counted.
    #[test]
    fn a_claim_seats_its_staker_only_if_it_answered_last_epoch() {
        let ctx = Ctx { code_hash: [9; 32], beacon: [7; 32], idlog: [6; 32] };
        let standing = [43u8; 32];
        let mut got = HashMap::new();
        let tl = testlot::stakers(&ctx, &mut got);
        assert_eq!(testlot::seats(&tl, &got, ctx.code_hash, ctx.beacon, 1).len(), 6);
        let pd = Definition::decode(&got[&tl.lot.pool]).unwrap();
        let (cp, st) = tl.staked[0].clone();
        let key = tl.keys[0].verifying_key().to_bytes();
        // Its CLAIM for epoch 2, stamped in epoch 1.
        let stamp = testlot::E1 + 100;
        let mut s2 = st.clone();
        let mut e = t::Entry { seq: st.entries.last().unwrap().seq + 1, prev: st.head(), nonce: [0xc2; 16], stamp, body: Body::Claim { from_epoch: 2, amount: 1_000 }, balance: st.balance(), auth: t::Auth::Work(0) };
        e.auth = t::Auth::Sig(ed25519_dalek::Signer::sign(&tl.keys[0], &e.message()).to_bytes());
        s2.entries.push(e.clone());
        got.insert(cp.id(), s2.encode());
        let q = craftworks_wallet::queue::Queued { token: t::Tok { definition: tl.lot.pool, item: [0; 32] }, owner: cp.owner, seq: e.seq };
        let record_id = craftworks_standing_contract::Params::keeper(tl.lot.beacon_code, ctx.beacon, &key).id(&standing);
        let answered = craftworks_standing::record::State { epochs: vec![craftworks_standing::record::Epoch { epoch: craftworks_standing::epoch(testlot::E1 + 50), answers: vec![craftworks_standing::Answer { key, peer: craftworks_standing::keeper_peer(&key), height: testlot::E1 + 50, picks: vec![], sig: [0; 64] }], ..Default::default() }] };
        // Each seated witness's verdict (the page's rule) — and, where it signs, the signatures and the pointer.
        let run = |got: &HashMap<[u8; 32], Vec<u8>>| -> (Vec<crate::witness::Verdict>, usize) {
            let verdicts: Vec<crate::witness::Verdict> = tl.keys.iter().map(|k| testlot::done(&|g| crate::witness::entry(&ctx, &tl.lot, g, &q, &k.verifying_key().to_bytes(), stamp + 20, &standing), got).unwrap()).filter(|v| *v != crate::witness::Verdict::NotSeated).collect();
            let mut g = got.clone();
            if verdicts.iter().all(|v| matches!(v, crate::witness::Verdict::Sign { .. })) {
                testlot::witness(&tl, &mut g, &cp, &pd, &s2, e.seq, verdicts.len());
                let ptr = testlot::pointer_to(&tl, &g, &ctx, &cp, &s2, e.seq, stamp + 30);
                testlot::post_pointers(&tl, &ctx, &mut g, 2, &[ptr]);
            }
            let v2 = testlot::seats(&tl, &g, ctx.code_hash, ctx.beacon, 2);
            (verdicts, v2.iter().filter(|c| c.owner == cp.owner).count())
        };
        // NO STANDING RECORD (or one that answered nothing in epoch 1): refused, not in V(2).
        got.insert(record_id, vec![]);
        let (verdicts, seated) = run(&got);
        assert!(!verdicts.is_empty() && verdicts.iter().all(|v| matches!(v, crate::witness::Verdict::Refused(w) if w.contains("answered nothing"))), "{verdicts:?}");
        assert_eq!(seated, 0, "a staker that answered nothing last epoch is not in V(2)");
        let stale = craftworks_standing::record::State { epochs: vec![craftworks_standing::record::Epoch { epoch: 1, answers: vec![craftworks_standing::Answer { height: 1100, ..answered.epochs[0].answers[0].clone() }], ..Default::default() }] };
        got.insert(record_id, craftworks_standing::codec::encode(&stale));
        assert_eq!(run(&got).1, 0, "an answer in the epoch BEFORE (epoch 0) proves nothing about epoch 1");
        // A RECORD THAT ANSWERED in epoch 1, before the claim: signed, pointed to, in V(2).
        let mut live = got.clone();
        live.insert(record_id, craftworks_standing::codec::encode(&answered));
        let (verdicts, seated) = run(&live);
        assert!(verdicts.iter().all(|v| matches!(v, crate::witness::Verdict::Sign { .. })), "{verdicts:?}");
        assert_eq!(seated, 1, "a live staker is in V(2)");
        // NEGATIVE CONTROL: liveness off — the silent node's claim is signed and counted.
        t::wallet::switches::LIVENESS.with(|c| c.set(false));
        got.insert(record_id, vec![]);
        let (_, seated_off) = run(&got);
        t::wallet::switches::LIVENESS.with(|c| c.set(true));
        println!("CONTROL (liveness off): the silent staker in V(2): {seated_off} — `seated == 0` fails");
        assert_eq!(seated_off, 1);
    }

    #[test]
    fn the_test_baseline_key_is_its_seeds() {
        use ed25519_dalek::SigningKey;
        let t = craftworks_token_contract::witness::BASELINE;
        assert_eq!(SigningKey::from_bytes(&craftworks_token_contract::witness::BASELINE_SEED).verifying_key().to_bytes(), t);
    }
}
