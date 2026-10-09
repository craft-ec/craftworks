//! What a parked read's re-descent actually COSTS (sdk#34).
//!
//! The issue proposes carrying a frontier so a parked read continues from the
//! arrived block instead of re-descending from the root. Before building that,
//! this measures the thing it would remove — with `expected()` (sdk#98) as the
//! denominator, which is the first use of the GAP on our own code.
//!
//! **What is modelled here, and what is not.** The mechanism in the issue is
//! that `read::get` has no entry but the root, so every re-entry walks from
//! the top. That is a property of the tree library, not of the engine, and it
//! is reproduced exactly: fetch what `Need` names, call `get` again, repeat.
//! The engine's own overheads are deliberately NOT in the numbers, because
//! adding them would only inflate a ratio whose cause is this loop.
//!
//! Two node behaviours, because they answer different questions:
//!
//! - **RETAINING** — the node keeps what it fetched, which is the normal case.
//! - **FORGETTING** — the node drops a block right after serving it, which is
//!   the `forget every 1` row in #32's test, and the case the issue says turns
//!   every read into `Unavailable`.

use craftworks_sdk::expected::{expected, Basis, Op};
use freenet_prolly::apply::{apply_into, Edit};
use freenet_prolly::build::init;
use freenet_prolly::read::get;
use freenet_prolly::store::{Blocks, MemBlocks, ReadError};
use freenet_prolly::Cid;
use std::cell::RefCell;
use std::collections::HashMap;

/// A store that counts every block it hands out, including repeats.
///
/// Repeats are the measurement: re-descending re-reads the same upper levels,
/// and a store that counted distinct blocks would report the waste as zero.
struct Counting {
    held: RefCell<HashMap<Cid, Vec<u8>>>,
    reads: RefCell<u64>,
    bytes: RefCell<u64>,
}

impl Counting {
    fn empty() -> Counting {
        Counting {
            held: RefCell::new(HashMap::new()),
            reads: RefCell::new(0),
            bytes: RefCell::new(0),
        }
    }
    fn put(&self, cid: Cid, bytes: Vec<u8>) {
        self.held.borrow_mut().insert(cid, bytes);
    }
    fn forget(&self, cid: &Cid) {
        self.held.borrow_mut().remove(cid);
    }
}

impl Blocks for Counting {
    fn get(&self, cid: &Cid) -> Option<&[u8]> {
        let held = self.held.borrow();
        let b = held.get(cid)?;
        *self.reads.borrow_mut() += 1;
        *self.bytes.borrow_mut() += b.len() as u64;
        // The borrow cannot escape the RefCell, so the bytes are leaked once
        // per block. This is a measurement harness; the alternative is
        // rewriting `Blocks` to return owned bytes, which would change the
        // thing being measured.
        let leaked: &'static [u8] = Box::leak(b.clone().into_boxed_slice());
        Some(leaked)
    }
}

/// How the node behaves toward a block it has just served.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Node {
    Retaining,
    Forgetting,
}

/// Run one cold read the way a parked read runs today: descend from the ROOT,
/// fetch whatever `Need` names, descend from the root again.
///
/// Returns (reads, bytes, entries, answered).
fn cold_read(source: &MemBlocks, root: &Cid, key: &[u8], node: Node) -> (u64, u64, u32, bool) {
    let store = Counting::empty();
    let mut entries = 0u32;
    let mut served: Vec<Cid> = Vec::new();

    loop {
        entries += 1;
        if entries > 64 {
            // Never settles. For a forgetting node that is not a harness
            // fault — it is the finding: the read cannot make progress,
            // because what it needs next is dropped before it is used.
            return (*store.reads.borrow(), *store.bytes.borrow(), entries, false);
        }

        match get(&store, root, key) {
            Ok(found) => {
                return (
                    *store.reads.borrow(),
                    *store.bytes.borrow(),
                    entries,
                    found.is_some(),
                )
            }
            Err(ReadError::Need(cids)) => {
                // A forgetting node retains only the block it most recently
                // served: what it handed over before that is already gone. The
                // eviction therefore happens HERE, when the next block is
                // served, not at the top of the call — because a delegate has
                // the arriving block's bytes in the call they arrive in, and a
                // model that took them away first would be deciding the answer
                // rather than measuring it.
                if node == Node::Forgetting {
                    for cid in served.drain(..) {
                        store.forget(&cid);
                    }
                }
                let mut progressed = false;
                for cid in cids {
                    if let Some(b) = source.get(&cid) {
                        store.put(cid, b.to_vec());
                        served.push(cid);
                        progressed = true;
                    }
                }
                if !progressed {
                    // Nothing could be fetched: this is the Unavailable case.
                    return (*store.reads.borrow(), *store.bytes.borrow(), entries, false);
                }
            }
            Err(e) => panic!("unexpected read error: {e:?}"),
        }
    }
}

/// The same read, CONTINUING from the deepest block it reached.
///
/// The whole change: on `Need`, the missing cid becomes the point the next
/// attempt starts from, instead of starting at the root again. `get` already
/// takes the cid to start at, so this needs no new entry point in the tree
/// library — it needs the caller to remember ONE cid.
fn cold_read_with_frontier(
    source: &MemBlocks,
    root: &Cid,
    key: &[u8],
    node: Node,
) -> (u64, u64, u32, bool) {
    let store = Counting::empty();
    let mut frontier = *root;
    let mut entries = 0u32;
    let mut served: Vec<Cid> = Vec::new();

    loop {
        entries += 1;
        if entries > 64 {
            return (*store.reads.borrow(), *store.bytes.borrow(), entries, false);
        }
        match get(&store, &frontier, key) {
            Ok(found) => {
                return (
                    *store.reads.borrow(),
                    *store.bytes.borrow(),
                    entries,
                    found.is_some(),
                )
            }
            Err(ReadError::Need(cids)) => {
                let Some(next) = cids.first().copied() else {
                    return (*store.reads.borrow(), *store.bytes.borrow(), entries, false);
                };
                let Some(bytes) = source.get(&next) else {
                    return (*store.reads.borrow(), *store.bytes.borrow(), entries, false);
                };
                // Same eviction rule as the re-descending arm: only the most
                // recently served block survives.
                if node == Node::Forgetting {
                    for cid in served.drain(..) {
                        store.forget(&cid);
                    }
                }
                store.put(next, bytes.to_vec());
                served.push(next);
                // The block that just arrived is where the next attempt starts.
                frontier = next;
            }
            Err(e) => panic!("unexpected read error: {e:?}"),
        }
    }
}

fn tree(n: usize) -> (MemBlocks, Cid, Vec<Vec<u8>>) {
    let mut blocks = MemBlocks::default();
    let mut root = init(&mut blocks);
    let keys: Vec<Vec<u8>> = (0..n).map(|i| format!("k{i:06}").into_bytes()).collect();
    for chunk in keys.chunks(64) {
        let edits: Vec<(Vec<u8>, Edit)> = chunk
            .iter()
            .map(|k| (k.clone(), Edit::Put(vec![b'v'; 200])))
            .collect();
        root = apply_into(&mut blocks, &root, &edits).unwrap().root;
    }
    (blocks, root, keys)
}

/// The per-entry read counts, which are the whole cost model.
///
/// Entry 1 reads the root and stops; entry 2 reads the root and its child;
/// entry N reads N levels. So a read of height `h` costs `h(h+1)/2` reads
/// where one descent costs `h` — the ratio is `(h+1)/2`, LINEAR in height.
/// Measured at two heights below rather than asserted from the formula.
fn measure(entries_in_tree: usize) -> (usize, f64, f64, f64, usize, usize) {
    let (source, root, keys) = tree(entries_in_tree);
    let height = freenet_prolly::read::height(&source, &root).unwrap();
    let mut d_blocks = 0.0;
    let mut a_reads = 0.0;
    let mut a_bytes_ratio = 0.0;
    let mut n = 0.0;
    let mut answered_forgetting = 0;
    let mut sampled = 0;
    let step = (keys.len() / 12).max(1);
    for key in keys.iter().step_by(step) {
        let want = expected(&source, &root, Op::Get(key));
        let (r_reads, r_bytes, _e, ok) = cold_read(&source, &root, key, Node::Retaining);
        assert!(ok);
        let (_f_reads, _f_bytes, f_ok) = {
            let (a, b, _c, d) = cold_read(&source, &root, key, Node::Forgetting);
            (a, b, d)
        };
        if f_ok {
            answered_forgetting += 1;
        }
        d_blocks += want.blocks as f64;
        a_reads += r_reads as f64;
        a_bytes_ratio += r_bytes as f64 / want.bytes as f64;
        n += 1.0;
        sampled += 1;
    }
    (
        height,
        d_blocks / n,
        a_reads / n,
        a_bytes_ratio / n,
        answered_forgetting,
        sampled,
    )
}

#[test]
fn the_re_descent_ratio_grows_with_the_trees_height() {
    let mut rows = Vec::new();
    for size in [2_000usize, 20_000, 160_000] {
        let (h, d, a, bytes_ratio, ans, sampled) = measure(size);
        println!(
            "{size:>7} entries  height {h}  derived {d:.1} blocks  actual {a:.1} reads  \
             reads {:.2}x  bytes {bytes_ratio:.2}x  forgetting answered {ans}/{sampled}",
            a / d
        );
        rows.push((h, a / d, ans));
    }
    // The prediction: the ratio is (h+1)/2, so a taller tree wastes MORE.
    for (h, ratio, _) in &rows {
        let predicted = (*h as f64 + 1.0) / 2.0;
        assert!(
            (ratio - predicted).abs() < 0.26,
            "at height {h} the measured ratio {ratio:.2} is not the predicted {predicted:.2}"
        );
    }
    // And the availability finding, which is not a ratio at all.
    for (_, _, answered) in &rows {
        assert_eq!(
            *answered, 0,
            "a node that drops what it served answered a read: the re-descent \
             would have to be gone for that"
        );
    }
}

#[test]
fn what_the_re_descent_costs() {
    let (source, root, keys) = tree(20_000);
    let height = freenet_prolly::read::height(&source, &root).unwrap();
    assert!(height >= 3, "a shallow tree would understate the effect");

    let mut retaining = Vec::new();
    let mut forgetting = Vec::new();

    for key in keys.iter().step_by(1_301) {
        // The DERIVATION: what one descent should cost, from the tree alone.
        let want = expected(&source, &root, Op::Get(key));
        assert_eq!(want.basis, Basis::Exact);

        let (r_reads, r_bytes, r_entries, ok) = cold_read(&source, &root, key, Node::Retaining);
        assert!(ok, "the retaining node answered the read");
        let (f_reads, f_bytes, _f_entries, f_ok) = cold_read(&source, &root, key, Node::Forgetting);

        retaining.push((want.blocks, want.bytes, r_reads, r_bytes, r_entries));
        forgetting.push((f_reads, f_bytes, f_ok));
    }

    let n = retaining.len() as f64;
    let d_blocks: f64 = retaining.iter().map(|x| x.0 as f64).sum::<f64>() / n;
    let d_bytes: f64 = retaining.iter().map(|x| x.1 as f64).sum::<f64>() / n;
    let a_reads: f64 = retaining.iter().map(|x| x.2 as f64).sum::<f64>() / n;
    let a_bytes: f64 = retaining.iter().map(|x| x.3 as f64).sum::<f64>() / n;
    let entries: f64 = retaining.iter().map(|x| x.4 as f64).sum::<f64>() / n;
    let answered = forgetting.iter().filter(|x| x.2).count();

    println!("tree height {height}, {} keys sampled", retaining.len());
    println!("DERIVED (one descent)   blocks {d_blocks:.1}  bytes {d_bytes:.0}");
    println!("ACTUAL  (re-descending) reads  {a_reads:.1}  bytes {a_bytes:.0}  over {entries:.1} entries");
    println!(
        "GAP  reads {:.2}x   bytes {:.2}x",
        a_reads / d_blocks,
        a_bytes / d_bytes
    );
    println!(
        "FORGETTING node: {answered} of {} reads answered",
        forgetting.len()
    );

    // The shape the issue predicts, asserted so the measurement cannot rot
    // into a number nobody checks.
    assert!(
        a_reads > d_blocks,
        "re-descending must cost more reads than one descent"
    );
}

/// THE ACCEPTANCE ROW (sdk#34): a node that keeps nothing still answers.
///
/// Ruled to be built for IMPOSSIBILITY, not speed. The re-descending read
/// cannot complete against a node that drops a block right after serving it,
/// because what it needs next is gone before it is used. Continuing from the
/// block that just arrived uses every block at the moment it arrives, so the
/// same node answers.
///
/// The ratio rows are reported beside it and are NOT the case for this change.
#[test]
fn continuing_from_the_frontier_answers_a_node_that_keeps_nothing() {
    for size in [2_000usize, 20_000, 160_000] {
        let (source, root, keys) = tree(size);
        let height = freenet_prolly::read::height(&source, &root).unwrap();
        let step = (keys.len() / 12).max(1);

        let (mut redescend_ok, mut frontier_ok, mut sampled) = (0, 0, 0);
        let (mut d_blocks, mut f_reads, mut r_reads) = (0.0f64, 0.0f64, 0.0f64);

        for key in keys.iter().step_by(step) {
            sampled += 1;
            let want = expected(&source, &root, Op::Get(key));
            d_blocks += want.blocks as f64;

            // THE CONTROL: the un-fixed path must still fail this row.
            let (rr, _, _, r_ok) = cold_read(&source, &root, key, Node::Forgetting);
            if r_ok {
                redescend_ok += 1;
            }
            r_reads += rr as f64;

            let (fr, _, _, f_ok) = cold_read_with_frontier(&source, &root, key, Node::Forgetting);
            if f_ok {
                frontier_ok += 1;
            }
            f_reads += fr as f64;
        }

        println!(
            "{size:>7} entries  height {height}  FORGETTING node: re-descending answered \
             {redescend_ok}/{sampled}, frontier answered {frontier_ok}/{sampled}  \
             (reads: derived {:.1}, re-descending {:.1}, frontier {:.1})",
            d_blocks / sampled as f64,
            r_reads / sampled as f64,
            f_reads / sampled as f64
        );

        assert_eq!(
            redescend_ok, 0,
            "the CONTROL stopped failing: the un-fixed path answered a node \
             that keeps nothing, so this test no longer shows what it claims"
        );
        assert_eq!(
            frontier_ok, sampled,
            "the acceptance row: every read must be answered by a node that \
             keeps nothing"
        );
    }
}
