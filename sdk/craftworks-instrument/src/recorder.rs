//! A pre-allocated ring that drops when full, and a separate handle for reading
//! it.
//!
//! The split is the point. [`Recorder`] implements [`Probe`] and is what
//! instrumented code holds — write-only, by the shape of the trait.
//! [`Recording`] is what a test holds, and it is the only way to look. Nothing
//! that has a `&dyn Probe` can read.

use core::cell::RefCell;

use crate::{
    vocab::{Dir, Key, Outcome},
    Answered, Entry, Event, Label, OpId, Probe, Site,
};

/// Events kept, and events dropped.
///
/// Interior mutability because [`Probe::event`] takes `&self` — it has to, or
/// instrumented code would need a `&mut` it cannot have while the thing it is
/// instrumenting is borrowed.
pub struct Recorder {
    inner: RefCell<Ring>,
}

struct Ring {
    slots: Vec<Event>,
    /// Next write position.
    at: usize,
    /// Total events offered, so `seq` is the sequence the spine promises.
    seq: u64,
    /// Events the ring refused because it was full. COUNTED: a silent drop and
    /// a quiet system look identical in a dump.
    dropped: u64,
    full: bool,
}

impl Recorder {
    /// A ring of `capacity` events, allocated once, here and nowhere else.
    pub fn with_capacity(capacity: usize) -> Recorder {
        Recorder {
            inner: RefCell::new(Ring::new(capacity)),
        }
    }

    /// A handle that can read what this recorder holds.
    pub fn recording(&self) -> Recording<'_> {
        Recording { r: self }
    }
}

impl Default for Recorder {
    fn default() -> Self {
        Recorder::with_capacity(4096)
    }
}

impl Probe for Recorder {
    fn event(&self, e: Event) {
        // A probe must not panic, and `RefCell` can. Reentrancy is possible in
        // principle — a `Drop` that emits while a dump is reading — so a failed
        // borrow drops the event rather than taking the process down.
        let Ok(mut ring) = self.inner.try_borrow_mut() else {
            return;
        };
        ring.push(e);
    }
}

/// What a reader can ask of a recording.
///
/// A trait so the same tests run against both recorders. Two implementations
/// of "what is outstanding" would drift, and the drift would be invisible
/// until a dump from one of them was wrong — which is the failure this crate
/// exists to prevent, arriving through its own back door.
pub trait Record {
    fn events(&self) -> Vec<Event>;
    fn offered(&self) -> u64;
    fn dropped(&self) -> u64;

    /// Requests with no matching response, by label.
    ///
    /// The number four voided harness runs could not see. It is a SUBTRACTION
    /// because `Edge` pairs the two directions by id — if the two were
    /// unrelated event types, this would be a heuristic.
    fn outstanding(&self) -> Vec<Label> {
        let mut open: Vec<Label> = Vec::new();
        for e in self.events() {
            if let Event::Edge { dir, id, .. } = e {
                match dir {
                    Dir::Request => open.push(id),
                    Dir::Response => {
                        if let Some(i) = open.iter().position(|l| *l == id) {
                            open.remove(i);
                        }
                    }
                }
            }
        }
        open
    }

    /// Spans that were entered and never exited.
    fn unfinished(&self) -> Vec<(Site, OpId)> {
        let mut open: Vec<(Site, OpId)> = Vec::new();
        for e in self.events() {
            match e {
                Event::Enter { site, op } => open.push((site, op)),
                Event::Exit { site, op, .. } => {
                    if let Some(i) = open.iter().position(|x| *x == (site, op)) {
                        open.remove(i);
                    }
                }
                _ => {}
            }
        }
        open
    }

    /// The sum of one counter across the recording.
    fn total(&self, key: Key) -> u64 {
        self.events()
            .iter()
            .filter_map(|e| match e {
                Event::Counter {
                    entry: Entry { key: k, value },
                    ..
                } if *k == key => Some(*value),
                _ => None,
            })
            .sum()
    }

    /// What happened to each LABELLED operation — the projection that ends
    /// hand-rolled per-key maps.
    ///
    /// Both failures this crate was extended for came from a tool counting
    /// answers instead of matching them. `harness#38`: three puts timed out,
    /// their acks arrived later, nobody consumed them, and every put from the
    /// sixth onward received the PREVIOUS put's answer — 0 of 20 reported
    /// against a node that had accepted 15. `harness#39` run 1: a group waited
    /// until its ack map held as many entries as it had members, filled the
    /// quota with LATE ACKS FROM EARLIER GROUPS, and printed 15/15 while its
    /// own keys went down as never answered.
    ///
    /// A count is not a pairing. When an answer NAMES what it answers — a
    /// `PutResponse` carries the contract key — the recording can say which
    /// operation ended, and every consumer reads the same answer here instead
    /// of deriving its own.
    fn answers(&self) -> Vec<(Label, Answered)> {
        // Exits keyed by the operation a label denotes, so "late" can mean what
        // it should: the answer arrived after its own operation was closed.
        let mut closed: Vec<(OpId, Outcome)> = Vec::new();
        let mut requested: Vec<(Label, usize)> = Vec::new();
        let mut answered: Vec<(Label, usize)> = Vec::new();
        let mut late: Vec<Label> = Vec::new();
        let bump = |v: &mut Vec<(Label, usize)>, l: Label| match v.iter_mut().find(|(x, _)| *x == l)
        {
            Some((_, n)) => *n += 1,
            None => v.push((l, 1)),
        };
        for e in self.events() {
            match e {
                Event::Exit { op, outcome, .. } => closed.push((op, outcome)),
                Event::Edge { dir, id, .. } => match dir {
                    Dir::Request => bump(&mut requested, id),
                    Dir::Response => {
                        bump(&mut answered, id);
                        // Closed already, and closed because nothing came back.
                        // An answer arriving now is LATE — it is not evidence
                        // that the operation succeeded, and it must not be
                        // allowed to close a different one.
                        if closed.iter().any(|(op, o)| {
                            *op == id.op()
                                && matches!(
                                    o,
                                    Outcome::Timeout | Outcome::Blocked | Outcome::Withdrawn
                                )
                        }) {
                            late.push(id);
                        }
                    }
                },
                _ => {}
            }
        }
        let mut out: Vec<(Label, Answered)> = Vec::new();
        for (l, asked) in &requested {
            let got = answered
                .iter()
                .find(|(x, _)| x == l)
                .map(|(_, n)| *n)
                .unwrap_or(0);
            let state = if got == 0 {
                Answered::Never
            } else if late.contains(l) {
                Answered::Late
            } else if got > *asked {
                Answered::Twice(got)
            } else {
                Answered::Once
            };
            out.push((*l, state));
        }
        // An answer for something this recorder never asked. Reported, never
        // attributed to whatever happened to be open.
        for (l, _) in &answered {
            if !requested.iter().any(|(x, _)| x == l) {
                out.push((*l, Answered::Foreign));
            }
        }
        out
    }

    /// How many operations ended each way, from [`answers`](Record::answers).
    ///
    /// One projection feeds a tool's progress line AND its per-key table, so
    /// the two cannot disagree. A harness assertion that they match becomes a
    /// property of the recording rather than of each tool — which is what
    /// turned a 99.5 % ack rate printed over a run whose records said 92.1 %
    /// into something the tool could catch itself.
    fn answer_counts(&self) -> Vec<(Answered, usize)> {
        let mut out: Vec<(Answered, usize)> = Vec::new();
        for (_, a) in self.answers() {
            let a = match a {
                Answered::Twice(_) => Answered::Twice(0),
                other => other,
            };
            match out.iter_mut().find(|(x, _)| *x == a) {
                Some((_, n)) => *n += 1,
                None => out.push((a, 1)),
            }
        }
        out
    }

    /// Bytes counted at OUR boundary for one operation: offered to the socket
    /// and read from it.
    ///
    /// Not what a node sends peer-to-peer — only the node can know that, and
    /// six external instruments were rejected trying to infer it
    /// (freenet-contracts#39). This settles what WE cost, where a wrapper
    /// already exists.
    fn bytes(&self, op: OpId) -> (u64, u64) {
        let (mut out, mut inb) = (0u64, 0u64);
        for e in self.events() {
            if let Event::Counter { op: o, entry, .. } = e {
                if o == op {
                    match entry.key {
                        Key::BytesOut => out += entry.value,
                        Key::BytesIn => inb += entry.value,
                        _ => {}
                    }
                }
            }
        }
        (out, inb)
    }

    /// How many spans ended each way.
    fn outcomes(&self) -> Vec<(Outcome, usize)> {
        let mut out: Vec<(Outcome, usize)> = Vec::new();
        for e in self.events() {
            if let Event::Exit { outcome, .. } = e {
                match out.iter_mut().find(|(o, _)| *o == outcome) {
                    Some((_, n)) => *n += 1,
                    None => out.push((outcome, 1)),
                }
            }
        }
        out
    }
}

impl Ring {
    /// Offer one event. The whole ring policy, in one place, used by both
    /// recorders: keep the LAST N, count what that costs, never grow.
    fn push(&mut self, e: Event) {
        self.seq += 1;
        let cap = self.slots.capacity();
        if self.slots.len() < cap {
            self.slots.push(e);
            self.at = self.slots.len() % cap;
            return;
        }
        self.full = true;
        self.dropped += 1;
        let at = self.at;
        self.slots[at] = e;
        self.at = (at + 1) % cap;
    }

    fn snapshot(&self) -> Vec<Event> {
        if !self.full {
            return self.slots.clone();
        }
        let mut out = Vec::with_capacity(self.slots.len());
        out.extend_from_slice(&self.slots[self.at..]);
        out.extend_from_slice(&self.slots[..self.at]);
        out
    }

    fn new(capacity: usize) -> Ring {
        let cap = capacity.max(1);
        Ring {
            slots: Vec::with_capacity(cap),
            at: 0,
            seq: 0,
            dropped: 0,
            full: false,
        }
    }
}

/// The read side. A test holds this; instrumented code never does.
pub struct Recording<'a> {
    r: &'a Recorder,
}

impl Record for Recording<'_> {
    fn events(&self) -> Vec<Event> {
        self.r.inner.borrow().snapshot()
    }

    fn offered(&self) -> u64 {
        self.r.inner.borrow().seq
    }

    fn dropped(&self) -> u64 {
        self.r.inner.borrow().dropped
    }
}

/// The same ring, usable from several threads.
///
/// The harness is multi-threaded and so is the engine; a `RefCell` recorder
/// cannot be shared across them. Everything else is identical on purpose —
/// one ring policy, one set of read methods, and the SAME tests run against
/// both, because two implementations of "what is outstanding" would drift and
/// the drift would only show as a wrong dump.
pub struct SyncRecorder {
    inner: std::sync::Mutex<Ring>,
}

impl SyncRecorder {
    pub fn with_capacity(capacity: usize) -> SyncRecorder {
        SyncRecorder {
            inner: std::sync::Mutex::new(Ring::new(capacity)),
        }
    }

    pub fn recording(&self) -> SyncRecording<'_> {
        SyncRecording { r: self }
    }

    /// The lock, with a poisoned one RECOVERED rather than unwrapped.
    ///
    /// A probe must never panic — a probe that fails a passing test is the
    /// worst kind of behaviour change — and `lock().unwrap()` panics for every
    /// caller forever once any thread has panicked while holding it. A ring of
    /// `Copy` events has no invariant a panic could have broken, so recovering
    /// is sound here in a way it would not be for an arbitrary structure.
    fn ring(&self) -> std::sync::MutexGuard<'_, Ring> {
        match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Default for SyncRecorder {
    fn default() -> Self {
        SyncRecorder::with_capacity(4096)
    }
}

impl Probe for SyncRecorder {
    fn event(&self, e: Event) {
        self.ring().push(e);
    }
}

/// The read side of a [`SyncRecorder`].
pub struct SyncRecording<'a> {
    r: &'a SyncRecorder,
}

impl Record for SyncRecording<'_> {
    fn events(&self) -> Vec<Event> {
        self.r.ring().snapshot()
    }

    fn offered(&self) -> u64 {
        self.r.ring().seq
    }

    fn dropped(&self) -> u64 {
        self.r.ring().dropped
    }
}
