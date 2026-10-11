//! THE PAGE'S WITNESS (TOKEN §7a, S3): a logged-in page whose account's $CRAFT stake was drawn into an entry's
//! committee checks the entry by THE WITNESS RULE (`wallet::may_witness`, the one rule Wren's tool runs too) over what
//! this page read, and — if it holds — names the hash its member key signs through the identity delegate
//! (`WitnessSign`: the delegate records the place first and never signs another entry there). Pure over `got`: a step
//! answers what to READ first, then its verdict.

use crate::tokens::{there, Ctx, Got, Lot, Step};
use craftworks_token_contract as t;
use craftworks_wallet::queue::Queued;
use std::cell::RefCell;
use t::{draw, supply, wallet, witness, ChainParams, Definition, State};

/// The beacon's OUTPUT at height `h`: from its state, else its outputs log. `Err(id)`: read that first; `Ok(Err(why))`:
/// not drawn yet, or not recorded.
pub fn out_at(lot: &Lot, got: Got, beacon_id: &[u8; 32], h: u64) -> Result<Result<[u8; 32], &'static str>, [u8; 32]> {
    beacon_out(&lot.beacon_code, got, beacon_id, h)
}

/// The BEACON's output at height `h` (beacon `beacon_id`, its outputs log under `beacon_code`): from its state while it
/// holds that step, else its log. `Err(id)`: read that first.
pub fn beacon_out(beacon_code: &[u8; 32], got: Got, beacon_id: &[u8; 32], h: u64) -> Result<Result<[u8; 32], &'static str>, [u8; 32]> {
    let beacon = there(got, beacon_id)?.and_then(|b| craftworks_beacon_contract::wire::decode(&b));
    if let Some(o) = beacon.as_ref().and_then(|b| b.steps.iter().find(|st| st.height == h).map(|st| st.out)) {
        return Ok(Ok(o));
    }
    if beacon.as_ref().is_none_or(|b| b.height() < h) {
        return Ok(Err("its lot is not drawn yet"));
    }
    let log = t::id(beacon_code, &craftworks_beacon_contract::log::Params::of(*beacon_id, h).encode());
    Ok(there(got, &log)?.and_then(|b| craftworks_beacon_contract::log::Log::decode(&b)).and_then(|l| l.entries.iter().find(|x| x.0 == h).map(|x| x.1)).ok_or("its lot's output is not recorded"))
}

/// What this page does about one queued entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Not drawn into its committee (its lot drawn, the committee computed): nothing to do, ever.
    NotSeated,
    /// NOT YET decidable (audit 10-11: these were `NotSeated`, and a delegate settled them for good): its lot not drawn
    /// yet, its token or chain not readable here, the entry not in the chain as read — asked again later.
    Later(&'static str),
    /// Seated, and the rule refuses it (why): not signed.
    Refused(String),
    /// Seated, and the rule takes it: sign this entry hash at (chain, seq) — `at`, the place's signatures container
    /// under its lot's claims epoch (where the signature goes; `at.chain`, `at.seq` its place).
    Sign { at: t::witness::Params, entry: [u8; 32] },
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
        return Ok(if want.borrow().is_empty() { Step::Done(Verdict::Later("its token or chain is not readable here")) } else { wanted(&want) });
    };
    let params = |tok: &t::Tok, owner: &[u8; 32], d: &Definition| ChainParams { code_hash: ctx.code_hash, definition: tok.definition, owner: *owner, beacon: d.beacon, idlog: d.idlog, item: tok.item };
    let p = params(&q.token, &q.owner, &d);
    let Some(s) = read(p.id()).and_then(|b| State::decode(&b)) else {
        return Ok(if want.borrow().is_empty() { Step::Done(Verdict::Later("its token or chain is not readable here")) } else { wanted(&want) });
    };
    let Some(i) = s.entries.iter().position(|e| e.seq == q.seq) else { return Ok(Step::Done(Verdict::Later("the entry is not in its chain as read"))) };
    let (lot_epoch, h) = wallet::lot_of(&s.entries[i], i.checked_sub(1).map(|j| &s.entries[j]), d.margin).map_err(String::from)?;
    // SEATED? The committee drawn for this place.
    match seated(ctx, lot, got, &d.beacon, &p.id(), q.seq, h, me) {
        Step::Want(w) => return Ok(Step::Want(w)),
        Step::Done(None) => return Ok(Step::Done(Verdict::Later("its lot is not drawn yet"))),
        Step::Done(Some(false)) => return Ok(Step::Done(Verdict::NotSeated)),
        _ => {}
    }
    // Every lot over the VALID STAKERS (`stakers`: witnessed stake, never the raw registry).
    let memo = crate::tokens::StakeMemo::default();
    let claims_of = |ep: u64| crate::tokens::stakers(ctx.code_hash, lot, got, d.beacon, &read, ep, &memo);
    // THE RULE, over what is read here (each read missing is asked for, then the rule run again).
    let sigs = |c: &[u8; 32], sq: u64, ep: u64| crate::tokens::witness_sigs(&read, &crate::tokens::witness_at(ctx.code_hash, lot, d.beacon, c, sq, ep));
    let outs = |hh: u64| match out_at(lot, got, &d.beacon, hh) {
        Err(w) => {
            want.borrow_mut().push(w);
            None
        }
        Ok(r) => r.ok(),
    };
    let stake_of = |o: &[u8; 32], at: u64| wallet::stake_from(&claims_of(at / supply::EPOCH), o);
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
        Ok(entry) => Verdict::Sign { at: crate::tokens::witness_at(ctx.code_hash, lot, d.beacon, &p.id(), q.seq, lot_epoch), entry },
        Err(why) => Verdict::Refused(why.to_string()),
    }))
}

/// SEATED: is witness key `me` drawn for place (`place`, `seq`), its lot at height `h` (beacon `beacon`): the claims
/// of that height's epoch, the committee drawn there — an entry's place, or an epoch statement's. `None`: its lot is not
/// drawn yet (or its output not recorded): not yet decidable.
#[allow(clippy::too_many_arguments)]
pub fn seated(ctx: &Ctx, lot: &Lot, got: Got, beacon: &[u8; 32], place: &[u8; 32], seq: u64, h: u64, me: &[u8; 32]) -> Step<Option<bool>> {
    let out = match out_at(lot, got, beacon, h) {
        Err(w) => return Step::Want(vec![w]),
        Ok(Err(_)) => return Step::Done(None),
        Ok(Ok(o)) => o,
    };
    let epoch = supply::epoch(h);
    // Over the VALID STAKERS (a claim nobody's witnessed stake backs takes no seat).
    let want: RefCell<Vec<[u8; 32]>> = RefCell::new(Vec::new());
    let read = |id: [u8; 32]| match there(got, &id) {
        Err(w) => {
            want.borrow_mut().push(w);
            None
        }
        Ok(b) => b,
    };
    let claims = crate::tokens::stakers(ctx.code_hash, lot, got, *beacon, &read, epoch, &crate::tokens::StakeMemo::default());
    let mut w = want.into_inner();
    if !w.is_empty() {
        w.sort();
        w.dedup();
        return Step::Want(w);
    }
    let valid = draw::valid(&lot.pool, epoch, &claims);
    let (k, _, _) = witness::seats(valid.len(), wallet::COMMITTEE);
    Step::Done(Some(draw::committee_of(&out, place, seq, &valid, k).iter().any(|m| claims[*m].key == *me)))
}

/// The SIGNATURES CONTAINER of epoch `epoch`'s statement (its witness place, seq 0, under its lot's claims epoch):
/// where every seated witness's signature on a root goes — made (empty) by whoever posts a root, so a witness with no
/// page open (the identity delegate: it cannot create a contract) only adds to it.
pub fn statement_at(ctx: &Ctx, lot: &Lot, epoch: u64) -> t::witness::Params {
    let h = craftworks_reputation::statement::lot_height(epoch);
    crate::tokens::witness_at(ctx.code_hash, lot, ctx.beacon, &craftworks_reputation::statement::witness_place(epoch), 0, supply::epoch(h))
}

/// An EPOCH STATEMENT's verdict for witness key `me`: the root this page computed for `epoch` (`root`: its own build,
/// never a posted one it did not compute) signed at the statement's place when `me` is seated there.
pub fn statement(ctx: &Ctx, lot: &Lot, got: Got, epoch: u64, root: &[u8; 32], me: &[u8; 32]) -> Step<Verdict> {
    let place = craftworks_reputation::statement::witness_place(epoch);
    let h = craftworks_reputation::statement::lot_height(epoch);
    match seated(ctx, lot, got, &ctx.beacon, &place, 0, h, me) {
        Step::Done(Some(true)) => Step::Done(Verdict::Sign { at: statement_at(ctx, lot, epoch), entry: *root }),
        Step::Done(Some(false)) => Step::Done(Verdict::NotSeated),
        Step::Done(None) => Step::Done(Verdict::Later("its lot is not drawn yet")),
        Step::Want(w) => Step::Want(w),
        Step::Sign(m) => Step::Sign(m),
    }
}

/// The WITNESS SIGNATURES each claimant gave on `queued` (`wallet::fees::count`, the one count) over what this page
/// read — each read missing asked for first.
pub fn signed(ctx: &Ctx, lot: &Lot, got: Got, queued: &[Queued], statements: &[craftworks_wallet::fees::Statement]) -> Step<std::collections::BTreeMap<[u8; 32], u64>> {
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
    let memo = crate::tokens::StakeMemo::default();
    let claims = |ep: u64| crate::tokens::stakers(ctx.code_hash, lot, got, beacon, &read, ep, &memo);
    let out = |h: u64| match out_at(lot, got, &beacon, h) {
        Err(w) => {
            want.borrow_mut().push(w);
            None
        }
        Ok(r) => r.ok(),
    };
    let sigs = |c: &[u8; 32], sq: u64, ep: u64| crate::tokens::witness_sigs(&read, &crate::tokens::witness_at(ctx.code_hash, lot, beacon, c, sq, ep));
    let counted = craftworks_wallet::fees::count(queued, statements, &craftworks_wallet::fees::Reads { pool: lot.pool, entry: &entry, claims: &claims, out: &out, sigs: &sigs });
    let mut w = want.into_inner();
    if w.is_empty() {
        return Step::Done(counted);
    }
    w.sort();
    w.dedup();
    Step::Want(w)
}

/// Which of `statements` (an epoch's posted roots) is ACCEPTED (`wallet::fees::statement_accepted`) — the first such
/// in order of their roots; `None`: none read as accepted here (reads it lacks count as none: ask it to read first).
pub fn accepted_statement(ctx: &Ctx, lot: &Lot, got: Got, statements: &[craftworks_wallet::fees::Statement]) -> Option<craftworks_wallet::fees::Statement> {
    let beacon = ctx.beacon;
    let read = |id: [u8; 32]| there(got, &id).ok().flatten();
    let memo = crate::tokens::StakeMemo::default();
    let claims = |ep: u64| crate::tokens::stakers(ctx.code_hash, lot, got, beacon, &read, ep, &memo);
    let out = |h: u64| out_at(lot, got, &beacon, h).ok().and_then(|r| r.ok());
    let sigs = |c: &[u8; 32], sq: u64, ep: u64| crate::tokens::witness_sigs(&read, &crate::tokens::witness_at(ctx.code_hash, lot, beacon, c, sq, ep));
    let entry = |_: &Queued| None;
    let r = craftworks_wallet::fees::Reads { pool: lot.pool, entry: &entry, claims: &claims, out: &out, sigs: &sigs };
    let mut sts = statements.to_vec();
    sts.sort_by_key(|s| s.root);
    sts.into_iter().find(|s| craftworks_wallet::fees::statement_accepted(&r, s))
}
