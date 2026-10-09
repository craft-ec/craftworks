//! THE RESERVED `craftworks.` PREFIX, BY TYPE (ARCHITECTURE §19, "An app is DATA in its owner's tree"; app-as-data P2):
//! an app's definition lives in reserved domains of the app (`craftworks.draft`, `craftworks.app`, and until P5
//! `craftworks.published`), and ONLY the SDK's definition doors write them -- through the one commit path. An ordinary
//! write naming one is refused, whatever door it came in by.

use craftworks_sdk::definition::DefKey;
use craftworks_sdk::store::{Delta, Edit, Read, Reads, Store};
use craftworks_sdk::{app, CreateAt, Db, SystemEnv};
use core_types::name::SystemDomain;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use testkit::MemStore;

fn fresh() -> Db<MemStore, SystemEnv> {
    Db::new(MemStore::default(), SystemEnv, [0u8; 4])
}

/// AN ORDINARY WRITE OF THE RESERVED PREFIX IS REFUSED: by the app namespace (`app::write`, every Session write's
/// name) and by the Db itself (a stored name that reached it some other way). Every reserved form: the three names,
/// the bare prefix, and a name under it.
#[test]
fn an_ordinary_write_of_the_reserved_prefix_is_refused_by_the_namespace_and_the_db() {
    let schema: craftworks_sdk::Schema = serde_json::from_value(json!({ "type": "T", "fields": [{ "name": "x", "kind": "text" }] })).unwrap();
    for name in ["craftworks.draft", "craftworks.app", "craftworks.published", "craftworks", "craftworks.anything"] {
        let through_namespace = app::write(Some("notes"), name);
        let stored = format!("notes.{name}");
        let mut db = fresh();
        let defined = db.define(&stored, &schema);
        let mut f = Map::new();
        f.insert("x".into(), json!("v"));
        let put = db.put(&stored, &f);
        assert!(
            through_namespace.is_err() && defined.is_err() && put.is_err(),
            "an ordinary write of a reserved name was not refused: `{name}` (namespace {:?}, define {:?}, put {:?})",
            through_namespace.map(|n| n.to_string()),
            defined,
            put.map(|r| r.id)
        );
    }
}

/// THE CONTROL: an ordinary name that merely CONTAINS the word is the app's own.
#[test]
fn a_name_that_is_not_under_the_prefix_is_the_apps_own() {
    for name in ["notes", "craftworks_notes", "my.notes.craftworks", "craftworksx.y"] {
        assert!(app::write(Some("notes"), name).is_ok(), "`{name}` is not reserved, yet refused");
    }
}

fn key(s: &str) -> DefKey {
    DefKey::parse(s).expect(s)
}

/// THE DOORS WRITE WHAT THE ORDINARY WRITES MAY NOT: a draft edited (put, replaced, deleted), published into
/// `craftworks.app`, read back by the one `definition` read -- behind the app's id (the tree's form) and with none
/// (the in-tab db's) alike. The live markers at the builder's own slot.
#[test]
fn the_definition_doors_edit_the_draft_and_publish_it() {
    for app in [Some("notes"), None] {
        let mut db = fresh();
        db.draft_put(app, &key("meta"), &json!({ "name": "Notes" })).unwrap();
        db.draft_put(app, &key("c/header"), &json!({ "kind": "Text" })).unwrap();
        db.draft_put(app, &key("d/rows"), &json!({ "type": "Row" })).unwrap();
        db.draft_put(app, &key("c/header"), &json!({ "kind": "Heading" })).unwrap();
        assert!(db.draft_delete(app, &key("d/rows")).unwrap(), "a draft record was not deleted");
        assert!(!db.draft_delete(app, &key("d/rows")).unwrap(), "an absent record was deleted");
        let want = vec![(key("c/header"), json!({ "kind": "Heading" })), (key("meta"), json!({ "name": "Notes" }))];
        let sorted = |v: Vec<craftworks_sdk::db::DefRecord>| {
            let mut v: Vec<(DefKey, Value)> = v.into_iter().map(|r| (r.key, r.body)).collect();
            v.sort_by(|a, b| a.0.cmp(&b.0));
            v
        };
        let unsorted = |v: Vec<(DefKey, Value)>| {
            let mut v = v;
            v.sort_by(|a, b| a.0.cmp(&b.0));
            v
        };
        assert_eq!(sorted(db.definition(app, SystemDomain::Draft).unwrap()), unsorted(want.clone()));
        assert!(db.definition(app, SystemDomain::App).unwrap().is_empty(), "the app holds a definition before a publish");
        assert_eq!(db.publish_definition(app).unwrap(), 2);
        assert_eq!(sorted(db.definition(app, SystemDomain::App).unwrap()), unsorted(want));
        assert_eq!(db.publish_definition(app).unwrap(), 0, "an unchanged draft published something");
        // The live marker: created, then found (create_at's answer), at the builder's slot.
        assert!(!db.is_published(app, "rows").unwrap());
        assert!(matches!(db.mark_published(app, "rows").unwrap(), CreateAt::Created(_)));
        assert!(matches!(db.mark_published(app, "rows").unwrap(), CreateAt::Exists(_)));
        assert!(db.is_published(app, "rows").unwrap());
        assert_eq!(craftworks_sdk::slot_from(0, "domain", "rows").len(), 16);
    }
}

/// A store that SAMPLES the app's published definition after every commit it takes: what a reader between commits
/// would see.
struct Sampling {
    inner: MemStore,
    prefix: Vec<u8>,
    seen: Vec<BTreeMap<Vec<u8>, Vec<u8>>>,
}
impl Store for Sampling {
    fn apply_batch(&mut self, edits: &[(Vec<u8>, Edit)]) -> Result<(), craftworks_sdk::Refused> {
        self.inner.apply_batch(edits)?;
        let mut hi = self.prefix.clone();
        hi.push(0xff);
        self.seen.push(self.inner.scan(&self.prefix, &hi, false, usize::MAX).expect("in memory").into_iter().collect());
        Ok(())
    }
}
impl Reads for Sampling {
    fn get(&mut self, key: &[u8]) -> Read<Option<Vec<u8>>> {
        self.inner.get(key)
    }
    fn scan(&mut self, lo: &[u8], hi: &[u8], reverse: bool, limit: usize) -> Read<Vec<(Vec<u8>, Vec<u8>)>> {
        self.inner.scan(lo, hi, reverse, limit)
    }
    fn root(&mut self) -> Read<[u8; 32]> {
        self.inner.root()
    }
    fn changes_since(&mut self, from: [u8; 32], lo: &[u8], hi: &[u8], max_entries: u32) -> Read<Delta> {
        self.inner.changes_since(from, lo, hi, max_entries)
    }
}

/// NO READER EVER SEES A MIXED DEFINITION (§19 "Publish is ONE write"; engineer1's E1): the app's published records
/// sampled after EVERY commit -- draft edits and publishes alike, over three publishes that write AND delete (a
/// record changed, one removed, one added; then the rest removed and new ones added) -- are each exactly the empty
/// definition or one whole published version. Mutant "publish one record per write" -> red.
#[test]
fn a_reader_between_commits_never_sees_a_mixed_definition() {
    let prefix = craftworks_sdk::db::record_prefix(&format!("notes.{}", SystemDomain::App.code()));
    let mut db = Db::new(Sampling { inner: MemStore::default(), prefix, seen: Vec::new() }, SystemEnv, [0u8; 4]);
    let app = Some("notes");
    let versions: Vec<Vec<(&str, Value)>> = vec![
        vec![("meta", json!({ "v": 1 })), ("c/a", json!(1)), ("c/b", json!(1)), ("d/x", json!(1))],
        vec![("meta", json!({ "v": 2 })), ("c/a", json!(2)), ("c/c", json!(2)), ("d/x", json!(1))],
        vec![("meta", json!({ "v": 3 })), ("c/d", json!(3)), ("d/y", json!(3))],
    ];
    let mut published: Vec<BTreeMap<Vec<u8>, Vec<u8>>> = vec![BTreeMap::new()];
    let mut had: Vec<&str> = Vec::new();
    let mut commits_per_publish = Vec::new();
    for v in &versions {
        for gone in had.iter().filter(|k| !v.iter().any(|(n, _)| n == *k)) {
            db.draft_delete(app, &key(gone)).unwrap();
        }
        for (k, body) in v {
            db.draft_put(app, &key(k), body).unwrap();
        }
        let before = db.store().seen.len();
        assert!(db.publish_definition(app).unwrap() > 0, "THE SETUP: a publish changed nothing");
        commits_per_publish.push(db.store().seen.len() - before);
        published.push(db.store().seen.last().unwrap().clone());
        had = v.iter().map(|(k, _)| *k).collect();
    }
    let seen = &db.store().seen;
    assert!(seen.len() > versions.len() * 3, "THE SETUP: too few commits sampled ({})", seen.len());
    for (i, s) in seen.iter().enumerate() {
        assert!(published.contains(s), "after commit {i} a reader saw a definition that is no published version: {} records", s.len());
    }
    // And the mechanism: each publish was ONE commit.
    assert_eq!(commits_per_publish, vec![1; versions.len()], "a publish was not ONE commit");
    // Non-vacuity: every published version was different and actually seen.
    for (a, b) in published.iter().zip(published.iter().skip(1)) {
        assert_ne!(a, b, "THE SETUP: two consecutive versions were equal");
    }
}

/// A definition is READ by whose app it is (§19 P3: the builder lists its projects by each one's `meta`): one tree,
/// two apps, each app's records only under its own name; a bad app id is refused by the one app rule.
#[test]
fn a_definition_is_read_by_app_and_only_that_apps() {
    let mut db = fresh();
    db.draft_put(Some("proj-a"), &key("meta"), &json!({ "name": "A" })).unwrap();
    db.draft_put(Some("proj-b"), &key("meta"), &json!({ "name": "B" })).unwrap();
    db.draft_put(Some("proj-b"), &key("c/list"), &json!({ "type": "table" })).unwrap();
    let names = |db: &mut Db<MemStore, SystemEnv>, app| {
        db.definition(Some(app), SystemDomain::Draft).unwrap().into_iter().map(|r| (r.key.to_string(), r.body)).collect::<BTreeMap<_, _>>()
    };
    assert_eq!(names(&mut db, "proj-a"), BTreeMap::from([("meta".to_string(), json!({ "name": "A" }))]));
    assert_eq!(names(&mut db, "proj-b").get("meta"), Some(&json!({ "name": "B" })));
    assert_eq!(names(&mut db, "proj-b").len(), 2, "B's draft read another app's records, or lost its own");
    assert!(names(&mut db, "proj-c").is_empty(), "an app with no draft read as someone else's");
    for bad in ["Proj", "a.b", "@b", "a/b", ""] {
        assert!(db.definition(Some(bad), SystemDomain::Draft).is_err(), "`{bad}` was read as an app id");
    }
}

/// The apps that hold a draft (§19 P3b: the builder's project list): each app once, sorted; an app with only
/// ordinary data is not one, and the unnamed app's own draft is not an app of the tree.
#[test]
fn the_apps_with_a_draft_are_listed_and_only_those() {
    let mut db = fresh();
    assert!(db.definition_apps().unwrap().is_empty(), "an empty tree lists an app");
    db.draft_put(Some("proj-b"), &key("meta"), &json!({ "name": "B" })).unwrap();
    db.draft_put(Some("proj-a"), &key("meta"), &json!({ "name": "A" })).unwrap();
    db.draft_put(Some("proj-a"), &key("c/x"), &json!({ "type": "t" })).unwrap();
    db.draft_put(None, &key("meta"), &json!({ "name": "unnamed" })).unwrap();
    let schema: craftworks_sdk::Schema = serde_json::from_value(json!({ "type": "Row", "fields": [{ "name": "x", "kind": "text" }] })).unwrap();
    db.define("proj-c.rows", &schema).unwrap();
    assert_eq!(db.definition_apps().unwrap(), vec!["proj-a".to_string(), "proj-b".to_string()]);
}

/// THE APP'S CODE AS DATA (app-as-data P5): a file's BYTES through the file door -- drafted, replaced, published by
/// the one publish write, and read back byte for byte by the one definition read -- in both name forms; `draftPut`
/// cannot write a file (one form per kind), and a definition record's body is unaffected.
#[test]
fn a_file_is_drafted_published_and_read_back_byte_for_byte() {
    for app in [Some("notes"), None] {
        let mut db = fresh();
        let js: Vec<u8> = (0..38_006u32).map(|i| (i % 251) as u8).collect(); // the builder's app.js, by size
        db.draft_put(app, &key("meta"), &json!({ "entry": "f/app.js" })).unwrap();
        db.draft_file(app, "app.js", b"old", &json!({})).unwrap();
        db.draft_file(app, "app.js", &js, &json!({})).unwrap();
        db.draft_file(app, "components/header.js", b"export default 1;", &json!({ "type": "text/javascript" })).unwrap();
        assert!(db.draft_put(app, &key("f/x.js"), &json!({})).is_err(), "draftPut wrote a file");
        assert_eq!(db.publish_definition(app).unwrap(), 3);
        let recs = db.definition(app, SystemDomain::App).unwrap();
        let get = |k: &str| recs.iter().find(|r| r.key.to_string() == k).cloned().unwrap_or_else(|| panic!("no {k}"));
        assert_eq!(get("f/app.js").bytes.as_deref(), Some(js.as_slice()), "a file did not read back byte for byte");
        assert_eq!(get("f/components/header.js").body, json!({ "type": "text/javascript" }));
        assert_eq!(get("meta").bytes, None, "a definition record came back with bytes");
        assert!(db.draft_file(app, "../x.js", b"x", &json!({})).is_err() && db.draft_file(app, "", b"x", &json!({})).is_err(), "a bad path was taken");
    }
}

/// THE ENTRY LOADS (the SDK the one owner): `meta.entry` must name a file of the draft, or the publish is refused by
/// name -- and nothing is published; a definition with no entry is a definition-only app and publishes.
#[test]
fn a_publish_whose_entry_is_not_a_file_of_the_draft_is_refused() {
    let mut db = fresh();
    let app = Some("notes");
    db.draft_file(app, "main.js", b"1", &json!({})).unwrap();
    for entry in [json!("f/app.js"), json!("c/app"), json!("app.js"), json!(7)] {
        db.draft_put(app, &key("meta"), &json!({ "entry": entry })).unwrap();
        let e = db.publish_definition(app).expect_err("an entry naming no file of the draft was published");
        assert!(e.to_string().contains("meta.entry"), "the refusal does not name the entry: {e}");
        assert!(db.definition(app, SystemDomain::App).unwrap().is_empty(), "a refused publish published something");
    }
    db.draft_put(app, &key("meta"), &json!({ "entry": "f/main.js" })).unwrap();
    assert_eq!(db.publish_definition(app).unwrap(), 2, "THE CONTROL: an entry that is a file of the draft publishes");
    let mut plain = fresh();
    plain.draft_put(app, &key("meta"), &json!({ "name": "no code" })).unwrap();
    assert_eq!(plain.publish_definition(app).unwrap(), 1, "a definition-only app (no entry) did not publish");
}

/// A FILE OVER A RECORD'S BOUND IS REFUSED BY NAME: its path and both sizes (the record's and the file's), and the
/// limit -- nothing written. THE CONTROL: a file under it is taken.
#[test]
fn a_file_over_the_record_bound_is_refused_naming_its_path_and_sizes() {
    let mut db = fresh();
    let big = vec![7u8; freenet_prolly::node::MAX_VALUE];
    let e = db.draft_file(Some("notes"), "vendor/big.js", &big, &json!({})).expect_err("a file over the bound was taken");
    let said = e.to_string();
    assert!(said.contains("f/vendor/big.js") && said.contains(&big.len().to_string()) && said.contains(&freenet_prolly::node::MAX_VALUE.to_string()), "the refusal does not name the path and sizes: {said}");
    assert!(db.definition(Some("notes"), SystemDomain::Draft).unwrap().is_empty(), "a refused file was written");
    db.draft_file(Some("notes"), "vendor/ok.js", &vec![7u8; 200_000], &json!({})).expect("THE CONTROL: a 200 KB file is taken");
}
