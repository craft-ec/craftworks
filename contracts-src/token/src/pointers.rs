//! THE FREEZE and THE POINTER INDEX (TOKEN §7a, audit C2b). A staker stands in claims epoch E's lots by a CLAIM entry in
//! its OWN pool chain (`Body::Claim`: only its writer writes there — no shared registry anyone can fill). Who stands is
//! FIXED at E's FREEZE, [`LEAD`] beacon steps before E starts: only acceptance by signatures made before it counts
//! (`wallet::valid_stakers`), so the set every lot of E is drawn from is fixed before any of E's beacon outputs exists.
//!
//! Readers FIND the claims through POINTERS: per (pool, epoch) a chain of POINTER BAGS (a kind of the token contract,
//! [`Params`]: the bag contract's items and work, the chained rule — a writer drops into the first bag with room, a
//! reader reads on to it) of [`Pointer`]s — a staker's DID, its claim's place and hash, the WITNESS SIGNATURES that
//! accepted it, and the beacon height it was posted at. Anyone may post or re-post one. A flood costs readers only: a
//! pointer is checked on CPU first ([`Pointer::accepted`]: its own signatures, by the committee drawn over `V(E−1)`,
//! accept its claim before the freeze) and only one that passes has its chain read.
//!
//! THE BAGS FREEZE as the claims registry did (review 10-12: a pointer posted late grew `V(E)` for the readers that saw
//! it — their committees split): an update reads the BEACON in the same round and takes a new pointer only while the
//! beacon is below the epoch's freeze, and only one dated (`Pointer::at`) at or below the beacon's height and below the
//! freeze ([`take`]); from the freeze on a bag keeps what it holds. And every reader ignores a pointer dated at or past
//! the freeze (`wallet::valid_stakers`). What stays (as it did for the registry): a bag never made before the freeze
//! can be PUT whole later (a first copy is taken without the beacon) — honestly dated items only, or it is ignored.

use crate::supply;
use crate::witness::{self, WitnessSig};

/// Steps before its epoch starts that an epoch's stake FREEZES.
pub const LEAD: u64 = supply::EPOCH / 8;
/// The most signatures a pointer carries: a full committee and the baseline's.
pub const SIGS_MAX: usize = crate::wallet::COMMITTEE + 1;
const MAGIC: &[u8; 4] = b"CWP2";

/// THE FREEZE of claims epoch `epoch`: the beacon height from which nothing grows its VALID STAKE
/// (`wallet::valid_stakers`: only acceptance by signatures made below it counts) and after which no claim for it is
/// written (`Body::Claim`).
pub fn frozen_at(epoch: u64) -> u64 {
    epoch.saturating_mul(supply::EPOCH).saturating_sub(LEAD)
}

/// The kind tag of a pointer bag (params' first byte).
pub const POINTERS: u8 = 9;

/// Bag `k` of the pointers to pool `pool`'s claims for epoch `epoch` (the address its items' work is counted at).
pub fn address(pool: &[u8; 32], epoch: u64, k: u32) -> [u8; 32] {
    *blake3::Hasher::new_derive_key("craftworks 2026-10-12 stake pointers").update(pool).update(&epoch.to_le_bytes()).update(&k.to_le_bytes()).finalize().as_bytes()
}

/// A POINTER BAG's params: the token code, the stake pool, the beacon (its height is what freezes the bag), the claims
/// epoch, the bag's place in the epoch's chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    pub code_hash: [u8; 32],
    pub pool: [u8; 32],
    pub beacon: [u8; 32],
    pub epoch: u64,
    pub k: u32,
}

impl Params {
    pub fn encode(&self) -> Vec<u8> {
        [&[POINTERS][..], &self.code_hash, &self.pool, &self.beacon, &self.epoch.to_le_bytes(), &self.k.to_le_bytes()].concat()
    }
    pub fn decode(b: &[u8]) -> Option<Params> {
        let mut r = crate::R(b);
        (r.u8()? == POINTERS).then_some(())?;
        let p = Params { code_hash: r.arr()?, pool: r.arr()?, beacon: r.arr()?, epoch: r.u64()?, k: u32::from_le_bytes(r.arr()?) };
        r.done().then_some(p)
    }
    pub fn id(&self) -> [u8; 32] {
        crate::id(&self.code_hash, &self.encode())
    }
    /// The address its items' work is counted at.
    pub fn address(&self) -> [u8; 32] {
        address(&self.pool, self.epoch, self.k)
    }
    /// Its pointers as a state holds them (none if it does not read).
    pub fn pointers(&self, state: &[u8]) -> Vec<Pointer> {
        read(self, state).unwrap_or_default().iter().filter_map(|i| Pointer::decode(craftworks_bag_contract::payload(i))).collect()
    }
    /// A state holding these pointers (each item ground for its work: what a writer PUTs, or sends as an update).
    pub fn state(&self, pointers: &[Pointer]) -> Vec<u8> {
        let a = self.address();
        let items: Vec<Vec<u8>> = pointers.iter().map(|x| craftworks_bag_contract::grind(&a, &x.encode())).collect();
        craftworks_bag_contract::encode(&a, &craftworks_bag_contract::merge(&a, &[], &items))
    }
}

/// A bag's items, if its state reads (the bag contract's rules: work, caps, order) and every item is a pointer.
pub fn read(p: &Params, state: &[u8]) -> Option<Vec<Vec<u8>>> {
    let items = craftworks_bag_contract::read(&p.address(), state)?;
    items.iter().all(|i| Pointer::decode(craftworks_bag_contract::payload(i)).is_some()).then_some(items)
}

/// Is a bag of these pointers' items FULL (the chained rule: writers move on, readers read on)?
pub fn full(items: &[Vec<u8>]) -> bool {
    craftworks_bag_contract::chained::full(items.iter().map(|i| craftworks_bag_contract::payload(i).len()))
}

/// THE FREEZE at admission: `held` takes the pointers of `given` (items) only while the beacon (`now`) is below the
/// epoch's freeze — each dated at or below `now` and below the freeze — merged by the bag's rule; from the freeze on it
/// keeps what it holds (a pointer arriving late is ignored, never a reason to refuse the copy: copies keep syncing).
pub fn take(p: &Params, held: &[Vec<u8>], given: &[Vec<u8>], now: u64) -> Vec<Vec<u8>> {
    let freeze = frozen_at(p.epoch);
    if now >= freeze {
        return held.to_vec();
    }
    let fresh: Vec<Vec<u8>> = given.iter().filter(|i| Pointer::decode(craftworks_bag_contract::payload(i)).is_some_and(|x| x.at <= now && x.at < freeze)).cloned().collect();
    craftworks_bag_contract::merge(&p.address(), held, &fresh)
}

/// A POINTER to staker `owner`'s claim: its pool chain's entry `seq` (hash `entry`), drawn at beacon height `height` (its
/// lot), the witness signatures on it — each (key, signed at, signature) — and `at`, the beacon height it was posted at
/// (checked against the beacon when a bag takes it: [`take`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pointer {
    pub owner: [u8; 32],
    pub seq: u64,
    pub entry: [u8; 32],
    pub height: u64,
    pub at: u64,
    pub sigs: Vec<([u8; 32], u64, [u8; 64])>,
}

impl Pointer {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&self.owner);
        v.extend_from_slice(&self.seq.to_le_bytes());
        v.extend_from_slice(&self.entry);
        v.extend_from_slice(&self.height.to_le_bytes());
        v.extend_from_slice(&self.at.to_le_bytes());
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
        let (owner, seq, entry, height, at) = (r.arr()?, r.u64()?, r.arr()?, r.u64()?, r.u64()?);
        let n = r.u8()? as usize;
        (n <= SIGS_MAX).then_some(())?;
        let sigs = (0..n).map(|_| Some((r.arr()?, r.u64()?, r.arr()?))).collect::<Option<Vec<_>>>()?;
        r.done().then_some(Pointer { owner, seq, entry, height, at, sigs })
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
