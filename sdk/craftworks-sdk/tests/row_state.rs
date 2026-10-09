//! What a record's OWN write is doing.
//!
//! "Saving", "saved here but not yet on the network" and "on the network" are
//! three different facts, and somebody deciding whether it is safe to close
//! the tab needs the second told apart from the third. A row that showed one
//! spinner for all three would be the thing craftworks-builder#20 part 2
//! exists to prevent.

use craftworks_sdk::store::{RowState, Store as _};
use craftworks_sdk::{Db, PageStore, SystemEnv, TreeStore};
use testkit::{PageConn, PageNode};

fn schema() -> craftworks_sdk::Schema {
    serde_json::from_value(serde_json::json!({
        "type": "Note",
        "fields": [{"name": "body", "kind": "text"}],
    }))
    .expect("a schema")
}

fn fields(v: &str) -> serde_json::Map<String, serde_json::Value> {
    let mut m = serde_json::Map::new();
    m.insert("body".into(), serde_json::Value::String(v.into()));
    m
}

/// A `Db` over the page's store on a fresh node, and the tab it reads through.
fn engine_db(node: &PageNode) -> (Db<PageStore<PageConn>, SystemEnv>, PageConn) {
    let (s, conn, _clock) = testkit::page_store(node);
    (Db::new(s, SystemEnv, [0u8; 4]), conn)
}

/// A record just written is PENDING, and the same record once its write is
/// published is CLEAN.
///
/// **Both arms.** A single arm passes against a constant, and a constant is
/// exactly what this replaced.
#[test]
fn a_record_reports_its_own_write_both_before_and_after_it_lands() {
    let node = PageNode::new();
    let (mut d, mut conn) = engine_db(&node);
    d.define("note", &schema()).expect("define");

    // The node answers after a round trip, as a real one does: its answers
    // are held until the end, so the write is taken and not yet published.
    conn.hold_answers();
    let write_id = d.store().writes.next_write_id();
    let r = d.put("note", &fields("hello")).expect("put");
    // PENDING, not clean: it has been sent and nothing has confirmed it.
    // Closing the tab now loses it, and the row has to be able to say so.
    assert_eq!(
        r.state,
        RowState::Pending,
        "a record just written claims nothing is in flight"
    );
    assert!(
        !r.state.is_settled(),
        "an unacknowledged row claimed to be saved"
    );

    let id = craftworks_sdk::id::from_hex(&r.id).expect("an id");

    // QUEUED: a write behind the commit in flight (R-b: the engine's queue).
    // A distinct fact from pending -- "yours has not gone yet" rather than
    // "yours has gone and nothing has answered" -- and the row says which.
    let second = d.put("note", &fields("behind it")).expect("put");
    assert_eq!(second.state, RowState::Queued, "a write behind the commit in flight is reported as if it had gone");
    assert_eq!(
        d.get("note", id).expect("get").expect("there").state,
        RowState::Pending,
        "the write in flight stopped saying so"
    );

    // The node answers: the commit lands and the write is published.
    conn.stop_holding();
    while conn.held() > 0 {
        let _ = conn.release_one();
        d.store_mut().sync();
    }
    d.store_mut().sync();
    assert!(!d.store().is_pending_write(write_id), "the write never published");
    let after = d.get("note", id).expect("get").expect("there");
    // Saved: CLEAN, or BACKED_UP once no parity is owed (sdk#294).
    assert!(matches!(after.state, RowState::Clean | RowState::BackedUp), "{:?}", after.state);
    assert!(
        after.state.is_settled(),
        "a published row still says it is in flight"
    );
}

/// A write that was ROLLED BACK does not go back to reporting `Clean`.
///
/// The moment a write ENDS unpublished (R-b: a named fate the page pulled --
/// `Failed`, `Lost`, `Conflict`...) it stops being in the queue, so a row
/// asking afterwards would be told "saved" about a write that never landed.
/// That is the worst answer available: the key stays ROLLED_BACK until it is
/// written again.
#[test]
fn a_write_that_ended_unpublished_is_not_reported_as_saved() {
    for fate in [page::fates::Fate::Failed { why: engine::FailWhy::TreeDamaged, signer: None }, page::fates::Fate::Lost, page::fates::Fate::Conflict { key: b"d\0note\0x".to_vec(), current: None, after: None }] {
        let mut w = craftworks_sdk::Writes::new(Box::new(|| 0));
        let key = b"d\0note\0x".to_vec();
        let edit = [(key.clone(), craftworks_sdk::Edit::Put(b"v".to_vec()))];
        let id = w.make(&[(key.clone(), protocol::Expect::Absent)], &edit).expect("made");
        assert!(!w.rolled_back(&key), "{fate:?}: a write in flight already says rolled back");
        w.on_fate(id, fate.clone());
        assert!(w.rolled_back(&key), "{fate:?}: a write that ended unpublished went back to reporting 'saved'");
        assert!(w.take_state_changed().contains(&key), "{fate:?}: the row was not told its write ended");
        // Writing again replaces the failure with something in flight.
        w.make(&[(key.clone(), protocol::Expect::Absent)], &edit).expect("made again");
        assert!(!w.rolled_back(&key), "{fate:?}: a key written again still says rolled back");
    }
}

/// THE CONTROL: a write that PUBLISHED is never reported rolled back.
/// Without it, a store that marked every ended write rolled back would pass
/// the test above.
#[test]
fn control_a_published_write_is_never_reported_rolled_back() {
    let mut w = craftworks_sdk::Writes::new(Box::new(|| 0));
    let key = b"d\0note\0x".to_vec();
    let id = w.make(&[(key.clone(), protocol::Expect::Absent)], &[(key.clone(), craftworks_sdk::Edit::Put(b"v".to_vec()))]).expect("made");
    w.on_fate(id, page::fates::Fate::Published { seq: 1, backed_up: false });
    assert!(!w.rolled_back(&key), "a published write was reported rolled back");
    assert!(w.take_ended().is_empty(), "a published write was told as ended unpublished");
}

/// The in-memory store answers `Clean`, always, and that is the TRUTH.
///
/// A write there is applied the moment it is made, so there is never a write
/// in flight to report. Stated rather than left implicit, because a surface
/// that answered `Unknown` would make every row of an unpublished project
/// claim this client cannot see its own data.
#[test]
fn the_in_memory_store_answers_clean_because_it_has_nothing_in_flight() {
    let mut d: Db<TreeStore, SystemEnv> = Db::new(TreeStore::new(), SystemEnv, [0u8; 4]);
    d.define("note", &schema()).expect("define");
    let r = d.put("note", &fields("hello")).expect("put");
    assert_eq!(r.state, RowState::Clean);
    assert!(r.state.is_settled());
}

/// The codes are a fixed list, distinct, and every variant has one.
#[test]
fn every_row_state_has_a_distinct_stable_code() {
    const KNOWN: &[&str] = &["CLEAN", "QUEUED", "PENDING", "ROLLED_BACK", "BACKED_UP"];
    let all = [
        RowState::Clean,
        RowState::Queued,
        RowState::Pending,
        RowState::RolledBack,
        RowState::BackedUp,
    ];
    let mut codes: Vec<&str> = all.iter().map(|s| s.code()).collect();
    for c in &codes {
        assert!(KNOWN.contains(c), "{c} is not in the published list");
    }
    codes.sort_unstable();
    let n = codes.len();
    codes.dedup();
    assert_eq!(codes.len(), n, "two states share a code");

    // Only CLEAN and BACKED_UP may claim to be saved.
    for s in all {
        assert_eq!(
            s.is_settled(),
            matches!(s, RowState::Clean | RowState::BackedUp),
            "{s:?} settled wrongly"
        );
    }
}

/// A MARKER still in flight reads PENDING through `published_state`, never saved -- though `is_published` already
/// says it EXISTS (builder#88: a publish is not reported done over a marker the node has not acknowledged). Both
/// arms: once the node answers, the same marker reads saved.
#[test]
fn a_marker_in_flight_reads_pending_not_saved_and_saved_once_it_lands() {
    let node = PageNode::new();
    let (mut d, mut conn) = engine_db(&node);
    conn.hold_answers();
    d.mark_published(None, "rows").expect("mark");
    assert!(d.is_published(None, "rows").expect("is"), "THE SETUP: the marker does not exist, so its state says nothing");
    let pending = d.published_state(None, "rows").expect("state").expect("a marker");
    assert!(!pending.state.is_settled(), "a marker the node has not acknowledged reads saved: {:?}", pending.state);
    conn.stop_holding();
    while conn.held() > 0 {
        let _ = conn.release_one();
        d.store_mut().sync();
    }
    d.store_mut().sync();
    let saved = d.published_state(None, "rows").expect("state").expect("a marker");
    assert!(saved.state.is_settled(), "a marker the node acknowledged still reads in flight: {:?}", saved.state);
    assert_eq!(d.published_state(None, "other").expect("state"), None, "an unmarked domain has a marker record");
}
