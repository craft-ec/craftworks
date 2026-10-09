//! THE HEAD VALUE: what a Register record says the head is. The ONE reader and writer of the VALUE's layout; the
//! RECORD around it (the Register's own framing) is the Register crate's alone, read by [`record_of`] (sdk#364).
//!
//! ```text
//! value  = root [32] ‖ ledger
//! ledger = (empty) | version u8 ‖ field*       field = tag u8 ‖ len u16 LE ‖ bytes, tags strictly ascending
//! ```
//!
//! **Root first**, deliberately: a reader that takes the value's first 32 bytes as the root stays right whatever
//! follows. A leading version byte would have made such a reader take `version ‖ 31 root bytes` for a root, which is
//! silently wrong, not a refusal. An EMPTY ledger (a bare 32-byte value) is valid forever: every head before this
//! format.
//!
//! Fields (the architect's review, docs 2026-09-23-ledger-format-attack.md, ruled by main):
//! * [`TAG_PREV`] `prev_seq u64 LE ‖ prev_root [32]`: the head this one was signed from. Omitted at the genesis,
//!   never zeros.
//! * [`TAG_PARITY`]: since ledger version 2, the race-put MARK (COMMIT-LIFE §P): the field's presence means "every
//!   group this head lists was recoverable (k of k+m) when it was signed"; its body is the ROOT's parity ids
//!   (sdk#335: the root is a group of one), `m × 32` bytes, or empty when the writer listed none. A head without it is
//!   pre-§P, and a reader says `NotScanned` for it. In version 1 the tag carried #119's scan front, a different meaning, so a v1 parity field
//!   is DROPPED on read, never taken for the mark. At most [`PARITY_MAX`] bytes.
//! * [`TAG_THROUGH`] `n u16 ‖ n × (device [16] ‖ seq u64 LE ‖ last u64 LE)`, sorted by device, unique: how far each
//!   device's writes are published, and when each last wrote. `device` is a per-INSTALL id minted once and kept in the
//!   signer's secret store, never derived from the key (one key can be shared across devices until #226).
//!
//! **Reading is TOLERANT, from this format on, for every reader** (main's ruling: apps pin an SDK rev for life, so this
//! is the last head change an old reader cannot survive). The root is the first 32 bytes. A ledger is parsed if one is
//! present; an unknown tag is SKIPPED; a longer device list than this build would write is READ, not refused. A ledger
//! that is malformed (out of order, duplicated, truncated, a known field of the wrong shape) or of an unknown VERSION
//! is read as NO ledger (`refused`), and a head is never "no head" because of its ledger. A reader must treat a
//! refused ledger as "no prev" (the merge's conservative cell), never as "built on mine".
//!
//! **The version** bumps only when an existing tag's MEANING changes, so that an old reader would be wrong. Adding a
//! tag never bumps it: old readers skip the new tag.
//!
//! **Writing** ([`value`]) normalises: devices sorted and unique, the list evicted to [`THROUGH_MAX`] by least
//! recently written (the lowest `last`, then the lowest device id: deterministic, so both sides of a merge evict the
//! same entry). It never refuses: a refusal past the bound would be an identity that can never write again. The
//! SIGNER checks a value with [`check`] before it signs, the one place a malformed ledger can be stopped.

use crate::Head;

/// The Register contract's cap on a record's value: the contract's own number, never restated.
pub const VALUE_MAX: usize = craftec_register_contract::wire::MAX_VALUE;
/// The root, at the front of every value.
pub const ROOT_LEN: usize = 32;
/// The ledger's version, the first byte after the root when a ledger is present. 2 since race put:
/// [`TAG_PARITY`]'s MEANING changed (the §P mark, not #119's scan front). Version 1 is still READ.
pub const LEDGER_VERSION: u8 = 2;
/// The version before [`LEDGER_VERSION`], still read: its [`TAG_PARITY`] meant something else and is dropped.
pub const LEDGER_VERSION_V1: u8 = 1;
pub const TAG_PREV: u8 = 1;
pub const TAG_PARITY: u8 = 2;
pub const TAG_THROUGH: u8 = 3;
/// Bytes of one field's header: tag and length.
const FIELD_HEADER: usize = 3;
const PREV_LEN: usize = 8 + 32;
/// The parity mark's cap: the root's parity ids, 8 × 32 B (sdk#335). The page asserts the tree's parity per group fits.
pub const PARITY_MAX: usize = 256;
/// Bytes of one device entry.
pub const THROUGH_ENTRY: usize = 16 + 8 + 8;
/// Most devices a ledger carries, so that a ledger with EVERY field at its largest still fits the value:
/// `(4096 − 32 root − 1 version − (3 + 40) prev − (3 + 256) parity − (3 + 2) list header) / 32` = 117. With no
/// parity mark there would be room for 125; the bound is the one that always holds.
pub const THROUGH_MAX: usize =
    (VALUE_MAX - ROOT_LEN - 1 - (FIELD_HEADER + PREV_LEN) - (FIELD_HEADER + PARITY_MAX) - (FIELD_HEADER + 2)) / THROUGH_ENTRY;

/// One device's entry in [`TAG_THROUGH`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Through {
    pub device: [u8; 16],
    /// The seq through which this device's writes are in this head.
    pub seq: u64,
    /// The seq at which this device last wrote (what eviction orders by).
    pub last: u64,
}

/// What a value carries beyond its root.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ledger {
    pub prev: Option<Head>,
    pub parity: Option<Vec<u8>>,
    pub through: Vec<Through>,
}

impl Ledger {
    pub fn is_empty(&self) -> bool {
        self.prev.is_none() && self.parity.is_none() && self.through.is_empty()
    }
}

/// A head value, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadValue {
    pub root: [u8; 32],
    /// Empty when there was none, or when it was `refused`.
    pub ledger: Ledger,
    /// A ledger was present and could not be read (malformed, or an unknown version): read as root-only, and to be
    /// treated as NO prev.
    pub refused: bool,
}

/// Why [`check`] refuses a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Malformed {
    ShortRoot,
    TooLong,
    Version(u8),
    Truncated,
    OutOfOrder,
    PrevShape,
    ParityTooLong,
    ThroughShape,
    ThroughUnsorted,
    ThroughTooMany,
}

/// The head a value names, TOLERANTLY: `None` only for a value shorter than a root.
pub fn read_value(value: &[u8]) -> Option<HeadValue> {
    let root: [u8; 32] = value.get(..ROOT_LEN)?.try_into().ok()?;
    let rest = &value[ROOT_LEN..];
    if rest.is_empty() {
        return Some(HeadValue { root, ledger: Ledger::default(), refused: false });
    }
    Some(match parse(rest, false) {
        Ok(ledger) => HeadValue { root, ledger, refused: false },
        Err(_) => HeadValue { root, ledger: Ledger::default(), refused: true },
    })
}

/// STRICT, for the signer before it signs: the framing, every known field's shape, and the bounds this build writes.
pub fn check(value: &[u8]) -> Result<(), Malformed> {
    if value.len() < ROOT_LEN {
        return Err(Malformed::ShortRoot);
    }
    if value.len() > VALUE_MAX {
        return Err(Malformed::TooLong);
    }
    let rest = &value[ROOT_LEN..];
    if rest.is_empty() {
        return Ok(());
    }
    parse(rest, true).map(|_| ())
}

fn parse(rest: &[u8], strict: bool) -> Result<Ledger, Malformed> {
    let (&version, mut rest) = rest.split_first().ok_or(Malformed::Truncated)?;
    // The signer signs only what this build writes; a reader still reads v1.
    if version != LEDGER_VERSION && (strict || version != LEDGER_VERSION_V1) {
        return Err(Malformed::Version(version));
    }
    let mut ledger = Ledger::default();
    let mut last_tag = 0u8;
    while !rest.is_empty() {
        let (&tag, r) = rest.split_first().ok_or(Malformed::Truncated)?;
        let (len, r) = r.split_at_checked(2).ok_or(Malformed::Truncated)?;
        let len = u16::from_le_bytes([len[0], len[1]]) as usize;
        let (body, r) = r.split_at_checked(len).ok_or(Malformed::Truncated)?;
        if tag <= last_tag {
            return Err(Malformed::OutOfOrder);
        }
        last_tag = tag;
        rest = r;
        match tag {
            TAG_PREV => {
                if body.len() != PREV_LEN {
                    return Err(Malformed::PrevShape);
                }
                let seq = u64::from_le_bytes(body[..8].try_into().expect("8"));
                let root: [u8; 32] = body[8..].try_into().expect("32");
                ledger.prev = Some(Head { seq, root });
            }
            TAG_PARITY => {
                if strict && body.len() > PARITY_MAX {
                    return Err(Malformed::ParityTooLong);
                }
                // v1's parity field was #119's scan front, not the §P mark.
                if version == LEDGER_VERSION {
                    ledger.parity = Some(body.to_vec());
                }
            }
            TAG_THROUGH => {
                let (n, entries) = body.split_at_checked(2).ok_or(Malformed::ThroughShape)?;
                let n = u16::from_le_bytes([n[0], n[1]]) as usize;
                if entries.len() != n * THROUGH_ENTRY {
                    return Err(Malformed::ThroughShape);
                }
                // A longer list than this build writes is READ (a newer writer may allow more); only the signer,
                // checking what this build is about to sign, holds it to the bound.
                if strict && n > THROUGH_MAX {
                    return Err(Malformed::ThroughTooMany);
                }
                let mut prev_dev: Option<[u8; 16]> = None;
                for e in entries.chunks_exact(THROUGH_ENTRY) {
                    let device: [u8; 16] = e[..16].try_into().expect("16");
                    if prev_dev.is_some_and(|p| p >= device) {
                        return Err(Malformed::ThroughUnsorted);
                    }
                    prev_dev = Some(device);
                    ledger.through.push(Through {
                        device,
                        seq: u64::from_le_bytes(e[16..24].try_into().expect("8")),
                        last: u64::from_le_bytes(e[24..32].try_into().expect("8")),
                    });
                }
            }
            // An unknown tag: skipped (a newer writer's field).
            _ => {}
        }
    }
    Ok(ledger)
}

/// The value a head is signed as: `root`, then the ledger if it carries anything, NORMALISED (devices sorted and
/// unique, evicted to [`THROUGH_MAX`]). Never refuses.
pub fn value(root: &[u8; 32], ledger: &Ledger) -> Vec<u8> {
    let mut v = root.to_vec();
    let through = normalise(&ledger.through);
    if ledger.prev.is_none() && ledger.parity.is_none() && through.is_empty() {
        return v;
    }
    v.push(LEDGER_VERSION);
    let mut field = |tag: u8, body: &[u8]| {
        v.push(tag);
        v.extend_from_slice(&(body.len() as u16).to_le_bytes());
        v.extend_from_slice(body);
    };
    if let Some(p) = &ledger.prev {
        let mut b = p.seq.to_le_bytes().to_vec();
        b.extend_from_slice(&p.root);
        field(TAG_PREV, &b);
    }
    if let Some(m) = &ledger.parity {
        field(TAG_PARITY, &m[..m.len().min(PARITY_MAX)]);
    }
    if !through.is_empty() {
        let mut b = (through.len() as u16).to_le_bytes().to_vec();
        for t in &through {
            b.extend_from_slice(&t.device);
            b.extend_from_slice(&t.seq.to_le_bytes());
            b.extend_from_slice(&t.last.to_le_bytes());
        }
        field(TAG_THROUGH, &b);
    }
    v
}

/// Sorted by device, one entry per device (the max of each), then evicted to the bound: the least recently written
/// first (lowest `last`, then lowest device id), so every side that normalises the same entries evicts the same.
fn normalise(through: &[Through]) -> Vec<Through> {
    let mut by: std::collections::BTreeMap<[u8; 16], Through> = std::collections::BTreeMap::new();
    for t in through {
        let e = by.entry(t.device).or_insert(*t);
        e.seq = e.seq.max(t.seq);
        e.last = e.last.max(t.last);
    }
    let mut out: Vec<Through> = by.into_values().collect();
    while out.len() > THROUGH_MAX {
        let (i, _) = out.iter().enumerate().min_by_key(|(_, t)| (t.last, t.device)).expect("non-empty");
        out.remove(i);
    }
    out
}

/// LEDGERS MERGE (the architect's #1, ruled by main): two heads at one seq under one root differ only in their
/// ledgers, and the loser's entries must not vanish with its head. Per device the max; of two parity marks the greater
/// (one root has one parity list, so they differ only when one side listed none -- and a list beats none); `prev` is
/// the winner's. Pure; the ledger-only commit that carries it is #225b's.
pub fn merge(winner: &Ledger, loser: &Ledger) -> Ledger {
    let mut through = winner.through.clone();
    through.extend_from_slice(&loser.through);
    let parity = match (&winner.parity, &loser.parity) {
        (Some(a), Some(b)) => Some(if b > a { b.clone() } else { a.clone() }),
        (a, b) => a.clone().or_else(|| b.clone()),
    };
    Ledger { prev: winner.prev, parity, through: normalise(&through) }
}

/// A Register state's record, `(seq, value)`, read by THE REGISTER CRATE (`wire::record_head`, sdk#364): the
/// record's authority-free prefix, so it needs no params -- a page reading a head it knows only by contract id can
/// read it. A state also carrying fork EVIDENCE is read (a hand-written copy refused it -- F56 -- and would have
/// locked a device out after a same-version race). It does NOT verify the signatures: the contract did, before the
/// node stored it. `None`: no record in it, or not a Register state.
pub fn record_of(state: &[u8]) -> Option<(u64, &[u8])> {
    let (_terminal, seq, value) = craftec_register_contract::wire::record_head(state)?;
    Some((seq, value))
}
