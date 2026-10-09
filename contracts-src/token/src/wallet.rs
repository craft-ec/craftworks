//! WALLET VERIFICATION (docs/TOKEN.md §7a, foundation 6): what makes a balance VALUE. An entry is value once it is
//! ACCEPTED — enough of its COMMITTEE (drawn by lot from the stake claims, [`crate::draw`]) signed it. A witness signs
//! only by INDUCTION ([`may_witness`]): the chain holds as written, the entry before it is accepted, and every entry it
//! cites (the send a receive credits, the lock a fill names, the lock and fill a claim names) is accepted. So an
//! accepted entry stands on an accepted past, and a wallet checks ONE entry — the one paying it — and walks nothing.

use crate::draw::StakeClaim;
use crate::witness::{self, WitnessSig};
use crate::{offers_of, supply, timed, walk, Body, ChainParams, Definition, Entry, Signer, State, Tok, Why};

/// The committee's size, and how many of it must sign (TOKEN §7a: open — to be set from measurement on the network).
pub const COMMITTEE: usize = 16;
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
}

/// The lot of an entry stamped `stamp` in a token of margin `margin`: the claims of its stamp's epoch, under the beacon
/// output TWO steps past the last height at which that stamp may still be appended (`stamp + margin`, the append rule).
/// So whoever writes it cannot know its committee in time to choose: not by picking a stamp within the margin, nor by
/// holding the entry back — and the step after is one a faster machine has not finished before everyone else either.
pub fn lot_of(stamp: u64, margin: u64) -> (u64, u64) {
    (supply::epoch(stamp), stamp + margin + 2)
}

/// Is `e` of `chain` ACCEPTED? `Ok`: how many of its committee signed it.
pub fn accept(src: &Source, chain: &ChainParams, e: &Entry) -> Result<usize, Why> {
    let margin = (src.definition)(&chain.definition).ok_or("an entry of a token not known")?.margin;
    let (epoch, height) = lot_of(e.stamp, margin);
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
    let c = walk(p, d, &prefix, &|_, _| None, &|_| None, None, 0, n, None).map_err(|(_, w)| w)?;
    let mut keys = c.retired;
    if let Some(Signer::Device(node, _)) = c.writer {
        keys.push(node);
    }
    Ok((c.staked, keys))
}

/// The entries `e` (of `p`'s chain) CITES in other chains, found in them as `chain` reads them: a receive's send; a
/// fill's lock (the taker's chain of the offer's token); a vesting step's vest; a claim's lock (the taker's chain) and fill (the maker's
/// chain of the lock's token). A mint cites its supply shard (signed by the mint authority, not witnessed); a refund
/// its own lock (accepted with its own past). `Err`: a cited entry not found.
pub fn cited(p: &ChainParams, s: &State, e: &Entry, chain: &dyn Fn(&Tok, &[u8; 32]) -> Option<State>) -> Result<Vec<(ChainParams, Entry)>, Why> {
    let find = |t: Tok, owner: [u8; 32], pick: &dyn Fn(&Entry) -> bool| -> Result<(ChainParams, Entry), Why> {
        let c = p.at(t, owner);
        let held = chain(&t, &owner).ok_or("a cited chain not held")?;
        held.entries.into_iter().find(|x| pick(x)).map(|x| (c, x)).ok_or("a cited entry not held")
    };
    Ok(match &e.body {
        Body::Receive { from, .. } | Body::Vested { from, .. } => vec![find(p.tok(), *from, &|x| crate::debit_for(x, &p.owner, e))?],
        Body::Fill { offer, taker, lock, .. } => {
            let o = offers_of(s).into_iter().find(|o| &o.0 == offer).ok_or("a fill of an offer not held")?;
            vec![find(o.1, *taker, &|x| x.nonce == *lock)?]
        }
        Body::Claim { taker, lock, token, maker, .. } => vec![find(p.tok(), *taker, &|x| x.nonce == *lock)?, find(*token, *maker, &|x| matches!(&x.body, Body::Fill { lock: l, taker: t, .. } if l == lock && t == taker))?],
        _ => vec![],
    })
}

/// THE WITNESS RULE: may a drawn witness sign `p`'s entry at `seq` (of token `d`)? The chain UP TO that entry holds — `holds`, the
/// contract's check from scratch, run on that prefix so the entry is among the last the contract checks in full (its
/// citations against what they cite, not only their names); for a CUT, also the history it summarises read whole and audited (`cut::whole`,
/// `cut::audit` — a holder that never had the prefix takes a cut as given) — and is not forked; the entry before it is
/// accepted; every entry it cites is accepted; the TIMED rules hold at its own stamp ([`timed`]: a refund, a release, a
/// fill, a citation — append rules a first copy skips); and it is not stamped ahead of the beacon (`now`: the beacon's height as
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
    timed(d, e.stamp, &prefix, e)?;
    match i.checked_sub(1) {
        Some(j) => {
            accept(src, p, &s.entries[j])?;
        }
        None if seq != 0 => return Err("an entry whose predecessor is not held"),
        None => {}
    }
    for (c, x) in cited(p, s, e, chain)? {
        accept(src, &c, &x)?;
    }
    Ok(e.hash())
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
