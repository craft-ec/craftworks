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

// ---- A PLACE'S WITNESS SIGNATURES: their own container (audit 10-11) ----
//
// They lived in a generic BAG (anyone adds, ranked by work past its caps): ~16 junk items of 16 KiB with a little more
// work than a signature's evicted every genuine one — the entry never accepted, an equivocation's proof gone. No rule
// over items alone can stop that: any well-formed item is free to make (a signature under a fresh key costs nothing),
// so a bag of N kept by ANY order loses its genuine items to N made to outrank them. What bounds the items is who may
// sign: only a key in the claims epoch's REGISTRY (frozen before the epoch's lots exist, at most `CLAIMS_MAX` claims)
// or the BASELINE can ever sit on a committee. So this container takes a signature only if it VERIFIES over the
// place's message under its key, and that key is the baseline's or a registered claim's; each key keeps its
// [`PER_KEY`] lowest entries (two distinct ones prove an equivocation; an honest key signs one). The whole is then
// bounded by the registry itself — nothing taken is ever dropped for room, there is none to make.

/// The kind tag of a place's witness-signature container (params' first byte).
pub const WITNESSES: u8 = 8;
/// The entries one key keeps signatures for at one place: enough to prove an equivocation.
pub const PER_KEY: usize = 2;
/// The most signatures by keys other than the baseline: every registered key's [`PER_KEY`].
pub const SIGNED_MAX: usize = crate::claims::CLAIMS_MAX * PER_KEY;
const MAGIC: &[u8; 4] = b"CWWS";

/// A place's container PARAMS: the token code, the stake pool, the beacon (the claims registry's), the CLAIMS EPOCH its
/// lot is drawn from (whose registry says who may sign), and the place — (chain, seq).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    pub code_hash: [u8; 32],
    pub pool: [u8; 32],
    pub beacon: [u8; 32],
    pub epoch: u64,
    pub chain: [u8; 32],
    pub seq: u64,
}

impl Params {
    pub fn encode(&self) -> Vec<u8> {
        [&[WITNESSES][..], &self.code_hash, &self.pool, &self.beacon, &self.epoch.to_le_bytes(), &self.chain, &self.seq.to_le_bytes()].concat()
    }
    pub fn decode(b: &[u8]) -> Option<Params> {
        let mut r = crate::R(b);
        (r.u8()? == WITNESSES).then_some(())?;
        let p = Params { code_hash: r.arr()?, pool: r.arr()?, beacon: r.arr()?, epoch: r.u64()?, chain: r.arr()?, seq: r.u64()? };
        r.done().then_some(p)
    }
    pub fn id(&self) -> [u8; 32] {
        crate::id(&self.code_hash, &self.encode())
    }
    /// The claims registry whose keys may sign here.
    pub fn registry(&self) -> crate::claims::Params {
        crate::claims::Params { code_hash: self.code_hash, pool: self.pool, beacon: self.beacon, epoch: self.epoch }
    }
    /// THE PLACE'S SIGNATURES as a state of this container holds them (none if it does not read) — the one read every
    /// reader of a place's signatures goes through.
    pub fn sigs(&self, state: &[u8]) -> Vec<WitnessSig> {
        Signatures::decode(state).map_or(vec![], |s| s.items.iter().map(|x| WitnessSig { chain: self.chain, seq: self.seq, entry: x.entry, sig: x.sig }).collect())
    }
    /// A state holding these signatures (each by its key, at this place): what a witness PUTs or sends as an update
    /// (its one), or a container as made empty (none).
    pub fn state(&self, sigs: &[([u8; 32], WitnessSig)]) -> Vec<u8> {
        let mut items: Vec<Signed> = sigs.iter().map(|(key, s)| Signed { key: *key, entry: s.entry, sig: s.sig }).collect();
        items.sort();
        items.dedup_by(|b, a| (a.key, a.entry) == (b.key, b.entry));
        Signatures { items }.encode()
    }
    fn verifies(&self, s: &Signed) -> bool {
        sig_ok(&s.key, &WitnessSig::message(&self.chain, self.seq, &s.entry), &s.sig)
    }
}

/// One signature held: `key`'s over `entry` at the container's place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Signed {
    pub key: [u8; 32],
    pub entry: [u8; 32],
    pub sig: [u8; 64],
}

/// A container's state: its signatures, sorted by (key, entry), one per pair.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Signatures {
    pub items: Vec<Signed>,
}

impl Signatures {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = MAGIC.to_vec();
        v.extend_from_slice(&(self.items.len() as u32).to_le_bytes());
        for s in &self.items {
            v.extend_from_slice(&s.key);
            v.extend_from_slice(&s.entry);
            v.extend_from_slice(&s.sig);
        }
        v
    }
    pub fn decode(b: &[u8]) -> Option<Signatures> {
        let mut r = crate::R(b.strip_prefix(MAGIC)?);
        let n = r.u32()? as usize;
        (n <= SIGNED_MAX + PER_KEY).then_some(())?;
        let items = (0..n).map(|_| Some(Signed { key: r.arr()?, entry: r.arr()?, sig: r.arr()? })).collect::<Option<Vec<_>>>()?;
        (r.done() && items.windows(2).all(|w| (w[0].key, w[0].entry) < (w[1].key, w[1].entry))).then_some(Signatures { items })
    }
}

/// A container's state HOLDS: every signature verifies over the place's message under its key, a key keeps at most
/// [`PER_KEY`] entries, and keys other than the baseline at most [`SIGNED_MAX`] in all. (Whether a key is registered is
/// the update's check — it reads the registry; a copy taken without it can only hold what is bounded here.)
pub fn holds(p: &Params, s: &Signatures) -> Result<(), crate::Why> {
    if s.items.iter().any(|x| !p.verifies(x)) {
        return Err("a witness signature that does not verify");
    }
    if s.items.iter().filter(|x| x.key != BASELINE).count() > SIGNED_MAX || s.items.windows(PER_KEY + 1).any(|w| w.iter().all(|x| x.key == w[0].key)) {
        return Err("more witness signatures than a place keeps");
    }
    Ok(())
}

/// Does taking `given` into `held` need the claims registry: a NEW signature by a key other than the baseline?
pub fn needs_registry(held: &Signatures, given: &Signatures) -> bool {
    given.items.iter().any(|x| x.key != BASELINE && !held.items.iter().any(|h| (h.key, h.entry) == (x.key, x.entry)))
}

/// THE MERGE — the same in every order: the union of the signatures that VERIFY, by the baseline, a key of
/// `registry`'s claims, or already HELD (taken once under a registry; a node's copy of the registry that has not yet
/// seen a claim never drops what another node took), one per (key, entry) (the lower signature bytes), each key's
/// [`PER_KEY`] lowest entries.
pub fn merge(p: &Params, held: &Signatures, given: &Signatures, registry: Option<&crate::claims::Registry>) -> Signatures {
    let keys: std::collections::BTreeSet<[u8; 32]> = registry.map(|r| r.claims.iter().map(|c| c.key).collect()).unwrap_or_default();
    let admitted = |x: &Signed| x.key == BASELINE || held.items.contains(x) || keys.contains(&x.key);
    let mut all: Vec<Signed> = held.items.iter().chain(&given.items).filter(|x| admitted(x) && p.verifies(x)).copied().collect();
    all.sort();
    all.dedup_by(|b, a| (a.key, a.entry) == (b.key, b.entry));
    let mut out: Vec<Signed> = Vec::with_capacity(all.len());
    for x in all {
        if out.iter().rev().take_while(|y| y.key == x.key).count() < PER_KEY {
            out.push(x);
        }
    }
    Signatures { items: out }
}
