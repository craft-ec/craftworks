//! A read wider than one request, on the page path (re-stated from the
//! Shell fixture's `chain_bound.rs`, deleted with it).
//!
//! The engine caps the GETs one request may cause (`max_gets_per_request`),
//! so a COLD read wider than that answers a SHORT page and a cursor, and the
//! cursor reads the rest — every row exactly once. A WARM read is one page of
//! 256 rows and costs no GET at all.

use protocol::{Reply, Request};
use testkit::page_node::Served;
use testkit::{PageConn, PageNode};

/// A node holding `n` rows, written through a tab.
fn written(n: usize) -> PageNode {
    let node = PageNode::new();
    let mut c = node.connect();
    c.client(&Request::Identity);
    let ops = (0..n).map(|i| protocol::Op::Put(format!("r/{i:05}").into_bytes(), vec![7u8; 40])).collect();
    c.client(&Request::forced_write(1, ops));
    assert!(node.head().is_some(), "the rows never published");
    node
}

fn read(req_id: u64, after: Option<Vec<u8>>) -> Request {
    Request::Range { req_id, lo: protocol::Bound::Unbounded, hi: protocol::Bound::Unbounded, reverse: false, after, max_entries: protocol::MAX_PAGE_ENTRIES }
}

/// The keys and cursor of request `req_id`'s page, and the GETs it caused: `(on the wire, WANTED, RACED)`. The cap
/// counts the WANTED ones -- a block's race (sdk#303: its group asked at once) rides inside its one fetch.
/// One request's page: its keys, its cursor, and its GETs `(on the wire, wanted, raced)`.
type Page = (Vec<Vec<u8>>, Option<Vec<u8>>, (usize, usize, usize));

fn page(c: &mut PageConn, req_id: u64, after: Option<Vec<u8>>) -> Page {
    let before = c.served(Served::Get);
    let counts = |c: &PageConn| c.with_server(|s| s.page.fetch_counts());
    let (w0, r0) = counts(c);
    let out = c.client(&read(req_id, after));
    let (w1, r1) = counts(c);
    let gets = (c.served(Served::Get) - before, w1 - w0, r1 - r0);
    let (keys, cursor) = out
        .iter()
        .filter_map(|f| protocol::decode_reply(f).ok())
        .find_map(|r| match r {
            Reply::Page { req_id: id, entries, cursor, .. } if id == req_id => Some((entries.into_iter().map(|(k, _)| k).collect(), cursor)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("request {req_id} was never answered"));
    (keys, cursor, gets)
}

#[test]
fn a_cold_read_wider_than_one_request_answers_a_short_page_and_its_cursor_reads_the_rest() {
    // A cap the read must meet: 300 rows are a handful of blocks, far under
    // the default 64, so the tab runs with a cap of 2 and the read is wider
    // than one request by construction.
    let params = engine::Params { max_gets_per_request: 2, ..engine::Params::default() };
    let cap = params.max_gets_per_request as usize;
    let cold = PageNode::cold_over(&written(300));
    let mut c = cold.connect_with(params);
    c.client(&Request::Identity);
    let (mut rows, mut after, mut per_request, mut wire) = (Vec::new(), None, Vec::new(), Vec::new());
    for req in 1..=20u64 {
        let (keys, cursor, (on_wire, wanted, raced)) = page(&mut c, req, after.clone());
        per_request.push(wanted);
        wire.push((on_wire, raced));
        rows.extend(keys);
        if cursor.is_none() {
            break;
        }
        after = cursor;
    }
    println!("  cold read: WANTED GETs per request {per_request:?}; (on the wire, raced) {wire:?}; {} rows", rows.len());
    assert_eq!(per_request[0], cap, "the first request's chain was not cut at exactly the cap");
    assert!(per_request.iter().all(|g| *g <= cap), "a request caused more than {cap} GETs: {per_request:?}");
    assert!(per_request.len() >= 2, "the read fitted one request: the test is empty");
    let distinct: std::collections::BTreeSet<&Vec<u8>> = rows.iter().collect();
    assert_eq!((rows.len(), distinct.len()), (300, 300), "not every row exactly once");
}

#[test]
fn control_a_warm_read_is_one_page_per_256_rows() {
    // WARM: the tab that wrote the rows holds their blocks in its own memory.
    let node = PageNode::new();
    let mut c = node.connect();
    c.client(&Request::Identity);
    let ops = (0..300).map(|i| protocol::Op::Put(format!("r/{i:05}").into_bytes(), vec![7u8; 40])).collect();
    c.client(&Request::forced_write(1, ops));
    let (keys, cursor, (gets, _, _)) = page(&mut c, 1, None);
    assert_eq!((keys.len(), gets), (256, 0));
    assert!(cursor.is_some());
}
