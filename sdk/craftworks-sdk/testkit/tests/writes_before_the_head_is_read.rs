//! A WRITE MADE BEFORE THE HEAD IS READ (sdk#223's write half).
//!
//! The tree an engine starts on is EMPTY. A write applied to it commits a head
//! that knows nothing of the head the person already has: on a node that holds
//! data, that write publishes a tree with only itself in it, and everything
//! written before is gone from the head everyone reads.
//!
//! The page used to hold such writes itself. The engine now parks the first
//! and answers `Busy` to the rest, and applies the parked one on the RECOVERED
//! root — which is what this checks, against a node that already holds data.

use protocol::{Op, Reply, Request, WriteState};
use testkit::page_node::PageNode;

fn told(r: &[Vec<u8>], id: u64) -> Vec<WriteState> {
    r.iter()
        .filter_map(|b| protocol::decode_reply(b).ok())
        .filter_map(|x| match x {
            Reply::WriteState { write_id, state } | Reply::SessionWriteState { write_id, state, .. } if write_id == id => Some(state),
            _ => None,
        })
        .collect()
}

/// Every key the node's head names, read by a tab that starts fresh.
fn rows_at_the_head(node: &PageNode) -> Vec<Vec<u8>> {
    let (_, root) = node.head().expect("the node has a head");
    node.tree(&root).expect("the head's tree is readable").keys().cloned().collect()
}

#[test]
fn a_write_made_before_the_head_is_read_lands_on_the_head_and_loses_nothing() {
    let node = PageNode::new();
    // A tab that DID read its head writes the data this person already has.
    let mut first = node.connect();
    first.client(&Request::Identity);
    let w = first.client(&Request::forced_write(1, vec![Op::Put(b"k/old".to_vec(), vec![7u8; 2000])]));
    assert!(told(&w, 1).contains(&WriteState::Published), "the first write: {:?}", told(&w, 1));
    let before = rows_at_the_head(&node);
    assert_eq!(before, vec![b"k/old".to_vec()], "THE CONTROL: the node holds the old data");

    // A NEW TAB writes at once — no Identity, so its engine has not read the
    // head and stands on the empty tree.
    let mut fresh = node.connect();
    let early = fresh.client(&Request::forced_write(1, vec![Op::Put(b"k/new".to_vec(), vec![8u8; 2000])]));
    assert!(
        !told(&early, 1).contains(&WriteState::Published),
        "a write published before the head was read: {:?}",
        told(&early, 1)
    );
    assert_eq!(
        rows_at_the_head(&node),
        before,
        "a write made before the head was read PUBLISHED over the person's head"
    );

    // Now it reads its head. The parked write is applied on the recovered
    // root, so both keys are at the head.
    fresh.client(&Request::Identity);
    let mut rows = rows_at_the_head(&node);
    rows.sort();
    assert_eq!(
        rows,
        vec![b"k/new".to_vec(), b"k/old".to_vec()],
        "after the head was read, the parked write did not land on it"
    );
}
