//! Writes through the PAGE (R-b): the engine owns the queue, so what the
//! outbox's tests asserted about bursts, order and refusals is asserted here
//! against the real page -- `PageStore` over `page::Server` over the node --
//! and against the tree the node publishes, never against a copy.
//!
//! Ported from `write_burst_drains.rs`, `outbox_multi_key.rs`,
//! `write_drain.rs`, `write_window.rs` and `write_refusals.rs`, whose outbox
//! (window, `Busy` re-send, timeout) went with R-b. sdk#291 is here too: a
//! write waiting its turn behind the engine's own commits is never rolled
//! back, however long the queue takes.

use craftworks_sdk::store::{Edit, Store};
use craftworks_sdk::{PageStore, Refused};
use testkit::{PageConn, PageNode};

fn store(node: &PageNode) -> (PageStore<PageConn>, PageConn, testkit::Clock) {
    let (mut s, conn, clock) = testkit::page_store(node);
    s.sync();
    (s, conn, clock)
}

/// The node answers everything it was holding, and what that causes.
fn release_all(s: &mut PageStore<PageConn>, conn: &mut PageConn) {
    conn.stop_holding();
    for step in 0.. {
        assert!(step < 200_000, "the node was still answering after 200,000 releases");
        if conn.held() == 0 {
            break;
        }
        let _ = conn.release_one();
        s.sync();
    }
    s.sync();
}

/// The tree the node published, as (key, value) rows.
fn published(node: &PageNode) -> std::collections::BTreeMap<Vec<u8>, Vec<u8>> {
    let (_, root) = node.head().expect("a head");
    node.tree(&root).expect("the tree is whole")
}

/// Every write of this store ended Published: nothing ended any other way.
fn nothing_ended_unpublished(s: &mut PageStore<PageConn>) {
    let ended = s.writes.take_ended();
    assert!(ended.is_empty(), "writes ended unpublished: {ended:?}");
    assert!(s.writes.take_conflicts().is_empty());
    assert!(s.writes.take_unread().is_empty());
}

/// **A burst of writes behind a commit in flight all reach the tree, in the
/// order they were made** (`write_burst_drains.rs`): twenty writes, the node
/// holding its answers, then answering. A key written twice keeps the LATER
/// value -- a later edit landing before an earlier one would leave the tree
/// holding the older value.
#[test]
fn a_burst_behind_a_commit_in_flight_all_publishes_in_order() {
    let node = PageNode::new();
    let (mut s, mut conn, _clock) = store(&node);
    conn.hold_answers();
    for i in 0..20u32 {
        s.put(format!("d/b/{i:02}").as_bytes(), b"first").expect("taken");
    }
    s.put(b"d/b/05", b"second").expect("taken");
    assert_eq!(s.queue_load().0, 21, "the burst is not all in the engine's queue");
    release_all(&mut s, &mut conn);
    let tree = published(&node);
    assert_eq!(tree.len(), 20, "not every key of the burst is in the tree");
    assert_eq!(tree.get(&b"d/b/05"[..]).map(Vec::as_slice), Some(&b"second"[..]), "the later edit of d/b/05 did not win: writes landed out of order");
    assert_eq!(s.unsaved_writes(), 0);
    nothing_ended_unpublished(&mut s);
}

/// THE CONTROL: with nothing made, nothing is sent -- a drain that sent
/// unconditionally would pass the test above.
#[test]
fn control_an_idle_page_sends_no_write() {
    let node = PageNode::new();
    let (mut s, conn, _clock) = store(&node);
    s.sync();
    let before = conn.served(testkit::page_node::Served::Put);
    for _ in 0..5 {
        s.sync();
    }
    assert_eq!(conn.served(testkit::page_node::Served::Put), before, "an idle page put something");
    assert!(node.head().is_none(), "an idle page published a head");
}

/// **Multi-key writes in a burst all publish** (`outbox_multi_key.rs`, sdk#139:
/// a 2-key write once stopped the outbox for good). Single- and multi-key
/// interleaved, behind a commit in flight.
#[test]
fn a_mixed_burst_of_single_and_multi_key_writes_all_publishes() {
    let node = PageNode::new();
    let (mut s, mut conn, _clock) = store(&node);
    conn.hold_answers();
    let mut keys = 0;
    for i in 0..12u32 {
        let edits: Vec<(Vec<u8>, Edit)> = if i % 3 == 0 {
            vec![(format!("d/m/{i:02}/a").into_bytes(), Edit::Put(b"a".to_vec())), (format!("d/m/{i:02}/b").into_bytes(), Edit::Put(b"b".to_vec()))]
        } else {
            vec![(format!("d/m/{i:02}").into_bytes(), Edit::Put(b"x".to_vec()))]
        };
        keys += edits.len();
        s.apply_batch(&edits).expect("taken");
    }
    release_all(&mut s, &mut conn);
    assert_eq!(published(&node).len(), keys, "a write of the mixed burst is not in the tree");
    assert_eq!(s.unsaved_writes(), 0);
    nothing_ended_unpublished(&mut s);
}

/// **300 writes, the node answering as they go: all 300 land, none refused**
/// (`write_drain.rs`: 256 landed and 44 were refused when nothing cleared a
/// pending write; `write_window.rs`: 300 rows).
#[test]
fn three_hundred_writes_all_land() {
    let node = PageNode::new();
    let (mut s, _conn, _clock) = store(&node);
    for i in 0..300u32 {
        s.put(format!("d/r/{i:04}").as_bytes(), b"row").expect("taken");
    }
    s.sync();
    assert_eq!(published(&node).len(), 300, "not every write landed");
    assert_eq!(s.unsaved_writes(), 0);
    nothing_ended_unpublished(&mut s);
}

/// **sdk#291: A QUEUE THAT WAITS LONGER THAN A MINUTE ROLLS NOTHING BACK.**
/// The outbox timed a write out 60 s after it was sent, and a write waiting
/// its turn behind one-commit-at-a-time was "unanswered" by that measure:
/// measured live, 34 of 3,000 paced puts rolled back in the tail, the records
/// gone while "saving" read 0. Here the node holds every answer for two
/// minutes of ticks: nothing ends, every write stays unsaved and says so, and
/// when the node answers every one publishes.
#[test]
fn a_queue_waiting_past_a_minute_rolls_nothing_back() {
    let node = PageNode::new();
    let (mut s, mut conn, clock) = store(&node);
    conn.hold_answers();
    for i in 0..50u32 {
        s.put(format!("d/q/{i:02}").as_bytes(), b"row").expect("taken");
    }
    for _ in 0..120 {
        clock.advance(1_000);
        let now = clock.now_ms();
        let _ = conn.tick_at(now);
        let _ = s.ask_after_applying();
        s.sync();
    }
    assert!(s.writes.take_ended().is_empty(), "a queued write was ended while it waited its turn (sdk#291)");
    assert_eq!(s.unsaved_writes(), 50, "a waiting write stopped being counted as unsaved -- 'saving' would read short");
    release_all(&mut s, &mut conn);
    assert_eq!(published(&node).len(), 50, "a write that waited did not publish");
    assert_eq!(s.unsaved_writes(), 0);
    nothing_ended_unpublished(&mut s);
}

/// 200 DISTINCT 2 KiB values in ONE write: over the engine's commit-block
/// budget (the default 128).
fn bulk(extra: &[&str]) -> Vec<(Vec<u8>, Edit)> {
    let mut edits: Vec<(Vec<u8>, Edit)> = (0..200u64)
        .map(|i| {
            let mut v = vec![0u8; 2048];
            v[..8].copy_from_slice(&i.to_le_bytes());
            (format!("d/bulk/{i:05}").into_bytes(), Edit::Put(v))
        })
        .chain(extra.iter().map(|k| (k.as_bytes().to_vec(), Edit::Put(b"from the bulk write".to_vec()))))
        .collect();
    edits.sort_by(|a, b| a.0.cmp(&b.0));
    edits
}

/// **A write the engine can never accept is refused ONCE, by name, at its
/// door -- and the writes behind it all publish** (`write_refusals.rs`,
/// craftworks-sdk#136). It is refused in the call that made it (the door is
/// synchronous in the page), so it is never in the queue, never shown, and
/// never sent again.
#[test]
fn a_write_the_engine_can_never_accept_is_refused_once_and_the_writes_behind_it_publish() {
    let node = PageNode::new();
    let (mut s, mut conn, _clock) = store(&node);
    conn.hold_answers();
    s.put(b"d/notes/first", b"x").expect("taken");
    let r = s.apply_batch(&bulk(&["d/shared"]));
    assert!(
        matches!(r, Err(Refused::TooLarge { bound: protocol::WriteBound::CommitBlocks, limit: 128, got }) if got > 128),
        "the bulk write was not refused TooLarge at its door: {r:?}"
    );
    // B writes the key the refused A would have: A is in no root, so B is
    // simply a write on the tree -- and publishes.
    s.put(b"d/shared", b"from B").expect("taken");
    for i in 0..10u32 {
        s.put(format!("d/notes/{i:03}").as_bytes(), b"x").expect("taken");
    }
    release_all(&mut s, &mut conn);
    let tree = published(&node);
    assert!(!tree.keys().any(|k| k.starts_with(b"d/bulk/")), "a key of the refused write is in the tree");
    assert_eq!(tree.get(&b"d/shared"[..]).map(Vec::as_slice), Some(&b"from B"[..]));
    assert_eq!(tree.len(), 12, "not every write behind the refused one published");
    nothing_ended_unpublished(&mut s);
}

/// THE WIRE BOUND: a write that cannot be SENT is refused on the spot, before
/// anything is framed (`write_refusals.rs`, craftworks-sdk#136).
#[test]
fn a_write_too_large_to_send_is_refused_before_it_is_sent() {
    let node = PageNode::new();
    let (mut s, _conn, _clock) = store(&node);
    let big = vec![7u8; protocol::MAX_MESSAGE + 1];
    let r = s.put(b"d/huge", &big);
    assert!(matches!(r, Err(Refused::TooLargeToSend { .. })), "{r:?}");
    assert_eq!(s.queue_load().0, 0, "the refused write reached the engine");
    assert!(node.head().is_none(), "the refused write published");
}

/// Commits and writes the page's engine has published, and the K9 counts.
fn commits_and_writes(conn: &PageConn) -> ((u64, u64), (u64, u64, u64)) {
    conn.with_server(|s| (s.page.commits_and_writes(), s.page.queue_counts()))
}

/// A burst of `n` writes made while the node holds its answers (one round
/// trip in flight), then answered: every one lands, none ends any other way,
/// and they ride FEW commits -- group commit (COMMIT-LIFE K9).
fn burst(n: u32) -> (u64, u64) {
    let node = PageNode::new();
    let (mut s, mut conn, _clock) = store(&node);
    conn.hold_answers();
    for i in 0..n {
        s.put(format!("d/g/{i:05}").as_bytes(), b"row").expect("taken");
    }
    release_all(&mut s, &mut conn);
    assert_eq!(published(&node).len(), n as usize, "not every write of the burst landed");
    assert_eq!(s.unsaved_writes(), 0);
    nothing_ended_unpublished(&mut s);
    let ((commits, writes), (differs, rederived, impossible)) = commits_and_writes(&conn);
    assert_eq!((differs, rederived, impossible), (0, 0, 0), "K9 counts");
    assert_eq!(writes, n as u64, "a write published outside a commit's count");
    (commits, writes)
}

/// **GROUP COMMIT, MEASURED: 300 and 3,000 writes land in a bounded number of
/// commits** (K9; core dev's acceptance). One write per commit made the round
/// trip the ceiling (~5 min for 300); the cut takes every write queued behind
/// the commit in flight, split only at the commit's block limit.
#[test]
fn a_burst_rides_few_commits() {
    for n in [300u32, 3_000] {
        let (commits, writes) = burst(n);
        println!("  {n} writes: {commits} commits, {:.1} writes per commit", writes as f64 / commits as f64);
        assert!(commits * 10 <= n as u64, "{n} writes took {commits} commits: not grouped");
    }
}

/// **A SPLIT GROUP** (K9 §4): with a small commit limit the cut ships in
/// pieces, each built on the one before, in order; every write lands, the
/// K9 count stays 0, and a later write never lands under an earlier one.
#[test]
fn a_group_past_the_commit_limit_ships_in_pieces_in_order() {
    let node = PageNode::new();
    let conn = node.connect_with(engine::Params { max_commit_blocks: 8, ..engine::Params::default() });
    let clock = testkit::Clock::new(0);
    let mut s = PageStore::new(clock.as_fn(), clock.as_fn());
    s.writes.client.send(&protocol::Request::Identity);
    s.set_host(conn.clone());
    s.sync();
    let mut c = conn.clone();
    c.hold_answers();
    for i in 0..200u32 {
        s.put(format!("d/p/{:03}", i % 50).as_bytes(), format!("v{i}").as_bytes()).expect("taken");
    }
    release_all(&mut s, &mut c);
    let tree = published(&node);
    for k in 0..50u32 {
        assert_eq!(tree.get(format!("d/p/{k:03}").as_bytes()).map(Vec::as_slice), Some(format!("v{}", 150 + k).as_bytes()), "key {k}: not the LAST write's value");
    }
    let ((commits, writes), counts) = commits_and_writes(&conn);
    println!("  200 writes, commit limit 8 blocks: {commits} commits (pieces), {writes} writes");
    assert_eq!(counts, (0, 0, 0), "K9 counts");
    assert!(commits > 2, "the group was never split: the limit is not what this measures");
    nothing_ended_unpublished(&mut s);
}

/// **BACKPRESSURE, END TO END** (K1): a burst past a small byte bound is told
/// QUEUE_FULL at the door; the writer waits for the queue to drain (the SDK's
/// `write()` does, on the session's wake) and makes the SAME write again --
/// and every write lands, none vanishes, none is refused for good.
#[test]
fn a_burst_past_the_byte_bound_waits_and_every_write_lands() {
    let node = PageNode::new();
    let conn = node.connect_with(engine::Params { max_queue_bytes: 400, ..engine::Params::default() });
    let clock = testkit::Clock::new(0);
    let mut s = PageStore::new(clock.as_fn(), clock.as_fn());
    s.writes.client.send(&protocol::Request::Identity);
    s.set_host(conn.clone());
    s.sync();
    let mut c = conn.clone();
    c.hold_answers();
    let (mut waits, n) = (0, 100u32);
    for i in 0..n {
        let k = format!("d/w/{i:03}");
        loop {
            match s.put(k.as_bytes(), b"row") {
                Ok(()) => break,
                Err(Refused::QueueFull { .. }) => {
                    // The wait: the node answers, a commit publishes, room.
                    waits += 1;
                    assert!(waits < 10_000, "the queue never drained");
                    release_all(&mut s, &mut c);
                    c.hold_answers();
                }
                Err(e) => panic!("write {i} refused for good: {e:?}"),
            }
        }
    }
    release_all(&mut s, &mut c);
    println!("  {n} writes past a 400-byte bound: {waits} waits, all taken");
    assert!(waits > 0, "the bound was never met: this measures nothing");
    assert_eq!(published(&node).len(), n as usize, "a write that waited did not land");
    nothing_ended_unpublished(&mut s);
}
