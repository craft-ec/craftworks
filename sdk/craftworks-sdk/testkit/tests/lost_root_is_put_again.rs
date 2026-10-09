//! A COMMIT'S ROOT THE NODE ACKED AND LOST IS PUT AGAIN (the live-repair writer stall, sdk#555's live step).
//!
//! The signer signs a head only over a root its node holds (`RootNotHeld`, retryable: signer/src/lib.rs). A node
//! that acked the root's PUT and then lost it answers that for ever, and the page, which took the ack, re-asked the
//! SIGN on its backoff and never PUT the root again: the write sat `Accepted`, the head never moved, and between asks
//! nothing waited in the interactive lane, so `not_answering()` said nothing (live-repair: "write 3 not within 300 s:
//! not answering None", its root dropped by ws-drop in both stalled runs). On `RootNotHeld` the page now PUTs the
//! root again from page memory (`Act::Reput`, the one re-PUT path).

use protocol::{Op, Reply, Request, WriteState};
use testkit::page_node::{PageConn, PageNode, Served};

const T0: u64 = 1_790_000_000_000;

fn write(n: u64) -> Request {
    Request::forced_write(n, (0..40u32).map(|i| Op::Put(format!("r/{n}/{i:03}").into_bytes(), vec![n as u8; 90])).collect())
}

fn told(all: &[Vec<u8>], id: u64) -> Vec<WriteState> {
    all.iter()
        .filter_map(|b| protocol::decode_reply(b).ok())
        .filter_map(|r| match r {
            Reply::WriteState { write_id, state } | Reply::SessionWriteState { write_id, state, .. } if write_id == id => Some(state),
            _ => None,
        })
        .collect()
}

/// The root a write's commit publishes, from a node that loses nothing (blocks are content-addressed, so the same
/// write builds the same root anywhere).
fn root_of_first_write() -> freenet_prolly::Cid {
    let node = PageNode::new();
    let mut c = node.connect();
    c.client(&Request::Identity);
    c.client(&write(1));
    node.head().expect("THE SETUP: the write published on a node that loses nothing").1
}

/// Write 1 on a node that loses its root's PUT `times` times, driven by `ticks` seconds of the page's clock.
fn run(times: u32, ticks: u64) -> (Vec<WriteState>, Option<u64>, PageConn) {
    let root = root_of_first_write();
    let node = PageNode::new();
    let mut c = node.connect();
    c.lose_puts_of(root, times);
    c.client(&Request::Identity);
    let mut all = c.client(&write(1));
    for t in 1..=ticks {
        all.extend(c.tick_at(T0 + t * 1_000));
    }
    assert!(c.lost_sends(&root) >= 1, "THE SETUP: the root's PUT was never lost");
    (told(&all, 1), node.head().map(|h| h.0), c)
}

/// Lost ONCE (a node that acked the root and lost it): the root is put again, and the write publishes.
/// Mutant "no Reput on RootNotHeld" -> the write stays `Accepted`/`Stalled`, the head never moves -> red.
#[test]
fn a_root_the_node_acked_and_lost_is_put_again_and_the_write_publishes() {
    let (states, head, c) = run(1, 30);
    assert!(states.contains(&WriteState::Published), "the write never published after its root was lost once: {states:?}");
    assert_eq!(head, Some(1), "the head never moved");
    assert!(c.served(Served::Sign) >= 2, "THE SETUP: the signer never refused the head (it signed at once)");
}

/// Lost EVERY time (a network that never keeps it): the write cannot publish, and it does not sit silent -- the root
/// is PUT again on the sign's own backoff, each refusal (rule 7: re-sent until answered; rule 8: no time ends it).
#[test]
fn a_root_the_node_never_keeps_is_put_again_on_every_refusal_never_silent() {
    let (states, head, c) = run(u32::MAX, 30);
    assert!(!states.iter().any(|s| s.terminal()), "a network that loses the root ended the write: {states:?}");
    assert_eq!(head, None, "a head was published over a root the node does not hold");
    let root = root_of_first_write();
    assert!(c.lost_sends(&root) >= 3, "the root was not put again on the refusals: {} PUT(s) of it", c.lost_sends(&root));
}
