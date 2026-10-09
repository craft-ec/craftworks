//! A read handed a well-formed id of the WRONG WIDTH answers "not found" — and
//! RECORDS it (craftworks-sdk#118, and the ruling recorded on that issue).
//!
//! Answering "not found" is right: the ids reaching a read come from outside.
//! It also makes one programmer error silent — a caller that truncated a
//! 64-hex id — so it is recorded in the instrument rather than thrown. Two
//! numbers from the reviewed vocabulary, `IdWidthGiven` and `IdWidthWanted`;
//! never the id, and never the domain, whose name is the app's.
//!
//! The chain is proved in two halves, each against the real parts:
//!   - `Db::get` → the store's `wrong_width` hook, over a store that forwards
//!     into a real instrument `Client`;
//!   - `PageStore` — what the browser session reads through — forwarding
//!     that hook into its own client's recording.
use craftworks_sdk::engine_client::Client;
use craftworks_sdk::id::{loc_from_hex, Loc};
use craftworks_sdk::store::IdWidth;
use craftworks_sdk::*;
use testkit::MemStore;
use instrument::{vocab::Key, Record as _};
use serde_json::{json, Map, Value};

struct FakeEnv(u64);
impl Env for FakeEnv {
    fn now_ms(&mut self) -> u64 {
        self.0 += 1;
        self.0
    }
    fn rand32(&mut self) -> u32 {
        7
    }
}

/// A MemStore whose `wrong_width` goes to a real instrument client.
struct Recorded {
    inner: MemStore,
    client: Client,
}
impl Store for Recorded {
    fn apply_batch(&mut self, edits: &[(Vec<u8>, store::Edit)]) -> Result<(), craftworks_sdk::Refused> {
        self.inner.apply_batch(edits)
    }
}
impl Reads for Recorded {
    fn get(&mut self, key: &[u8]) -> store::Read<Option<Vec<u8>>> {
        self.inner.get(key)
    }
    fn scan(&mut self, lo: &[u8], hi: &[u8], reverse: bool, limit: usize) -> store::Read<Vec<(Vec<u8>, Vec<u8>)>> {
        self.inner.scan(lo, hi, reverse, limit)
    }
    fn root(&mut self) -> store::Read<[u8; 32]> {
        self.inner.root()
    }
    fn changes_since(&mut self, f: [u8; 32], lo: &[u8], hi: &[u8], m: u32) -> store::Read<Delta> {
        self.inner.changes_since(f, lo, hi, m)
    }
    fn wrong_width(&self, given: IdWidth, wanted: IdWidth) {
        self.client.record_wrong_width(given, wanted);
    }
}

fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => unreachable!(),
    }
}

/// Two domains with names that are USER CONTENT, so a leak would be visible.
fn db() -> Db<Recorded, FakeEnv> {
    let mut client = Client::new();
    client.record_into(64);
    let mut d = Db::new(Recorded { inner: MemStore::default(), client }, FakeEnv(1), *b"dev1");
    d.define("medical", &Schema {
        type_name: "Note".into(),
        fields: vec![Field { name: "pid".into(), kind: Kind::Text, required: true }],
        parent: Some("pid".into()),
    }).unwrap();
    d.define("patient-notes", &Schema {
        type_name: "Plain".into(),
        fields: vec![Field { name: "body".into(), kind: Kind::Text, required: false }],
        parent: None,
    }).unwrap();
    d
}

fn totals(d: &Db<Recorded, FakeEnv>) -> (u64, u64) {
    let r = d.store().client.recording().expect("a recording is attached");
    (r.total(Key::IdWidthGiven), r.total(Key::IdWidthWanted))
}

#[test]
fn a_truncated_id_reads_as_absent_and_is_recorded_with_both_widths() {
    let mut d = db();
    let rec = d.put("medical", &obj(json!({"pid": "0123456789abcdef0123456789abcdef"}))).unwrap();
    // The caller cut the parent off a 64-hex id: the programmer error.
    let truncated = loc_from_hex(&rec.id).unwrap().rkey;
    assert!(d.get("medical", Loc::bare(truncated)).unwrap().is_none(), "absent, not a throw");
    assert_eq!(totals(&d), (32, 64), "given 32, wanted 64");
}

#[test]
fn a_parented_id_in_a_plain_domain_is_recorded_the_other_way_round() {
    let mut d = db();
    let sixty_four = loc_from_hex(&"0123456789abcdef".repeat(4)).unwrap();
    assert!(d.get("patient-notes", sixty_four).unwrap().is_none());
    assert_eq!(totals(&d), (64, 32));
}

/// THE CONTROL: a RIGHT-width id that simply is not there records NOTHING.
/// Without it, an event on every miss would pass both tests above.
#[test]
fn a_right_width_miss_records_nothing() {
    let mut d = db();
    let absent = loc_from_hex(&"f".repeat(32)).unwrap();
    assert!(d.get("patient-notes", absent).unwrap().is_none());
    let absent_parented = loc_from_hex(&"e".repeat(64)).unwrap();
    assert!(d.get("medical", absent_parented).unwrap().is_none());
    assert_eq!(totals(&d), (0, 0), "a plain miss is not a wrong width");
}

/// The event carries two numbers and nothing else: the domain names above are
/// user content, and the dump must not contain them.
#[test]
fn the_record_names_no_domain_and_no_id() {
    let mut d = db();
    let sixty_four = "0123456789abcdef".repeat(4);
    d.get("patient-notes", loc_from_hex(&sixty_four).unwrap()).unwrap();
    let dump = d.store().client.dump("wrong width");
    for secret in ["medical", "patient-notes", sixty_four.as_str(), "0123456789abcdef"] {
        assert!(!dump.contains(secret), "the dump contains {secret}:\n{dump}");
    }
    assert_eq!(totals(&d), (64, 32), "and the record IS there: {dump}");
}

/// Writes with the wrong width still REFUSE, and say which width they wanted.
#[test]
fn a_wrong_width_write_refuses_and_names_the_width_wanted() {
    let mut d = db();
    let bare = Loc::bare(loc_from_hex(&"a".repeat(32)).unwrap().rkey);
    let e = d.delete("medical", bare).unwrap_err().to_string();
    assert!(e.contains("wants a 64-hex id"), "got: {e}");
    let e = d.update("patient-notes", loc_from_hex(&"b".repeat(64)).unwrap(), &Map::new()).unwrap_err().to_string();
    assert!(e.contains("wants a 32-hex id"), "got: {e}");
}

/// The browser session reads through `PageStore`; its hook must reach its
/// client's recording, or the whole chain above records into nothing in the
/// product.
#[test]
fn page_store_forwards_the_hook_into_its_recording() {
    // Through the probed fixture, not built bare (fixture-gate).
    let node = testkit::PageNode::new();
    let (mut s, _conn, _clock) = testkit::page_store(&node);
    s.writes.client.record_into(16);
    craftworks_sdk::Reads::wrong_width(&s, IdWidth::Bare, IdWidth::Parented);
    let r = s.writes.client.recording().expect("a recording");
    assert_eq!((r.total(Key::IdWidthGiven), r.total(Key::IdWidthWanted)), (32, 64));
}
