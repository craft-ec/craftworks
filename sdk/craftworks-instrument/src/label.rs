//! Foreign ids, labelled.
//!
//! A block id, a peer key, a contract instance — anything drawn from a
//! population someone else can enumerate — is never written into a recording.
//! It is replaced by a LABEL: a kind and a small ordinal, `peer#3`, assigned in
//! the order the recorder first saw it.
//!
//! **Not truncated.** A prefix of an id drawn from an enumerable population is
//! not anonymous: with a few thousand candidates, a 32-bit prefix identifies
//! one of them. Truncation looks like redaction and is not, which is worse than
//! doing nothing, so this crate does not offer it.
//!
//! The label→id table is held by whoever is recording and is NEVER part of a
//! bundle. An operator reading a support bundle sees that `peer#3` was slow;
//! only the person who recorded it can say which peer that was.

use crate::vocab::Site;

/// What kind of foreign thing a label stands for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum Kind {
    Block,
    Peer,
    Contract,
    Request,
    /// One round of the SDK loader's bootstrap fetch (craftworks-sdk): a
    /// sequence of its own, apart from a page's sends ([`Kind::Request`]), so
    /// the two never collide in one recording.
    Fetch,
    /// A SPAN (`Enter`/`Exit`) a recorder's user numbers itself: its own
    /// sequence, so a span never shares an operation id with a request.
    Span,
    /// A DATA DOMAIN a page touched (craftworks-docs OBSERVABILITY §1): its ordinal is the domain's INDEX in the
    /// touching app's own published schema (`0..DOMAIN_SCHEMA_MAX`), or [`DOMAIN_UNLISTED`] for a touch the schema
    /// doesn't list -- never the domain's name, a row key or a value. Made by [`Label::domain`], which refuses the rest.
    Domain,
}

/// How many domains an app's schema may index for the recording (OBSERVABILITY §2.2): ordinals `0..DOMAIN_SCHEMA_MAX`.
pub const DOMAIN_SCHEMA_MAX: u32 = 64;
/// The one bucket for a touch the app's schema doesn't list.
pub const DOMAIN_UNLISTED: u32 = DOMAIN_SCHEMA_MAX;

impl Kind {
    /// Every kind, for a test or a reader to visit.
    pub const ALL: [Kind; 7] = [
        Kind::Request,
        Kind::Block,
        Kind::Peer,
        Kind::Contract,
        Kind::Fetch,
        Kind::Span,
        Kind::Domain,
    ];

    /// A small stable integer, carried in the top bits of the operation a label
    /// denotes ([`Label::op`]). Pinned by a test. `Request` is 0, so the op of
    /// every `req#n` is the plain `OpId(n)` it has always been.
    pub const fn code(self) -> u32 {
        match self {
            Kind::Request => 0,
            Kind::Block => 1,
            Kind::Peer => 2,
            Kind::Contract => 3,
            Kind::Fetch => 4,
            Kind::Span => 5,
            Kind::Domain => 6,
        }
    }

    /// The kind a [`code`](Kind::code) names, if any.
    pub const fn of_code(code: u32) -> Option<Kind> {
        match code {
            0 => Some(Kind::Request),
            1 => Some(Kind::Block),
            2 => Some(Kind::Peer),
            3 => Some(Kind::Contract),
            4 => Some(Kind::Fetch),
            5 => Some(Kind::Span),
            6 => Some(Kind::Domain),
            _ => None,
        }
    }

    pub const fn prefix(self) -> &'static str {
        match self {
            Kind::Block => "block",
            Kind::Peer => "peer",
            Kind::Contract => "contract",
            Kind::Request => "req",
            Kind::Fetch => "fetch",
            Kind::Span => "span",
            Kind::Domain => "domain",
        }
    }
}

/// `peer#3` — fixed size, `Copy`, and carrying nothing of the id itself.
///
/// Made only by [`Label::new`], which refuses an ordinal that would spill into
/// the kind bits of its operation id ([`MAX_ORDINAL`]): a sequence that reached
/// it would otherwise land in another kind's range -- the collision `op()`
/// exists to prevent, only later.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Label {
    kind: Kind,
    ordinal: u32,
}

impl Label {
    /// A label, or `None` for an ordinal past [`MAX_ORDINAL`]. A caller whose
    /// sequence reaches it stops labelling and says so; it never wraps.
    pub const fn new(kind: Kind, ordinal: u32) -> Option<Label> {
        if ordinal > MAX_ORDINAL {
            None
        } else {
            Some(Label { kind, ordinal })
        }
    }

    /// A data domain's label: its schema `index` (`< DOMAIN_SCHEMA_MAX`), or [`DOMAIN_UNLISTED`]. `None` past it:
    /// a domain ordinal is bounded, so a schema can't make one a counter of anything.
    pub const fn domain(index: u32) -> Option<Label> {
        if index > DOMAIN_UNLISTED {
            None
        } else {
            Some(Label {
                kind: Kind::Domain,
                ordinal: index,
            })
        }
    }

    pub const fn kind(self) -> Kind {
        self.kind
    }

    pub const fn ordinal(self) -> u32 {
        self.ordinal
    }

    /// The operation this label denotes.
    ///
    /// Every tool that has needed this derived it the same way and separately
    /// (`OpId(label.ordinal)`), which is two definitions of one fact waiting to
    /// disagree. It is stated once here so a recording can cross-reference an
    /// `Edge` with the `Exit` of the operation it belongs to — which is what
    /// makes "answered LATE" expressible at all.
    ///
    /// The KIND is part of it: `fetch#1` and `req#1` are two operations, and an
    /// `OpId` that forgot the kind made them one -- a loader round's `Exit`
    /// then closed a page send of the same ordinal, and `answers()` called that
    /// send LATE (found by craftworks-sdk's loader handover, one recording with
    /// both sequences). The kind's [`code`](Kind::code) rides in the top
    /// [`KIND_BITS`] bits; the ordinal keeps the rest ([`MAX_ORDINAL`]), and a
    /// larger one cannot be made ([`Label::new`]).
    pub const fn op(self) -> crate::OpId {
        crate::OpId::from_label(self.kind.code(), self.ordinal)
    }

    /// The label an operation id denotes, when it was made by [`Label::op`]:
    /// `None` for [`OpId::NONE`](crate::OpId::NONE) and for a kind code no
    /// [`Kind`] has.
    pub const fn of_op(op: crate::OpId) -> Option<Label> {
        let raw = op.raw();
        match Kind::of_code(raw >> ORDINAL_BITS) {
            Some(kind) => Some(Label {
                kind,
                ordinal: raw & MAX_ORDINAL,
            }),
            None => None,
        }
    }
}

/// Bits of an [`OpId`](crate::OpId) that carry a label's [`Kind`].
pub const KIND_BITS: u32 = 3;
/// Bits that carry its ordinal.
pub const ORDINAL_BITS: u32 = 32 - KIND_BITS;
/// The largest ordinal an operation id carries whole.
pub const MAX_ORDINAL: u32 = (1 << ORDINAL_BITS) - 1;

impl core::fmt::Display for Label {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}#{}", self.kind.prefix(), self.ordinal)
    }
}

/// Assigns labels, and keeps the only copy of the mapping.
///
/// Bounded like everything else here: past `CAP` distinct ids of a kind, new
/// ones all become `#CAP` rather than growing the table. A support bundle that
/// could be made to allocate without limit by a peer sending many ids is a
/// denial of service with extra steps.
pub struct Labels<Id> {
    seen: Vec<(Kind, Id)>,
    cap: u32,
}

impl<Id: PartialEq + Clone> Labels<Id> {
    pub const CAP: u32 = 512;

    pub fn new() -> Self {
        Labels {
            seen: Vec::with_capacity(64),
            cap: Self::CAP,
        }
    }

    /// The label for this id, assigning one on first sight.
    pub fn label(&mut self, kind: Kind, id: &Id) -> Label {
        if let Some(i) = self.seen.iter().position(|(k, x)| *k == kind && x == id) {
            return Label {
                kind,
                ordinal: i as u32,
            };
        }
        if (self.seen.len() as u32) < self.cap {
            self.seen.push((kind, id.clone()));
            Label {
                kind,
                ordinal: (self.seen.len() - 1) as u32,
            }
        } else {
            // Saturated rather than grown. Everything beyond the cap shares one
            // label, which a reader can see is a saturation and not a peer.
            Label {
                kind,
                ordinal: self.cap,
            }
        }
    }

    /// How many distinct ids this table holds — for the recorder's own report,
    /// never for a bundle.
    pub fn len(&self) -> usize {
        self.seen.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }

    /// Resolve a label back to its id. Held by the recorder's owner ONLY; this
    /// is the half a bundle must never contain.
    pub fn resolve(&self, label: Label) -> Option<&Id> {
        self.seen
            .iter()
            .filter(|(k, _)| *k == label.kind)
            .nth(label.ordinal as usize)
            .map(|(_, id)| id)
    }
}

impl<Id: PartialEq + Clone> Default for Labels<Id> {
    fn default() -> Self {
        Self::new()
    }
}

/// The site a label was first seen at, for the recorder's own diagnostics.
pub const LABELLED_AT: Site = Site::of("instrument::label");
