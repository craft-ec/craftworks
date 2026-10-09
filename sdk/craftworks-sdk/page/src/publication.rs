//! PUBLISH-LIFE (sdk#485 D; craftworks-docs docs/design/PUBLISH-LIFE.md): a LABEL's publication -- the head's, or a
//! site's -- as ONE enum per label whose cases carry their data, changed by ONE transition, [`Life::on`].
//!
//! ONE WRITER, BY TYPE: every label's `Life` lives in [`Pubs`], whose fields are private to this module; its ONLY
//! `&mut` method is [`Pubs::on`], which hands the event to the label's [`Life::on`] (private here too). Outside this
//! module a `Life` can be read, never written: the compiler holds the rule. The transition never sends: it returns
//! [`Act`]s, which the page carries out (a sign under a fresh id, an UPDATE, a register
//! read, an engine event). Every `match` over the state and the event names every case: a new one fails to compile
//! until the table has a cell for it.
//!
//! The doc's cells, by state (head table / site table):
//! * `Idle` -- nothing owed. `Reading` -- a site whose version is not read yet.
//! * `Signing` -- a Sign out (its id is DERIVED from the op on the wire, never stored).
//! * `BackingOff` -- a retryable refusal; the next Sign at `at` (P6: [`backoff`]).
//! * `OldSigner` -- an OLD signer refuses `Forked` on the head this page stands on: not re-asked until a head read
//!   shows the register past `at` (⁴).
//! * `Written` -- UPDATE sent with the signer's record; read back until it shows (P4).
//! * `Verifying` / `Landing` -- the head only: invariant 1b's read of a `NotNext`/`Forked`, and landing the signer's
//!   record one behind the register.
//! * `Ended` -- a site's end (Published / Superseded / Refused / Cancelled).

// No catch-all over a state or an event: a new case fails the build until the table has its cell (clippy, run by
// the gate with -D warnings).
#![deny(clippy::wildcard_enum_match_arm, clippy::match_wildcard_for_single_variants)]

use crate::judge::{judge_site, Heard, Judged, SiteJudged};
use crate::{rto, HeadRead, Publication, BACKOFF_MS, HEAD_READS, WAITING_FOR_SITE};
use freenet_prolly::Cid;
use signer_proto::{Answer as A, Why};
use std::collections::BTreeMap;

/// Every label's publication: the head's, and each site's by app id. Private fields; [`Pubs::on`] is the one writer.
#[derive(Debug, Default)]
pub(crate) struct Pubs {
    head: Life,
    sites: BTreeMap<String, Life>,
}

impl Pubs {
    pub(crate) fn head(&self) -> &Life {
        &self.head
    }

    pub(crate) fn site(&self, app: &str) -> Option<&Life> {
        self.sites.get(app)
    }

    /// Every label, the head first.
    pub(crate) fn labels(&self) -> Vec<crate::Label> {
        std::iter::once(crate::Label::Head).chain(self.sites.keys().map(|a| crate::Label::Site(a.clone()))).collect()
    }

    /// Every label's life, the head first.
    pub(crate) fn lives(&self) -> impl Iterator<Item = &Life> {
        std::iter::once(&self.head).chain(self.sites.values())
    }

    /// THE ONE WRITER: `ev` for `label`, through its one transition. A site is made by `Publish` alone; any other
    /// event for a site never asked is for nobody.
    pub(crate) fn on(&mut self, label: &crate::Label, ev: Ev<'_>, cx: &Cx) -> Vec<Act> {
        match label {
            crate::Label::Head => self.head.on(Kind::Head, ev, cx),
            crate::Label::Site(app) => match self.sites.get_mut(app) {
                Some(life) => life.on(Kind::Site, ev, cx),
                None if matches!(ev, Ev::Publish(_)) => self.sites.entry(app.clone()).or_default().on(Kind::Site, ev, cx),
                None => Vec::new(),
            },
        }
    }

    /// A state set directly, for a unit test of one cell.
    #[cfg(test)]
    pub(crate) fn set_head_for_test(&mut self, life: Life) {
        self.head = life;
    }
}

/// P6: THE backoff of a publication's retry, after `tries` refusals (or silent reads) in a row.
pub(crate) fn backoff(tries: u32) -> u64 {
    (BACKOFF_MS << tries.min(5)).min(rto::RTO_MAX_MS as u64)
}

/// What a label owes: the head -- a commit's `(seq, root)` built on `base` (its prev is `(seq - 1, base)`, P2); a
/// site -- version `seq` of bundle `root`, after the version whose value is `base`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Owed {
    pub(crate) seq: u64,
    pub(crate) root: Cid,
    pub(crate) base: Cid,
}

/// Is this the head's life or a site's? The label's one stated fact the table reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Head,
    Site,
}

/// ONE label's publication (PUBLISH-LIFE's enum).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum Life {
    #[default]
    Idle,
    Reading { value: Cid },
    /// `refusals`: retryable refusals in a row before this ask; `why`: the last of them (a site shows
    /// `HeadUnknown` as what it waits for).
    Signing { owed: Owed, refusals: u32, why: Option<Why> },
    BackingOff { owed: Owed, at: u64, refusals: u32, why: Why },
    OldSigner { owed: Owed, at: u64 },
    Written { owed: Owed, record: Vec<u8>, stale: u32 },
    /// `named`: the head the signer named. `at`: the next read, when the last showed no head to land from.
    Verifying { owed: Owed, named: (u64, Cid), tries: u32, at: Option<u64> },
    /// `from`: the register head the landing's sign asks from (`None`: landing THIS page's own record, UPDATEd as it
    /// is -- `MineWins`). `updates`: UPDATEs this landing has sent.
    Landing { owed: Owed, named: (u64, Cid), from: Option<(u64, Cid)>, updates: u32, tries: u32 },
    Ended(Publication),
}

/// Which read of the head register answered (each is its own wait).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadFrom {
    /// This page's own read-back after an UPDATE (or a pushed state that stands in for it).
    ReadBack,
    /// 1b's read of a head the signer named.
    Verify,
    /// A hint's read (the node's `HeadChanged`, or the idle backstop).
    Hint,
}

/// The events of PUBLISH-LIFE (E1–E12), as the page hands them in.
pub(crate) enum Ev<'a> {
    /// E1: the engine's `UpdateHead` -- a commit to publish.
    Owe(Owed),
    /// E2: `publish_site(app, value)`.
    Publish(Cid),
    /// E3–E8: the signer's answer to THIS label's Sign in flight.
    Signer(A),
    /// The UPDATE's answer (F56: it decides nothing; the read that follows does).
    Updated,
    /// A lost UPDATE was re-sent (a landing counts it).
    UpdateResent,
    /// A lost Sign: rebuilt under a fresh id from the state (the landing's or the commit's).
    SignLost,
    /// E9 for the head: a register read, judged; `confirms`: it shows THIS page's commit (the page's one rule).
    Read { from: ReadFrom, j: &'a Judged, confirms: bool },
    /// Any head read showed the register at `seq` (the old signer's wait, ⁴).
    HeadSeen(u64),
    /// E9 for a site: its register read.
    SiteRead(Option<&'a HeadRead>),
    /// E10: the page's clock.
    Due,
    /// E11: the engine moved; an owed head it no longer builds on is dead.
    Dead,
    /// E12: the person cancels (a site).
    Cancel,
    /// E8 from the NODE's side, for a site: its op could not be sent, or its PUT was refused -- a final end, named.
    NodeRefused(String),
}

/// What the page knows that a transition reads.
pub(crate) struct Cx {
    pub(crate) now: u64,
    /// The head the engine publishes.
    pub(crate) published: (u64, Cid),
    pub(crate) engine_has_head: bool,
}

/// What a transition asks the page to do, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Act {
    /// A Sign from `(prev_seq, prev_root)` for `(seq, root)`, under a fresh id (SG02).
    Sign { prev_seq: u64, prev_root: Cid, seq: u64, root: Cid },
    /// UPDATE the label's register with these exact bytes (P3).
    Update(Vec<u8>),
    /// Read the label's register for this wait.
    Read(ReadWait),
    /// Read it again for this wait, unless that read is already out (a stale read-back, at the tick).
    ReadIfIdle(ReadWait),
    /// Every wait of the label ends (a publication replaced, dead or ended).
    EndWaits,
    /// The head register's `heard` is adopted: the ENGINE's `HeadConflict`, the one door (¹⁰).
    Adopt(Heard),
    /// The read-back showed this page's commit at `seq`: the engine's `HeadConfirmed`.
    Confirmed(u64),
    /// A FINAL end of the owed commit at `seq`: the engine's `HeadRefused` (⁵); `why` is said once, with the writes
    /// it failed.
    Refused { seq: u64, why: String },
    /// A record the signer returned for the head: kept whole (invariant 2), and for the tie-break.
    Note(Vec<u8>),
    /// The old signer's fork at `seq`: named once (the page keeps which seq it named).
    Forked(u64),
    /// PUT this block again from page memory: the signer said the node does not hold the commit's ROOT
    /// (`RootNotHeld`, E6). An acked root the node then lost is otherwise never put again, and the sign is re-asked
    /// for ever against a node that cannot hold it (the live-repair writer stall).
    Reput(Cid),
    /// A landing began (counted), or sent its `n`th UPDATE.
    Landing,
    LandingUpdates(u32),
}

/// The register read a transition asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadWait {
    ReadBack,
    Verify,
    /// The head's warm read (a `HeadUnknown` signer's node is made to hold the register).
    Warm,
}

impl Life {
    /// The owed record, in every state that has one.
    pub(crate) fn owed(&self) -> Option<&Owed> {
        match self {
            Life::Signing { owed, .. }
            | Life::BackingOff { owed, .. }
            | Life::OldSigner { owed, .. }
            | Life::Written { owed, .. }
            | Life::Verifying { owed, .. }
            | Life::Landing { owed, .. } => Some(owed),
            Life::Idle | Life::Reading { .. } | Life::Ended(_) => None,
        }
    }

    /// The timer this state keeps, if any (E10).
    pub(crate) fn due_at(&self) -> Option<u64> {
        match self {
            Life::BackingOff { at, .. } => Some(*at),
            Life::Verifying { at, .. } => *at,
            Life::Idle
            | Life::Reading { .. }
            | Life::Signing { .. }
            | Life::OldSigner { .. }
            | Life::Written { .. }
            | Life::Landing { .. }
            | Life::Ended(_) => None,
        }
    }

    /// 1b's register read is owed (a hint is not acted on while it is).
    pub(crate) fn verifying(&self) -> bool {
        matches!(self, Life::Verifying { .. } | Life::Landing { .. })
    }

    /// A site's publication, DERIVED (the one owner is this state). `None`: nothing was ever asked.
    pub(crate) fn publication(&self) -> Option<Publication> {
        let waiting = |why: Option<&Why>| Publication::Publishing { waiting_for: (why == Some(&Why::HeadUnknown)).then_some(WAITING_FOR_SITE) };
        match self {
            Life::Idle => None,
            Life::Ended(p) => Some(p.clone()),
            Life::Signing { why, .. } => Some(waiting(why.as_ref())),
            Life::BackingOff { why, .. } => Some(waiting(Some(why))),
            Life::Reading { .. } | Life::OldSigner { .. } | Life::Written { .. } | Life::Verifying { .. } | Life::Landing { .. } => Some(waiting(None)),
        }
    }

    /// THE TRANSITION: the one place a label's publication changes. Returns what the page must do, in order.
    fn on(&mut self, kind: Kind, ev: Ev<'_>, cx: &Cx) -> Vec<Act> {
        let mut acts = Vec::new();
        let next = match kind {
            Kind::Head => self.head(ev, cx, &mut acts),
            Kind::Site => self.site(ev, cx, &mut acts),
        };
        if let Some(next) = next {
            *self = next;
        }
        acts
    }

    /// A Sign of `owed`, from its prev (P2).
    fn sign(owed: &Owed, acts: &mut Vec<Act>) {
        acts.push(Act::Sign { prev_seq: owed.seq - 1, prev_root: owed.base, seq: owed.seq, root: owed.root });
    }

    fn signing(owed: Owed, acts: &mut Vec<Act>) -> Life {
        Self::sign(&owed, acts);
        Life::Signing { owed, refusals: 0, why: None }
    }

    /// The head's table. `None`: the state stays as it is.
    fn head(&self, ev: Ev<'_>, cx: &Cx, acts: &mut Vec<Act>) -> Option<Life> {
        match ev {
            // E1: a newer commit's head replaces the owed one (P1, ³): its waits end with it.
            Ev::Owe(owed) => {
                acts.push(Act::EndWaits);
                Some(Self::signing(owed, acts))
            }
            Ev::Publish(_) | Ev::SiteRead(_) | Ev::Cancel | Ev::NodeRefused(_) => None, // a site's events: never the head's
            Ev::Signer(s) => self.head_signer(s, cx, acts),
            Ev::Updated => self.updated(acts),
            Ev::UpdateResent => match self {
                Life::Landing { owed, named, from, updates, tries } => {
                    acts.push(Act::LandingUpdates(updates + 1));
                    Some(Life::Landing { owed: *owed, named: *named, from: *from, updates: updates + 1, tries: *tries })
                }
                Life::Idle | Life::Reading { .. } | Life::Signing { .. } | Life::BackingOff { .. } | Life::OldSigner { .. } | Life::Written { .. } | Life::Verifying { .. } | Life::Ended(_) => None,
            },
            Ev::SignLost => self.sign_lost(acts),
            Ev::Read { from, j, confirms } => self.head_read(from, j, confirms, cx, acts),
            // ⁴: the register moved past the old signer's fork: it signs again.
            Ev::HeadSeen(seq) => match self {
                Life::OldSigner { owed, at } if seq > *at => Some(Self::signing(*owed, acts)),
                Life::OldSigner { .. }
                | Life::Idle
                | Life::Reading { .. }
                | Life::Signing { .. }
                | Life::BackingOff { .. }
                | Life::Written { .. }
                | Life::Verifying { .. }
                | Life::Landing { .. }
                | Life::Ended(_) => None,
            },
            Ev::Due => self.due(cx, acts),
            // E11: a head whose seq the engine published, or whose base it no longer stands on, is dead.
            Ev::Dead => match self.owed() {
                Some(o) if o.seq <= cx.published.0 || (o.seq - 1, o.base) != cx.published => {
                    acts.push(Act::EndWaits);
                    Some(Life::Idle)
                }
                Some(_) | None => None,
            },
        }
    }

    // ---- Cells the head's and a site's tables share: a site is never in a head-only state (Verifying, Landing,
    // OldSigner), so one function serves both.

    /// The UPDATE's answer: a written record is read back; a LANDING's UPDATE is judged by its own read, never by a
    /// commit's read-back.
    fn updated(&self, acts: &mut Vec<Act>) -> Option<Life> {
        match self {
            Life::Written { .. } => acts.push(Act::Read(ReadWait::ReadBack)),
            Life::Landing { .. } => acts.push(Act::Read(ReadWait::Verify)),
            Life::Idle | Life::Reading { .. } | Life::Signing { .. } | Life::BackingOff { .. } | Life::OldSigner { .. } | Life::Verifying { .. } | Life::Ended(_) => {}
        }
        None
    }

    /// A lost Sign, rebuilt under a fresh id from the state: the commit's (or site's) own, or the landing's.
    fn sign_lost(&self, acts: &mut Vec<Act>) -> Option<Life> {
        match self {
            Life::Signing { owed, .. } => Self::sign(owed, acts),
            Life::Landing { named, from: Some((prev_seq, prev_root)), .. } => {
                acts.push(Act::Sign { prev_seq: *prev_seq, prev_root: *prev_root, seq: prev_seq + 1, root: named.1 })
            }
            Life::Landing { from: None, .. } | Life::Idle | Life::Reading { .. } | Life::BackingOff { .. } | Life::OldSigner { .. } | Life::Written { .. } | Life::Verifying { .. } | Life::Ended(_) => {}
        }
        None
    }

    /// E10: a backoff come due signs again; a verify come due reads; a written record read stale reads again (no
    /// read-back on the wire).
    fn due(&self, cx: &Cx, acts: &mut Vec<Act>) -> Option<Life> {
        match self {
            Life::BackingOff { owed, at, refusals, why } if *at <= cx.now => {
                Self::sign(owed, acts);
                Some(Life::Signing { owed: *owed, refusals: *refusals, why: Some(why.clone()) })
            }
            Life::Verifying { owed, named, tries, at: Some(at) } if *at <= cx.now => {
                acts.push(Act::Read(ReadWait::Verify));
                Some(Life::Verifying { owed: *owed, named: *named, tries: *tries, at: None })
            }
            Life::Written { stale, .. } if *stale > 0 => {
                acts.push(Act::ReadIfIdle(ReadWait::ReadBack));
                None
            }
            Life::BackingOff { .. } | Life::Verifying { .. } | Life::Written { .. } | Life::Idle | Life::Reading { .. } | Life::Signing { .. } | Life::OldSigner { .. } | Life::Landing { .. } | Life::Ended(_) => None,
        }
    }

    /// E3–E8 on the head.
    fn head_signer(&self, s: A, cx: &Cx, acts: &mut Vec<Act>) -> Option<Life> {
        let now = cx.now;
        match self {
            Life::Signing { owed, refusals, .. } => {
                let owed = *owed;
                match s {
                    // E3: the signer's bytes, UPDATEd as they are (P3). An `AlreadySigned` record may be another
                    // page's root at this seq: the read-back judges by THIS commit's (seq, root) (the model's M5).
                    A::Signed(state) | A::AlreadySigned(state) => {
                        acts.push(Act::Note(state.clone()));
                        acts.push(Act::Update(state.clone()));
                        Some(Life::Written { owed, record: state, stale: 0 })
                    }
                    // E5: NOT ADOPTED YET (invariant 1b): the register is read first.
                    A::NotNext { current } => {
                        acts.push(Act::Read(ReadWait::Verify));
                        Some(Life::Verifying { owed, named: (current.seq, current.root), tries: 0, at: None })
                    }
                    // E6: the one retryable set, on the one backoff; `HeadUnknown` reads the register first.
                    A::Refused(why) if why.retryable() => {
                        if why == Why::HeadUnknown {
                            acts.push(Act::Read(ReadWait::Warm));
                        }
                        if why == Why::RootNotHeld {
                            acts.push(Act::Reput(owed.root));
                        }
                        let refusals = refusals + 1;
                        Some(Life::BackingOff { owed, at: now + backoff(refusals), refusals, why })
                    }
                    // E7: an OLD signer (THE SAME IDENTITY NEVER FORKS, sdk#225). On the head this page stands on it
                    // refuses every ask until the register passes that seq: not re-asked (a loop), named once (⁴).
                    A::Refused(Why::Forked { read, .. }) => {
                        if cx.published == (read.seq, read.root) {
                            acts.push(Act::Forked(read.seq));
                            Some(Life::OldSigner { owed, at: read.seq })
                        } else {
                            acts.push(Act::Read(ReadWait::Verify));
                            Some(Life::Verifying { owed, named: (read.seq, read.root), tries: 0, at: None })
                        }
                    }
                    // E8: FINAL -- an end, and the engine hears it (⁵).
                    A::Refused(why) => Some(Self::refused(&owed, format!("the signer refused: {why:?}"), acts)),
                    other @ (A::Provisioned | A::Held { .. } | A::Register { .. }) => Some(Self::refused(&owed, format!("the signer answered a sign with {other:?}"), acts)),
                }
            }
            Life::Landing { owed, named, from, updates, tries } => {
                let owed = *owed;
                match s {
                    // The record's bytes, UPDATEd as they are, then read (the same deadline and backoff).
                    A::Signed(state) | A::AlreadySigned(state) => {
                        acts.push(Act::Note(state.clone()));
                        acts.push(Act::Update(state));
                        acts.push(Act::LandingUpdates(updates + 1));
                        Some(Life::Landing { owed, named: *named, from: *from, updates: updates + 1, tries: *tries })
                    }
                    A::NotNext { current } => {
                        let tries = tries + 1;
                        Some(Life::Verifying { owed, named: (named.0.max(current.seq), current.root), tries, at: Some(now + backoff(tries)) })
                    }
                    // The one retryable set, and NotSuccessor: a landing asks from the register's own head, which
                    // may have moved by the time it lands.
                    A::Refused(why) if why.retryable() || why == Why::NotSuccessor => {
                        if why == Why::RootNotHeld {
                            acts.push(Act::Reput(owed.root));
                        }
                        let tries = tries + 1;
                        Some(Life::Verifying { owed, named: *named, tries, at: Some(now + backoff(tries)) })
                    }
                    // Every other refusal: FINAL (⁵). (No FORKED arm needed: a landing asks from the register's own
                    // head, one seq behind the record, so the equal-seq fork check cannot fire, ⁷.)
                    A::Refused(why) => Some(Self::refused(&owed, format!("the signer refused a landing: {why:?}"), acts)),
                    other @ (A::Provisioned | A::Held { .. } | A::Register { .. }) => Some(Self::refused(&owed, format!("the signer answered a landing with {other:?}"), acts)),
                }
            }
            // ¹/⁶: nothing is out to answer (the page takes only the answer under a Sign op's own id).
            Life::Idle | Life::Reading { .. } | Life::BackingOff { .. } | Life::OldSigner { .. } | Life::Written { .. } | Life::Verifying { .. } | Life::Ended(_) => None,
        }
    }

    /// E8, and 1b's unrecoverable record: the owed commit ENDS, and the engine hears it (⁵).
    fn refused(owed: &Owed, why: String, acts: &mut Vec<Act>) -> Life {
        acts.push(Act::EndWaits);
        acts.push(Act::Refused { seq: owed.seq, why });
        Life::Idle
    }

    /// E9 for the head.
    fn head_read(&self, from: ReadFrom, j: &Judged, confirms: bool, cx: &Cx, acts: &mut Vec<Act>) -> Option<Life> {
        match from {
            ReadFrom::ReadBack => self.read_back(j, confirms, acts),
            ReadFrom::Verify => self.verify(j, cx, acts),
            ReadFrom::Hint => self.hint(j, confirms, cx, acts),
        }
    }

    /// The register read back after an UPDATE: the only way a commit is Published (P4).
    fn read_back(&self, j: &Judged, confirms: bool, acts: &mut Vec<Act>) -> Option<Life> {
        if let (Judged::Head(heard), true) = (j, confirms) {
            acts.push(Act::Confirmed(heard.seq()));
            return None;
        }
        let (owed, record, stale) = match self {
            Life::Written { owed, record, stale } => (owed, record, stale),
            // A read-back that outlived its commit.
            Life::Idle | Life::Reading { .. } | Life::Signing { .. } | Life::BackingOff { .. } | Life::OldSigner { .. } | Life::Verifying { .. } | Life::Landing { .. } | Life::Ended(_) => return None,
        };
        match j {
            // Not visible yet -- or THIS page's record wins the tie-break (the node has not merged my UPDATE): read
            // again; after HEAD_READS, UPDATE again.
            Judged::MineWins { .. } => Some(Self::stale(*owed, record, *stale, acts)),
            Judged::Head(heard) if heard.seq() < owed.seq => Some(Self::stale(*owed, record, *stale, acts)),
            Judged::Head(heard) => {
                acts.push(Act::EndWaits);
                acts.push(Act::Adopt(*heard));
                Some(Life::Idle)
            }
            Judged::NoHead => {
                acts.push(Act::Update(record.clone()));
                Some(Life::Written { owed: *owed, record: record.clone(), stale: *stale })
            }
        }
    }

    /// A stale read of a written record (head or site): counted; after HEAD_READS, UPDATE again.
    fn stale(owed: Owed, record: &[u8], stale: u32, acts: &mut Vec<Act>) -> Life {
        let stale = stale + 1;
        if stale >= HEAD_READS {
            acts.push(Act::Update(record.to_vec()));
            return Life::Written { owed, record: record.to_vec(), stale: 0 };
        }
        Life::Written { owed, record: record.to_vec(), stale }
    }

    /// 1b's read (`Verifying`, `Landing`).
    fn verify(&self, j: &Judged, cx: &Cx, acts: &mut Vec<Act>) -> Option<Life> {
        let (owed, named, from, updates, tries) = match self {
            Life::Verifying { owed, named, tries, .. } => (*owed, *named, None, 0, *tries),
            Life::Landing { owed, named, from, updates, tries } => (*owed, *named, Some(*from), *updates, *tries),
            // A verify read that outlived its state.
            Life::Idle | Life::Reading { .. } | Life::Signing { .. } | Life::BackingOff { .. } | Life::OldSigner { .. } | Life::Written { .. } | Life::Ended(_) => return None,
        };
        let h = j.shown();
        let reg_seq = h.map_or(0, |(s, _)| s);
        // THIS page's own record at the register's seq wins the tie-break: LAND it (its UPDATE has not merged here).
        if let Judged::MineWins { seq, record, .. } = j {
            if *seq >= named.0 {
                acts.push(Act::Update(record.clone()));
                acts.push(Act::LandingUpdates(updates + 1));
                return Some(Life::Landing { owed, named, from: from.flatten(), updates: updates + 1, tries });
            }
        }
        if let Judged::Head(heard) = j {
            if heard.seq() >= named.0 {
                acts.push(Act::EndWaits);
                acts.push(Act::Adopt(*heard));
                return Some(Life::Idle);
            }
        }
        // 2+ behind: UNRECOVERABLE (the signer keeps one record) -- an end of the owed commit, the engine told.
        if named.0 > reg_seq + 1 {
            let why = format!("the signer's record (seq {}) is {} ahead of the register (seq {reg_seq}): unrecoverable, and 1b says unreachable", named.0, named.0 - reg_seq);
            return Some(Self::refused(&owed, why, acts));
        }
        let prev = match h {
            Some(h) => h,
            // No head at all: the record's prev is the genesis, where this engine stands if it adopted none.
            None if cx.published.0 == 0 => (0, cx.published.1),
            None => {
                let tries = tries + 1;
                return Some(Life::Verifying { owed, named, tries, at: Some(cx.now + backoff(tries)) });
            }
        };
        if from.is_none() {
            acts.push(Act::Landing);
        }
        acts.push(Act::Sign { prev_seq: prev.0, prev_root: prev.1, seq: prev.0 + 1, root: named.1 });
        Some(Life::Landing { owed, named, from: Some(prev), updates, tries })
    }

    /// A hint's read: the RELOAD TRIGGER (sdk#225). Never acted on while 1b's read is owed.
    fn hint(&self, j: &Judged, confirms: bool, cx: &Cx, acts: &mut Vec<Act>) -> Option<Life> {
        let (seq, root) = j.shown()?;
        if !cx.engine_has_head || self.verifying() {
            return None;
        }
        if (seq, root) == cx.published || seq < cx.published.0 {
            return None;
        }
        if let Life::Written { owed, .. } = self {
            if owed.seq == seq {
                return self.read_back(j, confirms, acts);
            }
        }
        match j {
            // Mine wins at this seq: landed again (its UPDATE has not merged here), never displaced.
            Judged::MineWins { record, .. } => {
                acts.push(Act::Update(record.clone()));
                None
            }
            // A newer head: adopted, and an owed head dies with the commit it was (¹⁰).
            Judged::Head(heard) => {
                if self.owed().is_some() {
                    acts.push(Act::EndWaits);
                }
                acts.push(Act::Adopt(*heard));
                Some(Life::Idle)
            }
            Judged::NoHead => None,
        }
    }

    /// A site's table. `None`: the state stays as it is.
    fn site(&self, ev: Ev<'_>, cx: &Cx, acts: &mut Vec<Act>) -> Option<Life> {
        match ev {
            // E2: publishing again starts over with the new bundle: its version is read first.
            Ev::Publish(value) => {
                acts.push(Act::EndWaits);
                acts.push(Act::Read(ReadWait::ReadBack));
                Some(Life::Reading { value })
            }
            Ev::Owe(_) | Ev::Read { .. } | Ev::HeadSeen(_) | Ev::Dead | Ev::UpdateResent => None, // the head's events
            Ev::Signer(s) => self.site_signer(s, cx, acts),
            Ev::Updated => self.updated(acts),
            Ev::SignLost => self.sign_lost(acts),
            Ev::SiteRead(read) => self.site_read(read, acts),
            Ev::Due => self.due(cx, acts),
            Ev::NodeRefused(why) => match self {
                Life::Idle | Life::Ended(_) => None,
                Life::Reading { .. } | Life::Signing { .. } | Life::BackingOff { .. } | Life::OldSigner { .. } | Life::Written { .. } | Life::Verifying { .. } | Life::Landing { .. } => {
                    Some(Self::end_site(Publication::Refused(why), acts))
                }
            },
            // E12: every wait ends, and it says so. Idle and Ended: nothing to cancel.
            Ev::Cancel => match self {
                Life::Idle | Life::Ended(_) => None,
                Life::Reading { .. } | Life::Signing { .. } | Life::BackingOff { .. } | Life::OldSigner { .. } | Life::Written { .. } | Life::Verifying { .. } | Life::Landing { .. } => {
                    Some(Self::end_site(Publication::Cancelled, acts))
                }
            },
        }
    }

    fn end_site(how: Publication, acts: &mut Vec<Act>) -> Life {
        acts.push(Act::EndWaits);
        Life::Ended(how)
    }

    /// Sign the site's bundle `value` as the version after `(seq, prev)`.
    fn rebase_site(seq: u64, value: Cid, prev: Cid, acts: &mut Vec<Act>) -> Life {
        Self::signing(Owed { seq: seq + 1, root: value, base: prev }, acts)
    }

    /// A SITE's signer answer (the module table's Site column).
    fn site_signer(&self, s: A, cx: &Cx, acts: &mut Vec<Act>) -> Option<Life> {
        let (owed, refusals) = match self {
            Life::Signing { owed, refusals, .. } => (owed, refusals),
            // ¹/⁶/⁹: nothing out to answer.
            Life::Idle | Life::Reading { .. } | Life::BackingOff { .. } | Life::OldSigner { .. } | Life::Written { .. } | Life::Verifying { .. } | Life::Landing { .. } | Life::Ended(_) => return None,
        };
        let owed = *owed;
        match s {
            A::Signed(state) => Some(Self::site_write(owed, state, acts)),
            A::AlreadySigned(state) => match HeadRead::from_record(&state) {
                Some(h) if h.value() == owed.root.as_slice() => Some(Self::site_write(owed, state, acts)),
                // Another bundle: ask FROM that record (a site record cannot be landed without its web bytes).
                Some(h) => match <[u8; 32]>::try_from(h.value()) {
                    Ok(v) => Some(Self::rebase_site(h.seq, owed.root, v, acts)),
                    Err(_) => Some(Self::end_site(Publication::Refused("the signer's record is not a site's".into()), acts)),
                },
                None => Some(Self::end_site(Publication::Refused("the signer's record does not read".into()), acts)),
            },
            // Nothing is adopted, so 1b's read-first is not needed (the architect's Q1).
            A::NotNext { current } => Some(Self::rebase_site(current.seq, owed.root, current.root, acts)),
            A::Refused(why) if why.retryable() => {
                let refusals = refusals + 1;
                Some(Life::BackingOff { owed, at: cx.now + backoff(refusals), refusals, why })
            }
            A::Refused(why) => Some(Self::end_site(Publication::Refused(format!("{why:?}")), acts)),
            other @ (A::Provisioned | A::Held { .. } | A::Register { .. }) => {
                Some(Self::end_site(Publication::Refused(format!("the signer answered a site's sign with {other:?}")), acts))
            }
        }
    }

    /// The site's write: the signer's exact bytes (invariant 2).
    fn site_write(owed: Owed, state: Vec<u8>, acts: &mut Vec<Act>) -> Life {
        acts.push(Act::Update(state.clone()));
        Life::Written { owed, record: state, stale: 0 }
    }

    /// A SITE's read: before it signs, where the next version follows from (NotFound = the genesis); after its
    /// write, its record = Published, newer = Superseded, else read again (HEAD_READS → write again).
    fn site_read(&self, read: Option<&HeadRead>, acts: &mut Vec<Act>) -> Option<Life> {
        match self {
            Life::Reading { value } => match read {
                None => Some(Self::rebase_site(0, *value, [0u8; 32], acts)),
                // THE SITE IS CURRENT: it already holds this exact bundle (its value is the bundle's hash), so it is
                // PUBLISHED at that version -- nothing is signed or written again (a publish after a reload had no
                // record of it and wrote the same bytes as the next version).
                Some(h) if h.value() == value.as_slice() => Some(Self::end_site(Publication::Published { version: h.seq }, acts)),
                Some(h) => match <[u8; 32]>::try_from(h.value()) {
                    Ok(v) => Some(Self::rebase_site(h.seq, *value, v, acts)),
                    Err(_) => Some(Self::end_site(Publication::Refused("the site holds a record that is not a site's".into()), acts)),
                },
            },
            Life::Written { owed, record, stale } => match judge_site(owed.seq, owed.root.as_slice(), read) {
                SiteJudged::Mine => Some(Self::end_site(Publication::Published { version: owed.seq }, acts)),
                SiteJudged::Superseded(version) => Some(Self::end_site(Publication::Superseded { version }, acts)),
                SiteJudged::NotYet => Some(Self::stale(*owed, record, *stale, acts)),
            },
            // ⁸: a site is read only while Reading or Written.
            Life::Idle | Life::Signing { .. } | Life::BackingOff { .. } | Life::OldSigner { .. } | Life::Verifying { .. } | Life::Landing { .. } | Life::Ended(_) => None,
        }
    }
}
