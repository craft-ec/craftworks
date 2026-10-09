//! Permanent instrumentation: a frozen four-event spine, a trait that can never
//! become an input, a recorder that drops rather than grows, and a dump that
//! prints itself when a test fails.
//!
//! # Why it is shaped like this
//!
//! Every constraint here was bought by a failure.
//!
//! - **`Edge` pairs a request with its response by id**, so *outstanding* is a
//!   subtraction rather than a consumer-side guess. Four harness runs were
//!   voided in one day for want of that number: a reader that had sent
//!   thousands of requests nobody would ever answer looked exactly like a
//!   reader doing its job.
//! - **`fn event(&self, e: Event)` returns unit and the trait has no read-back
//!   method.** Instrumented code therefore cannot branch on the probe, so the
//!   probe cannot become an input. That is the failure mode that turns a
//!   measurement into a story about itself.
//! - **A full ring DROPS and counts the drop.** It never grows (an unbounded
//!   buffer is a leak in a long run) and never panics (a probe that fails a
//!   passing test is the worst kind of behaviour change).
//! - **A per-recorder SEQUENCE, never a clock.** A delegate has no clock (F32),
//!   and a wall-clock stamp is a correlation handle across recordings.
//! - **A site CANNOT emit text into a recording, even deliberately.** The four
//!   event shapes carry `Site`, `OpId`, `Outcome`, `Label` and `Entry`, and the
//!   only string in any of them is `Site`'s `&'static str`. Writing a leak is
//!   not caught in review — it does not COMPILE:
//!
//!   ```compile_fail
//!   let note: String = std::env::args().next().unwrap_or_default();
//!   let _ = instrument::Event::Counter {
//!       site: instrument::Site::of(&note),   // `note` does not live long enough
//!       op: instrument::OpId::NONE,
//!       entry: instrument::Entry { key: instrument::Key::Effects, value: 0 },
//!   };
//!   ```
//!
//!   The guard is the `'static` LIFETIME, not the absence of strings: a value
//!   derived from user data cannot have that lifetime. The honest limit is that
//!   `Box::leak` would defeat it — the type system makes an accidental leak
//!   impossible and a deliberate one obvious, which is the most a type can do.
//!
//!   **This is why a `Note(String)` variant "just for diagnostics" must never be
//!   added.** It would not weaken the rule, it would DELETE the mechanism that
//!   holds it, and every site would go back to being trusted to remember. The
//!   reviewed vocabulary would become advice.
//!
//! - **No user content by VOCABULARY** — see [`vocab`]. Not "the payload is
//!   small": every key and enumerated value comes from a reviewed list, because
//!   the probes ship in production as the support tool.

pub mod dump;
pub mod gap;
pub mod label;
pub mod publish;
pub mod recorder;
pub mod vocab;

pub use gap::{Actual, Expected, Gap};
pub use label::{Kind, Label, Labels};
pub use recorder::{Record, Recorder, Recording, SyncRecorder, SyncRecording};
pub use vocab::{Bucket, Dir, Key, Outcome, Site, SizeClass};

/// The stream's version. ONE integer, at the stream level.
///
/// Not per event: a version on every event is a cost paid forever for a change
/// made rarely. An unknown PAYLOAD entry is skippable; an unknown SPINE variant
/// is not — which is why the spine is frozen and small, and everything that
/// churns rides in the payload. The same append-only rule ARCHITECTURE §19
/// already imposes on record encodings.
///
/// 2: an operation id CARRIES its label's kind (`Label::op`), so the same value
/// means something else than in version 1 -- a format change though no field
/// moved (craftworks-sdk's loader handover found `fetch#1` and `req#1`
/// colliding in one recording).
pub const STREAM_VERSION: u16 = 2;

/// Correlates the parts of one operation.
///
/// Made ONLY by [`Label::op`] -- which puts the label's [`Kind`] in the top
/// bits -- or [`OpId::NONE`]. The field is private: a raw id chosen by hand
/// would sit in kind 0's range and collide with a real label's operation.
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub struct OpId(u32);

impl OpId {
    /// A counter that belongs to no single operation — a connection-wide total,
    /// a ring drop. A sentinel rather than an `Option` so the event stays
    /// `Copy` and the same size whatever it carries. DISJOINT from every
    /// label's operation: its top bits are kind code 7, which no [`Kind`] has
    /// (a test enumerates them).
    pub const NONE: OpId = OpId(u32::MAX);

    /// The id as a number, for a reader of a recording.
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// `Label::op`'s encoding: the kind's code in the top bits. Crate-private.
    pub(crate) const fn from_label(code: u32, ordinal: u32) -> OpId {
        OpId((code << label::ORDINAL_BITS) | ordinal)
    }
}

/// What happened to a labelled operation, as the RECORDING sees it.
///
/// Derived from labelled pairs, never from a count: the two failures that
/// bought this (`harness#38`, `harness#39` run 1) were both a tool counting
/// answers rather than matching them to what they named.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Answered {
    /// Asked, and answered.
    Once,
    /// Asked, and nothing came back.
    Never,
    /// Answered AFTER its own operation had been closed for want of an answer.
    /// Not a success: it says the pairing of everything after it is suspect.
    Late,
    /// Answered more times than it was asked. The count is what arrived.
    Twice(usize),
    /// An answer for something this recorder never asked. Reported on its own,
    /// never attributed to whatever happened to be open.
    Foreign,
}

/// One payload entry: a reviewed key and a number. That is the whole open part.
///
/// No strings, no bytes, no generics. A site that wants to say something the
/// [`Key`] list cannot express adds a key to that list, in a reviewed change —
/// which is exactly the friction that keeps user data out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry {
    pub key: Key,
    pub value: u64,
}

/// The spine. Closed, frozen, four variants.
///
/// `Copy` and borrow-free on purpose: an event that borrows is an event that
/// can keep user bytes alive, and an event that allocates is a probe with a
/// cost that depends on what it is looking at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Event {
    /// An operation began.
    Enter { site: Site, op: OpId },
    /// An operation ended, and how.
    Exit {
        site: Site,
        op: OpId,
        outcome: Outcome,
    },
    /// A number, from the reviewed vocabulary, for one operation.
    ///
    /// `op` is [`OpId::NONE`] when the number belongs to no single operation.
    /// It is here rather than in a fifth variant because bytes-per-operation
    /// is the COST tier, and cost has always been `Counter`'s job — the spine
    /// stays at four.
    Counter { site: Site, op: OpId, entry: Entry },
    /// A request or a response carrying a LABELLED foreign id.
    ///
    /// The label, never the id — see [`label`].
    Edge { site: Site, dir: Dir, id: Label },
}

impl Event {
    pub const fn site(&self) -> Site {
        match self {
            Event::Enter { site, .. }
            | Event::Exit { site, .. }
            | Event::Counter { site, .. }
            | Event::Edge { site, .. } => *site,
        }
    }
}

/// Where events go.
///
/// **There is no method to read events back.** That is the guarantee the rest
/// of this crate is built on: instrumented code holds a `&dyn Probe` and can
/// do exactly one thing with it. A test that wants to READ a recording holds a
/// [`Recording`] handle instead, which the instrumented code never sees.
pub trait Probe {
    fn event(&self, e: Event);
}

/// The default: records nothing, costs a call the optimiser can usually see
/// through.
///
/// Its existence is what lets "recording off" be a PARAMETER rather than a
/// rebuild, and what the output differential compares against.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoProbe;

impl Probe for NoProbe {
    #[inline(always)]
    fn event(&self, _e: Event) {}
}

/// So `&P` is a probe wherever `P` is — callers should not have to care.
impl<P: Probe + ?Sized> Probe for &P {
    #[inline]
    fn event(&self, e: Event) {
        (**self).event(e)
    }
}

/// Emit an `Enter` now and the matching `Exit` on drop.
///
/// A span whose `Exit` is written by hand is a span that is missing whenever
/// the function returns early — which is precisely when a reader needs it.
pub struct Span<'p, P: Probe + ?Sized> {
    probe: &'p P,
    site: Site,
    op: OpId,
    outcome: Outcome,
}

impl<'p, P: Probe + ?Sized> Span<'p, P> {
    pub fn enter(probe: &'p P, site: Site, op: OpId) -> Self {
        probe.event(Event::Enter { site, op });
        Span {
            probe,
            site,
            op,
            // Nothing said otherwise, so it ended by falling off the end. A
            // span that vanished mid-flight is `Blocked`, set by the caller.
            outcome: Outcome::Ok,
        }
    }

    /// Say how it ended. The `Exit` is still emitted by the drop.
    pub fn finish(mut self, outcome: Outcome) {
        self.outcome = outcome;
    }
}

impl<P: Probe + ?Sized> Drop for Span<'_, P> {
    fn drop(&mut self) {
        self.probe.event(Event::Exit {
            site: self.site,
            op: self.op,
            outcome: self.outcome,
        });
    }
}
