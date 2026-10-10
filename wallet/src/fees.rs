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

/// A STATEMENT's place: its root signed at `statement::witness_place(epoch)` (seq 0), its lot drawn at `h`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Statement {
    pub place: [u8; 32],
    pub root: [u8; 32],
    pub h: u64,
}

/// The claimants SEATED for (`chain`, `seq`) whose signature over `entry` is in its bag (each seat once), and how many
/// of them acceptance still needs once the baseline's seat is counted.
fn seated(r: &Reads, chain: &[u8; 32], seq: u64, entry: &[u8; 32], h: u64) -> Option<(Vec<[u8; 32]>, usize)> {
    let o = (r.out)(h)?;
    let epoch = craftworks_token_contract::supply::epoch(h);
    let claims = (r.claims)(epoch);
    let valid = draw::valid(&r.pool, epoch, &claims);
    let (k, base, need) = witness::seats(valid.len(), wallet::COMMITTEE);
    let members = draw::committee_of(&o, chain, seq, &valid, k);
    let sigs = (r.sigs)(chain, seq);
    let signers: Vec<[u8; 32]> = members.iter().filter(|m| sigs.iter().any(|s| s.chain == *chain && s.seq == seq && s.entry == *entry && s.signed_by(&claims[**m].key))).map(|m| claims[*m].owner).collect();
    // (The baseline's seat counts toward acceptance — it earns nothing: the root never does.)
    let baseline = base && sigs.iter().any(|s| s.chain == *chain && s.seq == seq && s.entry == *entry && s.signed_by(&witness::BASELINE));
    Some((signers, need.saturating_sub(baseline as usize)))
}

/// Is statement `st`'s root ACCEPTED by this count's rule (its seated signatures, with the baseline's, reach the
/// committee's need)? The one test page and witnesses pick the epoch's root by (what the next statement carries on).
pub fn statement_accepted(r: &Reads, st: &Statement) -> bool {
    seated(r, &st.place, 0, &st.root, st.h).is_some_and(|(signers, need)| signers.len() >= need.max(1) || (need == 0))
}

/// The signatures each claimant gave on `queued` entries (each place once) and on the epoch STATEMENTS' roots
/// (`statements`: each posted root — counted only where its seated signatures reach the committee's need, so a root
/// nobody accepted pays nothing).
pub fn count(queued: &[Queued], statements: &[Statement], r: &Reads) -> BTreeMap<[u8; 32], u64> {
    let mut out: BTreeMap<[u8; 32], u64> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for q in queued {
        let Some((chain, e, pred, margin)) = (r.entry)(q) else { continue };
        if !seen.insert((chain, e.seq)) {
            continue;
        }
        let Ok((_, h)) = wallet::lot_of(&e, pred.as_ref(), margin) else { continue };
        for o in seated(r, &chain, e.seq, &e.hash(), h).map_or(vec![], |x| x.0) {
            *out.entry(o).or_default() += 1;
        }
    }
    for st in statements {
        if !seen.insert((st.place, 0)) {
            continue;
        }
        let Some((signers, need)) = seated(r, &st.place, 0, &st.root, st.h) else { continue };
        if signers.len() >= need.max(1) {
            for o in signers {
                *out.entry(o).or_default() += 1;
            }
        }
    }
    out
}
