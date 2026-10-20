use super::*;

const A: &[u8] = &[0xA; 16];
const B: &[u8] = &[0xB; 16];

/// Two editors from the maker's first change, editing at once: every order of their rows, duplicates too, gives one
/// document — the same text, the same marks.
#[test]
fn every_order_of_the_rows_and_duplicates_give_one_document() {
    let (mut a, first) = Doc::create(A).unwrap();
    let mut b = Doc::load(B, &[&first]).unwrap();
    let a1 = a.splice(0, 0, "Hello world").unwrap();
    b.apply(&[&a1]).unwrap();
    // At once: A makes "Hello" bold and appends; B inserts in the middle.
    let a2 = a.mark(0, 5, "bold", None, true).unwrap();
    let a3 = a.splice(11, 0, "!").unwrap();
    let b1 = b.splice(5, 0, ", dear").unwrap();
    let rows: Vec<&[u8]> = vec![&first, &a1, &a2, &a3, &b1];
    let mut seen = None;
    for order in [vec![0, 1, 2, 3, 4], vec![4, 3, 2, 1, 0], vec![2, 4, 0, 3, 1, 1, 4]] {
        let mut d = Doc::load(&[0xC; 16], &order.iter().map(|&i| rows[i]).collect::<Vec<_>>()).unwrap();
        let got = (d.text().unwrap(), d.marks().unwrap(), d.heads());
        assert_eq!(got.0, "Hello, dear world!");
        assert_eq!(got.1, vec![Span { start: 0, end: 5, name: "bold".into(), value: None }]);
        if let Some(s) = &seen {
            assert_eq!(&got, s, "every order the same");
        }
        seen = Some(got);
    }
}

/// A row ahead of what it builds on (B's edit before the maker's first change) waits, then counts once its base is in.
#[test]
fn a_row_ahead_of_its_base_counts_once_its_base_is_read() {
    let (mut a, first) = Doc::create(A).unwrap();
    let a1 = a.splice(0, 0, "abc").unwrap();
    let mut d = Doc::load(B, &[&a1]).unwrap();
    assert!(d.text().is_err(), "no text before the maker's first change");
    assert!(d.apply(&[&first]).unwrap());
    assert_eq!(d.text().unwrap(), "abc");
}

/// A snapshot loads as the rows did, and rows written after it apply on top.
#[test]
fn a_snapshot_then_later_rows() {
    let (mut a, first) = Doc::create(A).unwrap();
    let a1 = a.splice(0, 0, "one").unwrap();
    let snap = a.save();
    let a2 = a.splice(3, 0, " two").unwrap();
    let d = Doc::load(B, &[&snap, &a2, &first, &a1]).unwrap();
    assert_eq!(d.text().unwrap(), "one two");
}

/// Positions are UTF-16 units (a page's text field): an emoji is two.
#[test]
fn positions_count_utf16_units() {
    let (mut a, _) = Doc::create(A).unwrap();
    a.splice(0, 0, "😀x").unwrap();
    a.splice(2, 0, "-").unwrap();
    assert_eq!(a.text().unwrap(), "😀-x");
}

/// Changes written together (a page batches a burst of keystrokes into ONE row): their bytes concatenated load as each
/// one alone would.
#[test]
fn concatenated_changes_load_as_one_row() {
    let (mut a, first) = Doc::create(A).unwrap();
    let mut row = Vec::new();
    for (i, c) in "typed".chars().enumerate() {
        row.extend(a.splice(i, 0, &c.to_string()).unwrap());
    }
    let d = Doc::load(B, &[&first, &row]).unwrap();
    assert_eq!(d.text().unwrap(), "typed");
}

/// A mark EXPANDS AFTER (what a page's own copy assumes — `doc-editor`'s `shifted`): text put at its end joins it, at
/// its start does not.
#[test]
fn a_mark_grows_at_its_end_not_at_its_start() {
    let (mut a, _) = Doc::create(A).unwrap();
    a.splice(0, 0, "abcdef").unwrap();
    a.mark(2, 5, "bold", None, true).unwrap();
    a.splice(5, 0, "X").unwrap();
    assert_eq!(a.marks().unwrap()[0].end, 6, "at its end: joins it");
    a.splice(2, 0, "Y").unwrap();
    let m = &a.marks().unwrap()[0];
    assert_eq!((m.start, m.end), (3, 7), "at its start: shifted, not joined");
}

/// CELLS: each key's last write wins, the same in every order; a cleared key is gone; two editors setting different
/// keys at once keep both.
#[test]
fn cells_merge_per_key() {
    let (mut a, first) = Doc::create(A).unwrap();
    let mut b = Doc::load(B, &[&first]).unwrap();
    let a1 = a.set("c:A1", Some("1")).unwrap();
    let b1 = b.set("c:B1", Some("=A1*2")).unwrap();
    let a2 = a.set("c:A1", Some("5")).unwrap();
    let a3 = a.set("c:C1", Some("gone")).unwrap();
    let a4 = a.set("c:C1", None).unwrap();
    let rows: Vec<&[u8]> = vec![&first, &a1, &b1, &a2, &a3, &a4];
    for order in [vec![0, 1, 2, 3, 4, 5], vec![5, 4, 3, 2, 1, 0]] {
        let d = Doc::load(&[0xC; 16], &order.iter().map(|&i| rows[i]).collect::<Vec<_>>()).unwrap();
        assert_eq!(d.cells().unwrap(), vec![("c:A1".into(), "5".into()), ("c:B1".into(), "=A1*2".into())]);
    }
}
