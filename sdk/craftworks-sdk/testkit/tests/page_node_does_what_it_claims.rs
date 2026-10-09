//! The page path's node fixture (`testkit::page_node`) does what it claims.
//!
//! A fixture is trusted by every test built on it, so each claim it makes is
//! checked here against something it cannot fake: the REAL signer's record,
//! the REAL Register's merged state, a count per op kind.

use testkit::page_node::{PageNode, Served};

fn put(id: u64, k: &str, v: &str) -> protocol::Request {
    protocol::Request::forced_write(id, vec![protocol::Op::Put(k.as_bytes().to_vec(), v.as_bytes().to_vec())])
}

fn published(frames: &[Vec<u8>], id: u64) -> bool {
    frames.iter().filter_map(|f| protocol::decode_reply(f).ok()).any(|r| {
        matches!(r, protocol::Reply::WriteState { write_id, state: protocol::WriteState::Published }
            | protocol::Reply::SessionWriteState { write_id, state: protocol::WriteState::Published, .. } if write_id == id)
    })
}

/// A write through a tab PUBLISHES: blocks put, the head SIGNED by the real
/// signer and UPDATED through the real Register — and the head the node holds
/// is the root of a tree that holds the write.
#[test]
fn a_write_publishes_through_the_real_signer_and_register() {
    let node = PageNode::new();
    let mut tab = node.connect();
    tab.client(&protocol::Request::Identity);
    assert!(node.head().is_none(), "a fresh node already has a head");
    let out = tab.client(&put(1, "k", "v"));
    assert!(published(&out, 1), "the write did not publish: {:?}", tab.replies());
    for what in [Served::Put, Served::Sign, Served::Head] {
        assert!(tab.served(what) > 0, "the node never served {what:?}: a fixture that answers less than it claims");
    }
    let (seq, root) = node.head().expect("the Register holds a head");
    assert_eq!(seq, 1);
    let tree = node.tree(&root).expect("every block of the head's tree is on the node");
    assert_eq!(tree.get(b"k".as_slice()).map(Vec::as_slice), Some(b"v".as_slice()), "the head's tree lacks the write");
}

/// TWO TABS, ONE NODE: each has its own engine, and a second tab opened after
/// the first published reads the first's head and its rows.
#[test]
fn two_tabs_share_one_node_and_each_has_its_own_engine() {
    let node = PageNode::new();
    let mut a = node.connect();
    a.client(&protocol::Request::Identity);
    assert!(published(&a.client(&put(1, "k", "from a")), 1));
    let mut b = node.connect();
    b.client(&protocol::Request::Identity);
    let got = b.client(&protocol::Request::Get { req_id: 9, key: b"k".to_vec() });
    let value = got.iter().filter_map(|f| protocol::decode_reply(f).ok()).find_map(|r| match r {
        protocol::Reply::Value { req_id: 9, value, .. } => Some(value),
        _ => None,
    });
    assert_eq!(value, Some(Some(b"from a".to_vec())), "tab B did not read tab A's published row");
    assert_eq!(b.served(Served::Put), 0, "tab B put blocks it never wrote: the tabs share one engine");
}

/// HOLDING holds: with answers held, a write does not publish; released one by
/// one, it does.
#[test]
fn held_answers_are_held_and_release_delivers_them() {
    let node = PageNode::new();
    let mut tab = node.connect();
    tab.client(&protocol::Request::Identity);
    tab.hold_answers();
    let out = tab.client(&put(1, "k", "v"));
    assert!(!published(&out, 1), "published with every answer held");
    assert!(tab.held() > 0, "holding queued nothing");
    tab.stop_holding();
    let mut all = Vec::new();
    for _ in 0..100 {
        if tab.held() == 0 {
            break;
        }
        all.extend(tab.release_one());
    }
    assert!(published(&all, 1), "released, the write still did not publish");
}

/// A COLD node holds no block and answers a GET from the network, keeping
/// what it fetched: a second tab reading the same row asks its node (every tab
/// has its own memory) and the node answers it LOCALLY, off the network.
#[test]
fn a_cold_node_fetches_from_the_network_and_keeps_what_it_fetched() {
    let warm = PageNode::new();
    let mut w = warm.connect();
    w.client(&protocol::Request::Identity);
    assert!(published(&w.client(&put(1, "k", "v")), 1));
    let cold = PageNode::cold_over(&warm);
    assert_eq!(cold.blocks(), 0, "a cold node started with blocks");
    let mut c = cold.connect();
    c.client(&protocol::Request::Identity);
    c.client(&protocol::Request::Get { req_id: 1, key: b"k".to_vec() });
    let far = cold.network_fetches();
    assert!(far > 0 && c.served(Served::Get) >= far, "a cold read fetched nothing from the network");
    assert!(cold.blocks() > 0, "the cold node kept nothing it fetched");
    let mut c2 = cold.connect();
    c2.client(&protocol::Request::Identity);
    let got = c2.client(&protocol::Request::Get { req_id: 1, key: b"k".to_vec() });
    assert!(got.iter().filter_map(|f| protocol::decode_reply(f).ok()).any(|r| matches!(r, protocol::Reply::Value { req_id: 1, value: Some(_) })), "the second tab did not read the row");
    assert!(c2.served(Served::Get) > 0, "the second tab asked its node nothing: it cannot have read the row");
    assert_eq!(cold.network_fetches(), far, "a block the node fetched once was fetched from the network again");
}

/// The CLOCK a test drives (re-stated from the Shell fixture's own test): the
/// closure handed to a constructor reads the live clock, not a snapshot.
#[test]
fn the_clock_can_be_driven() {
    let c = testkit::Clock::new(1_000);
    assert_eq!(c.now_ms(), 1_000);
    c.advance(500);
    assert_eq!(c.now_ms(), 1_500);
    let f = c.as_fn();
    assert_eq!(f(), 1_500);
    c.advance(250);
    assert_eq!(f(), 1_750, "the closure reads the live clock, not a snapshot");
}
