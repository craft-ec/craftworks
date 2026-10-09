//! THE PROTOCOL'S RULES, ON THE PAGE (re-stated from the Shell's
//! `engine-delegate/tests/shell.rs`, deleted with the Shell).
//!
//! `page::Server` answers the same protocol the Shell did, so the rules the
//! Shell's tests pinned are pinned here on it: a message is read exactly or
//! not at all, what cannot be understood is refused by name, Identity says
//! whether the head can be written and which contract it is, and a subscribed
//! range is pushed a `Changed` for a commit inside it — and nothing for one
//! outside it.

use engine::Params;
use page::server::{Server, SignerFacts};
use page::{Ms, Page, PutPath};
use protocol::{Reply, Request};
use testkit::PageNode;

fn decode(frames: &[Vec<u8>]) -> Vec<Reply> {
    frames.iter().filter_map(|f| protocol::decode_reply(f).ok()).collect()
}

fn session_frame(r: &Request) -> Vec<u8> {
    protocol::encode_session_request(protocol::CURRENT, protocol::mint_session(0x5e55_1017), r).expect("encodes")
}

fn write_k() -> Request {
    Request::forced_write(1, vec![protocol::Op::Put(b"k".to_vec(), b"v".to_vec())])
}

/// A request with TRAILING bytes is refused, never read as its prefix — which
/// is how one message becomes another — and the page applies nothing for it.
#[test]
fn a_message_with_trailing_bytes_is_refused_rather_than_half_read() {
    let good = session_frame(&write_k());
    assert!(matches!(protocol::decode_request(&good), protocol::Incoming::Ok(_)), "a well-formed write was refused, so the check below shows nothing");
    let mut trailing = good.clone();
    trailing.extend_from_slice(b"and then some");
    assert!(!matches!(protocol::decode_request(&trailing), protocol::Incoming::Ok(_)), "a request with trailing bytes was accepted");

    let node = PageNode::new();
    let mut c = node.connect();
    c.client(&Request::Identity);
    let out = decode(&c.frame(&trailing));
    assert!(out.iter().any(|r| matches!(r, Reply::Dropped { .. })), "the page did not answer the trailing-bytes frame Dropped: {out:?}");
    assert!(node.head().is_none(), "the prefix of a refused frame was applied");
    // The control: the same write, well-formed, publishes.
    let ok = decode(&c.frame(&good));
    assert!(ok.iter().any(|r| matches!(r, Reply::SessionWriteState { state: protocol::WriteState::Published, .. })), "{ok:?}");
}

/// What the page cannot understand is REFUSED BY NAME and causes no node
/// operation: `Dropped` for bytes with no version, `Unsupported` for bytes
/// whose first two spell one this build does not serve (the version is read
/// first, by design). Never silence, never served. The control: a
/// well-formed request on the same page is served.
#[test]
fn what_the_page_cannot_understand_is_refused_by_name() {
    let node = PageNode::new();
    let mut c = node.connect();
    c.client(&Request::Identity);
    let before = c.served(testkit::page_node::Served::Put);
    let mut refused = Vec::new();
    for junk in [vec![], vec![0xFF; 64], b"not bincode at all".to_vec()] {
        let out = decode(&c.frame(&junk));
        assert_eq!(out.len(), 1, "one refusal per unusable message: {out:?}");
        refused.extend(out.into_iter().map(|r| matches!(r, Reply::Dropped { reason: protocol::Dropped::Unparseable }) || matches!(r, Reply::Unsupported { .. })));
    }
    assert_eq!(refused, [true, true, true], "an unusable message was not refused by name");
    assert_eq!(c.served(testkit::page_node::Served::Put), before, "garbage produced a node operation");
    let out = decode(&c.client(&Request::Get { req_id: 1, key: b"k".to_vec() }));
    assert!(out.iter().any(|r| matches!(r, Reply::Value { req_id: 1, .. })), "a well-formed request was not served: {out:?}");
}

/// Identity says whether a head can be WRITTEN, both ways, and NAMES the head
/// contract — from what the Session learnt of its signer (`SignerFacts`).
#[test]
fn identity_reports_whether_a_head_can_be_written_and_names_its_contract() {
    let identity = |facts: SignerFacts| {
        let mut s = Server::new(Page::unstarted(Params::default(), PutPath::Page, Ms(0)), facts);
        s.client(&protocol::encode_request(1, &Request::Identity).expect("encodes"));
        decode(&s.take_replies())
            .into_iter()
            .find_map(|r| match r {
                Reply::Identity { head_writable, head_id, .. } => Some((head_writable, head_id)),
                _ => None,
            })
            .expect("Identity answers")
    };
    assert_eq!(identity(SignerFacts { head_writable: true, head_id: [4; 32] }), (true, [4; 32]), "a provisioned page misreported its head");
    assert_eq!(
        identity(SignerFacts { head_writable: false, head_id: [0; 32] }),
        (false, [0; 32]),
        "an UNPROVISIONED page reported it can write a head; a page believing this never provisions and every write is dropped"
    );
}

/// A SUBSCRIBED range is answered `Subscribed`, and pushed a `Changed` for a
/// commit inside it; a commit OUTSIDE it pushes nothing.
#[test]
fn a_subscribed_range_is_pushed_a_changed_and_only_for_its_own_keys() {
    let run = |lo: &[u8]| {
        let node = PageNode::new();
        let mut c = node.connect();
        c.client(&Request::Identity);
        let sub = decode(&c.frame(&session_frame(&Request::SubscribeRange { sub_id: 7, lo: protocol::Bound::Included(lo.to_vec()), hi: protocol::Bound::Unbounded })));
        let accepted: Vec<_> = sub.iter().filter_map(|r| match r {
            Reply::Subscribed { accepted, .. } => Some(*accepted),
            _ => None,
        }).collect();
        assert_eq!(accepted, [protocol::Accepted::Yes], "the subscribe was not answered; a client not told cannot tell 'subscribed' from 'lost'");
        let out = decode(&c.frame(&session_frame(&write_k())));
        assert!(out.iter().any(|r| matches!(r, Reply::SessionWriteState { state: protocol::WriteState::Published, .. })), "the write did not publish: {out:?}");
        out.iter().filter(|r| matches!(r, Reply::Changed { .. })).count()
    };
    assert!(run(b"k") > 0, "a commit inside the subscribed range pushed no Changed");
    assert_eq!(run(b"zzz"), 0, "a commit outside the subscribed range pushed a Changed");
}
