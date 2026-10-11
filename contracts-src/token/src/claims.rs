//! THE CLAIMS REGISTRY (TOKEN §7a): who stands in an epoch's lots — one contract per (stake pool, epoch) (params
//! [`Params`]), a stake claim per staker. It FREEZES [`LEAD`] beacon steps before its epoch starts: from then on it
//! takes no claim, so the set every lot of the epoch is drawn from is fixed before any of its beacon outputs exists — a
//! staker cannot wait for an output and then claim only with the identities whose tickets are low.

use crate::draw::StakeClaim;
use crate::{supply, Why};

pub const CLAIMS: u8 = 7;
/// Steps before its epoch starts that a registry freezes.
pub const LEAD: u64 = supply::EPOCH / 8;
/// The most claims one registry holds.
pub const CLAIMS_MAX: usize = 4096;

/// A registry's PARAMS: the token code's hash, the stake pool's token, the beacon, the epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    pub code_hash: [u8; 32],
    pub pool: [u8; 32],
    pub beacon: [u8; 32],
    pub epoch: u64,
}

impl Params {
    pub fn encode(&self) -> Vec<u8> {
        [&[CLAIMS][..], &self.code_hash, &self.pool, &self.beacon, &self.epoch.to_le_bytes()].concat()
    }
    pub fn decode(b: &[u8]) -> Option<Params> {
        let mut r = crate::R(b);
        (r.u8()? == CLAIMS).then_some(())?;
        let p = Params { code_hash: r.arr()?, pool: r.arr()?, beacon: r.arr()?, epoch: r.u64()? };
        r.done().then_some(p)
    }
    pub fn id(&self) -> [u8; 32] {
        crate::id(&self.code_hash, &self.encode())
    }
    /// The beacon height from which it takes no claim.
    pub fn frozen_at(&self) -> u64 {
        frozen_at(self.epoch)
    }
}

/// THE FREEZE of claims epoch `epoch`'s registry: the beacon height from which it takes no claim — and the height
/// `epoch`'s VALID STAKE is fixed at (`wallet::valid_stakers`: only acceptance by signatures made below it counts).
pub fn frozen_at(epoch: u64) -> u64 {
    epoch.saturating_mul(supply::EPOCH).saturating_sub(LEAD)
}

/// A registry: one claim per (staker, key), sorted by both (audit 10-11: one per staker let anyone replace a staker's
/// claim with one of a lower hash under the forger's own key — the staker's signatures then counted for nothing; now
/// both stay, and only the one the staker's stake answers for is VALID, `wallet::valid_stakers`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Registry {
    pub claims: Vec<StakeClaim>,
}

impl Registry {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = b"CWCL".to_vec();
        v.extend_from_slice(&(self.claims.len() as u32).to_le_bytes());
        for c in &self.claims {
            v.extend_from_slice(&crate::wallet::bytes::stake_claim(c));
        }
        v
    }
    pub fn decode(b: &[u8]) -> Option<Registry> {
        let mut r = crate::R(b.strip_prefix(b"CWCL")?);
        let n = r.u32()? as usize;
        (n <= CLAIMS_MAX).then_some(())?;
        let len = crate::wallet::bytes::stake_claim(&StakeClaim { owner: [0; 32], amount: 0, epoch: 0, key: [0; 32], sig: [0; 64] }).len();
        let claims = (0..n).map(|_| crate::wallet::bytes::read_stake_claim(r.take(len)?)).collect::<Option<Vec<_>>>()?;
        (r.done() && claims.windows(2).all(|w| (w[0].owner, w[0].key) < (w[1].owner, w[1].key))).then_some(Registry { claims })
    }
}

/// Every claim of this epoch, signed by its key (`StakeClaim::holds`).
pub fn holds(p: &Params, r: &Registry) -> Result<(), Why> {
    if r.claims.iter().any(|c| c.epoch != p.epoch || !c.holds(&p.pool)) {
        return Err("a claim that is not this epoch's or not signed");
    }
    Ok(())
}

/// Copies settle the same in every order: per (staker, key), the claim with the lower hash.
pub fn merge(a: &Registry, b: &Registry) -> Registry {
    let h = |c: &StakeClaim| *blake3::hash(&crate::wallet::bytes::stake_claim(c)).as_bytes();
    let mut all: Vec<StakeClaim> = a.claims.clone();
    for c in &b.claims {
        match all.iter_mut().find(|x| x.owner == c.owner && x.key == c.key) {
            Some(x) => {
                if h(c) < h(x) {
                    *x = c.clone();
                }
            }
            None => all.push(c.clone()),
        }
    }
    all.sort_by_key(|c| (c.owner, c.key));
    Registry { claims: all }
}

/// THE FREEZE: `held` takes `given` (merged) only while the beacon (`now`) is below [`Params::frozen_at`]; after, it
/// keeps what it holds (a claim arriving late is ignored, never a reason to refuse the copy: copies keep syncing).
/// Stakers claim an epoch AHEAD, so honest claims are everywhere long before; one sent at the deadline can leave copies
/// differing on that staker for that epoch — some lots then fail to gather signatures (liveness), never a fake counted.
pub fn take(p: &Params, held: &Registry, given: &Registry, now: u64) -> Registry {
    if now >= p.frozen_at() {
        return held.clone();
    }
    merge(held, given)
}
