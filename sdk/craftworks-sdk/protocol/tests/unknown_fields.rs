//! §19: a build must not delete what a newer build wrote.
//!
//! The hard case is not READING a newer record — it is writing one back. An
//! app built against v1 opens a record a v2 app created, changes one field,
//! and saves. If the v1 encoder emits only the fields it knows, the v2 fields
//! are gone and nothing reports an error: the v2 app finds its data missing,
//! later, with nothing to say what removed it.

use protocol::record::{Record, RecordError};

/// What "this build" knows about. Everything else is a stranger's.
const KNOWN: &[u32] = &[1, 2];

fn field(tag: u32, bytes: &[u8]) -> Vec<u8> {
    let mut out = tag.to_le_bytes().to_vec();
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
    out
}

/// A record from a NEWER schema: fields this build has never heard of,
/// before, between and after the ones it has.
fn from_the_future() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&field(7, b"a v2 field, first"));
    b.extend_from_slice(&field(1, b"title"));
    b.extend_from_slice(&field(9, b"a v2 field, in the middle"));
    b.extend_from_slice(&field(2, b"body"));
    b.extend_from_slice(&field(42, &[0xFFu8; 300]));
    b
}

/// Rewriting a field this build owns keeps every field it does not, byte for
/// byte and in place.
#[test]
fn a_rewrite_preserves_unknown_fields_exactly_and_in_position() {
    let original = from_the_future();
    let mut r = Record::decode(&original).expect("a well-formed record");
    assert_eq!(
        r.unknown(KNOWN),
        vec![7, 9, 42],
        "the fixture does not actually carry unknown fields, so this proves \
         nothing"
    );

    // Change only what this build owns.
    r.set(1, b"a new title".to_vec());
    let rewritten = r.encode();
    let back = Record::decode(&rewritten).expect("the rewrite must decode");

    assert_eq!(back.get(1), Some(&b"a new title"[..]), "the edit was lost");
    for (tag, expect) in [
        (7u32, &b"a v2 field, first"[..]),
        (9, &b"a v2 field, in the middle"[..]),
        (2, &b"body"[..]),
    ] {
        assert_eq!(
            back.get(tag),
            Some(expect),
            "field {tag} did not survive a rewrite by a build that does not \
             understand it"
        );
    }
    assert_eq!(
        back.get(42),
        Some(&[0xFFu8; 300][..]),
        "the largest unknown field was truncated or dropped"
    );

    // ORDER too. A reorder changes the bytes of a record whose content did
    // not change — and a record is addressed by its bytes, so that is a
    // different block and every reader holding the old id loses it.
    assert_eq!(
        back.tags(),
        vec![7, 1, 9, 2, 42],
        "the fields came back in a different order, which is a different \
         record id for the same content"
    );
}

/// Touching NOTHING is byte-for-byte identical.
///
/// The sharpest form of the promise: decode and re-encode must be the
/// identity, or merely opening a record changes it.
#[test]
fn decoding_and_re_encoding_changes_nothing() {
    let original = from_the_future();
    let r = Record::decode(&original).expect("decodes");
    assert_eq!(
        r.encode(),
        original,
        "opening a record and saving it unchanged produced DIFFERENT bytes, \
         so every reader holding its id has lost it"
    );
}

/// The control: a rewrite that drops unknown fields is DETECTED by the test
/// above.
///
/// Without this, "the fields survived" would read identically if the fixture
/// had no unknown fields, or if `unknown()` always returned empty. This
/// builds the record a naive v1 encoder would write — its own fields only —
/// and asserts it differs.
#[test]
fn a_naive_rewrite_that_drops_unknown_fields_is_visibly_different() {
    let original = from_the_future();
    let full = Record::decode(&original).expect("decodes");

    // What a build that serialised only its own struct would emit.
    let mut naive = Record::new();
    for t in KNOWN {
        if let Some(v) = full.get(*t) {
            naive.set(*t, v.to_vec());
        }
    }
    let naive_bytes = naive.encode();

    assert_ne!(
        naive_bytes,
        full.encode(),
        "dropping every unknown field produced the SAME bytes as keeping \
         them, so this test cannot tell the two apart"
    );
    assert!(
        naive_bytes.len() < original.len(),
        "the naive rewrite is not smaller, so it did not drop anything"
    );
    assert_eq!(
        Record::decode(&naive_bytes)
            .expect("decodes")
            .unknown(KNOWN),
        Vec::<u32>::new(),
        "the naive rewrite still carries unknown fields"
    );
}

/// A record that is not well-formed is REFUSED, not half-read.
#[test]
fn a_damaged_record_is_refused() {
    let original = from_the_future();
    // Truncations.
    let mut refused = 0usize;
    for n in 1..original.len() {
        if Record::decode(&original[..n]).is_err() {
            refused += 1;
        }
    }
    assert!(
        refused > original.len() / 2,
        "only {refused} of {} truncations were refused; a decoder that reads \
         a prefix as a whole record silently loses the tail",
        original.len() - 1
    );

    // A length that claims more than is there.
    let mut lying = field(1, b"short");
    lying[4] = 0xFF;
    assert_eq!(
        Record::decode(&lying),
        Err(RecordError::BadLength),
        "a field claiming more bytes than the record holds was accepted"
    );

    // Two fields with one tag: "the" value of it has no single answer.
    let mut twice = field(1, b"one");
    twice.extend_from_slice(&field(1, b"two"));
    assert_eq!(Record::decode(&twice), Err(RecordError::DuplicateTag(1)));
}
