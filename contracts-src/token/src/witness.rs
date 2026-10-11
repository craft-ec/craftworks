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
        craftworks_identity::witness_message(chain, seq, entry)
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

/// A staker's STAKE as a lot reads it: the amount, and the keys it burns for.
pub type Stake = (u128, Vec<[u8; 32]>);

/// Two witness signatures by `key`, at one (chain, seq), naming different entries.
pub fn equivocation(key: &[u8; 32], a: &WitnessSig, b: &WitnessSig) -> bool {
    a.chain == b.chain && a.seq == b.seq && a.entry != b.entry && a.signed_by(key) && b.signed_by(key)
}

/// ACCEPTED (TOKEN §7a): the entry `entry` at (`chain`, `seq`) carries at least `need` signatures from its committee —
/// the `k` lowest tickets among the `claims` of `epoch` under the beacon output `out` — each by a member whose REAL
/// stake covers its claim and whose claim's key is one its stake can be burned for (`stake_of`: its chain's staked
/// amount and burnable keys — its writer's, and those it retired while staked — as of the epoch's start; read only for
/// those who signed). Returns how many counted. A signature by anyone else, for another entry, by an over-claimer, or
/// under a key its chain's stake does not answer for counts for nothing.
/// THE BASELINE (owner 10-10: "wren account start as a baseline … so it scale from that"): Wren's witness key — the
/// public key of the device Wren's words credentialed. While an epoch has fewer than [`BASELINE_UNTIL`] valid stake
/// claims it holds a SEAT in every committee, its signature counted WITHOUT stake (it has none to burn: a visible
/// bootstrap trust in Wren); from that many live stakers on it has no seat — stepping out by itself.
#[cfg(not(feature = "testlot"))]
pub const BASELINE: [u8; 32] = [0xeb, 0x51, 0x44, 0x08, 0x05, 0xff, 0xd0, 0xe1, 0xb0, 0x92, 0x07, 0x4e, 0x4f, 0x16, 0x01, 0xd5, 0x3a, 0x66, 0x93, 0xc7, 0x4b, 0x92, 0x23, 0x2f, 0xa8, 0xd4, 0xc5, 0xa3, 0xce, 0xdc, 0x1b, 0x1c];
/// (TESTS of what is drawn over VALID STAKE — `testlot`: a baseline whose seed is known, `BASELINE_SEED` — so a test lot
/// bootstraps as the network does: the baseline signs the first stakers' entries.)
#[cfg(feature = "testlot")]
pub const BASELINE: [u8; 32] = [0xca, 0x7d, 0x75, 0xb4, 0xea, 0x1d, 0x5a, 0x86, 0x1a, 0xa5, 0x83, 0x84, 0x06, 0x78, 0x2d, 0xfc, 0x62, 0xcd, 0xee, 0x7b, 0x05, 0x9a, 0x33, 0x4f, 0x0d, 0xa1, 0x53, 0xd5, 0x93, 0x86, 0xa0, 0x9a];
#[cfg(feature = "testlot")]
pub const BASELINE_SEED: [u8; 32] = [0xBA; 32];
pub const BASELINE_UNTIL: usize = 6;

/// An epoch's SEATS for `live` valid claims (TOKEN §7d, a small network: committees SCALE with the live stakers): drawn
/// seats `min(max, live)`, the baseline's seat below [`BASELINE_UNTIL`], and the signatures needed — ⌈⅔⌉ of the seats.
pub fn seats(live: usize, max: usize) -> (usize, bool, usize) {
    let drawn = live.min(max);
    let base = live < BASELINE_UNTIL;
    let all = drawn + base as usize;
    (drawn, base, (2 * all).div_ceil(3))
}

/// Is entry (`chain`, `seq`, `entry`) ACCEPTED — signed by enough of its COMMITTEE (the lot over `epoch`'s `claims` drawn
/// from `out`; at most `max` drawn seats, and the BASELINE's seat while the network is small: [`seats`)? A claimer's
/// signature counts only with its stake (`stake_of`: its real amount and burnable keys — its writer's, and those it
/// retired while staked — as of the epoch's start; read only for those who signed). Returns how many counted. A
/// signature by anyone else, for another entry, by an over-claimer, or under a key its chain's stake does not answer
/// for counts for nothing.
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
    max: usize,
    stake_of: &dyn Fn(&[u8; 32]) -> Option<Stake>,
) -> Result<usize, crate::Why> {
    let valid = crate::draw::valid(definition, epoch, claims);
    let (k, base, need) = seats(valid.len(), max);
    let members = crate::draw::committee_of(out, chain, seq, &valid, k);
    let mine = |s: &&WitnessSig| s.chain == *chain && s.seq == seq && s.entry == *entry;
    let mut counted: Vec<usize> = Vec::new();
    for s in sigs.iter().filter(mine) {
        let Some(&m) = members.iter().find(|&&m| !counted.contains(&m) && s.signed_by(&claims[m].key)) else { continue };
        if stake_of(&claims[m].owner).is_some_and(|(real, keys)| real >= claims[m].amount && keys.contains(&claims[m].key)) {
            counted.push(m);
        }
    }
    let baseline = base && sigs.iter().filter(mine).any(|s| s.signed_by(&BASELINE));
    let n = counted.len() + baseline as usize;
    if n < need {
        return Err("an entry its committee has not witnessed");
    }
    Ok(n)
}
