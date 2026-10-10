//! WITNESS FEES (TOKEN §7d: the pot's 10% to witnesses, by the signatures they gave): the ONE count every statement
//! builder runs (pages and witnesses alike) — for each entry queued for its witnesses in the epoch, each signature in
//! its witness bag over THAT entry by a key SEATED for its place (the committee its lot drew from the claims), once per
//! seat; counted for the seat's claimant (a DID). Spam signatures (unseated keys, other entries) count nothing.

use crate::queue::Queued;
use craftworks_token_contract::{draw, wallet, witness, Entry};
use std::collections::{BTreeMap, BTreeSet};

/// What the count reads.
pub struct Reads<'a> {
    /// The stake pool's token (the lot's).
    pub pool: [u8; 32],
    /// A queued entry's place: (its chain's id, the entry, the one before it, its token's margin) — `None`: not read.
    pub entry: &'a dyn Fn(&Queued) -> Option<([u8; 32], Entry, Option<Entry>, u64)>,
    pub claims: &'a dyn Fn(u64) -> Vec<draw::StakeClaim>,
    pub out: &'a dyn Fn(u64) -> Option<[u8; 32]>,
    pub sigs: &'a dyn Fn(&[u8; 32], u64) -> Vec<witness::WitnessSig>,
}

/// The signatures each claimant gave on `queued` (each place once).
pub fn count(queued: &[Queued], r: &Reads) -> BTreeMap<[u8; 32], u64> {
    let mut out: BTreeMap<[u8; 32], u64> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for q in queued {
        let Some((chain, e, pred, margin)) = (r.entry)(q) else { continue };
        if !seen.insert((chain, e.seq)) {
            continue;
        }
        let Ok((epoch, h)) = wallet::lot_of(&e, pred.as_ref(), margin) else { continue };
        let Some(o) = (r.out)(h) else { continue };
        let claims = (r.claims)(epoch);
        let valid = draw::valid(&r.pool, epoch, &claims);
        let (k, _, _) = witness::seats(valid.len(), wallet::COMMITTEE);
        let members = draw::committee_of(&o, &chain, e.seq, &valid, k);
        let hash = e.hash();
        let sigs = (r.sigs)(&chain, e.seq);
        for m in members {
            if sigs.iter().any(|s| s.chain == chain && s.seq == e.seq && s.entry == hash && s.signed_by(&claims[m].key)) {
                *out.entry(claims[m].owner).or_default() += 1;
            }
        }
    }
    out
}
