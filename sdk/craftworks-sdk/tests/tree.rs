//! The tree behind `Store`. What matters here is not that reads and writes work
//! — the collection suite covers that on every store — but the thing only this
//! store CLAIMS: a root hash that stands for the contents.
//!
//! `cargo test --test tree -- --nocapture` prints the measurements.

use craftworks_sdk::id::from_hex;
use craftworks_sdk::store::{sorted_edits, Edit};
use craftworks_sdk::tree_store::Options;
use craftworks_sdk::*;
use testkit::MemStore;
use freenet_prolly::build::TreeBuilder;
use freenet_prolly::node::MAX_INLINE;
use std::collections::BTreeMap;

type Map = BTreeMap<Vec<u8>, Vec<u8>>;

fn rng(seed: u64) -> impl FnMut() -> u64 {
    let mut s = seed | 1;
    move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    }
}

/// The oracle: build the same contents from scratch. A prolly tree's shape is a
/// pure function of its contents, so this is what the root MUST be however the
/// store got there.
///
/// It pushes RAW BYTES, so the inline-or-reference choice is made by the
/// library's own rule — the same call the store makes. An oracle that restated
/// the rule could drift from it, and then this whole file would be comparing
/// two implementations of one mistake.
fn built_from_scratch(m: &Map) -> Cid {
    let mut t = TreeBuilder::new(|_, _: &[u8]| {});
    for (k, v) in m {
        t.push_bytes(k, v).unwrap();
    }
    t.finish().unwrap()
}

fn key(i: u64) -> Vec<u8> {
    format!("k/{i:08}").into_bytes()
}

/// Values across the inline boundary, so both kinds occur.
fn value(r: &mut impl FnMut() -> u64) -> Vec<u8> {
    let len = match r() % 4 {
        0 => 0,
        1 => (r() % 200) as usize,
        2 => MAX_INLINE + (r() % 8) as usize, // just over: a referenced value
        _ => (r() % 5000) as usize,
    };
    vec![(r() % 251) as u8; len]
}

/// Random op sequences against the reference, checking everything after each op.
fn differential(seed: u64, ops: usize, store: &mut (impl Store + Reads)) -> Map {
    let mut r = rng(seed);
    let mut want = Map::new();
    let mut mem = MemStore::default();
    for n in 0..ops {
        let i = r() % 60;
        match r() % 10 {
            0..=5 => {
                let (k, v) = (key(i), value(&mut r));
                store.put(&k, &v).expect("the store took the write");
                mem.put(&k, &v).expect("the store took the write");
                want.insert(k, v);
            }
            6..=7 => {
                let k = key(i);
                let a = store.delete(&k);
                let b = mem.delete(&k);
                assert_eq!(a, b, "op {n}: delete disagreed about whether it existed");
                want.remove(&k);
            }
            _ => {
                let (lo, hi) = (key(r() % 30), key(30 + r() % 30));
                let limit = 1 + (r() % 20) as usize;
                for reverse in [false, true] {
                    assert_eq!(
                        Reads::scan(store, &lo, &hi, reverse, limit),
                        Reads::scan(&mut mem, &lo, &hi, reverse, limit),
                        "op {n}: scan disagreed"
                    );
                }
            }
        }
        // Every key, every op — reads must agree the whole way, not at the end.
        for j in 0..60 {
            assert_eq!(
                Reads::get(store, &key(j)),
                Reads::get(&mut mem, &key(j)),
                "op {n}: get disagreed"
            );
        }
    }
    want
}

#[test]
fn a_tree_store_answers_exactly_like_the_reference() {
    for seed in [1u64, 2, 3] {
        let mut t = TreeStore::new();
        let want = differential(seed, 200, &mut t);
        assert_eq!(
            t.root(),
            built_from_scratch(&want),
            "seed {seed}: the root must be the one a rebuild gives"
        );
    }
}

/// `root()` is a claim about CONTENTS. Two stores that were written in
/// different orders, with different intermediate states, must agree — that is
/// the whole point of a history-independent tree, and it is the property the
/// builder's tree panel and every future sync will rest on.
#[test]
fn the_root_depends_on_the_contents_and_not_on_the_order() {
    let mut r = rng(99);
    let target: Map = (0..400u64).map(|i| (key(i), value(&mut r))).collect();

    let mut roots = Vec::new();
    for seed in [7u64, 8, 9] {
        let mut r = rng(seed);
        let mut order: Vec<&Vec<u8>> = target.keys().collect();
        for i in (1..order.len()).rev() {
            order.swap(i, (r() % (i as u64 + 1)) as usize);
        }
        let mut t = TreeStore::new();
        // With detours: junk written and removed again, values written wrong
        // and corrected. None of it may show in the root.
        for (n, k) in order.iter().enumerate() {
            if n.is_multiple_of(5) {
                t.put(k, b"wrong").expect("the store took the write");
            }
            t.put(k, &target[*k]).expect("the store took the write");
            if n.is_multiple_of(7) {
                let mut junk = (*k).clone();
                junk.push(b'~');
                t.put(&junk, b"junk").expect("the store took the write");
                t.delete(&junk).expect("the store took the write");
            }
        }
        roots.push(t.root());
    }
    assert_eq!(roots[0], built_from_scratch(&target));
    assert!(
        roots.iter().all(|r| *r == roots[0]),
        "three write orders, three different roots"
    );
}

/// A batch must be exactly the same fact as the writes it contains — in the
/// contents AND in the root. If it were not, a record and its index entries
/// would land as a state no reader should ever see.
#[test]
fn a_batch_equals_the_same_edits_one_at_a_time() {
    let mut r = rng(21);
    for round in 0..20 {
        let n = 1 + (r() % 12) as usize;
        let edits: Vec<(Vec<u8>, Edit)> = (0..n)
            .map(|_| {
                let k = key(r() % 40);
                if r().is_multiple_of(5) {
                    (k, Edit::Delete)
                } else {
                    (k, Edit::Put(value(&mut r)))
                }
            })
            .collect();

        // The same starting point for both.
        let seed_store = |s: &mut TreeStore| {
            for i in 0..40u64 {
                s.put(&key(i), format!("start-{i}").as_bytes()).expect("the store took the write");
            }
        };
        let (mut batched, mut singly) = (TreeStore::new(), TreeStore::new());
        seed_store(&mut batched);
        seed_store(&mut singly);
        assert_eq!(batched.root(), singly.root(), "same start");

        batched.apply_batch(&edits).expect("the store took the write");
        // Singly, in the SDK's own order — which is what the batch resolves to.
        for (k, e) in sorted_edits(edits.clone()) {
            match e {
                Edit::Put(v) => singly.put(&k, &v).expect("a tree refuses nothing"),
                Edit::Delete => singly.delete(&k).expect("a tree refuses nothing"),
            }
        }
        assert_eq!(
            batched.root(),
            singly.root(),
            "round {round}: {n} edits as a batch gave a different root"
        );
        for i in 0..40u64 {
            assert_eq!(
                Reads::get(&mut batched, &key(i)),
                Reads::get(&mut singly, &key(i)),
                "round {round}"
            );
        }
    }
}

/// Last-write-wins is the SDK's rule and it is applied before the tree sees
/// anything, so two writes to one key in a batch are one edit.
#[test]
fn the_sdk_sorts_and_dedupes_before_the_tree_sees_a_batch() {
    let edits = vec![
        (key(5), Edit::Put(b"first".to_vec())),
        (key(1), Edit::Put(b"early".to_vec())),
        (key(5), Edit::Put(b"second".to_vec())),
        (key(3), Edit::Put(b"mid".to_vec())),
        (key(5), Edit::Delete),
    ];
    let done = sorted_edits(edits.clone());
    assert_eq!(
        done.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>(),
        vec![key(1), key(3), key(5)],
        "sorted, one entry per key"
    );
    assert_eq!(done[2].1, Edit::Delete, "the last write to a key wins");

    let mut t = TreeStore::new();
    t.apply_batch(&edits).expect("the store took the write");
    assert_eq!(
        Reads::get(&mut t, &key(1)).unwrap().as_deref(),
        Some(&b"early"[..])
    );
    assert_eq!(Reads::get(&mut t, &key(5)), Ok(None), "the delete was last");
    // And the same batch offered to the reference store agrees.
    let mut m = MemStore::default();
    m.apply_batch(&edits).expect("the store took the write");
    for i in [1u64, 3, 5] {
        assert_eq!(
            Reads::get(&mut t, &key(i)),
            Reads::get(&mut m, &key(i)),
            "key {i}"
        );
    }
}

/// A batch the tree refuses leaves the store exactly as it was — `apply` emits
/// nothing and changes nothing, so there is no half-written state to clean up.
///
/// It is loud rather than silent: `Store` has no error channel, and a write that
/// quietly did nothing would be worse than a stopped test. See the PR for the
/// API note.
#[test]
fn a_refused_batch_changes_nothing() {
    use freenet_prolly::node::MAX_KEY;
    let mut t = TreeStore::new();
    for i in 0..30u64 {
        t.put(&key(i), b"v").expect("the store took the write");
    }
    let (before_root, before_stats) = (t.root(), t.stats());

    let too_long = vec![b'k'; MAX_KEY + 1];
    let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        t.apply_batch(&[
            (key(1), Edit::Put(b"ok".to_vec())),
            (too_long, Edit::Put(b"no".to_vec())),
        ]).expect("the store took the write");
    }));
    assert!(refused.is_err(), "an over-long key must not pass silently");
    assert_eq!(t.root(), before_root, "the root moved on a refused batch");
    assert_eq!(t.stats(), before_stats, "blocks changed on a refused batch");
    assert_eq!(
        Reads::get(&mut t, &key(1)).unwrap().as_deref(),
        Some(&b"v"[..]),
        "and no edit landed"
    );
}

/// Negative control: a store that forgets the blocks of values too large to
/// inline must FAIL. Without this, nothing proves the referenced-value path is
/// exercised at all — every assertion above would pass on a suite of short
/// values.
#[test]
fn a_store_that_drops_value_blocks_fails() {
    let mut lossy = TreeStore::with_options(Options { keep_value_blocks: false });
    let broke = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        differential(1, 200, &mut lossy);
    }));
    assert!(
        broke.is_err(),
        "dropping value blocks went unnoticed, so referenced values are not covered"
    );

    // And the control is narrow: with the blocks kept, the same run passes.
    let mut good = TreeStore::new();
    differential(1, 200, &mut good);

    // The values that make the difference really are in the data.
    let mut r = rng(1);
    let refs = (0..200)
        .filter(|_| value(&mut r).len() > MAX_INLINE)
        .count();
    assert!(refs > 10, "only {refs} referenced values in the run");
}

#[test]
fn stats_describe_the_tree() {
    let mut t = TreeStore::new();
    let empty = t.stats();
    assert_eq!(empty.height, 1, "the empty tree is one leaf");
    assert_eq!(empty.blocks, 1);

    let mut r = rng(5);
    for i in 0..5000u64 {
        t.put(&key(i), &vec![7u8; 100 + (r() % 100) as usize]).expect("the store took the write");
    }
    let s = t.stats();
    assert!(s.height >= 2, "5000 records need more than one leaf");
    assert!(s.blocks > s.height, "{s:?}");
    assert!(s.bytes > 5000 * 100, "{s:?}");
    // Nothing is removed: superseded nodes stay, so blocks only grow.
    let before = t.stats().blocks;
    t.put(&key(0), b"changed").expect("the store took the write");
    assert!(t.stats().blocks >= before, "history must not be dropped");
}

/// What the real tree costs in memory, against the reference store.
///
/// Record keys are rkeys, which are time-ordered, so every record write is an
/// APPEND. That is measured separately from writes scattered across the
/// keyspace: they are different costs, and appending is the one the SDK does.
///
/// **Ignored by default: run it with `--release`.**
///
///     cargo test --release --test tree -- --ignored --nocapture
///
/// Not for speed — for honesty. This prints RATES, and a rate measured in a
/// debug build is not a rate: the same body runs about 150x slower there, so
/// the numbers it would publish from `cargo test` are wrong by two orders of
/// magnitude while looking exactly as authoritative. It also cost the default
/// suite 143 of its 145 seconds, which is what made anyone look.
#[test]
#[ignore = "a measurement: run with --release, or the rates it prints are not rates"]
fn cost_against_the_reference_store() {
    const N: u64 = 10_000;
    let mut r = rng(4);
    let vals: Vec<Vec<u8>> = (0..N).map(|_| vec![(r() % 251) as u8; 220]).collect();

    fn time<F: FnOnce()>(f: F) -> std::time::Duration {
        let t = std::time::Instant::now();
        f();
        t.elapsed()
    }

    let mut tree = TreeStore::new();
    let tree_put = time(|| {
        for i in 0..N {
            tree.put(&key(i), &vals[i as usize]).expect("the store took the write");
        }
    });
    let mut mem = MemStore::default();
    let mem_put = time(|| {
        for i in 0..N {
            mem.put(&key(i), &vals[i as usize]).expect("the store took the write");
        }
    });

    let (lo, hi) = (key(0), key(N));
    let tree_scan = time(|| {
        assert_eq!(
            tree.scan(&lo, &hi, false, usize::MAX).unwrap().len(),
            N as usize
        );
    });
    let mem_scan = time(|| {
        assert_eq!(
            mem.scan(&lo, &hi, false, usize::MAX).unwrap().len(),
            N as usize
        );
    });

    // One batch of the same size, which is what a real write path does.
    let mut batched = TreeStore::new();
    let edits: Vec<(Vec<u8>, Edit)> = (0..N)
        .map(|i| (key(i), Edit::Put(vals[i as usize].clone())))
        .collect();
    let tree_batch = time(|| batched.apply_batch(&edits).expect("the store took the write"));
    assert_eq!(batched.root(), tree.root(), "a batch is the same contents");

    let rate = |d: std::time::Duration| N as f64 / d.as_secs_f64();
    println!("{N} records of ~220 B, in memory");
    println!(
        "  put, one at a time   tree {:>9.0} ops/s   mem {:>10.0} ops/s   ({:.0}x)",
        rate(tree_put),
        rate(mem_put),
        rate(mem_put) / rate(tree_put)
    );
    println!(
        "  put, one batch       tree {:>9.0} ops/s   ({:.0}x faster than one at a time)",
        rate(tree_batch),
        rate(tree_batch) / rate(tree_put)
    );
    println!(
        "  full scan            tree {:>9.0} ops/s   mem {:>10.0} ops/s   ({:.1}x)",
        rate(tree_scan),
        rate(mem_scan),
        rate(mem_scan) / rate(tree_scan)
    );
    let s = tree.stats();
    println!(
        "  tree: {} blocks, {} B, height {}",
        s.blocks, s.bytes, s.height
    );

    // Scattered writes, for contrast: the same count, keys all over the range.
    let mut r2 = rng(8);
    let order: Vec<u64> = {
        let mut v: Vec<u64> = (0..N).collect();
        for i in (1..v.len()).rev() {
            v.swap(i, (r2() % (i as u64 + 1)) as usize);
        }
        v
    };
    let mut scattered = TreeStore::new();
    let scattered_put = time(|| {
        for &i in &order {
            scattered.put(&key(i), &vals[i as usize]).expect("the store took the write");
        }
    });
    assert_eq!(scattered.root(), tree.root(), "same contents, same root");
    println!(
        "  put, scattered       tree {:>9.0} ops/s   ({:.1}x the append rate)",
        rate(scattered_put),
        rate(scattered_put) / rate(tree_put)
    );
}

/// A record too large for the tree must come back as an ERROR the app can
/// handle, not as a stop. `Store` has no error channel and the wasm build is
/// `panic = abort`, so a limit breach that reached the store would take the
/// whole SDK down with it — `Db` screens first, which is what makes the
/// `unreachable!` in `TreeStore` an honest claim rather than a hope.
#[test]
fn an_oversize_record_is_refused_and_the_database_still_works() {
    use craftworks_sdk::id::SystemEnv;
    use freenet_prolly::node::MAX_VALUE;
    use serde_json::{json, Map, Value as J};

    let schema: craftworks_sdk::Schema = serde_json::from_value(json!({
        "type": "Note",
        "fields": [{"name": "body", "kind": "text", "required": true}]
    }))
    .unwrap();
    let obj = |v: J| -> Map<String, J> { v.as_object().unwrap().clone() };

    let mut d = Db::new(TreeStore::new(), SystemEnv, *b"dev1");
    d.define("notes", &schema).unwrap();
    let ok = d.put("notes", &obj(json!({"body": "small"}))).unwrap();
    let (root, stats) = (d.store().root(), d.store().stats());

    let huge = "x".repeat(MAX_VALUE + 1024);
    let err = d
        .put("notes", &obj(json!({ "body": huge })))
        .expect_err("a record over the limit must be refused, not stored");
    // The CODE is what an app branches on: no reload makes a record smaller,
    // so this must not look transient.
    assert_eq!(err.code(), "TOO_LARGE");
    assert!(!err.is_transient());
    let err = err.to_string();
    assert!(err.contains(&MAX_VALUE.to_string()), "{err}");
    assert!(
        err.contains("file") || err.contains("blob"),
        "the message must say what to do instead: {err}"
    );

    // Nothing moved.
    assert_eq!(d.store().root(), root, "a refused write moved the root");
    assert_eq!(
        d.store().stats(),
        stats,
        "a refused write changed the store"
    );

    // And the database still works afterwards.
    let after = d.put("notes", &obj(json!({"body": "still here"}))).unwrap();
    assert_ne!(d.store().root(), root);
    assert_eq!(d.count("notes").unwrap(), 2);
    assert!(d
        .get("notes", from_hex(&ok.id).unwrap())
        .unwrap()
        .is_some());
    assert!(d
        .get("notes", from_hex(&after.id).unwrap())
        .unwrap()
        .is_some());

    // The largest legal record IS stored, so the refusal is the size and not
    // the screen refusing everything large.
    let big = "y".repeat(MAX_VALUE - 512);
    d.put("notes", &obj(json!({ "body": big })))
        .expect("a record just under the limit must be stored");
    assert_eq!(d.count("notes").unwrap(), 3);
}
