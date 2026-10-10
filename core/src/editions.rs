//! EDITIONS on an item (TOKEN §1c, §7c; feature 28 — shown on the item's page): an edition SOLD ON DEMAND read, bought and collected from what the
//! page has read — pure, over `got` (a contract's state as read; `Some(empty)`: read and NOT THERE). Each step answers
//! what to READ next (`Want`), the next message for the identity delegate to SIGN (`Sign`, a token entry: `TokenSign`),
//! or what it built (`Done`). The wallet core builds every entry; nothing here decides a token rule.

use craftworks_token_contract as t;
use craftworks_wallet::{Asking, Held, Sources, Wallet};
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

/// An EDITION as the item page shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Edition {
    pub id: [u8; 32],
    pub definition: Definition,
    /// Copies minted (admitted) so far, over every record read.
    pub minted: u128,
    /// The first record with room (`None`: sold out — the cap reached).
    pub open: Option<u64>,
    /// Each record read: its admitted mints.
    pub records: Vec<t::supply::Shard>,
}

/// A PUT: a token contract (by its params) and its state.
#[derive(Debug, Clone, PartialEq)]
pub struct Put {
    pub params: Vec<u8>,
    pub state: Vec<u8>,
}

/// What a buy built: the payment's chain and the record to put, and the copies it buys.
#[derive(Debug, Clone, PartialEq)]
pub struct Bought {
    pub puts: Vec<Put>,
    pub record: u64,
    pub first: u128,
    pub amount: u128,
}

fn there(got: Got, id: &[u8; 32]) -> Result<Option<Vec<u8>>, [u8; 32]> {
    match got(id) {
        None => Err(*id),
        Some(b) if b.is_empty() => Ok(None),
        Some(b) => Ok(Some(b)),
    }
}

fn record_params(ctx: &Ctx, edition: &[u8; 32], k: u64) -> t::supply::Params {
    t::supply::Params { code_hash: ctx.code_hash, definition: *edition, beacon: ctx.beacon, epoch: k }
}

/// A chain's params: `owner`'s chain of `token` (`item`: a numbered copy's, else zero).
pub fn chain(ctx: &Ctx, d: &Definition, token: &[u8; 32], owner: &[u8; 32], item: [u8; 32]) -> ChainParams {
    ChainParams { code_hash: ctx.code_hash, definition: *token, owner: *owner, beacon: ctx.beacon, idlog: d.idlog, item }
}

/// The EDITION `id`: its definition and its records read in order until one with room (or the cap).
pub fn edition(ctx: &Ctx, got: Got, id: &[u8; 32]) -> Result<Step<Edition>, String> {
    let d = match there(got, id) {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(None) => return Err("no such edition".into()),
        Ok(Some(b)) => Definition::decode(&b).ok_or("not a token definition")?,
    };
    if !d.capped() {
        return Err("not an edition (no records)".into());
    }
    let size = t::supply::record_size(&d);
    let mut records = Vec::new();
    let mut minted = 0u128;
    for k in 0u64.. {
        let lo = (k as u128) * size + 1;
        if d.cap > 0 && lo > d.cap {
            return Ok(Step::Done(Edition { id: *id, definition: d, minted, open: None, records }));
        }
        let rid = record_params(ctx, id, k).id();
        let shard = match there(got, &rid) {
            Err(w) => return Ok(Step::Want(vec![w])),
            Ok(None) => t::supply::Shard::default(),
            Ok(Some(b)) => t::supply::Shard::decode(&b).ok_or("a record that does not read")?,
        };
        let admitted: Vec<t::supply::Mint> = t::supply::admitted(&d, &shard).into_iter().cloned().collect();
        let here: u128 = admitted.iter().map(|m| m.amount).sum();
        minted += here;
        let hi = ((k as u128 + 1) * size).min(if d.cap > 0 { d.cap } else { u128::MAX });
        let room = hi + 1 - lo.max(d.genesis_amount + 1) > here;
        records.push(t::supply::Shard { mints: admitted });
        if room {
            return Ok(Step::Done(Edition { id: *id, definition: d, minted, open: Some(k), records }));
        }
    }
    unreachable!()
}

fn device_of(dev: &Device) -> Asking {
    Asking::new(dev.public, dev.credential.clone(), dev.key_seq, dev.sigs.clone())
}

/// BUY `amount` copies of the edition `id` for `buyer` (its DID) at beacon height `now`: the payment from the buyer's
/// chain of the sale's token, and the paid mint added to the edition's open record.
pub fn buy(ctx: &Ctx, got: Got, id: &[u8; 32], buyer: &[u8; 32], amount: u128, now: u64, dev: &Device) -> Result<Step<Bought>, String> {
    let e = match edition(ctx, got, id)? {
        Step::Done(e) => e,
        Step::Want(w) => return Ok(Step::Want(w)),
        Step::Sign(m) => return Ok(Step::Sign(m)),
    };
    let k = e.open.ok_or("sold out")?;
    let (tok, ..) = e.definition.sale.ok_or("not sold on demand")?;
    let pay_def = match there(got, &tok.definition) {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(None) => return Err("the sale's token is not there".into()),
        Ok(Some(b)) => Definition::decode(&b).ok_or("the sale's token does not read")?,
    };
    let pp = ChainParams { code_hash: ctx.code_hash, definition: tok.definition, owner: *buyer, beacon: ctx.beacon, idlog: e.definition.idlog, item: tok.item };
    let mut want = Vec::new();
    let pay_state = match there(got, &pp.id()) {
        Err(w) => {
            want.push(w);
            None
        }
        Ok(None) => return Err("no money: the buyer holds none of the sale's token".into()),
        Ok(Some(b)) => Some(State::decode(&b).ok_or("the buyer's chain does not read")?),
    };
    let keys = match there(got, &pp.keys_id()) {
        Err(w) => {
            want.push(w);
            None
        }
        Ok(None) => return Err("the buyer's key log is not there".into()),
        Ok(Some(b)) => Some(craftworks_idlog_contract::read(buyer, &b).ok_or("the buyer's key log does not verify")?),
    };
    let (Some(state), Some(keys)) = (pay_state, keys) else { return Ok(Step::Want(want)) };
    let held = Held { params: pp, definition: pay_def, state, blocks: Default::default() };
    let asking = device_of(dev);
    let w = Wallet { device: &asking, keys: &keys };
    let record = &e.records[k as usize];
    let built = w.buy(&held, &e.definition, id, (k, record), amount, now);
    if let Some(m) = asking.next() {
        return Ok(Step::Sign(m));
    }
    let (entries, mint) = built.map_err(String::from)?;
    let mut s = held.state.clone();
    s.entries.extend(entries);
    let first = u128::from_be_bytes(mint.nonce);
    let mut shard = record.clone();
    shard.mints.push(mint);
    Ok(Step::Done(Bought {
        puts: vec![Put { params: held.params.encode(), state: s.encode() }, Put { params: record_params(ctx, id, k).encode(), state: shard.encode() }],
        record: k,
        first,
        amount,
    }))
}

/// COLLECT copy `n` of a NUMBERED edition `id` into `owner`'s chain of it (its mint read from its record): the chain to
/// put, or `None` when there is nothing to collect (not minted to `owner`, or credited already).
pub fn collect_copy(ctx: &Ctx, got: Got, id: &[u8; 32], owner: &[u8; 32], n: u128, now: u64, dev: &Device) -> Result<Step<Option<Put>>, String> {
    let d = match there(got, id) {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(None) => return Err("no such edition".into()),
        Ok(Some(b)) => Definition::decode(&b).ok_or("not a token definition")?,
    };
    if d.ids != t::Ids::Numbered {
        return Err("not a numbered edition".into());
    }
    let k = t::supply::record_of(&d, n).ok_or("no such copy")?;
    let cp = chain(ctx, &d, id, owner, t::numbered_item(id, n));
    let mut want = Vec::new();
    let mut read = |x: [u8; 32]| match there(got, &x) {
        Err(w) => {
            want.push(w);
            None
        }
        Ok(b) => Some(b),
    };
    let (rec, mine, keys) = (read(record_params(ctx, id, k).id()), read(cp.id()), read(cp.keys_id()));
    let (Some(rec), Some(mine), Some(keys)) = (rec, mine, keys) else { return Ok(Step::Want(want)) };
    let shard = rec.map(|b| t::supply::Shard::decode(&b).ok_or("a record that does not read")).transpose()?.unwrap_or_default();
    let state = mine.map(|b| State::decode(&b).ok_or("the copy's chain does not read")).transpose()?.unwrap_or_default();
    let keys = craftworks_idlog_contract::read(owner, &keys.ok_or("the owner's key log is not there")?).ok_or("the owner's key log does not verify")?;
    let held = Held { params: cp, definition: d, state, blocks: Default::default() };
    let asking = device_of(dev);
    let w = Wallet { device: &asking, keys: &keys };
    let shards = [shard];
    let got_entries = w.collect(&held, &Sources { chains: &[], blocks: &Default::default(), programs: &[], shards: &shards }, now);
    if let Some(m) = asking.next() {
        return Ok(Step::Sign(m));
    }
    let entries = got_entries.map_err(String::from)?;
    if entries.is_empty() {
        return Ok(Step::Done(None));
    }
    let mut s = held.state;
    s.entries.extend(entries);
    Ok(Step::Done(Some(Put { params: held.params.encode(), state: s.encode() })))
}

/// The copies of a numbered edition minted to `owner`, from the records read (`Edition::records`).
pub fn copies_of(e: &Edition, owner: &[u8; 32]) -> Vec<u128> {
    let mut out = Vec::new();
    for r in &e.records {
        for m in r.mints.iter().filter(|m| m.to == *owner) {
            let a = u128::from_be_bytes(m.nonce);
            out.extend(a..a + m.amount);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};
    use std::collections::HashMap;

    const IDLOG: [u8; 32] = [6; 32];
    const OWNER_SEED: [u8; 32] = [101; 32];
    const NODE_SEED: [u8; 32] = [12; 32];

    fn log() -> craftworks_idlog_contract::Log {
        let next = SigningKey::from_bytes(&[151; 32]).verifying_key().to_bytes();
        let e = craftworks_idlog_contract::Event::signed(0, [0; 32], &OWNER_SEED, craftworks_idlog_contract::commit(&next), [1; 32], [1; 32], vec![]).unwrap();
        craftworks_idlog_contract::Log { events: vec![e] }
    }

    /// The page's loop: step, and on `Sign` have the "delegate" (the node key) sign and step again.
    fn run<T>(dev: &mut Device, step: &mut dyn FnMut(&Device) -> Result<Step<T>, String>) -> Result<T, String> {
        for _ in 0..8 {
            match step(dev)? {
                Step::Done(x) => return Ok(x),
                Step::Sign(m) => {
                    assert!(m.starts_with(t::ENTRY_DOMAIN));
                    dev.sigs.push((m.clone(), SigningKey::from_bytes(&NODE_SEED).sign(&m).to_bytes()));
                }
                Step::Want(w) => panic!("wants {:?}", w.iter().map(|x| x[0]).collect::<Vec<_>>()),
            }
        }
        panic!("no end")
    }

    #[test]
    fn a_page_buys_a_copy_on_demand_and_collects_it() {
        let ctx = Ctx { code_hash: [9; 32], beacon: [7; 32], idlog: IDLOG };
        let lg = log();
        let did = lg.events[0].id();
        let node = SigningKey::from_bytes(&NODE_SEED).verifying_key().to_bytes();
        let mut dev = Device { public: node, credential: craftworks_identity::credential(&did, &node, &[], &OWNER_SEED), key_seq: 0, sigs: vec![] };
        let base = Definition { code_hash: ctx.code_hash, decimals: 0, min_send: 1, genesis_owner: did, genesis_amount: 1000, name: b"P".to_vec(), nonce: [0; 16], beacon: ctx.beacon, margin: 2, idlog: IDLOG, mint: None, freeze: None, ids: t::Ids::None, royalty: None, cap: 0, item: [0; 32], version: [0; 32], soulbound: false, transfer_fee: None, sale: None };
        let pid = t::id(&ctx.code_hash, &base.encode());
        let sd = Definition { ids: t::Ids::Numbered, cap: 10, genesis_amount: 0, name: b"S".to_vec(), sale: Some((t::Tok { definition: pid, item: [0; 32] }, 10, [9; 32])), ..base.clone() };
        let sid = t::id(&ctx.code_hash, &sd.encode());
        // The buyer's chain of P: its genesis, written by this node (the wallet, signing by asking).
        let pp = chain(&ctx, &base, &pid, &did, [0; 32]);
        let mut pay = State::default();
        let h = Held { params: pp.clone(), definition: base.clone(), state: State::default(), blocks: Default::default() };
        let genesis = run(&mut dev, &mut |d| {
            let a = device_of(d);
            let r = Wallet { device: &a, keys: &lg }.genesis(&h, 1);
            Ok(match a.next() {
                Some(m) => Step::Sign(m),
                None => Step::Done(r.map_err(String::from)?),
            })
        })
        .unwrap();
        pay.entries.extend(genesis);
        let mut got: HashMap<[u8; 32], Vec<u8>> = HashMap::new();
        got.insert(sid, sd.encode());
        got.insert(pid, base.encode());
        got.insert(pp.id(), pay.encode());
        got.insert(pp.keys_id(), lg.encode());
        // Before the record is read: wanted.
        let g = got.clone();
        assert_eq!(edition(&ctx, &|id| g.get(id).cloned(), &sid), Ok(Step::Want(vec![record_params(&ctx, &sid, 0).id()])));
        got.insert(record_params(&ctx, &sid, 0).id(), vec![]);
        let g = got.clone();
        let Ok(Step::Done(e)) = edition(&ctx, &|id| g.get(id).cloned(), &sid) else { panic!("the edition") };
        assert_eq!((e.minted, e.open), (0, Some(0)));
        // BUY one copy at height 5.
        let b = run(&mut dev, &mut |d| buy(&ctx, &|id| g.get(id).cloned(), &sid, &did, 1, 5, d)).unwrap();
        assert_eq!((b.record, b.first, b.amount), (0, 1, 1));
        let paid = State::decode(&b.puts[0].state).unwrap();
        assert_eq!(t::check(&pp, &base, &paid, &|_, _| None, &|_| None, Some(&lg), &t::Extra::NONE), Ok(()));
        assert_eq!(paid.balance(), 990);
        let rp = record_params(&ctx, &sid, 0);
        let rec = t::supply::Shard::decode(&b.puts[1].state).unwrap();
        assert_eq!(t::supply::holds(&rp, &sd, &Default::default(), &rec, &|m| t::supply::payment_ok(&rp, &sd, m, Some(&paid))), Ok(()));
        // The record as put: the edition shows it, and the buyer COLLECTS copy #1.
        got.insert(rp.id(), b.puts[1].state.clone());
        got.insert(pp.id(), b.puts[0].state.clone());
        let cp = chain(&ctx, &sd, &sid, &did, t::numbered_item(&sid, 1));
        got.insert(cp.id(), vec![]);
        let g = got.clone();
        let Ok(Step::Done(e)) = edition(&ctx, &|id| g.get(id).cloned(), &sid) else { panic!("the edition") };
        assert_eq!((e.minted, copies_of(&e, &did)), (1, vec![1]));
        let put = run(&mut dev, &mut |d| collect_copy(&ctx, &|id| g.get(id).cloned(), &sid, &did, 1, 6, d)).unwrap().expect("a credit");
        let c = State::decode(&put.state).unwrap();
        assert_eq!(c.balance(), 1);
        assert_eq!(t::check(&cp, &sd, &c, &|_, _| None, &|k| (k == 0).then(|| rec.clone()), Some(&lg), &t::Extra::NONE), Ok(()));
        // Not the buyer's copy (#2, never minted): nothing to collect.
        got.insert(chain(&ctx, &sd, &sid, &did, t::numbered_item(&sid, 2)).id(), vec![]);
        let g = got.clone();
        let other = run(&mut dev, &mut |d| collect_copy(&ctx, &|id| g.get(id).cloned(), &sid, &did, 2, 6, d));
        assert_eq!(other, Ok(None));
    }
}
