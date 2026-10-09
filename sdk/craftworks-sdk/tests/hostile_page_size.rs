//! A caller's `max_entries` is a licence to spend the ENGINE's work, and a
//! future proof's (sdk#15).
//!
//! `verify_range`'s cost-to-refuse bound is `2 * height + max_entries` blocks,
//! and `max_entries` comes off the wire. So the number a caller sends decides
//! how much a client must hash to find out it was lied to — which is the
//! hostile-cost shape: the asker pays nothing and the checker pays everything.
//!
//! **What this file establishes, and what it does not.** Nothing in this
//! workspace calls `prove_range` or `verify_range` yet, so there is no proof
//! to clamp "before it is fetched" — there is nothing to fetch. What exists is
//! the page clamp, and these tests pin it against hostile inputs through the
//! REAL shell, so that the first proof-backed reader inherits a bound that is
//! already enforced and already tested rather than one written for it later.

use protocol::{Reply, Request, MAX_PAGE_ENTRIES};
use testkit::PageNode;

fn write(n: u64) -> Request {
    Request::forced_write(n, vec![protocol::Op::Put(
            format!("k/{n:06}").into_bytes(),
            vec![(n % 251) as u8; 64],
        )])
}

/// A node with enough rows that a page limit can actually bite.
fn node_with_rows(n: u64) -> (PageNode, testkit::PageConn) {
    let node = PageNode::new();
    let mut c = node.connect();
    c.client(&Request::Identity);
    for i in 1..=n {
        c.client(&write(i));
    }
    (node, c)
}

fn page_of(conn: &testkit::PageConn, req_id: u64) -> Option<(usize, u32)> {
    conn.replies().into_iter().find_map(|r| match r {
        Reply::Page {
            req_id: id,
            entries,
            max_entries,
            ..
        } if id == req_id => Some((entries.len(), max_entries)),
        _ => None,
    })
}

fn ask(conn: &mut testkit::PageConn, req_id: u64, max_entries: u32) {
    conn.client(&Request::Range {
        req_id,
        lo: protocol::Bound::Unbounded,
        hi: protocol::Bound::Unbounded,
        reverse: false,
        after: None,
        max_entries,
    });
}

/// THE BOUND: what a caller asks for is not what it gets.
#[test]
fn a_hostile_page_size_is_clamped_before_the_engine_reads_anything() {
    let (_node, mut c) = node_with_rows(300);

    // 60,000 entries is the issue's number: at `2 * height + max_entries`
    // blocks it licenses a ~60,000-block proof.
    for (req_id, asked) in [(1u64, 60_000u32), (2, u32::MAX), (3, 10_000)] {
        ask(&mut c, req_id, asked);
        let (rows, used) = page_of(&c, req_id).expect("a page came back");

        assert_eq!(
            used, MAX_PAGE_ENTRIES,
            "asked for {asked} and the engine reported using {used}: the clamp \
             is what stops a caller licensing work it does not pay for"
        );
        assert!(
            rows <= MAX_PAGE_ENTRIES as usize,
            "asked for {asked} and got {rows} rows"
        );
        assert!(
            (rows as u32) < asked,
            "the page was not actually smaller than the request, so this test \
             would pass against no clamp at all"
        );
    }
}

/// The clamp is REPORTED, not applied silently.
///
/// A caller that asked for a thousand rows and got a hundred, with nothing
/// saying so, reads the short page as the end of the range and stops.
#[test]
fn the_clamp_comes_back_with_the_page() {
    let (_node, mut c) = node_with_rows(300);
    ask(&mut c, 9, 60_000);
    let (rows, used) = page_of(&c, 9).expect("a page came back");

    assert_eq!(used, MAX_PAGE_ENTRIES);
    assert!(
        rows > 0,
        "a page of nothing would satisfy every bound above for the wrong \
         reason"
    );
}

/// `0` is not "no limit": it becomes one entry.
///
/// Pinned here because the two readers of this field used to disagree — the
/// shell clamping to 1 and the engine treating 0 as unlimited — and a caller
/// cannot be right about a value two readers interpret differently.
#[test]
fn zero_is_one_entry_not_unlimited() {
    let (_node, mut c) = node_with_rows(300);
    ask(&mut c, 11, 0);
    let (rows, used) = page_of(&c, 11).expect("a page came back");

    assert_eq!(used, 1, "0 must clamp UP to one, never to unlimited");
    assert!(rows <= 1);
}

/// A request already inside the bound is left alone.
///
/// Without this, a clamp that returned `MAX_PAGE_ENTRIES` for everything would
/// pass every test above.
#[test]
fn a_modest_request_is_not_clamped() {
    let (_node, mut c) = node_with_rows(300);
    ask(&mut c, 12, 8);
    let (rows, used) = page_of(&c, 12).expect("a page came back");

    assert_eq!(used, 8, "8 is inside the bound and must be honoured");
    assert!(rows <= 8);
}
