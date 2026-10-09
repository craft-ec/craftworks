//! craftworks-sdk#160: a write that changes NOTHING, through the path an app
//! takes -- `Db` over the page's store over the real engine (`PageNode`).
//!
//! `Db::define` writes the schema unconditionally, and an app defines every
//! domain on every open. Traced here before the fix, LIVE, not latent: the
//! client does not drop an identical write -- it went on the wire, was told
//! `[Accepted]` and nothing more in 90 s, and the next write on another key
//! was told `Busy` six times as the outbox re-sent it.

use craftworks_sdk::store::{Edit, Store as _};
use craftworks_sdk::*;
use serde_json::json;
use testkit::page_node::PageNode;

mod support;
use support::page_tab::Tab;

struct At;
impl Env for At {
    fn now_ms(&mut self) -> u64 {
        1_750_000_000_000
    }
    fn rand32(&mut self) -> u32 {
        7
    }
}

fn schema() -> Schema {
    serde_json::from_value(json!({
        "type": "Task",
        "fields": [{ "name": "title", "kind": "text", "required": true }]
    }))
    .unwrap()
}

#[test]
fn defining_a_domain_twice_publishes_and_the_next_write_goes() {
    use protocol::WriteState as W;
    let node = PageNode::new();
    let mut p = Tab::open(&node, At, [1, 2, 3, 4]);
    let w1 = p.db.store().writes.next_write_id();
    p.db.define("tasks", &schema()).expect("the first define");
    p.pump();
    p.seconds(5);
    let puts_before = p.conn.served(testkit::page_node::Served::Put);

    let w2 = p.db.store().writes.next_write_id();
    p.db.define("tasks", &schema())
        .expect("the same define again");
    p.pump();
    p.seconds(90);
    let puts_for_w2 = p.conn.served(testkit::page_node::Served::Put) - puts_before;

    let w3 = p.db.store().writes.next_write_id();
    p.db.store_mut()
        .apply_batch(&[(b"d/other".to_vec(), Edit::Put(b"x".to_vec()))])
        .expect("the store took the write");
    p.pump();
    p.seconds(5);

    let told = |id: u64| p.told(id);
    assert_eq!(told(w1), vec![W::Accepted, W::Published, W::ParityComplete]);
    assert_eq!(
        p.sends(w2),
        Some(1),
        "the identical define is not on the wire once -- this test no longer traces the engine"
    );
    assert_eq!(
        told(w2),
        vec![W::Accepted, W::Published, W::ParityComplete],
        "the identical define was not told it is published"
    );
    assert_eq!(
        puts_for_w2, 0,
        "a write that changes nothing put {puts_for_w2} block(s)"
    );
    assert_eq!(
        told(w3).first(),
        Some(&W::Accepted),
        "the write after the no-op was not accepted: {:?}",
        told(w3)
    );
    assert!(told(w3).contains(&W::Published), "{:?}", told(w3));
    assert_eq!(p.sends(w3), Some(1), "the next write was re-sent");
}
