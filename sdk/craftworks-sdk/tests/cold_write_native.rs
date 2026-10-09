//! A COLD WRITE, against a REAL engine.
//!
//! The JavaScript test for this drives a FAKE session. It is worth having —
//! it is the path an app takes — but it exercises the retry LOOP and never
//! the parking underneath it, which is the half that decides whether there is
//! anything to retry.
//!
//! Measured, in review: with the recovery inline in `web/src/session.rs`, the
//! sdk#89 defect could be put straight back — `define` returning a ticketless
//! `NotLoaded` — and the whole suite stayed green. The defect that cost five
//! acceptance runs was reintroducible with nothing to show for it.
//!
//! So the decision lives in the SDK (`PageStore::decide`, READ-STATE), where
//! this drives it against the page path's engine and node.
//!
//! What a cold write IS: somebody's tree is on the node, this page has read
//! none of its blocks, and the very first thing it does is `define` a domain.
//! That define must read the existing schema before it can check the new one
//! against it, and the walk to it stops on blocks this page does not hold.

use craftworks_sdk::{Outcome, SystemEnv};
use testkit::page_node::{PageNode, Served};

mod support;
type Tab = support::page_tab::Tab<SystemEnv>;

fn schema() -> craftworks_sdk::Schema {
    serde_json::from_str(r#"{"type":"Task","fields":[{"name":"title","kind":"text"}]}"#).expect("a schema this build can read")
}

/// A node holding a PUBLISHED tree deep enough that a walk needs more than
/// its root, and a FRESH page on it that has read none of it.
fn cold() -> (PageNode, Tab) {
    let node = PageNode::new();
    let mut publisher = Tab::open(&node, SystemEnv, [1u8; 4]);
    let notes: craftworks_sdk::Schema = serde_json::from_str(r#"{"type":"Note","fields":[{"name":"body","kind":"text"}]}"#).expect("a schema");
    publisher.call(|d| d.define("notes", &notes)).expect("define");
    for i in 0..300 {
        let mut f = serde_json::Map::new();
        f.insert("body".into(), serde_json::json!(format!("note {i} {}", "x".repeat(64))));
        publisher.call(|d| d.put("notes", &f)).expect("put");
        if i % 20 == 0 {
            publisher.seconds(1);
        }
    }
    publisher.seconds(10);
    assert_eq!(publisher.db.store().unsaved_writes(), 0, "the publisher's tree did not publish");
    let page = Tab::open(&node, SystemEnv, [7u8; 4]);
    (node, page)
}

/// One attempt at a define, through the decision a session makes.
fn try_define(p: &mut Tab, domain: &str) -> Outcome<()> {
    let r = p.db.define(domain, &schema());
    p.db.store_mut().decide(r)
}

/// **THE DEFECT, AS A TEST.** A cold define parks, asks, and then APPLIES.
#[test]
fn a_cold_define_parks_asks_and_then_applies() {
    let (_node, mut p) = cold();
    let before = p.conn.served(Served::Get);

    // FIRST ATTEMPT: the walk to the existing schema stops on missing blocks.
    let ticket = match try_define(&mut p, "tasks") {
        Outcome::Wait(_, t) => t,
        Outcome::Told(e) => panic!(
            "a cold define was TOLD `{e:?}` with no ticket. That is sdk#89: nothing \
             can retry it, so the app keeps a form on screen, a button saying \
             Published, and refuses every write with `has no schema; define it first`"
        ),
        Outcome::Done(()) => panic!("a cold define succeeded with nothing read, so this page is not cold and the test measures a warm one"),
    };

    // AND IT ASKED: the ticket is an engine read that fetched blocks. A
    // ticket nothing fetches for waits for ever.
    p.pump();
    assert!(p.conn.served(Served::Get) > before, "the define parked without fetching the blocks it is waiting on");
    let ended = p.db.store_mut().take_ended();
    assert!(ended.iter().any(|(id, how)| *id == ticket && *how == craftworks_sdk::Ended::Loaded), "the ticket did not end LOADED: {ended:?}");

    // SECOND ATTEMPT, resumed on the ticket's root, as a page asks again —
    // through as many hops as the walk needs.
    p.db.store_mut().resume(ticket);
    p.call(|d| d.define("tasks", &schema())).expect("the define did not apply after its blocks arrived");

    // AND THE TREE HOLDS IT. "It stopped erroring" is not the claim.
    let got = p.call(|d| d.schema("tasks")).expect("the schema read is answerable now");
    assert_eq!(
        got.as_ref().map(|s| s.type_name.as_str()),
        Some(schema().type_name.as_str()),
        "the define reported success and the domain has no schema"
    );
}

/// The same for the writes that go through `need_schema`.
#[test]
fn a_cold_put_update_and_delete_park_too() {
    for what in ["put", "update", "delete"] {
        let (_node, mut p) = cold();
        let mut fields = serde_json::Map::new();
        fields.insert("title".into(), serde_json::json!("x"));
        let r = match what {
            "put" => p.db.put("tasks", &fields).map(|_| ()),
            "update" => p.db.update("tasks", [9u8; 16], &fields).map(|_| ()),
            _ => p.db.delete("tasks", [9u8; 16]).map(|_| ()),
        };
        match p.db.store_mut().decide(r) {
            Outcome::Wait(_, t) => assert!(t > 0, "{what}: parked with an unusable ticket"),
            Outcome::Told(e) => panic!(
                "a cold {what} was TOLD `{e:?}` with no ticket. It reads `need_schema` \
                 before it applies, so a cold one is 'I could not look', not 'invalid'"
            ),
            Outcome::Done(()) => panic!("{what}: succeeded on a page that had read nothing"),
        }
    }
}

/// **THE CONTROL.** An INVALID write is refused, with no ticket and no fetch.
#[test]
fn control_an_invalid_write_is_refused_and_asks_for_nothing() {
    let (_node, mut p) = cold();
    let before = p.conn.served(Served::Get);
    // A domain name no fetch can make legal.
    match try_define(&mut p, "") {
        Outcome::Told(_) => {}
        Outcome::Wait(e, t) => panic!(
            "an invalid define was parked on ticket {t} for `{e:?}`. No fetch makes a \
             bad domain name good, so this waits for something that cannot help"
        ),
        Outcome::Done(()) => panic!("an invalid define was accepted"),
    }
    p.pump();
    assert_eq!(p.conn.served(Served::Get), before, "it fetched blocks for a write no block can fix");
    assert_eq!(p.db.store().open_tickets(), 0, "it opened a ticket for a write no block can fix");
}
