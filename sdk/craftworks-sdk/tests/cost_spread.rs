//! What a client operation COSTS THE NODE, and why there is no cost ratchet.
//!
//! This began as the precondition for one: is the observed side stable enough
//! to ratchet on at all? The measurement said no, for a better reason than
//! noise — see the bottom of this file — and what survives is the invariant it
//! turned up, which is worth pinning on its own.
//!
//! The original question, kept because it is the one to re-ask if anyone
//! proposes the ratchet again:
//!
//! Asked before building, because a ratchet on a noisy number produces false
//! failures, and a detector that cries wolf is ignored inside a week — and a
//! cost ratchet fires on every PR, so it would cry often.
//!
//! Two kinds of spread, because only one of them is a real question:
//!
//! - **REPLAY spread**: the same operation on the same tree, five times. A
//!   deterministic fixture makes this zero, and a zero here proves nothing
//!   except that the harness is deterministic.
//! - **INSTANCE spread**: five DIFFERENT representative operations of the same
//!   kind — different keys, different ranges. That is the variation a ratchet
//!   actually meets, because the next PR's run is not a replay of this one.

use craftworks_sdk::expected::{expected, Op};

/// Blocks fetched per round trip: a property of the transport, not the tree.
const BATCH: u64 = 16;
use protocol::Request;
use std::ops::Bound;
use testkit::page_node::Served;
use testkit::{PageConn, PageNode};

fn write(n: u64) -> Request {
    Request::forced_write(n, vec![protocol::Op::Put(
            format!("k/{n:06}").into_bytes(),
            vec![(n % 251) as u8; 64],
        )])
}

/// What the node was asked to DO for one client operation, and what crossed.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Observed {
    ops: usize,
    bytes: u64,
}

fn observe(c: &mut PageConn, r: &Request) -> Observed {
    let before_ops = [Served::Put, Served::Get, Served::Head, Served::ReadHead, Served::Sign]
        .iter()
        .map(|s| c.served(*s))
        .sum::<usize>();
    let before_bytes = c.bytes_handed_to_this_node();
    c.client(r);
    let after_ops = [Served::Put, Served::Get, Served::Head, Served::ReadHead, Served::Sign]
        .iter()
        .map(|s| c.served(*s))
        .sum::<usize>();
    Observed {
        ops: after_ops - before_ops,
        bytes: c.bytes_handed_to_this_node() - before_bytes,
    }
}

fn spread(xs: &[u64]) -> String {
    let lo = xs.iter().min().copied().unwrap_or(0);
    let hi = xs.iter().max().copied().unwrap_or(0);
    let mean = xs.iter().sum::<u64>() as f64 / xs.len().max(1) as f64;
    let rel = if mean > 0.0 {
        (hi - lo) as f64 / mean
    } else {
        0.0
    };
    format!("min {lo} max {hi} mean {mean:.1} spread {rel:.2}x")
}

/// A WARM read costs the node NOTHING, and that is the finding.
///
/// The page reads blocks it holds (it wrote them) from its own memory; it
/// does not ask the node. So a point read and a range both cost zero node
/// operations, whatever the derived figure says the path is — and a ratchet
/// over that would pin 0 against 0 for ever and fire never.
///
/// It is also a real property: if a warm read ever starts asking the node for
/// something, that is a regression this catches, and an assertion says so
/// better than a baseline file would.
#[test]
fn a_warm_read_costs_the_node_nothing_and_a_commit_costs_its_named_mix() {
    let node = PageNode::new();
    let mut c = node.connect();
    c.client(&Request::Identity);
    for i in 1..=400 {
        c.client(&write(i));
    }

    let root = node.head().expect("a head after 400 writes").1;
    let store = node.clone();

    // ---- COMMIT: five DIFFERENT writes ------------------------------------
    let kinds = [Served::Put, Served::Get, Served::Head, Served::ReadHead, Served::Sign];
    let mut commit_ops = Vec::new();
    let mut commit_bytes = Vec::new();
    let mut commit_kinds = Vec::new();
    for i in 401..=405 {
        let before: Vec<usize> = kinds.iter().map(|k| c.served(*k)).collect();
        let o = observe(&mut c, &write(i));
        commit_kinds.push(kinds.iter().zip(before).map(|(k, b)| (*k, c.served(*k) - b)).collect::<Vec<_>>());
        commit_ops.push(o.ops as u64);
        commit_bytes.push(o.bytes);
    }

    // ---- COMMIT REPLAY: the same write, five times ------------------------
    let mut replay_ops = Vec::new();
    for _ in 0..5 {
        let o = observe(&mut c, &write(500));
        replay_ops.push(o.ops as u64);
    }

    // ---- POINT READ: five different keys ----------------------------------
    let mut read_ops = Vec::new();
    let mut read_derived = Vec::new();
    for (n, i) in (1..=400).step_by(80).enumerate() {
        let key = format!("k/{i:06}").into_bytes();
        let o = observe(
            &mut c,
            &Request::Get {
                req_id: 9000 + n as u64,
                key: key.clone(),
            },
        );
        read_ops.push(o.ops as u64);
        read_derived.push(expected(&store, &root, Op::Get(&key)).blocks);
    }

    // ---- RANGE: five different ranges -------------------------------------
    let mut range_ops = Vec::new();
    let mut range_derived = Vec::new();
    for (n, i) in (1..=300).step_by(60).enumerate() {
        let lo = format!("k/{i:06}").into_bytes();
        let hi = format!("k/{:06}", i + 50).into_bytes();
        let o = observe(
            &mut c,
            &Request::Range {
                req_id: 8000 + n as u64,
                lo: protocol::Bound::Included(lo.clone()),
                hi: protocol::Bound::Included(hi.clone()),
                reverse: false,
                after: None,
                max_entries: protocol::MAX_PAGE_ENTRIES,
            },
        );
        range_ops.push(o.ops as u64);
        range_derived.push(
            expected(
                &store,
                &root,
                Op::Scan {
                    lo: Bound::Included(&lo),
                    hi: Bound::Included(&hi),
                    batch: BATCH,
                },
            )
            .blocks,
        );
    }

    println!("\n=== observed node-ops per client operation, real fixture ===");
    println!("commit  (5 different writes) ops: {}", spread(&commit_ops));
    println!("commit  (5 different writes) bytes: {}", spread(&commit_bytes));
    println!("commit  (SAME write x5)      ops: {}", spread(&replay_ops));
    println!("point read (5 keys)          ops: {}", spread(&read_ops));
    println!("point read (5 keys)      DERIVED: {}", spread(&read_derived));
    println!("range      (5 ranges)        ops: {}", spread(&range_ops));
    println!("range      (5 ranges)    DERIVED: {}", spread(&range_derived));
    println!("\nraw commit ops {commit_ops:?}  bytes {commit_bytes:?}");
    println!("raw read ops {read_ops:?} derived {read_derived:?}");
    println!("raw range ops {range_ops:?} derived {range_derived:?}");

    // ---- what the measurement decided, as assertions ----------------------
    // Every `.all()` below is true of an EMPTY vector, so the counts come
    // first: without this the whole test passes when nothing ran.
    assert_eq!(read_ops.len(), 5, "five point reads were meant to run");
    assert_eq!(range_ops.len(), 5, "five ranges were meant to run");
    assert_eq!(commit_ops.len(), 5, "five commits were meant to run");
    assert_eq!(read_derived.len(), 5);

    assert!(
        read_ops.iter().all(|&n| n == 0),
        "a warm point read asked the node for something: {read_ops:?}. Either \
the delegate stopped reading held blocks directly, or the fixture stopped \
holding them — both are worth knowing.",
    );
    assert!(
        range_ops.iter().all(|&n| n == 0),
        "a warm range asked the node for something: {range_ops:?}",
    );
    assert!(
        read_derived.iter().all(|&n| n >= 2),
        "the DERIVED side must be non-zero, or the assertions above pass \
because nothing was measured rather than because nothing was asked",
    );
    // THE PAGE PATH'S COMMIT, per kind. 2 + 2 x PARITY block PUTs: the
    // changed path (root and leaf), the leaf's group's PARITY parity (race
    // put, COMMIT-LIFE §P: all in the SAME round), and the ROOT's own PARITY
    // parity (sdk#335: the root is a group of one, so a reader whose root
    // block is silent reads it from any one of them). Plus the head UPDATE, the
    // signer's request (the engine holds no key), and the head READ after the
    // UPDATE, because an UPDATE's answer says nothing about which record the
    // Register kept (F56). Per kind, so a change to any one fails by name.
    let want = vec![(Served::Put, 2 + 2 * engine::PARITY), (Served::Get, 0), (Served::Head, 1), (Served::ReadHead, 1), (Served::Sign, 1)];
    assert!(
        commit_kinds.iter().all(|k| *k == want),
        "a commit took a different mix of node operations: {commit_kinds:?}. \
2 + 2 x PARITY PUTs (root, leaf, the leaf group's parity, the root's parity: one round, §P, sdk#335), one head \
UPDATE, one signer request, one head READ; a change here is a change to \
what a write costs every app.",
    );
    assert!(
        commit_ops.iter().all(|&n| n == want.iter().map(|(_, n)| n).sum::<usize>() as u64),
        "a commit took a different number of node operations: {commit_ops:?} (the mix above sums to {})",
        want.iter().map(|(_, n)| n).sum::<usize>(),
    );
    // Bytes are NOT asserted: they climb monotonically with the tree
    // (2192 → 2512 over five consecutive writes), so any figure pinned here
    // would fail the next time the fixture's size changed. That drift is the
    // reason there is no bytes ratchet, and writing it down is worth more
    // than a number that would be edited away.
}
