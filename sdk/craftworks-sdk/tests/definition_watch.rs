//! THE DEFINITION'S LIVE WATCH (§19 P3, `Session::bind_definition`): a watch key built from
//! `Db::definition_domain(app, which)` is told when that definition changes and at no other time. A draft edit tells
//! the draft's watch and not the published one's; a publish tells the published one's; an ordinary write of the same
//! app tells neither. With an app (a session's) and without one (the in-tab db's names). The diff is the tree's
//! (`LiveBindings::take_changed` over `Db::watch_range`), over the page path's store.

use core_types::name::SystemDomain;
use craftworks_sdk::definition::DefKey;
use craftworks_sdk::live_bindings::WatchKey;
use craftworks_sdk::{Db, LiveBindings, SystemEnv};
use serde_json::json;

type D = Db<craftworks_sdk::PageStore<testkit::PageConn>, SystemEnv>;

fn watches(app: Option<&str>) {
    let node = testkit::PageNode::new();
    let (store, _conn, _clock) = testkit::page_store(&node);
    let mut db: D = Db::new(store, SystemEnv, [0u8; 4]);
    let key = |w| WatchKey::of(craftworks_sdk::app::StoredName::of_tree(D::definition_domain(app, w)));
    let (draft, published) = (key(SystemDomain::Draft), key(SystemDomain::App));
    let mut live = LiveBindings::default();
    live.bind(draft.clone());
    live.bind(published.clone());
    let head = db.store_mut().head();
    live.rendered(&draft, head);
    live.rendered(&published, head);
    let mut changed = |db: &mut D| {
        let head = db.store_mut().head();
        let c = live.take_changed(db.store_mut(), head, D::watch_range);
        for k in &c {
            live.rendered(k, head);
        }
        c
    };
    let tag = app.unwrap_or("<no app>");
    assert_eq!(changed(&mut db), Vec::<WatchKey>::new(), "{tag}: nothing changed and a definition was told");

    db.draft_put(app, &DefKey::parse("meta").unwrap(), &json!({ "name": "Notes" })).unwrap();
    assert_eq!(changed(&mut db), vec![draft.clone()], "{tag}: a draft edit was not told to the draft's watch alone");

    db.publish_definition(app).unwrap();
    assert_eq!(changed(&mut db), vec![published.clone()], "{tag}: a publish was not told to the published definition's watch alone");

    // THE CONTROL: an ordinary write of the same app moves the head and tells neither.
    let rows = app.map_or("rows".to_string(), |a| format!("{a}.rows"));
    let schema: craftworks_sdk::Schema = serde_json::from_value(json!({ "type": "Row", "fields": [{ "name": "x", "kind": "text" }] })).unwrap();
    db.define(&rows, &schema).unwrap();
    db.put(&rows, &serde_json::from_value(json!({ "x": "1" })).unwrap()).unwrap();
    assert!(db.store_mut().head().is_some());
    assert_eq!(changed(&mut db), Vec::<WatchKey>::new(), "{tag}: an ordinary write told a definition's watch");
}

#[test]
fn a_definition_watch_is_told_of_its_own_definition_only_in_an_apps_session() {
    watches(Some("proj-a"));
}

#[test]
fn a_definition_watch_is_told_of_its_own_definition_only_with_no_app() {
    watches(None);
}
