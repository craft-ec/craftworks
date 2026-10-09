//! THE ONE PUBLISH FILTER (craftworks-docs OBSERVABILITY §2; craftworks-sdk#399): the only way a recording leaves the
//! device. A page writes one [`Published`] record per window per app into the person's observation tree, and before
//! Phase 7 this filter and the vocabulary's no-user-content rule are the WHOLE protection -- the tree is public.
//!
//! What leaves, and at what grain, is decided once, per key, by [`vocab::publish`](crate::vocab::publish) (the leak
//! table). This module only applies it:
//! - a counter of a LOCAL key is dropped silently (it was never publishable; `dropped` counts publishable events lost);
//! - a published counter leaves at its [`Grain`]: a [`Bucket`], a [`SizeClass`], whole seconds from the WINDOW's
//!   start, or a closed enum code;
//! - an operation's ending leaves as its [`Outcome`]; `Enter` and `Edge` events stay local in this first cut;
//! - the counters of a DATA DOMAIN (the [`Kind::Domain`] label's operation) are folded into one [`DomainCounts`] per
//!   domain: counts bucketed, bytes as size classes;
//! - OPERATIONS ARE RENUMBERED per record, in order of first appearance: a ring's own ordinal (`req#17`) is the page's
//!   exact operation count since load, an unbucketed activity count across windows (the architect on #399). In a
//!   record an ordinal only links events inside that record;
//! - the label table never leaves (it is not an input here), and no foreign id can: events carry labels, never ids;
//! - the encoded record is at most [`MAX_RECORD_BYTES`]: past it, events are dropped (last first), domain counts kept,
//!   and the loss is counted in `dropped`.

use crate::label::{Kind, Label, DOMAIN_UNLISTED};
use crate::recorder::Record;
use crate::vocab::{
    publish as publishability, Bucket, Grain, Key, Outcome, Publish, Site, SizeClass, StatusClass,
    PUBLISHED_OFFSET_GRAIN_MS,
};
use crate::{Event, OpId};

/// B: the most an encoded record may take (OBSERVABILITY §3: ≤ 16 KiB; ≤ 64 domains × ~60 B fit well within).
pub const MAX_RECORD_BYTES: usize = 16 * 1024;

/// The encoding's version byte.
pub const RECORD_VERSION: u8 = 1;

/// One window: the minute its record is keyed by, and where it starts on the RECORDING's offset clock (the ring's
/// `OffsetMs` values are milliseconds from the recording's start).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Window {
    pub minute: u64,
    pub start_ms: u64,
    /// The ring's drop count ([`Record::dropped`], a running total since load) when this window began: a record
    /// publishes the drops of ITS window only.
    pub dropped_at_start: u64,
    /// Publishable events lost OUTSIDE the ring before this window: a whole earlier record the page could not keep
    /// (its bounded pending list overflowed, sdk#399 step 4) -- that record's events plus its own drop count. ADDED to
    /// this window's loss, never folded into `dropped_at_start` (whose one meaning is the ring's total at the start;
    /// adding to it would LOWER the published count).
    pub lost_before: u64,
}

/// A window's length: one minute.
pub const WINDOW_MS: u64 = 60_000;

/// The build header a page's first window carries -- see [`HEADER_LEAK`](crate::vocab::HEADER_LEAK).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Header {
    /// The sha256 of the SDK wasm the page RAN.
    pub sdk_sha: [u8; 32],
    /// The app's published version.
    pub app_version: u64,
}

/// An operation, as a record names it: its kind, and an ordinal that is LOCAL to the record (see the module doc).
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub struct PubOp {
    pub kind: u8,
    pub ordinal: u32,
}

/// One published event.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PubEvent {
    /// An operation ended.
    Exit {
        site: Site,
        op: PubOp,
        outcome: Outcome,
    },
    /// An operation still UNANSWERED at the window's end: a stall, which support needs -- see
    /// [`UNANSWERED_LEAK`](crate::vocab::UNANSWERED_LEAK).
    Unanswered { site: Site, op: PubOp },
    /// A published key's value, already at its grain: a bucket's or a class's code, whole seconds, or an enum code.
    Counter {
        site: Site,
        op: Option<PubOp>,
        key: Key,
        value: u64,
    },
}

/// What one page did to one data domain in the window, at the published grain.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DomainCounts {
    /// The domain's ordinal in the app's schema, or [`DOMAIN_UNLISTED`](crate::label::DOMAIN_UNLISTED).
    pub domain: u32,
    pub reads: Bucket,
    pub writes: Bucket,
    pub fails: Bucket,
    pub bytes_read: SizeClass,
    pub bytes_written: SizeClass,
}

/// One window's record: the filter's whole output. Closed, and without a string a person could have written.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Published {
    pub minute: u64,
    pub header: Option<Header>,
    pub events: Vec<PubEvent>,
    pub domains: Vec<DomainCounts>,
    /// PUBLISHABLE events lost: the ring's drops plus those cut to fit [`MAX_RECORD_BYTES`]. One meaning only.
    pub dropped: Bucket,
}

/// Apply the filter to one window's recording.
pub fn publish(rec: &impl Record, window: Window, header: Option<Header>) -> Published {
    let all = rec.events();
    // Which OPERATIONS may leave (the architect on instrument#18): one that ENDED non-Ok in this window, or is still
    // unanswered at its end. An Ok operation's events stay local: their count and offsets would be the page's exact
    // activity, which the domain buckets exist to coarsen. The LAST exit of an operation in the window decides.
    let mut ops_seen: Vec<(OpId, Site, Option<Outcome>)> = Vec::new();
    let mut note = |op: OpId, site: Site, exit: Option<Outcome>| {
        if op == OpId::NONE || Label::of_op(op).is_none_or(|l| l.kind() == Kind::Domain) {
            return;
        }
        match ops_seen.iter_mut().find(|(o, _, _)| *o == op) {
            Some(seen) => {
                if exit.is_some() {
                    seen.2 = exit;
                }
            }
            None => ops_seen.push((op, site, exit)),
        }
    };
    for e in &all {
        match *e {
            Event::Enter { site, op } => note(op, site, None),
            Event::Exit { site, op, outcome } => note(op, site, Some(outcome)),
            Event::Counter { site, op, .. } => note(op, site, None),
            Event::Edge { .. } => {}
        }
    }
    let leaves = |op: OpId| {
        ops_seen
            .iter()
            .any(|(o, _, exit)| *o == op && !matches!(exit, Some(Outcome::Ok)))
    };
    let mut ops: Vec<OpId> = Vec::new();
    let mut local = |op: OpId| -> Option<PubOp> {
        let label = Label::of_op(op)?;
        let at = match ops.iter().position(|o| *o == op) {
            Some(at) => at,
            None => {
                ops.push(op);
                ops.len() - 1
            }
        };
        Some(PubOp {
            kind: label.kind().code() as u8,
            ordinal: at as u32,
        })
    };
    // Per domain: (reads, writes, fails, bytes read, bytes written), summed exactly here and coarsened on the way out.
    let mut domains: Vec<(u32, [u64; 5])> = Vec::new();
    let mut events = Vec::new();
    for e in all {
        match e {
            Event::Exit { site, op, outcome } => {
                if outcome != Outcome::Ok && leaves(op) {
                    if let Some(op) = local(op) {
                        events.push(PubEvent::Exit { site, op, outcome });
                    }
                }
            }
            Event::Counter { site, op, entry } => {
                let Publish::Published { grain, .. } = publishability(entry.key) else {
                    continue;
                };
                if let Some(slot) = domain_slot(entry.key) {
                    // A domain count belongs to its domain's label, and to nothing else. An ordinal past the schema
                    // bound FOLDS into `unlisted`: never truncated into another domain on the way out.
                    let Some(label) = Label::of_op(op).filter(|l| l.kind() == Kind::Domain) else {
                        continue;
                    };
                    let d = label.ordinal().min(DOMAIN_UNLISTED);
                    let at = match domains.iter().position(|(x, _)| *x == d) {
                        Some(at) => at,
                        None => {
                            domains.push((d, [0; 5]));
                            domains.len() - 1
                        }
                    };
                    domains[at].1[slot] = domains[at].1[slot].saturating_add(entry.value);
                    continue;
                }
                // Only an operation that leaves carries its counters out; a connection-wide counter stays local.
                if op == OpId::NONE || !leaves(op) {
                    continue;
                }
                let value = match grain {
                    Grain::Bucket => Bucket::of(entry.value) as u64,
                    Grain::SizeClass => {
                        SizeClass::of(usize::try_from(entry.value).unwrap_or(usize::MAX)) as u64
                    }
                    // Only an offset INSIDE the window's minute: seconds 0..59, never below the minute by construction.
                    Grain::Seconds => {
                        if entry.value < window.start_ms
                            || entry.value - window.start_ms >= WINDOW_MS
                        {
                            continue;
                        }
                        (entry.value - window.start_ms) / PUBLISHED_OFFSET_GRAIN_MS
                    }
                    Grain::Enum => match enum_code(entry.key, entry.value) {
                        Some(v) => v,
                        None => continue,
                    },
                };
                if let Some(op) = local(op) {
                    events.push(PubEvent::Counter {
                        site,
                        op: Some(op),
                        key: entry.key,
                        value,
                    });
                }
            }
            // Local in this first cut: an Enter adds nothing an Exit doesn't; an Edge names a foreign thing.
            Event::Enter { .. } | Event::Edge { .. } => {}
        }
    }
    // A STALL: every operation still unanswered at the window's end is said, once, where it first appeared.
    for (op, site, exit) in &ops_seen {
        if exit.is_none() {
            if let Some(op) = local(*op) {
                events.push(PubEvent::Unanswered { site: *site, op });
            }
        }
    }
    domains.sort_by_key(|(d, _)| *d);
    let domains = domains
        .into_iter()
        .map(|(domain, c)| DomainCounts {
            domain,
            reads: Bucket::of(c[0]),
            writes: Bucket::of(c[1]),
            fails: Bucket::of(c[2]),
            bytes_read: SizeClass::of(usize::try_from(c[3]).unwrap_or(usize::MAX)),
            bytes_written: SizeClass::of(usize::try_from(c[4]).unwrap_or(usize::MAX)),
        })
        .collect();
    // THIS window's drops (the ring's count is its running total since load), plus what was lost outside the ring
    // before it (a record the page could not keep).
    let lost = rec
        .dropped()
        .saturating_sub(window.dropped_at_start)
        .saturating_add(window.lost_before);
    let mut out = Published {
        minute: window.minute,
        header,
        events,
        domains,
        dropped: Bucket::of(lost),
    };
    let mut cut = 0u64;
    // The bound: events go first, last first; the domain counts stay.
    while out.encode().len() > MAX_RECORD_BYTES && out.events.pop().is_some() {
        cut += 1;
        out.dropped = Bucket::of(lost.saturating_add(cut));
    }
    out
}

/// The slot of a domain key in a [`DomainCounts`] sum, or `None` for any other key.
const fn domain_slot(k: Key) -> Option<usize> {
    match k {
        Key::RecordReads => Some(0),
        Key::RecordWrites => Some(1),
        Key::RecordFails => Some(2),
        Key::RecordBytesRead => Some(3),
        Key::RecordBytesWritten => Some(4),
        _ => None,
    }
}

/// An enum-grain key's value, if it IS one of its enum's codes: anything else is not a vocabulary value and doesn't leave.
fn enum_code(k: Key, v: u64) -> Option<u64> {
    match k {
        Key::StatusClass => StatusClass::ALL.iter().any(|c| c.code() == v).then_some(v),
        _ => None,
    }
}

// ---- the encoding: canonical, deterministic, versioned (the tree's value) ----

fn outcome_code(o: Outcome) -> (u8, u16) {
    match o {
        Outcome::Ok => (0, 0),
        Outcome::Missing => (1, 0),
        Outcome::Refused(c) => (2, c),
        Outcome::Blocked => (3, 0),
        Outcome::Timeout => (4, 0),
        Outcome::Withdrawn => (5, 0),
    }
}

fn outcome_of(tag: u8, c: u16) -> Option<Outcome> {
    Some(match tag {
        0 => Outcome::Ok,
        1 => Outcome::Missing,
        2 => Outcome::Refused(c),
        3 => Outcome::Blocked,
        4 => Outcome::Timeout,
        5 => Outcome::Withdrawn,
        _ => return None,
    })
}

/// The published keys' wire codes. Only a published key has one: a Local key cannot be encoded at all.
const KEY_CODES: [(Key, u8); 8] = [
    (Key::RecordReads, 1),
    (Key::RecordWrites, 2),
    (Key::RecordFails, 3),
    (Key::RecordBytesRead, 4),
    (Key::RecordBytesWritten, 5),
    (Key::Attempts, 6),
    (Key::StatusClass, 7),
    (Key::OffsetMs, 8),
];

fn key_code(k: Key) -> Option<u8> {
    KEY_CODES.iter().find(|(key, _)| *key == k).map(|(_, c)| *c)
}

fn key_of(c: u8) -> Option<Key> {
    KEY_CODES
        .iter()
        .find(|(_, code)| *code == c)
        .map(|(k, _)| *k)
}

const BUCKETS: [Bucket; 6] = [
    Bucket::Zero,
    Bucket::One,
    Bucket::UpTo9,
    Bucket::UpTo99,
    Bucket::UpTo999,
    Bucket::Over999,
];
const CLASSES: [SizeClass; 7] = [
    SizeClass::Empty,
    SizeClass::UpTo1K,
    SizeClass::UpTo4K,
    SizeClass::UpTo16K,
    SizeClass::UpTo64K,
    SizeClass::UpTo256K,
    SizeClass::Over256K,
];

/// Sites decoded from a record: a site is a `&'static str` of the EMITTING crate, so a reader maps names it knows.
/// An unknown name decodes as this site (the name itself is kept out of a `&'static` by design).
pub const UNKNOWN_SITE: Site = Site::of("instrument::publish::unknown-site");

impl Published {
    /// The canonical bytes: the observation tree's value for this window.
    pub fn encode(&self) -> Vec<u8> {
        let mut b = vec![RECORD_VERSION];
        b.extend_from_slice(&self.minute.to_be_bytes());
        match self.header {
            Some(h) => {
                b.push(1);
                b.extend_from_slice(&h.sdk_sha);
                b.extend_from_slice(&h.app_version.to_be_bytes());
            }
            None => b.push(0),
        }
        b.push(self.dropped as u8);
        b.push(self.domains.len() as u8);
        for d in &self.domains {
            b.push(d.domain as u8);
            b.extend_from_slice(&[
                d.reads as u8,
                d.writes as u8,
                d.fails as u8,
                d.bytes_read as u8,
                d.bytes_written as u8,
            ]);
        }
        b.extend_from_slice(&(self.events.len() as u32).to_be_bytes());
        let op = |b: &mut Vec<u8>, op: Option<PubOp>| match op {
            Some(o) => {
                b.push(o.kind);
                b.extend_from_slice(&o.ordinal.to_be_bytes());
            }
            None => b.push(0xFF),
        };
        for e in &self.events {
            match *e {
                PubEvent::Exit {
                    site,
                    op: o,
                    outcome,
                } => {
                    b.push(0);
                    site_bytes(&mut b, site);
                    op(&mut b, Some(o));
                    let (t, c) = outcome_code(outcome);
                    b.push(t);
                    b.extend_from_slice(&c.to_be_bytes());
                }
                PubEvent::Unanswered { site, op: o } => {
                    b.push(2);
                    site_bytes(&mut b, site);
                    op(&mut b, Some(o));
                }
                PubEvent::Counter {
                    site,
                    op: o,
                    key,
                    value,
                } => {
                    b.push(1);
                    site_bytes(&mut b, site);
                    op(&mut b, o);
                    b.push(key_code(key).expect("only a published key is ever in a record"));
                    b.extend_from_slice(&value.to_be_bytes());
                }
            }
        }
        b
    }

    /// Read a record back (support's view, `dump` over it). `sites` maps the names a reader knows back to their
    /// [`Site`]; anything else is [`UNKNOWN_SITE`]. `None` for bytes that aren't a record of this version.
    pub fn decode(bytes: &[u8], sites: &[Site]) -> Option<Published> {
        let mut r = Reader { b: bytes, at: 0 };
        if r.u8()? != RECORD_VERSION {
            return None;
        }
        let minute = r.u64()?;
        let header = match r.u8()? {
            0 => None,
            1 => Some(Header {
                sdk_sha: r.array()?,
                app_version: r.u64()?,
            }),
            _ => return None,
        };
        let dropped = *BUCKETS.get(r.u8()? as usize)?;
        let mut domains = Vec::new();
        for _ in 0..r.u8()? {
            domains.push(DomainCounts {
                domain: r.u8()? as u32,
                reads: *BUCKETS.get(r.u8()? as usize)?,
                writes: *BUCKETS.get(r.u8()? as usize)?,
                fails: *BUCKETS.get(r.u8()? as usize)?,
                bytes_read: *CLASSES.get(r.u8()? as usize)?,
                bytes_written: *CLASSES.get(r.u8()? as usize)?,
            });
        }
        let n = u32::from_be_bytes(r.array()?);
        let mut events = Vec::new();
        for _ in 0..n {
            let tag = r.u8()?;
            let name_len = r.u8()? as usize;
            let name = std::str::from_utf8(r.take(name_len)?).ok()?;
            let site = sites
                .iter()
                .copied()
                .find(|s| s.name() == name)
                .unwrap_or(UNKNOWN_SITE);
            let kind = r.u8()?;
            let op = if kind == 0xFF {
                None
            } else {
                Some(PubOp {
                    kind,
                    ordinal: u32::from_be_bytes(r.array()?),
                })
            };
            events.push(match tag {
                0 => {
                    let t = r.u8()?;
                    PubEvent::Exit {
                        site,
                        op: op?,
                        outcome: outcome_of(t, u16::from_be_bytes(r.array()?))?,
                    }
                }
                1 => PubEvent::Counter {
                    site,
                    op,
                    key: key_of(r.u8()?)?,
                    value: r.u64()?,
                },
                2 => PubEvent::Unanswered { site, op: op? },
                _ => return None,
            });
        }
        (r.at == bytes.len()).then_some(Published {
            minute,
            header,
            events,
            domains,
            dropped,
        })
    }
}

fn site_bytes(b: &mut Vec<u8>, site: Site) {
    let name = site.name().as_bytes();
    let n = name.len().min(255);
    b.push(n as u8);
    b.extend_from_slice(&name[..n]);
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.b.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_be_bytes(self.array()?))
    }
    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.take(N)?.try_into().ok()
    }
}
