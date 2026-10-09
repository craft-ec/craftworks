//! sdk#150 PR 3: a person's FIRST save on a fresh tree gets its redundancy.
//!
//! The architect's probe, as a test. Before PR 3, with a tick while the
//! first commit was in flight, its parity puts went out `after` the
//! empty tree's root -- which nobody puts or confirms -- were held, dropped
//! with the call, and (recorded as `sent`) never asked for again: not after
//! 130 ticks, a Flush, or a second write. Write 1 was told `Published` and
//! never `ParityComplete`. The CONTROL is the same first commit with no tick
//! in flight, which completed then and must still.

use protocol::{Op, Reply, Request, WriteState};
use testkit::page_node::{PageConn, PageNode};

fn value(i: u64, size: usize) -> Vec<u8> {
    let mut v = vec![0u8; size];
    v[..8].copy_from_slice(&i.to_le_bytes());
    v
}

fn told(r: &[Vec<u8>], id: u64) -> Vec<WriteState> {
    r.iter()
        .filter_map(|b| protocol::decode_reply(b).ok())
        .filter_map(|x| match x {
            Reply::WriteState { write_id, state } if write_id == id => Some(state),
            _ => None,
        })
        .collect()
}

fn drain(c: &mut PageConn, out: &mut Vec<Vec<u8>>) {
    while c.held() > 0 {
        out.extend(c.release_one());
    }
}

/// (what write 1 was told, how many parity ids the published root lists and
/// how many of them the node HOLDS, whether the page is still waiting on an op
/// once everything has settled)
fn first_commit(ticks_in_flight: bool) -> (Vec<WriteState>, (usize, usize), bool) {
    let node = PageNode::new();
    let mut c = node.connect();
    c.client(&Request::Identity);
    c.hold_answers();
    let mut all = c.client(&Request::forced_write(1, vec![
            Op::Put(b"k/big".to_vec(), value(7, 30 * 1024)),
            Op::Put(b"k/small".to_vec(), b"small".to_vec()),
        ]));
    let base = 1_790_000_000_000u64;
    let mut k = 1u64;
    while c.held() > 0 && k < 40 {
        if ticks_in_flight {
            all.extend(c.tick_at(base + k * 1000));
        }
        all.extend(c.release_one());
        k += 1;
    }
    for j in 0..10u64 {
        all.extend(c.tick_at(base + (k + j) * 1000));
        drain(&mut c, &mut all);
    }
    // The redundancy IS on the node: every parity id the published root lists
    // is held. (Since race put, COMMIT-LIFE §P, the parity goes out in the
    // commit's own round, so "PUTs after the publish" is 0 by design; what
    // was promised is that the parity exists, and this asks the node.)
    let (_, root) = node.head().expect("published");
    let listed: Vec<_> = {
        let bytes = freenet_prolly::store::Blocks::get(&node, &root).expect("the root is held").to_vec();
        freenet_prolly::node::Node::parse(&bytes).expect("a node").parity().collect()
    };
    let held = listed.iter().filter(|p| node.holds(p)).count();
    (told(&all, 1), (listed.len(), held), c.with_server(|s| s.page.waiting()))
}

#[test]
fn the_first_commit_gets_its_parity_with_or_without_a_tick_in_flight() {
    for ticks in [true, false] {
        let (st, (listed, held), waiting) = first_commit(ticks);
        println!("  ticks in flight: {ticks:<5}  parity listed {listed}, held {held}  still waiting {waiting}  told {st:?}");
        assert!(
            st.contains(&WriteState::Published),
            "ticks={ticks}: never published: {st:?}"
        );
        // NOTHING STRANDED. On the Shell: no delegate call ended with an
        // effect it had not sent (`max_stranded == 0`). A page has no call to
        // end, so the same claim is that once everything has settled the page
        // is waiting on NO op — a put or a sign it made and never had answered.
        assert!(!waiting, "ticks={ticks}: the page is still waiting on an op it made");
        assert!(listed > 0, "ticks={ticks}: the published root lists no parity: the test is empty");
        assert_eq!(held, listed, "ticks={ticks}: the node holds {held} of the {listed} parity blocks the published root lists");
        assert!(
            st.contains(&WriteState::ParityComplete),
            "ticks={ticks}: published and never ParityComplete: {st:?}"
        );
    }
}
