//! The page's WRITE half (`Writes`, R-b): the frames it sends, and what the
//! app is told of each of this client's writes. The QUEUE is the engine's
//! (`engine/tests/`, `tests/page_writes.rs`); what is here is only what this
//! client knows: framing, its own writes' keys, and the notices a PULLED fate
//! becomes.
//!
//! Ported from `tests/cached_store.rs`, whose outbox (window, `Busy`
//! re-send, `Lost` re-send, timeout) went with R-b.

use craftworks_sdk::{Edit, StoreError, Writes};
use page::fates::Fate;

fn writes() -> Writes {
    Writes::new(Box::new(|| 0))
}

fn put(k: &[u8], v: &[u8]) -> Vec<(Vec<u8>, Edit)> {
    vec![(k.to_vec(), Edit::Put(v.to_vec()))]
}

#[test]
fn the_error_says_what_to_do_about_it() {
    let msg = StoreError::NotLoaded.to_string();
    assert!(msg.contains("reload"), "unhelpful: {msg}");
    assert!(msg.contains("not known to be empty"), "the message does not distinguish itself from empty: {msg}");
    // R-b: the queue's refusal says what the person can do -- wait.
    let full = craftworks_sdk::Refused::QueueFull { bytes: 4000, limit: 4096 }.to_string();
    assert!(full.contains("wait"), "QUEUE_FULL does not say to wait: {full}");
    assert!(full.contains("4096"), "QUEUE_FULL does not say how full: {full}");
}

/// **THE TIME A PAGE SENDS IS THE WALL CLOCK, QUANTISED** (`protocol::tick_of`,
/// one place), and two tabs reading the same second send the same number.
#[test]
fn a_tick_carries_the_wall_clock_in_the_protocol_s_unit() {
    let mut s = writes();
    s.send_tick(1_700_000_123_456);
    let out = s.take_outbound();
    assert_eq!(out.len(), 1, "one tick, one frame");
    let want = protocol::tick_of(1_700_000_123_456);
    match protocol::decode_request(&out[0]) {
        protocol::Incoming::Ok(protocol::Envelope { body: protocol::Request::Tick { now }, .. }) => {
            assert_eq!(now, want, "the page quantised the clock its own way")
        }
        other => panic!("send_tick queued {other:?}, so the engine is told no time at all"),
    }
    let (mut a, mut b) = (writes(), writes());
    a.send_tick(1_700_000_123_456);
    b.send_tick(1_700_000_123_999);
    let bodies = |fs: Vec<Vec<u8>>| -> Vec<protocol::Request> {
        fs.iter()
            .filter_map(|f| match protocol::decode_request(f) {
                protocol::Incoming::Ok(e) => Some(e.body),
                _ => None,
            })
            .collect()
    };
    assert_eq!(bodies(a.take_outbound()), bodies(b.take_outbound()), "two tabs reading the same second sent different times");
}

/// A CONTROL for the quantum: two instants a tick apart are NOT the same.
#[test]
fn control_a_later_tick_is_a_later_number() {
    let (mut early, mut late) = (writes(), writes());
    early.send_tick(1_700_000_000_000);
    late.send_tick(1_700_000_000_000 + protocol::TICK_MS);
    assert_ne!(early.take_outbound(), late.take_outbound(), "a whole tick apart and the same number");
}

/// **A FLUSH IS WHAT A CLOSING PAGE SAYS**, and it must be a frame.
#[test]
fn a_flush_is_queued_as_a_frame() {
    let mut s = writes();
    s.send_flush();
    let out = s.take_outbound();
    assert_eq!(out.len(), 1, "one flush, one frame");
    assert!(
        matches!(protocol::decode_request(&out[0]), protocol::Incoming::Ok(protocol::Envelope { body: protocol::Request::Flush, .. })),
        "send_flush queued something else"
    );
}

/// A write is ONE frame, a `Commit` when it declares reads, carrying exactly
/// the ops it was made with -- and nothing more is sent for it, ever (R-b:
/// no window, no re-send; the engine owns the queue).
#[test]
fn a_write_is_one_frame_with_its_reads_and_is_never_sent_again() {
    let mut s = writes();
    let id = s.make(&[(b"a/1".to_vec(), protocol::Expect::Absent)], &put(b"a/1", b"v")).expect("made");
    let out = s.take_outbound();
    assert_eq!(out.len(), 1);
    match protocol::decode_request(&out[0]) {
        protocol::Incoming::Ok(env) => match env.body {
            protocol::Request::Commit { write_id, reads, ops } => {
                assert_eq!(write_id, id);
                assert_eq!(reads, vec![(b"a/1".to_vec(), protocol::Expect::Absent)]);
                assert_eq!(ops, vec![protocol::Op::Put(b"a/1".to_vec(), b"v".to_vec())]);
            }
            other => panic!("not a Commit: {other:?}"),
        },
        other => panic!("{other:?}"),
    }
    for fate in [Fate::Lost, Fate::Failed { why: engine::FailWhy::TreeDamaged, signer: None }] {
        s.on_fate(id, fate);
    }
    assert!(s.take_outbound().is_empty(), "an ended write went on the wire again");
}

/// A write that CONFLICTS on one key and also wrote another: after the
/// Conflict nothing of it is open, and BOTH keys say rolled back -- the
/// other key is not the conflicted one, so only the fall clears it (W6).
/// Control: the same write Published leaves both not rolled back.
#[test]
fn a_conflicted_write_leaves_none_of_its_own_keys_pending() {
    use craftworks_sdk::read_token::read_token;
    for published in [true, false] {
        let mut s = writes();
        let edits = vec![(b"a/1".to_vec(), Edit::Put(b"mine1".to_vec())), (b"a/2".to_vec(), Edit::Put(b"mine2".to_vec()))];
        let reads = vec![(b"a/1".to_vec(), protocol::Expect::Value(read_token(b"old1"))), (b"a/2".to_vec(), protocol::Expect::Value(read_token(b"old2")))];
        let id = s.make(&reads, &edits).expect("made");
        let fate = if published { Fate::Published { seq: 2, backed_up: false } } else { Fate::Conflict { key: b"a/1".to_vec(), current: Some(read_token(b"theirs")), after: None } };
        s.on_fate(id, fate);
        assert!(!s.is_open(id), "an ended write is still open");
        for k in [&b"a/1"[..], b"a/2"] {
            assert_eq!(s.rolled_back(k), !published, "published={published}: {k:?} rolled back wrongly");
        }
        if !published {
            let c = s.take_conflicts();
            assert_eq!(c.len(), 1);
            assert_eq!((c[0].write_id, c[0].key.as_slice()), (id, &b"a/1"[..]), "the conflict does not name its key");
        }
    }
}

/// builder#107: a client's OWN write ending names its KEYS, once, so the rows
/// showing it re-read -- "saving" to "saved", or to gone. A write still in
/// the queue names none (its stage is the engine's; nothing about its row's
/// settled state changed).
#[test]
fn an_own_writes_end_names_its_keys_once() {
    let both = vec![b"a/1".to_vec(), b"a/2".to_vec()];
    for fate in [Fate::Published { seq: 1, backed_up: false }, Fate::Lost, Fate::Failed { why: engine::FailWhy::TreeDamaged, signer: None }] {
        let mut s = writes();
        let edits = vec![(b"a/1".to_vec(), Edit::Put(b"v1".to_vec())), (b"a/2".to_vec(), Edit::Put(b"v2".to_vec()))];
        let id = s.make(&[(b"a/1".to_vec(), protocol::Expect::Absent), (b"a/2".to_vec(), protocol::Expect::Absent)], &edits).expect("made");
        s.on_fate(id, Fate::Queued);
        assert!(s.take_state_changed().is_empty(), "{fate:?}: a queued stage named keys");
        s.on_fate(id, fate.clone());
        assert_eq!(s.take_state_changed(), both, "{fate:?}: the ended write did not name its keys");
        s.on_fate(id, fate.clone());
        assert!(s.take_state_changed().is_empty(), "{fate:?}: a repeated fate named the keys again");
    }
}

/// **QueueFull is the door's, never an end** (sdk#450): the call that made the write returns it, and the write is
/// closed there (`refused_at_door`), so a QueueFull fate for it later ends nothing and names nothing -- no second
/// telling. The SDK makes the write again as a new one.
#[test]
fn a_write_refused_queue_full_at_the_door_is_closed_by_the_refusal() {
    let mut s = writes();
    let id = s.make(&[(b"a/1".to_vec(), protocol::Expect::Absent)], &[(b"a/1".to_vec(), Edit::Put(b"v".to_vec()))]).expect("made");
    assert!(s.is_open(id), "THE SETUP: the made write is not open");
    s.refused_at_door(id, craftworks_sdk::Refused::QueueFull { bytes: 1, limit: 1 });
    assert!(!s.is_open(id), "a write refused at the door is still open");
    s.on_fate(id, Fate::QueueFull { bytes: 1, limit: 1 });
    assert!(s.take_ended().is_empty() && s.take_state_changed().is_empty(), "a door-refused write was told again as an end");
}

/// **A QueueFull fate for a still-OPEN write is loud in debug and a counted no-op in release** (sdk#450, the
/// architect: a panic is a dead wasm page). `hand_over` rules it out; if it ever arrives it ends and names nothing.
#[test]
fn a_late_queue_full_for_an_open_write_ends_nothing_and_is_counted() {
    let mut s = writes();
    let id = s.make(&[(b"a/1".to_vec(), protocol::Expect::Absent)], &[(b"a/1".to_vec(), Edit::Put(b"v".to_vec()))]).expect("made");
    let late = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.on_fate(id, Fate::QueueFull { bytes: 1, limit: 1 })));
    assert_eq!(late.is_err(), cfg!(debug_assertions), "debug is not loud (or release panics: a dead wasm page)");
    if !cfg!(debug_assertions) {
        assert_eq!(s.late_door_verdicts, 1, "the late door verdict was not counted");
    }
    assert!(s.is_open(id), "a late QueueFull ended the write");
    assert!(s.take_ended().is_empty() && s.take_state_changed().is_empty(), "a late QueueFull was told as an end");
}

/// sdk#264 / builder#107: a SUPERSEDED write names its keys, so a plain
/// binding on them re-reads the winner's value. The control: a `Superseded`
/// for another session names none.
#[test]
fn a_superseded_write_names_its_keys() {
    let named = |ours: bool| -> Vec<Vec<u8>> {
        let mut s = writes();
        s.make(&[(b"a/1".to_vec(), protocol::Expect::Any)], &put(b"a/1", b"mine")).expect("made");
        let _ = s.take_outbound();
        let session = s.client.session().expect("a session");
        let to = if ours { session } else { session + 1 };
        s.on_inbound(&protocol::encode_reply(&protocol::Reply::Superseded { session: to, write_id: 1, seq: 2, root: [2; 32], keys: vec![b"a/1".to_vec()] }).expect("encodes"));
        s.take_state_changed()
    };
    assert_eq!(named(true), vec![b"a/1".to_vec()], "a superseded write did not name its keys");
    assert_eq!(named(false), Vec::<Vec<u8>>::new(), "another session's Superseded named keys here");
}

/// The keys a state change names become DOMAINS by `Db::domain_of_key`: a
/// record key names its domain; a schema key names none.
#[test]
fn a_record_key_names_its_domain_and_a_schema_key_none() {
    use craftworks_sdk::db::record_key;
    type D = craftworks_sdk::Db<testkit::MemStore, craftworks_sdk::SystemEnv>;
    let loc = craftworks_sdk::id::loc_from_hex(&"ab".repeat(16)).expect("an id");
    assert_eq!(D::domain_of_key(&record_key("notes", loc)), Some("notes".to_string()));
    let mut schema = vec![0u8];
    schema.extend_from_slice(b"schema\0notes");
    assert_eq!(D::domain_of_key(&schema), None, "a schema key named a domain");
    assert_eq!(D::domain_of_key(b""), None);
}

/// builder#107, after craftworks-sdk#267: own state changes are reported by
/// the domain the APP names, once each, never the stored `<app>.<name>`.
#[test]
fn own_state_changes_name_the_app_relative_domain_its_bindings_are_keyed_by() {
    use craftworks_sdk::db::record_key;
    type D = craftworks_sdk::Db<testkit::MemStore, craftworks_sdk::SystemEnv>;
    let loc = craftworks_sdk::id::loc_from_hex(&"ab".repeat(16)).expect("an id");
    let app = "rmud6o02cnfqk0001";
    let mine = record_key(&format!("{app}.notes"), loc);
    let also_mine = record_key(&format!("{app}.craftworks.published"), loc);
    let theirs = record_key("otherapp.notes", loc);
    let mut schema = vec![0u8];
    schema.extend_from_slice(format!("schema\0{app}.notes").as_bytes());
    let names = |v: Vec<craftworks_sdk::app::AppName>| v.iter().map(|a| a.as_str().to_string()).collect::<Vec<_>>();
    assert_eq!(
        names(D::own_domains_of_keys(Some(app), &[mine.clone(), also_mine.clone(), mine.clone(), theirs.clone(), schema])),
        vec!["craftworks.published".to_string(), "notes".to_string()],
    );
    assert!(!names(D::own_domains_of_keys(Some(app), std::slice::from_ref(&mine))).contains(&format!("{app}.notes")), "the stored name leaked through");
    assert_eq!(names(D::own_domains_of_keys(None, &[record_key("notes", loc)])), vec!["notes".to_string()]);
}

/// **A FORCED write that fell `Lost` is NAMED as forced** (sdk#235, WRITE-PATH
/// ⁷): the engine never re-applies it (it has no premise to re-check), and
/// the app is told which it was. Control: a checked write's `Lost` (its tries
/// spent in the engine) is named as that.
#[test]
fn a_forced_write_that_fell_lost_is_named_as_forced() {
    use craftworks_sdk::writes::Ended;
    for (reads, want) in [(protocol::Expect::Any, Ended::ForcedLost), (protocol::Expect::Absent, Ended::Lost)] {
        let mut s = writes();
        let id = s.make(&[(b"a/1".to_vec(), reads.clone())], &put(b"a/1", b"v")).expect("made");
        s.on_fate(id, Fate::Lost);
        assert_eq!(s.take_ended(), vec![(id, want.clone())], "{reads:?}: the fallen write was not named as {want:?}");
        assert!(s.rolled_back(b"a/1"), "the row still claims the fallen write");
    }
}

/// **`Unread` is named WITH its key** (sdk#235): our bug, never the person's.
#[test]
fn an_unread_write_is_named_with_its_key() {
    let mut s = writes();
    let id = s.make(&[(b"a/1".to_vec(), protocol::Expect::Absent)], &put(b"a/1", b"one")).expect("made");
    s.on_fate(id, Fate::Unread { key: b"a/1".to_vec() });
    let told = s.take_unread();
    assert_eq!(told.len(), 1, "{told:?}");
    assert_eq!((told[0].write_ids.clone(), told[0].key.clone()), (vec![id], Some(b"a/1".to_vec())));
    assert!(s.rolled_back(b"a/1"), "the row still claims a value that was never saved");
}
