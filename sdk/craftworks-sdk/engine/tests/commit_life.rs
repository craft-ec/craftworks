//! A COMMIT'S LIFE, AGAINST ITS TABLE (COMMIT-LIFE rev 5, sdk#481; the owner's "Structure before code").
//!
//! THE REFERENCE is the table itself -- [`cell`], a pure function of (stage, event) written from the document, not from
//! the engine -- and a seeded model drives the REAL engine through the table's columns (E1-E14: PUT answers held,
//! failed or rejected; heads shown as mine, older, foreign or refused; ticks), asserting at every step that the
//! engine's stage and the fates it tells are the cell's. At rest, every write was told exactly one terminal fate and
//! nothing stalled-told outlives its write.
//!
//! Each known-broken cell of the document's § Defects is a NAMED case below, red on the engine as the rework found it
//! (`54dd8e2` + #502): A1 a dead end withdraws nothing, A2 Heading goes back to Racing, A5 the stalled record outlives
//! a publish, A6 a silent Racing commit is ended on time, A8 a never-sent head reads the witness and spends a try, A9
//! a rejected root holds Heading for ever.
use std::collections::{BTreeMap, BTreeSet};

use engine::{ClientId, CommitStage, Effect, Event, Expect, Op, Params, State, Witness, WriteId};
use freenet_prolly::Cid;

mod common;
use common::{Harness, Store};

const T0: u64 = 1_790_000_000;

/// A write of one NEW key: it reads the key absent, so it is not forced (a forced write is `Lost` at its first dead
/// commit by WRITE-PATH ⁷, which would hide the try count A8 is about) and no two of them conflict.
fn write(id: u64) -> Event {
    let key = format!("m/{id:05}").into_bytes();
    Event::Write {
        client: ClientId(1),
        write_id: WriteId(id),
        ops: vec![(key.clone(), Op::Put(vec![id as u8; 300]))],
        reads: vec![(key, Expect::Absent)],
        deferred: false,
    }
}

/// A write of many new keys: a tree of several leaves, so one ack is not race-ready.
fn big_write(id: u64) -> Event {
    let keys: Vec<Vec<u8>> = (0..24u8).map(|i| format!("b/{id:03}/{i:02}").into_bytes()).collect();
    Event::Write {
        client: ClientId(1),
        write_id: WriteId(id),
        ops: keys.iter().map(|k| (k.clone(), Op::Put(vec![id as u8; 700]))).collect(),
        reads: keys.into_iter().map(|k| (k, Expect::Absent)).collect(),
        deferred: false,
    }
}

/// A write wide enough that a changed group has more members than parity (k > m): rejecting its first wave -- all
/// but the held-back parity -- drops it below k.
fn wide_write(id: u64) -> Event {
    let keys: Vec<Vec<u8>> = (0..300u32).map(|i| format!("w/{id:03}/{i:04}").into_bytes()).collect();
    Event::Write {
        client: ClientId(1),
        write_id: WriteId(id),
        ops: keys.iter().map(|k| (k.clone(), Op::Put(vec![id as u8; 700]))).collect(),
        reads: keys.into_iter().map(|k| (k, Expect::Absent)).collect(),
        deferred: false,
    }
}

fn told(fx: &[Effect], id: u64) -> Vec<State> {
    fx.iter()
        .filter_map(|f| match f {
            Effect::Notify { write_id, state, .. } if write_id.0 == id => Some(*state),
            _ => None,
        })
        .collect()
}

fn terminal(s: State) -> bool {
    matches!(s, State::Published | State::Failed { .. } | State::Lost | State::Unknown | State::Conflict)
}

fn puts(fx: &[Effect]) -> Vec<Cid> {
    fx.iter()
        .filter_map(|f| match f {
            Effect::PutBlock { id, .. } | Effect::PutPack { id, .. } => Some(*id),
            _ => None,
        })
        .collect()
}

fn withdrawn(fx: &[Effect]) -> BTreeSet<Cid> {
    fx.iter().filter_map(|f| if let Effect::Withdraw { id } = f { Some(*id) } else { None }).collect()
}

fn head_sent(fx: &[Effect]) -> Option<(u64, Cid)> {
    fx.iter().find_map(|f| if let Effect::UpdateHead { seq, root, .. } = f { Some((*seq, *root)) } else { None })
}

fn harness(params: Params) -> Harness {
    let mut h = Harness::new(params, Store::fresh());
    let _ = h.step(Event::Tick(T0));
    h
}

/// Confirm every PUT in `fx` (and what that releases) until the head is sent: its (seq, root) and every effect seen.
/// `hold` is never confirmed.
fn to_heading(h: &mut Harness, fx: Vec<Effect>, hold: &BTreeSet<Cid>) -> ((u64, Cid), Vec<Effect>) {
    let mut seen = fx.clone();
    let mut fx = fx;
    for _ in 0..64 {
        if let Some(head) = head_sent(&fx) {
            return (head, seen);
        }
        let mut next = Vec::new();
        for id in puts(&fx) {
            if !hold.contains(&id) {
                next.extend(h.step(Event::PutConfirmed(id)));
            }
        }
        seen.extend(next.iter().cloned());
        fx = next;
    }
    panic!("THE SETUP: the commit never sent its head");
}

// ------------------------------------------------------------------------------------------------------------------
// THE REFERENCE: the commit table, as a function.
// ------------------------------------------------------------------------------------------------------------------

/// The table's columns, as the model steps them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Col {
    /// E1's trigger: a write arrives.
    Write,
    /// E2.
    Ack,
    /// E3.
    Fail,
    /// E4, on the Heading commit's ROOT.
    RejectRoot,
    /// E4, on another block of the commit in flight.
    RejectOwn,
    /// E6.
    Mine,
    /// E7.
    Older,
    /// E8.
    Foreign,
    /// E9.
    RefusedThis,
    /// E10.
    RefusedOther,
    /// E11 / E12: a tick.
    Tick,
}

/// What a cell says happens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cell {
    /// Any stage may follow (Idle: E1 may or may not cut).
    Free,
    /// The stage stays, and no write of the cut is told a terminal fate.
    Stay,
    /// Racing × E2: Racing, or Heading once race-ready.
    MayHead,
    /// Heading × E7: stays, and the head is RE-ISSUED (C2, A2).
    Reissue,
    /// The commit ends; the next stage is Idle or the next cut's Racing (K1).
    End(End),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum End {
    Published,
    Failed,
    /// A foreign head while Racing: the head never left -- the witness is not read and no try is spent (A8).
    DeadRacing,
    /// A foreign head while Heading: the witness decides (⁵).
    DeadHeading,
}

/// THE TABLE (COMMIT-LIFE rev 5, § The commit table).
fn cell(stage: CommitStage, col: Col) -> Cell {
    match (stage, col) {
        (CommitStage::Idle, _) => Cell::Free,
        (CommitStage::Racing, Col::Write | Col::Fail | Col::Older | Col::RefusedOther | Col::Tick) => Cell::Stay,
        (CommitStage::Racing, Col::Ack) => Cell::MayHead,
        (CommitStage::Racing, Col::RejectOwn | Col::RejectRoot) => Cell::End(End::Failed),
        (CommitStage::Racing, Col::Foreign) => Cell::End(End::DeadRacing),
        // Impossible cells: the model never steps them (no head was sent for this seq).
        (CommitStage::Racing, Col::Mine | Col::RefusedThis) => unreachable!("an impossible cell was stepped"),
        (CommitStage::Heading, Col::Write | Col::Ack | Col::Fail | Col::RejectOwn | Col::RefusedOther | Col::Tick) => Cell::Stay,
        (CommitStage::Heading, Col::RejectRoot) => Cell::End(End::Failed),
        (CommitStage::Heading, Col::Mine) => Cell::End(End::Published),
        (CommitStage::Heading, Col::Older) => Cell::Reissue,
        (CommitStage::Heading, Col::Foreign) => Cell::End(End::DeadHeading),
        (CommitStage::Heading, Col::RefusedThis) => Cell::End(End::Failed),
    }
}

// ------------------------------------------------------------------------------------------------------------------
// THE MODEL.
// ------------------------------------------------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

struct Model {
    h: Harness,
    rng: Rng,
    now: u64,
    next_write: u64,
    empty: Cid,
    /// PUTs out and not yet answered.
    out: BTreeSet<Cid>,
    /// The witness told before the last foreign head.
    witness: Option<Witness>,
    /// The PUTs of the commit in flight (since it started).
    mine: BTreeSet<Cid>,
    /// The head sent for the commit in flight.
    head: Option<(u64, Cid)>,
    /// Each write's terminal fates told.
    fates: BTreeMap<u64, Vec<State>>,
    /// Writes whose commit sent a head (only these may be `Lost` or `Unknown`: K6, A8).
    headed: BTreeSet<u64>,
    reach: BTreeMap<(String, Col), u64>,
}

impl Model {
    fn new(seed: u64) -> Self {
        let h = harness(Params::default());
        let empty = h.published_root();
        Model { h, rng: Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1), now: T0, next_write: 1, empty, out: BTreeSet::new(), witness: None, mine: BTreeSet::new(), head: None, fates: BTreeMap::new(), headed: BTreeSet::new(), reach: BTreeMap::new() }
    }

    /// The writes the commit in flight carries.
    fn cut(&self) -> Vec<u64> {
        self.h.engine().queue_stages().filter(|(_, _, s)| *s == engine::Stage::Committing).map(|(_, w, _)| w.0).collect()
    }

    /// Pick a column the stage can be stepped in, and its event.
    fn pick(&mut self, stage: CommitStage) -> (Col, Event) {
        loop {
            let col = match self.rng.below(20) {
                0..=1 => Col::Write,
                2..=8 => Col::Ack,
                9 => Col::Fail,
                10 => Col::RejectRoot,
                11 => Col::RejectOwn,
                12..=13 => Col::Mine,
                14 => Col::Older,
                15 => Col::Foreign,
                16 => Col::RefusedThis,
                17 => Col::RefusedOther,
                _ => Col::Tick,
            };
            let pick_out = |m: &mut Model| -> Option<Cid> {
                if m.out.is_empty() {
                    return None;
                }
                let i = m.rng.below(m.out.len() as u64) as usize;
                m.out.iter().nth(i).copied()
            };
            let ev = match col {
                Col::Write => {
                    let id = self.next_write;
                    self.next_write += 1;
                    write(id)
                }
                Col::Ack => match pick_out(self) {
                    Some(id) => Event::PutConfirmed(id),
                    None => continue,
                },
                Col::Fail => match pick_out(self) {
                    Some(id) => Event::PutFailed(id),
                    None => continue,
                },
                Col::RejectRoot => match (stage, self.head) {
                    (CommitStage::Heading, Some((_, root))) if self.out.contains(&root) => Event::PutRejected(root),
                    _ => continue,
                },
                // Only in Racing: a Heading commit's other blocks may be members whose group still reaches k, which
                // the named cases cover; the model keeps to the cells it can judge from outside.
                Col::RejectOwn => {
                    let own: Vec<Cid> = self.mine.intersection(&self.out).copied().collect();
                    match stage {
                        // With no Backing open, every id of `mine` is the commit in flight's (a published commit's
                        // held-back parity goes out in the step the next commit starts).
                        CommitStage::Racing if !own.is_empty() && self.h.engine().backing() == 0 => Event::PutRejected(own[self.rng.below(own.len() as u64) as usize]),
                        _ => continue,
                    }
                }
                Col::Mine => match (stage, self.head) {
                    (CommitStage::Heading, Some((seq, _))) => Event::HeadConfirmed(seq),
                    _ => continue,
                },
                Col::Older => match stage {
                    CommitStage::Idle => continue,
                    _ => Event::HeadMissing,
                },
                Col::Foreign => {
                    let seq = self.h.engine().committing_seq().unwrap_or(self.h.engine().published_seq() + 1);
                    let witness = match self.rng.below(3) {
                        0 => None,
                        1 => Some(Witness::NotThere),
                        _ => Some(Witness::Unknown),
                    };
                    self.witness = witness;
                    self.h.engine_mut().set_witness(witness);
                    Event::HeadConflict { seq, root: self.empty }
                }
                Col::RefusedThis => match (stage, self.head) {
                    (CommitStage::Heading, Some((seq, _))) => Event::HeadRefused { seq },
                    _ => continue,
                },
                Col::RefusedOther => Event::HeadRefused { seq: self.h.engine().published_seq() + 1000 },
                Col::Tick => {
                    self.now += if self.rng.below(4) == 0 { 30 } else { 1 };
                    Event::Tick(self.now)
                }
            };
            return (col, ev);
        }
    }

    /// Step one event and judge it against its cell.
    fn step(&mut self, seed: u64, n: usize, col: Col, ev: Event) {
        let before = self.h.engine().commit_stage();
        let cut = self.cut();
        let owed: BTreeSet<Cid> = if before == CommitStage::Idle { BTreeSet::new() } else { self.mine.intersection(&self.out).copied().collect() };
        let backing_before = self.h.engine().backing();
        let rejected_root = matches!((&ev, self.head), (Event::PutRejected(id), Some((_, root))) if *id == root);
        let fx = self.h.step(ev);
        *self.reach.entry((format!("{before:?}"), col)).or_default() += 1;
        let after = self.h.engine().commit_stage();
        let at = format!("seed {seed} step {n}: {before:?} × {col:?}");

        // Bookkeeping from what the engine emitted.
        // In order: an end withdraws a dead commit's PUT and the next cut may put the same block again, in one step.
        for f in &fx {
            if let Effect::PutBlock { id, .. } | Effect::PutPack { id, .. } = f {
                self.out.insert(*id);
            }
            if let Effect::Withdraw { id } = f {
                self.out.remove(id);
            }
        }
        for f in &fx {
            if let Effect::Notify { write_id, state, .. } = f {
                if terminal(*state) {
                    self.fates.entry(write_id.0).or_default().push(*state);
                    if matches!(state, State::Lost | State::Unknown) {
                        assert!(self.headed.contains(&write_id.0), "{at}: write {} told {state:?}, but no commit of it ever sent a head (K6, A8)", write_id.0);
                    }
                }
            }
        }
        let ended = |w: &u64| told(&fx, *w).into_iter().any(terminal);

        assert_eq!(self.h.engine().impossible_transitions(), 0, "{at}: an impossible transition was counted");
        match cell(before, col) {
            Cell::Free => {}
            Cell::Stay => {
                assert_eq!(after, before, "{at}: the stage moved in a Stay cell (C5 for a Tick: no end on time alone)");
                assert!(!cut.iter().any(ended), "{at}: a write of the cut was told a terminal fate in a Stay cell");
            }
            Cell::MayHead => {
                assert!(matches!(after, CommitStage::Racing | CommitStage::Heading), "{at}: Racing × an ack left {after:?}");
                assert!(!cut.iter().any(ended), "{at}: an ack ended a write");
            }
            Cell::Reissue => {
                assert_eq!(after, CommitStage::Heading, "{at}: Heading × an older head left Heading (C2, A2)");
                assert!(head_sent(&fx).is_some(), "{at}: Heading × an older head did not RE-ISSUE the head");
            }
            Cell::End(end) => {
                assert!(matches!(after, CommitStage::Idle | CommitStage::Racing), "{at}: an end left {after:?}");
                for w in &cut {
                    let t = told(&fx, *w);
                    match end {
                        End::Published => assert!(t.contains(&State::Published), "{at}: write {w} not Published: {t:?}"),
                        End::Failed => assert!(t.iter().any(|s| matches!(s, State::Failed { .. })), "{at}: write {w} not Failed: {t:?}"),
                        End::DeadRacing => assert!(!t.iter().any(|s| terminal(*s)), "{at}: write {w} of a never-headed commit was told {t:?} (A8: it goes again)"),
                        End::DeadHeading => {}
                    }
                }
                // A1 / C3: a non-Published end withdraws what the dead commit still had on the way (none of it is an
                // earlier commit's straggler while no Backing is open) -- except a Heading commit whose witness is
                // `Unknown`: it may have landed, and its blocks may be in the published tree.
                let may_have_landed = end == End::DeadHeading && self.witness == Some(Witness::Unknown);
                if end != End::Published && backing_before == 0 && !rejected_root && !may_have_landed {
                    let again: BTreeSet<Cid> = puts(&fx).into_iter().collect();
                    let gone = withdrawn(&fx);
                    let left: Vec<&Cid> = owed.iter().filter(|id| !again.contains(*id) && !gone.contains(*id)).collect();
                    assert!(left.is_empty(), "{at}: {end:?} left {} of the dead commit's PUTs un-withdrawn (A1)", left.len());
                }
            }
        }
        // A new commit started in this step (from Idle, or after an end): its PUTs are this step's.
        let started = after != CommitStage::Idle && (before == CommitStage::Idle || matches!(cell(before, col), Cell::End(_)));
        if started {
            self.mine = puts(&fx).into_iter().collect();
        } else {
            self.mine.extend(puts(&fx));
        }
        if let Some(head) = head_sent(&fx) {
            self.head = Some(head);
            self.headed.extend(self.cut());
        }
        if after == CommitStage::Idle || matches!(cell(before, col), Cell::End(_)) && head_sent(&fx).is_none() {
            self.head = None;
        }
        if after == CommitStage::Racing && before != CommitStage::Racing {
            self.head = None;
        }
    }

    /// Faults stop: every PUT answered, every head shown, until nothing is in flight.
    fn rest(&mut self, seed: u64) {
        for _ in 0..500 {
            let stage = self.h.engine().commit_stage();
            if stage == CommitStage::Idle && self.h.engine().queued_writes() == 0 {
                return;
            }
            let ev = match (stage, self.head, self.out.iter().next().copied()) {
                (CommitStage::Heading, Some((seq, _)), _) => (Col::Mine, Event::HeadConfirmed(seq)),
                (_, _, Some(id)) => (Col::Ack, Event::PutConfirmed(id)),
                _ => {
                    self.now += 1;
                    (Col::Tick, Event::Tick(self.now))
                }
            };
            if let Event::PutConfirmed(id) = &ev.1 {
                self.out.remove(id);
            }
            self.step(seed, usize::MAX, ev.0, ev.1);
        }
        panic!("seed {seed}: never came to rest: {:?}, {} queued", self.h.engine().commit_stage(), self.h.engine().queued_writes());
    }
}

#[test]
fn the_engine_follows_the_commit_table() {
    let mut reach: BTreeMap<(String, Col), u64> = BTreeMap::new();
    for seed in 1..=48u64 {
        let mut m = Model::new(seed);
        for n in 0..300 {
            let stage = m.h.engine().commit_stage();
            let (col, ev) = m.pick(stage);
            if let Event::PutConfirmed(id) | Event::PutFailed(id) | Event::PutRejected(id) = &ev {
                m.out.remove(id);
            }
            m.step(seed, n, col, ev);
        }
        m.rest(seed);
        for w in 1..m.next_write {
            let f = m.fates.get(&w).cloned().unwrap_or_default();
            assert_eq!(f.len(), 1, "seed {seed}: write {w} was told {} terminal fates at rest: {f:?} (K6)", f.len());
        }
        assert_eq!(m.h.engine().stalled_told(), 0, "seed {seed}: a stalled-told record outlived its write (A5, C4)");
        for (k, v) in m.reach {
            *reach.entry(k).or_default() += v;
        }
    }
    for ((stage, col), n) in &reach {
        println!("reach {stage} × {col:?}: {n}");
    }
    // ZEROS NAMED (WRITE-PATH's lesson): impossible cells, cells the model does not pick (Heading × a non-root own
    // block: the named cases judge it), and Idle cells that need a commit.
    let cols = [Col::Write, Col::Ack, Col::Fail, Col::RejectRoot, Col::RejectOwn, Col::Mine, Col::Older, Col::Foreign, Col::RefusedThis, Col::RefusedOther, Col::Tick];
    for stage in ["Idle", "Racing", "Heading"] {
        for col in cols {
            if !reach.contains_key(&(stage.to_string(), col)) {
                println!("reach {stage} × {col:?}: 0");
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------------------------
// THE NAMED DEFECT CELLS (COMMIT-LIFE § Defects).
// ------------------------------------------------------------------------------------------------------------------

/// **A6, Racing × E12:** a Racing commit whose PUTs are never answered is NEVER ended by time (rule 8, C5): no try
/// spent, no `Lost`, the same commit in flight; when the acks come, it publishes.
#[test]
fn a6_a_silent_racing_commit_is_never_ended_on_time() {
    let mut h = harness(Params::default());
    let first = h.step(write(1));
    let seq = h.engine().committing_seq().expect("THE SETUP: no commit");
    let mut all = puts(&first);
    let mut later = Vec::new();
    for t in 1..=3_000u64 {
        let fx = h.step(Event::Tick(T0 + t));
        all.extend(puts(&fx));
        later.extend(fx);
    }
    let fates: Vec<State> = told(&later, 1).into_iter().filter(|s| terminal(*s)).collect();
    assert!(fates.is_empty(), "a silent Racing commit's write was ended on time: {fates:?}");
    let ended = withdrawn(&later);
    assert!(ended.is_empty(), "a silent Racing commit was ended on time and restarted: {} of its PUTs withdrawn", ended.len());
    assert_eq!(h.engine().lost_fell(), (0, 0), "a try was spent on silence");
    assert_eq!((h.engine().commit_stage(), h.engine().committing_seq()), (CommitStage::Racing, Some(seq)), "the commit in flight did not stay Racing");
    let ((seq, _), _) = to_heading(&mut h, all.iter().map(|id| Effect::PutBlock { id: *id, bytes: Vec::new(), after: Vec::new() }).collect(), &BTreeSet::new());
    let fx = h.step(Event::HeadConfirmed(seq));
    assert!(told(&fx, 1).contains(&State::Published), "the commit did not publish when its acks came");
}

/// **A8, Racing × E8:** a foreign head while Racing -- this commit's head never left -- reads NO witness (an
/// `Unknown` would say "may have been saved" of a group that cannot have landed) and spends NO try: ten foreign moves
/// in a row, and the write is never `Lost` or `Unknown`, and publishes. Mutant "spend a try" -> red.
#[test]
fn a8_foreign_moves_while_racing_read_no_witness_and_spend_no_try() {
    let mut h = harness(Params::default());
    let empty = h.published_root();
    let mut fx = h.step(write(1));
    let mut told_all = Vec::new();
    let mut rounds = 0;
    for round in 0..10u64 {
        // The write leaving the queue here IS the defect (it was told a fate), asserted below.
        let Some(seq) = h.engine().committing_seq() else { break };
        assert_eq!(h.engine().commit_stage(), CommitStage::Racing, "THE SETUP: round {round} not Racing");
        rounds += 1;
        let witness = if round % 2 == 0 { Some(Witness::Unknown) } else { None };
        h.engine_mut().set_witness(witness);
        fx = h.step(Event::HeadConflict { seq, root: empty });
        told_all.extend(told(&fx, 1));
    }
    let bad: Vec<&State> = told_all.iter().filter(|s| matches!(s, State::Lost | State::Unknown)).collect();
    assert!(bad.is_empty(), "a write whose head never left was told {bad:?} after {rounds} foreign moves while Racing");
    assert_eq!(rounds, 10, "the write left the queue after {rounds} foreign moves while Racing: told {told_all:?}");
    let ((seq, _), _) = to_heading(&mut h, fx, &BTreeSet::new());
    let done = h.step(Event::HeadConfirmed(seq));
    assert!(told(&done, 1).contains(&State::Published), "the write did not publish after the foreign moves: {:?}", told(&done, 1));
}

/// **A9, Heading × E4 on the root:** the ROOT is a group of one with parity, so the head is signed with any 1 of it --
/// the root itself un-acked. The signer refuses a head whose root is not on the node (`RootNotHeld`, retryable), and a
/// REJECTED root never will be: the commit must end `Failed`, never sit in Heading re-asked for ever.
#[test]
fn a9_a_rejected_root_ends_a_heading_commit_failed() {
    let mut h = harness(Params::default());
    let fx = h.step(write(1));
    let root = h.root();
    let ((_, sent_root), _) = to_heading(&mut h, fx, &BTreeSet::from([root]));
    assert_eq!(sent_root, root, "THE SETUP: the head names another root");
    let fx = h.step(Event::PutRejected(root));
    assert!(told(&fx, 1).contains(&State::Failed { why: engine::FailWhy::BlockRejected }), "a Heading commit whose root was rejected did not end Failed: {:?}", told(&fx, 1));
    assert_eq!(h.engine().commit_stage(), CommitStage::Idle, "the commit whose root can never land stayed in flight");
}

/// **A2, Heading × E7 (the `head_before_packs` control arm):** a head read older than the commit RE-ISSUES the head;
/// Heading never goes back to Racing (C2).
#[test]
fn a2_heading_never_goes_back_to_racing() {
    let mut h = harness(Params { head_before_packs: true, ..Params::default() });
    let first = h.step(big_write(1));
    // head_before_packs sends the head on the FIRST confirmation, before race-ready: a leaf, not the root.
    let root = h.root();
    let one = puts(&first).into_iter().find(|id| *id != root).expect("THE SETUP: no non-root PUT");
    let fx = h.step(Event::PutConfirmed(one));
    let (seq, _) = head_sent(&fx).expect("THE SETUP: head_before_packs did not send the head on the first ack");
    assert_eq!(h.engine().commit_stage(), CommitStage::Heading);
    let fx = h.step(Event::HeadMissing);
    assert_eq!(h.engine().commit_stage(), CommitStage::Heading, "Heading went back to Racing on an older head");
    assert_eq!(head_sent(&fx).map(|(s, _)| s), Some(seq), "the head was not re-issued");
}

/// **A5, Heading × E6 (and the witness's landed end):** a write told `Stalled` that then publishes leaves no stalled
/// record behind (C4): the record goes with the write.
#[test]
fn a5_a_stalled_write_that_publishes_leaves_no_stalled_record() {
    let mut h = harness(Params::default());
    let first = h.step(write(1));
    let mut stalled = Vec::new();
    for t in 1..=100u64 {
        stalled.extend(h.step(Event::Tick(T0 + t)));
    }
    assert!(told(&stalled, 1).contains(&State::Stalled), "THE SETUP: the write was never told Stalled");
    let ((seq, _), _) = to_heading(&mut h, first, &BTreeSet::new());
    let fx = h.step(Event::HeadConfirmed(seq));
    assert!(told(&fx, 1).contains(&State::Published), "THE SETUP: not published");
    assert_eq!(h.engine().stalled_told(), 0, "a published write is still recorded as told Stalled");
}

/// **A1, Racing × E8 (and every non-Published end):** the dead commit's PUTs still on the way are WITHDRAWN (C3), as
/// `fail_commit` does -- not left re-sending for a commit that no longer exists.
#[test]
fn a1_a_dead_commits_puts_are_withdrawn() {
    let mut h = harness(Params::default());
    // A published base, so the dead commit's blocks differ from its re-application's on the foreign (empty) root.
    let empty = h.published_root();
    let fx = h.step(write(1));
    let ((seq, _), seen) = to_heading(&mut h, fx, &BTreeSet::new());
    let landed = h.step(Event::HeadConfirmed(seq));
    // Drain the first commit's Backing (its held-back parity), so nothing of an earlier commit is still owed.
    for id in puts(&seen).into_iter().chain(puts(&landed)) {
        let _ = h.step(Event::PutConfirmed(id));
    }
    assert_eq!(h.engine().backing(), 0, "THE SETUP: the first commit's Backing did not drain");
    let fx = h.step(write(2));
    let dead: BTreeSet<Cid> = puts(&fx).into_iter().collect();
    assert!(!dead.is_empty() && h.engine().commit_stage() == CommitStage::Racing, "THE SETUP: the second commit is not Racing");
    let seq = h.engine().committing_seq().expect("in flight");
    let fx = h.step(Event::HeadConflict { seq, root: empty });
    let again: BTreeSet<Cid> = puts(&fx).into_iter().collect();
    let gone = withdrawn(&fx);
    let left: Vec<&Cid> = dead.iter().filter(|id| !again.contains(*id) && !gone.contains(*id)).collect();
    assert!(left.is_empty(), "{} of the dead commit's {} PUTs were left un-withdrawn by its foreign end", left.len(), dead.len());
}

/// **Heading × E8, the group LANDED (⁵):** its confirmation lost, a foreign head WITNESSES the group
/// (`through` at its last arrival). Its blocks are in the published tree, so the one teardown never withdraws them
/// (C3 withdraws only what did not land): what it still owes goes on toward BACKED_UP as a Backing. Mutant "withdraw on
/// every Dead end" -> red.
#[test]
fn a_landed_groups_blocks_are_kept_not_withdrawn() {
    let mut h = harness(Params::default());
    let fx = h.step(big_write(1));
    let root = h.root();
    // Heading with the root still owed: its parity made the group of one ready.
    let ((seq, _), _) = to_heading(&mut h, fx, &BTreeSet::from([root]));
    let through = h.engine().committing_through().expect("in flight");
    h.engine_mut().set_witness(Some(Witness::Through(through)));
    let fx = h.step(Event::HeadConflict { seq, root: h.published_root() });
    assert!(told(&fx, 1).contains(&State::Published), "THE SETUP: the landed group was not Published: {:?}", told(&fx, 1));
    let gone = withdrawn(&fx);
    assert!(!gone.contains(&root), "a block of a group that LANDED was withdrawn");
    assert!(h.engine().backing() > 0, "what the landed commit still owes is tracked by no Backing");
}

/// **A9, Heading x E4, the BELOW-k half** (the `head_before_packs` arm: the only one that enters Heading before
/// race-ready): rejecting a commit's blocks one by one -- never its root -- until some changed group can no longer
/// reach k ends the commit `Failed` at that rejection; before it, each rejection leaves it in flight (the head may
/// land at k). Mutant "only the root ends it" -> red.
#[test]
fn a9_a_group_rejected_below_k_ends_a_heading_commit_failed() {
    let mut h = harness(Params { head_before_packs: true, ..Params::default() });
    let first = h.step(wide_write(1));
    let root = h.root();
    let own: Vec<Cid> = puts(&first).into_iter().filter(|id| *id != root).collect();
    let one = *own.first().expect("THE SETUP: no non-root PUT");
    let fx = h.step(Event::PutConfirmed(one));
    assert!(head_sent(&fx).is_some(), "THE SETUP: head_before_packs did not send the head on the first ack");
    let mut ended = None;
    for (n, id) in own.iter().skip(1).enumerate() {
        let fx = h.step(Event::PutRejected(*id));
        if told(&fx, 1).contains(&State::Failed { why: engine::FailWhy::BlockRejected }) {
            ended = Some(n + 1);
            break;
        }
        assert_eq!(h.engine().commit_stage(), CommitStage::Heading, "rejection {} ended the commit without telling Failed", n + 1);
    }
    let n = ended.expect("no group fell below k however many blocks were rejected: the commit sat in Heading");
    println!("Failed at rejection {n} of {}", own.len() - 1);
    assert_eq!(h.engine().commit_stage(), CommitStage::Idle);
}
