//! THE PAGE'S OPENING, AS TWO TABLES (sdk#484; craftworks-docs `docs/design/OPENING.md`, the architect's OK on draft
//! 2). Each machine is ONE enum whose cases carry their own data, and ONE pure transition function: `step` returns the
//! next state, the effects `PageIo` carries out, and the CELL the event landed in. `PageIo` changes a machine's state
//! only by applying a step (the one-writer rule; a source test holds it). No `_ =>` over a state or an event: a new
//! case fails to compile until its row or column is written.
//!
//! - [`Opening`]: the page's standing with its node's SIGNER (was `asking`, `claimed`, `asked`, `signer_registered`,
//!   `first`, `needs_key`, `minted`, `provisioned`, `refused`).
//! - [`HeadKnown`]: does the HEAD exist (was `register_seen`, `signer_has_record`, `head_failed_pending`).
//!
//! An impossible cell (a) is never a panic (#450): the state is unchanged and the cell is COUNTED, so the model test
//! asserts the count stays 0 over sequences a real node could produce.

// NO CATCH-ALL over a state or an event (CLAUDE.md, engineer2's #485): a new case must fail to compile until its row or
// column is written. The two lints catch `_ =>`, a named catch-all (`other =>`), and a `_` standing for ONE remaining
// variant (only the second catches that); the control plants each.
#![deny(clippy::wildcard_enum_match_arm, clippy::match_wildcard_for_single_variants)]

use crate::Asked;
use freenet_stdlib::prelude::DelegateContainer;

/// The signer's FIRST request: the Register query (`begin`, `ask`) or the Provision (`provision`, `provision_with`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum First {
    Query,
    Provision(Vec<u8>),
}

/// How an OWNER's opening started: begun (`begin`, `provision`), or an asked page CLAIMED after its signer answered --
/// the answer stays what it was (`PageIo::asked`), and it decides whether the tree exists yet (`no_tree_yet`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Via {
    Begun,
    Claimed(Asked),
}

/// Machine 1: the page's standing with its node's signer. O1: a page is exactly one of a reader, asking, or an owner
/// opening/open/refused. O2: Refused and Open are one case each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Opening {
    /// Nothing asked, nothing registered.
    New,
    /// The signer's registration is in flight; `then` goes once it is answered (#260: sent together, 7 in 12 were
    /// answered EMPTY and the page waited for ever).
    Registering { then: First, via: Via },
    /// Registered; "which Register do you sign for?" is in flight.
    Querying { via: Via },
    /// The signer holds no key: the caller mints one (`provision_with`). O4: the only state it acts in.
    NeedsKey { via: Via },
    /// The key is sent. `minted`: this page minted it, so a `KeyAlreadyProvisioned` falls back to asking (sdk#343); a
    /// GIVEN key is refused.
    Provisioning { key: Vec<u8>, minted: bool, via: Via },
    /// This page's tree: the signer signs for the Register page-io names (`register_id`, its one owner).
    Open { via: Via },
    /// The signer or the node refused the opening, in its words.
    Refused { why: String, via: Via },
    /// `ask()`: the query sent WITHOUT registering; `None` until answered.
    Asking { answer: Option<Asked> },
    /// A view of someone else's head (sdk#239): installs, mints and provisions nothing (O5).
    Reader,
}

/// What moves [`Opening`]: the API's calls, the node's answers, and the socket.
pub(crate) enum OpenEvent {
    Begin(DelegateContainer),
    Provide(DelegateContainer, Vec<u8>),
    Ask,
    Claim(DelegateContainer),
    /// A minted key and the Register params it names.
    ProvisionWith(Vec<u8>, Vec<u8>),
    /// An EMPTY response from the signer's delegate (`wire` maps it to `Ack(Registered)`): the registration's answer
    /// while one is out; to an ASKING page, the node's own "no such signer here" (0.2.136); otherwise nothing (#260).
    EmptyAck(String),
    /// "No such delegate" naming the signer (0.2.137+, #5729): to an asking page the answer; otherwise nothing.
    DelegateMissing(String),
    /// The Register query's answer: the params it signs under and their instance id, or none (no key).
    Register(Option<(Vec<u8>, [u8; 32])>),
    QueryRefused(String),
    Provisioned,
    /// `Refused(KeyAlreadyProvisioned)` to a provision.
    KeyAlreadyHere,
    ProvisionRefused(String),
    /// A keyless `Incoming::Refused`: while the first request is out, the node refusing IT.
    NodeRefused(String),
    /// The socket was replaced (`PageIo::reconnected`): the node dropped the client's subscriptions and push routing,
    /// never the signer's install (freenet v0.2.138 client_events.rs 1712-1730, 1871-1925; probe live-reconnect).
    Reconnected,
}

impl OpenEvent {
    /// Every event kind, for the cross-product and model tests (one sample each; the data never changes the cell).
    #[cfg(test)]
    pub(crate) fn samples(c: &DelegateContainer) -> Vec<OpenEvent> {
        vec![
            OpenEvent::Begin(c.clone()),
            OpenEvent::Provide(c.clone(), vec![7]),
            OpenEvent::Ask,
            OpenEvent::Claim(c.clone()),
            OpenEvent::ProvisionWith(vec![7], vec![8]),
            OpenEvent::EmptyAck("empty".into()),
            OpenEvent::DelegateMissing("missing".into()),
            OpenEvent::Register(Some((vec![9], [9; 32]))),
            OpenEvent::Register(None),
            OpenEvent::QueryRefused("q".into()),
            OpenEvent::Provisioned,
            OpenEvent::KeyAlreadyHere,
            OpenEvent::ProvisionRefused("p".into()),
            OpenEvent::NodeRefused("n".into()),
            OpenEvent::Reconnected,
        ]
    }
}

/// What `PageIo` carries out for a step, in order (after the state is set).
pub(crate) enum OpenEffect {
    /// Hold the signer's container and send its registration (`Ext::RegisterSigner`).
    Register(DelegateContainer),
    /// The registration's deadline ends and the first request goes.
    RegistrationAnswered,
    /// The (new) first request goes (`Ext::SignerFirst`).
    SendFirst,
    /// The first request was answered by something that is not a signer answer (a node's refusal, "no signer"):
    /// its deadline ends.
    FirstAnswered,
    /// Name the Register this page's head lives in.
    SetRegister(Vec<u8>),
    /// Tell the server the signer signs for the Register.
    SignerProvisioned,
    /// Tell the engine whether this page can sign now.
    StepCanSign,
    /// The caller's error, or the signer's refusal, NAMED.
    Unusable(String),
}

/// Where an event landed: the table's cell kind. The cross-product test counts them against OPENING.md's counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Cell {
    /// The state changed.
    Transition,
    /// Nothing changes, and the page's sender re-sends what is out (the Reconnected column).
    Stays,
    /// Nothing for this state to do (an API call that means nothing here, or a non-answer re-sent on the RTO).
    Ignored,
    /// (i) A late or duplicate answer to a request already answered (a re-send's twin).
    Late,
    /// (r) A keyless refusal that names no op of this state: counted by the caller, ends nothing (sdk#433).
    Counted,
    /// The caller's error, named in `unusable`; nothing changes.
    RefusedCall,
    /// `claim`'s answer without a change: true if this page was claimed, else false.
    ClaimAnswer,
    /// (a) An answer to a request this state never sent: counted, state unchanged, never a panic.
    Impossible,
}

pub(crate) struct Step {
    pub next: Option<Opening>,
    pub effects: Vec<OpenEffect>,
    pub cell: Cell,
    /// `claim`'s answer.
    pub claimed: bool,
}

fn go(next: Opening, effects: Vec<OpenEffect>) -> Step {
    Step { next: Some(next), effects, cell: Cell::Transition, claimed: false }
}
fn stay(cell: Cell) -> Step {
    Step { next: None, effects: Vec::new(), cell, claimed: false }
}
/// The signer refused this page's provision, in its words: opening ends, named.
fn provision_refused(via: &Via, why: &str) -> Step {
    let why = format!("the signer refused provisioning: {why}");
    go(Opening::Refused { why: why.clone(), via: via.clone() }, vec![OpenEffect::Unusable(why)])
}

fn refuse(why: &str) -> Step {
    Step { next: None, effects: vec![OpenEffect::Unusable(why.into())], cell: Cell::RefusedCall, claimed: false }
}

const NOT_ASKED_FOR_A_KEY: &str = "provision_with: the signer was not asked, or already holds a key";
const READER_OPENS_NOTHING: &str = "read-only: a reader opens nothing — it registers, asks, mints and provisions nothing";
const READER_PROVISIONS_NOTHING: &str = "read-only: provisioning refused — a reader installs nothing";

impl Opening {
    /// How this owner's opening started; `None` for a page that is not an owner (new, asking, a reader).
    pub(crate) fn via(&self) -> Option<&Via> {
        match self {
            Opening::Registering { via, .. }
            | Opening::Querying { via }
            | Opening::NeedsKey { via }
            | Opening::Provisioning { via, .. }
            | Opening::Open { via }
            | Opening::Refused { via, .. } => Some(via),
            Opening::New | Opening::Asking { .. } | Opening::Reader => None,
        }
    }

    /// The first request, if this state has one to send (framed by `pump` when the page's sender sends it). O3: out
    /// in exactly Querying, Provisioning and Asking{None}; Registering HOLDS its `then` until the registration is
    /// answered.
    pub(crate) fn first(&self) -> Option<First> {
        match self {
            Opening::Querying { .. } | Opening::Asking { answer: None } => Some(First::Query),
            Opening::Provisioning { key, .. } => Some(First::Provision(key.clone())),
            Opening::Registering { then, .. } => Some(then.clone()),
            Opening::New | Opening::NeedsKey { .. } | Opening::Open { .. } | Opening::Refused { .. } | Opening::Asking { answer: Some(_) } | Opening::Reader => None,
        }
    }

    /// A provision's answer where no provision is out: the twin of one already answered where a provision was ever
    /// sent (an owner's states), impossible where none ever is (new, asking, a reader).
    fn no_provision_out(&self) -> Step {
        match self {
            Opening::Registering { .. } | Opening::Querying { .. } | Opening::NeedsKey { .. } | Opening::Open { .. } | Opening::Refused { .. } | Opening::Provisioning { .. } => stay(Cell::Late),
            Opening::New | Opening::Asking { .. } | Opening::Reader => stay(Cell::Impossible),
        }
    }

    /// THE TRANSITION FUNCTION (OPENING.md, Machine 1). Pure.
    pub(crate) fn step(&self, ev: OpenEvent) -> Step {
        use Opening as S;
        use OpenEvent as E;
        use OpenEffect as F;
        match ev {
            // ---- the API ----
            E::Begin(c) => match self {
                S::New => go(S::Registering { then: First::Query, via: Via::Begun }, vec![F::Register(c)]),
                S::Reader => refuse(READER_OPENS_NOTHING),
                S::Registering { .. } | S::Querying { .. } | S::NeedsKey { .. } | S::Provisioning { .. } | S::Open { .. } | S::Refused { .. } | S::Asking { .. } => stay(Cell::Ignored),
            },
            E::Provide(c, key) => match self {
                S::New => go(S::Registering { then: First::Provision(key), via: Via::Begun }, vec![F::Register(c)]),
                S::Reader => refuse(READER_PROVISIONS_NOTHING),
                S::Registering { .. } | S::Querying { .. } | S::NeedsKey { .. } | S::Provisioning { .. } | S::Open { .. } | S::Refused { .. } | S::Asking { .. } => stay(Cell::Ignored),
            },
            E::Ask => match self {
                S::New => go(S::Asking { answer: None }, vec![F::SendFirst]),
                S::Reader => refuse(READER_OPENS_NOTHING),
                S::Registering { .. } | S::Querying { .. } | S::NeedsKey { .. } | S::Provisioning { .. } | S::Open { .. } | S::Refused { .. } | S::Asking { .. } => stay(Cell::Ignored),
            },
            E::Claim(c) => match self {
                S::Asking { answer: Some(a) } => {
                    let via = Via::Claimed(a.clone());
                    let mut s = match a {
                        Asked::Register(_) => go(S::Open { via }, vec![F::SignerProvisioned]),
                        Asked::NoKey => go(S::NeedsKey { via }, vec![]),
                        // No signer here: registered, then asked -- the same route as `begin`'s.
                        Asked::NoSigner(_) => go(S::Registering { then: First::Query, via }, vec![F::Register(c)]),
                        Asked::Refused(_) => return Step { claimed: false, ..stay(Cell::ClaimAnswer) },
                    };
                    s.claimed = true;
                    s
                }
                S::Asking { answer: None } | S::New | S::Reader => Step { claimed: false, ..stay(Cell::ClaimAnswer) },
                S::Registering { .. } | S::Querying { .. } | S::NeedsKey { .. } | S::Provisioning { .. } | S::Open { .. } | S::Refused { .. } => {
                    Step { claimed: matches!(self.via(), Some(Via::Claimed(_))), ..stay(Cell::ClaimAnswer) }
                }
            },
            E::ProvisionWith(key, params) => match self {
                S::NeedsKey { via } => go(S::Provisioning { key, minted: true, via: via.clone() }, vec![F::SetRegister(params), F::SendFirst]),
                S::Reader => refuse(READER_OPENS_NOTHING),
                S::New | S::Registering { .. } | S::Querying { .. } | S::Provisioning { .. } | S::Open { .. } | S::Refused { .. } | S::Asking { .. } => refuse(NOT_ASKED_FOR_A_KEY),
            },
            // ---- the node ----
            E::EmptyAck(said) => match self {
                S::Registering { then, via } => {
                    let next = match then {
                        First::Query => S::Querying { via: via.clone() },
                        First::Provision(key) => S::Provisioning { key: key.clone(), minted: false, via: via.clone() },
                    };
                    go(next, vec![F::RegistrationAnswered])
                }
                // An asking page registered nothing: the node's own "no such signer here" (0.2.136).
                S::Asking { answer: None } => go(S::Asking { answer: Some(Asked::NoSigner(said)) }, vec![F::FirstAnswered, F::StepCanSign]),
                // After registering, an EMPTY to the first request arrived before the registration took (#260): not
                // an answer, re-sent on the RTO.
                S::Querying { .. } | S::Provisioning { .. } => stay(Cell::Ignored),
                S::New | S::NeedsKey { .. } | S::Open { .. } | S::Refused { .. } | S::Asking { answer: Some(_) } => stay(Cell::Late),
                // A reader owns no signer answer (`owns`).
                S::Reader => stay(Cell::Impossible),
            },
            E::DelegateMissing(said) => match self {
                S::Asking { answer: None } => go(S::Asking { answer: Some(Asked::NoSigner(said)) }, vec![F::FirstAnswered, F::StepCanSign]),
                // The registration is out, or the request came before it took (#260): re-sent on the RTO.
                S::Registering { .. } | S::Querying { .. } | S::Provisioning { .. } => stay(Cell::Ignored),
                S::New | S::NeedsKey { .. } | S::Open { .. } | S::Refused { .. } | S::Asking { answer: Some(_) } => stay(Cell::Late),
                S::Reader => stay(Cell::Impossible),
            },
            E::Register(named) => match self {
                S::Querying { via } => match named {
                    Some((params, _)) => go(S::Open { via: via.clone() }, vec![F::SetRegister(params), F::SignerProvisioned]),
                    None => go(S::NeedsKey { via: via.clone() }, vec![]),
                },
                S::Asking { answer: None } => match named {
                    Some((params, id)) => go(S::Asking { answer: Some(Asked::Register(id)) }, vec![F::SetRegister(params), F::StepCanSign]),
                    None => go(S::Asking { answer: Some(Asked::NoKey) }, vec![F::StepCanSign]),
                },
                // The query was re-sent on its RTO and answered twice: the twin of an answer already taken.
                S::Registering { .. } | S::NeedsKey { .. } | S::Provisioning { .. } | S::Open { .. } | S::Refused { .. } | S::Asking { answer: Some(_) } => stay(Cell::Late),
                S::New | S::Reader => stay(Cell::Impossible),
            },
            E::QueryRefused(why) => match self {
                S::Querying { via } => {
                    let said = format!("the signer refused to say which Register it signs for: {why}");
                    go(S::Refused { why: said, via: via.clone() }, vec![F::Unusable(format!("the signer refused the register query: {why}"))])
                }
                S::Asking { answer: None } => go(S::Asking { answer: Some(Asked::Refused(format!("the signer refused to say which Register it signs for: {why}"))) }, vec![]),
                S::Registering { .. } | S::NeedsKey { .. } | S::Provisioning { .. } | S::Open { .. } | S::Refused { .. } | S::Asking { answer: Some(_) } => stay(Cell::Late),
                S::New | S::Reader => stay(Cell::Impossible),
            },
            E::Provisioned => match self {
                S::Provisioning { via, .. } => go(S::Open { via: via.clone() }, vec![F::SignerProvisioned]),
                S::New | S::Registering { .. } | S::Querying { .. } | S::NeedsKey { .. } | S::Open { .. } | S::Refused { .. } | S::Asking { .. } | S::Reader => self.no_provision_out(),
            },
            E::KeyAlreadyHere => match self {
                // Another page, told "no key" as this one was, provisioned first (sdk#343): ask again and open THAT
                // Register -- one identity per node (rule 15); the minted key is dropped.
                S::Provisioning { minted: true, via, .. } => go(S::Querying { via: via.clone() }, vec![F::SendFirst]),
                S::Provisioning { minted: false, via, .. } => provision_refused(via, "KeyAlreadyProvisioned"),
                S::New | S::Registering { .. } | S::Querying { .. } | S::NeedsKey { .. } | S::Open { .. } | S::Refused { .. } | S::Asking { .. } | S::Reader => self.no_provision_out(),
            },
            E::ProvisionRefused(why) => match self {
                S::Provisioning { via, .. } => provision_refused(via, &why),
                S::New | S::Registering { .. } | S::Querying { .. } | S::NeedsKey { .. } | S::Open { .. } | S::Refused { .. } | S::Asking { .. } | S::Reader => self.no_provision_out(),
            },
            E::NodeRefused(said) => match self {
                S::Querying { via } | S::Provisioning { via, .. } => go(S::Refused { why: format!("the node refused: {said}"), via: via.clone() }, vec![F::FirstAnswered]),
                S::Asking { answer: None } => go(S::Asking { answer: Some(Asked::Refused(format!("the node refused: {said}"))) }, vec![F::FirstAnswered]),
                // No first request is out (Registering HOLDS it): the refusal names nothing of this state.
                S::New | S::Registering { .. } | S::NeedsKey { .. } | S::Open { .. } | S::Refused { .. } | S::Asking { answer: Some(_) } | S::Reader => stay(Cell::Counted),
            },
            E::Reconnected => match self {
                // What is out stays out; the page's sender re-sends it on its RTO (rule 7). The install survives.
                S::Registering { .. } | S::Querying { .. } | S::Provisioning { .. } | S::Asking { answer: None } => stay(Cell::Stays),
                S::New | S::NeedsKey { .. } | S::Open { .. } | S::Refused { .. } | S::Asking { answer: Some(_) } | S::Reader => stay(Cell::Ignored),
            },
        }
    }
}

/// Machine 2: does the head exist? H1: "no head" (which lets a first commit CREATE the Register) is said ONLY when the
/// signer holds no record for it (sdk#175, F55, F56). H2: every record-query answer lands in a state that decides or
/// asks again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HeadKnown {
    Unknown,
    /// A NotFound came, and the signer's record query is out.
    AskingRecord,
    /// The Register was read or its PUT answered: it exists.
    Seen,
    /// The signer holds no record: "no head" is said.
    NoRecord,
    /// The signer SAID it holds one: a NotFound is F55's false one, silence.
    HasRecord,
    /// A READER's head: assumed to exist because someone named it; nobody said so.
    Named,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HeadEvent {
    /// A GET answered with the head.
    Got,
    /// Its PUT or UPDATE answered.
    PutAcked,
    /// An explicit NotFound for the head.
    NotFound,
    /// The record query's answer: has a record, has none, or neither.
    Record(Option<bool>),
    Reconnected,
}

impl HeadEvent {
    #[cfg(test)]
    pub(crate) const ALL: [HeadEvent; 7] =
        [HeadEvent::Got, HeadEvent::PutAcked, HeadEvent::NotFound, HeadEvent::Record(Some(false)), HeadEvent::Record(Some(true)), HeadEvent::Record(None), HeadEvent::Reconnected];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HeadEffect {
    /// Send the signer's record query (`ask_record`).
    AskRecord,
    /// Tell the page "no head" (its first commit creates the Register).
    NoHead,
    /// "No head" was said and the signer now holds a record (another tab or device signed first): counted.
    Contradicted,
}

impl HeadKnown {
    #[cfg(test)]
    pub(crate) const ALL: [HeadKnown; 6] = [HeadKnown::Unknown, HeadKnown::AskingRecord, HeadKnown::Seen, HeadKnown::NoRecord, HeadKnown::HasRecord, HeadKnown::Named];

    /// Does the Register exist on the node (a head goes out as an UPDATE, not the PUT that creates it)?
    pub(crate) fn seen(self) -> bool {
        match self {
            HeadKnown::Seen => true,
            HeadKnown::Unknown | HeadKnown::AskingRecord | HeadKnown::NoRecord | HeadKnown::HasRecord | HeadKnown::Named => false,
        }
    }

    /// THE TRANSITION FUNCTION (OPENING.md, Machine 2). Pure: (next, effects, cell).
    pub(crate) fn step(self, ev: HeadEvent) -> (HeadKnown, Vec<HeadEffect>, Cell) {
        use HeadEffect as F;
        use HeadEvent as E;
        use HeadKnown as S;
        let stay = |c| (self, Vec::new(), c);
        match (self, ev) {
            (S::Seen, E::Got | E::PutAcked) => stay(Cell::Stays),
            (S::Unknown | S::AskingRecord | S::NoRecord | S::HasRecord | S::Named, E::Got) => (S::Seen, vec![], Cell::Transition),
            (S::Unknown | S::AskingRecord | S::NoRecord | S::HasRecord, E::PutAcked) => (S::Seen, vec![], Cell::Transition),
            (S::Named, E::PutAcked) => stay(Cell::Impossible),
            (S::Unknown, E::NotFound) => (S::AskingRecord, vec![F::AskRecord], Cell::Transition),
            // One record ask is out; the head read is re-sent on its RTO.
            (S::AskingRecord, E::NotFound) => stay(Cell::Stays),
            // The head exists (or someone named it, or the signer holds a record): F55's false NotFound, silence.
            (S::Seen | S::HasRecord | S::Named, E::NotFound) => stay(Cell::Stays),
            // "None" was a SNAPSHOT: another tab or device may have created the Register since. Ask the signer again
            // (a cheap local answer) rather than repeat a stale "no head" and create over an existing Register (H1;
            // the architect on sdk#499). The read is still answered: AskingRecord answers on the record's reply.
            (S::NoRecord, E::NotFound) => (S::AskingRecord, vec![F::AskRecord], Cell::Transition),
            (S::AskingRecord, E::Record(Some(false))) => (S::NoRecord, vec![F::NoHead], Cell::Transition),
            // D1: the record exists, so the NotFound was silence; nothing is pending.
            (S::AskingRecord, E::Record(Some(true))) => (S::HasRecord, vec![], Cell::Transition),
            // D2's cell: undecided, so the next NotFound asks again.
            (S::AskingRecord, E::Record(None)) => (S::Unknown, vec![], Cell::Transition),
            (S::NoRecord, E::Record(Some(true))) => (S::HasRecord, vec![F::Contradicted], Cell::Transition),
            (S::Seen | S::HasRecord, E::Record(_)) | (S::NoRecord, E::Record(Some(false) | None)) => stay(Cell::Late),
            // Asked once, then undecided (D2's cell): the first ask's twin may still arrive.
            (S::Unknown, E::Record(_)) => stay(Cell::Late),
            // A reader never asks the signer anything.
            (S::Named, E::Record(_)) => stay(Cell::Impossible),
            // The record query, if out, is re-sent on its RTO; the head exists or not whatever the socket did.
            (S::Unknown | S::AskingRecord | S::Seen | S::NoRecord | S::HasRecord | S::Named, E::Reconnected) => stay(Cell::Stays),
        }
    }
}

/// ONE WRITER, BY TYPE (CLAUDE.md "One writer, by type"; the architect on sdk#499): the page's opening lives in this
/// struct's PRIVATE field. It starts only as an owner's (`owner`) or a reader's (`reader`), and its one `&mut` method
/// is [`OpeningState::step`], which applies the table. No code outside this module can assign, clear or swap the
/// state: the compiler holds the rule a source scan could only approximate.
pub(crate) struct OpeningState(Opening);

impl OpeningState {
    /// A person's own page: `New`.
    pub(crate) fn owner() -> Self {
        OpeningState(Opening::New)
    }
    /// A view of someone else's head: `Reader`, for good.
    pub(crate) fn reader() -> Self {
        OpeningState(Opening::Reader)
    }
    /// The state, to read.
    pub(crate) fn get(&self) -> &Opening {
        &self.0
    }
    /// THE ONE WRITER: the table's step for `ev`, applied. Returns the step (its effects, its cell, `claim`'s answer).
    pub(crate) fn step(&mut self, ev: OpenEvent) -> Step {
        let mut step = self.0.step(ev);
        if let Some(next) = step.next.take() {
            self.0 = next;
        }
        step
    }
}

/// ONE WRITER, BY TYPE, for Machine 2 (as [`OpeningState`]).
pub(crate) struct HeadKnownState(HeadKnown);

impl HeadKnownState {
    /// An owner's head: `Unknown` until read or answered.
    pub(crate) fn unknown() -> Self {
        HeadKnownState(HeadKnown::Unknown)
    }
    /// A reader's head: `Named` (someone published it; nobody said it exists).
    pub(crate) fn named() -> Self {
        HeadKnownState(HeadKnown::Named)
    }
    pub(crate) fn get(&self) -> HeadKnown {
        self.0
    }
    /// THE ONE WRITER: the table's step for `ev`, applied. Returns its effects and its cell.
    pub(crate) fn step(&mut self, ev: HeadEvent) -> (Vec<HeadEffect>, Cell) {
        let (next, effects, cell) = self.0.step(ev);
        self.0 = next;
        (effects, cell)
    }
}

/// Machine 3: is THIS CONNECTION told of head moves? (sdk#490, OPENING.md Machine 3.) S1: `live_mode` says subscribed
/// exactly in `Subscribed`. S2: a head move DELIVERED ON THIS CONNECTION proves its subscription -- only this
/// connection's frames reach page-io (js/connection.js drops a replaced socket's, and a test holds it). S3: a replaced
/// socket ends the old subscription; the re-read with subscribe goes at once, so a reconnect lands in `Asked`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HeadSub {
    /// No head read with subscribe sent on this page yet.
    Unasked,
    /// Sent, not yet answered or proven.
    Asked,
    /// Answered (the read's `Got`, the node's `Subscribed` ack) or proven (a move delivered, S2).
    Subscribed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SubEvent {
    /// The head read WITH subscribe was framed.
    ReadSent,
    Got,
    NotFound,
    Refused,
    /// `Ack(Subscribed(register))`.
    SubscribedAck,
    /// A head move delivered on this connection.
    HeadChanged,
    Reconnected,
}

impl SubEvent {
    #[cfg(test)]
    pub(crate) const ALL: [SubEvent; 7] =
        [SubEvent::ReadSent, SubEvent::Got, SubEvent::NotFound, SubEvent::Refused, SubEvent::SubscribedAck, SubEvent::HeadChanged, SubEvent::Reconnected];
}

impl HeadSub {
    #[cfg(test)]
    pub(crate) const ALL: [HeadSub; 3] = [HeadSub::Unasked, HeadSub::Asked, HeadSub::Subscribed];

    /// THE TRANSITION FUNCTION (OPENING.md, Machine 3). Pure. The counters (moves delivered, failed reads) are counted
    /// by the caller; they decide nothing.
    pub(crate) fn step(self, ev: SubEvent) -> (HeadSub, Cell) {
        use HeadSub as S;
        use SubEvent as E;
        match (self, ev) {
            (S::Unasked, E::ReadSent) => (S::Asked, Cell::Transition),
            // Nothing was asked: no answer, and no subscription to deliver a move.
            (S::Unasked, E::Got | E::NotFound | E::Refused | E::SubscribedAck | E::HeadChanged) => (self, Cell::Impossible),
            (S::Unasked, E::Reconnected) => (self, Cell::Stays),
            (S::Asked, E::ReadSent | E::NotFound | E::Refused | E::Reconnected) => (self, Cell::Stays),
            // S2 (sdk#490's defect cell): a delivered move proves the subscription, as the read's answer does.
            (S::Asked, E::Got | E::SubscribedAck | E::HeadChanged) => (S::Subscribed, Cell::Transition),
            (S::Subscribed, E::ReadSent | E::Got | E::NotFound | E::Refused | E::SubscribedAck | E::HeadChanged) => (self, Cell::Stays),
            // S3: the node dropped this connection's subscription; the re-read is on its way.
            (S::Subscribed, E::Reconnected) => (S::Asked, Cell::Transition),
        }
    }
}

/// ONE WRITER, BY TYPE, for Machine 3 (as [`OpeningState`]).
pub(crate) struct HeadSubState(HeadSub);

impl HeadSubState {
    /// No head read sent yet.
    pub(crate) fn unasked() -> Self {
        HeadSubState(HeadSub::Unasked)
    }
    pub(crate) fn get(&self) -> HeadSub {
        self.0
    }
    /// THE ONE WRITER: the table's step for `ev`, applied. Returns its cell.
    pub(crate) fn step(&mut self, ev: SubEvent) -> Cell {
        let (next, cell) = self.0.step(ev);
        self.0 = next;
        cell
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn container() -> DelegateContainer {
        wire::delegate_from_code(b"opening table signer").0
    }

    /// One state per ROW of OPENING.md's table 1 (data chosen so each row's cells are the row's: Provisioning minted,
    /// Asking{Some} answered "no key").
    fn rows() -> Vec<(&'static str, Opening)> {
        let via = Via::Begun;
        vec![
            ("New", Opening::New),
            ("Registering{then}", Opening::Registering { then: First::Query, via: via.clone() }),
            ("Querying", Opening::Querying { via: via.clone() }),
            ("NeedsKey", Opening::NeedsKey { via: via.clone() }),
            ("Provisioning{minted}", Opening::Provisioning { key: vec![1], minted: true, via: via.clone() }),
            ("Open", Opening::Open { via: via.clone() }),
            ("Refused{why}", Opening::Refused { why: "w".into(), via }),
            ("Asking{None}", Opening::Asking { answer: None }),
            ("Asking{Some(a)}", Opening::Asking { answer: Some(Asked::NoKey) }),
            ("Reader", Opening::Reader),
        ]
    }

    const COLUMNS: [&str; 15] = [
        "Begin", "Provide", "Ask", "Claim", "ProvisionWith", "EmptyAck", "DelegateMissing", "Register(Some)", "Register(None)", "QueryRefused", "Provisioned",
        "KeyAlreadyHere", "ProvisionRefused", "NodeRefused", "Reconnected",
    ];

    fn word(s: &Step) -> String {
        match (s.cell, &s.next) {
            (Cell::Transition, Some(n)) => format!("→ {}", format!("{n:?}").split([' ', '{']).next().unwrap_or_default()),
            (Cell::ClaimAnswer, _) => if s.claimed { "true".into() } else { "false".into() },
            (c, _) => format!("{c:?}"),
        }
    }

    /// EVERY CELL IS DECIDED (the cross product), and the table's counts are pinned: a changed cell changes a count,
    /// and OPENING.md's table is this one (the test prints it, row by row).
    #[test]
    fn every_opening_cell_is_decided_and_the_counts_are_the_documents() {
        let mut counts = std::collections::BTreeMap::new();
        println!("| state \\ event | {} |", COLUMNS.join(" | "));
        for (name, state) in rows() {
            let cells: Vec<String> = OpenEvent::samples(&container())
                .into_iter()
                .map(|ev| {
                    let s = state.step(ev);
                    *counts.entry(s.cell).or_insert(0usize) += 1;
                    assert_eq!(s.next.is_some(), s.cell == Cell::Transition, "{name}: a state change outside a Transition cell");
                    word(&s)
                })
                .collect();
            assert_eq!(cells.len(), COLUMNS.len(), "a column without an event sample");
            println!("| **{name}** | {} |", cells.join(" | "));
        }
        println!("{counts:?}");
        let n = |c| counts.get(&c).copied().unwrap_or(0);
        assert_eq!(counts.values().sum::<usize>(), 150);
        assert_eq!(
            [n(Cell::Transition), n(Cell::Stays), n(Cell::Ignored), n(Cell::Late), n(Cell::Counted), n(Cell::RefusedCall), n(Cell::ClaimAnswer), n(Cell::Impossible)],
            [20, 4, 35, 43, 7, 12, 9, 20],
            "a cell changed: OPENING.md's table 1 and its counts change with it"
        );
    }

    /// Machine 2's table, printed, with its counts pinned.
    #[test]
    fn every_head_cell_is_decided_and_the_counts_are_the_documents() {
        let mut counts = std::collections::BTreeMap::new();
        for s in HeadKnown::ALL {
            let row: Vec<String> = HeadEvent::ALL
                .iter()
                .map(|e| {
                    let (next, effects, cell) = s.step(*e);
                    *counts.entry(cell).or_insert(0usize) += 1;
                    format!("{cell:?} {next:?} {effects:?}")
                })
                .collect();
            println!("| **{s:?}** | {} |", row.join(" | "));
        }
        println!("{counts:?}");
        let n = |c| counts.get(&c).copied().unwrap_or(0);
        assert_eq!([n(Cell::Transition), n(Cell::Stays), n(Cell::Late), n(Cell::Impossible)], [15, 12, 11, 4], "a cell changed: OPENING.md table 2 and its counts change with it");
    }

    use crate::seeded::Rng;

    /// What a REAL node may answer a page in `s`: only what is out (the first request, or the registration), a twin of
    /// an answer already given, a keyless refusal, or the socket's replacement. Never an answer to nothing.
    fn node_event(s: &Opening, rng: &mut Rng, said: &mut Vec<fn() -> OpenEvent>) -> OpenEvent {
        let answers_query: [fn() -> OpenEvent; 4] = [
            || OpenEvent::Register(Some((vec![3], [3; 32]))),
            || OpenEvent::Register(None),
            || OpenEvent::QueryRefused("q".into()),
            || OpenEvent::NodeRefused("n".into()),
        ];
        let answers_provision: [fn() -> OpenEvent; 4] =
            [|| OpenEvent::Provisioned, || OpenEvent::KeyAlreadyHere, || OpenEvent::ProvisionRefused("p".into()), || OpenEvent::NodeRefused("n".into())];
        let pick = |rng: &mut Rng, xs: &[fn() -> OpenEvent], said: &mut Vec<fn() -> OpenEvent>| {
            let f = xs[rng.pick(xs.len())];
            said.push(f);
            f()
        };
        match rng.pick(5) {
            0 => OpenEvent::Reconnected,
            1 if !said.is_empty() => said[rng.pick(said.len())](),
            _ => match s {
                Opening::Registering { .. } => [OpenEvent::EmptyAck("e".into()), OpenEvent::DelegateMissing("m".into()), OpenEvent::NodeRefused("n".into())].into_iter().nth(rng.pick(3)).expect("3"),
                // #260: a request that arrived before the registration took is answered EMPTY (0.2.136) or Missing.
                Opening::Querying { .. } | Opening::Provisioning { .. } if rng.pick(4) == 0 => {
                    if rng.pick(2) == 0 { OpenEvent::EmptyAck("e".into()) } else { OpenEvent::DelegateMissing("m".into()) }
                }
                Opening::Querying { .. } => pick(rng, &answers_query, said),
                Opening::Asking { answer: None } => match rng.pick(3) {
                    0 => OpenEvent::EmptyAck("e".into()),
                    1 => OpenEvent::DelegateMissing("m".into()),
                    _ => pick(rng, &answers_query, said),
                },
                Opening::Provisioning { .. } => pick(rng, &answers_provision, said),
                Opening::New | Opening::NeedsKey { .. } | Opening::Open { .. } | Opening::Refused { .. } | Opening::Asking { answer: Some(_) } | Opening::Reader => OpenEvent::NodeRefused("n".into()),
            },
        }
    }

    fn api_event(rng: &mut Rng) -> OpenEvent {
        match rng.pick(5) {
            0 => OpenEvent::Begin(container()),
            1 => OpenEvent::Provide(container(), vec![5]),
            2 => OpenEvent::Ask,
            3 => OpenEvent::Claim(container()),
            _ => OpenEvent::ProvisionWith(vec![6], vec![7]),
        }
    }

    fn kind(e: &OpenEvent) -> &'static str {
        match e {
            OpenEvent::Begin(_) => "Begin",
            OpenEvent::Provide(..) => "Provide",
            OpenEvent::Ask => "Ask",
            OpenEvent::Claim(_) => "Claim",
            OpenEvent::ProvisionWith(..) => "ProvisionWith",
            OpenEvent::EmptyAck(_) => "EmptyAck",
            OpenEvent::DelegateMissing(_) => "DelegateMissing",
            OpenEvent::Register(_) => "Register",
            OpenEvent::QueryRefused(_) => "QueryRefused",
            OpenEvent::Provisioned => "Provisioned",
            OpenEvent::KeyAlreadyHere => "KeyAlreadyHere",
            OpenEvent::ProvisionRefused(_) => "ProvisionRefused",
            OpenEvent::NodeRefused(_) => "NodeRefused",
            OpenEvent::Reconnected => "Reconnected",
        }
    }

    /// THE MODEL (Machine 1): 300 seeded runs x 200 steps from New and from Reader, a real node's answers only, with
    /// the invariants checked after EVERY step. Shown red on the broken versions OPENING.md lists (tools/mutant.sh).
    #[test]
    fn the_opening_model_holds_its_invariants_on_every_step() {
        let mut steps = 0usize;
        // The cells each check is about, and how often the model reached them (a check never reached is decoration).
        let mut reached: std::collections::BTreeMap<(String, &'static str), usize> = std::collections::BTreeMap::new();
        for seed in 1..=300u64 {
            for start in [Opening::New, Opening::Reader] {
                let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
                let mut s = start.clone();
                let mut said = Vec::new();
                for i in 0..200 {
                    let ev = if rng.pick(3) == 0 { api_event(&mut rng) } else { node_event(&s, &mut rng, &mut said) };
                    let k = kind(&ev);
                    let row = format!("{s:?}").split([' ', '{']).next().unwrap_or_default().to_string();
                    *reached.entry((row, k)).or_insert(0) += 1;
                    let st = s.step(ev);
                    let at = format!("seed {seed} step {i}: {s:?} x {k}");
                    assert_ne!(st.cell, Cell::Impossible, "{at}: a real node's answer landed in an impossible cell");
                    let next = st.next.clone().unwrap_or_else(|| s.clone());
                    let sends_first = st.effects.iter().any(|f| matches!(f, OpenEffect::SendFirst | OpenEffect::RegistrationAnswered));
                    // O3: the first request goes out only into a state that names it.
                    if sends_first {
                        assert!(matches!(next, Opening::Querying { .. } | Opening::Provisioning { .. } | Opening::Asking { answer: None }), "{at}: a first request sent into {next:?}");
                    }
                    // O4: a key is minted (provision_with accepted) only in NeedsKey.
                    if k == "ProvisionWith" && st.cell == Cell::Transition {
                        assert!(matches!(s, Opening::NeedsKey { .. }), "{at}: provision_with acted outside NeedsKey");
                    }
                    // O5: a reader installs, asks, mints and provisions nothing.
                    if matches!(s, Opening::Reader) {
                        assert!(st.effects.iter().all(|f| matches!(f, OpenEffect::Unusable(_))), "{at}: a reader did something on the node");
                        assert_eq!(next, Opening::Reader, "{at}: a reader became something else");
                    }
                    // #260: an EMPTY or Missing to an owner that REGISTERED is not an answer.
                    if matches!(s, Opening::Querying { .. } | Opening::Provisioning { .. }) && matches!(k, "EmptyAck" | "DelegateMissing") {
                        assert_eq!(next, s, "{at}: an empty answer after registering was read as an answer");
                    }
                    // The Reconnected column: the socket's replacement loses nothing of the opening.
                    if k == "Reconnected" {
                        assert_eq!(next, s, "{at}: a reconnect changed the opening");
                    }
                    s = next;
                    steps += 1;
                }
            }
        }
        println!("{steps} steps checked");
        for cell in [("Querying", "EmptyAck"), ("Provisioning", "DelegateMissing"), ("NeedsKey", "ProvisionWith"), ("Querying", "ProvisionWith"), ("Open", "Reconnected"), ("Provisioning", "KeyAlreadyHere"), ("Asking", "EmptyAck"), ("Reader", "Begin"), ("Asking", "Claim")] {
            let n = reached.get(&(cell.0.to_string(), cell.1)).copied().unwrap_or(0);
            println!("reached {} x {}: {n}", cell.0, cell.1);
            assert!(n > 0, "the model never reached {} x {}, a cell one of its checks is about", cell.0, cell.1);
        }
    }

    /// THE MODEL (Machine 2): 300 seeded runs x 200 steps; the record query is answered only when asked (or its twin
    /// later). H1, H2 and the counted contradiction after every step.
    #[test]
    fn the_head_model_holds_h1_and_h2_on_every_step() {
        // How often the model reached the cells its checks are about: a check on a cell never reached is decoration.
        let (mut contradicted, mut stale_reads) = (0usize, 0usize);
        for seed in 1..=300u64 {
            let mut rng = Rng(seed.wrapping_mul(0xD1B5_4A32_D192_ED03) | 1);
            let mut s = HeadKnown::Unknown;
            let mut asked = false;
            // THE WORLD: does the signer hold a record? Another tab or device may create one at any step.
            let mut world_has_record = rng.pick(2) == 0;
            // THE REFERENCE for H1: the signer said "none" SINCE the world last changed (a stale "none" is no word).
            let mut fresh_none = false;
            for i in 0..200 {
                if !world_has_record && rng.pick(10) == 0 {
                    world_has_record = true; // another device signed first
                    fresh_none = false;
                }
                // Seen is where a page's head question ends: the model goes on with a FRESH page and world, so every run
                // keeps visiting the undecided states its checks are about.
                if s == HeadKnown::Seen {
                    s = HeadKnown::Unknown;
                    asked = false;
                    world_has_record = rng.pick(2) == 0;
                    fresh_none = false;
                }
                let ev = match rng.pick(12) {
                    0 => HeadEvent::Got,
                    1 => HeadEvent::PutAcked,
                    2 => HeadEvent::Reconnected,
                    // The signer answers what the world holds, or neither.
                    3..=6 if asked => HeadEvent::Record(if rng.pick(4) == 0 { None } else { Some(world_has_record) }),
                    _ => HeadEvent::NotFound,
                };
                if matches!(ev, HeadEvent::Record(_)) && !asked {
                    continue; // a real node answers only a record query this page sent
                }
                if let HeadEvent::Record(Some(h)) = ev {
                    fresh_none = !h;
                }
                let (next, effects, cell) = s.step(ev);
                let at = format!("seed {seed} step {i}: {s:?} x {ev:?}");
                assert_ne!(cell, Cell::Impossible, "{at}");
                if effects.contains(&HeadEffect::AskRecord) {
                    asked = true;
                }
                // H1: "no head" is said only where the signer said it holds no record.
                if effects.contains(&HeadEffect::NoHead) {
                    assert!(fresh_none, "{at}: 'no head' said without the signer's FRESH word that it holds no record (a stale one, after another device created the Register, would fork it)");
                    assert_eq!(next, HeadKnown::NoRecord, "{at}: 'no head' said outside NoRecord");
                }
                // H2: an answered record query never leaves the head read waiting on it.
                if s == HeadKnown::AskingRecord && matches!(ev, HeadEvent::Record(_)) {
                    assert_ne!(next, HeadKnown::AskingRecord, "{at}: the record's answer left the head pending");
                }
                if s == HeadKnown::NoRecord && ev == HeadEvent::NotFound && world_has_record {
                    stale_reads += 1;
                }
                if s == HeadKnown::NoRecord && next == HeadKnown::HasRecord {
                    contradicted += 1;
                    assert!(effects.contains(&HeadEffect::Contradicted), "{at}: 'no head' contradicted and not counted");
                }
                s = next;
            }
        }
        println!("reached: NoRecord x Record(true) {contradicted} times; a read in NoRecord after another device created the Register {stale_reads} times");
        assert!(contradicted > 0 && stale_reads > 0, "the model never reached the cells its H1 and contradiction checks are about");
    }

    /// Machine 3's table, printed, its counts pinned.
    #[test]
    fn every_head_sub_cell_is_decided_and_the_counts_are_the_documents() {
        let mut counts = std::collections::BTreeMap::new();
        for s in HeadSub::ALL {
            let row: Vec<String> = SubEvent::ALL.iter().map(|e| {
                let (next, cell) = s.step(*e);
                *counts.entry(cell).or_insert(0usize) += 1;
                if next == s { format!("{cell:?}") } else { format!("→ {next:?}") }
            }).collect();
            println!("| **{s:?}** | {} |", row.join(" | "));
        }
        let n = |c| counts.get(&c).copied().unwrap_or(0);
        assert_eq!([n(Cell::Transition), n(Cell::Stays), n(Cell::Impossible)], [5, 11, 5], "a cell changed: OPENING.md table 3 changes with it");
    }

    /// THE MODEL (Machine 3), over a node that is the REFERENCE: whether it holds this connection's subscription. A read
    /// answered `Got` subscribes; a NotFound may or may not (measured: the page that created its own head WAS told,
    /// sdk#490); a reconnect drops it (S3); a move or a `Subscribed` ack is sent only by a node that holds it. After every
    /// step: S1 (never "subscribed" where the node holds none) and S2 (a delivered move leaves the page subscribed).
    #[test]
    fn the_head_sub_model_holds_s1_to_s3_on_every_step() {
        let (mut proven_by_move, mut dropped_by_reconnect) = (0usize, 0usize);
        for seed in 1..=300u64 {
            let mut rng = Rng(seed.wrapping_mul(0xA24B_AED4_963E_E407) | 1);
            let (mut s, mut node_holds, mut read_out) = (HeadSub::Unasked, false, false);
            for i in 0..200 {
                let ev = match rng.pick(7) {
                    0 => SubEvent::ReadSent,
                    1 if read_out => SubEvent::Got,
                    2 if read_out => SubEvent::NotFound,
                    3 if read_out => SubEvent::Refused,
                    4 if node_holds => SubEvent::SubscribedAck,
                    5 if node_holds => SubEvent::HeadChanged,
                    6 => SubEvent::Reconnected,
                    _ => continue,
                };
                match ev {
                    SubEvent::ReadSent => read_out = true,
                    SubEvent::Got => node_holds = true,
                    SubEvent::NotFound => node_holds |= rng.pick(2) == 0,
                    SubEvent::Refused | SubEvent::SubscribedAck | SubEvent::HeadChanged => {}
                    // S3: the node drops the connection's subscription; the page re-reads at once.
                    SubEvent::Reconnected => {
                        node_holds = false;
                        read_out = s != HeadSub::Unasked;
                    }
                }
                let (next, cell) = s.step(ev);
                let at = format!("seed {seed} step {i}: {s:?} x {ev:?}");
                assert_ne!(cell, Cell::Impossible, "{at}: a real node's frame landed in an impossible cell");
                assert!(next != HeadSub::Subscribed || node_holds, "{at}: S1 -- 'subscribed' where the node holds no subscription");
                if ev == SubEvent::HeadChanged {
                    proven_by_move += usize::from(s == HeadSub::Asked);
                    assert_eq!(next, HeadSub::Subscribed, "{at}: S2 -- a delivered move did not prove the subscription");
                }
                if ev == SubEvent::Reconnected && s == HeadSub::Subscribed {
                    dropped_by_reconnect += 1;
                }
                s = next;
            }
        }
        println!("reached: Asked x HeadChanged {proven_by_move}; Subscribed x Reconnected {dropped_by_reconnect}");
        assert!(proven_by_move > 0 && dropped_by_reconnect > 0, "the model never reached the cells S2 and S3 are about");
    }

}
