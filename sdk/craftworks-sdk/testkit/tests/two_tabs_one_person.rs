//! sdk#265: two tabs of ONE person (one node, one signer, one Register) write
//! at the same moment. EVERY write a tab is told Published must be in the
//! tree a fresh tab reads — the same identity never forks, and a same-seq
//! race is merged key by key (#225b, #246).

use protocol::{Op, Reply, Request, WriteState};
use testkit::PageNode;

fn put(id: u64, k: &str) -> Request {
    Request::forced_write(id, vec![Op::Put(k.as_bytes().to_vec(), k.as_bytes().to_vec())])
}

/// Write states a batch of reply frames told, by (write id).
fn told(frames: &[Vec<u8>]) -> Vec<(u64, WriteState)> {
    frames
        .iter()
        .filter_map(|f| protocol::decode_reply(f).ok())
        .filter_map(|r| match r {
            Reply::WriteState { write_id, state } | Reply::SessionWriteState { write_id, state, .. } => Some((write_id, state)),
            _ => None,
        })
        .collect()
}

fn keys_on_node(node: &PageNode) -> Vec<String> {
    let (_, root) = node.head().expect("a head");
    node.tree(&root).expect("the tree is held").keys().map(|k| String::from_utf8_lossy(k).into_owned()).collect()
}

/// The 3-row case: x puts x1, y puts y1, their answers interleaved one by
/// one, then x puts x2. A write told Published that the tree lacks is data
/// loss.
#[test]
fn every_write_told_published_by_either_tab_is_in_the_tree() {
    let node = PageNode::new();
    let (mut x, mut y) = (node.connect(), node.connect());
    x.client(&Request::Identity);
    y.client(&Request::Identity);
    x.hold_answers();
    y.hold_answers();
    let mut xs = x.client(&put(1, "x1"));
    let mut ys = y.client(&put(1, "y1"));
    let mut heads = vec![node.head()];
    x.stop_holding();
    y.stop_holding();
    for _ in 0..200 {
        let (hx, hy) = (x.held(), y.held());
        if hx == 0 && hy == 0 {
            break;
        }
        if hx > 0 {
            xs.extend(x.release_one());
        }
        if hy > 0 {
            ys.extend(y.release_one());
        }
        if heads.last() != Some(&node.head()) {
            heads.push(node.head());
        }
    }
    xs.extend(x.client(&put(2, "x2")));
    heads.push(node.head());
    // Let both settle: time passes, both tick.
    for t in 1..=20u64 {
        xs.extend(x.tick_at(x.now_ms() + t * 1_000));
        ys.extend(y.tick_at(y.now_ms() + t * 1_000));
    }
    heads.push(node.head());
    let (tx, ty) = (told(&xs), told(&ys));
    println!("  x told {tx:?}\n  y told {ty:?}\n  register heads in order: {heads:?}");
    let published = |t: &[(u64, WriteState)], id| t.iter().any(|(w, s)| *w == id && *s == WriteState::Published);
    let keys = keys_on_node(&node);
    println!("  the tree: {keys:?}");
    for (label, t, id, key) in [("x", &tx, 1, "x1"), ("x", &tx, 2, "x2"), ("y", &ty, 1, "y1")] {
        if published(t, id) {
            assert!(keys.contains(&key.to_string()), "tab {label}'s write {key} was told Published and is NOT in the tree (sdk#265): {keys:?}");
        }
    }
    // And a fresh tab reads the same.
    let mut fresh = node.connect();
    fresh.client(&Request::Identity);
    let page = fresh.client(&Request::Range { req_id: 9, lo: protocol::Bound::Unbounded, hi: protocol::Bound::Unbounded, reverse: false, after: None, max_entries: 100 });
    let seen: Vec<String> = page
        .iter()
        .filter_map(|f| protocol::decode_reply(f).ok())
        .find_map(|r| match r {
            Reply::Page { req_id: 9, entries, .. } => Some(entries.into_iter().map(|(k, _)| String::from_utf8_lossy(&k).into_owned()).collect()),
            _ => None,
        })
        .expect("the fresh tab read");
    assert_eq!(seen, keys, "the fresh tab reads another tree than the node's head");
}

/// sdk#265, the architect's item 3 (PRECEDENCE with sdk#225b). A `Lost`
/// commit has exactly ONE owner. #225b's merge/probe classifies the keys of
/// writes that PUBLISHED on a tip the winner displaced, and tells their
/// session `Superseded`; #265 re-sends a write told `Lost`, which by
/// definition never published. So no write is ever both — and a tab told
/// `Lost` for a write is told it ONCE per attempt, never once by the engine
/// and again by the merge.
///
/// The control that the merge path is not simply absent lives beside it:
/// `page/tests/server_differential.rs` drives a displacement and asserts the
/// record IS told `Superseded`.
#[test]
fn a_lost_write_is_the_client_s_to_re_send_or_the_merge_s_to_supersede_never_both() {
    let node = PageNode::new();
    let (mut x, mut y) = (node.connect(), node.connect());
    x.client(&Request::Identity);
    y.client(&Request::Identity);
    x.hold_answers();
    y.hold_answers();
    let mut xs = x.client(&put(1, "x1"));
    let mut ys = y.client(&put(1, "y1"));
    x.stop_holding();
    y.stop_holding();
    for _ in 0..200 {
        let (hx, hy) = (x.held(), y.held());
        if hx == 0 && hy == 0 {
            break;
        }
        if hx > 0 {
            xs.extend(x.release_one());
        }
        if hy > 0 {
            ys.extend(y.release_one());
        }
    }
    for t in 1..=20u64 {
        xs.extend(x.tick_at(x.now_ms() + t * 1_000));
        ys.extend(y.tick_at(y.now_ms() + t * 1_000));
    }
    let superseded = |frames: &[Vec<u8>]| -> Vec<u64> {
        frames
            .iter()
            .filter_map(|f| protocol::decode_reply(f).ok())
            .filter_map(|r| match r {
                Reply::Superseded { write_id, .. } => Some(write_id),
                _ => None,
            })
            .collect()
    };
    let mut saw_lost = false;
    for (label, frames) in [("x", &xs), ("y", &ys)] {
        let lost: Vec<u64> = told(frames).into_iter().filter(|(_, s)| *s == WriteState::Lost).map(|(w, _)| w).collect();
        saw_lost |= !lost.is_empty();
        let merged = superseded(frames);
        for id in &lost {
            assert!(!merged.contains(id), "tab {label}'s write {id} was told Lost AND superseded by the merge: two owners, two rounds");
        }
        println!("  tab {label}: told Lost {lost:?}, told Superseded {merged:?}");
    }
    assert!(saw_lost, "no tab was told Lost in this race: the test cannot say anything about who owns a Lost");
}

/// One tab as the SDK drives it: a `PageStore` over its own `page::Server`.
struct Tab {
    store: craftworks_sdk::PageStore<testkit::PageConn>,
    conn: testkit::PageConn,
}

impl Tab {
    fn open(node: &PageNode) -> Tab {
        let (mut store, conn, _clock) = testkit::page_store(node);
        store.sync();
        Tab { store, conn }
    }

    /// Carry what is waiting both ways.
    fn pump(&mut self) {
        self.store.sync();
    }

    /// Deliver ONE held node answer, then what it caused.
    fn release(&mut self) {
        let _ = self.conn.release_one();
        self.pump();
    }
}

/// sdk#265 AT THE CLIENT: the same race, each tab a `PageStore` as the SDK
/// runs it. The tab that loses the same-seq race has its in-flight write's
/// commit die: a FOREIGN MOVE, and its ENGINE re-applies the write at the
/// front on the head that won (R-b; go-back-N in the engine, once). Every row
/// both tabs wrote is in the tree, and neither tab holds an unsaved write.
#[test]
fn two_tabs_writing_at_once_lose_nothing_at_the_client() {
    use craftworks_sdk::store::Store as _;
    // Each write WITH its read, as a `Db` write makes it. A store-level `put`
    // is FORCED (`Expect::Any`, sdk#235) and a forced write whose commit dies
    // falls, named, instead of going again -- so it would not test #265's
    // go-again, which is what this test is for.
    let create = |k: &[u8]| (vec![(k.to_vec(), protocol::Expect::Absent)], vec![(k.to_vec(), craftworks_sdk::store::Edit::Put(k.to_vec()))]);
    let node = PageNode::new();
    let (mut x, mut y) = (Tab::open(&node), Tab::open(&node));
    x.conn.hold_answers();
    y.conn.hold_answers();
    let (r, e) = create(b"x1");
    x.store.apply_commit(&r, &e).expect("x takes x1");
    x.pump();
    let (r, e) = create(b"y1");
    y.store.apply_commit(&r, &e).expect("y takes y1");
    y.pump();
    x.conn.stop_holding();
    y.conn.stop_holding();
    for _ in 0..400 {
        if x.conn.held() == 0 && y.conn.held() == 0 {
            break;
        }
        if x.conn.held() > 0 {
            x.release();
        }
        if y.conn.held() > 0 {
            y.release();
        }
    }
    let (r, e) = create(b"x2");
    x.store.apply_commit(&r, &e).expect("x takes x2");
    x.pump();
    for t in 1..=30u64 {
        for tab in [&mut x, &mut y] {
            let now = tab.conn.now_ms() + t * 1_000;
            let _ = tab.conn.tick_at(now);
            tab.pump();
        }
    }
    let keys = keys_on_node(&node);
    println!("  the tree: {keys:?}; x unsaved {}, y unsaved {}", x.store.unsaved_writes(), y.store.unsaved_writes());
    assert_eq!(keys, ["x1", "x2", "y1"], "a row one of the two tabs wrote is not in the tree (sdk#265)");
    assert_eq!((x.store.unsaved_writes(), y.store.unsaved_writes()), (0, 0), "a tab still holds an unsaved write");
    let ended: Vec<_> = x.store.writes.take_ended().into_iter().chain(y.store.writes.take_ended()).collect();
    assert!(ended.is_empty(), "a write ended unpublished: {ended:?}");
}
