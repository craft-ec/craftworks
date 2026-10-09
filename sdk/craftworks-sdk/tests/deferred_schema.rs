//! OPENING A PUBLISHED APP COMMITS NOTHING (sdk#350; the owner: viewing writes nothing, on any node).
//!
//! An app defines its domains on open. On a published app's own tree (an ASKED session: the SDK sets
//! `Db::set_defer_schema` there, and only there) those defines are DEFERRED: the engine takes them and commits
//! them only in the company of a data write. So opening commits nothing, the person's first write commits ONE
//! head carrying the defines and the row, and a held define is never "unsaved" -- a viewer's close must not say
//! "you have unsaved changes". A builder's Db never defers: a publisher's schema edit commits as it always did.
//! (The engine's own rule -- the cut, go-back-N, a refused data write -- is engine/tests', sdk#358.)

use craftworks_sdk::{Db, PageStore, SystemEnv};
use testkit::{PageConn, PageNode};

fn schema() -> craftworks_sdk::Schema {
    serde_json::from_value(serde_json::json!({ "type": "Note", "fields": [{"name": "body", "kind": "text"}] })).expect("a schema")
}

fn fields(v: &str) -> serde_json::Map<String, serde_json::Value> {
    let mut m = serde_json::Map::new();
    m.insert("body".into(), serde_json::Value::String(v.into()));
    m
}

fn engine_db(node: &PageNode) -> (Db<PageStore<PageConn>, SystemEnv>, PageConn) {
    let (s, conn, _clock) = testkit::page_store(node);
    (Db::new(s, SystemEnv, [0u8; 4]), conn)
}

/// Let the node answer everything it holds, and the store take the answers.
fn settle(d: &mut Db<PageStore<PageConn>, SystemEnv>, conn: &mut PageConn) {
    for _ in 0..10_000 {
        d.store_mut().sync();
        if conn.held() == 0 {
            break;
        }
        let _ = conn.release_one();
    }
    d.store_mut().sync();
}

/// **An asked app's defines commit NOTHING and are not unsaved; the first data write commits ONE head with
/// both.** Mutant "the SDK does not mark a define deferred" -> red: the defines commit on open.
#[test]
fn opening_an_app_commits_nothing_and_the_first_write_commits_the_defines_with_it() {
    let node = PageNode::new();
    let (mut d, mut conn) = engine_db(&node);
    d.set_defer_schema(true);
    d.define("note", &schema()).expect("define");
    d.define("guest", &schema()).expect("define");
    settle(&mut d, &mut conn);
    assert_eq!(node.head(), None, "opening the app committed its defines: the owner's rule is that viewing writes nothing");
    assert_eq!(d.store().unsaved_writes(), 0, "a held define counted as unsaved: a viewer's close would say 'unsaved changes'");
    let (queued, _) = d.store().queue_load();
    assert_eq!(queued, 2, "THE CONTROL: the engine does not hold the two defines ({queued})");

    conn.hold_answers();
    d.put("guest", &fields("hello")).expect("the first data write");
    assert!(d.store().unsaved_writes() >= 1, "the person's own write is not unsaved before it is published");
    conn.stop_holding();
    settle(&mut d, &mut conn);
    let (seq, root) = node.head().expect("the first data write published nothing");
    assert_eq!(seq, 1, "the defines and the write took {seq} commits, not ONE");
    let tree = node.tree(&root).expect("the published tree");
    assert_eq!(tree.len(), 3, "the one commit does not carry both defines and the row: {} keys", tree.len());
    assert_eq!(d.store().unsaved_writes(), 0, "writes still unsaved after they were published");
}

/// **A builder's define commits on its own, as it always did** (its Db never defers). Also the control for the
/// test above: the same define, not deferred, DOES commit on the same node.
#[test]
fn a_builders_define_still_commits_alone() {
    let node = PageNode::new();
    let (mut d, mut conn) = engine_db(&node);
    d.define("note", &schema()).expect("define");
    settle(&mut d, &mut conn);
    let (seq, root) = node.head().expect("a builder's define did not commit");
    assert_eq!(seq, 1);
    assert_eq!(node.tree(&root).expect("the tree").len(), 1, "the define is not in the head");
}
