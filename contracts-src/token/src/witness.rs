//! WITNESSES (TOKEN §7a): a witness — an account whose chain holds STAKE — signs an entry of another chain it re-checked:
//! "chain C's entry at seq S is the one with hash H". Two such signatures by ONE key at one (chain, seq) naming
//! different entries are an EQUIVOCATION — the proof that burns the witness's stake (`Body::Slash`).

use crate::sig_ok;

/// "Chain `chain`'s entry at `seq` is `entry`", signed at beacon height `at` — by a witness's writer key. WHEN it was
/// signed is signed too: valid stake (`wallet::valid_stakers`) counts only signatures made before a claims epoch's
/// registry froze, so a signature cannot be dated earlier than it was made (and its container takes it only near the
/// beacon's height then: [`AT_SLACK`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WitnessSig {
    pub chain: [u8; 32],
    pub seq: u64,
    pub entry: [u8; 32],
    pub at: u64,
    pub sig: [u8; 64],
}

impl WitnessSig {
    pub fn message(chain: &[u8; 32], seq: u64, entry: &[u8; 32], at: u64) -> Vec<u8> {
        craftworks_identity::witness_message(chain, seq, entry, at)
    }
    pub fn signed_by(&self, key: &[u8; 32]) -> bool {
        sig_ok(key, &Self::message(&self.chain, self.seq, &self.entry, self.at), &self.sig)
    }
    pub(crate) fn put(&self, v: &mut Vec<u8>) {
        v.extend_from_slice(&self.chain);
        v.extend_from_slice(&self.seq.to_le_bytes());
        v.extend_from_slice(&self.entry);
        v.extend_from_slice(&self.at.to_le_bytes());
        v.extend_from_slice(&self.sig);
    }
    pub(crate) fn read(r: &mut crate::R) -> Option<WitnessSig> {
        Some(WitnessSig { chain: r.arr()?, seq: r.u64()?, entry: r.arr()?, at: r.u64()?, sig: r.arr()? })
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
    accepted_before(definition, chain, seq, entry, out, epoch, claims, sigs, max, stake_of, u64::MAX)
}

/// ACCEPTED BY HEIGHT `before` ([`accepted`], counting only signatures made at a beacon height below `before` — their
/// signed `at`). What VALID STAKE asks (`wallet::valid_stakers`, TOKEN §7a): a staker counts in claims epoch E only if
/// its last pre-E entry was accepted before E's registry froze (`claims::frozen_at`), so `V(E)` — and every committee
/// and threshold drawn over it — never changes once E's lots exist (measured 10-11: a stake entry accepted hours late
/// grew `V(E)` after the fact, raised the threshold, and unaccepted a statement a mint already cited — its chain wedged).
#[allow(clippy::too_many_arguments)]
pub fn accepted_before(
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
    before: u64,
) -> Result<usize, crate::Why> {
    let valid = crate::draw::valid(definition, epoch, claims);
    let (k, base, need) = seats(valid.len(), max);
    let members = crate::draw::committee_of(out, chain, seq, &valid, k);
    let mine = |s: &&WitnessSig| s.chain == *chain && s.seq == seq && s.entry == *entry && s.at < before;
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
// sign: only a key that WROTE A CLAIM for the claims epoch (`Body::Claim`, in its staker's own pool chain, before the
// epoch's freeze — audit C2b: no shared registry) or the BASELINE can ever sit on a committee. So this container takes
// a signature only if it VERIFIES over the place's message under its key, and that key is the baseline's or the writer
// key of such a claim in the pool chain of the staker the signature NAMES (`Signed::owner`: the update reads that chain
// in the same round, [`may_sign`]); each key keeps its [`PER_KEY`] lowest entries (two distinct ones prove an
// equivocation; an honest key signs one). A junk signer then costs a pool chain the token contract takes, with stake
// and a claim before the freeze — and past [`SIGNERS_MAX`] keys the lowest keys are kept ([`merge`]).
//
// WHEN A SIGNATURE WAS MADE (10-11: valid stake counts only acceptance before a claims epoch froze — `accepted_before`):
// each signature carries the beacon height `at` it was signed at, under its signature. The update never takes `at` on
// trust: it reads the BEACON in the same round and takes a new signature only if `at` is not above the beacon's height
// and is not dated back across an epoch's FREEZE (`pointers::frozen_at`) by more than [`AT_SLACK`] steps ([`fresh`]) — a
// signature cannot be dated into the past to join an epoch's valid stake after that epoch froze, the one thing `at`
// decides. (Not a plain "at most AT_SLACK below the beacon": copies sync by UPDATE too, so a lagging copy is sent
// signatures taken long ago elsewhere, and a delegate's round signs at the height it read when it started — those
// would be refused forever, and copies would differ on what is accepted. Within one freeze period an older `at` changes
// no verdict.) Per (key, entry) the EARLIEST `at` taken is kept; a replay dated later never replaces it.

/// The kind tag of a place's witness-signature container (params' first byte).
pub const WITNESSES: u8 = 8;
/// The entries one key keeps signatures for at one place: enough to prove an equivocation.
pub const PER_KEY: usize = 2;
/// The most keys other than the baseline's a place keeps signatures of.
pub const SIGNERS_MAX: usize = 4096;
/// The most signatures by keys other than the baseline: every kept key's [`PER_KEY`].
pub const SIGNED_MAX: usize = SIGNERS_MAX * PER_KEY;
/// How far back across a freeze a new signature's `at` may be dated, below the beacon's height as the taking node reads
/// it: 16 steps (~16 minutes at ~60 s a step). Room for a signer's beacon read lagging the node's and the update's trip
/// around a freeze — and small beside [`crate::pointers::LEAD`] (512 steps between an epoch's freeze and its first
/// lot), so every signature dated before a freeze is taken before that epoch's lots exist.
pub const AT_SLACK: u64 = 16;
const MAGIC: &[u8; 4] = b"CWW3";

/// May a signature dated `at` be taken while the beacon is at `now`? Not dated ahead of the beacon, and — unless within
/// [`AT_SLACK`] — no freeze (`pointers::frozen_at` of any epoch) between `at` and `now`: what `at` decides
/// (`accepted_before` at a freeze) is then the same as had it arrived when made.
pub fn fresh(at: u64, now: u64) -> bool {
    let period = |h: u64| h.saturating_add(crate::pointers::LEAD) / crate::supply::EPOCH; // freezes at or below h
    at <= now && (now - at <= AT_SLACK || period(at) == period(now))
}

/// A place's container PARAMS: the token code, the stake pool, the beacon, the key log code (with the pool and the
/// beacon: what names a staker's pool chain), the CLAIMS EPOCH its lot is drawn from (whose claims say who may sign),
/// and the place — (chain, seq).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    pub code_hash: [u8; 32],
    pub pool: [u8; 32],
    pub beacon: [u8; 32],
    pub idlog: [u8; 32],
    pub epoch: u64,
    pub chain: [u8; 32],
    pub seq: u64,
}

impl Params {
    pub fn encode(&self) -> Vec<u8> {
        [&[WITNESSES][..], &self.code_hash, &self.pool, &self.beacon, &self.idlog, &self.epoch.to_le_bytes(), &self.chain, &self.seq.to_le_bytes()].concat()
    }
    pub fn decode(b: &[u8]) -> Option<Params> {
        let mut r = crate::R(b);
        (r.u8()? == WITNESSES).then_some(())?;
        let p = Params { code_hash: r.arr()?, pool: r.arr()?, beacon: r.arr()?, idlog: r.arr()?, epoch: r.u64()?, chain: r.arr()?, seq: r.u64()? };
        r.done().then_some(p)
    }
    pub fn id(&self) -> [u8; 32] {
        crate::id(&self.code_hash, &self.encode())
    }
    /// Staker `owner`'s POOL CHAIN (where the claim that lets its key sign here is).
    pub fn pool_chain(&self, owner: &[u8; 32]) -> crate::ChainParams {
        crate::ChainParams { code_hash: self.code_hash, definition: self.pool, owner: *owner, beacon: self.beacon, idlog: self.idlog, item: [0; 32] }
    }
    /// THE PLACE'S SIGNATURES as a state of this container holds them (none if it does not read) — the one read every
    /// reader of a place's signatures goes through.
    pub fn sigs(&self, state: &[u8]) -> Vec<WitnessSig> {
        Signatures::decode(state).map_or(vec![], |s| s.items.iter().map(|x| WitnessSig { chain: self.chain, seq: self.seq, entry: x.entry, at: x.at, sig: x.sig }).collect())
    }
    /// A state holding these signatures (each by its key, for the staker it names — the baseline names none: [0; 32] —
    /// at this place): what a witness PUTs or sends as an update (its one), or a container as made empty (none).
    pub fn state(&self, sigs: &[([u8; 32], [u8; 32], WitnessSig)]) -> Vec<u8> {
        let mut items: Vec<Signed> = sigs.iter().map(|(key, owner, s)| Signed { key: *key, owner: *owner, entry: s.entry, at: s.at, sig: s.sig }).collect();
        items.sort();
        items.dedup_by(|b, a| (a.key, a.entry) == (b.key, b.entry));
        Signatures { items }.encode()
    }
    fn verifies(&self, s: &Signed) -> bool {
        sig_ok(&s.key, &WitnessSig::message(&self.chain, self.seq, &s.entry, s.at), &s.sig)
    }
}

/// One signature held: `key`'s over `entry` at the container's place, signed at beacon height `at` — by the staker
/// `owner` names (whose claim lets the key sign: [`may_sign`]). (Ordered by key, then entry, then the rest: the first of
/// a (key, entry) is its earliest.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Signed {
    pub key: [u8; 32],
    pub entry: [u8; 32],
    pub at: u64,
    pub owner: [u8; 32],
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
            v.extend_from_slice(&s.owner);
            v.extend_from_slice(&s.entry);
            v.extend_from_slice(&s.at.to_le_bytes());
            v.extend_from_slice(&s.sig);
        }
        v
    }
    pub fn decode(b: &[u8]) -> Option<Signatures> {
        let mut r = crate::R(b.strip_prefix(MAGIC)?);
        let n = r.u32()? as usize;
        (n <= SIGNED_MAX + PER_KEY).then_some(())?;
        let items = (0..n).map(|_| Some(Signed { key: r.arr()?, owner: r.arr()?, entry: r.arr()?, at: r.u64()?, sig: r.arr()? })).collect::<Option<Vec<_>>>()?;
        (r.done() && items.windows(2).all(|w| (w[0].key, w[0].entry) < (w[1].key, w[1].entry))).then_some(Signatures { items })
    }
}

/// A container's state HOLDS: every signature verifies over the place's message (its `at` included) under its key, a
/// key keeps at most [`PER_KEY`] entries, and keys other than the baseline at most [`SIGNED_MAX`] in all. (Whether a key
/// may sign — its claim — and whether `at` was near the beacon when taken, are the update's checks — it reads the
/// signers' pool chains and the beacon; a copy taken without them can only hold what is bounded here.)
pub fn holds(p: &Params, s: &Signatures) -> Result<(), crate::Why> {
    if s.items.iter().any(|x| !p.verifies(x)) {
        return Err("a witness signature that does not verify");
    }
    if s.items.iter().filter(|x| x.key != BASELINE).count() > SIGNED_MAX || s.items.windows(PER_KEY + 1).any(|w| w.iter().all(|x| x.key == w[0].key)) {
        return Err("more witness signatures than a place keeps");
    }
    Ok(())
}

/// Is `x` NEW to `held` — not already there with an `at` as early (a replay dated the same or later changes nothing)?
fn new_to(held: &Signatures, x: &Signed) -> bool {
    !held.items.iter().any(|h| (h.key, h.entry) == (x.key, x.entry) && h.at <= x.at)
}

/// What taking `given` into `held` must READ in the same round: (the BEACON — any new signature: its `at` is checked
/// against the beacon's height; the POOL CHAINS of the stakers new signatures by keys other than the baseline's name —
/// at most `max`, the lowest owners: the rest wait for a later update).
pub fn needs(held: &Signatures, given: &Signatures, max: usize) -> (bool, Vec<[u8; 32]>) {
    let new: Vec<&Signed> = given.items.iter().filter(|x| new_to(held, x)).collect();
    let mut owners: Vec<[u8; 32]> = new.iter().filter(|x| x.key != BASELINE).map(|x| x.owner).collect();
    owners.sort();
    owners.dedup();
    owners.truncate(max);
    (!new.is_empty(), owners)
}

/// MAY `key` SIGN at a place of claims epoch `epoch`, as its staker's pool chain `chain` reads (the related state the
/// update read)? Its writer wrote a CLAIM for `epoch` with that key, before the epoch's freeze (`Body::Claim`: the
/// chain's own rules made it — signed by its writer, stamped before the freeze). Whether the claim is VALID stake is
/// the lot's question (`wallet::valid_stakers`); this only bounds who can fill a place's container.
pub fn may_sign(chain: &crate::State, epoch: u64, key: &[u8; 32]) -> bool {
    chain.fork.is_none()
        && chain.entries.iter().enumerate().any(|(i, e)| {
            matches!(e.body, crate::Body::Claim { from_epoch, .. } if from_epoch == epoch)
                && e.stamp < crate::pointers::frozen_at(epoch)
                && matches!(crate::writer_at(chain, i), Some(crate::Signer::Device(node, _)) if node == *key)
        })
}

/// THE MERGE — the same in every order for the same reads: the union of the signatures that VERIFY and are either HELD
/// (taken once under a beacon and the signers' chains; a node whose copies have not caught up never drops what another
/// took) or NEW and taken now — by the baseline, or by a key [`may_sign`] here as its named staker's pool chain reads
/// (`chain`: by owner, as read in this round — not read: not taken now), dated `at` near the beacon's height `now`
/// ([`fresh`]; no beacon read: nothing new is taken) — one per (key, entry) (the EARLIEST `at`, then the rest), each
/// key's [`PER_KEY`] lowest entries; past [`SIGNERS_MAX`] keys other than the baseline's, the lowest keys.
pub fn merge(p: &Params, held: &Signatures, given: &Signatures, chain: &dyn Fn(&[u8; 32]) -> Option<crate::State>, now: Option<u64>) -> Signatures {
    let known = |x: &Signed| x.key == BASELINE || chain(&x.owner).is_some_and(|c| may_sign(&c, p.epoch, &x.key));
    let taken = |x: &Signed| held.items.contains(x) || (now.is_some_and(|n| fresh(x.at, n)) && known(x));
    let mut all: Vec<Signed> = held.items.iter().chain(&given.items).filter(|x| taken(x) && p.verifies(x)).copied().collect();
    all.sort();
    all.dedup_by(|b, a| (a.key, a.entry) == (b.key, b.entry));
    let mut out: Vec<Signed> = Vec::with_capacity(all.len());
    let mut signers = 0usize;
    for x in all {
        let first = out.last().is_none_or(|y| y.key != x.key);
        if first && x.key != BASELINE {
            if signers == SIGNERS_MAX {
                continue;
            }
            signers += 1;
        }
        if out.iter().rev().take_while(|y| y.key == x.key).count() < PER_KEY {
            out.push(x);
        }
    }
    Signatures { items: out }
}
