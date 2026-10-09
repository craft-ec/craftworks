//! `Db::count` from the tree's own aggregate (craftworks-sdk#123).
//!
//! It enumerated every key and value in the domain to return a number every
//! node already carries (`freenet_prolly::aggregate`, merged and never
//! called). The falsifier: wiring it to the aggregate must change NO count —
//! `count == scan().len()` with zero differing cases — and the cost bound must
//! FAIL against the enumerating implementation, or it is a bound nothing can
//! blow.

use craftworks_sdk::store::{Edit, Reads, Store as _};
use craftworks_sdk::*;
use testkit::MemStore;
use serde_json::{json, Map, Value};

struct FakeEnv {
    now: u64,
    n: u32,
}
impl Env for FakeEnv {
    fn now_ms(&mut self) -> u64 {
        self.now += 1;
        self.now
    }
    fn rand32(&mut self) -> u32 {
        self.n = self.n.wrapping_mul(1664525).wrapping_add(1013904223);
        self.n
    }
}

fn fields(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => unreachable!(),
    }
}

fn schema() -> Schema {
    serde_json::from_value(json!({ "type": "Row", "fields": [{ "name": "n", "kind": "int" }] })).unwrap()
}

/// Domains of several sizes, an empty one, one emptied by deletes, and two
/// whose names are prefixes of each other — over any store.
fn fill<S: Store + Reads>(d: &mut Db<S, FakeEnv>) -> Vec<&'static str> {
    let sizes = [("empty", 0usize), ("one", 1), ("some", 17), ("task", 300), ("tasks", 1200)];
    for (name, n) in sizes {
        d.define(name, &schema()).unwrap();
        for i in 0..n {
            d.put(name, &fields(json!({ "n": i as i64 }))).unwrap();
        }
    }
    d.define("emptied", &schema()).unwrap();
    let ids: Vec<_> = (0..40).map(|i| d.put("emptied", &fields(json!({ "n": i }))).unwrap().id).collect();
    for id in ids {
        d.delete("emptied", id::loc_from_hex(&id).unwrap()).unwrap();
    }
    // Half of `some` deleted too: a domain that is neither full nor empty.
    let rows = d.scan("some", Scan::default()).unwrap();
    for r in rows.iter().step_by(2) {
        d.delete("some", id::loc_from_hex(&r.id).unwrap()).unwrap();
    }
    vec!["empty", "one", "some", "task", "tasks", "emptied"]
}

/// **THE FALSIFIER, over the TREE: the aggregate's count equals the scan's,
/// zero differing cases.**
#[test]
fn a_count_from_the_aggregate_equals_the_scan_for_every_domain() {
    let mut d = Db::new(TreeStore::new(), FakeEnv { now: 1_750_000_000_000, n: 7 }, [1, 2, 3, 4]);
    let mut differing = Vec::new();
    for name in fill(&mut d) {
        let counted = d.count(name).unwrap();
        let scanned = d.scan(name, Scan::default()).unwrap().len();
        if counted != scanned {
            differing.push((name, counted, scanned));
        }
    }
    assert!(differing.is_empty(), "count and scan disagree: {differing:?}");
    assert_eq!(d.count("emptied").unwrap(), 0, "a domain emptied by deletes counts no tombstone");
    assert_eq!(d.count("some").unwrap(), 8);
    assert_eq!(d.count("task").unwrap(), 300, "`tasks` does not bleed into `task`");
}

/// The same over the in-memory map, which keeps the enumerating default.
#[test]
fn a_store_with_no_tree_keeps_the_enumerating_count_and_agrees() {
    let mut d = Db::new(MemStore::default(), FakeEnv { now: 1_750_000_000_000, n: 7 }, [1, 2, 3, 4]);
    for name in fill(&mut d) {
        assert_eq!(d.count(name).unwrap(), d.scan(name, Scan::default()).unwrap().len(), "{name}");
    }
}

/// A tree of `n` entries under one domain's range, written in one batch.
fn tree(n: u32) -> (TreeStore, Vec<u8>, Vec<u8>) {
    let mut s = TreeStore::new();
    let (lo, hi) = Db::<TreeStore, SystemEnv>::domain_range("big");
    let edits: Vec<_> = (0..n)
        .map(|i| {
            let mut k = lo.clone();
            k.extend_from_slice(&[0u8; 12]);
            k.extend_from_slice(&i.to_be_bytes());
            (k, Edit::Put(vec![1u8; 40]))
        })
        .collect();
    s.apply_batch(&edits).expect("the store took the write");
    // Neighbours on both sides, so the range's edges fall inside nodes.
    s.apply_batch(&[(b"\x01bi\x00zz".to_vec(), Edit::Put(b"x".to_vec())), (b"\x01bigger\x00a".to_vec(), Edit::Put(b"x".to_vec()))]).expect("the store took the write");
    (s, lo, hi)
}

/// **The cost: O(height) node reads for a count, not one per entry — and the
/// same bound FAILS against enumerating.**
#[test]
fn a_count_over_twenty_thousand_entries_reads_o_of_height_nodes() {
    let (mut s, lo, hi) = tree(20_000);
    let height = s.stats().height as u64;
    let bound = 2 * height - 1;

    let _ = s.take_distinct_block_reads();
    let before = s.block_reads();
    let counted = s.count(&lo, &hi).unwrap();
    let count_calls = s.block_reads() - before;
    let count_reads = s.take_distinct_block_reads() as u64;

    let before = s.block_reads();
    let enumerated = s.scan(&lo, &hi, false, usize::MAX).unwrap().len() as u64;
    let scan_calls = s.block_reads() - before;
    let scan_reads = s.take_distinct_block_reads() as u64;

    println!("  20,000 entries, height {height}: count {counted} — {count_reads} distinct blocks ({count_calls} reads), bound {bound}; enumerating {enumerated} — {scan_reads} distinct ({scan_calls} reads)");
    assert_eq!(counted, 20_000);
    assert_eq!(counted, enumerated);
    assert!(count_reads <= bound, "a count read {count_reads} nodes, over 2·height−1 = {bound}");
    // THE CONTROL: the same bound, against the enumerating implementation
    // `Db::count` had, must FAIL — or it is a bound nothing can blow.
    assert!(scan_reads > bound, "enumerating read only {scan_reads} nodes: the bound above proves nothing");
}

/// An empty range, and a range that holds nothing, count 0 without being
/// mistaken for "could not look".
#[test]
fn an_empty_range_counts_zero() {
    let (mut s, _, _) = tree(500);
    let (lo, hi) = Db::<TreeStore, SystemEnv>::domain_range("nothing-here");
    assert_eq!(s.count(&lo, &hi).unwrap(), 0);
    assert_eq!(s.count(b"b", b"a").unwrap(), 0, "an inverted range");
}

/// **Through the entry an app uses: `Db::count` is the aggregate's cost**, not
/// the enumeration it used to be. (The test above calls the store; this one
/// is what proves `Db::count` still ASKS for it.)
#[test]
fn db_count_is_o_of_height_too() {
    let (s, _, _) = tree(20_000);
    let bound = 2 * s.stats().height - 1;
    let mut d = Db::new(s, SystemEnv, [9, 9, 9, 9]);
    let _ = d.store().take_distinct_block_reads();
    assert_eq!(d.count("big").unwrap(), 20_000);
    let reads = d.store().take_distinct_block_reads();
    println!("  Db::count over 20,000: {reads} distinct blocks (bound {bound})");
    assert!(reads <= bound, "Db::count read {reads} distinct blocks: it is enumerating again");
}

/// Ranges whose `lo` and `hi` land EXACTLY on stored keys — where an
/// off-by-one in the descent's inside/edge classification would live — count
/// what a scan returns. Every 37th key as `lo`, every 53rd as `hi`.
#[test]
fn counts_with_bounds_on_stored_keys_equal_the_scan() {
    let (mut s, lo, hi) = tree(4_000);
    let keys: Vec<Vec<u8>> = s.scan(&lo, &hi, false, usize::MAX).unwrap().into_iter().map(|(k, _)| k).collect();
    let mut cases = 0;
    let mut differing = Vec::new();
    for a in keys.iter().step_by(37) {
        for b in keys.iter().step_by(53) {
            let counted = s.count(a, b).unwrap();
            let scanned = s.scan(a, b, false, usize::MAX).unwrap().len() as u64;
            cases += 1;
            if counted != scanned {
                differing.push((counted, scanned));
            }
        }
    }
    assert!(cases > 5_000, "only {cases} cases: the sweep is too thin to find a boundary slip");
    assert!(differing.is_empty(), "{} of {cases} bounded counts differ from the scan: {:?}", differing.len(), &differing[..differing.len().min(5)]);
}
