//! THE WRITE PATH'S MODEL, over the REAL engine queue (R-b).
//!
//! Before R-b this model drove the client's outbox (`CachedStore`) against a
//! SCRIPTED node that answered `Busy`, dropped and misrouted verdicts, refused
//! past its queue and forgot its context; its pinned sweep table was what
//! sdk#265, #249 and #235 were proven against. The outbox and the delegate it
//! was a client of are gone: the engine owns the queue in the page, a write's
//! fate is PULLED, and nothing on the client times out, re-sends or holds a
//! write back. So the scripted node is replaced by the real one -- two TABS
//! OF ONE PERSON (two pages, one node, one signer, one Register) racing, whose
//! same-seq races are real foreign moves -- and the faults are the ones a
//! node still has: answers HELD and released one at a time (reordered across
//! the two tabs), answers LOST (the page's own deadlines re-ask), and the
//! clock JUMPING forward. The PROPERTIES are kept (the mapping table in the PR
//! says where each old assertion lives now):
//!
//! * NO WRITE IS LOST SILENTLY (W4, sdk#291): every write made ends exactly
//!   once -- refused at its door by name, Published, or in a NAMED fate the
//!   app was told. "Vanished, never told" is counted and must be 0.
//! * VALUE PARITY WITH THE TREE (W4/W6 as revised, the false rollback): at
//!   rest, each key holds the value of the LAST write made on it that ended
//!   Published -- so a write told it fell is not in the tree, and one told
//!   Published is (unless the same tab wrote the key again after it).
//! * ORDER (W2, go-back-N): per key, the tree never holds an older write's
//!   value over a later Published one of the same tab (the parity check,
//!   which knows make order).
//! * FORCED -> LOST NAMED (sdk#235): a forced write whose commit dies is told
//!   so, as forced; it never lands after being told.
//! * AT REST: nothing unsaved, the queue empty, no `Busy`, and no rebuilt
//!   commit that differed from its warm root (K9).
//!
//! Not here, and where they are: dependants re-run in app order is `Db`'s
//! (page/tests/server_differential.rs, the chain and budget tests); a write's
//! frame carrying exactly its ops is `tests/writes.rs`.
//!
//! THE LOOP RULE: every loop that waits advances the clock or is bounded, and
//! fails by name.

use craftworks_sdk::store::{Edit, Reads, Store};
use craftworks_sdk::PageStore;
use std::collections::{BTreeMap, BTreeSet};
use testkit::{PageConn, PageNode};

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

/// How a write of the model ended, as the tab was told.
#[derive(Debug, Clone, PartialEq)]
enum End {
    RefusedAtDoor(String),
    Published,
    Named(String),
}

struct Made {
    id: u64,
    ops: Vec<(Vec<u8>, Option<Vec<u8>>)>,
    forced: bool,
    end: Vec<End>,
}

struct Tab {
    store: PageStore<PageConn>,
    conn: PageConn,
    clock: testkit::Clock,
    prefix: &'static str,
    made: Vec<Made>,
}

#[derive(Debug)]
struct Finding {
    class: &'static str,
    detail: String,
}

const KEYS: u64 = 8;
const STEPS: usize = 150;

impl Tab {
    fn open(node: &PageNode, prefix: &'static str) -> Tab {
        let (mut store, conn, clock) = testkit::page_store(node);
        store.sync();
        Tab { store, conn, clock, prefix, made: Vec::new() }
    }

    fn key(&self, i: u64) -> Vec<u8> {
        format!("{}/{i}", self.prefix).into_bytes()
    }

    /// Everything the tab was told since the last call, onto its writes.
    fn collect(&mut self) {
        self.store.sync();
        let mut told: Vec<(u64, End)> = Vec::new();
        told.extend(self.store.writes.take_published().into_iter().map(|id| (id, End::Published)));
        told.extend(self.store.writes.take_ended().into_iter().map(|(id, e)| (id, End::Named(format!("{e:?}")))));
        told.extend(self.store.writes.take_conflicts().into_iter().map(|c| (c.write_id, End::Named("Conflict".into()))));
        told.extend(self.store.writes.take_unread().into_iter().flat_map(|u| u.write_ids.into_iter().map(|id| (id, End::Named("Unread".into())))));
        for (id, end) in told {
            match self.made.iter_mut().find(|m| m.id == id) {
                Some(m) => m.end.push(end),
                None => self.made.push(Made { id, ops: Vec::new(), forced: false, end: vec![End::Named(format!("STRANGER {end:?}"))] }),
            }
        }
    }

    /// A write of 1-2 keys: CHECKED (its reads the warm root's values, as a
    /// `Db` write makes them) or FORCED (`Expect::Any`, a store-level batch).
    fn write(&mut self, rng: &mut Rng, step: usize) {
        let n = 1 + rng.below(2);
        let mut ops: BTreeMap<Vec<u8>, Option<Vec<u8>>> = BTreeMap::new();
        for _ in 0..n {
            let k = self.key(rng.below(KEYS));
            let v = if rng.below(6) == 0 { None } else { Some(format!("{}{step}", self.prefix).into_bytes()) };
            ops.insert(k, v);
        }
        let mut forced = rng.below(3) == 0;
        let mut reads = Vec::new();
        if !forced {
            for k in ops.keys() {
                match self.store.get(k) {
                    Ok(Some(v)) => reads.push((k.clone(), protocol::Expect::Value(craftworks_sdk::read_token::read_token(&v)))),
                    Ok(None) => reads.push((k.clone(), protocol::Expect::Absent)),
                    // Not walkable now (its path cold, or a write of its own
                    // still applying): the store-level form, forced.
                    Err(_) => {
                        forced = true;
                        break;
                    }
                }
            }
            self.store.take_ticket();
            self.store.unpin();
        }
        if forced {
            reads = ops.keys().map(|k| (k.clone(), protocol::Expect::Any)).collect();
        }
        let edits: Vec<(Vec<u8>, Edit)> = ops.iter().map(|(k, v)| (k.clone(), v.clone().map_or(Edit::Delete, Edit::Put))).collect();
        let id = self.store.writes.next_write_id();
        let r = self.store.apply_commit(&reads, &edits);
        let end = match r {
            Ok(()) => Vec::new(),
            Err(why) => vec![End::RefusedAtDoor(format!("{why:?}"))],
        };
        self.made.push(Made { id, ops: ops.into_iter().collect(), forced, end });
    }

    fn tick(&mut self, ms: u64) {
        self.clock.advance(ms);
        let now = self.clock.now_ms();
        let _ = self.conn.tick_at(now);
        let _ = self.store.ask_after_applying();
        self.collect();
    }
}

/// What a run's writes came to: made, Published, ended named (by fate),
/// refused at the door, forced.
#[derive(Default, Debug)]
struct Counts {
    made: usize,
    published: usize,
    named: BTreeMap<String, usize>,
    door: usize,
    forced: usize,
}

fn run(seed: u64) -> (Vec<Finding>, Counts) {
    let mut rng = Rng(seed);
    let node = PageNode::new();
    let mut tabs = [Tab::open(&node, "a"), Tab::open(&node, "b")];
    for step in 0..STEPS {
        let t = rng.below(2) as usize;
        match rng.below(10) {
            0..=4 => tabs[t].write(&mut rng, step),
            5 => tabs[t].conn.hold_answers(),
            6 => tabs[t].conn.stop_holding(),
            7 => {
                if tabs[t].conn.held() > 0 {
                    let _ = tabs[t].conn.release_one();
                }
            }
            // An answer LOST (its request's deadline re-asks).
            8 => {
                if rng.below(3) == 0 {
                    tabs[t].conn.lose_one_held();
                }
            }
            // Time passes: a second, or (rarely) a jump of ten minutes.
            _ => {
                let ms = if rng.below(8) == 0 { 600_000 } else { 1_000 };
                tabs[t].tick(ms);
            }
        }
        tabs[t].collect();
    }
    // TO REST: faults stop, every held answer is delivered, time passes.
    for tab in tabs.iter_mut() {
        tab.conn.stop_holding();
    }
    for round in 0..400 {
        let mut busy = false;
        for tab in tabs.iter_mut() {
            while tab.conn.held() > 0 {
                let _ = tab.conn.release_one();
                busy = true;
            }
            tab.tick(1_000);
            busy |= tab.store.unsaved_writes() > 0 || tab.store.queue_load().0 > 0;
        }
        if !busy && round > 5 {
            break;
        }
    }
    let mut found = Vec::new();
    let mut counts = Counts::default();
    let tree = node.head().and_then(|(_, r)| node.tree(&r)).unwrap_or_default();
    for tab in tabs.iter_mut() {
        tab.collect();
        let p = tab.prefix;
        // Every write ended exactly once.
        for m in &tab.made {
            counts.made += 1;
            counts.forced += usize::from(m.forced);
            match m.end.first() {
                Some(End::Published) => counts.published += 1,
                Some(End::RefusedAtDoor(_)) => counts.door += 1,
                Some(End::Named(s)) => *counts.named.entry(s.clone()).or_default() += 1,
                None => {}
            }
            match m.end.len() {
                0 => found.push(Finding { class: "A WRITE VANISHED, NEVER TOLD", detail: format!("seed {seed} tab {p} write {} {:?}", m.id, m.ops) }),
                1 => {}
                _ => found.push(Finding { class: "A WRITE WAS TOLD TWICE", detail: format!("seed {seed} tab {p} write {}: {:?}", m.id, m.end) }),
            }
            if m.end.iter().any(|e| matches!(e, End::Named(s) if s.starts_with("STRANGER"))) {
                found.push(Finding { class: "A FATE FOR A WRITE NEVER MADE", detail: format!("seed {seed} tab {p}: {:?}", m.end) });
            }
            if m.end.iter().any(|e| matches!(e, End::Named(s) if s == "Lost")) && m.forced {
                found.push(Finding { class: "A FORCED WRITE FELL, NOT NAMED AS FORCED", detail: format!("seed {seed} tab {p} write {}", m.id) });
            }
        }
        // VALUE PARITY: each key holds its last Published write's value.
        for i in 0..KEYS {
            let k = tab.key(i);
            let want: Option<Vec<u8>> = tab
                .made
                .iter()
                .filter(|m| m.end == [End::Published])
                .filter_map(|m| m.ops.iter().find(|(kk, _)| *kk == k).map(|(_, v)| v.clone()))
                .next_back()
                .flatten();
            let got = tree.get(&k).cloned();
            if got != want {
                // Which write's value is it? The finding names the cause.
                let whose: Vec<String> = tab
                    .made
                    .iter()
                    .filter_map(|m| m.ops.iter().find(|(kk, _)| *kk == k).map(|(_, v)| format!("w{}={:?}{}:{:?}", m.id, v.as_deref().map(String::from_utf8_lossy), if m.forced { "(forced)" } else { "" }, m.end)))
                    .collect();
                found.push(Finding {
                    class: "THE TREE IS NOT WHAT THE TAB WAS TOLD",
                    detail: format!("seed {seed} key {}: tree {:?}, told {:?}; writes of it, in order: {whose:?}", String::from_utf8_lossy(&k), got.as_deref().map(String::from_utf8_lossy), want.as_deref().map(String::from_utf8_lossy)),
                });
            }
        }
        let unsaved = tab.store.unsaved_writes();
        if unsaved > 0 || tab.store.queue_load().0 > 0 {
            found.push(Finding { class: "NOT AT REST: WRITES STILL QUEUED", detail: format!("seed {seed} tab {p}: {unsaved} unsaved") });
        }
        let (differs, rederived, impossible) = tab.conn.with_server(|s| s.page.queue_counts());
        if differs + rederived + impossible > 0 {
            found.push(Finding { class: "THE QUEUE'S OWN COUNTS ARE NOT ZERO", detail: format!("seed {seed} tab {p}: K9 differs {differs}, own-publish re-derived {rederived}, impossible {impossible}") });
        }
        // NEVER APPLIED TWICE (W3; K9 §6: write_id -> seq is one-to-one). A
        // write reaches a head once: carried by one own commit that published
        // (`writes_published`), or witnessed landed unheard, or found already
        // in the tree (a no-op group, #164). Each such is told Published once;
        // a merge re-application publishes told to no one. A write carried by two commits counts twice here and breaks it
        // -- value parity cannot see a second apply of the same assignment.
        let told_published = tab.made.iter().filter(|m| m.end.first() == Some(&End::Published)).count() as u64;
        let (carried, witnessed, noop, merged) = tab.conn.with_server(|s| (s.page.commits_and_writes().1, s.page.landed_by_witness(), s.page.noop_published(), s.merge_published()));
        if carried + witnessed + noop != told_published + merged {
            found.push(Finding { class: "A WRITE REACHED A HEAD TWICE (write_id -> seq not one-to-one)", detail: format!("seed {seed} tab {p}: carried by own commits {carried} + witnessed {witnessed} + no-op {noop}, told Published {told_published} + merge re-applications {merged}") });
        }
        let busy = tab.conn.with_server(|s| s.busy_told());
        if busy > 0 {
            found.push(Finding { class: "A WRITE WAS TOLD BUSY", detail: format!("seed {seed} tab {p}: {busy}") });
        }
    }
    (found, counts)
}

/// **THE WRITE PATH'S PROPERTIES HOLD ON THE REAL QUEUE**: 200 seeds of two
/// tabs of one person racing under held, reordered and lost answers and
/// clock jumps -- nothing vanishes untold, nothing is told twice, the tree is
/// what each tab was told, and the queue comes to rest. COVERAGE is printed
/// and asserted beside it: the runs really made the situations the
/// properties are about, or a green sweep would be a sweep of nothing.
#[test]
fn the_write_path_properties_hold_on_the_real_queue() {
    let mut classes: BTreeMap<&'static str, (usize, String)> = BTreeMap::new();
    let mut total = Counts::default();
    // 200 seeds at the full count (`CRAFTWORKS_MODEL_SEEDS` takes a share, on every core: testkit::model); a share
    // runs on until a commit was forced and a write ended named, never past the 200.
    let runs = testkit::model::until(0, testkit::model::share(200), 200, run, |all| all.iter().any(|(_, (_, c))| c.forced > 0) && all.iter().any(|(_, (_, c))| !c.named.is_empty()));
    let n = runs.len();
    for (_, (found, c)) in runs {
        let mut seen: BTreeSet<&'static str> = BTreeSet::new();
        for f in found {
            if seen.insert(f.class) {
                classes.entry(f.class).or_insert((0, f.detail.clone())).0 += 1;
            }
        }
        total.made += c.made;
        total.published += c.published;
        total.door += c.door;
        total.forced += c.forced;
        for (k, v) in c.named {
            *total.named.entry(k).or_default() += v;
        }
    }
    println!("  {n} seeds: {} writes made, {} Published, ended named {:?}, {} refused at the door, {} forced", total.made, total.published, total.named, total.door, total.forced);
    for (class, (k, first)) in &classes {
        println!("  {class}: {k} of {n} seeds — first: {first}");
    }
    if testkit::model::alone().is_none() {
        // 5,000 writes over the full 200 seeds: 25 a seed.
        assert!(total.made > 25 * n && total.published > 0 && total.forced > 0, "the model made too little to check anything: {total:?}");
        assert!(!total.named.is_empty(), "no write ever ended named: the races never killed a commit, so 'nothing vanishes untold' was never tested");
    }
    assert!(classes.is_empty(), "the write path failed its model: {classes:?}");
}
