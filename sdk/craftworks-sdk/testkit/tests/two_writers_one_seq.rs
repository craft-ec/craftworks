//! sdk#178, two writers: two engines on one node, each believing the tree is
//! empty, both commit seq 1. The Register takes the first and refuses the
//! second (an equal seq), answering with the head it holds -- the same seq
//! under ANOTHER root. The second writer must be told `Lost`, never
//! `Published` (the shell compared the seq only); it adopts the head that
//! won, and its re-send publishes at seq 2 with both writes readable.

use protocol::{Op, Reply, Request, WriteState};
use testkit::page_node::PageNode;

fn told(r: &[Vec<u8>], id: u64) -> Vec<WriteState> {
    r.iter()
        .filter_map(|b| protocol::decode_reply(b).ok())
        .filter_map(|x| match x {
            Reply::WriteState { write_id, state } if write_id == id => Some(state),
            _ => None,
        })
        .collect()
}

#[test]
fn a_second_writer_at_the_same_seq_is_lost_not_published_and_its_re_send_publishes() {
    let node = PageNode::new();
    let (mut a, mut b) = (node.connect(), node.connect());
    a.client(&Request::Identity);
    b.client(&Request::Identity); // both read NO head: each will commit seq 1
    let wa = a.client(&Request::forced_write(1, vec![Op::Put(b"k/a".to_vec(), vec![1u8; 3000])]));
    assert!(
        told(&wa, 1).contains(&WriteState::Published),
        "A: {:?}",
        told(&wa, 1)
    );
    let a_head = node.head().expect("A's head");
    let wb = b.client(&Request::forced_write(1, vec![Op::Put(b"k/b".to_vec(), vec![2u8; 3000])]));
    let st = told(&wb, 1);
    assert!(
        !st.contains(&WriteState::Published),
        "B was told Published at A's seq: {st:?}"
    );
    assert!(
        st.contains(&WriteState::Lost),
        "B was not told Lost: {st:?}"
    );
    assert_eq!(node.head(), Some(a_head), "the Register's head moved");
    // B re-sends, as its outbox does, on the head that won.
    let again = b.client(&Request::forced_write(2, vec![Op::Put(b"k/b".to_vec(), vec![2u8; 3000])]));
    assert!(
        told(&again, 2).contains(&WriteState::Published),
        "{:?}",
        told(&again, 2)
    );
    assert_eq!(node.head().map(|h| h.0), Some(2));
    for key in [b"k/a".as_slice(), b"k/b".as_slice()] {
        let r = b.client(&Request::Get {
            req_id: 9,
            key: key.to_vec(),
        });
        let got = r
            .iter()
            .filter_map(|x| protocol::decode_reply(x).ok())
            .any(|x| matches!(x, Reply::Value { value: Some(_), .. }));
        assert!(
            got,
            "{} is not readable after the re-send",
            String::from_utf8_lossy(key)
        );
    }
}
