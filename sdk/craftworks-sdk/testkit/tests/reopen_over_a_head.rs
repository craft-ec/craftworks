//! A tab that OPENS over a head another tab published (a reload, a second
//! tab): it has none of the tree's blocks, so its first walk stops, the
//! engine reads the range at that walk's root — before the head is recovered,
//! at the head once it is — and the resumed walk answers the whole tree. The
//! native half of the notes acceptance's "after a reload, all 50 are back".

use craftworks_sdk::store::{Edit, Reads, Store};
use craftworks_sdk::{Ended, StoreError};

#[test]
fn a_tab_opening_over_a_published_head_reads_all_of_it() {
    let node = testkit::PageNode::new();
    let (mut a, conn_a, _k) = testkit::page_store(&node);
    for i in 0..50u8 {
        a.apply_batch(&[(vec![b'n', i], Edit::Put(vec![i; 40]))]).expect("taken");
    }
    for t in 1..20 {
        conn_a.clone().tick_at(t * 1000);
        a.sync();
    }
    assert_eq!(node.head().map(|h| h.0), Some(50), "tab a did not publish its 50 writes");

    let (mut b, conn_b, _k2) = testkit::page_store(&node);
    let mut hops = 0;
    let rows = loop {
        hops += 1;
        assert!(hops <= 8, "the read took more than eight hops");
        let r = b.scan(b"n", b"o", false, usize::MAX);
        let ticket = b.take_ticket();
        b.unpin();
        match r {
            Ok(rows) => break rows,
            Err(StoreError::NotLoaded) => {}
            Err(e) => panic!("{e}"),
        }
        let t = ticket.expect("a refused read has a ticket to wait on");
        let mut how = None;
        for i in 0..50 {
            how = b.take_ended().into_iter().find(|(id, _)| *id == t).map(|(_, h)| h);
            if how.is_some() {
                break;
            }
            conn_b.clone().tick_at(100_000 + i * 100);
        }
        assert_eq!(how, Some(Ended::Loaded), "the ticket did not end LOADED");
        b.resume(t);
    };
    assert_eq!(rows.len(), 50, "the reopened tab did not read all of the head");
    assert!(hops >= 2, "the first walk answered with none of the blocks held: the case this test is about did not happen");
    println!("  50 rows in {hops} hops");
}
