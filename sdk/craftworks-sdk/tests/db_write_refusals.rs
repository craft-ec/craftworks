//! A write the store REFUSED is answered as refused, never as made
//! (craftworks-sdk#180), and the refusal NAMES what the caller can do.
//!
//! Since R-b the only bound on writes waiting to be saved is the BYTES the
//! ENGINE's queue holds (COMMIT-LIFE K1; no count cap): past it the store
//! says `QUEUE_FULL` -- by name, with the bytes and the limit -- and the
//! SDK's `write()` WAITS on it (backpressure) and names it to the app only at
//! the app's deadline. Here, at the store, it is the answer the wait is made
//! of: never a generic failure, never a write answered `Created` that is not
//! there, and the same write made again once the queue drains is taken. (Before R-b the client's copy held the bound and
//! answered `NO_ROOM`; measured then: 300 `create_at` against its cap of 256
//! answered `Ok(Created)` 300 times while 45 had been dropped.)
//!
//! Driven over the real `Db` on the page's store over the page path's node,
//! the schema published, then the node's answers HELD -- as a real node's
//! arrive after a round trip -- so every write after it waits in the queue.

use craftworks_sdk::{CreateAt, DbError, Refused, Schema, SystemEnv};
use serde_json::{json, Map, Value};

mod support;
type Tab = support::page_tab::Tab<SystemEnv>;

/// A slot's `created` in the past, so no write is refused for its clock.
const PAST_MS: u64 = 1_700_000_000_000;

/// A tab whose page's queue bound is `params`, the schema PUBLISHED, then the
/// node silent: what it writes from here waits in the queue.
fn db_with(params: engine::Params) -> Tab {
    let node = testkit::PageNode::new();
    let mut t = Tab::open_with(&node, params, SystemEnv, [1, 2, 3, 4]);
    let sc: Schema = serde_json::from_value(json!({ "type": "Row", "fields": [{ "name": "n", "kind": "int" }] })).unwrap();
    t.call(|d| d.define("rows", &sc)).expect("the schema is written");
    t.seconds(3);
    assert_eq!(t.db.store().unsaved_writes(), 0, "the schema did not publish");
    t.conn.hold_answers();
    t
}

fn fields(n: i64) -> Map<String, Value> {
    match json!({ "n": n }) {
        Value::Object(m) => m,
        _ => unreachable!(),
    }
}

fn loc(r: &craftworks_sdk::Record) -> craftworks_sdk::id::Loc {
    craftworks_sdk::id::loc_from_hex(&r.id).expect("a record's id parses")
}

fn slot(i: u64) -> craftworks_sdk::RKey {
    craftworks_sdk::slot_from(PAST_MS, "p", &format!("r{i}"))
}

/// The node answers everything it held: the commit in flight lands, and the
/// writes behind it go.
fn node_answers(t: &mut Tab) {
    t.conn.stop_holding();
    while t.conn.held() > 0 {
        let _ = t.conn.release_one();
        t.pump();
    }
    t.seconds(3);
}

/// A small byte bound, so a burst meets it.
const BOUND: usize = 2_000;

fn small() -> engine::Params {
    engine::Params { max_queue_bytes: BOUND, ..engine::Params::default() }
}

/// **300 `create_at` past the queue's byte bound: every `Created` is really
/// there, and the rest are told `QUEUE_FULL` by name**, with the limit.
#[test]
fn create_at_past_the_queue_bound_is_told_queue_full_and_nothing_created_is_absent() {
    let mut t = db_with(small());
    let d = &mut t.db;
    let (mut created, mut full, mut absent) = (0usize, 0usize, Vec::new());
    for i in 0..300u64 {
        match d.create_at("rows", slot(i), &fields(i as i64)) {
            Ok(CreateAt::Created(r)) => {
                created += 1;
                // Created means TAKEN: readable at once, on the warm root.
                if d.get("rows", loc(&r)).expect("the range is loaded").is_none() {
                    absent.push(i);
                }
            }
            Ok(CreateAt::Exists(_)) => panic!("row {i}: nothing was there to exist"),
            Err(e) => {
                assert_eq!(e.code(), "QUEUE_FULL", "row {i}: {e}");
                assert!(e.is_retryable(), "QUEUE_FULL is retryable once the queue drains");
                assert_eq!(e.cap(), Some(BOUND), "row {i}: not told the limit");
                full += 1;
            }
        }
    }
    println!("  300 create_at, byte bound {BOUND}: created {created}, QUEUE_FULL {full}, created-but-absent {}", absent.len());
    assert!(absent.is_empty(), "answered Created, and not there: rows {absent:?}");
    assert!(created > 0 && full > 0, "the bound was not met, or nothing was taken: created {created}, full {full}");
    assert_eq!(created + full, 300);
}

/// `put` past the bound: told by name, and the queue did not take it.
#[test]
fn put_past_the_bound_is_told_queue_full() {
    let mut t = db_with(engine::Params { max_queue_bytes: 1, ..engine::Params::default() });
    let d = &mut t.db;
    d.put("rows", &fields(1)).expect("a session's first write is always taken");
    let e = d.put("rows", &fields(2)).expect_err("no room for a second");
    assert_eq!(e.code(), "QUEUE_FULL", "{e}");
    assert!(matches!(e, DbError::WriteRefused(Refused::QueueFull { limit: 1, bytes }) if bytes > 0), "{e:?}");
    assert_eq!(d.store().queue_load().0, 1, "the refused write entered the queue anyway");
}

/// `update` past the bound: refused, and the record keeps its old value.
#[test]
fn update_past_the_bound_is_refused_and_the_record_is_unchanged() {
    let mut t = db_with(engine::Params { max_queue_bytes: 1, ..engine::Params::default() });
    let d = &mut t.db;
    let r = d.put("rows", &fields(1)).expect("room");
    let e = d.update("rows", loc(&r), &fields(2)).expect_err("no room");
    assert_eq!(e.code(), "QUEUE_FULL", "{e}");
    let now = d.get("rows", loc(&r)).expect("loaded").expect("still there");
    assert_eq!(now.fields.get("n"), Some(&json!(1)), "the refused update shows on screen");
}

/// `delete` past the bound: refused, and the record is still there.
#[test]
fn delete_past_the_bound_is_refused_and_the_record_stays() {
    let mut t = db_with(engine::Params { max_queue_bytes: 1, ..engine::Params::default() });
    let d = &mut t.db;
    let r = d.put("rows", &fields(1)).expect("room");
    let e = d.delete("rows", loc(&r)).expect_err("no room");
    assert_eq!(e.code(), "QUEUE_FULL", "{e}");
    assert!(d.get("rows", loc(&r)).expect("loaded").is_some(), "the refused delete removed it");
}

/// A page with NO SESSION makes no writes — and says so, by its own code.
#[test]
fn a_page_with_no_session_is_refused_no_session() {
    let node = testkit::PageNode::new();
    let mut t = Tab::open(&node, SystemEnv, [1, 2, 3, 4]);
    t.db.store_mut().writes.client = craftworks_sdk::Client::from_random(None);
    let d = &mut t.db;
    let sc: Schema = serde_json::from_value(json!({ "type": "Row", "fields": [{ "name": "n", "kind": "int" }] })).unwrap();
    let e = d.define("rows", &sc).expect_err("no session, no write");
    assert_eq!(e.code(), "NO_SESSION", "{e}");
    assert!(!e.is_retryable(), "waiting does not mint a session; a reload does");
    assert!(matches!(e, DbError::WriteRefused(Refused::NoSession)));
}

/// **Once the queue drains, the SAME write made again succeeds** — the
/// recovery `QUEUE_FULL` names.
#[test]
fn a_write_refused_queue_full_succeeds_once_the_queue_drains() {
    let mut t = db_with(small());
    let d = &mut t.db;
    let mut i = 0u64;
    let e = loop {
        match d.create_at("rows", slot(i), &fields(i as i64)) {
            Ok(_) => i += 1,
            Err(e) => break e,
        }
        assert!(i < 1000, "the queue never filled");
    };
    assert_eq!(e.code(), "QUEUE_FULL");
    node_answers(&mut t);
    let d = &mut t.db;
    match d.create_at("rows", slot(i), &fields(i as i64)) {
        Ok(CreateAt::Created(_)) => {}
        other => panic!("the retry after the queue drained did not create: {other:?}"),
    }
}
