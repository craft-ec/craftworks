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

/// Where an entry's ACCEPTANCE is read from (TOKEN §7a): the stake POOL's token (whose claims the lot draws from), and
/// the BAG and BEACON codes' hashes (a witness bag's contract id; the beacon's outputs log, for heights its state no
/// longer keeps).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lot {
    pub pool: [u8; 32],
    pub bag: [u8; 32],
    pub beacon_code: [u8; 32],
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

/// ACCEPTANCE at a PLACE (`place`, `seq`: a chain entry's, or a statement's) of `entry` (its hash, or a root), drawn
/// from claims epoch `epoch` at beacon height `h`.
#[allow(clippy::too_many_arguments)]
/// A lot's memo of `V(E)` per claims epoch, for one evaluation (`stakers`).
pub type StakeMemo = std::cell::RefCell<std::collections::BTreeMap<u64, Vec<t::draw::StakeClaim>>>;

/// The VALID STAKERS of claims epoch `epoch` on `lot` (`wallet::valid_stakers`, TOKEN §7a): every read through `read`
/// (a contract not read: `None` — the caller records it and asks for it, then evaluates again).
pub fn stakers(code_hash: [u8; 32], lot: &Lot, got: Got, beacon: [u8; 32], read: &dyn Fn([u8; 32]) -> Option<Vec<u8>>, epoch: u64, memo: &StakeMemo) -> Vec<t::draw::StakeClaim> {
    let Some(pd) = read(lot.pool).and_then(|b| Definition::decode(&b)) else { return Vec::new() };
    let registry = |ep: u64| read(t::claims::Params { code_hash, pool: lot.pool, beacon, epoch: ep }.id()).and_then(|b| t::claims::Registry::decode(&b)).map_or(vec![], |r| r.claims);
    let chain = |owner: &[u8; 32]| {
        let cp = ChainParams { code_hash, definition: lot.pool, owner: *owner, beacon: pd.beacon, idlog: pd.idlog, item: [0; 32] };
        read(cp.id()).and_then(|b| State::decode(&b)).map(|st| (cp, st))
    };
    let out = |h: u64| match crate::witness::beacon_out(&lot.beacon_code, got, &beacon, h) {
        Err(w) => read(w).and(None),
        Ok(r) => r.ok(),
    };
    let sigs = |c: &[u8; 32], sq: u64| {
        let a = t::wallet::witness_bag(c, sq);
        read(t::id(&lot.bag, &a)).and_then(|b| craftworks_bag_contract::read(&a, &b)).unwrap_or_default().iter().filter_map(|x| t::wallet::bytes::read_witness_sig(craftworks_bag_contract::payload(x))).collect()
    };
    t::wallet::valid_stakers(&t::wallet::StakeReads { pool: lot.pool, pool_def: &pd, registry: &registry, chain: &chain, out: &out, sigs: &sigs }, epoch, memo)
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
    let bag_addr = wallet::witness_bag(place, seq);
    let sigs: Vec<witness::WitnessSig> = read(t::id(&lot.bag, &bag_addr)).and_then(|b| craftworks_bag_contract::read(&bag_addr, &b)).unwrap_or_default().iter().filter_map(|i| wallet::bytes::read_witness_sig(craftworks_bag_contract::payload(i))).collect();
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
/// drawn committee into its witness bag.
#[cfg(any(test, feature = "testlot"))]
pub mod testlot {
    use super::*;
    use craftworks_wallet::{Held, Sources, Wallet};
    use ed25519_dalek::{Signer as _, SigningKey};
    use std::collections::HashMap;
    use t::{draw::StakeClaim, wallet, witness, Body};

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
    }

    /// Six stakers in `got`, for tokens on `ctx`'s beacon.
    pub fn stakers(ctx: &Ctx, got: &mut HashMap<[u8; 32], Vec<u8>>) -> TestLot {
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
        got.insert(h0.params.id(), first.encode());
        let mut staked = vec![(h0.params.clone(), first.clone())];
        // The others: receive, then stake.
        for (d, l, v) in &people[1..] {
            let w = Wallet { device: v, keys: l };
            let mut h = held(d);
            let chains = [(*d0, first.clone())];
            h.state.entries = w.collect(&h, &Sources { chains: &chains, blocks: &Default::default(), programs: &[], shards: &[] }, 4).unwrap();
            h.state.entries.extend(w.debit(&h, Body::Stake { amount: 1_000 }, 5).unwrap());
            got.insert(h.params.id(), h.state.encode());
            staked.push((h.params.clone(), h.state.clone()));
        }
        for (d, l, _) in &people {
            got.insert(chain(ctx, &pd, &pool, d, [0; 32]).keys_id(), l.encode());
        }
        // Their CLAIMS for epoch 1.
        let mut claims: Vec<StakeClaim> = people.iter().map(|(d, _, v)| {
            let kb = v.key.verifying_key().to_bytes();
            StakeClaim { owner: *d, amount: 1_000, epoch: 1, key: kb, sig: v.key.sign(&StakeClaim::message(&pool, d, 1_000, 1, &kb)).to_bytes() }
        }).collect();
        claims.sort_by_key(|c| c.owner); // a registry lists them by owner
        got.insert(t::claims::Params { code_hash: ctx.code_hash, pool, beacon: ctx.beacon, epoch: 1 }.id(), t::claims::Registry { claims }.encode());
        // A BEACON past every lot (one step, tiny params), and its outputs log for the heights before.
        let bp = craftworks_beacon_contract::Params { k: 16, segs: 1, sample: 1, window: 2, keep: 600, seed: [5; 32] };
        let top = craftworks_beacon_contract::State { steps: vec![craftworks_beacon_contract::Step::compute(&bp, [0; 32], TOP, [1; 32])], alts: vec![], frauds: vec![] };
        got.insert(ctx.beacon, craftworks_beacon_contract::wire::encode(&top));
        let lot = Lot { pool, bag: [41; 32], beacon_code: [42; 32] };
        let span = craftworks_beacon_contract::log::SPAN;
        for index in 0..TOP / span + 1 {
            let entries = (index * span..((index + 1) * span).min(TOP)).filter(|h| *h > 0).map(|h| (h, *blake3::hash(&h.to_le_bytes()).as_bytes())).collect();
            got.insert(t::id(&lot.beacon_code, &craftworks_beacon_contract::log::Params { beacon: ctx.beacon, index }.encode()), craftworks_beacon_contract::log::Log { entries }.encode());
        }
        // THE BASELINE signs each staker's last entry before epoch 1 (no stakers yet: its seat alone accepts) — so their
        // stake is VALID stake (`wallet::valid_stakers`), as Wren's witness makes the network's first stakers'.
        let baseline = SigningKey::from_bytes(&witness::BASELINE_SEED);
        for (cp, st) in &staked {
            let e = st.entries.last().unwrap();
            let a = wallet::witness_bag(&cp.id(), e.seq);
            let item = wallet::bytes::witness_sig(&witness::WitnessSig { chain: cp.id(), seq: e.seq, entry: e.hash(), sig: baseline.sign(&witness::WitnessSig::message(&cp.id(), e.seq, &e.hash())).to_bytes() });
            got.insert(t::id(&lot.bag, &a), craftworks_bag_contract::encode(&a, &[craftworks_bag_contract::grind(&a, &item)]));
        }
        TestLot { lot, keys: people.into_iter().map(|p| p.2.key).collect() }
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

    /// Entry `seq` of chain `p` (state `s`, token `d`) SIGNED by the first `n` members of its committee, into its bag.
    pub fn witness(tl: &TestLot, got: &mut HashMap<[u8; 32], Vec<u8>>, p: &ChainParams, d: &Definition, s: &State, seq: u64, n: usize) {
        let i = s.entries.iter().position(|e| e.seq == seq).unwrap();
        let e = &s.entries[i];
        let (epoch, h) = wallet::lot_of(e, i.checked_sub(1).map(|j| &s.entries[j]), d.margin).unwrap();
        let out = *blake3::hash(&h.to_le_bytes()).as_bytes();
        let reg = t::claims::Registry::decode(&got[&t::claims::Params { code_hash: p.code_hash, pool: tl.lot.pool, beacon: d.beacon, epoch }.id()]).unwrap();
        let valid = t::draw::valid(&tl.lot.pool, epoch, &reg.claims);
        let (k, _, _) = witness::seats(valid.len(), wallet::COMMITTEE);
        let members = t::draw::committee_of(&out, &p.id(), seq, &valid, k);
        let addr = wallet::witness_bag(&p.id(), seq);
        let items: Vec<Vec<u8>> = members.iter().take(n).map(|m| {
            let key = tl.keys.iter().find(|k| k.verifying_key().to_bytes() == reg.claims[*m].key).unwrap();
            let sig = witness::WitnessSig { chain: p.id(), seq, entry: e.hash(), sig: key.sign(&witness::WitnessSig::message(&p.id(), seq, &e.hash())).to_bytes() };
            craftworks_bag_contract::grind(&addr, &wallet::bytes::witness_sig(&sig))
        }).collect();
        got.insert(t::id(&tl.lot.bag, &addr), craftworks_bag_contract::encode(&addr, &items));
    }

    /// The epoch STATEMENT `root` signed by the first `n` members of its committee (at its witness place).
    pub fn witness_statement(tl: &TestLot, got: &mut HashMap<[u8; 32], Vec<u8>>, code_hash: [u8; 32], beacon: [u8; 32], epoch: u64, root: &[u8; 32], n: usize) {
        use craftworks_reputation::statement::{lot_height, witness_place};
        let h = lot_height(epoch);
        let ce = h / t::supply::EPOCH;
        let out = *blake3::hash(&h.to_le_bytes()).as_bytes();
        let place = witness_place(epoch);
        let reg = t::claims::Registry::decode(&got[&t::claims::Params { code_hash, pool: tl.lot.pool, beacon, epoch: ce }.id()]).unwrap();
        let valid = t::draw::valid(&tl.lot.pool, ce, &reg.claims);
        let (k, _, _) = witness::seats(valid.len(), wallet::COMMITTEE);
        let members = t::draw::committee_of(&out, &place, 0, &valid, k);
        let addr = wallet::witness_bag(&place, 0);
        let items: Vec<Vec<u8>> = members.iter().take(n).map(|m| {
            let key = tl.keys.iter().find(|k| k.verifying_key().to_bytes() == reg.claims[*m].key).unwrap();
            let sig = witness::WitnessSig { chain: place, seq: 0, entry: *root, sig: key.sign(&witness::WitnessSig::message(&place, 0, root)).to_bytes() };
            craftworks_bag_contract::grind(&addr, &wallet::bytes::witness_sig(&sig))
        }).collect();
        got.insert(t::id(&tl.lot.bag, &addr), craftworks_bag_contract::encode(&addr, &items));
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

    /// VALID STAKE (audit 10-11): a staker whose stake no witness signed is no staker. The six test stakers' chains
    /// are signed by the baseline (the lot's bootstrap); one's signature taken away, that one drops out of `V(1)`.
    #[test]
    fn only_witnessed_stake_is_valid_stake() {
        let ctx = Ctx { code_hash: [9; 32], beacon: [7; 32], idlog: [6; 32] };
        let mut got = HashMap::new();
        let tl = testlot::stakers(&ctx, &mut got);
        let v = |got: &HashMap<[u8; 32], Vec<u8>>| testlot::done(&|g| Ok(valid_of(&ctx, &tl.lot, g, 1)), got).unwrap();
        assert_eq!(v(&got).len(), 6, "every staker's stake witnessed");
        // Staker 2's last entry before epoch 1: its witness bag emptied.
        let reg = t::claims::Registry::decode(&got[&t::claims::Params { code_hash: ctx.code_hash, pool: tl.lot.pool, beacon: ctx.beacon, epoch: 1 }.id()]).unwrap();
        let two = reg.claims.iter().find(|c| c.key == tl.keys[2].verifying_key().to_bytes()).unwrap().owner;
        let pd = Definition::decode(&got[&tl.lot.pool]).unwrap();
        let cp = ChainParams { code_hash: ctx.code_hash, definition: tl.lot.pool, owner: two, beacon: pd.beacon, idlog: pd.idlog, item: [0; 32] };
        let st = State::decode(&got[&cp.id()]).unwrap();
        let last = st.entries.last().unwrap();
        let a = t::wallet::witness_bag(&cp.id(), last.seq);
        got.insert(t::id(&tl.lot.bag, &a), craftworks_bag_contract::encode(&a, &[]));
        let after = v(&got);
        assert_eq!(after.len(), 5, "its stake no longer counts");
        assert!(!after.iter().any(|c| c.owner == two));
    }

    #[test]
    fn the_test_baseline_key_is_its_seeds() {
        use ed25519_dalek::SigningKey;
        let t = craftworks_token_contract::witness::BASELINE;
        assert_eq!(SigningKey::from_bytes(&craftworks_token_contract::witness::BASELINE_SEED).verifying_key().to_bytes(), t);
    }
}
