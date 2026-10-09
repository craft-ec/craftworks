//! THE APP NAMESPACE's rules (the forest ruling), natively: what the web
//! Session applies to every name that crosses into it.
use craftworks_sdk::app::{check, own, read, write, StoredName};

/// What `own` makes of a name as the TREE holds it — app-relative, or `None`.
fn mine(app: Option<&str>, stored: &str) -> Option<String> {
    own(app, &StoredName::of_tree(stored.to_string())).map(|a| a.as_str().to_string())
}

#[test]
fn an_apps_names_are_relative_and_stored_under_its_own_prefix() {
    assert_eq!(write(Some("alpha"), "notes").unwrap().as_str(), "alpha.notes");
    assert_eq!(read(Some("alpha"), "notes").unwrap().as_str(), "alpha.notes");
    // A watch key keeps its parent band: the prefix goes before the domain.
    assert_eq!(read(Some("alpha"), "notes#00ff").unwrap().as_str(), "alpha.notes#00ff");
    assert_eq!(mine(Some("alpha"), "alpha.notes").as_deref(), Some("notes"));
}

#[test]
fn another_apps_data_is_read_absolutely_and_never_written() {
    assert_eq!(read(Some("alpha"), "@beta/notes").unwrap().as_str(), "beta.notes");
    let e = write(Some("alpha"), "@beta/notes").unwrap_err();
    assert!(e.to_string().contains("another app's"), "{e}");
    assert!(read(Some("alpha"), "@beta").is_err(), "an @ name with no /name was read");
    assert!(read(Some("alpha"), "@BAD/notes").is_err(), "an @ name with a bad app id was read");
}

#[test]
fn a_session_with_no_app_writes_nothing_and_reads_names_as_given() {
    let e = write(None, "notes").unwrap_err();
    assert!(e.to_string().starts_with("refused") || e.to_string().contains("no app"), "{e}");
    assert_eq!(read(None, "notes").unwrap().as_str(), "notes");
    assert_eq!(mine(None, "whatever.notes").as_deref(), Some("whatever.notes"));
}

/// The listing an app sees is ITS domains only — and a name that merely
/// STARTS with the app id is somebody else's (`alphabet` is not `alpha`).
#[test]
fn an_app_sees_only_its_own_domains_and_a_longer_app_id_is_not_it() {
    for other in ["beta.notes", "alphabet.notes", "alpha-2.notes", "alpha", "notes"] {
        assert_eq!(mine(Some("alpha"), other), None, "`{other}` was taken for alpha's");
    }
}

#[test]
fn an_app_id_is_one() {
    for ok in ["a", "notes-example", "app_1", &"x".repeat(32)] {
        assert!(check(ok).is_ok(), "{ok}");
    }
    for bad in ["", "has.dot", "UPPER", "sl/ash", "@x", &"x".repeat(33)] {
        assert!(check(bad).is_err(), "{bad} was taken for an app id");
    }
}

// ── sdk#276: an app id never eats into the names the app writes ──────────

use craftworks_sdk::app::{check_name, MAX_APP, MAX_NAME};
use craftworks_sdk::db::{record_key, MAX_DOMAIN};
use craftworks_sdk::id::Loc;
use craftworks_sdk::{Db, Env, Scan};
use testkit::MemStore;

struct Clock(u64, u32);
impl Env for Clock {
    fn now_ms(&mut self) -> u64 {
        self.0
    }
    fn rand32(&mut self) -> u32 {
        self.1 = self.1.wrapping_mul(1664525).wrapping_add(1013904223);
        self.1
    }
}

/// Define, put, read and scan through the REAL core, under the stored name
/// the app's own name maps to.
fn round_trip(app: &str, name: &str) -> Result<usize, String> {
    let stored = write(Some(app), name).map_err(|e| e.to_string())?;
    let mut d = Db::new(MemStore::default(), Clock(1_700_000_000_000, 7), *b"dev1");
    let schema = serde_json::from_value(serde_json::json!({ "type": "Row", "fields": [{ "name": "n", "kind": "int" }] })).unwrap();
    d.define(&stored, &schema).map_err(|e| e.to_string())?;
    let mut f = serde_json::Map::new();
    f.insert("n".into(), serde_json::json!(1));
    let r = d.put(&stored, &f).map_err(|e| e.to_string())?;
    let at = craftworks_sdk::id::from_hex(&r.id).ok_or_else(|| format!("id `{}` is not a bare rkey", r.id))?;
    assert!(d.get(&stored, at).map_err(|e| e.to_string())?.is_some(), "the record put under `{stored}` does not read back");
    Ok(d.scan(&stored, Scan { reverse: false, limit: 0, after: None }).map_err(|e| e.to_string())?.len())
}

/// **The builder's case** (sdk#276): a 17-character project id and its own
/// 20-character bookkeeping domain. Refused before, as a 38-character domain.
/// Since app-as-data P2 that domain is RESERVED (`SystemDomain::Published`):
/// an ordinary write of it is refused, and its door writes it -- under the
/// same project id, within the same budget.
#[test]
fn the_builders_project_id_and_its_domain_write() {
    let (app, marker) = ("rmud6o02cnfqk0001", core_types::name::SystemDomain::Published.code());
    assert!(round_trip(app, marker).is_err_and(|e| e.contains("reserved")), "an ordinary write of `{marker}` was not refused");
    let mut d = Db::new(MemStore::default(), Clock(1_700_000_000_000, 7), *b"dev1");
    assert!(matches!(d.mark_published(Some(app), "rows"), Ok(craftworks_sdk::CreateAt::Created(_))), "the marker door did not write under `{app}`");
    assert!(d.is_published(Some(app), "rows").unwrap());
    assert_eq!(round_trip(app, "rows"), Ok(1), "THE CONTROL: the project's own domain writes");
}

/// **An app at the longest id writes a name at the longest name**, and reads
/// and scans it. An id at the maximum used to leave NO characters at all.
#[test]
fn an_app_at_the_longest_id_writes_the_longest_name() {
    let (app, name) = ("a".repeat(MAX_APP), "n".repeat(MAX_NAME));
    assert_eq!(round_trip(&app, &name), Ok(1));
    assert_eq!(write(Some(&app), &name).unwrap().len(), MAX_DOMAIN, "the stored name is the whole budget");
}

/// **A name one past the rule is refused by name — the name the app WROTE**,
/// not the prefixed one it never saw; and the same for a read, and for the
/// domain half of a watch key.
#[test]
fn a_name_past_the_rule_is_refused_naming_the_apps_own_name() {
    let long = "n".repeat(MAX_NAME + 1);
    for e in [write(Some("alpha"), &long).unwrap_err(), read(Some("alpha"), &long).unwrap_err(), read(Some("alpha"), &format!("{long}#00ff")).unwrap_err()] {
        let said = e.to_string();
        assert!(said.contains(&format!("`{long}`")), "the refusal does not name the app's own name: {said}");
        assert!(!said.contains("alpha."), "the refusal names the PREFIXED name, which the app never wrote: {said}");
    }
    assert!(check_name(&"n".repeat(MAX_NAME)).is_ok(), "THE CONTROL: a name at the rule is a name");
    assert!(read(Some("alpha"), &format!("@beta/{long}")).is_err(), "another app's name past the rule was read");
}

/// **The key budget**: the longest stored domain, a parent and an id, is far
/// inside the tree's own bound. Printed, so the margin is a number.
#[test]
fn the_longest_key_an_app_can_make_is_inside_max_key() {
    let stored = write(Some(&"a".repeat(MAX_APP)), &"n".repeat(MAX_NAME)).unwrap();
    let k = record_key(&stored, Loc::under([0xff; 16], [0xff; 16]));
    let max = freenet_prolly::node::MAX_KEY;
    println!("  the longest record key is {} bytes; MAX_KEY is {max}", k.len());
    assert!(k.len() <= max, "a {} byte key is over MAX_KEY {max}", k.len());
}
