//! THE FREEZE and THE POINTER INDEX (TOKEN §7a, audit C2b). A staker stands in claims epoch E's lots by a CLAIM entry in
//! its OWN pool chain (`Body::Claim`: only its writer writes there — no shared registry anyone can fill). Who stands is
//! FIXED at E's FREEZE, [`LEAD`] beacon steps before E starts: only acceptance by signatures made before it counts
//! (`wallet::valid_stakers`), so the set every lot of E is drawn from is fixed before any of E's beacon outputs exists.
//!
//! Readers FIND the claims through POINTERS: per (pool, epoch) a chain of bags ([`address`], the bag contract's chained
//! rule: a writer drops into the first bag with room, a reader reads on to it) of [`Pointer`]s — a staker's DID, its
//! claim's place and hash, and the WITNESS SIGNATURES that accepted it. Anyone may post or re-post one. A flood costs
//! readers only: a pointer is checked on CPU first ([`Pointer::accepted`]: its own signatures, by the committee drawn
//! over `V(E−1)`, accept its claim before the freeze) and only one that passes has its chain read.

use crate::supply;
use crate::witness::{self, WitnessSig};

/// Steps before its epoch starts that an epoch's stake FREEZES.
pub const LEAD: u64 = supply::EPOCH / 8;
/// The most signatures a pointer carries: a full committee and the baseline's.
pub const SIGS_MAX: usize = crate::wallet::COMMITTEE + 1;
const MAGIC: &[u8; 4] = b"CWP1";

/// THE FREEZE of claims epoch `epoch`: the beacon height from which nothing grows its VALID STAKE
/// (`wallet::valid_stakers`: only acceptance by signatures made below it counts) and after which no claim for it is
/// written (`Body::Claim`).
pub fn frozen_at(epoch: u64) -> u64 {
    epoch.saturating_mul(supply::EPOCH).saturating_sub(LEAD)
}

/// Bag `k` of the pointers to pool `pool`'s claims for epoch `epoch` (a bag contract's address).
pub fn address(pool: &[u8; 32], epoch: u64, k: u32) -> [u8; 32] {
    *blake3::Hasher::new_derive_key("craftworks 2026-10-12 stake pointers").update(pool).update(&epoch.to_le_bytes()).update(&k.to_le_bytes()).finalize().as_bytes()
}

/// A POINTER to staker `owner`'s claim: its pool chain's entry `seq` (hash `entry`), drawn at beacon height `height` (its
/// lot), and the witness signatures on it — each (key, signed at, signature).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pointer {
    pub owner: [u8; 32],
    pub seq: u64,
    pub entry: [u8; 32],
    pub height: u64,
    pub sigs: Vec<([u8; 32], u64, [u8; 64])>,
}

impl Pointer {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&self.owner);
        v.extend_from_slice(&self.seq.to_le_bytes());
        v.extend_from_slice(&self.entry);
        v.extend_from_slice(&self.height.to_le_bytes());
        v.push(self.sigs.len() as u8);
        for (k, at, s) in &self.sigs {
            v.extend_from_slice(k);
            v.extend_from_slice(&at.to_le_bytes());
            v.extend_from_slice(s);
        }
        v
    }
    pub fn decode(b: &[u8]) -> Option<Pointer> {
        let mut r = crate::R(b.strip_prefix(MAGIC)?);
        let (owner, seq, entry, height) = (r.arr()?, r.u64()?, r.arr()?, r.u64()?);
        let n = r.u8()? as usize;
        (n <= SIGS_MAX).then_some(())?;
        let sigs = (0..n).map(|_| Some((r.arr()?, r.u64()?, r.arr()?))).collect::<Option<Vec<_>>>()?;
        r.done().then_some(Pointer { owner, seq, entry, height, sigs })
    }
    /// The witness signatures it carries, at its place (`chain`: the staker's pool chain id).
    pub fn witness_sigs(&self, chain: &[u8; 32]) -> Vec<WitnessSig> {
        self.sigs.iter().map(|(_, at, sig)| WitnessSig { chain: *chain, seq: self.seq, entry: self.entry, at: *at, sig: *sig }).collect()
    }
    /// THE PREFILTER — on CPU, no read but the lot's beacon output (`out`): is this pointer's claim ACCEPTED for epoch
    /// `epoch` by the signatures it carries — its lot in epoch `epoch − 1` and below `epoch`'s freeze, signed by enough
    /// of the committee drawn there over `prior` (`V(epoch − 1)`), each signature made before the freeze? `chain`: the
    /// staker's pool chain id (its params are the pool's and the owner's). A forged pointer — a made-up owner, junk or
    /// borrowed signatures — fails here, and its chain is never read.
    pub fn accepted(&self, pool: &[u8; 32], chain: &[u8; 32], epoch: u64, out: &dyn Fn(u64) -> Option<[u8; 32]>, prior: &[crate::draw::StakeClaim]) -> bool {
        let freeze = frozen_at(epoch);
        if epoch == 0 || supply::epoch(self.height) != epoch - 1 || self.height >= freeze {
            return false;
        }
        let Some(o) = out(self.height) else { return false };
        witness::accepted_before(pool, chain, self.seq, &self.entry, &o, epoch - 1, prior, &self.witness_sigs(chain), crate::wallet::COMMITTEE, &|w| crate::wallet::stake_from(prior, w), freeze).is_ok()
    }
}
