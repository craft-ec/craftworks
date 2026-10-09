//! WITNESSES (TOKEN §7a): a witness — an account whose chain holds STAKE — signs an entry of another chain it re-checked:
//! "chain C's entry at seq S is the one with hash H". Two such signatures by ONE key at one (chain, seq) naming
//! different entries are an EQUIVOCATION — the proof that burns the witness's stake (`Body::Slash`).

use crate::sig_ok;

/// "Chain `chain`'s entry at `seq` is `entry`" — signed by a witness's writer key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WitnessSig {
    pub chain: [u8; 32],
    pub seq: u64,
    pub entry: [u8; 32],
    pub sig: [u8; 64],
}

impl WitnessSig {
    pub fn message(chain: &[u8; 32], seq: u64, entry: &[u8; 32]) -> Vec<u8> {
        [&b"craftworks token witness"[..], chain, &seq.to_le_bytes(), entry].concat()
    }
    pub fn signed_by(&self, key: &[u8; 32]) -> bool {
        sig_ok(key, &Self::message(&self.chain, self.seq, &self.entry), &self.sig)
    }
    pub(crate) fn put(&self, v: &mut Vec<u8>) {
        v.extend_from_slice(&self.chain);
        v.extend_from_slice(&self.seq.to_le_bytes());
        v.extend_from_slice(&self.entry);
        v.extend_from_slice(&self.sig);
    }
    pub(crate) fn read(r: &mut crate::R) -> Option<WitnessSig> {
        Some(WitnessSig { chain: r.arr()?, seq: r.u64()?, entry: r.arr()?, sig: r.arr()? })
    }
}

/// Two witness signatures by `key`, at one (chain, seq), naming different entries.
pub fn equivocation(key: &[u8; 32], a: &WitnessSig, b: &WitnessSig) -> bool {
    a.chain == b.chain && a.seq == b.seq && a.entry != b.entry && a.signed_by(key) && b.signed_by(key)
}

/// ACCEPTED (TOKEN §7a): the entry `entry` at (`chain`, `seq`) carries at least `need` signatures from its committee —
/// the `k` lowest tickets among the `claims` of `epoch` under the beacon output `out` — each by a member whose REAL
/// stake (`stake_of`: its chain's staked amount, read only for those who signed) covers its claim. Returns how many
/// counted. A signature by anyone else, for another entry, or by an over-claimer counts for nothing.
#[allow(clippy::too_many_arguments)]
pub fn accepted(
    definition: &[u8; 32],
    chain: &[u8; 32],
    seq: u64,
    entry: &[u8; 32],
    out: &[u8; 32],
    epoch: u64,
    claims: &[crate::draw::StakeClaim],
    sigs: &[WitnessSig],
    k: usize,
    need: usize,
    stake_of: &dyn Fn(&[u8; 32]) -> Option<u128>,
) -> Result<usize, crate::Why> {
    let members = crate::draw::committee(definition, out, chain, seq, epoch, claims, k);
    let mut counted: Vec<usize> = Vec::new();
    for s in sigs {
        if s.chain != *chain || s.seq != seq || s.entry != *entry {
            continue;
        }
        let Some(&m) = members.iter().find(|&&m| !counted.contains(&m) && s.signed_by(&claims[m].key)) else { continue };
        if stake_of(&claims[m].owner).is_some_and(|real| real >= claims[m].amount) {
            counted.push(m);
        }
    }
    if counted.len() < need {
        return Err("an entry its committee has not witnessed");
    }
    Ok(counted.len())
}
