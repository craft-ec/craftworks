//! READ-STATE's MODEL TEST (craftworks-docs docs/design/READ-STATE.md,
//! § The model test).
//!
//! Written BEFORE slice R's code, against the read path of that day (a row
//! copy with range loads), and red there on the defects it exists to see: at
//! ca3f7c4, over seeds 0..40 × 120 steps, "READ IS NOT THE TREE AT ITS HEAD"
//! 8/40, "READ DID NOT ANSWER" (a ticketless NOT_LOADED) 14/40, "AT REST, Y'S
//! READ IS NOT THE FINAL TREE" 5/40. That reader is gone with its code; the
//! controls here are MUTATIONS of the walking reader instead (READ-STATE §
//! The model test, Controls).
//!
//! A seeded PRNG (splitmix64) drives ONE node and two tabs of one person: tab
//! x WRITES (so the head keeps moving) and tab y READS random ranges while it
//! adopts x's heads — and WRITES too, in bursts deep enough to queue behind
//! its own commit. The ORACLE is the node's own tree at the head y stands on,
//! read from the node, never from y, with y's own writes that are still in
//! flight applied on top in the order y made them (read-your-writes). A read
//! is right when it equals the oracle at the head y stood on when the read
//! began, or at the head it had adopted when it ended.
//!
//! Checked on every read: it equals the oracle (inv. 2), and it ENDS — an
//! answer within a bounded number of hops, never a ticketless refusal or a
//! loop (inv. 8). Checked at rest: y's writes settled, y on the final head,
//! y's full read is the node's final tree; no ticket left open; the engine's
//! queue drained; no write told `Busy` (R-b).
//!
//! A FAST phase: x writes on EVERY step while y reads on every step, so the
//! head moves faster than walks finish (the architect's chase).
//!
//! THE LOOP RULE (as `write_path_model.rs`): every loop that waits advances a
//! clock or is bounded, and fails by name.

use craftworks_sdk::{Ended, PageStore, Reads, Store, StoreError};
use protocol::Request;
use std::collections::BTreeMap;
use testkit::page_node::{PageConn, PageNode};

// ---------------------------------------------------------------- the dice

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Keys the model writes and reads: a small pool, so ranges overlap writes.
const KEYS: u64 = 48;
fn key(i: u64) -> Vec<u8> {
    format!("k/{i:03}").into_bytes()
}

/// The ranges a reader asks for: the whole space, halves, and a narrow slice —
/// wide loads and narrow reads is the shape that broke the row copy (defect 5).
fn range(r: &mut Rng) -> (Vec<u8>, Vec<u8>) {
    match r.below(4) {
        0 => (b"k/".to_vec(), b"l/".to_vec()),
        1 => (key(0), key(KEYS / 2)),
        2 => (key(KEYS / 2), b"l/".to_vec()),
        _ => {
            let a = r.below(KEYS - 4);
            (key(a), key(a + 4))
        }
    }
}

/// What one write leaves at each key (`None`: deleted).
type Ops = Vec<(Vec<u8>, Option<Vec<u8>>)>;

/// One write: one to three keys, puts and deletes.
fn ops(r: &mut Rng, step: usize, who: &str) -> Ops {
    let n = 1 + r.below(3);
    let mut m = BTreeMap::new();
    for _ in 0..n {
        let k = key(r.below(KEYS));
        let v = if r.below(5) == 0 { None } else { Some(format!("{who}{step}").into_bytes()) };
        m.insert(k, v);
    }
    m.into_iter().collect()
}

/// The FAST phase's values are WIDE: the tree spans many blocks, so a head
/// that moves changes blocks a read of another range has not got — without
/// it the whole tree is one or two blocks, adopting a head fetches them, and
/// no walk could ever chase.
const FAST_VALUE: usize = 700;

fn widen(o: Ops) -> Ops {
    o.into_iter().map(|(k, v)| (k, v.map(|mut v| { v.resize(FAST_VALUE, b'.'); v }))).collect()
}

// --------------------------------------------------------------- the reader

/// Rows in key order, as a read answers them.
type Rows = Vec<(Vec<u8>, Vec<u8>)>;

/// How a read ended when it did not answer.
#[derive(Debug, Clone, PartialEq)]
enum Fail {
    /// Refused with nothing to wait on (a ticketless NotLoaded).
    Ticketless(String),
    /// Still not answered after the bounded number of hops: a loop.
    Stuck(usize),
    /// Its ticket ended without the blocks (UNAVAILABLE).
    Ended(String),
    /// Refused for another reason.
    Other(String),
}

/// How many hops one read may take before it is a loop — `engine-db.js`'s
/// `MAX_HOPS`, the bound a page actually has.
const READ_ROUNDS: usize = 8;

/// The controls (READ-STATE § The model test): defects re-planted in the
/// walking reader, each of which the model must see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mutant {
    /// A head COPIED into the reader and walked from, never refreshed after
    /// the first read (defects 2–3: `head_root` stuck at the first head).
    HeadCopy,
    /// A woken read NOT resumed: the next walk chases the moving head, and
    /// each hop may find another block missing (the architect's chase).
    NoResume,
    /// A read covering this page's own write still APPLYING (in no root yet,
    /// R-b) no longer waits for it: it misses its own write
    /// (read-your-writes; the overlay's third case, R-a's live find).
    NoWaitOnApplying,
    /// LIVE never asked: the page does not diff its bindings on a head move
    /// (a tab that made no write could never see another's).
    DeafLive,
}

/// y's ONE LIVE binding: this range, re-read whenever the diff from what it
/// last rendered says the range changed.
const LIVE: (&[u8], &[u8]) = (b"k/", b"k/024");
const LIVE_KEY: &str = "live";

fn live_key() -> craftworks_sdk::live_bindings::WatchKey {
    craftworks_sdk::live_bindings::WatchKey::of(craftworks_sdk::app::StoredName::of_tree(LIVE_KEY.into()))
}

/// THE WALKING READER (slice R): a `PageStore` over tab y, read the way
/// `engine-db.js`'s `once` reads — a refusal's ticket waited on, then RESUMED
/// so the next walk is pinned to its root.
struct Walks {
    store: PageStore<PageConn>,
    conn: PageConn,
    /// Tickets that ended, as the page's drain collects them.
    ended: BTreeMap<u64, Ended>,
    /// A MUTATION for the controls, or `None` for the reader as shipped.
    mutant: Option<Mutant>,
    /// The head-copy mutant's copy.
    stale: Option<[u8; 32]>,
    /// The LIVE binding's `RenderedAt` (the SDK's), and what it shows.
    live: craftworks_sdk::LiveBindings,
    rendered: Option<Rows>,
    /// LIVE re-reads that did not answer, named.
    live_failed: Vec<String>,
}

impl Walks {
    fn open(node: &PageNode, mutant: Option<Mutant>) -> Walks {
        let (mut store, conn, _clock) = testkit::page_store(node);
        store.wait_on_applying = mutant != Some(Mutant::NoWaitOnApplying);
        let mut w = Walks { store, conn, ended: BTreeMap::new(), mutant, stale: None, live: Default::default(), rendered: None, live_failed: Vec::new() };
        w.drain();
        w.live.bind(live_key());
        w.rerender();
        w
    }

    /// Replies a direct call on the tab returned: the store's to read, as
    /// the page's pump hands them over.
    fn feed(&mut self, replies: Vec<Vec<u8>>) {
        for r in &replies {
            self.store.writes.on_inbound(r);
        }
        self.store.sync();
    }

    /// The node answers everything it was holding.
    fn release_all(&mut self) {
        self.conn.stop_holding();
        while self.conn.held() > 0 {
            let r = self.conn.release_one();
            self.feed(r);
        }
    }

    fn drain(&mut self) {
        for (t, how) in self.store.take_ended() {
            self.ended.insert(t, how);
        }
    }

    fn scan(&mut self, lo: &[u8], hi: &[u8]) -> Result<Rows, StoreError> {
        if self.mutant == Some(Mutant::HeadCopy) {
            let head = match self.stale {
                Some(h) => h,
                None => {
                    let h = self.store.head().ok_or(StoreError::NotLoaded)?;
                    self.stale = Some(h);
                    h
                }
            };
            self.store.pin(head);
        }
        self.store.scan(lo, hi, false, usize::MAX)
    }

    /// Read `[lo, hi)` as a page does. `meanwhile` runs while a hop waits on
    /// its ticket — in the fast phase it moves the head, so a walk that is
    /// not pinned to its ticket's root finds new blocks missing every hop.
    fn read(&mut self, lo: &[u8], hi: &[u8], meanwhile: &mut dyn FnMut(&mut Walks)) -> Result<Rows, Fail> {
        for hop in 0..READ_ROUNDS {
            let r = self.scan(lo, hi);
            self.store.unpin();
            match r {
                Ok(rows) => return Ok(rows),
                Err(StoreError::NotLoaded) => {
                    let Some(t) = self.store.take_ticket() else {
                        return Err(Fail::Ticketless(format!("hop {hop}")));
                    };
                    meanwhile(self);
                    // The page's drain, on every message, until the ticket
                    // ends: bounded, and a tick each time round.
                    let mut how = None;
                    for _ in 0..50 {
                        self.drain();
                        if let Some(h) = self.ended.remove(&t) {
                            how = Some(h);
                            break;
                        }
                        let now = self.conn.now_ms() + 100;
                        let r = self.conn.tick_at(now);
                        self.feed(r);
                    }
                    match how {
                        Some(Ended::Loaded) => {
                            if self.mutant != Some(Mutant::NoResume) {
                                self.store.resume(t);
                            }
                        }
                        Some(other) => return Err(Fail::Ended(other.code().into())),
                        None => return Err(Fail::Ended("never ended".into())),
                    }
                }
                Err(e) => return Err(Fail::Other(e.to_string())),
            }
        }
        Err(Fail::Stuck(READ_ROUNDS))
    }

    /// The head y stands on: its PUBLISHED head (the oracle's root).
    fn head(&self) -> (u64, [u8; 32]) {
        self.conn.with_server(|s| s.page.published())
    }

    fn hint(&mut self, now_ms: u64) {
        self.conn.with_server(|s| s.head_hint());
        self.tick(now_ms);
    }

    /// The LIVE binding's re-read: what it shows now.
    fn rerender(&mut self) {
        match self.read(LIVE.0, LIVE.1, &mut |_| {}) {
            Ok(rows) => {
                self.rendered = Some(rows);
                // RenderedAt: the root this read ANSWERED at, set now that it
                // has completed (as `engine-db.js` calls `session.rendered`).
                let at = self.store.answered_at();
                self.live.rendered(&live_key(), at);
            }
            Err(f) => self.live_failed.push(format!("{f:?}")),
        }
    }

    /// What the page's drain does after every message: the session says which
    /// LIVE ranges changed since each was last told, and those re-run.
    fn live_pump(&mut self) {
        if self.mutant == Some(Mutant::DeafLive) {
            return;
        }
        let head = self.store.head();
        let changed = self.live.take_changed(&mut self.store, head, |_| Some((LIVE.0.to_vec(), LIVE.1.to_vec())));
        if !changed.is_empty() {
            self.rerender();
        }
    }

    fn tick(&mut self, now_ms: u64) {
        let r = self.conn.tick_at(now_ms);
        self.feed(r);
        let _ = self.store.ask_after_applying();
        self.store.sync();
        self.drain();
        self.live_pump();
    }

    /// A write, through the store. FORCED (`Expect::Any` at every key,
    /// sdk#235): a read of each key first would park the burst on fetches and
    /// the model is about what a READ shows, not about premises; a forced
    /// write told Lost falls, named, and the oracle drops it as it does any
    /// write no longer in flight. Its id, or `None` if the store refused it.
    fn write(&mut self, ops: &Ops) -> Option<u64> {
        let edits: Vec<(Vec<u8>, craftworks_sdk::Edit)> = ops
            .iter()
            .map(|(k, v)| (k.clone(), v.clone().map_or(craftworks_sdk::Edit::Delete, craftworks_sdk::Edit::Put)))
            .collect();
        let reads: Vec<(Vec<u8>, protocol::Expect)> = ops.iter().map(|(k, _)| (k.clone(), protocol::Expect::Any)).collect();
        self.store.apply_commit(&reads, &edits).ok()?;
        self.store.last_write_id()
    }

    fn pending(&self, id: u64) -> bool {
        self.store.is_pending_write(id)
    }
}

// ------------------------------------------------------------------ the run

#[derive(Debug)]
struct Finding {
    class: &'static str,
    detail: String,
}

/// One seeded run. Returns what the model found.
fn run(seed: u64, mutant: Option<Mutant>, steps: usize, fast: usize) -> Vec<Finding> {
    let mut rng = Rng(seed);
    let node = PageNode::new();
    let mut x = node.connect();
    x.client(&Request::Identity);
    let mut y = Walks::open(&node, mutant);
    let mut found = Vec::new();
    let mut now = 1_000u64;
    let mut x_write = 0u64;
    // y's own writes, in the order made: what the oracle lays over the tree.
    let mut mine: Vec<(u64, Ops)> = Vec::new();
    for step in 0..steps + fast {
        now += 250;
        let fast_phase = step >= steps;
        let roll = rng.below(10);
        // x WRITES: the head moves. On every step of the fast phase.
        if fast_phase || roll <= 2 {
            x_write += 1;
            let o = ops(&mut rng, step, "x");
            let o = if fast_phase { widen(o) } else { o };
            let ops = o.into_iter().map(|(k, v)| v.map_or(protocol::Op::Delete(k.clone()), |v| protocol::Op::Put(k, v))).collect();
            x.client(&Request::forced_write(x_write, ops));
            x.tick_at(now);
            if fast_phase {
                y.hint(now);
            }
        }
        // In the fast phase y bursts too, every fifth step, on a tree of many
        // blocks it has only partly read: its first write may be PARKED.
        if fast_phase && step % 5 == 0 {
            burst(&mut y, &mut rng, step, now, seed, &mut mine, &mut found);
        }
        let read_now = fast_phase || roll >= 7;
        if !fast_phase {
            match roll {
                // y WRITES, in a burst: two or three writes at once, so the
                // later ones meet y's own commit in flight (`Busy`) and are
                // held by y's outbox, in no root yet.
                //
                // The node's answers are HELD for the burst, as a real node's
                // arrive after a round trip: the first write's commit stays in
                // flight and the rest meet it. Each key y wrote is read back
                // AT ONCE — read-your-writes while a commit is in flight — and
                // then the node answers.
                3 => burst(&mut y, &mut rng, step, now, seed, &mut mine, &mut found),
                4..=5 => y.hint(now),
                6 => y.tick(now),
                _ => {}
            }
        }
        if !read_now {
            continue;
        }
        let (lo, hi) = range(&mut rng);
        let before = y.head();
        let flying_before: Vec<u64> = mine.iter().map(|(id, _)| *id).filter(|id| y.pending(*id)).collect();
        // In the fast phase the head moves again WHILE a hop waits.
        let mut meanwhile = |y: &mut Walks| {
            if fast_phase {
                x_write += 1;
                let o = widen(ops(&mut rng, step, "x"));
                let ops = o.into_iter().map(|(k, v)| v.map_or(protocol::Op::Delete(k.clone()), |v| protocol::Op::Put(k, v))).collect();
                x.client(&Request::forced_write(x_write, ops));
                x.tick_at(now);
                // Adopted WITHOUT the LIVE re-run: that runs after the drain,
                // and here it would fetch the new head's blocks for the read
                // this hop is waiting on, hiding a chase.
                y.conn.with_server(|s| s.head_hint());
                let r = y.conn.tick_at(now);
                y.feed(r);
            }
        };
        let got = y.read(&lo, &hi, &mut meanwhile);
        let after = y.head();
        let flying_after: Vec<u64> = mine.iter().map(|(id, _)| *id).filter(|id| y.pending(*id)).collect();
        match got {
            Err(f) => found.push(Finding { class: "READ DID NOT ANSWER", detail: format!("seed {seed} step {step}: [{}, {}) — {f:?}", show(&lo), show(&hi)) }),
            Ok(rows) => {
                // The oracle: the node's tree at a head y stood on, with y's
                // own writes still in flight laid over it in the order made.
                // A head at seq 0 is the EMPTY tree. Any other root the node
                // cannot read is the harness's fault, named.
                let want = |(seq, root): (u64, [u8; 32]), flying: &[u64]| -> Option<Rows> {
                    let mut tree = if seq == 0 { BTreeMap::new() } else { node.tree(&root)? };
                    for (id, o) in &mine {
                        if flying.contains(id) {
                            for (k, v) in o {
                                match v {
                                    Some(v) => tree.insert(k.clone(), v.clone()),
                                    None => tree.remove(k),
                                };
                            }
                        }
                    }
                    Some(tree.range(lo.clone()..hi.clone()).map(|(k, v)| (k.clone(), v.clone())).collect())
                };
                let cands = [(before, &flying_before), (after, &flying_before), (before, &flying_after), (after, &flying_after)];
                if cands.iter().all(|(h, f)| want(*h, f).is_none()) {
                    found.push(Finding { class: "HARNESS: THE ORACLE CANNOT READ Y'S HEAD", detail: format!("seed {seed} step {step}: seq {}", before.0) });
                    continue;
                }
                if !cands.iter().any(|(h, f)| want(*h, f).as_ref() == Some(&rows)) {
                    let w = want(before, &flying_before).map(|v| v.len());
                    found.push(Finding {
                        class: "READ IS NOT THE TREE AT ITS HEAD",
                        detail: format!("seed {seed} step {step}: [{}, {}) got {} rows, the oracle has {w:?}", show(&lo), show(&hi), rows.len()),
                    });
                }
            }
        }
    }
    // AT REST: x stops; y's writes settle, y adopts until its head is the
    // node's, then reads all.
    for _ in 0..120 {
        now += 1_000;
        x.tick_at(now);
        y.hint(now);
        let settled = mine.iter().all(|(id, _)| !y.pending(*id));
        if settled && Some(y.head().1) == node.head().map(|h| h.1) {
            break;
        }
    }
    if mine.iter().any(|(id, _)| y.pending(*id)) {
        found.push(Finding { class: "Y'S WRITES NEVER SETTLED", detail: format!("seed {seed}") });
    }
    if Some(y.head().1) != node.head().map(|h| h.1) {
        found.push(Finding { class: "Y NEVER ADOPTED THE FINAL HEAD", detail: format!("seed {seed}") });
    } else {
        match y.read(b"k/", b"l/", &mut |_| {}) {
            Err(f) => found.push(Finding { class: "READ DID NOT ANSWER", detail: format!("seed {seed} at rest: {f:?}") }),
            Ok(rows) => {
                let tree: Rows = node.tree(&y.head().1).map(|t| t.into_iter().collect()).unwrap_or_default();
                if rows != tree {
                    found.push(Finding { class: "AT REST, Y'S READ IS NOT THE FINAL TREE", detail: format!("seed {seed}: {} rows vs {}", rows.len(), tree.len()) });
                }
            }
        }
    }
    // Every LIVE binding shows the oracle at the final head (READ-STATE §
    // The model test, at rest), and none of its re-reads failed.
    y.live_pump();
    let want: Rows = node
        .tree(&y.head().1)
        .map(|t| t.range(LIVE.0.to_vec()..LIVE.1.to_vec()).map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    if y.rendered.as_ref() != Some(&want) {
        found.push(Finding { class: "A LIVE BINDING DOES NOT SHOW THE FINAL TREE", detail: format!("seed {seed}: shows {:?} rows, the tree has {}", y.rendered.as_ref().map(|r| r.len()), want.len()) });
    }
    // Quiet at rest: with the head not moving, LIVE names nothing — a
    // binding told of a change it was already told of re-runs for ever.
    let head = y.store.head();
    let again = y.live.take_changed(&mut y.store, head, |_| Some((LIVE.0.to_vec(), LIVE.1.to_vec())));
    if !again.is_empty() {
        found.push(Finding { class: "LIVE NAMES A CHANGE IT ALREADY NAMED", detail: format!("seed {seed}: {again:?} with the head unmoved") });
    }
    if !y.live_failed.is_empty() {
        found.push(Finding { class: "A LIVE RE-READ DID NOT ANSWER", detail: format!("seed {seed}: {:?}", y.live_failed[0]) });
    }
    // Every ticket ENDED (the architect's rule: none left pending).
    y.tick(now + 1);
    let open = y.store.open_tickets();
    if open != 0 {
        found.push(Finding { class: "A TICKET NEVER ENDED", detail: format!("seed {seed}: {open} open at rest") });
    }
    // NO `Busy` ON THE PAGE PATH (R-b; COMMIT-LIFE K1): a write is queued by
    // the engine and a bound refuses it by name, never `Busy`.
    // Counted where `Busy` is MADE (page::Server), not where the harness sees
    // replies: the store consumes its own write verdicts inside `sync`, so a
    // count in `feed` saw almost none (measured: 0 on R-a, where bursts DO
    // meet Busy — a blind check).
    let busy = y.conn.with_server(|s| s.busy_told());
    if busy > 0 {
        found.push(Finding { class: "A WRITE WAS TOLD BUSY ON THE PAGE PATH", detail: format!("seed {seed}: {busy} Busy verdict(s)") });
    }
    // THE QUEUE DRAINS (R-b): at rest nothing of y's is left in the engine's
    // queue -- every write ended, Published or in a named fate.
    // K9 and footnote 4, counted where they happen: a rebuilt commit whose
    // root differed from its warm root, a re-derivation on own publish, an
    // impossible stage move -- each must be 0.
    let (differs, rederived, impossible) = y.conn.with_server(|s| s.page.queue_counts());
    if differs + rederived + impossible > 0 {
        found.push(Finding { class: "THE QUEUE'S OWN COUNTS ARE NOT ZERO", detail: format!("seed {seed}: K9 differs {differs}, own-publish re-derived {rederived}, impossible {impossible}") });
    }
    let (queued, _) = y.store.queue_load();
    if queued != 0 {
        found.push(Finding { class: "THE QUEUE DID NOT DRAIN AT REST", detail: format!("seed {seed}: {queued} write(s) still queued") });
    }
    found
}

/// y WRITES, in a burst: two or three writes at once, so the later ones meet
/// y's own commit in flight (`Busy`) and are held by y's outbox, in no root
/// yet — and, on a tree y has not read all of, the FIRST is parked by the
/// engine while it fetches the blocks it lands on, sent and unanswered.
///
/// The node's answers are HELD for the burst, as a real node's arrive after a
/// round trip. Each key y wrote is read back AT ONCE — read-your-writes while
/// a commit is in flight or a write is parked — and then the node answers.
fn burst(y: &mut Walks, rng: &mut Rng, step: usize, now: u64, seed: u64, mine: &mut Vec<(u64, Ops)>, found: &mut Vec<Finding>) {
    y.conn.hold_answers();
    let mut last: BTreeMap<Vec<u8>, Option<Vec<u8>>> = BTreeMap::new();
    for _ in 0..(2 + rng.below(2)) {
        let o = ops(rng, step, "y");
        if let Some(id) = y.write(&o) {
            last.extend(o.iter().cloned());
            mine.push((id, o));
        }
    }
    for (k, v) in &last {
        // A key whose blocks this tab does not hold cannot be read while the
        // node is silent; it is read below.
        if let Ok(got) = y.store.get(k) {
            if &got != v {
                found.push(Finding {
                    class: "A WRITE IS NOT READ BACK WHILE ITS COMMIT IS IN FLIGHT",
                    detail: format!("seed {seed} step {step}: {} reads {:?}, y wrote {:?}", show(k), got.as_deref().map(show), v.as_deref().map(show)),
                });
            }
        }
        y.store.take_ticket();
    }
    y.release_all();
    y.tick(now);
}

fn show(k: &[u8]) -> String {
    String::from_utf8_lossy(k).into_owned()
}

type Classes = BTreeMap<&'static str, (usize, String)>;

/// Every seed's findings, by class: the seeds run on every core (`testkit::model`), folded in seed order.
fn sweep(seeds: std::ops::Range<u64>, mutant: Option<Mutant>, steps: usize, fast: usize) -> Classes {
    fold(testkit::model::run(seeds, |seed| run(seed, mutant, steps, fast)))
}

fn fold(runs: Vec<(u64, Vec<Finding>)>) -> Classes {
    let mut by: Classes = BTreeMap::new();
    for (_, found) in runs {
        let mut seen = std::collections::BTreeSet::new();
        for f in found {
            if seen.insert(f.class) {
                let e = by.entry(f.class).or_insert((0, f.detail.clone()));
                e.0 += 1;
            }
        }
    }
    by
}

fn print(found: &Classes, n: u64) {
    for (class, (runs, first)) in found {
        println!("  {class}: {runs} of {n} seeds — first: {first}");
    }
}

/// THE WIDE SWEEP (ignored; its counts go in the PR): `RSM_SEEDS` seeds
/// (default 2,000), each twice as long as the gate's, a quarter of it fast.
#[test]
#[ignore = "wide sweep: RSM_SEEDS=2000 cargo test --release -p testkit --test read_state_model wide -- --ignored --nocapture"]
fn wide_sweep() {
    let n: u64 = std::env::var("RSM_SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(2_000);
    let found = sweep(0..n, None, 240, 80);
    print(&found, n);
    println!("  {n} seeds × (240 steps + 80 fast): {} class(es) found", found.len());
    assert!(found.is_empty(), "{found:?}");
}

/// SLICE R'S READER IS GREEN on the model that was red on the row copy: the
/// same 40 seeds, and 160 more (the full count; `CRAFTWORKS_MODEL_SEEDS` takes a share).
#[test]
fn slice_r_reader_is_green() {
    let n = testkit::model::share(200);
    let found = sweep(0..n, None, 120, 40);
    print(&found, n);
    assert!(found.is_empty(), "the walking reader failed the model: {found:?}");
    println!("  {n} seeds × (120 steps + 40 fast): nothing found");
}

/// THE CONTROLS: each mutation re-plants a defect in the walking reader, and
/// the model must see every one — or it is blind, not the reader right.
#[test]
fn the_model_sees_each_planted_defect() {
    for m in [Mutant::HeadCopy, Mutant::NoResume, Mutant::NoWaitOnApplying, Mutant::DeafLive] {
        // A share of the 40 first, then on until the defect is seen, never past the 40 it always ran.
        let runs = testkit::model::until(0, testkit::model::share(40), 40, |seed| run(seed, Some(m), 120, 40), |all| all.iter().any(|(_, f)| !f.is_empty()));
        let n = runs.len() as u64;
        let found = fold(runs);
        println!("{m:?}:");
        print(&found, n);
        assert!(testkit::model::alone().is_some() || !found.is_empty(), "the model did not see the planted {m:?}: it is blind to it");
    }
}
