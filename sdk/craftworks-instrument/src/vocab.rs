//! The vocabulary: every word an event may contain.
//!
//! **Redaction is by VOCABULARY, not by type.** A fixed-size, `Copy`, strongly
//! typed value can still be sensitive — `domain = "medical"` is all three. So
//! the guarantee this crate offers is not "the payload is small" but "every key
//! and every enumerated value came from this file, which is reviewed".
//!
//! A site therefore cannot emit a value DERIVED from user data, however small.
//! The three shapes that would otherwise leak are each given a lawful form:
//!
//! - **Foreign ids** are LABELLED per bundle (`peer#3`), never truncated. A
//!   truncated id drawn from an enumerable population is not anonymous — a
//!   few thousand candidates are trivially re-identified — so truncation is
//!   not offered at all. The label→id table lives with whoever recorded it and
//!   is never part of a bundle.
//! - **Sizes of sealed data** are reported as a PADDED CLASS, because the exact
//!   length of a ciphertext is a property of the plaintext.
//! - **Counts derived from user data** are BUCKETED, and times are COARSENED
//!   offsets from the start of the recording.
//!
//! **The bar RISES rather than falls.** ARCHITECTURE §5 now sends execution
//! facts to an adjacent REPLICATED tree, so "never leaves the device unasked"
//! becomes "published by construction" — a recording that was merely private
//! before is shared by default, and every guarantee above has to hold against
//! that.
//!
//! The guard is the `&'static str` lifetime, which a runtime value cannot have:
//!
//! ```compile_fail
//! # use instrument::vocab::Site;
//! let from_user: String = std::env::args().next().unwrap();
//! let site = Site::of(&from_user); // does not live long enough
//! ```
//!
//! `Box::leak` would defeat it. That is the honest limit: the lifetime stops
//! the accident, not someone determined to leak.

/// Where an event happened.
///
/// A newtype so it cannot be built from a runtime string: the only way to make
/// one is [`Site::of`], which takes a `&'static str` — a compile-time literal
/// in the emitting crate, never anything a user supplied.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Site(&'static str);

impl Site {
    /// `&'static str` is the whole guard: a value derived from user data cannot
    /// have that lifetime without being leaked deliberately.
    pub const fn of(name: &'static str) -> Site {
        Site(name)
    }
    pub const fn name(self) -> &'static str {
        self.0
    }
}

/// What a payload entry is ABOUT. Closed, and every addition is reviewed.
///
/// # The test every key must pass
///
/// **Observability is DERIVED where the fact is a property of the DATA, and
/// INSTRUMENTED only where it is a property of an EXECUTION.**
///
/// Derived — never a key here:
/// - how much a tree stores: already in every node's aggregate (ARCHITECTURE §5);
/// - which blocks a write rewrote: a `diff` of two roots, read only where they
///   differ;
/// - what an operation SHOULD cost: a pure function of the tree's shape, since
///   node boundaries are deterministic.
///
/// Instrumented — what these keys are for:
/// - time, and anything measured against a clock the data does not have;
/// - bytes ACTUALLY transferred, which is not what the data says they should be;
/// - hops, attempts, retries;
/// - where an operation died, and **what did not happen at all**.
///
/// # The third class: WOULD-PAY-OR-STEER
///
/// The sort is three ways, not two. Anything that can **pay or steer a
/// choice** — quota, keeper selection, reputation — is not a self-observation
/// at all, and may never be emitted as one.
///
/// ARCHITECTURE §5 settles why: *"if node behaviour steers keeper selection,
/// 'B says A answered in T' counts and 'A says A is fast' does not — the
/// subject never attests to itself."* Such a fact is a **receipt the
/// COUNTERPARTY signs into its own tree**, double-entry, and a balance moves
/// only where both halves agree (§14). A self-reported number cannot be any of
/// that: it has one author, who profits from it.
///
/// So this crate's keys are for **diagnosis only**. §5 again: *"Observations
/// that only DIAGNOSE need no receipt, because nobody profits from a wrong
/// latency figure that steers nothing; the two classes are separated from the
/// first event, not later."*
///
/// [`PayOrSteer`] holds the list, and it is EMPTY. It is not empty because
/// nobody thought of one — it is empty because a key that belongs there does
/// not belong in this crate. The list exists so that the first quota counter
/// has somewhere to be declared, and declaring it fails a test with the
/// reason, rather than looking like an ordinary event and quietly becoming
/// one.
///
/// A key that reports a derivable fact is DELETED, not maintained: it is a
/// second account of something the data already answers, and two accounts of
/// one fact disagree. Three were removed by that test —
/// [`Record::outstanding`](crate::Record::outstanding) already derives
/// outstanding work from the edges, [`Record::dropped`](crate::Record::dropped)
/// already reads the ring's own counter, and a tree's depth is a property of
/// its root.
///
/// The test has a cost when it is not applied. A 1 KiB block was measured
/// costing 240-395 KB to fetch (F46) and it took six rejected instruments and a
/// night to establish, because nothing said what it SHOULD have cost. Against a
/// derived expectation the same run reads as a ratio on the first attempt.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum Key {
    /// Blocks asked for.
    Reads,
    /// Blocks asked for that the source did not have.
    Misses,
    /// Bytes handed back, as a padded class — see [`SizeClass`].
    BytesClass,
    /// Requests sent on a connection.
    Sent,
    /// Responses received.
    Received,
    /// A bucketed count derived from user data — see [`Bucket`].
    CountBucket,
    /// A coarsened offset from the start of the recording, in milliseconds.
    OffsetMs,
    /// Retries, re-puts, re-sends.
    Attempts,
    /// Answers a caller gave up waiting for and is still OWED.
    ///
    /// A bounded wait that expires does not cancel anything: the node still
    /// owes that answer, and it arrives later on a connection that carries no
    /// correlation id. Until something can say WHICH request an answer
    /// settles, a transport that pairs by position is shifted by one from the
    /// moment this is non-zero — which is the defect that made a node look
    /// like it had refused fifteen puts it had in fact accepted
    /// (freenet-harness#38).
    ///
    /// It does not go down. Nothing can prove which later answer settled an
    /// owed one, so a decrement would be a guess written as a fact; this
    /// counts how many times pairing-by-position lost its footing.
    Owed,
    /// Effects the core returned in one delegate call.
    ///
    /// These six are the delegate's PER-CALL REPORT. They exist because a
    /// delegate prints nothing and the node says nothing about it, so five
    /// different breaks in the write path all presented as one symptom — a
    /// write that stops at `Accepted` — and these counts are what separated
    /// them. Every one is a number the engine already had; none is derived
    /// from user data.
    Effects,
    /// Node operations issued in one delegate call.
    Ops,
    /// Blocks put and not yet read back.
    Awaiting,
    /// Puts confirmed by reading them back.
    ///
    /// The read-back, not the acknowledgement: `durable` has always meant the
    /// block can be SERVED, never that an operation finished (F22/F31).
    ReadBack,
    /// Effects still queued when the call ended, and therefore LOST.
    ///
    /// Must be zero. It is recorded rather than asserted so that a non-zero one
    /// is visible in a dump without anybody doing arithmetic.
    Stranded,
    /// Inbound messages this build could not use, by reason.
    ///
    /// The reason is a closed vocabulary — see [`DropReason`] — never a string.
    DroppedMsgs,
    /// Bytes offered to the socket for one operation, counted at OUR boundary.
    ///
    /// A byte COUNT is not derived from content — it is the size of what was
    /// sent, which the transport already knows — so it is exact rather than
    /// bucketed. The size of SEALED data is different and goes through
    /// [`SizeClass`]: a ciphertext's length is a property of its plaintext.
    ///
    /// This measures what WE cost, not what a node sends peer-to-peer. Six
    /// external instruments were rejected trying to infer the latter from
    /// outside (freenet-contracts#39); only the node can count that, and this
    /// is the half we are positioned to count honestly.
    BytesOut,
    /// Bytes read from the socket for one operation, counted at OUR boundary.
    BytesIn,
    /// Answers received while [`Owed`](Key::Owed) was non-zero.
    ///
    /// Counted, and deliberately NOT attributed to any operation: an answer
    /// that might belong to an abandoned request must not close a running one.
    /// A reader seeing this above zero knows the pairing in that recording is
    /// no longer trustworthy, which is exactly what the healthy-looking line
    /// over a shifted stream failed to say.
    Ambiguous,
    /// The WIDTH, in hex characters, of a record id a read was handed — 32 or
    /// 64 — when it did not match the width its domain keys by.
    ///
    /// A read answers "not found" for an id its domain cannot address (the ids
    /// reaching a read come from outside: a stale link, an id stored before a
    /// domain declared a parent — craftworks-sdk#118). That is honest, and it
    /// makes one programmer error SILENT: a caller that truncated a 64-hex id.
    /// This pair keeps that diagnosable without a throw. Only the widths: never
    /// the id, and never the domain, whose name is the app's and so falls
    /// under the vocabulary rule.
    IdWidthGiven,
    /// The width its domain wanted — see [`IdWidthGiven`](Key::IdWidthGiven).
    IdWidthWanted,

    /// The retransmission timeout a sender USED, in milliseconds, coarsened
    /// ([`coarsen_ms`]): at the moment it armed an operation, and after each
    /// round-trip sample.
    ///
    /// These four keys are the page's retry clock (craftworks-sdk#386,
    /// sdk#397). It had to be reconstructed from wire frames once: a lost PUT
    /// waited a minute though its siblings answered in two seconds, and the
    /// cause -- requests dated against a clock started at 0, so the first
    /// answers were "samples" of ~1.8e12 ms that pinned the timeout at its
    /// ceiling -- was invisible to everything that recorded.
    ///
    /// NOT derivable: the RTO is a function of the samples, but re-deriving it
    /// means REPLAYING the estimator, which assumes the very code a reader is
    /// diagnosing is right. This is what the sender actually used.
    RtoMs,
    /// A round-trip SAMPLE a sender fed its estimator, in milliseconds,
    /// coarsened. The value as the sender computed it -- a wrong clock shows
    /// here as an impossible sample, which is the point.
    SampleMs,
    /// When an operation is due to be sent again if unanswered: an OFFSET from
    /// the start of the recording in milliseconds, coarsened -- never the
    /// sender's own clock, which is a correlation handle.
    ArmedAtMs,
    /// Where a RE-ARM moved an operation's deadline (an answer to another
    /// operation restarted its timer): an offset like [`ArmedAtMs`](Key::ArmedAtMs).
    ReArmedAtMs,
    /// How ONE round of a fetch ended, as a [`StatusClass`] code: the
    /// SDK loader's `served()` / `raceK` (craftworks-sdk, the bootstrap fetch
    /// that runs before the page exists). A class, never the status line or
    /// the URL -- a URL names the file, and the file is the app's.
    StatusClass,

    /// Records of one DATA DOMAIN (a `d/<domain>/…` collection, ARCHITECTURE §5) the page READ for a client request,
    /// counted where the request names its keys (craftworks-sdk `page::server`), never in the engine. The operation it
    /// belongs to is the domain's [`Label`](crate::Label) of kind [`Domain`](crate::Kind::Domain): the domain's ORDINAL
    /// in the touching app's own published schema, never its name (OBSERVABILITY §1). Published ALWAYS bucketed
    /// ([`Bucket`]): it counts the person's activity.
    RecordReads,
    /// Records of one data domain the page WROTE — see [`RecordReads`](Key::RecordReads).
    RecordWrites,
    /// Reads or writes of one data domain that FAILED — see [`RecordReads`](Key::RecordReads).
    RecordFails,
    /// Bytes of records of one data domain read — published ALWAYS as a padded [`SizeClass`], never exact.
    RecordBytesRead,
    /// Bytes of records of one data domain written — see [`RecordBytesRead`](Key::RecordBytesRead).
    RecordBytesWritten,
    /// A line SAID to the app at this site (an `unusable` line, craftworks-sdk#482): COUNTED here, one per line, with
    /// no text -- the SITE names which line; the text itself (which can name keys, or a node's refusal) stays only in
    /// the app's own drained list, never in a recording.
    Said,

    /// NOT A REAL KEY. A negative control, and it cannot ship: `cfg(test)`.
    ///
    /// Every real key in this crate is [`Class::Diagnostic`], which leaves the
    /// classifier indistinguishable from a function that returns that constant
    /// — so a later edit flattening it would pass every test. This is the
    /// sample that makes the sort OBSERVABLE: a key that must come back
    /// [`Class::PayOrSteer`], so a constant classifier fails.
    #[cfg(test)]
    SampleQuotaSpent,

    /// NOT A REAL KEY. The same control for [`Class::Derivable`].
    #[cfg(test)]
    SampleDepth,
}

/// Keys that would PAY or STEER, and therefore may never be self-observed.
///
/// Empty, and that is the point. A fact that can pay or steer a choice is a
/// receipt the counterparty signs into its own tree (ARCHITECTURE §5, §14) —
/// double-entry, where a balance moves only where both halves agree. A
/// self-reported number has one author, who profits from it.
///
/// If a key ever belongs here, it does not belong in this crate: it belongs in
/// the record the counterparty writes. The list exists so that the attempt is
/// DECLARED rather than silently made, and so the test below can say why.
pub const PAY_OR_STEER: &[Key] = &[];

/// Every key, for tests that must visit all of them.
///
/// The list can drift from the enum, so it is NOT the guard: [`class`] is,
/// because its match is exhaustive and a new key fails to compile until it is
/// sorted. This exists so the audit tests can iterate.
pub const ALL: &[Key] = &[
    Key::Reads,
    Key::Misses,
    Key::BytesClass,
    Key::Sent,
    Key::Received,
    Key::CountBucket,
    Key::OffsetMs,
    Key::Attempts,
    Key::Owed,
    Key::Effects,
    Key::Ops,
    Key::Awaiting,
    Key::ReadBack,
    Key::Stranded,
    Key::DroppedMsgs,
    Key::BytesOut,
    Key::BytesIn,
    Key::Ambiguous,
    Key::IdWidthGiven,
    Key::IdWidthWanted,
    Key::RtoMs,
    Key::SampleMs,
    Key::ArmedAtMs,
    Key::ReArmedAtMs,
    Key::StatusClass,
    Key::RecordReads,
    Key::RecordWrites,
    Key::RecordFails,
    Key::RecordBytesRead,
    Key::RecordBytesWritten,
    Key::Said,
];

/// Which of the three audit classes a [`Key`] falls in.
///
/// The audit sorts three ways, not two, and [`class`] performs the sort as an
/// exhaustive match — so a key cannot be added without someone deciding which
/// class it is in. That is the whole mechanism: the first quota counter is
/// stopped at compile time by a missing match arm, instead of looking like an
/// ordinary event and quietly becoming one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    /// A property of the DATA, computable without running anything.
    ///
    /// Never a key: it is DELETED, because a stored copy of a derivable fact
    /// is a second answer that can disagree with the first.
    Derivable,
    /// A property of THIS EXECUTION, and nothing else reads it.
    ///
    /// The only class this crate carries. Nobody profits from a wrong latency
    /// figure that steers nothing (ARCHITECTURE §5), so it needs no receipt.
    Diagnostic,
    /// A fact that could PAY or STEER a choice.
    ///
    /// Never a key, and never a self-observation: see [`PAY_OR_STEER`].
    PayOrSteer,
}

/// Sort a key into its audit class.
///
/// Exhaustive on purpose — no wildcard arm. Adding a [`Key`] without sorting
/// it is a compile error, which is the point.
pub const fn class(k: Key) -> Class {
    match k {
        // The negative controls; see their docs on [`Key`].
        #[cfg(test)]
        Key::SampleQuotaSpent => Class::PayOrSteer,
        #[cfg(test)]
        Key::SampleDepth => Class::Derivable,
        Key::Reads => Class::Diagnostic,
        Key::Misses => Class::Diagnostic,
        Key::BytesClass => Class::Diagnostic,
        Key::Sent => Class::Diagnostic,
        Key::Received => Class::Diagnostic,
        Key::CountBucket => Class::Diagnostic,
        Key::OffsetMs => Class::Diagnostic,
        Key::Attempts => Class::Diagnostic,
        Key::Owed => Class::Diagnostic,
        Key::Effects => Class::Diagnostic,
        Key::Ops => Class::Diagnostic,
        Key::Awaiting => Class::Diagnostic,
        Key::ReadBack => Class::Diagnostic,
        Key::Stranded => Class::Diagnostic,
        Key::DroppedMsgs => Class::Diagnostic,
        Key::BytesOut => Class::Diagnostic,
        Key::BytesIn => Class::Diagnostic,
        Key::Ambiguous => Class::Diagnostic,
        // A property of one read. Nothing reads it back and nothing steers on
        // it, so it needs no receipt.
        Key::IdWidthGiven => Class::Diagnostic,
        Key::IdWidthWanted => Class::Diagnostic,
        // The page's retry clock: what one sender used and measured, in one
        // execution. Nothing reads it back and nothing steers on it.
        Key::RtoMs => Class::Diagnostic,
        Key::SampleMs => Class::Diagnostic,
        Key::ArmedAtMs => Class::Diagnostic,
        Key::ReArmedAtMs => Class::Diagnostic,
        // How one fetch round ended: one loader, one execution.
        Key::StatusClass => Class::Diagnostic,
        // What one page did to one data domain in one window: diagnostics for support, never a meter (§14: bytes
        // moved gate a tier only through counterparty receipts).
        Key::RecordReads => Class::Diagnostic,
        Key::RecordWrites => Class::Diagnostic,
        Key::RecordFails => Class::Diagnostic,
        Key::RecordBytesRead => Class::Diagnostic,
        Key::RecordBytesWritten => Class::Diagnostic,
        Key::Said => Class::Diagnostic,
    }
}

/// The grain a PUBLISHED key leaves the device at (craftworks-docs OBSERVABILITY §2.3): never the ring's own.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Grain {
    /// A [`Bucket`]: a count of the person's activity is never exact once it leaves.
    Bucket,
    /// A padded [`SizeClass`] (powers of two): a volume is never exact once it leaves.
    SizeClass,
    /// Whole seconds from the WINDOW's start ([`PUBLISHED_OFFSET_GRAIN_MS`]), never the recording's: windows can't be
    /// aligned below their key's minute.
    Seconds,
    /// A value of a closed vocabulary enum (its code), which carries nothing a person wrote.
    Enum,
}

/// The coarsest-allowed offset grain once published: one second.
pub const PUBLISHED_OFFSET_GRAIN_MS: u64 = 1_000;

/// May a key LEAVE the device, through the one publish filter ([`publish`](crate::publish::publish))?
///
/// THE LEAK TABLE, in code (OBSERVABILITY §2.2): each published key states, in the same line that publishes it, what
/// it tells anyone who reads the person's (public, for now) observation tree. Every change to this table, to a grain,
/// or to the filter is reviewed as PRIVACY-CRITICAL, with its leak line in the PR body.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Publish {
    /// Never leaves the ring. The default.
    Local,
    /// Leaves, at `grain`, and says `leak`.
    Published { grain: Grain, leak: &'static str },
}

/// The leak line of the one field of a record that is not a key: the build header (the SDK wasm sha the page RAN,
/// the app's version), carried by a page's first window.
pub const HEADER_LEAK: &str = "which SDK build and app version the person ran";

/// The leak line of an operation's ending ([`Outcome`]): only a NON-Ok ending is published (an Ok operation's events
/// stay local -- their count would be the page's exact activity).
pub const OUTCOME_LEAK: &str = "that a failure of that class happened in that minute";

/// The leak line of an operation still UNANSWERED at a window's end (a stall), published with its own counters.
pub const UNANSWERED_LEAK: &str = "that an operation was still unanswered at the minute's end";

/// Sort a key into [`Publish`]: LOCAL unless listed. Exhaustive on purpose -- no wildcard arm -- so a new key is a
/// compile error until someone decides whether it may leave, the way [`class`] makes them decide what it is.
pub const fn publish(k: Key) -> Publish {
    const DOMAIN_COUNT: &str = "reveals which of an app's collections a person used, per minute, at bucket grain; the collection's name is public in the app's schema";
    const DOMAIN_BYTES: &str = "reveals the VOLUME class moved per collection per minute: a large class in a media collection says a file was stored or viewed (§5 leak 2, 'metering precision is leak precision')";
    match k {
        #[cfg(test)]
        Key::SampleQuotaSpent | Key::SampleDepth => Publish::Local,
        Key::RecordReads | Key::RecordWrites | Key::RecordFails => Publish::Published {
            grain: Grain::Bucket,
            leak: DOMAIN_COUNT,
        },
        Key::RecordBytesRead | Key::RecordBytesWritten => Publish::Published {
            grain: Grain::SizeClass,
            leak: DOMAIN_BYTES,
        },
        Key::Attempts => Publish::Published {
            grain: Grain::Bucket,
            leak: "how many times an operation was tried in that minute, at bucket grain",
        },
        Key::StatusClass => Publish::Published {
            grain: Grain::Enum,
            leak: OUTCOME_LEAK,
        },
        Key::OffsetMs => Publish::Published {
            grain: Grain::Seconds,
            leak: "when within the minute an operation ran, to the second",
        },
        Key::Reads
        | Key::Misses
        | Key::BytesClass
        | Key::Sent
        | Key::Received
        | Key::CountBucket
        | Key::Owed
        | Key::Effects
        | Key::Ops
        | Key::Awaiting
        | Key::ReadBack
        | Key::Stranded
        | Key::DroppedMsgs
        | Key::BytesOut
        | Key::BytesIn
        | Key::Ambiguous
        | Key::IdWidthGiven
        | Key::IdWidthWanted
        | Key::RtoMs
        | Key::SampleMs
        | Key::ArmedAtMs
        | Key::ReArmedAtMs
        | Key::Said => Publish::Local,
    }
}

/// Why an inbound message was discarded.
///
/// Closed, and mirroring the protocol's own `Dropped` — a reason is a
/// vocabulary value, never a string. A free-text reason is the easiest place
/// for user data to arrive by accident, and this one crosses into a support
/// bundle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum DropReason {
    /// Not a message this build can read at all.
    Unparseable,
    /// Trailing bytes: the prefix parsed, and a prefix is not a message.
    TrailingBytes,
    /// Bigger than this build will decode.
    TooLarge,
    /// A response about something nobody asked for.
    Unexpected,
    /// A message kind this side has no use for.
    NotForUs,
    /// It parsed, and answered fewer items than were asked: the rest of the
    /// request is unanswered (a batched query's reply short of its request).
    ShortAnswer,
    /// A SECOND answer for a GET the sender re-asked once the node's own GET was over (craftworks-sdk#447: the node
    /// does not dedupe client GETs, so a re-ask past the node's bound B may overlap a stalled earlier GET, and both can
    /// answer). The first answer ends the GET; this is the second, which arrives when nothing waits on the key and is
    /// dropped. A GET's answer names only its key, so it is counted as the answer that exists only if BOTH node GETs
    /// lived -- the observable for the known overlap (a header-then-stall GET runs to ~2 × B). Distinct from
    /// [`Unexpected`](DropReason::Unexpected): a count of these moves B.
    AnsweredAfterReask,
}

impl DropReason {
    /// A small stable integer, so a reason can ride in a `Counter` without a
    /// second event shape.
    pub const fn code(self) -> u64 {
        match self {
            DropReason::Unparseable => 0,
            DropReason::TrailingBytes => 1,
            DropReason::TooLarge => 2,
            DropReason::Unexpected => 3,
            DropReason::NotForUs => 4,
            DropReason::ShortAnswer => 5,
            DropReason::AnsweredAfterReask => 6,
        }
    }
}

/// How an operation ended.
///
/// `Refused(code)` carries a small stable integer, never a string: a reason
/// string is the easiest place for user data to arrive by accident.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    Ok,
    Missing,
    Refused(u16),
    Blocked,
    Timeout,
    /// Nobody needs it any more: SUPERSEDED by a later send of the same
    /// operation, cancelled, or stood in for. Not [`Blocked`](Outcome::Blocked)
    /// -- that says something stopped it -- and one word per meaning, or a
    /// reader of a dump misreads a superseded send as a stuck one (the
    /// architect on craftworks-sdk#407).
    Withdrawn,
}

/// How ONE round of a fetch ended ([`Key::StatusClass`]): closed, so no status
/// line, URL or body can ride in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StatusClass {
    /// A 2xx answer: the file.
    Ok,
    /// 404: this source does not hold it (yet).
    NotFound,
    /// A 5xx answer.
    ServerError,
    /// Any other HTTP status.
    OtherHttp,
    /// Cancelled on this side (a person's cancel, or a race that had enough).
    Abort,
    /// No HTTP answer at all: the request or its body failed.
    NetworkError,
}

impl StatusClass {
    /// Every class, for a generator or a test to visit.
    pub const ALL: [StatusClass; 6] = [
        StatusClass::Ok,
        StatusClass::NotFound,
        StatusClass::ServerError,
        StatusClass::OtherHttp,
        StatusClass::Abort,
        StatusClass::NetworkError,
    ];

    /// A small stable integer (a [`Key::StatusClass`] value). Pinned by a
    /// test: a support bundle is decoded long after it was written.
    pub const fn code(self) -> u64 {
        match self {
            StatusClass::Ok => 0,
            StatusClass::NotFound => 1,
            StatusClass::ServerError => 2,
            StatusClass::OtherHttp => 3,
            StatusClass::Abort => 4,
            StatusClass::NetworkError => 5,
        }
    }

    /// The class of an HTTP status: the first [`HTTP_CLASSES`] range that holds
    /// it, else [`OtherHttp`](StatusClass::OtherHttp). Computed from the table
    /// so another language's recorder is GENERATED from the same rule.
    pub const fn of_http(status: u16) -> StatusClass {
        let mut i = 0;
        while i < HTTP_CLASSES.len() {
            let (lo, hi, class) = HTTP_CLASSES[i];
            if status >= lo && status <= hi {
                return class;
            }
            i += 1;
        }
        StatusClass::OtherHttp
    }
}

/// THE HTTP classification as data: `(lowest, highest, class)`, first match
/// wins, anything else is [`StatusClass::OtherHttp`].
pub const HTTP_CLASSES: [(u16, u16, StatusClass); 3] = [
    (200, 299, StatusClass::Ok),
    (404, 404, StatusClass::NotFound),
    (500, 599, StatusClass::ServerError),
];

/// Which way an [`Edge`](crate::Event::Edge) points.
///
/// The pair is what makes OUTSTANDING a subtraction rather than a consumer-side
/// guess — the distinction that cost four harness runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Request,
    Response,
}

/// A size reported as the class it falls in, never exactly.
///
/// The exact length of sealed bytes is a property of the plaintext, so a probe
/// that ships in production must not carry it. The classes are the ones the
/// storage layer already pads to, so this loses nothing a reader had.
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum SizeClass {
    Empty,
    UpTo1K,
    UpTo4K,
    UpTo16K,
    UpTo64K,
    UpTo256K,
    Over256K,
}

impl SizeClass {
    pub const fn of(bytes: usize) -> SizeClass {
        match bytes {
            0 => SizeClass::Empty,
            1..=1024 => SizeClass::UpTo1K,
            1025..=4096 => SizeClass::UpTo4K,
            4097..=16384 => SizeClass::UpTo16K,
            16385..=65536 => SizeClass::UpTo64K,
            65537..=262144 => SizeClass::UpTo256K,
            _ => SizeClass::Over256K,
        }
    }
}

/// A count reported as the bucket it falls in.
///
/// "This person has 3 contacts" is user data; "this person has between 1 and 9"
/// is what an operator needs to read a trace.
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum Bucket {
    Zero,
    One,
    UpTo9,
    UpTo99,
    UpTo999,
    Over999,
}

impl Bucket {
    pub const fn of(n: u64) -> Bucket {
        match n {
            0 => Bucket::Zero,
            1 => Bucket::One,
            2..=9 => Bucket::UpTo9,
            10..=99 => Bucket::UpTo99,
            100..=999 => Bucket::UpTo999,
            _ => Bucket::Over999,
        }
    }
}

/// Milliseconds since the recording started, coarsened.
///
/// Not a clock: a wall-clock timestamp is both unavailable in a delegate (F32)
/// and a correlation handle across recordings. A coarse offset says how long
/// something took without saying when it happened.
pub const fn coarsen_ms(ms: u64) -> u64 {
    let mut i = 0;
    while i < COARSEN_BANDS.len() {
        let (below, grain) = COARSEN_BANDS[i];
        if ms < below {
            return (ms / grain) * grain;
        }
        i += 1;
    }
    (ms / COARSEN_LAST_GRAIN) * COARSEN_LAST_GRAIN
}

/// THE coarsening rule as data: `(below this many ms, round down to this
/// grain)`, in order, then [`COARSEN_LAST_GRAIN`] for everything longer. Data
/// so a recorder in another language is GENERATED from it rather than
/// re-implemented by hand (the SDK's loader, craftworks-sdk).
pub const COARSEN_BANDS: [(u64, u64); 2] = [(1_000, 10), (60_000, 100)];
/// The grain past the last band. No ceiling: a 1.8e12 ms value stays itself.
pub const COARSEN_LAST_GRAIN: u64 = 1_000;

#[cfg(test)]
mod audit {
    use super::*;

    /// What class each key is in, stated ONE BY ONE.
    ///
    /// A test that only asserts "no key is in the forbidden class" is passed by
    /// a classifier that returns a constant, and by an edit that later flattens
    /// one. This table says what the answer IS, so a wrong answer is a failing
    /// test rather than a silent reclassification.
    ///
    /// **Every real key is `Diagnostic` today, and that is a fact about this
    /// crate rather than a shortcut.** A key here reports a property of one
    /// EXECUTION, nothing else reads it, and nobody profits from a wrong
    /// latency figure that steers nothing (ARCHITECTURE §5) — so none of them
    /// needs a receipt. The day one of them is not diagnostic, this table
    /// changes deliberately, in the same commit as the key.
    const SORTED: &[(Key, Class)] = &[
        (Key::Reads, Class::Diagnostic),
        (Key::Misses, Class::Diagnostic),
        (Key::BytesClass, Class::Diagnostic),
        (Key::Sent, Class::Diagnostic),
        (Key::Received, Class::Diagnostic),
        (Key::CountBucket, Class::Diagnostic),
        (Key::OffsetMs, Class::Diagnostic),
        (Key::Attempts, Class::Diagnostic),
        (Key::Owed, Class::Diagnostic),
        (Key::Effects, Class::Diagnostic),
        (Key::Ops, Class::Diagnostic),
        (Key::Awaiting, Class::Diagnostic),
        (Key::ReadBack, Class::Diagnostic),
        (Key::Stranded, Class::Diagnostic),
        (Key::DroppedMsgs, Class::Diagnostic),
        (Key::BytesOut, Class::Diagnostic),
        (Key::BytesIn, Class::Diagnostic),
        (Key::Ambiguous, Class::Diagnostic),
        (Key::IdWidthGiven, Class::Diagnostic),
        (Key::IdWidthWanted, Class::Diagnostic),
        (Key::RtoMs, Class::Diagnostic),
        (Key::SampleMs, Class::Diagnostic),
        (Key::ArmedAtMs, Class::Diagnostic),
        (Key::ReArmedAtMs, Class::Diagnostic),
        (Key::StatusClass, Class::Diagnostic),
        (Key::RecordReads, Class::Diagnostic),
        (Key::RecordWrites, Class::Diagnostic),
        (Key::RecordFails, Class::Diagnostic),
        (Key::RecordBytesRead, Class::Diagnostic),
        (Key::RecordBytesWritten, Class::Diagnostic),
        (Key::Said, Class::Diagnostic),
    ];

    /// THE CONTROL: the classifier can tell the three classes apart.
    ///
    /// Without this, every other test in this module passes against
    /// `fn class(_) -> Class { Class::Diagnostic }`, because every real key is
    /// diagnostic. The samples are `cfg(test)` keys that cannot ship and are
    /// never emitted; they exist so the sort is OBSERVABLE.
    #[test]
    fn the_classifier_tells_the_three_classes_apart() {
        assert_eq!(class(Key::SampleQuotaSpent), Class::PayOrSteer);
        assert_eq!(class(Key::SampleDepth), Class::Derivable);
        assert_eq!(class(Key::Reads), Class::Diagnostic);

        // Three distinct answers from the real function. A classifier that
        // returned a constant would fail here first, and every other test in
        // this module would still pass — which is why this one exists.
        let (pay, derivable, diagnostic) = (
            class(Key::SampleQuotaSpent),
            class(Key::SampleDepth),
            class(Key::Reads),
        );
        assert!(
            pay != derivable && derivable != diagnostic && pay != diagnostic,
            "the classifier gave fewer than three distinct answers \
             ({pay:?}, {derivable:?}, {diagnostic:?}), so it is \
             indistinguishable from a constant and proves nothing about any key"
        );
    }

    /// Every key's class, one by one.
    #[test]
    fn each_key_is_in_the_class_the_table_states() {
        assert_eq!(
            SORTED.len(),
            ALL.len(),
            "a key was added without being stated in SORTED"
        );
        for (k, want) in SORTED {
            assert_eq!(
                class(*k),
                *want,
                "{k:?} is not in the class the audit states it is in"
            );
        }
        for k in ALL {
            assert!(
                SORTED.iter().any(|(s, _)| s == k),
                "{k:?} is emitted but the audit never states its class"
            );
        }
    }

    /// No key may be in the PAY-OR-STEER class.
    ///
    /// ARCHITECTURE §5: "the subject never attests to itself". A fact that can
    /// pay or steer a choice is a receipt the COUNTERPARTY signs into its own
    /// tree — double-entry, where a balance moves only where both halves
    /// agree (§14). This crate emits self-observations, so a key here would be
    /// the half that is worth nothing alone, published as if it were the whole
    /// record. It does not belong in this crate at all; it belongs in the
    /// record the counterparty writes.
    #[test]
    fn nothing_that_could_pay_or_steer_is_self_observed() {
        assert!(
            PAY_OR_STEER.is_empty(),
            "a key that can pay or steer is not a self-observation: it is a \
             receipt the counterparty signs into its own tree (ARCHITECTURE \
             §5, §14). Move it there — do not emit it from here."
        );
        for k in ALL {
            assert_ne!(
                class(*k),
                Class::PayOrSteer,
                "{k:?} was sorted PayOrSteer but is still emitted as a \
                 self-observation"
            );
        }
    }

    /// No key reports a derivable fact.
    #[test]
    fn nothing_derivable_is_stored() {
        for k in ALL {
            assert_ne!(
                class(*k),
                Class::Derivable,
                "{k:?} is derivable, so it is a second answer that can \
                 disagree with the first — delete it, do not maintain it"
            );
        }
    }
}
