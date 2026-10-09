//! A domain's key range: the one place a name becomes bytes.
//!
//! `preload` used to take `[[lo, hi], …]` raw key strings, so a caller had to
//! know that a domain's records live under a particular prefix and where that
//! range ends. That is the one thing this boundary exists to hide — and a
//! layout hard-coded into apps could never change afterwards without breaking
//! every one of them.

use craftworks_sdk::{Db, SystemEnv};
use testkit::MemStore;

/// The key layout is `Db`'s, whatever store it reads through.
type D = Db<MemStore, SystemEnv>;

/// A domain's range contains its own records and stops before the next
/// domain's.
#[test]
fn a_domains_range_holds_its_records_and_nothing_else() {
    let (lo, hi) = D::domain_range("tasks");
    assert!(lo < hi, "an empty range asks for nothing");

    let id = [7u8; 16];
    let key = craftworks_sdk::db::record_key("tasks", id);
    assert!(
        key >= lo && key < hi,
        "a record of the domain falls outside the range that is supposed to load it"
    );

    // THE CONTROL: another domain's record does NOT fall in it. Without this,
    // a `domain_range` returning the whole key space would pass the assertion
    // above — and would make every preload fetch the entire database.
    let other = craftworks_sdk::db::record_key("notes", id);
    assert!(
        other < lo || other >= hi,
        "another domain's records fall inside this domain's range"
    );
}

/// Two domains have two ranges. A constant would make preload load the same
/// thing however many domains it was given.
#[test]
fn different_domains_have_different_ranges() {
    assert_ne!(D::domain_range("tasks"), D::domain_range("notes"));
}

/// A domain whose name is a PREFIX of another's does not swallow it.
///
/// `task` and `tasks` are different domains. A range built by prefix without
/// a separator would make preloading `task` fetch every record of `tasks`
/// too — and, worse, make a scan of `task` report them as its own.
#[test]
fn a_domain_is_not_swallowed_by_one_whose_name_extends_it() {
    let (lo, hi) = D::domain_range("task");
    let inner = craftworks_sdk::db::record_key("tasks", [1u8; 16]);
    assert!(
        inner < lo || inner >= hi,
        "records of `tasks` fall inside the range of `task`"
    );
}

/// The watch-key layout round-trips (a LIVE binding's key names the range its
/// `RenderedAt` diff covers), and a string that is not a key names nothing.
#[test]
fn a_watch_key_names_its_range_and_nothing_else_does() {
    assert_eq!(D::watch_range("note"), Some(D::domain_range("note")));
    let k = D::watch_key("note", Some(&[7u8; 16]));
    assert_eq!(D::watch_range(&k), Some(D::parent_range("note", &[7u8; 16])));
    for bad in ["", "No", "note#", "note#xyz", "note#0123", "a#b#c"] {
        assert_eq!(D::watch_range(bad), None, "{bad:?}");
    }
}
