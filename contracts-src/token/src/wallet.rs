//! WALLET VERIFICATION (docs/TOKEN.md §7a, foundation 6): what makes a balance VALUE. An entry is value once it is
//! ACCEPTED — enough of its COMMITTEE (drawn by lot from the stake claims, [`crate::draw`]) signed it. A witness signs
//! only by INDUCTION ([`may_witness`]): the chain holds as written, the entry before it is accepted, and every entry it
//! cites (the send a receive credits, the lock a fill names, the lock and fill a claim names) is accepted. So an
//! accepted entry stands on an accepted past, and a wallet checks ONE entry — the one paying it — and walks nothing.

use crate::draw::StakeClaim;
use crate::witness::{self, WitnessSig};
use crate::{offers_of, supply, walk, Body, ChainParams, Definition, Entry, Signer, State, Tok, Why};

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
}

/// The lot of an entry stamped `stamp`: the claims of its stamp's epoch, under the beacon output one step after it
/// (nobody knows the committee before the entry is written).
pub fn lot_of(stamp: u64) -> (u64, u64) {
    (supply::epoch(stamp), stamp + 1)
}

/// Is `e` of `chain` ACCEPTED? `Ok`: how many of its committee signed it.
pub fn accept(src: &Source, chain: &ChainParams, e: &Entry) -> Result<usize, Why> {
    let (epoch, height) = lot_of(e.stamp);
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
    let c = walk(p, d, &prefix, &|_, _| None, &|_| None, None, 0, n).map_err(|(_, w)| w)?;
    let mut keys = c.retired;
    if let Some(Signer::Device(node, _)) = c.writer {
        keys.push(node);
    }
    Ok((c.staked, keys))
}

/// The entries `e` (of `p`'s chain) CITES in other chains, found in them as `chain` reads them: a receive's send; a
/// fill's lock (the taker's chain of the offer's token); a claim's lock (the taker's chain) and fill (the maker's
/// chain of the lock's token). A mint cites its supply shard (signed by the mint authority, not witnessed); a refund
/// its own lock (accepted with its own past). `Err`: a cited entry not found.
pub fn cited(p: &ChainParams, s: &State, e: &Entry, chain: &dyn Fn(&Tok, &[u8; 32]) -> Option<State>) -> Result<Vec<(ChainParams, Entry)>, Why> {
    let find = |t: Tok, owner: [u8; 32], pick: &dyn Fn(&Entry) -> bool| -> Result<(ChainParams, Entry), Why> {
        let c = p.at(t, owner);
        let held = chain(&t, &owner).ok_or("a cited chain not held")?;
        held.entries.into_iter().find(|x| pick(x)).map(|x| (c, x)).ok_or("a cited entry not held")
    };
    Ok(match &e.body {
        Body::Receive { from, nonce, amount, cited, .. } => vec![find(p.tok(), *from, &|x| x.nonce == *nonce && x.stamp == *cited && matches!(x.body, Body::Send { to, amount: a } | Body::Spend { to, amount: a } | Body::Fill { taker: to, amount: a, .. } if to == p.owner && a == *amount))?],
        Body::Fill { offer, taker, lock, .. } => {
            let o = offers_of(s).into_iter().find(|o| &o.0 == offer).ok_or("a fill of an offer not held")?;
            vec![find(o.1, *taker, &|x| x.nonce == *lock)?]
        }
        Body::Claim { taker, lock, token, maker, .. } => vec![find(p.tok(), *taker, &|x| x.nonce == *lock)?, find(*token, *maker, &|x| matches!(&x.body, Body::Fill { lock: l, taker: t, .. } if l == lock && t == taker))?],
        _ => vec![],
    })
}

/// THE WITNESS RULE: may a drawn witness sign `p`'s entry at `seq`? The chain UP TO that entry holds — `holds`, the
/// contract's check from scratch, run on that prefix so the entry is among the last the contract checks in full (its
/// citations against what they cite, not only their names); for a CUT, also the history it summarises read whole and audited (`cut::whole`,
/// `cut::audit` — a holder that never had the prefix takes a cut as given) — and is not forked; the entry before it is
/// accepted; every entry it cites is accepted. `Ok`: the entry's hash, what the witness signs. (Never sign two entries
/// at one seq: that is the equivocation that burns the witness's stake — the witness's own memory.)
pub fn may_witness(src: &Source, p: &ChainParams, s: &State, seq: u64, holds: &dyn Fn(&State) -> Result<(), Why>, chain: &dyn Fn(&Tok, &[u8; 32]) -> Option<State>) -> Result<[u8; 32], Why> {
    if s.fork.is_some() {
        return Err("a forked chain");
    }
    let i = s.entries.iter().position(|e| e.seq == seq).ok_or("an entry not held")?;
    holds(&State { entries: s.entries[..=i].to_vec(), fork: None })?;
    let e = &s.entries[i];
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

/// THE BAGS (TOKEN §7a) a token's wallets and witnesses share — stake claims, witness signatures — one per kind, named
/// by the token, each item one record below.
pub fn bag_name(kind: &str, definition: &[u8; 32]) -> String {
    format!("token {kind} {}", definition.iter().map(|b| format!("{b:02x}")).collect::<String>())
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
