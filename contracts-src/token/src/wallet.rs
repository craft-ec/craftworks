//! WALLET VERIFICATION (docs/TOKEN.md §7a, foundation 6): what makes a balance VALUE. An entry is value once it is
//! ACCEPTED — enough of its COMMITTEE (drawn by lot from the stake claims, [`crate::draw`]) signed it. A witness signs
//! only by INDUCTION ([`may_witness`]): the chain holds as written, the entry before it is accepted, and every entry it
//! cites (the send a receive credits, the lock a fill names, the lock and fill a claim names) is accepted. So an
//! accepted entry stands on an accepted past, and a wallet checks ONE entry — the one paying it — and walks nothing.

use crate::draw::StakeClaim;
use crate::witness::{self, WitnessSig};
use crate::{supply, timed, walk, Body, ChainParams, Definition, Entry, Signer, State, Tok, Why};

/// The committee's most DRAWN seats; how many must sign follows the seats (`witness::seats`: ⌈⅔⌉ — 16 seats, 11; a
/// small network fewer, with the baseline's seat).
pub const COMMITTEE: usize = 16;
/// How far behind the beacon a witness still signs an entry's stamp: one epoch. Further back, a writer could pick, among
/// past epochs whose outputs are known, one whose committee for its place suits it (a first copy is taken without the
/// append rules); within it, a lot is one try an epoch. A witness has a whole epoch to reach an entry. (One it never
/// reached — its witnesses away — is not stuck: its owner SUPERSEDES it, TOKEN §3d, `crate::may_supersede`.)
pub const FRESH: u64 = supply::EPOCH;
/// The signatures a FULL committee needs (16 drawn seats, no baseline).
pub const NEED: usize = 11;

/// Where a wallet (or a witness) reads the lot from: the STAKE POOL's token (`pool`: whose stake weighs the draw — one
/// open pool for every token), its stake claims by epoch, the witness signatures held for (chain id, seq), the beacon's
/// OUTPUT at a height, and a claimer's REAL stake at a height ([`stake_at`] over its chain in the pool — read only for
/// those who signed). A place's signatures — (chain id, seq) — are read for a CLAIMS EPOCH (the third argument: its
/// lot's), from that epoch's container of the place (`witness::Params`: its registry says who may sign there).
pub struct Source<'a> {
    pub pool: [u8; 32],
    pub claims: &'a dyn Fn(u64) -> Vec<StakeClaim>,
    pub sigs: Sigs<'a>,
    pub out: &'a dyn Fn(u64) -> Option<[u8; 32]>,
    pub stake_of: &'a dyn Fn(&[u8; 32], u64) -> Option<witness::Stake>,
    /// A token's definition (its MARGIN sets when an entry's lot is drawn).
    pub definition: &'a dyn Fn(&[u8; 32]) -> Option<Definition>,
    /// A PROGRAM's state by its contract id (an escrow's release and payouts are checked against it).
    pub program: &'a dyn Fn(&[u8; 32]) -> Option<Vec<u8>>,
    /// A token's FREEZE LIST, by its definition (a debit of a frozen — or paused — account is never signed).
    pub freeze: &'a dyn Fn(&[u8; 32]) -> Option<crate::freeze::List>,
    /// An account's KEY LOG, by its DID (an owner entry is signed only on the account's current key).
    pub keys: &'a dyn Fn(&[u8; 32]) -> Option<craftworks_idlog_contract::Log>,
    /// A chain WHOLE — its history read back from its cut's blocks (`cut::whole`), where its live copy starts at a cut:
    /// an escrow, its release or a sale behind a cut is still found (audit 10-10: a seller's cut hid its escrow from
    /// the buyer's payout).
    pub history: &'a dyn Fn(&Tok, &[u8; 32]) -> Option<State>,
    /// A keeper's STANDING RECORD, by its key (the standing contract named by it: `standing::Params::keeper`) — what a
    /// stake claim's witness reads for its LIVENESS ([`live`]).
    pub standing: &'a dyn Fn(&[u8; 32]) -> Option<craftworks_standing::record::State>,
}

/// A place's SIGNATURES as read: (chain id, seq, the claims epoch of its lot) → those its container holds.
pub type Sigs<'a> = &'a dyn Fn(&[u8; 32], u64, u64) -> Vec<WitnessSig>;

/// A chain to find an entry `nonce` in: its live copy, or — the entry not there and the copy starting at a cut — its
/// whole history.
fn holding(src: &Source, chain: &dyn Fn(&Tok, &[u8; 32]) -> Option<State>, t: &Tok, o: &[u8; 32], has: &dyn Fn(&State) -> bool) -> Option<State> {
    let live = chain(t, o)?;
    if has(&live) || !live.entries.first().is_some_and(|e| matches!(e.body, Body::Cut { .. })) {
        return Some(live);
    }
    (src.history)(t, o).filter(|h| has(h)).or(Some(live))
}

/// The LOT of an entry: (the claims' epoch, the beacon height its output is drawn at). Fixed by the entry's PLACE —
/// never by what is written there: its base is its PREDECESSOR's stamp (two entries at one place share it, so they
/// share one committee, and anyone signing both is burned); drawn `margin + 2` steps past the base, and never before
/// the entry's own epoch's first step (an account idle for long is witnessed by the epoch's stakers, the claims frozen
/// before any of its outputs existed: [`crate::claims`]). A chain's first entry, with no predecessor, at its epoch's
/// first step. What stays choosable: writing in a LATER epoch draws again — one try an epoch.
pub fn lot_of(e: &Entry, pred: Option<&Entry>, margin: u64) -> Result<(u64, u64), Why> {
    // (The epoch's first STEP: its start + 1 — there is no step at height 0.)
    let start = supply::epoch(e.stamp) * supply::EPOCH + 1;
    let h = match pred {
        Some(p) => (p.stamp + margin + 2).max(start),
        None if e.seq == 0 => start,
        None => return Err("an entry whose predecessor is not held"),
    };
    Ok((supply::epoch(h), h))
}

/// Is `e` of `chain` ACCEPTED (`pred`: the entry before it in its chain — its lot's base)? `Ok`: how many of its
/// committee signed it.
pub fn accept(src: &Source, chain: &ChainParams, e: &Entry, pred: Option<&Entry>) -> Result<usize, Why> {
    let margin = (src.definition)(&chain.definition).ok_or("an entry of a token not known")?.margin;
    let (epoch, height) = lot_of(e, pred, margin)?;
    let out = (src.out)(height).ok_or("an entry whose lot is not drawn yet")?;
    let id = chain.id();
    let at = epoch * supply::EPOCH;
    witness::accepted(&src.pool, &id, e.seq, &e.hash(), &out, epoch, &(src.claims)(epoch), &(src.sigs)(&id, e.seq, epoch), COMMITTEE, &|o| (src.stake_of)(o, at))
}

/// Is standing epoch `epoch`'s STATEMENT root `root` ACCEPTED — its committee (drawn at `statement::lot_height`)
/// signed that root at its `statement::witness_place` (seq 0)? What a payout by a statement rests on (a keeper mint's
/// line): a root anyone may post, value only once witnessed.
pub fn statement_accept(src: &Source, epoch: u64, root: &[u8; 32]) -> Result<usize, Why> {
    use craftworks_reputation::statement::{lot_height, witness_place};
    let h = lot_height(epoch);
    let out = (src.out)(h).ok_or("a statement whose lot is not drawn yet")?;
    let ce = h / supply::EPOCH;
    let place = witness_place(epoch);
    witness::accepted(&src.pool, &place, 0, root, &out, ce, &(src.claims)(ce), &(src.sigs)(&place, 0, ce), COMMITTEE, &|o| (src.stake_of)(o, ce * supply::EPOCH))
}

/// What VALID STAKE (TOKEN §7a) is read from: the POINTERS posted for an epoch (every bag of its chain, decoded), a
/// staker's POOL CHAIN (its params and state, whole from seq 0), the beacon's outputs, a place's witness signatures. A
/// read not there: `None` / empty.
pub struct StakeReads<'a> {
    pub pool: [u8; 32],
    pub pool_def: &'a Definition,
    pub pointers: &'a dyn Fn(u64) -> Vec<crate::pointers::Pointer>,
    pub chain: &'a dyn Fn(&[u8; 32]) -> Option<(ChainParams, State)>,
    pub out: &'a dyn Fn(u64) -> Option<[u8; 32]>,
    pub sigs: Sigs<'a>,
}

/// TESTS ONLY (`testlot`): each check of [`valid_stakers`] and the claim witness's liveness ([`live`]) can be turned
/// off — the NEGATIVE CONTROLS that show a test fails without the mechanism it tests. Never in a build.
#[cfg(any(test, feature = "testlot"))]
pub mod switches {
    use std::cell::Cell;
    thread_local! {
        /// The pointer PREFILTER (`Pointer::accepted`): off — every pointer's chain is read.
        pub static PREFILTER: Cell<bool> = const { Cell::new(true) };
        /// The ACCEPTANCE of a staker's last entry before the freeze: off — a claim nobody witnessed counts.
        pub static ACCEPTANCE: Cell<bool> = const { Cell::new(true) };
        /// The LIVENESS of a claim's writer (its standing record): off — a claim of a node that answered nothing is signed.
        pub static LIVENESS: Cell<bool> = const { Cell::new(true) };
        /// The FREEZE of pointers (a pointer dated at or past the epoch's freeze ignored): off — a late pointer counts.
        pub static LATE: Cell<bool> = const { Cell::new(true) };
    }
}
#[cfg(any(test, feature = "testlot"))]
fn on(s: &'static std::thread::LocalKey<std::cell::Cell<bool>>) -> bool {
    s.with(|c| c.get())
}
#[cfg(not(any(test, feature = "testlot")))]
macro_rules! on {
    ($x:expr) => {
        true
    };
}
#[cfg(any(test, feature = "testlot"))]
macro_rules! on {
    ($x:ident) => {
        on(&switches::$x)
    };
}

/// A staker's SEAT in claims epoch `epoch` as its pool chain (`cp`, `st`; the pool `d`) stands at the epoch's start —
/// before any acceptance is asked: its last `Body::Claim` stamped before the start is for `epoch`, no unstake after it,
/// its writer a device whose key the stake at the start burns for, and that stake at least the claim. `None`: no seat.
/// (Also the index of the staker's LAST entry before the start: what must be ACCEPTED before the freeze.)
pub fn seat_of(cp: &ChainParams, d: &Definition, st: &State, epoch: u64) -> Option<(StakeClaim, usize)> {
    let start = epoch.checked_mul(supply::EPOCH)?;
    if st.fork.is_some() {
        return None;
    }
    let n = st.entries.iter().take_while(|e| e.stamp < start).count();
    let j = n.checked_sub(1)?;
    let c = st.entries[..n].iter().rposition(|e| matches!(e.body, Body::Claim { .. }))?;
    let Body::Claim { from_epoch, amount } = st.entries[c].body else { return None };
    if from_epoch != epoch || st.entries[c + 1..n].iter().any(|e| matches!(e.body, Body::Unstake { .. })) {
        return None;
    }
    let Some(Signer::Device(key, _)) = crate::writer_at(st, c) else { return None };
    let (staked, keys) = stake_at(cp, d, st, start).ok()?;
    if staked < amount || !keys.contains(&key) {
        return None;
    }
    Some((StakeClaim { owner: cp.owner, amount, epoch, key }, j))
}

/// The VALID STAKERS of claims epoch `epoch`, `V(E)` (TOKEN §7a; audit C2b, 10-12): who stands in E's lots. The
/// CANDIDATES are `V(E−1)` and the stakers POINTED to for E before its freeze ([`crate::pointers`]: anyone posts one,
/// the bags freeze, and a pointer dated at or past the freeze is ignored here too; so a pointer is
/// first checked on CPU — its own signatures accept its claim before the freeze, `Pointer::accepted` — and only one that
/// passes has its chain read: a flood of forged pointers costs readers CPU, never a read, and never a seat). A candidate
/// counts if its pool chain gives it a SEAT ([`seat_of`]: a claim for E, its stake behind it, at E's start) and its LAST
/// entry before E's start is ACCEPTED — by its committee drawn from `V` of that entry's own lot epoch, counting only
/// signatures made BEFORE E's FREEZE (`witness::accepted_before` at `pointers::frozen_at(E)`). A witness signs an entry
/// only once its predecessor is accepted (`may_witness`), so that one acceptance vouches for the whole history — the
/// claim among it, which its witnesses signed only on its writer's LIVENESS ([`live`]: its standing record answered in
/// the epoch the claim is stamped in). So a sleeping node — no answer, no claim — is not seated, and a claim cannot be
/// filled, replaced or crowded out: only the writer writes its chain. FIXED AT THE FREEZE (10-11): an acceptance arriving
/// later never grows `V(E)`. `memo`: the epochs already computed in this evaluation.
pub fn valid_stakers(r: &StakeReads, epoch: u64, memo: &std::cell::RefCell<std::collections::BTreeMap<u64, Vec<StakeClaim>>>) -> Vec<StakeClaim> {
    if let Some(v) = memo.borrow().get(&epoch) {
        return v.clone();
    }
    // (Marked before the recursion: an epoch never asks for itself, but a read loop must end.)
    memo.borrow_mut().insert(epoch, Vec::new());
    if epoch == 0 || epoch.checked_mul(supply::EPOCH).is_none() {
        return Vec::new();
    }
    let freeze = crate::pointers::frozen_at(epoch);
    let before = valid_stakers(r, epoch - 1, memo);
    let pool_chain = |owner: &[u8; 32]| ChainParams { code_hash: r.pool_def.code_hash, definition: r.pool, owner: *owner, beacon: r.pool_def.beacon, idlog: r.pool_def.idlog, item: [0; 32] };
    // THE CANDIDATES: last epoch's stakers, then each pointed-to staker whose pointer passes the prefilter.
    let mut owners: Vec<[u8; 32]> = before.iter().map(|c| c.owner).collect();
    for p in (r.pointers)(epoch) {
        // FROZEN (review 10-12): a pointer dated at or past the freeze is nobody's candidate — every reader ignores it, as
        // every bag refused it after the freeze (`pointers::take`): `V(E)` the same for all.
        if owners.contains(&p.owner) || (p.at >= freeze && on!(LATE)) {
            continue;
        }
        if !on!(PREFILTER) || p.accepted(&r.pool, &pool_chain(&p.owner).id(), epoch, r.out, &before) {
            owners.push(p.owner);
        }
    }
    owners.sort();
    owners.dedup();
    let mut out: Vec<StakeClaim> = Vec::new();
    for owner in owners {
        let Some((cp, st)) = (r.chain)(&owner) else { continue };
        let Some((seat, j)) = seat_of(&cp, r.pool_def, &st, epoch) else { continue };
        let e = &st.entries[j];
        let Ok((lot_epoch, h)) = lot_of(e, j.checked_sub(1).map(|k| &st.entries[k]), r.pool_def.margin) else { continue };
        if lot_epoch >= epoch {
            continue;
        }
        if on!(ACCEPTANCE) {
            let Some(o) = (r.out)(h) else { continue };
            let prior = valid_stakers(r, lot_epoch, memo);
            let id = cp.id();
            let sigs = (r.sigs)(&id, e.seq, lot_epoch);
            if witness::accepted_before(&r.pool, &id, e.seq, &e.hash(), &o, lot_epoch, &prior, &sigs, COMMITTEE, &|w| stake_from(&prior, w), freeze).is_err() {
                continue;
            }
        }
        out.push(seat);
    }
    memo.borrow_mut().insert(epoch, out.clone());
    out
}

/// LIVENESS (TOKEN §7a, owner 10-12: "stakers offline must not stall epochs"): did the writer of a stake claim stamped
/// at `stamp` — its witness key `key`, its standing record `record` — ANSWER the beacon (a sampled height, checked from
/// scratch by its standing contract) in the claims epoch the claim is stamped in, at or before the claim? What a
/// claim's witness checks before it signs (`may_witness`): the record is read while it still holds that epoch (records
/// keep the newest few standing epochs), and the acceptance it signs keeps the verdict for good.
pub fn live(record: Option<&craftworks_standing::record::State>, key: &[u8; 32], stamp: u64) -> bool {
    let from = supply::epoch(stamp) * supply::EPOCH;
    record.is_some_and(|r| r.epochs.iter().any(|e| e.answers.iter().any(|a| a.key == *key && a.height >= from && a.height <= stamp)))
}

/// A staker's stake as its VALID claim stands (`valid_stakers`: its stake already proved — what a signature counts for).
pub fn stake_from(valid: &[StakeClaim], owner: &[u8; 32]) -> Option<witness::Stake> {
    valid.iter().find(|c| c.owner == *owner).map(|c| (c.amount, vec![c.key]))
}

/// A staker's STAKE at beacon height `at` — what its chain (`p`, in the pool `d`) held staked after its entries stamped
/// before `at` — and the keys that stake burns for: its device writer's then, and those it had retired while staked.
/// (Unbonding amounts stay burnable but are not counted.) `Err`: the chain as given starts after `at` (a cut) — read it
/// whole first (`cut::whole`).
pub fn stake_at(p: &ChainParams, d: &Definition, s: &State, at: u64) -> Result<witness::Stake, Why> {
    let n = s.entries.iter().take_while(|e| e.stamp < at).count();
    if s.entries.first().is_some_and(|e| e.seq != 0 && e.stamp >= at) {
        return Err("a stake before what is held");
    }
    let prefix = State { entries: s.entries[..n].to_vec(), fork: None };
    let c = walk(p, d, &prefix, &|_, _| None, &|_| None, None, 0, n, &crate::Extra::NONE).map_err(|(_, w)| w)?;
    let mut keys = c.retired;
    if let Some(Signer::Device(node, _)) = c.writer {
        keys.push(node);
    }
    // AN EQUIVOCATED CHAIN STAKES NOTHING (audit 10-11): a fork freezes the chain — no burn can be appended to it any
    // more (forking over a pending burn would otherwise keep its stake counting): its signatures count for nothing.
    if s.fork.is_some() {
        return Ok((0, keys));
    }
    Ok((c.staked, keys))
}

/// The entries `e` (of `p`'s chain) CITES in other chains, found in them as `chain` reads them: a receive's send; a
/// vesting step's vest. A mint cites its supply shard (signed by the mint authority, not witnessed); an escrow's
/// release and payouts, its program (checked by `program_entry`). `Err`: a cited entry not found.
/// Each with the entry before it in its chain (its lot's base; `None` if that chain is held from it).
#[allow(clippy::type_complexity)]
pub fn cited(p: &ChainParams, e: &Entry, chain: &dyn Fn(&Tok, &[u8; 32]) -> Option<State>) -> Result<Vec<(ChainParams, Entry, Option<Entry>)>, Why> {
    let find = |t: Tok, owner: [u8; 32], pick: &dyn Fn(&Entry) -> bool| -> Result<(ChainParams, Entry, Option<Entry>), Why> {
        let c = p.at(t, owner);
        let held = chain(&t, &owner).ok_or("a cited chain not held")?;
        let i = held.entries.iter().position(pick).ok_or("a cited entry not held")?;
        // Never on an entry its owner SUPERSEDED (TOKEN §3d): it was never value, and never will be.
        if crate::is_superseded(&held, held.entries[i].seq) {
            return Err("a cited entry its owner superseded");
        }
        Ok((c, held.entries[i].clone(), i.checked_sub(1).map(|j| held.entries[j].clone())))
    };
    Ok(match &e.body {
        Body::Receive { from, .. } | Body::Vested { from, .. } | Body::FeeClaim { from, .. } => vec![find(p.tok(), *from, &|x| crate::debit_for(x, &p.owner, e))?],
        // A holders' share: the claimant's chain of the copy — the entry that says it HELD the copy at the sale (its
        // last stamped at or before it) accepted.
        Body::HolderPayout { sold, copy, at, .. } => {
            let t = Tok { definition: sold.0.definition, item: crate::numbered_item(&sold.0.definition, *copy) };
            let held = chain(&t, &p.owner).ok_or("a cited chain not held")?;
            let i = held.entries.iter().rposition(|x| x.stamp <= *at).ok_or("a cited entry not held")?;
            vec![(p.at(t, p.owner), held.entries[i].clone(), i.checked_sub(1).map(|j| held.entries[j].clone()))]
        }
        _ => vec![],
    })
}

/// THE WITNESS RULE: may a drawn witness sign `p`'s entry at `seq` (of token `d`)? The chain UP TO that entry holds — `holds`, the
/// contract's check from scratch, run on that prefix so the entry is among the last the contract checks in full (its
/// citations against what they cite, not only their names); for a CUT, also the history it summarises read whole and audited (`cut::whole`,
/// `cut::audit` — a holder that never had the prefix takes a cut as given) — and is not forked; the entry before it is
/// accepted; every entry it cites is accepted; the TIMED rules hold at its own stamp ([`timed`]: a refund, a release, a
/// fill, a citation — append rules a first copy skips); and it is stamped neither ahead of the beacon nor more than
/// [`FRESH`] behind it (`now`: the beacon's height as
/// the witness reads it — a first copy is taken by a node without the append rules, so a stamp from the future, which
/// would let a timed entry such as a vesting step or a refund come early, is refused here). `Ok`: the entry's hash, what
/// the witness signs. (Never sign two entries
/// at one seq: that is the equivocation that burns the witness's stake — the witness's own memory.)
#[allow(clippy::too_many_arguments)]
pub fn may_witness(src: &Source, p: &ChainParams, d: &Definition, s: &State, seq: u64, now: u64, holds: &dyn Fn(&State) -> Result<(), Why>, chain: &dyn Fn(&Tok, &[u8; 32]) -> Option<State>) -> Result<[u8; 32], Why> {
    if s.fork.is_some() {
        return Err("a forked chain");
    }
    let i = s.entries.iter().position(|e| e.seq == seq).ok_or("an entry not held")?;
    // A SUPERSEDED entry is void (TOKEN §3d): never signed, whatever its stamp.
    if crate::is_superseded(s, seq) {
        return Err("an entry its owner superseded");
    }
    let prefix = State { entries: s.entries[..=i].to_vec(), fork: None };
    holds(&prefix)?;
    let e = &s.entries[i];
    if e.stamp > now {
        return Err("an entry stamped ahead of the beacon");
    }
    if e.stamp.saturating_add(FRESH) < now {
        return Err("an entry stamped too long ago");
    }
    // A SPEND (an allowance's or an operator's) is witnessed within the margin of its stamp, as an append takes it — a
    // spender whose permission was turned off cannot date one back to before (audit 10-10: up to an epoch back).
    if matches!(e.body, Body::Spend { .. }) && e.stamp.saturating_add(d.margin) < now {
        return Err("a spend stamped too long ago");
    }
    timed(d, e.stamp, &prefix, e)?;
    // FROZEN or PAUSED (TOKEN §2): a debit of an account its freeze list said frozen at the entry's stamp — the append
    // rule a first copy skips (audit 10-10).
    // (A supersede moves no value: what it takes back never was.)
    let debit = !matches!(e.body, Body::Supersede { .. }) && e.balance < i.checked_sub(1).map_or(0, |j| s.entries[j].balance);
    if d.freeze.is_some() && debit {
        let list = (src.freeze)(&p.definition).ok_or("the freeze list not read")?;
        if list.frozen_at(&p.owner, e.stamp) {
            return Err("a debit of a frozen account");
        }
    }
    // A place already TAKEN: another entry at this seq accepted (an owner's later outranking entry, or a fork shown to
    // another committee) — never a second one (audit 10-10).
    taken(src, p, d.margin, s, i)?;
    // A RELEASE or PAYOUT of a program: the program BACKED by real, accepted escrows to it.
    program_entry(src, p, s, i, chain)?;
    // A STAKE CLAIM: its writer LIVE — its standing record answered in the epoch it is stamped in ([`live`]).
    if matches!(e.body, Body::Claim { .. }) && on!(LIVENESS) {
        let Some(Signer::Device(key, _)) = crate::writer_at(s, i) else { return Err("a stake claim with no device writer") };
        if !live((src.standing)(&key).as_ref(), &key, e.stamp) {
            return Err("a stake claim whose writer answered nothing this epoch");
        }
    }
    // A KEEPER MINT: its line under the statement root ACCEPTED for its epoch (the contract checked the line holds
    // under the root it names; any root may be posted).
    if let Body::KeeperMint { epoch, root, .. } = &e.body {
        statement_accept(src, *epoch, root)?;
    }
    match (&e.body, i.checked_sub(1)) {
        // A SUPERSEDE stands on the entry before what it voids, not on its (void) predecessor (TOKEN §3d).
        (Body::Supersede { from }, _) => supersede_rests(src, p, d.margin, s, i, *from)?,
        (_, Some(j)) => {
            accept(src, p, &s.entries[j], j.checked_sub(1).map(|k| &s.entries[k]))?;
        }
        (_, None) if seq != 0 => return Err("an entry whose predecessor is not held"),
        (_, None) => {}
    }
    // On the account's CURRENT key (the append rule a first copy skips): a device stolen before a rotation signs
    // nothing a witness takes.
    crate::on_current_key(s, i, (src.keys)(&p.owner).as_ref())?;
    for (c, x, before) in cited(p, e, chain)? {
        accept(src, &c, &x, before.as_ref())?;
    }
    Ok(e.hash())
}

/// What a SUPERSEDE (entry `i` of `s`, voiding seqs `from` on) stands on (TOKEN §3d): the entry before `from` ACCEPTED
/// (so it voids only an unaccepted tail), and NO witness signature at any place it voids — under each place's own lot
/// (where its committee signs). What the contract already holds it to (`walk`): a tail stamped so long ago that no
/// witness signs it, nor any container takes a signature of it, again (`may_supersede`). So nothing anyone was credited
/// by — nor anything a committee began to sign — is ever voided; and a voided entry can never become value after.
pub fn supersede_rests(src: &Source, p: &ChainParams, margin: u64, s: &State, i: usize, from: u64) -> Result<(), Why> {
    let id = p.id();
    let j = s.entries[..i].iter().position(|x| x.seq == from).ok_or("a supersede of nothing held")?;
    match j.checked_sub(1) {
        Some(k) => {
            accept(src, p, &s.entries[k], k.checked_sub(1).map(|m| &s.entries[m]))?;
        }
        None if from != 0 => return Err("an entry whose predecessor is not held"),
        None => {}
    }
    for k in j..i {
        let x = &s.entries[k];
        let (lot_epoch, _) = lot_of(x, k.checked_sub(1).map(|m| &s.entries[m]), margin)?;
        if (src.sigs)(&id, x.seq, lot_epoch).iter().any(|w| w.chain == id && w.seq == x.seq) {
            return Err("a supersede of an entry a witness signed");
        }
    }
    Ok(())
}

/// A program's section BACKED (TOKEN §5): it holds (conservation per token), and every escrow it lists is an `Escrow`
/// to THIS program, of that amount, in the chain it names, and ACCEPTED. What a program pays out is then only ever
/// what real escrows put in — whatever the program does.
pub fn program_backed(src: &Source, base: &ChainParams, prog: &[u8; 32], chain: &dyn Fn(&Tok, &[u8; 32]) -> Option<State>) -> Result<crate::escrow::Section, Why> {
    let b = (src.program)(prog).ok_or("the program is not read")?;
    let (sec, rest) = crate::escrow::Section::decode_prefix(&b).ok_or("a program with no escrow section")?;
    sec.holds()?;
    let mut rooted: Option<u64> = None;
    for ((tok, owner, nonce), amount, th) in &sec.escrows {
        let c = holding(src, chain, tok, owner, &|c: &State| c.entries.iter().any(|x| &x.nonce == nonce)).ok_or("an escrow's chain not read")?;
        let i = c.entries.iter().position(|x| &x.nonce == nonce).ok_or("a program listing an escrow that is not there")?;
        let x = &c.entries[i];
        let Body::Escrow { program, amount: a, terms, .. } = &x.body else { return Err("a program listing an escrow that is not there") };
        if program != prog || a != amount || crate::escrow::terms_hash(terms) != *th {
            return Err("a program listing an escrow that is not there");
        }
        // STATEMENT-ROOTED (the allowance): every such escrow names one epoch.
        if let Some(e) = crate::escrow::statement_epoch(terms) {
            if rooted.is_some_and(|r| r != e) {
                return Err("a program listing escrows for two epochs");
            }
            rooted = Some(e);
        }
        // Reclaimed by its owner (the program never took it in time): nothing of this program is signed on.
        if c.entries[i..].iter().any(|y| matches!(&y.body, Body::Reclaim { escrow } if escrow == nonce)) {
            return Err("a program listing an escrow reclaimed");
        }
        accept(src, &base.at(*tok, *owner), x, i.checked_sub(1).map(|j| &c.entries[j]))?;
    }
    // Its verdict's ROOT (right after its section) the one ACCEPTED for the epoch — a verdict under any other root backs
    // nothing (and its escrows are reclaimable).
    if let Some(e) = rooted {
        if !sec.refunds.is_empty() || !sec.pays.is_empty() {
            let root: [u8; 32] = rest.get(..32).and_then(|r| r.try_into().ok()).ok_or("a statement-rooted verdict with no root")?;
            statement_accept(src, e, &root)?;
        }
    }
    Ok(sec)
}

/// The witness rule for an `Unescrow` / `Payout` (entry `i` of `s`): its program backed, and the entry its verdict.
pub(crate) fn program_entry(src: &Source, p: &ChainParams, s: &State, i: usize, chain: &dyn Fn(&Tok, &[u8; 32]) -> Option<State>) -> Result<(), Why> {
    let e = &s.entries[i];
    match &e.body {
        Body::Unescrow { escrow, refund, verdict } => {
            let prog = s.entries[..i]
                .iter()
                .find_map(|x| match &x.body {
                    Body::Escrow { program, .. } if &x.nonce == escrow => Some(*program),
                    _ => None,
                })
                .ok_or("a release of no escrow held")?;
            let sec = program_backed(src, p, &prog, chain)?;
            (sec.refund_of(&(p.tok(), p.owner, *escrow)) == Some(*refund) && sec.hash() == *verdict).then_some(()).ok_or("a release that is not its program's verdict")
        }
        Body::Payout { program, key, amount, verdict, .. } => {
            let sec = program_backed(src, p, program, chain)?;
            (sec.pay(key) == Some((p.tok(), p.owner, *amount)) && sec.hash() == *verdict).then_some(()).ok_or("a payout that is not its program's verdict")?;
            released_under(src, p, &sec, chain)
        }
        // A reclaim: its program has NOT taken the escrow — as this witness reads it (not there, or not listing it).
        Body::Reclaim { escrow } => {
            let prog = s.entries[..i]
                .iter()
                .find_map(|x| match &x.body {
                    Body::Escrow { program, .. } if &x.nonce == escrow => Some(*program),
                    _ => None,
                })
                .ok_or("a release of no escrow held")?;
            // (A program whose listing is not backed — an escrow in it fake or reclaimed — took nothing: its verdict
            // is never paid out, so its escrows come back.)
            let listed = (src.program)(&prog).and_then(|b| crate::escrow::Section::decode_prefix(&b).map(|x| x.0)).is_some_and(|sec| sec.escrows.iter().any(|(r, ..)| *r == (p.tok(), p.owner, *escrow)));
            // (Judged on this chain as it stood BEFORE the reclaim — else the reclaim itself would unback the program.)
            let before = State { entries: s.entries[..i].to_vec(), fork: s.fork.clone() };
            let upto = |t: &Tok, o: &[u8; 32]| if *t == p.tok() && *o == p.owner { Some(before.clone()) } else { chain(t, o) };
            let took = listed && program_backed(src, p, &prog, &upto).is_ok();
            (!took).then_some(()).ok_or("a reclaim of an escrow its program took")
        }
        Body::HolderPayout { program, sold, at, verdict, .. } => {
            // The sale's time is the sold copy's ESCROW's stamp (one holder of each copy then): `at` must be it.
            let c = holding(src, chain, &sold.0, &sold.1, &|c: &State| sale_at(c, &sold.2).is_some()).ok_or("an escrow's chain not read")?;
            if sale_at(&c, &sold.2) != Some(*at) {
                return Err("a holders' share dated other than its sale");
            }
            let sec = program_backed(src, p, program, chain)?;
            if sec.hash() != *verdict {
                return Err("a holders' share that is not its program's verdict");
            }
            sec.pay(&crate::escrow::fee_key(sold)).filter(|(t, to, _)| *t == p.tok() && *to == crate::HOLDERS).ok_or("a holders' share that is not its program's verdict")?;
            released_under(src, p, &sec, chain)
        }
        _ => Ok(()),
    }
}

/// Is the place of entry `i` already TAKEN by a DIFFERENT entry its committee accepted? The lot is drawn from the
/// predecessor's stamp (and the entry's epoch), so the committees of every epoch from the predecessor's to this
/// entry's are asked (`margin`: its token's).
fn taken(src: &Source, p: &ChainParams, margin: u64, s: &State, i: usize) -> Result<(), Why> {
    let e = &s.entries[i];
    let id = p.id();
    let pred = i.checked_sub(1).map(|j| &s.entries[j]);
    let from = pred.map_or(supply::epoch(e.stamp), |x| supply::epoch(x.stamp));
    for epoch in from..=supply::epoch(e.stamp) {
        let start = epoch * supply::EPOCH + 1;
        let h = pred.map_or(start, |x| (x.stamp + margin + 2).max(start));
        // (Each lot's signatures are kept under its own claims epoch: `witness::Params`.)
        let sigs = (src.sigs)(&id, e.seq, supply::epoch(h));
        let mut others: Vec<[u8; 32]> = sigs.iter().filter(|x| x.chain == id && x.seq == e.seq && x.entry != e.hash()).map(|x| x.entry).collect();
        others.sort();
        others.dedup();
        if others.is_empty() {
            continue;
        }
        let Some(out) = (src.out)(h) else { continue };
        let at = supply::epoch(h) * supply::EPOCH;
        for other in &others {
            if witness::accepted(&src.pool, &id, e.seq, other, &out, supply::epoch(h), &(src.claims)(supply::epoch(h)), &sigs, COMMITTEE, &|o| (src.stake_of)(o, at)).is_ok() {
                return Err("a place another entry was accepted at");
            }
        }
    }
    Ok(())
}

/// ONE VERDICT PER PROGRAM (TOKEN §4): a payout is signed only once EVERY escrow its program lists was released under
/// THIS section — an accepted `Unescrow` naming its hash in the escrow's chain. An escrow is released once, so of two
/// verdicts a program's copies might hold (a crossing and a refund; two order sets), only one is ever paid out.
fn released_under(src: &Source, p: &ChainParams, sec: &crate::escrow::Section, chain: &dyn Fn(&Tok, &[u8; 32]) -> Option<State>) -> Result<(), Why> {
    let h = sec.hash();
    for ((tok, owner, nonce), ..) in &sec.escrows {
        let released = |c: &State| c.entries.iter().any(|x| matches!(&x.body, Body::Unescrow { escrow, verdict, .. } if escrow == nonce && *verdict == h));
        let c = holding(src, chain, tok, owner, &released).ok_or("an escrow's chain not read")?;
        let i = c.entries.iter().position(|x| matches!(&x.body, Body::Unescrow { escrow, verdict, .. } if escrow == nonce && *verdict == h)).ok_or("a payout before every escrow of its verdict is released")?;
        accept(src, &p.at(*tok, *owner), &c.entries[i], i.checked_sub(1).map(|j| &c.entries[j]))?;
    }
    Ok(())
}

/// When a copy was SOLD (TOKEN §1c): the stamp of its escrow `nonce` in its seller's chain — the moment its edition's
/// holders' shares are of.
pub fn sale_at(seller: &State, nonce: &[u8; 16]) -> Option<u64> {
    seller.entries.iter().find(|x| &x.nonce == nonce && matches!(x.body, Body::Escrow { .. })).map(|x| x.stamp)
}

/// THE WATCH (TOKEN §7a): the EQUIVOCATIONS in one place's witness signatures (`sigs`: its container) by the stakers of
/// `claims` — a key that signed two different entries at that (chain, seq). Each: (its staker, the burn to append to
/// that staker's chain of the pool — anyone may, with work). One per staker.
pub fn equivocations(sigs: &[WitnessSig], claims: &[StakeClaim]) -> Vec<([u8; 32], Body)> {
    let mut out: Vec<([u8; 32], Body)> = Vec::new();
    for c in claims {
        if out.iter().any(|(o, _)| *o == c.owner) {
            continue;
        }
        let mine: Vec<&WitnessSig> = sigs.iter().filter(|s| s.signed_by(&c.key)).collect();
        if let Some((a, b)) = mine.iter().flat_map(|a| mine.iter().map(move |b| (*a, *b))).find(|(a, b)| witness::equivocation(&c.key, a, b)) {
            out.push((c.owner, Body::Slash { key: c.key, a: a.clone(), b: b.clone() }));
        }
    }
    out
}
