//! sdk#187 review: bounding parity WAITERS must not cost an ordinary burst
//! its ParityComplete. 200 one-row writes back to back (the client sends the
//! next as soon as one publishes, ticking a second at a time), then five
//! quiet minutes. On main every one of them hears ParityComplete; with the
//! waiter cap at 256 full group ids, 112 of 200 never did. Named by a group's
//! first id (32 B) the cap holds 1024 and the caps still sum under the bound.
use protocol::{Op, Reply, Request, WriteState};
use testkit::page_node::PageNode;

const T0: u64 = 1_790_000_000_000;

fn states(r: &[Vec<u8>], id: u64) -> Vec<WriteState> {
    r.iter()
        .filter_map(|b| protocol::decode_reply(b).ok())
        .filter_map(|x| match x {
            Reply::WriteState { write_id, state } if write_id == id => Some(state),
            _ => None,
        })
        .collect()
}

#[test]
fn two_hundred_one_row_writes_back_to_back_all_hear_parity_complete() {
    let node = PageNode::new();
    let mut c = node.connect();
    c.client(&Request::Identity);
    let mut all: Vec<Vec<u8>> = Vec::new();
    let mut tick = 0u64;
    let writes = 200u64;
    for w in 0..writes {
        let mut v = vec![0u8; 1000];
        v[..8].copy_from_slice(&w.to_le_bytes());
        let ops = vec![Op::Put(format!("k/{w:07}").into_bytes(), v)];
        let mut r = c.client(&Request::forced_write(100 + w, ops.clone()));
        for _ in 0..20 {
            if states(&r, 100 + w).contains(&WriteState::Published) {
                break;
            }
            if states(&r, 100 + w).last() == Some(&WriteState::Busy) {
                r = c.client(&Request::forced_write(100 + w, ops.clone()));
            }
            tick += 1;
            r.extend(c.tick_at(T0 + tick * 1000));
        }
        assert!(
            states(&r, 100 + w).contains(&WriteState::Published),
            "write {w} never published"
        );
        all.extend(r);
    }
    for _ in 0..300 {
        tick += 1;
        all.extend(c.tick_at(T0 + tick * 1000));
    }
    let complete = (0..writes)
        .filter(|w| states(&all, 100 + w).contains(&WriteState::ParityComplete))
        .count();
    // (The Shell also printed what its engine SHED to keep the delegate's
    // context saveable. A page has no context to keep, so nothing is shed.)
    println!("  {complete} of {writes} one-row writes heard ParityComplete");
    assert_eq!(
        complete as u64,
        writes,
        "writes never told ParityComplete: {}",
        writes - complete as u64
    );
}
