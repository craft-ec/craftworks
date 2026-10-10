//! THE PAGE'S WITNESS (TOKEN §7a, S3): a logged-in page whose account's $CRAFT stake was drawn into an entry's
//! committee checks the entry by THE WITNESS RULE (`wallet::may_witness`, the one rule Wren's tool runs too) over what
//! this page read, and — if it holds — names the hash its member key signs through the identity delegate
//! (`WitnessSign`: the delegate records the place first and never signs another entry there). Pure over `got`: a step
//! answers what to READ first, then its verdict.

use crate::tokens::{there, Ctx, Got, Lot, Step};
use craftworks_token_contract as t;
use craftworks_wallet::queue::Queued;
use std::cell::RefCell;
use t::{claims, draw, supply, wallet, witness, ChainParams, Definition, State};

/// The beacon's OUTPUT at height `h`: from its state, else its outputs log. `Err(id)`: read that first; `Ok(Err(why))`:
/// not drawn yet, or not recorded.
pub(crate) fn out_at(lot: &Lot, got: Got, beacon_id: &[u8; 32], h: u64) -> Result<Result<[u8; 32], &'static str>, [u8; 32]> {
    let beacon = there(got, beacon_id)?.and_then(|b| craftworks_beacon_contract::wire::decode(&b));
    if let Some(o) = beacon.as_ref().and_then(|b| b.steps.iter().find(|st| st.height == h).map(|st| st.out)) {
        return Ok(Ok(o));
    }
    if beacon.as_ref().is_none_or(|b| b.height() < h) {
        return Ok(Err("its lot is not drawn yet"));
    }
    let log = t::id(&lot.beacon_code, &craftworks_beacon_contract::log::Params::of(*beacon_id, h).encode());
    Ok(there(got, &log)?.and_then(|b| craftworks_beacon_contract::log::Log::decode(&b)).and_then(|l| l.entries.iter().find(|x| x.0 == h).map(|x| x.1)).ok_or("its lot's output is not recorded"))
}

/// What this page does about one queued entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Not drawn into its committee (or its lot not drawn yet): nothing to do.
    NotSeated,
    /// Seated, and the rule refuses it (why): not signed.
    Refused(String),
    /// Seated, and the rule takes it: sign this entry hash at (chain, seq).
    Sign { chain: [u8; 32], seq: u64, entry: [u8; 32] },
}

/// The VERDICT on queued entry `q` for the witness key `me` (this node's member key, its account's claim's), at beacon
/// height `now`.
pub fn entry(ctx: &Ctx, lot: &Lot, got: Got, q: &Queued, me: &[u8; 32], now: u64) -> Result<Step<Verdict>, String> {
    let want: RefCell<Vec<[u8; 32]>> = RefCell::new(Vec::new());
    let read = |id: [u8; 32]| -> Option<Vec<u8>> {
        match there(got, &id) {
            Err(w) => {
                want.borrow_mut().push(w);
                None
            }
            Ok(b) => b,
        }
    };
    let wanted = |want: &RefCell<Vec<[u8; 32]>>| {
        let mut w = want.borrow().clone();
        w.sort();
        w.dedup();
        Step::Want(w)
    };
    let definition = |tid: &[u8; 32]| read(*tid).and_then(|b| Definition::decode(&b));
    let Some(d) = definition(&q.token.definition) else {
        return Ok(if want.borrow().is_empty() { Step::Done(Verdict::Refused("the token's definition is not there".into())) } else { wanted(&want) });
    };
    let params = |tok: &t::Tok, owner: &[u8; 32], d: &Definition| ChainParams { code_hash: ctx.code_hash, definition: tok.definition, owner: *owner, beacon: d.beacon, idlog: d.idlog, item: tok.item };
    let p = params(&q.token, &q.owner, &d);
    let Some(s) = read(p.id()).and_then(|b| State::decode(&b)) else {
        return Ok(if want.borrow().is_empty() { Step::Done(Verdict::Refused("the chain is not there".into())) } else { wanted(&want) });
    };
    let Some(i) = s.entries.iter().position(|e| e.seq == q.seq) else { return Ok(Step::Done(Verdict::Refused("the entry is not held".into()))) };
    let (epoch, h) = wallet::lot_of(&s.entries[i], i.checked_sub(1).map(|j| &s.entries[j]), d.margin).map_err(String::from)?;
    let out = match out_at(lot, got, &d.beacon, h) {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(Err(_)) => return Ok(Step::Done(Verdict::NotSeated)),
        Ok(Ok(o)) => o,
    };
    // SEATED? The epoch's claims, the committee drawn for this place.
    let registry = |ep: u64| claims::Params { code_hash: ctx.code_hash, pool: lot.pool, beacon: d.beacon, epoch: ep }.id();
    let claims_of = |ep: u64| read(registry(ep)).and_then(|b| claims::Registry::decode(&b)).map_or(vec![], |r| r.claims);
    let claims = claims_of(epoch);
    if !want.borrow().is_empty() {
        return Ok(wanted(&want));
    }
    let valid = draw::valid(&lot.pool, epoch, &claims);
    let (k, _, _) = witness::seats(valid.len(), wallet::COMMITTEE);
    if !draw::committee_of(&out, &p.id(), q.seq, &valid, k).iter().any(|m| claims[*m].key == *me) {
        return Ok(Step::Done(Verdict::NotSeated));
    }
    // THE RULE, over what is read here (each read missing is asked for, then the rule run again).
    let sigs = |c: &[u8; 32], sq: u64| {
        let a = wallet::witness_bag(c, sq);
        read(t::id(&lot.bag, &a)).and_then(|b| craftworks_bag_contract::read(&a, &b)).unwrap_or_default().iter().filter_map(|x| wallet::bytes::read_witness_sig(craftworks_bag_contract::payload(x))).collect()
    };
    let outs = |hh: u64| match out_at(lot, got, &d.beacon, hh) {
        Err(w) => {
            want.borrow_mut().push(w);
            None
        }
        Ok(r) => r.ok(),
    };
    let pool_def = definition(&lot.pool);
    let stake_of = |o: &[u8; 32], at: u64| {
        let pd = pool_def.as_ref()?;
        let cp = params(&t::Tok { definition: lot.pool, item: [0; 32] }, o, pd);
        wallet::stake_at(&cp, pd, &read(cp.id()).and_then(|b| State::decode(&b))?, at).ok()
    };
    let program = |id: &[u8; 32]| read(*id);
    let freeze = |tid: &[u8; 32]| {
        let td = definition(tid)?;
        read(t::freeze::Params { code_hash: ctx.code_hash, definition: *tid, beacon: td.beacon }.id()).and_then(|b| t::freeze::List::decode(&b))
    };
    let keys = |o: &[u8; 32]| read(t::id(&d.idlog, o)).and_then(|b| craftworks_idlog_contract::read(o, &b));
    let src = wallet::Source { pool: lot.pool, claims: &claims_of, sigs: &sigs, out: &outs, stake_of: &stake_of, definition: &definition, program: &program, freeze: &freeze, keys: &keys, history: &|_, _| None };
    let cited = |tok: &t::Tok, o: &[u8; 32]| {
        let td = definition(&tok.definition)?;
        read(params(tok, o, &td).id()).and_then(|b| State::decode(&b))
    };
    let shard = |k: u64| read(supply::Params { code_hash: ctx.code_hash, definition: q.token.definition, beacon: d.beacon, epoch: k }.id()).and_then(|b| supply::Shard::decode(&b));
    let owner_keys = keys(&q.owner);
    let holds = |st: &State| t::check(&p, &d, st, &cited, &shard, owner_keys.as_ref(), &t::Extra { ops: None, program: Some(&program) });
    let r = wallet::may_witness(&src, &p, &d, &s, q.seq, now, &holds, &cited);
    if !want.borrow().is_empty() {
        return Ok(wanted(&want));
    }
    Ok(Step::Done(match r {
        Ok(entry) => Verdict::Sign { chain: p.id(), seq: q.seq, entry },
        Err(why) => Verdict::Refused(why.to_string()),
    }))
}

/// The WITNESS SIGNATURES each claimant gave on `queued` (`wallet::fees::count`, the one count) over what this page
/// read — each read missing asked for first.
pub fn signed(ctx: &Ctx, lot: &Lot, got: Got, queued: &[Queued]) -> Step<std::collections::BTreeMap<[u8; 32], u64>> {
    let want: RefCell<Vec<[u8; 32]>> = RefCell::new(Vec::new());
    let read = |id: [u8; 32]| -> Option<Vec<u8>> {
        match there(got, &id) {
            Err(w) => {
                want.borrow_mut().push(w);
                None
            }
            Ok(b) => b,
        }
    };
    let entry = |q: &Queued| {
        let d = read(q.token.definition).and_then(|b| Definition::decode(&b))?;
        let p = ChainParams { code_hash: ctx.code_hash, definition: q.token.definition, owner: q.owner, beacon: d.beacon, idlog: d.idlog, item: q.token.item };
        let s = read(p.id()).and_then(|b| State::decode(&b))?;
        let i = s.entries.iter().position(|e| e.seq == q.seq)?;
        Some((p.id(), s.entries[i].clone(), i.checked_sub(1).map(|j| s.entries[j].clone()), d.margin))
    };
    let beacon = ctx.beacon;
    let claims = |ep: u64| read(claims::Params { code_hash: ctx.code_hash, pool: lot.pool, beacon, epoch: ep }.id()).and_then(|b| claims::Registry::decode(&b)).map_or(vec![], |r| r.claims);
    let out = |h: u64| match out_at(lot, got, &beacon, h) {
        Err(w) => {
            want.borrow_mut().push(w);
            None
        }
        Ok(r) => r.ok(),
    };
    let sigs = |c: &[u8; 32], sq: u64| {
        let a = wallet::witness_bag(c, sq);
        read(t::id(&lot.bag, &a)).and_then(|b| craftworks_bag_contract::read(&a, &b)).unwrap_or_default().iter().filter_map(|x| wallet::bytes::read_witness_sig(craftworks_bag_contract::payload(x))).collect()
    };
    let counted = craftworks_wallet::fees::count(queued, &craftworks_wallet::fees::Reads { pool: lot.pool, entry: &entry, claims: &claims, out: &out, sigs: &sigs });
    let mut w = want.into_inner();
    if w.is_empty() {
        return Step::Done(counted);
    }
    w.sort();
    w.dedup();
    Step::Want(w)
}
