//! Children of one parent are a BAND, and reading them is bounded.
//!
//! Before craftworks-sdk#122 a record's key could not carry a parent, so the
//! parent lived in a field and "the children of P" could only be a scan of the
//! whole domain followed by a filter — by construction, not by oversight. The
//! builder painted a project list by calling that once per row, which made
//! painting the list quadratic in the library.
//!
//! The measurement here is the point of the issue, so it is asserted rather
//! than described: a children-of-P read must not grow with the number of
//! records under OTHER parents. Without that control this change is a refactor
//! with no evidence.
use craftworks_sdk::id::{from_hex, loc_from_hex};
use craftworks_sdk::*;
use testkit::MemStore;
use serde_json::{json, Map, Value};

struct FakeEnv {
    now: u64,
    n: u32,
}
impl Env for FakeEnv {
    fn now_ms(&mut self) -> u64 {
        self.now += 1;
        self.now
    }
    fn rand32(&mut self) -> u32 {
        self.n = self.n.wrapping_mul(1664525).wrapping_add(1013904223);
        self.n
    }
}

/// A store that counts the keys each scan actually visits.
///
/// THE INSTRUMENT. "Cheaper" is not a claim a test can make by timing a
/// BTreeMap; what distinguishes a band read from a scan-and-filter is how many
/// keys the store is asked to hand back, and that is countable exactly.
#[derive(Default)]
struct Counting {
    inner: MemStore,
    visited: std::cell::Cell<usize>,
}
impl Store for Counting {
    fn apply_batch(&mut self, edits: &[(Vec<u8>, store::Edit)]) -> Result<(), craftworks_sdk::Refused> {
        self.inner.apply_batch(edits)
    }
}
impl Reads for Counting {
    fn get(&mut self, key: &[u8]) -> store::Read<Option<Vec<u8>>> {
        self.inner.get(key)
    }
    fn scan(
        &mut self,
        lo: &[u8],
        hi: &[u8],
        reverse: bool,
        limit: usize,
    ) -> store::Read<Vec<(Vec<u8>, Vec<u8>)>> {
        let out = self.inner.scan(lo, hi, reverse, limit)?;
        self.visited.set(self.visited.get() + out.len());
        Ok(out)
    }
    fn root(&mut self) -> store::Read<[u8; 32]> {
        self.inner.root()
    }
    fn changes_since(
        &mut self,
        from: [u8; 32],
        lo: &[u8],
        hi: &[u8],
        max_entries: u32,
    ) -> store::Read<Delta> {
        self.inner.changes_since(from, lo, hi, max_entries)
    }
}

fn comp_schema() -> Schema {
    Schema {
        type_name: "Component".into(),
        fields: vec![
            Field { name: "project".into(), kind: Kind::Text, required: true },
            Field { name: "title".into(), kind: Kind::Text, required: false },
        ],
        parent: Some("project".into()),
    }
}

fn fields(parent: &str, title: &str) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("project".into(), json!(parent));
    m.insert("title".into(), json!(title));
    m
}

fn db() -> Db<Counting, FakeEnv> {
    Db::new(Counting::default(), FakeEnv { now: 1, n: 7 }, *b"dev1")
}

/// THE MEASUREMENT THE ISSUE ASKS FOR.
///
/// Two parents, 10 children and 1,000. Reading the 10 must cost the 10 — not
/// the 1,010 that a scan-and-filter of the domain would visit.
#[test]
fn a_children_read_does_not_grow_with_other_parents() {
    let mut d = db();
    d.define("component", &comp_schema()).unwrap();

    let small = "00000000000000000000000000000001";
    let large = "00000000000000000000000000000002";
    for i in 0..10 {
        d.put("component", &fields(small, &format!("s{i}"))).unwrap();
    }
    for i in 0..1000 {
        d.put("component", &fields(large, &format!("l{i}"))).unwrap();
    }

    let p = from_hex(small).unwrap();
    d.store().visited.set(0);
    let kids = d.children("component", &p, Scan::default()).unwrap();
    let visited = d.store().visited.get();

    assert_eq!(kids.len(), 10, "the band holds exactly this parent's children");
    assert_eq!(
        visited, 10,
        "reading {} children visited {visited} keys: a children-of-P read must \
         cost this parent's band, not the domain — the 1,000 under the other \
         parent are what a scan-and-filter would have walked",
        kids.len(),
    );

    // THE CONTROL that gives the number above its meaning: the whole domain IS
    // 1,010 keys, so an unbounded scan really would have visited them all.
    d.store().visited.set(0);
    let all = d.scan("component", Scan::default()).unwrap();
    assert_eq!(all.len(), 1010);
    assert_eq!(d.store().visited.get(), 1010);
}

/// The id an app holds addresses the record, whether or not it has a parent.
#[test]
fn a_parented_id_round_trips_through_get_update_and_delete() {
    let mut d = db();
    d.define("component", &comp_schema()).unwrap();
    let pid = "0000000000000000000000000000000a";
    let rec = d.put("component", &fields(pid, "first")).unwrap();

    // 64 hex: parent then rkey. Self-describing by LENGTH, so an app never has
    // to be told which kind of id it is holding, and never parses one.
    assert_eq!(rec.id.len(), 64);
    let loc = loc_from_hex(&rec.id).unwrap();
    assert_eq!(loc.parent, Some(from_hex(pid).unwrap()));

    let got = d.get("component", loc).unwrap().expect("addressable by its id");
    assert_eq!(got.fields["title"], json!("first"));

    let mut patch = Map::new();
    patch.insert("title".into(), json!("second"));
    let up = d.update("component", loc, &patch).unwrap();
    assert_eq!(up.fields["title"], json!("second"));
    assert_eq!(up.id, rec.id, "an update does not move the record");

    assert!(d.delete("component", loc).unwrap());
    assert!(d.get("component", loc).unwrap().is_none());
}

/// A bare rkey cannot address a record in a parent-keyed domain. A READ of one
/// answers `None`; a WRITE refuses.
///
/// **This reverses what #122 first said**, which was that `None` here is "a
/// WRONG ANSWER, with the app reporting no such record about one sitting right
/// there". That scenario needs a caller holding a 64-hex id and cutting the
/// parent off it — which the design forbids: an app never takes an id apart.
/// The bare ids that actually reach a read come from OUTSIDE — a value stored
/// before #122 made component ids 64 hex, a pasted link — and for those
/// "absent" is the honest answer (craftworks-sdk#118). A write still refuses,
/// because "nothing to do" would hide the caller's mistake.
#[test]
fn a_bare_id_reads_as_absent_and_a_write_with_one_refuses() {
    let mut d = db();
    d.define("component", &comp_schema()).unwrap();
    let rec = d.put("component", &fields("0000000000000000000000000000000a", "x")).unwrap();
    let bare = loc_from_hex(&rec.id).unwrap().rkey;

    assert!(d.get("component", bare).unwrap().is_none());
    // THE CONTROL: the record is there under its full id, so `None` above is
    // about the id, not about an empty domain.
    assert!(d.get("component", loc_from_hex(&rec.id).unwrap()).unwrap().is_some());
    let e = d.delete("component", bare).unwrap_err().to_string();
    assert!(e.contains("wants a 64-hex id"), "a refused write names the width it wanted; got: {e}");

    // And the other direction: a parented id in a domain with no parent.
    d.define(
        "note",
        &Schema {
            type_name: "Note".into(),
            fields: vec![Field { name: "body".into(), kind: Kind::Text, required: false }],
            parent: None,
        },
    )
    .unwrap();
    let plain = d.put("note", &Map::new()).unwrap();
    assert_eq!(plain.id.len(), 32, "no parent, no parent in the id");
    assert!(d.get("note", loc_from_hex(&rec.id).unwrap()).unwrap().is_none());
    let e = d.delete("note", loc_from_hex(&rec.id).unwrap()).unwrap_err().to_string();
    assert!(e.contains("does not key its records under a parent"), "got: {e}");
}

/// THE CONTROL FOR EVERYTHING ELSE: a domain that declares no parent is keyed,
/// addressed and read exactly as before, so nothing existing migrates.
#[test]
fn a_domain_without_a_parent_is_untouched() {
    let mut d = db();
    d.define(
        "note",
        &Schema {
            type_name: "Note".into(),
            fields: vec![Field { name: "body".into(), kind: Kind::Text, required: false }],
            parent: None,
        },
    )
    .unwrap();
    let mut m = Map::new();
    m.insert("body".into(), json!("hello"));
    let r = d.put("note", &m).unwrap();

    assert_eq!(r.id.len(), 32);
    let rk = from_hex(&r.id).unwrap();
    assert_eq!(d.get("note", rk).unwrap().unwrap().fields["body"], json!("hello"));
    assert_eq!(
        craftworks_sdk::db::record_key("note", rk).len(),
        1 + "note".len() + 1 + 16,
        "the key is the tag, the domain, a separator and the 16-byte id — as it was",
    );

    // And it has no children to read, which is a refusal rather than an empty
    // answer: "none" and "the question does not apply here" are different.
    let e = d.children("note", &rk, Scan::default()).unwrap_err().to_string();
    assert!(e.contains("does not declare a parent"), "got: {e}");
}

/// The parent decides the key, so it is the one part of a schema that cannot
/// be evolved: declaring, removing or moving it re-keys every record in the
/// domain. That migration is free only while no real data exists.
#[test]
fn the_parent_declaration_cannot_be_changed_by_a_later_define() {
    let mut d = db();
    d.define("component", &comp_schema()).unwrap();

    let mut without = comp_schema();
    without.parent = None;
    let e = d.define("component", &without).unwrap_err().to_string();
    assert!(e.contains("re-keys every record"), "got: {e}");

    let mut moved = comp_schema();
    moved.parent = Some("title".into());
    assert!(d.define("component", &moved).is_err());

    // Re-defining the SAME schema is still fine: the guard is about change.
    d.define("component", &comp_schema()).unwrap();
}

/// A parent field must exist, be text and be required — a record keyed under a
/// parent cannot be written without one.
#[test]
fn a_parent_field_must_be_declared_text_and_required() {
    let mut d = db();
    let mut s = comp_schema();
    s.parent = Some("nope".into());
    assert!(d.define("component", &s).unwrap_err().to_string().contains("not one of"));

    let mut s = comp_schema();
    s.fields[0].required = false;
    assert!(d.define("component", &s).unwrap_err().to_string().contains("must be required"));

    let mut s = comp_schema();
    s.fields.push(Field { name: "n".into(), kind: Kind::Int, required: true });
    s.parent = Some("n".into());
    assert!(d.define("component", &s).unwrap_err().to_string().contains("must be text"));
}

/// A patch cannot move a record between parents, because the parent decides
/// the key — it would leave the record filed under the old one while claiming
/// the new. A re-parent is a delete and a write, and it is not `update`.
#[test]
fn update_refuses_to_re_parent() {
    let mut d = db();
    d.define("component", &comp_schema()).unwrap();
    let rec = d.put("component", &fields("0000000000000000000000000000000a", "x")).unwrap();
    let loc = loc_from_hex(&rec.id).unwrap();

    let mut patch = Map::new();
    patch.insert("project".into(), json!("0000000000000000000000000000000b"));
    let e = d.update("component", loc, &patch).unwrap_err().to_string();
    assert!(e.contains("decides the record's key"), "got: {e}");

    // Patching it to the SAME parent is not a move, and is allowed.
    let mut same = Map::new();
    same.insert("project".into(), json!("0000000000000000000000000000000a"));
    same.insert("title".into(), json!("y"));
    assert_eq!(d.update("component", loc, &same).unwrap().fields["title"], json!("y"));
}

/// A record whose parent field is missing or unparseable is refused at the
/// write, not keyed under something invented.
#[test]
fn a_missing_or_malformed_parent_is_refused_at_the_write() {
    let mut d = db();
    d.define("component", &comp_schema()).unwrap();

    let mut m = Map::new();
    m.insert("title".into(), json!("orphan"));
    assert!(d.put("component", &m).is_err());

    let mut m = Map::new();
    m.insert("project".into(), json!("not-an-id"));
    let e = d.put("component", &m).unwrap_err().to_string();
    assert!(e.contains("32 hex characters"), "got: {e}");
}
