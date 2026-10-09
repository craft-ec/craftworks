//! Encodings, and the checks that make each one canonical.
//!
//! Every integer is little-endian. Every structure here has exactly one valid
//! encoding: a host that accepted two spellings of one logical state would let
//! the same decision hash two ways, and the merge order (which breaks ties on
//! hashes) would stop being a function of the decision.

use ed25519_dalek::{Signature, VerifyingKey};

pub const MAGIC: &[u8; 4] = b"RG01";
/// Domain separator for the signed message. Distinct from [`MAGIC`] so a state
/// can never be read as a signed message or the reverse.
pub const SIG_DOMAIN: &[u8; 8] = b"RG01-sig";
pub const MAX_LABEL: usize = 64;
/// A terminal record's value is the successor's IDENTITY — `BLAKE3("RG01-succ"
/// ‖ params)`, see [`crate::succ`] — never a Freenet instance id, which names
/// CODE and so would die on upgrade. The length is what a host checks.
pub const TERMINAL_VALUE_LEN: usize = 32;
pub const MAX_VALUE: usize = 4096;
pub const MAX_N: usize = 16;
pub const KEY_LEN: usize = 32;
pub const SIG_LEN: usize = 64;
pub const HASH_LEN: usize = 32;
/// `terminal(1) ‖ seq(8) ‖ BLAKE3(value)(32)`, the part of a record that is
/// signed and that evidence carries in place of the value.
const SIGNED_FIELDS: usize = 1 + 8 + HASH_LEN;

/// Field modulus p = 2²⁵⁵ − 19, little-endian. A public key's y coordinate must
/// be below it, or the same point has more than one encoding.
const P_LE: [u8; 32] = [
    0xed, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f,
];

/// Is this the canonical encoding of a point, i.e. is y < p? `VerifyingKey`
/// accepts a handful of y ≥ p encodings, which decompress to points that also
/// have a canonical spelling — two names for one signer.
fn key_is_canonical(b: &[u8; KEY_LEN]) -> bool {
    let mut y = *b;
    y[31] &= 0x7f; // the top bit is the x sign, not part of y
    for i in (0..KEY_LEN).rev() {
        if y[i] != P_LE[i] {
            return y[i] < P_LE[i];
        }
    }
    false // y == p
}

/// A key the contract will accept: canonically encoded, on the curve, and not
/// small-order (a small-order key verifies some signatures under every message).
fn usable_key(b: &[u8]) -> Option<VerifyingKey> {
    let b: [u8; KEY_LEN] = b.try_into().ok()?;
    if !key_is_canonical(&b) {
        return None;
    }
    let k = VerifyingKey::from_bytes(&b).ok()?;
    (!k.is_weak()).then_some(k)
}

/// Who may write this register. Fixed for the register's whole life: changing it
/// means moving the head (ARCHITECTURE.md §3), which is what terminal records
/// are for.
#[derive(Clone, Debug)]
pub enum Authority {
    /// mode 0 — one key.
    One(VerifyingKey),
    /// mode 1 — `k` of `n`, keys strictly sorted.
    Quorum { k: usize, keys: Vec<VerifyingKey> },
}

impl Authority {
    /// How many signatures a valid record carries. Exactly this many: fewer is
    /// under threshold, more is a second encoding of the same decision.
    pub fn sigs_required(&self) -> usize {
        match self {
            Authority::One(_) => 1,
            Authority::Quorum { k, .. } => *k,
        }
    }
    /// Bytes of the signer bitmap in an encoding: none in mode 0, where there is
    /// nothing to choose.
    fn bitmap_len(&self) -> usize {
        match self {
            Authority::One(_) => 0,
            Authority::Quorum { .. } => 2,
        }
    }
}

/// `"RG01" ‖ mode(1) ‖ authority ‖ label`. The authority lives in the params, so
/// the contract key commits to it and a host needs nothing else to check a
/// signature.
#[derive(Clone, Debug)]
pub struct Params {
    pub authority: Authority,
    pub label: Vec<u8>,
    /// BLAKE3 of the parameter bytes, which every signature is bound to.
    pub hash: [u8; HASH_LEN],
}

impl Params {
    pub fn parse(b: &[u8]) -> Option<Params> {
        let rest = b.strip_prefix(MAGIC)?;
        let (mode, rest) = rest.split_first()?;
        let (authority, rest) = match mode {
            0 => {
                let (k, rest) = rest.split_at_checked(KEY_LEN)?;
                (Authority::One(usable_key(k)?), rest)
            }
            1 => {
                let (&k, rest) = rest.split_first()?;
                let (&n, rest) = rest.split_first()?;
                let (k, n) = (k as usize, n as usize);
                if k == 0 || n == 0 || k > n || n > MAX_N {
                    return None;
                }
                let (raw, rest) = rest.split_at_checked(n * KEY_LEN)?;
                let mut keys = Vec::with_capacity(n);
                for (i, chunk) in raw.chunks_exact(KEY_LEN).enumerate() {
                    // Strictly sorted: an order to index the bitmap by, and no
                    // key twice (which would let one signer fill two slots).
                    if i > 0 && chunk <= &raw[(i - 1) * KEY_LEN..i * KEY_LEN] {
                        return None;
                    }
                    keys.push(usable_key(chunk)?);
                }
                (Authority::Quorum { k, keys }, rest)
            }
            _ => return None,
        };
        if rest.len() > MAX_LABEL {
            return None;
        }
        Some(Params {
            authority,
            label: rest.to_vec(),
            hash: *blake3::hash(b).as_bytes(),
        })
    }

    /// The bytes a signer signs. Bound to the params — not to the contract code —
    /// so that after a code upgrade anyone can re-publish the latest record under
    /// the new code and have it verify. Max-merge makes the old records harmless.
    pub fn signed_message(&self, terminal: bool, seq: u64, value_hash: &[u8; HASH_LEN]) -> Vec<u8> {
        let mut m = Vec::with_capacity(SIG_DOMAIN.len() + HASH_LEN + SIGNED_FIELDS);
        m.extend_from_slice(SIG_DOMAIN);
        m.extend_from_slice(&self.hash);
        m.push(u8::from(terminal));
        m.extend_from_slice(&seq.to_le_bytes());
        m.extend_from_slice(value_hash);
        m
    }
}

/// What a record decides, with no trace of who witnessed it: the lattice is
/// over these, and signatures are evidence that a decision was made, never part
/// of its identity.
///
/// This matters for more than tidiness. An Ed25519 signature is not unique to
/// its signer — whoever holds a key can mint unlimited distinct valid
/// signatures of one message by choosing another nonce. Anything that ordered
/// states by their encodings would therefore let one rogue member of a keyset
/// produce an endless stream of strictly "better" states carrying the same
/// decision, each one replacing the last on every host and waking every
/// subscriber, with no equivocation and nothing attributable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    pub terminal: bool,
    pub seq: u64,
    pub value_hash: [u8; HASH_LEN],
}

impl Decision {
    /// Strongest first: a terminal ends the register, then the higher `seq`,
    /// then the LOWER value hash. Total over distinct decisions, and it never
    /// consults a signature.
    pub fn rank(&self) -> (bool, u64, core::cmp::Reverse<[u8; HASH_LEN]>) {
        (self.terminal, self.seq, core::cmp::Reverse(self.value_hash))
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1 + 8 + HASH_LEN);
        out.push(u8::from(self.terminal));
        out.extend_from_slice(&self.seq.to_le_bytes());
        out.extend_from_slice(&self.value_hash);
        out
    }
}

/// The signed part of a record: what a signature covers, and exactly what
/// evidence of equivocation needs. It carries the value's hash, never the value,
/// so equivocation can be proved to someone who holds neither version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signed {
    pub terminal: bool,
    pub seq: u64,
    pub value_hash: [u8; HASH_LEN],
    /// Which of the `n` keys signed, bit `i` for key `i`. Always 0 in mode 0.
    pub bitmap: u16,
    /// Exactly `k` signatures, ordered by the key index they belong to.
    pub sigs: Vec<[u8; SIG_LEN]>,
}

impl Signed {
    pub fn decision(&self) -> Decision {
        Decision {
            terminal: self.terminal,
            seq: self.seq,
            value_hash: self.value_hash,
        }
    }

    fn encoded_len(a: &Authority) -> usize {
        SIGNED_FIELDS + a.bitmap_len() + a.sigs_required() * SIG_LEN
    }

    fn write_head(&self, out: &mut Vec<u8>) {
        out.push(u8::from(self.terminal));
        out.extend_from_slice(&self.seq.to_le_bytes());
    }

    fn write_sigs(&self, out: &mut Vec<u8>, a: &Authority) {
        if a.bitmap_len() != 0 {
            out.extend_from_slice(&self.bitmap.to_le_bytes());
        }
        for s in &self.sigs {
            out.extend_from_slice(s);
        }
    }

    pub fn encode(&self, a: &Authority) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::encoded_len(a));
        self.write_head(&mut out);
        out.extend_from_slice(&self.value_hash);
        self.write_sigs(&mut out, a);
        out
    }

    /// Read the signature block — bitmap and signatures — and refuse every
    /// non-canonical shape: a count that disagrees with the bitmap, a bit for a
    /// key that does not exist, and in mode 1 a popcount that is not exactly k.
    fn parse_sigs(b: &[u8], a: &Authority) -> Option<(u16, Vec<[u8; SIG_LEN]>)> {
        let (bitmap, rest) = match a {
            Authority::One(_) => (0u16, b),
            Authority::Quorum { k, keys } => {
                let (raw, rest) = b.split_at_checked(2)?;
                let bitmap = u16::from_le_bytes([raw[0], raw[1]]);
                // Exactly k signers, and no bit outside the keyset. The mask is
                // built by shifting DOWN: `1 << n` overflows a u16 at n = 16,
                // which is a legal keyset size.
                let allowed = u16::MAX >> (u16::BITS as usize - keys.len());
                if bitmap.count_ones() as usize != *k || bitmap & !allowed != 0 {
                    return None;
                }
                (bitmap, rest)
            }
        };
        let want = a.sigs_required();
        if rest.len() != want * SIG_LEN {
            return None;
        }
        let sigs = rest
            .chunks_exact(SIG_LEN)
            .map(|c| c.try_into().expect("chunk is SIG_LEN"))
            .collect();
        Some((bitmap, sigs))
    }

    fn parse(b: &[u8], a: &Authority) -> Option<Signed> {
        if b.len() != Self::encoded_len(a) {
            return None;
        }
        let (&terminal, rest) = b.split_first()?;
        let (seq, rest) = rest.split_at_checked(8)?;
        let (value_hash, rest) = rest.split_at_checked(HASH_LEN)?;
        let (bitmap, sigs) = Self::parse_sigs(rest, a)?;
        Some(Signed {
            terminal: canonical_bool(terminal)?,
            seq: u64::from_le_bytes(seq.try_into().ok()?),
            value_hash: value_hash.try_into().ok()?,
            bitmap,
            sigs,
        })
    }

    /// Do the required signatures verify, under the flagged keys, in order?
    ///
    /// `verify_strict` is deliberate: it rejects a signature whose scalar S is
    /// not fully reduced (so `S` and `S + L` are not both accepted for one
    /// message) and rejects small-order `R` and `A`.
    pub fn verify(&self, p: &Params) -> bool {
        let msg = p.signed_message(self.terminal, self.seq, &self.value_hash);
        let keys: Vec<&VerifyingKey> = match &p.authority {
            Authority::One(k) => vec![k],
            Authority::Quorum { keys, .. } => (0..keys.len())
                .filter(|i| self.bitmap & (1 << i) != 0)
                .map(|i| &keys[i])
                .collect(),
        };
        if keys.len() != self.sigs.len() {
            return false;
        }
        // Each signature is checked against its own signer, so a signature out of
        // key order fails, and one signer cannot fill two slots.
        keys.iter().zip(&self.sigs).all(|(k, s)| {
            count_verification();
            k.verify_strict(&msg, &Signature::from_bytes(s)).is_ok()
        })
    }
}

/// The Tail contract reads and writes the same signed part (ARCHITECTURE.md §1, Tail): one encoding, one
/// verifier. LIBRARY only, so register.wasm does not move.
#[cfg(feature = "library")]
impl Signed {
    /// Bytes of an encoded `Signed` under this authority.
    pub fn len_for(a: &Authority) -> usize {
        Self::encoded_len(a)
    }
    /// Parse exactly one canonical encoding; no trailing bytes.
    pub fn parse_for(b: &[u8], a: &Authority) -> Option<Signed> {
        Self::parse(b, a)
    }
}

/// Counts individual signature checks, so a test can assert that a candidate
/// which cannot change the state costs a parse and no verification. Behind the
/// `testing` feature, so the contract's wasm has neither the counter nor the
/// branch.
///
/// Thread-local, not a global: the test runner runs tests in parallel, and a
/// shared counter would report other tests' work as this one's — which reads as
/// a real measurement and is not.
#[cfg(any(test, feature = "testing"))]
pub mod verifications {
    use core::cell::Cell;
    thread_local! {
        pub(super) static COUNT: Cell<usize> = const { Cell::new(0) };
    }
    /// Signature checks on this thread since the last [`reset`].
    pub fn count() -> usize {
        COUNT.with(|c| c.get())
    }
    pub fn reset() {
        COUNT.with(|c| c.set(0));
    }
}

fn count_verification() {
    #[cfg(any(test, feature = "testing"))]
    verifications::COUNT.with(|c| c.set(c.get() + 1));
}

fn canonical_bool(b: u8) -> Option<bool> {
    match b {
        0 => Some(false),
        1 => Some(true),
        _ => None, // any other byte is a second spelling of true
    }
}

/// A record: the signed decision together with the value it decides on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub signed: Signed,
    pub value: Vec<u8>,
}

impl Record {
    pub fn new(signed: Signed, value: Vec<u8>) -> Option<Record> {
        // The hash in the signed part is what a signature commits to, so a record
        // whose value does not match it is not a record of that decision.
        let shape_ok = if signed.terminal {
            value.len() == TERMINAL_VALUE_LEN
        } else {
            value.len() <= MAX_VALUE
        };
        (shape_ok && *blake3::hash(&value).as_bytes() == signed.value_hash)
            .then_some(Record { signed, value })
    }

    pub fn decision(&self) -> Decision {
        self.signed.decision()
    }

    pub fn encode(&self, a: &Authority) -> Vec<u8> {
        let mut out = Vec::new();
        self.signed.write_head(&mut out);
        // `as u16` would silently wrap a value longer than 64 KiB and write a
        // length that disagrees with the bytes after it — a short read of a long
        // value. Saturating instead keeps the encoding unparseable (the length
        // is over MAX_VALUE either way), so an over-cap record is refused rather
        // than quietly truncated. Records from `parse` and `new` are already
        // within the cap; this makes that impossible to bypass, not just
        // unlikely.
        let vlen = u16::try_from(self.value.len()).unwrap_or(u16::MAX);
        out.extend_from_slice(&vlen.to_le_bytes());
        out.extend_from_slice(&self.value);
        self.signed.write_sigs(&mut out, a);
        out
    }

    /// Returns the record and the bytes left over, since a state may hold
    /// evidence after it.
    ///
    /// TODO(next register epoch): parse the authority-free prefix through ONE
    /// function shared with [`record_head`], which repeats it today only so
    /// that this contract's code (and so register.wasm's hash) stays put;
    /// `record_head_agrees_with_read` holds the two together until then.
    fn parse(b: &[u8], a: &Authority) -> Option<(Record, usize)> {
        let (&terminal, rest) = b.split_first()?;
        let (seq, rest) = rest.split_at_checked(8)?;
        let (vlen, rest) = rest.split_at_checked(2)?;
        let vlen = u16::from_le_bytes([vlen[0], vlen[1]]) as usize;
        if vlen > MAX_VALUE {
            return None;
        }
        let (value, rest) = rest.split_at_checked(vlen)?;
        let terminal = canonical_bool(terminal)?;
        // A terminal record's value IS the successor's IDENTITY (see
        // TERMINAL_VALUE_LEN), so a host refuses one that cannot name a register.
        if terminal && vlen != TERMINAL_VALUE_LEN {
            return None;
        }
        let sig_len = a.bitmap_len() + a.sigs_required() * SIG_LEN;
        let (sig_block, _) = rest.split_at_checked(sig_len)?;
        let (bitmap, sigs) = Signed::parse_sigs(sig_block, a)?;
        let signed = Signed {
            terminal,
            seq: u64::from_le_bytes(seq.try_into().ok()?),
            value_hash: *blake3::hash(value).as_bytes(),
            bitmap,
            sigs,
        };
        let used = 1 + 8 + 2 + vlen + sig_len;
        Some((
            Record {
                signed,
                value: value.to_vec(),
            },
            used,
        ))
    }

    pub fn verify(&self, p: &Params) -> bool {
        self.signed.verify(p)
    }
}

/// Two conflicting signed decisions. Permanent, and provable without either
/// value: the pair is ordered so one fork has one encoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evidence {
    pub a: Signed,
    pub b: Signed,
}

impl Evidence {
    /// The pair, lower DECISION first, if these two really do equivocate. The
    /// side order must not depend on the witnesses: two people proving one fork
    /// with different signatures have to produce the same pair.
    pub fn new(x: &Signed, y: &Signed, _auth: &Authority) -> Option<Evidence> {
        if !conflicts(x, y) {
            return None;
        }
        let (a, b) = if x.decision().rank() <= y.decision().rank() {
            (x.clone(), y.clone())
        } else {
            (y.clone(), x.clone())
        };
        Some(Evidence { a, b })
    }

    /// What this proves, with the witnesses projected out: the unordered pair of
    /// conflicting decisions.
    pub fn decisions(&self) -> (Decision, Decision) {
        (self.a.decision(), self.b.decision())
    }

    /// The total order used to keep ONE proof per register. Terminal pairs first
    /// — a register that was ended twice is the worse fact — then the sequence
    /// numbers NUMERICALLY (not by their little-endian bytes, under which seq
    /// 256 would sort before seq 1), then the lower value hash, then the higher.
    /// Built only from decisions, so which proof is kept cannot be steered by
    /// re-signing one.
    pub fn key(&self) -> (bool, u64, u64, [u8; HASH_LEN], [u8; HASH_LEN]) {
        let (x, y) = self.decisions();
        let terminal_pair = x.terminal && y.terminal;
        (
            // `false` sorts first, so a terminal pair must map to `false`.
            !terminal_pair,
            x.seq.min(y.seq),
            x.seq.max(y.seq),
            x.value_hash.min(y.value_hash),
            x.value_hash.max(y.value_hash),
        )
    }

    pub fn encode(&self, a: &Authority) -> Vec<u8> {
        let mut out = self.a.encode(a);
        out.extend_from_slice(&self.b.encode(a));
        out
    }

    fn parse(b: &[u8], a: &Authority) -> Option<Evidence> {
        let side = Signed::encoded_len(a);
        let (x, y) = b.split_at_checked(side)?;
        let e = Evidence {
            a: Signed::parse(x, a)?,
            b: Signed::parse(y, a)?,
        };
        // Canonical order by decision, and it must actually be a conflict.
        (e.a.decision().rank() <= e.b.decision().rank() && conflicts(&e.a, &e.b)).then_some(e)
    }

    pub fn verify(&self, p: &Params) -> bool {
        self.a.verify(p) && self.b.verify(p)
    }
}

/// Do these two signed decisions equivocate? Equal `seq` with different values,
/// or two terminals with different values whatever their `seq` — a writer only
/// gets to end the register once.
///
/// Defined on the VALUE (via its hash), never on the encoding: the same
/// `(seq, value)` signed by two different subsets of a keyset is one decision
/// spelled twice, not a fork.
pub fn conflicts(x: &Signed, y: &Signed) -> bool {
    x.value_hash != y.value_hash && (x.seq == y.seq || (x.terminal && y.terminal))
}

/// `"RG01" ‖ flags(1) ‖ record? ‖ evidence?`.
///
/// A state's record as `(terminal, seq, value)`, WITHOUT the params: structure
/// only -- the magic, the flags, and the record's authority-free prefix (the
/// value comes before the signatures, whose count is the authority's).
///
/// For a reader that holds a state but not its params: a page reading a head it
/// knows only by contract id, or the signer's own freshly signed record. **Safe
/// only for a state a host already VALIDATED**: a node runs `validate_state` (the
/// full `read`, every signature) before it stores or serves a state, so what a
/// client is handed has been checked there -- and is never checked again here.
/// Anything that must TRUST a state it did not get from a node calls [`crate::read`]
/// with the params. `None` for no record, or a structure that is not one.
///
/// **LIBRARY ONLY** (`feature = "library"`, never the contract's): adding a
/// function to the contract build moves register.wasm's hash, and that re-keys
/// every Register (measured, #117). So this is compiled out of the wasm, and its
/// prefix parse is kept beside `Record::parse`'s rather than shared with it,
/// which would change the contract's code; `record_head_agrees_with_read` pins
/// the two to each other, and closure-gate.sh pins the wasm's hash.
///
/// TODO(next register epoch): when the contract's code changes anyway, `Record::parse`
/// and this share one prefix parse, and the repeat here goes.
#[cfg(any(test, feature = "library"))]
pub fn record_head(state: &[u8]) -> Option<(bool, u64, &[u8])> {
    let rest = state.strip_prefix(MAGIC)?;
    let (&flags, rest) = rest.split_first()?;
    if flags & FLAG_RECORD == 0 || flags & !(FLAG_RECORD | FLAG_EVIDENCE) != 0 {
        return None;
    }
    let (&terminal, rest) = rest.split_first()?;
    let terminal = canonical_bool(terminal)?;
    let (seq, rest) = rest.split_at_checked(8)?;
    let (vlen, rest) = rest.split_at_checked(2)?;
    let vlen = u16::from_le_bytes([vlen[0], vlen[1]]) as usize;
    if vlen > MAX_VALUE || (terminal && vlen != TERMINAL_VALUE_LEN) {
        return None;
    }
    Some((
        terminal,
        u64::from_le_bytes(seq.try_into().ok()?),
        rest.get(..vlen)?,
    ))
}

/// The empty register is the empty byte string, never `MAGIC ‖ 0` — otherwise
/// "nothing here" would have two encodings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegState {
    pub record: Option<Record>,
    pub evidence: Option<Evidence>,
}

const FLAG_RECORD: u8 = 0b01;
const FLAG_EVIDENCE: u8 = 0b10;

impl RegState {
    pub fn is_empty(&self) -> bool {
        self.record.is_none() && self.evidence.is_none()
    }

    /// A fork is proved exactly when evidence is held.
    pub fn forked(&self) -> bool {
        self.evidence.is_some()
    }

    pub fn encode(&self, a: &Authority) -> Vec<u8> {
        if self.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::from(*MAGIC);
        out.push(
            u8::from(self.record.is_some()) * FLAG_RECORD
                + u8::from(self.evidence.is_some()) * FLAG_EVIDENCE,
        );
        if let Some(r) = &self.record {
            out.extend_from_slice(&r.encode(a));
        }
        if let Some(e) = &self.evidence {
            out.extend_from_slice(&e.encode(a));
        }
        out
    }

    /// Parse and verify. `validate_state` uses this: a whole state offered to a
    /// host is checked in full.
    pub fn parse(b: &[u8], p: &Params) -> Option<RegState> {
        let s = Self::parse_unverified(b, p)?;
        s.verify(p).then_some(s)
    }

    /// Every signature in the state holds.
    pub fn verify(&self, p: &Params) -> bool {
        self.record.as_ref().is_none_or(|r| r.verify(p))
            && self.evidence.as_ref().is_none_or(|e| e.verify(p))
    }

    /// Structure only: magic, flags, canonical encodings, lengths, and that any
    /// evidence really is a pair of conflicting decisions. Signatures are NOT
    /// checked.
    ///
    /// `update_state` parses first and verifies only what could change the
    /// state, so a replayed record or another witness of the decision already
    /// held costs a parse instead of `k` signature checks. That is a real
    /// difference: `k` can be 16, and anyone may send anything.
    pub fn parse_unverified(b: &[u8], p: &Params) -> Option<RegState> {
        if b.is_empty() {
            return Some(RegState::default());
        }
        let rest = b.strip_prefix(MAGIC)?;
        let (&flags, rest) = rest.split_first()?;
        // 0 would be a second spelling of the empty state; unknown bits are a
        // format nobody here can check.
        if flags == 0 || flags & !(FLAG_RECORD | FLAG_EVIDENCE) != 0 {
            return None;
        }
        let mut state = RegState::default();
        let mut rest = rest;
        if flags & FLAG_RECORD != 0 {
            let (r, used) = Record::parse(rest, &p.authority)?;
            state.record = Some(r);
            rest = &rest[used..];
        }
        if flags & FLAG_EVIDENCE != 0 {
            // Both sides are fixed-size, so this consumes exactly what is left
            // or fails — which is also what rejects trailing bytes.
            let e = Evidence::parse(rest, &p.authority)?;
            state.evidence = Some(e);
        } else if !rest.is_empty() {
            return None;
        }
        Some(state)
    }
}

#[cfg(test)]
mod record_head_tests {
    use super::record_head;
    use crate::testing::{keyset, World};
    use crate::wire::Authority;

    /// `record_head` (no params, structure only) and the crate's full `read` (params, every signature) agree on
    /// `(terminal, seq, value)` for every valid state: mode 0 and a 2-of-3 quorum, with and without fork evidence.
    #[test]
    fn record_head_agrees_with_read() {
        let (mut compared, mut with_evidence, mut quorum, mut single) = (0, 0, 0, 0);
        for w in [keyset(1, 1, true), keyset(2, 3, false)] {
            let mut states = w.sample_states();
            // And explicitly: a record held beside PROOF OF A FORK (two signed decisions at one seq), the state a
            // hand-written parser once refused (F56).
            let a = w.record(false, 7, b"one head");
            if let Some(b) = w.alternate(false, 7, b"another head") {
                let mut s = w.state(a.clone());
                s.evidence = Some(w.evidence_of(&a, &b));
                states.push(s);
            }
            for s in states {
                let bytes = w.encode(&s);
                let (_, full) = crate::read(&w.params_bytes, &bytes)
                    .expect("a sample state the crate itself reads");
                let want = full
                    .record
                    .as_ref()
                    .map(|r| (r.signed.terminal, r.signed.seq, r.value.clone()));
                let got = record_head(&bytes).map(|(t, q, v)| (t, q, v.to_vec()));
                assert_eq!(
                    got,
                    want,
                    "record_head and read disagree on a state (evidence: {})",
                    s.evidence.is_some()
                );
                compared += 1;
                with_evidence += usize::from(s.evidence.is_some() && s.record.is_some());
                quorum += usize::from(matches!(w.auth, Authority::Quorum { .. }));
                single += usize::from(matches!(w.auth, Authority::One(_)));
            }
        }
        println!("record_head vs read: {compared} states ({single} mode 0, {quorum} quorum), {with_evidence} with a record and fork evidence");
        assert!(
            single >= 3 && quorum >= 3 && with_evidence >= 2,
            "not a real comparison: {single}/{quorum}/{with_evidence}"
        );
    }

    /// THE CONTROL: a record_head that read the wrong field would fail above; and it refuses what is not a record.
    #[test]
    fn record_head_refuses_what_is_not_a_record() {
        let w: World = keyset(1, 1, true);
        let bytes = w.encode(&w.state(w.record(false, 3, b"v")));
        assert!(record_head(&bytes).is_some());
        assert_eq!(record_head(&[]), None, "the empty register has no record");
        assert_eq!(
            record_head(&bytes[..bytes.len().min(10)]),
            None,
            "a truncated state"
        );
        let mut bad = bytes.clone();
        bad[0] = b'X';
        assert_eq!(record_head(&bad), None, "a wrong magic");
        let mut flags = bytes.clone();
        flags[4] = 0b10; // evidence only, no record
        assert_eq!(record_head(&flags), None, "a state with no record flag");
    }
}
