//! EDITIONS on an item (TOKEN §1c, §7c; feature 28 — shown on the item's page): an edition SOLD ON DEMAND read, bought and collected from what the
//! page has read — pure, over `got` (a contract's state as read; `Some(empty)`: read and NOT THERE). Each step answers
//! what to READ next (`Want`), the next message for the identity delegate to SIGN (`Sign`, a token entry: `TokenSign`),
//! or what it built (`Done`). The wallet core builds every entry; nothing here decides a token rule.

use craftworks_token_contract as t;
use craftworks_wallet::{Held, Sources, Wallet};
pub use crate::tokens::{chain, device, Ctx, Device, Got, Put, Step};
use crate::tokens::{on_our_clock, short, there};
use t::{Definition, State};

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

/// What a buy built: the payment's chain and the record to put, and the copies it buys.
#[derive(Debug, Clone, PartialEq)]
pub struct Bought {
    pub puts: Vec<Put>,
    pub record: u64,
    pub first: u128,
    pub amount: u128,
    /// The one-time owner the copies were minted to (the paying chain's: the buyer's, privately).
    pub to: [u8; 32],
}

/// Record `k` of the edition `edition` (its definition `d`: on its own beacon).
fn record_params(ctx: &Ctx, d: &Definition, edition: &[u8; 32], k: u64) -> t::supply::Params {
    t::supply::Params { code_hash: ctx.code_hash, definition: *edition, beacon: d.beacon, epoch: k }
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
        let rid = record_params(ctx, &d, id, k).id();
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

/// BUY `amount` copies of the edition `id` PRIVATELY (TOKEN §7f): paid from one of this wallet's stealth chains of the
/// sale's token (`mine`: the found ones' R and owner; `seed`: this wallet's) that holds the price — the largest first —
/// signed by its one-time key; the paid mint added to the edition's open record, to that same one-time owner (the copy
/// is the buyer's privately: its chain owned by a key only the buyer derives).
#[allow(clippy::too_many_arguments)]
pub fn buy(ctx: &Ctx, got: Got, id: &[u8; 32], seed: &[u8; 32], mine: &[([u8; 32], [u8; 32])], amount: u128, now: u64) -> Result<Step<Bought>, String> {
    let e = match edition(ctx, got, id)? {
        Step::Done(e) => e,
        Step::Want(w) => return Ok(Step::Want(w)),
        Step::Sign(m) => return Ok(Step::Sign(m)),
    };
    let k = e.open.ok_or("sold out")?;
    let (tok, price, _) = e.definition.sale.ok_or("not sold on demand")?;
    let pay_def = match there(got, &tok.definition) {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(None) => return Err("the sale's token is not there".into()),
        Ok(Some(b)) => Definition::decode(&b).ok_or("the sale's token does not read")?,
    };
    on_our_clock(ctx, &e.definition)?;
    on_our_clock(ctx, &pay_def)?;
    let cost = price.checked_mul(amount).ok_or("a balance overflow")?;
    let mut want = Vec::new();
    let mut chains = Vec::new();
    for (r, to) in mine {
        let p = chain(ctx, &pay_def, &tok.definition, to, tok.item);
        match there(got, &p.id()) {
            Err(w) => want.push(w),
            Ok(None) => {}
            Ok(Some(b)) => chains.push((*r, p, State::decode(&b).ok_or("a chain that does not read")?)),
        }
    }
    if !want.is_empty() {
        return Ok(Step::Want(want));
    }
    chains.sort_by_key(|c| std::cmp::Reverse(c.2.balance()));
    let (r, p, state) = chains.into_iter().find(|c| c.2.balance() >= cost + t::transfer_fee(&pay_def, &c.1.owner, cost)).ok_or_else(|| format!("no money: no private chain of the sale's token {} holds {}", short(&tok.definition), cost))?;
    let one = craftworks_wallet::stealth::Keys::from_seed(seed).find(&r, &p.owner).ok_or("a chain that is not this wallet's")?;
    let none = craftworks_idlog_contract::Log { events: vec![] };
    let held = Held { params: p, definition: pay_def, state, blocks: Default::default() };
    let record = &e.records[k as usize];
    let (entries, mint) = Wallet { device: &one, keys: &none }.buy(&held, &e.definition, id, (k, record), amount, now).map_err(String::from)?;
    let mut s = held.state.clone();
    s.entries.extend(entries);
    let first = u128::from_be_bytes(mint.nonce);
    let to = mint.to;
    let mut shard = record.clone();
    shard.mints.push(mint);
    Ok(Step::Done(Bought {
        puts: vec![Put { params: held.params.encode(), state: s.encode() }, Put { params: record_params(ctx, &e.definition, id, k).encode(), state: shard.encode() }],
        record: k,
        first,
        amount,
        to,
    }))
}

/// COLLECT copy `n` of the edition `id` into `owner`'s chain of it (its mint read from its record): a numbered copy's
/// own chain, or for plain copies the owner's one chain of the edition (every mint to it in `n`'s record). The chain
/// to put, or `None` when there is nothing to collect (not minted to `owner`, or credited already). A credit is worked:
/// nothing signs — `owner` a one-time owner (no key log) as well as a DID.
pub fn collect_copy(ctx: &Ctx, got: Got, id: &[u8; 32], owner: &[u8; 32], n: u128, now: u64) -> Result<Step<Option<Put>>, String> {
    let d = match there(got, id) {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(None) => return Err("no such edition".into()),
        Ok(Some(b)) => Definition::decode(&b).ok_or("not a token definition")?,
    };
    if !d.capped() {
        return Err("not an edition (no records)".into());
    }
    on_our_clock(ctx, &d)?;
    let k = t::supply::record_of(&d, n).ok_or("no such copy")?;
    let item = if d.ids == t::Ids::Numbered { t::numbered_item(id, n) } else { [0; 32] };
    let cp = chain(ctx, &d, id, owner, item);
    let mut want = Vec::new();
    let mut read = |x: [u8; 32]| match there(got, &x) {
        Err(w) => {
            want.push(w);
            None
        }
        Ok(b) => Some(b),
    };
    let (rec, mine, keys) = (read(record_params(ctx, &d, id, k).id()), read(cp.id()), read(cp.keys_id()));
    let (Some(rec), Some(mine), Some(keys)) = (rec, mine, keys) else { return Ok(Step::Want(want)) };
    let shard = rec.map(|b| t::supply::Shard::decode(&b).ok_or("a record that does not read")).transpose()?.unwrap_or_default();
    let state = mine.map(|b| State::decode(&b).ok_or("the copy's chain does not read")).transpose()?.unwrap_or_default();
    let keys = match keys {
        Some(b) => craftworks_idlog_contract::read(owner, &b).ok_or("the owner's key log does not verify")?,
        None => craftworks_idlog_contract::Log { events: vec![] },
    };
    let held = Held { params: cp, definition: d, state, blocks: Default::default() };
    let anyone = craftworks_wallet::Asking::new([0; 32], vec![], 0, vec![]);
    let w = Wallet { device: &anyone, keys: &keys };
    let shards = [shard];
    let entries = w.collect(&held, &Sources { chains: &[], blocks: &Default::default(), programs: &[], shards: &shards }, now).map_err(String::from)?;
    if entries.is_empty() {
        return Ok(Step::Done(None));
    }
    let mut s = held.state;
    s.entries.extend(entries);
    Ok(Step::Done(Some(Put { params: held.params.encode(), state: s.encode() })))
}

/// The 32-byte id a token names a Craftworks ITEM by (`Definition::item`): from the item's ref (`<did>/<id>`, or
/// `space:<id>/<id>`) — the ONE derivation, so an edition made for an item and the item's page agree.
pub fn item_id(item_ref: &str) -> [u8; 32] {
    *blake3::Hasher::new_derive_key("craftworks token item 2026-10-10").update(item_ref.as_bytes()).finalize().as_bytes()
}

/// A NEW EDITION's definition: of the item `item_ref`, sold on demand at `price` of `pay_token` a copy to `payee` — the
/// author's one-time owner (TOKEN §7f: a DID holds only stake), also its creator and its royalty's — with no mint
/// authority (copies are minted by payment alone), `cap` copies (0: none, numbered only), a royalty of `royalty_bps`
/// (0: none). Refused as the token refuses it (its definition must decode back).
#[allow(clippy::too_many_arguments)]
pub fn new_edition(ctx: &Ctx, item_ref: &str, payee: &[u8; 32], name: &str, cap: u128, numbered: bool, pay_token: &[u8; 32], price: u128, royalty_bps: u16, nonce: [u8; 16]) -> Result<Definition, &'static str> {
    let d = Definition {
        code_hash: ctx.code_hash,
        decimals: 0,
        min_send: 1,
        genesis_owner: *payee,
        genesis_amount: 0,
        name: name.as_bytes().to_vec(),
        nonce,
        beacon: ctx.beacon,
        margin: 2,
        idlog: ctx.idlog,
        mint: None,
        freeze: None,
        ids: if numbered { t::Ids::Numbered } else { t::Ids::None },
        royalty: (royalty_bps > 0).then_some((royalty_bps, *payee)),
        cap,
        item: item_id(item_ref),
        version: [0; 32],
        soulbound: false,
        transfer_fee: None,
        sale: Some((t::Tok { definition: *pay_token, item: [0; 32] }, price, *payee)),
        keepers: None,
    };
    if Definition::decode(&d.encode()).as_ref() != Some(&d) {
        return Err("not an edition the token takes (a price, and a cap or numbered copies)");
    }
    Ok(d)
}

/// Is `e` an edition the item `item_ref` SHOWS: one of this item (TOKEN §1c). Its AUTHOR is the item's: the item lists
/// its editions in its own metadata (`meta.editions`), which only its author writes — an edition's creator is a one-time
/// owner (§7f), naming no one.
pub fn of_item(e: &Edition, item_ref: &str) -> Result<(), &'static str> {
    if e.definition.item != item_id(item_ref) {
        return Err("an edition of another item");
    }
    Ok(())
}

/// The SALES of an edition, as ANNOUNCEMENTS to its payee (TOKEN §7f): each paid mint in the records read names its
/// payment (the payer, the minted-to owner, and its nonce) — what the payee collects, as any private payment. (`r` is
/// not needed to collect: zero.)
pub fn sales(e: &Edition) -> Vec<craftworks_wallet::stealth::Announce> {
    let Some((tok, _, payee)) = e.definition.sale else { return vec![] };
    e.records.iter().flat_map(|r| &r.mints).filter_map(|m| m.paid.map(|(_, nonce)| craftworks_wallet::stealth::Announce { token: tok.definition, from: m.to, nonce, to: payee, r: [0; 32] })).collect()
}

/// The copies of a numbered edition minted to any of `owners` (this wallet's one-time owners), from the records read.
pub fn copies_of(e: &Edition, owners: &[[u8; 32]]) -> Vec<u128> {
    let mut out = Vec::new();
    for r in &e.records {
        for m in r.mints.iter().filter(|m| owners.contains(&m.to)) {
            let a = u128::from_be_bytes(m.nonce);
            out.extend(a..a + m.amount);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use craftworks_wallet::stealth::{self, Keys};
    use std::collections::HashMap;

    /// PRIVATELY end to end (TOKEN §7f): the author's edition pays a one-time owner of its own; the buyer pays from a
    /// stealth chain of its own and the copy is minted to that one-time owner; the copy is collected (worked, no key
    /// log); the author collects the sale from the record's mints. No DID anywhere.
    #[test]
    fn a_page_buys_a_copy_privately_and_the_author_collects_the_sale() {
        let ctx = Ctx { code_hash: [9; 32], beacon: [7; 32], idlog: [6; 32] };
        let (author, buyer) = (Keys::from_seed(&[31; 32]), Keys::from_seed(&[32; 32]));
        // The buyer's money: a pay token P whose genesis is the buyer's one-time owner (a creator's stealth chain).
        let bp = stealth::pay(&buyer.public(), &[1; 32]).unwrap();
        let bk = buyer.find(&bp.r, &bp.to).unwrap();
        let base = Definition { code_hash: ctx.code_hash, decimals: 0, min_send: 1, genesis_owner: bp.to, genesis_amount: 1000, name: b"P".to_vec(), nonce: [0; 16], beacon: ctx.beacon, margin: 2, idlog: ctx.idlog, mint: None, freeze: None, ids: t::Ids::None, royalty: None, cap: 0, item: [0; 32], version: [0; 32], soulbound: false, transfer_fee: None, sale: None, keepers: None };
        let pid = t::id(&ctx.code_hash, &base.encode());
        let pp = chain(&ctx, &base, &pid, &bp.to, [0; 32]);
        let none = craftworks_idlog_contract::Log { events: vec![] };
        let mut ps = State::default();
        ps.entries = Wallet { device: &bk, keys: &none }.genesis(&Held { params: pp, definition: base.clone(), state: State::default(), blocks: Default::default() }, 1).unwrap();
        // The author's edition: its payee the author's own one-time owner.
        let ap = stealth::pay(&author.public(), &[2; 32]).unwrap();
        let sd = new_edition(&ctx, "did/t1", &ap.to, "Gold", 10, true, &pid, 10, 1000, [5; 16]).unwrap();
        assert_eq!((sd.genesis_owner, sd.sale.map(|s| s.2), sd.royalty), (ap.to, Some(ap.to), Some((1000, ap.to))));
        let sid = t::id(&ctx.code_hash, &sd.encode());
        let mut got: HashMap<[u8; 32], Vec<u8>> = HashMap::new();
        for (k, v) in [(sid, sd.encode()), (pid, base.encode()), (pp.id(), ps.encode()), (record_params(&ctx, &sd, &sid, 0).id(), vec![])] {
            got.insert(k, v);
        }
        let g = got.clone();
        let Ok(Step::Done(e)) = edition(&ctx, &|id| g.get(id).cloned(), &sid) else { panic!("the edition") };
        assert_eq!((e.minted, e.open), (0, Some(0)));
        // Shown on the item it names (its author: the item's own `meta.editions`).
        assert_eq!(of_item(&e, "did/t1"), Ok(()));
        assert_eq!(of_item(&e, "did/t2"), Err("an edition of another item"));
        // BUY one copy at height 5, from the buyer's stealth chain; another wallet's seed cannot spend it.
        let mine = [(bp.r, bp.to)];
        assert_eq!(buy(&ctx, &|id| g.get(id).cloned(), &sid, &[31; 32], &mine, 1, 5), Err("a chain that is not this wallet's".into()));
        assert_eq!(buy(&ctx, &|id| g.get(id).cloned(), &sid, &[32; 32], &mine, 101, 5), Err(format!("no money: no private chain of the sale's token {} holds 1010", short(&pid))));
        // (In epoch 1, where the test's stakers stand: the author takes the sale once its payment is witnessed.)
        let e1 = crate::tokens::testlot::E1;
        let Ok(Step::Done(b)) = buy(&ctx, &|id| g.get(id).cloned(), &sid, &[32; 32], &mine, 1, e1 + 5) else { panic!("bought") };
        assert_eq!((b.record, b.first, b.amount, b.to), (0, 1, 1, bp.to));
        let paid = State::decode(&b.puts[0].state).unwrap();
        assert_eq!((t::check(&pp, &base, &paid, &|_, _| None, &|_| None, None, &t::Extra::NONE), paid.balance()), (Ok(()), 990));
        let rp = record_params(&ctx, &sd, &sid, 0);
        let rec = t::supply::Shard::decode(&b.puts[1].state).unwrap();
        assert_eq!(t::supply::holds(&rp, &sd, &Default::default(), &rec, &|m| t::supply::payment_ok(&rp, &sd, m, Some(&paid))), Ok(()));
        assert_eq!(rec.mints[0].to, bp.to);
        // The copy: minted to the buyer's one-time owner, collected (worked; no key log needed).
        got.insert(rp.id(), b.puts[1].state.clone());
        got.insert(pp.id(), b.puts[0].state.clone());
        let cp = chain(&ctx, &sd, &sid, &bp.to, t::numbered_item(&sid, 1));
        got.insert(cp.id(), vec![]);
        got.insert(cp.keys_id(), vec![]);
        let g = got.clone();
        let Ok(Step::Done(e)) = edition(&ctx, &|id| g.get(id).cloned(), &sid) else { panic!("the edition") };
        assert_eq!((e.minted, copies_of(&e, &[bp.to]), copies_of(&e, &[ap.to])), (1, vec![1], vec![]));
        let Ok(Step::Done(Some(put))) = collect_copy(&ctx, &|id| g.get(id).cloned(), &sid, &bp.to, 1, e1 + 6) else { panic!("collected") };
        let c = State::decode(&put.state).unwrap();
        assert_eq!(t::check(&cp, &sd, &c, &|_, _| None, &|k| (k == 0).then(|| rec.clone()), None, &t::Extra::NONE), Ok(()));
        assert_eq!(c.balance(), 1);
        // THE SALE, collected by the author: the record's mint is its announcement.
        let sold = sales(&e);
        assert_eq!(sold.len(), 1);
        assert_eq!((sold[0].from, sold[0].to, sold[0].token), (bp.to, ap.to, pid));
        let payee = chain(&ctx, &base, &pid, &ap.to, [0; 32]);
        got.insert(payee.id(), vec![]);
        let tl = crate::tokens::testlot::stakers(&ctx, &mut got);
        let take = |got: &HashMap<[u8; 32], Vec<u8>>| crate::tokens::testlot::done(&|g| crate::wallet::receive_private(&ctx, &tl.lot, g, &sold[0], e1 + 8), got);
        assert_eq!(take(&got).map(|r| (r.amount, r.unwitnessed)), Ok((0, 1)), "the sale is not value before its payment is witnessed");
        crate::tokens::testlot::witness(&tl, &mut got, &pp, &base, &paid, paid.entries.last().unwrap().seq, 6);
        let r = take(&got).unwrap();
        assert_eq!(r.amount, 10);
        let ps2 = State::decode(&r.put.unwrap().state).unwrap();
        assert_eq!(t::check(&payee, &base, &ps2, &|_, o| (o == &bp.to).then(|| paid.clone()), &|_| None, None, &t::Extra::NONE), Ok(()));
        // A new edition: as the token takes it — a price, and a cap or numbered copies.
        assert_eq!(new_edition(&ctx, "did/t1", &ap.to, "x", 0, false, &pid, 10, 0, [5; 16]).err(), Some("not an edition the token takes (a price, and a cap or numbered copies)"));
        assert_eq!(new_edition(&ctx, "did/t1", &ap.to, "x", 5, false, &pid, 0, 0, [5; 16]).err(), Some("not an edition the token takes (a price, and a cap or numbered copies)"));
    }
}
