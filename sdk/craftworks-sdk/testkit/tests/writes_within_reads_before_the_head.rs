//! W8 BEFORE THE HEAD (sdk#235; WRITE-PATH.md): a write that changes a key it
//! did not read is refused AT THE DOOR — before the pre-head park, which
//! answers `Accepted` at once (sdk#223) — so `Unread` can never follow
//! `Accepted`. Through the fixture: a tab that has not asked `Identity` has an
//! engine that has not read its head.

use protocol::{Expect, Op, Reply, Request, WriteState};
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

#[test]
fn before_the_head_an_unread_write_is_refused_at_the_door_and_never_parked() {
    let node = PageNode::new();
    let mut tab = node.connect(); // no Identity: its engine has not read the head
    // A v4 client with a session: the one that may be told `Unread` (the same
    // bundle, as `Conflicted`). A legacy client is told `Failed` — below.
    const SESSION: u64 = 0x5e55_1011;
    let early = tab.client_as(SESSION, protocol::CURRENT, &Request::Commit {
        write_id: 1,
        reads: vec![(b"k/a".to_vec(), Expect::Absent)],
        ops: vec![Op::Put(b"k/a".to_vec(), b"1".to_vec()), Op::Put(b"k/b".to_vec(), b"2".to_vec())],
    });
    let st = told(&early, 1);
    assert!(st.contains(&WriteState::Unread), "a write with an unread key before the head was not refused as Unread: {st:?}");
    assert!(!st.contains(&WriteState::Accepted), "it was Accepted (parked) before it was judged: {st:?}");
    // Reading the head releases NOTHING for it: it was never parked.
    let after = tab.client_as(SESSION, protocol::CURRENT, &Request::Identity);
    assert!(told(&after, 1).is_empty(), "the refused write was parked and released on the head: {:?}", told(&after, 1));
    assert!(node.head().is_none(), "something was published for a refused write");

    // OLD CLIENTS (architect's rule): a legacy client cannot decode `Unread`,
    // and dropping an unknown tag would end the write "may have been saved".
    // It is told `Failed`, which is true to it.
    let mut legacy = node.connect();
    let old = legacy.client(&Request::Commit {
        write_id: 1,
        reads: vec![(b"k/a".to_vec(), Expect::Absent)],
        ops: vec![Op::Put(b"k/a".to_vec(), b"1".to_vec()), Op::Put(b"k/b".to_vec(), b"2".to_vec())],
    });
    assert_eq!(told(&old, 1), vec![WriteState::Failed], "a legacy client was not told Failed for an Unread write");

    // THE CONTROL: the same shape WITH both reads, before the head, IS parked
    // — answered Accepted at once — so the refusal above is the door's.
    let mut other = node.connect();
    let parked = other.client(&Request::Commit {
        write_id: 1,
        reads: vec![(b"k/a".to_vec(), Expect::Absent), (b"k/b".to_vec(), Expect::Absent)],
        ops: vec![Op::Put(b"k/a".to_vec(), b"1".to_vec()), Op::Put(b"k/b".to_vec(), b"2".to_vec())],
    });
    assert!(told(&parked, 1).contains(&WriteState::Accepted), "THE CONTROL: a write that read its keys was not parked: {:?}", told(&parked, 1));
}
