//! The node's block store, as a test sees it.
//!
//! `Blocks::get` hands out `Option<&[u8]>` tied to `&self`, so a store that
//! can be written to WHILE an engine holds it cannot hand out borrows of
//! anything it might later move. This one keeps each block's bytes at a fixed
//! address for the lifetime of the process and hands out `&'static [u8]`.
//!
//! That is a leak, and it is deliberate and test-only: a test process writes a
//! bounded number of blocks and then exits. The alternative is threading a
//! lifetime through every engine and harness, which would shape the production
//! type around a convenience of the tests.

#![allow(dead_code)]

use engine::{ClientId, Effect, Engine, Event, Op, Params, State, WriteId};
use freenet_prolly::build::TreeBuilder;
use freenet_prolly::store::Blocks;
use freenet_prolly::store::MemBlocks;
use freenet_prolly::Cid;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

thread_local! {
    /// One store per test thread.
    ///
    /// `Store::default()` hands back a handle to it, so a test and the engine
    /// it built share the node's blocks without the test threading a value
    /// through every helper. Thread-local, so tests running in parallel do
    /// not see each other's blocks — the isolation a global would lose.
    ///
    /// This is test scaffolding. The engine itself has no global state, which
    /// `no_global_state_in_the_engine` asserts.
    static THREAD_STORE: Inner = Inner::default();
}

#[derive(Clone, Default)]
struct Inner {
    blocks: Rc<RefCell<BTreeMap<Cid, &'static [u8]>>>,
    forgotten: Rc<RefCell<std::collections::BTreeSet<Cid>>>,
    reads: Rc<std::cell::Cell<usize>>,
}

#[derive(Clone)]
pub struct Store {
    inner: Rc<RefCell<BTreeMap<Cid, &'static [u8]>>>,
    /// Blocks this store pretends not to have, for the forgetful case.
    forgotten: Rc<RefCell<std::collections::BTreeSet<Cid>>>,
    /// Every `Blocks::get`, counted. A cost claim about a walk that is
    /// supposed to read NOTHING can only be checked by something that would
    /// see it if it read one block.
    reads: Rc<std::cell::Cell<usize>>,
}

impl Default for Store {
    fn default() -> Self {
        THREAD_STORE.with(|i| Store {
            inner: i.blocks.clone(),
            forgotten: i.forgotten.clone(),
            reads: i.reads.clone(),
        })
    }
}

/// Step the engine and let the node absorb what it emitted.
///
/// The engine keeps no blocks, so a test that only calls `step` leaves the
/// tree nowhere: the next call reads through `Blocks` and finds nothing. On a
/// real node the puts land and the node holds them; here the store does it.
/// Every converted test goes through this rather than `step` directly.
/// A macro rather than a function so it works whether the engine is an owned
/// binding or a `&mut` parameter — the call sites are both, and a function
/// would need each one to spell its own borrow.
#[macro_export]
macro_rules! stepped {
    ($e:expr, $ev:expr) => {{
        let mut out = $e.step($ev);
        // Into the engine's OWN store: a test that builds an engine over an
        // isolated store would otherwise absorb into the thread's, and every
        // block it emitted would be unreadable to the engine that emitted it.
        $e.blocks().absorb(&out);
        {
            use common::AnswerHeld as _;
            $e.answer_held(&mut out);
        }
        out
    }};
}

/// Answer every `ConfirmHeld` in `out` AS THE PAGE DOES (sdk#416, sdk#454): a block the node holds with
/// `PutConfirmed`, one it does not with `HeldUnknown` (the page's `AskHeld` answered absent), at once -- never left
/// unanswered, which production never does. The store is these tests' node. A test that plays the page itself says so
/// by name (`Rig::withhold_held` in race_put) and answers each `ConfirmHeld` itself.
#[allow(dead_code)]
pub trait AnswerHeld {
    fn answer_held(&mut self, out: &mut Vec<Effect>);
}

impl AnswerHeld for Engine<Store> {
    fn answer_held(&mut self, out: &mut Vec<Effect>) {
        answer_held(self, out)
    }
}

#[allow(dead_code)]
pub fn answer_held(e: &mut Engine<Store>, out: &mut Vec<Effect>) {
    let mut i = 0;
    while i < out.len() {
        if let Effect::ConfirmHeld { id } = out[i] {
            let held = e.blocks().node_holds(&id);
            let more = e.step(if held { Event::PutConfirmed(id) } else { Event::HeldUnknown(id) });
            e.blocks().absorb(&more);
            out.extend(more);
        }
        i += 1;
    }
}

/// A HAND-WRITTEN DRIVER'S CATCH-ALL (the architect on #413 x #424): an effect that owes the engine no answer may be
/// ignored; one the engine WAITS on (a fetch, a put, a head, a held question) may not -- dropping it is a silent stall,
/// which is how #424's `ConfirmHeld` stalled #413's read_repair driver without an error. Every driver over `Effect`
/// ends in `other => common::no_answer_owed(&other)`, and `drivers_have_no_silent_catch_all` holds that by source.
#[allow(dead_code)]
#[track_caller]
pub fn no_answer_owed(f: &Effect) {
    match f {
        Effect::Notify { .. }
        | Effect::Progress { .. }
        | Effect::Subscribed { .. }
        | Effect::Changed { .. }
        | Effect::Conflicted { .. }
        | Effect::Unread { .. }
        | Effect::Reply { .. }
        | Effect::Withdraw { .. }
        // Nobody wants the block any more: the page ends its GET; no engine wait hangs on it (WANTED-LIFE).
        | Effect::Unwanted { .. }
        | Effect::Keep { .. }
        // A repair's re-put: its answer confirms nothing (the architect's (b)), so nothing in the engine waits on it.
        | Effect::PutRepaired { .. }
        // The node's answer for audit passes (sdk#530): the audit's, and nothing in the engine waits on it.
        | Effect::AuditAnswered { .. } => {}
        owed => panic!("a test driver dropped an effect the engine waits on (a silent stall): {owed:?}"),
    }
}

/// `Engine::new` with the thread's store, so a converted test reads as it did.
///
/// The engine is told the tree is NEW (`HeadMissing`), because a write made
/// before the head is recovered waits for it (sdk#223) and these tests write
/// straight away. A test about recovery itself sends `Start` and its own
/// answer, which resets this.
pub fn new_store_params(params: Params) -> Engine<Store> {
    let mut e = Engine::new(params, Store::default());
    let _ = e.step(Event::HeadMissing);
    e
}

/// What a refusal must leave untouched: the engine's bookkeeping BYTE FOR
/// BYTE (`state_digest`, the one field walk `context_len` also sizes), plus
/// what lives outside it -- the commit in flight's seq, the queue stage by
/// stage, owed parity and the counters a taken write moves.
#[allow(dead_code)]
pub fn fingerprint(e: &Engine<Store>) -> impl PartialEq + std::fmt::Debug {
    (
        e.state_digest(),
        e.committing_seq(),
        e.queue_stages().collect::<Vec<_>>(),
        (e.queue_load(), e.queued_writes(), e.unsaved_writes()),
        (e.owed_groups(), e.commits_and_writes(), e.forced_writes()),
    )
}

/// A COLD READER of `root`: an engine that has adopted it and holds none of
/// its blocks, on a store of its own (returned, for what the page keeps).
/// A reader on a store of its own that has NOT adopted anything: it learns
/// its head the way a page does, by a `HeadRead` (with the head's mark told
/// first, as the page tells it).
pub fn fresh_reader(params: Params) -> Engine<Store> {
    Engine::new(params, Store::fresh())
}

pub fn cold_reader(root: Cid, params: Params) -> (Engine<Store>, Store) {
    let store = Store::fresh();
    let mut e = Engine::new(params, store.clone());
    e.adopt_root_for_test(root);
    (e, store)
}

impl Store {
    /// A store of its very own, for a test that wants two.
    pub fn fresh() -> Self {
        Store {
            inner: Rc::new(RefCell::new(BTreeMap::new())),
            forgotten: Rc::new(RefCell::new(std::collections::BTreeSet::new())),
            reads: Rc::new(std::cell::Cell::new(0)),
        }
    }

    /// Blocks handed out since this store was made.
    pub fn reads(&self) -> usize {
        self.reads.get()
    }
}

impl Blocks for Store {
    fn get(&self, cid: &Cid) -> Option<&[u8]> {
        // Counted BEFORE the forgotten check: an ASK is a read whether or not
        // the store chooses to answer it, and counting only the hits would
        // make a walk over blocks nobody holds look free.
        self.reads.set(self.reads.get() + 1);
        if self.forgotten.borrow().contains(cid) {
            return None;
        }
        self.inner.borrow().get(cid).copied()
    }
}

impl Store {
    pub fn put(&self, id: Cid, bytes: &[u8]) {
        // Re-putting the same block is normal on a network, and the leak is
        // per call, so dedupe: a fuzz loop puts the same id thousands of times.
        if self.inner.borrow().contains_key(&id) {
            return;
        }
        let leaked: &'static [u8] = Box::leak(bytes.to_vec().into_boxed_slice());
        self.inner.borrow_mut().insert(id, leaked);
    }

    pub fn len(&self) -> usize {
        self.inner.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Absorb what a commit emitted, the way a node does once its puts land.
    ///
    /// A pack is unpacked: what the node ends up holding is the blocks inside
    /// it, under their own ids, which is the whole point of the format.
    pub fn absorb(&self, effects: &[Effect]) {
        // Not a driver: it only STORES what was put (the driver answers every effect), so it picks out the puts and has no
        // catch-all to drop anything with.
        for f in effects {
            if let Effect::PutPack { id, bytes, .. } = f {
                for (mid, mbytes) in engine::pack::members(bytes) {
                    self.put(mid, &mbytes);
                }
                self.put(*id, bytes);
            }
            // A queued write's warm-apply block (R-b): the page keeps it.
            if let Effect::PutBlock { id, bytes, .. } | Effect::Keep { id, bytes } = f {
                self.put(*id, bytes);
            }
        }
    }

    /// Forget a block: the node evicted it.
    ///
    /// F33: a sync read does NOT refresh hosting, so a block a descent read on
    /// one call can be gone on the next, and nothing the engine does prevents
    /// it. A store that cannot express that cannot test for it.
    pub fn forget(&self, cid: Cid) {
        self.forgotten.borrow_mut().insert(cid);
    }

    pub fn remember(&self, cid: &Cid) {
        self.forgotten.borrow_mut().remove(cid);
    }

    /// The NODE loses a block (unlike `forget`, which models only the page's memory): `node_holds` says no after this,
    /// so the harness answers a `ConfirmHeld` for it as the page would -- `HeldUnknown` (sdk#454).
    #[allow(dead_code)]
    pub fn lose(&self, cid: &Cid) {
        self.inner.borrow_mut().remove(cid);
    }

    /// Does the NODE hold `cid`? A block `forget` hid is still the node's (it models the page's memory losing it),
    /// and is not counted as a read.
    #[allow(dead_code)]
    pub fn node_holds(&self, cid: &Cid) -> bool {
        self.inner.borrow().contains_key(cid)
    }
}

/// One long-lived engine, driven step by step as the page drives it (the
/// engine lives in the page, READ-STATE design B), with the node's store
/// absorbing whatever each step put.
pub struct Harness {
    pub store: Store,
    engine: Engine<Store>,
    /// What the engine shed, summed over every step (sdk#162).
    pub shed: engine::Shed,
}

impl Harness {
    pub fn new(params: Params, store: Store) -> Self {
        let mut engine = Engine::new(params, store.clone());
        // A NEW tree, and the engine is told so: a write before recovery
        // waits for it (sdk#223). A test that reads a head (`Start` then
        // `HeadRead`) resets and redoes this, as `Start` does.
        let _ = engine.step(Event::HeadMissing);
        Harness { store, engine, shed: engine::Shed::default() }
    }

    pub fn step(&mut self, ev: Event) -> Vec<Effect> {
        let mut out = self.engine.step(ev);
        let s = self.engine.take_shed();
        add(&mut self.shed, s);
        // The node holds what the commit put.
        self.store.absorb(&out);
        // And answers for a changed group's other members it holds (sdk#416), as the page does.
        answer_held(&mut self.engine, &mut out);
        out
    }

    /// Owed parity groups, as the engine stands.
    pub fn owed_groups(&self) -> usize {
        self.engine.owed_groups()
    }

    /// Who wants block `id` (WANTED-LIFE: a read, a repair, the parked write).
    #[allow(dead_code)]
    pub fn readers_of(&self, id: &Cid) -> engine::Readers {
        self.engine.readers_of(id)
    }

    /// The root the engine would report.
    pub fn root(&self) -> Cid {
        self.engine.root()
    }

    /// The engine's read repairs so far: `(started, rebuilt, given up)`.
    #[allow(dead_code)]
    pub fn repair_counts(&self) -> (u64, u64, u64) {
        self.engine.repair_counts()
    }

    pub fn published_root(&self) -> Cid {
        self.engine.published_root()
    }

    /// The engine, for what a test reads of it.
    #[allow(dead_code)]
    pub fn engine(&self) -> &Engine<Store> {
        &self.engine
    }

    /// The engine, for what a test tells it before a step (the witness, as the page does).
    #[allow(dead_code)]
    pub fn engine_mut(&mut self) -> &mut Engine<Store> {
        &mut self.engine
    }
}

/// A writer engine holding everything, and the blocks it produced.
///
/// A reader is given NONE of them: it starts from a published root and must
/// fetch its way to an answer, which is the situation a cold reader is
/// actually in.
///
/// Built by driving a real engine, so the fixture is the tree this code
/// actually writes rather than one a second builder agrees it should be.
/// Put every internal node's PARITY on the network too, as a writer's race put does (rule 10): a fixture read with
/// racing on (sdk#303) then asks real groups, not ids the network never had.
#[allow(dead_code)]
pub fn publish_parity(root: Cid, all: &mut MemBlocks) {
    let mut at = vec![root];
    while let Some(id) = at.pop() {
        let bytes = all.get(&id).expect("a tree block").to_vec();
        let Ok(n) = freenet_prolly::node::Node::parse(&bytes) else { continue };
        // A leaf's groups are its values stored by reference.
        for (pid, pb) in freenet_prolly::parity::blocks_of(&n, &*all).expect("every member held") {
            all.insert(pid, &pb);
        }
        if !n.is_leaf() {
            at.extend((0..n.len()).map(|i| n.child(i).0));
        }
    }
}

pub fn tree(records: &BTreeMap<Vec<u8>, Vec<u8>>) -> (Cid, MemBlocks) {
    // The writer gets a store of ITS OWN. Sharing the thread's store would put
    // the whole tree where the reader can see it, and every "cold" read below
    // would be a warm one — which is exactly what happened: five tests failed
    // with "0 fetches" because nothing was ever cold.
    let ws = Store::fresh();
    // The fixture writer is not subject to `max_commit_blocks`.
    //
    // That cap exists because a pack's bytes do not survive the `process()`
    // return that made them, so a live commit must fit in ONE return. This
    // writer has no returns: it builds a tree in process, to BE the fixture
    // the tests then read. Capping it would mean no test could use a tree
    // bigger than one live commit, which is the opposite of what a read test
    // wants — a deep tree is the whole point.
    //
    // A bulk import on a real node is the same shape and gets the same
    // answer the cap gives: split it, which is the client's outbox's job.
    let mut w = Engine::new(
        Params {
            max_commit_blocks: usize::MAX,
            // Never saves a context: an in-process writer has no bound to keep.
            max_context_bytes: usize::MAX,
            ..Params::default()
        },
        ws.clone(),
    );
    let ops: Vec<(Vec<u8>, Op)> = records
        .iter()
        .map(|(k, v)| (k.clone(), Op::Put(v.clone())))
        .collect();
    // A NEW tree: there is no head to recover, and the engine is told so. A
    // write before recovery waits for it (sdk#223), so this writer says it.
    let _ = w.step(Event::HeadMissing);
    let mut queue = {
        let out = w.step(Event::forced_write(ClientId(1), WriteId(1), ops));
        ws.absorb(&out);
        out
    };
    let mut all = MemBlocks::default();
    let mut guard = 0;
    while let Some(f) = queue.pop() {
        guard += 1;
        assert!(guard < 100_000, "the writer did not settle");
        match f {
            Effect::PutPack { id, bytes, .. } => {
                // A pack is a TRANSPORT. What the network ends up holding is
                // the blocks inside it, under their own ids — that is the
                // whole point of the format, and a fixture that kept only the
                // pack would model a network no reader could read.
                for (mid, mbytes) in engine::pack::members(&bytes) {
                    all.insert(mid, &mbytes);
                }
                all.insert(id, &bytes);
                let o = w.step(Event::PutConfirmed(id));
                ws.absorb(&o);
                queue.extend(o);
            }
            Effect::PutBlock { id, bytes, .. } => {
                all.insert(id, &bytes);
                let o = w.step(Event::PutConfirmed(id));
                ws.absorb(&o);
                queue.extend(o);
            }
            Effect::UpdateHead { seq, .. } => {
                let o = w.step(Event::HeadConfirmed(seq));
                ws.absorb(&o);
                queue.extend(o);
            }
            Effect::Notify {
                state: State::Published,
                ..
            } => {}
            other => no_answer_owed(&other),
        }
    }
    (w.published_root(), all)
}

/// The root a from-scratch build of the same records produces.
///
/// This is the oracle that matters: the pipeline may reorder, fold, retry and
/// coalesce as much as it likes, and the tree it ends up naming must be the
/// one the library would have built from the final records in one pass. It is
/// the INDEPENDENT side of the differential -- `tree` above descends from the
/// engine, and two sides that both did would only prove the engine agrees
/// with itself.
pub fn rebuild(records: &BTreeMap<Vec<u8>, Vec<u8>>) -> Cid {
    let mut t = TreeBuilder::new(|_, _: &[u8]| {});
    for (k, v) in records {
        t.push_bytes(k, v).unwrap();
    }
    t.finish().unwrap()
}

fn add(total: &mut engine::Shed, s: engine::Shed) {
    total.reads += s.reads;
    total.writes += s.writes;
    total.waits += s.waits;
    total.uncoded += s.uncoded;
}
