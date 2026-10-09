//! WALLET VERIFICATION (docs/TOKEN.md §7a, foundation 6): what makes a balance VALUE. An entry is value once it is
//! ACCEPTED — enough of its COMMITTEE (drawn by lot from the stake claims, [`crate::draw`]) signed it. A witness signs
//! only by INDUCTION ([`may_witness`]): the chain holds as written, the entry before it is accepted, and every entry it
//! cites (the send a receive credits, the lock a fill names, the lock and fill a claim names) is accepted. So an
//! accepted entry stands on an accepted past, and a wallet checks ONE entry — the one paying it — and walks nothing.

use crate::draw::StakeClaim;
use crate::witness::{self, WitnessSig};
use crate::{supply, timed, walk, Body, ChainParams, Definition, Entry, Signer, State, Tok, Why};

/// The committee's size, and how many of it must sign (TOKEN §7a: open — to be set from measurement on the network).
pub const COMMITTEE: usize = 16;
/// How far behind the beacon a witness still signs an entry's stamp: one epoch. Further back, a writer could pick, among
/// past epochs whose outputs are known, one whose committee for its place suits it (a first copy is taken without the
/// append rules); within it, a lot is one try an epoch. A witness has a whole epoch to reach an entry.
pub const FRESH: u64 = supply::EPOCH;
pub const NEED: usize = 11;

/// Where a wallet (or a witness) reads the lot from: the STAKE POOL's token (`pool`: whose stake weighs the draw — one
/// open pool for every token), its stake claims by epoch, the witness signatures held for (chain id, seq), the beacon's
/// OUTPUT at a height, and a claimer's REAL stake at a height ([`stake_at`] over its chain in the pool — read only for
/// those who signed).
pub struct Source<'a> {
    pub pool: [u8; 32],
    pub claims: &'a dyn Fn(u64) -> Vec<StakeClaim>,
    pub sigs: &'a dyn Fn(&[u8; 32], u64) -> Vec<WitnessSig>,
    pub out: &'a dyn Fn(u64) -> Option<[u8; 32]>,
    pub stake_of: &'a dyn Fn(&[u8; 32], u64) -> Option<witness::Stake>,
    /// A token's definition (its MARGIN sets when an entry's lot is drawn).
    pub definition: &'a dyn Fn(&[u8; 32]) -> Option<Definition>,
    /// A PROGRAM's state by its contract id (an escrow's release and payouts are checked against it).
    pub program: &'a dyn Fn(&[u8; 32]) -> Option<Vec<u8>>,
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
    witness::accepted(&src.pool, &id, e.seq, &e.hash(), &out, epoch, &(src.claims)(epoch), &(src.sigs)(&id, e.seq), COMMITTEE, NEED, &|o| (src.stake_of)(o, at))
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
        Ok((c, held.entries[i].clone(), i.checked_sub(1).map(|j| held.entries[j].clone())))
    };
    Ok(match &e.body {
        Body::Receive { from, .. } | Body::Vested { from, .. } => vec![find(p.tok(), *from, &|x| crate::debit_for(x, &p.owner, e))?],
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
    let prefix = State { entries: s.entries[..=i].to_vec(), fork: None };
    holds(&prefix)?;
    let e = &s.entries[i];
    if e.stamp > now {
        return Err("an entry stamped ahead of the beacon");
    }
    if e.stamp.saturating_add(FRESH) < now {
        return Err("an entry stamped too long ago");
    }
    timed(d, e.stamp, &prefix, e)?;
    // A RELEASE or PAYOUT of a program: the program BACKED by real, accepted escrows to it.
    program_entry(src, p, s, i, chain)?;
    match i.checked_sub(1) {
        Some(j) => {
            accept(src, p, &s.entries[j], j.checked_sub(1).map(|k| &s.entries[k]))?;
        }
        None if seq != 0 => return Err("an entry whose predecessor is not held"),
        None => {}
    }
    for (c, x, before) in cited(p, e, chain)? {
        accept(src, &c, &x, before.as_ref())?;
    }
    Ok(e.hash())
}

/// A program's section BACKED (TOKEN §5): it holds (conservation per token), and every escrow it lists is an `Escrow`
/// to THIS program, of that amount, in the chain it names, and ACCEPTED. What a program pays out is then only ever
/// what real escrows put in — whatever the program does.
pub fn program_backed(src: &Source, base: &ChainParams, prog: &[u8; 32], chain: &dyn Fn(&Tok, &[u8; 32]) -> Option<State>) -> Result<crate::escrow::Section, Why> {
    let b = (src.program)(prog).ok_or("the program is not read")?;
    let (sec, _) = crate::escrow::Section::decode_prefix(&b).ok_or("a program with no escrow section")?;
    sec.holds()?;
    for ((tok, owner, nonce), amount, th) in &sec.escrows {
        let c = chain(tok, owner).ok_or("an escrow's chain not read")?;
        let i = c.entries.iter().position(|x| &x.nonce == nonce).ok_or("a program listing an escrow that is not there")?;
        let x = &c.entries[i];
        if !matches!(&x.body, Body::Escrow { program, amount: a, terms } if program == prog && a == amount && crate::escrow::terms_hash(terms) == *th) {
            return Err("a program listing an escrow that is not there");
        }
        accept(src, &base.at(*tok, *owner), x, i.checked_sub(1).map(|j| &c.entries[j]))?;
    }
    Ok(sec)
}

/// The witness rule for an `Unescrow` / `Payout` (entry `i` of `s`): its program backed, and the entry its verdict.
fn program_entry(src: &Source, p: &ChainParams, s: &State, i: usize, chain: &dyn Fn(&Tok, &[u8; 32]) -> Option<State>) -> Result<(), Why> {
    let e = &s.entries[i];
    match &e.body {
        Body::Unescrow { escrow, refund } => {
            let prog = s.entries[..i]
                .iter()
                .find_map(|x| match &x.body {
                    Body::Escrow { program, .. } if &x.nonce == escrow => Some(*program),
                    _ => None,
                })
                .ok_or("a release of no escrow held")?;
            let sec = program_backed(src, p, &prog, chain)?;
            (sec.refund_of(&(p.tok(), p.owner, *escrow)) == Some(*refund)).then_some(()).ok_or("a release that is not its program's verdict")
        }
        Body::Payout { program, key, amount, .. } => {
            let sec = program_backed(src, p, program, chain)?;
            (sec.pay(key) == Some((p.tok(), p.owner, *amount))).then_some(()).ok_or("a payout that is not its program's verdict")
        }
        _ => Ok(()),
    }
}

/// THE WATCH (TOKEN §7a): the EQUIVOCATIONS in one place's witness signatures (`sigs`: its bag) by the stakers of
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

/// THE BAGS (TOKEN §7a) the lot is read from — open sets anyone adds to (`bag` contracts, their params these addresses):
/// a pool's STAKE CLAIMS of one epoch, and the WITNESS SIGNATURES of one entry (chain id, seq) — what a wallet reads to
/// accept that entry, one small bag per payment. Each item one record below.
pub fn stakers_bag(pool: &[u8; 32], epoch: u64) -> [u8; 32] {
    *blake3::Hasher::new_derive_key("craftworks token stakers bag").update(pool).update(&epoch.to_le_bytes()).finalize().as_bytes()
}
pub fn witness_bag(chain: &[u8; 32], seq: u64) -> [u8; 32] {
    *blake3::Hasher::new_derive_key("craftworks token witness bag").update(chain).update(&seq.to_le_bytes()).finalize().as_bytes()
}

/// The records' BYTES — exact: trailing bytes refused.
pub mod bytes {
    use crate::draw::StakeClaim;
    use crate::witness::WitnessSig;
    use crate::R;

    pub fn witness_sig(w: &WitnessSig) -> Vec<u8> {
        let mut v = Vec::new();
        w.put(&mut v);
        v
    }
    pub fn read_witness_sig(b: &[u8]) -> Option<WitnessSig> {
        let mut r = R(b);
        let w = WitnessSig::read(&mut r)?;
        r.done().then_some(w)
    }

    pub fn stake_claim(c: &StakeClaim) -> Vec<u8> {
        [&c.owner[..], &c.amount.to_le_bytes(), &c.epoch.to_le_bytes(), &c.key, &c.sig].concat()
    }
    pub fn read_stake_claim(b: &[u8]) -> Option<StakeClaim> {
        let mut r = R(b);
        let c = StakeClaim { owner: r.arr()?, amount: r.u128()?, epoch: r.u64()?, key: r.arr()?, sig: r.arr()? };
        r.done().then_some(c)
    }
}
