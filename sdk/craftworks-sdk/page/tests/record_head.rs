//! EVERY head reader reads the ONE format (signer_proto::head): the page's
//! `HeadRead`, the delegate's `head_of`, and the signer's own read agree on
//! every value, and all are TOLERANT (main's ruling: the last head change an
//! old reader cannot survive).

use contract_keys::register::{head_of, head_state};
use signer_proto::head::{value, Ledger};

fn params() -> Vec<u8> {
    let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
    wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME)
}

fn ledgered(seq: u64, root: [u8; 32]) -> Vec<u8> {
    let prev = signer_proto::Head { seq: seq - 1, root: [1; 32] };
    head_state(&params(), &[7u8; 32], seq, &value(&root, &Ledger { prev: Some(prev), ..Ledger::default() })).expect("signs")
}

#[test]
fn the_page_and_the_delegate_read_every_head_alike() {
    for st in [
        head_state(&params(), &[7u8; 32], 1, &[3; 32]).expect("a bare head"),
        ledgered(2, [250; 32]),
        ledgered(u64::MAX - 1, [0; 32]),
    ] {
        let h = page::HeadRead::from_record(&st).expect("the page reads it");
        assert_eq!(Some((h.seq, h.root())), head_of(&st));
    }
}

/// A value whose ledger carries a field no build knows still yields its root, on
/// every reader.
#[test]
fn an_unknown_trailing_ledger_still_yields_the_root_on_every_reader() {
    let mut v = value(&[9; 32], &Ledger { prev: Some(signer_proto::Head { seq: 4, root: [2; 32] }), ..Ledger::default() });
    v.extend_from_slice(&[250, 2, 0, 7, 7]);
    let st = head_state(&params(), &[7u8; 32], 5, &v).expect("signs");
    let h = page::HeadRead::from_record(&st).expect("the page reads it");
    assert_eq!((h.seq, h.root(), h.prev()), (5, [9; 32], Some((4, [2; 32]))));
    assert_eq!(head_of(&st), Some((5, [9; 32])));
}

/// A malformed ledger is read ROOT-ONLY and gives NO prev (main's ruling (1): it
/// can never classify as "built on mine").
#[test]
fn a_malformed_ledger_reads_root_only_with_no_prev() {
    let mut v = value(&[9; 32], &Ledger { prev: Some(signer_proto::Head { seq: 4, root: [2; 32] }), ..Ledger::default() });
    v.truncate(v.len() - 3);
    let st = head_state(&params(), &[7u8; 32], 5, &v).expect("signs");
    let h = page::HeadRead::from_record(&st).expect("still a head");
    assert_eq!((h.root(), h.prev()), ([9; 32], None));
    assert_eq!(head_of(&st), Some((5, [9; 32])), "the delegate refused a head over its ledger");
}

/// THE ONE-TIME BREAK, named: the reader BEFORE this format demanded a value of
/// exactly 32 bytes, so a build from before it reads every ledgered head as NO
/// HEAD (an empty app). In development: rebuild everything on the key, then write.
#[test]
fn the_old_exact_root_reader_is_what_breaks_once() {
    fn old_head_of(state: &[u8]) -> Option<(u64, [u8; 32])> {
        let (seq, v) = signer_proto::head::record_of(state)?;
        Some((seq, v.try_into().ok()?))
    }
    let bare = head_state(&params(), &[7u8; 32], 1, &[3; 32]).expect("signs");
    assert_eq!(old_head_of(&bare), Some((1, [3; 32])), "an old head still reads on the old rule");
    assert_eq!(old_head_of(&ledgered(2, [4; 32])), None, "the old reader, on a ledgered head: 'no head'");
    assert_eq!(head_of(&ledgered(2, [4; 32])), Some((2, [4; 32])));
}

/// `Page::sign_ledger_of`: PREV from the prev, omitted at the genesis; every
/// head carries race put's §P mark (COMMIT-LIFE §P), the genesis too.
#[test]
fn a_sign_carries_its_prev_and_the_genesis_carries_none() {
    let p = page::Page::new(engine::Params::default(), page::PutPath::Page);
    let g = p.sign_ledger_of(0, [0; 32], 1, [5; 32]);
    let gh = page::HeadRead::from_value(1, &[[5u8; 32].as_slice(), &g].concat()).expect("a head");
    assert_eq!(gh.prev(), None, "the genesis carries a PREV");
    assert!(gh.mark().is_some(), "the genesis head carries no §P mark");
    let l = p.sign_ledger_of(3, [8; 32], 4, [5; 32]);
    let h = page::HeadRead::from_value(4, &[[5u8; 32].as_slice(), &l].concat()).expect("a head");
    assert_eq!(h.prev(), Some((3, [8; 32])));
}

/// THE WITNESS (COMMIT-LIFE ⁵): a head's ledger says how far a page's writes
/// are in it. This page's entry → `Through`; absent from a list that was
/// never at its bound → `NotThere`; absent from a FULL list (an entry may
/// have been evicted) → `Unknown`, never `NotThere` (core dev on docs#27:
/// absent is not below).
#[test]
fn a_heads_witness_of_a_page_is_through_not_there_or_unknown() {
    use signer_proto::head::{value, Ledger, Through, THROUGH_MAX};
    let me = [7u8; 16];
    let other = |i: usize| {
        let mut d = [0u8; 16];
        d[..8].copy_from_slice(&(i as u64 + 1000).to_le_bytes());
        Through { device: d, seq: 1, last: i as u64 + 1 }
    };
    let head = |through: Vec<Through>| page::HeadRead::from_value(5, &value(&[1; 32], &Ledger { through, ..Ledger::default() })).expect("a head");
    let mine = Through { device: me, seq: 42, last: 5 };
    assert_eq!(head(vec![other(0), mine]).witness_of(&me, Some(5)), engine::Witness::Through(42));
    assert_eq!(head(vec![other(0), other(1)]).witness_of(&me, Some(5)), engine::Witness::NotThere, "absent from a list never at its bound");
    let full: Vec<Through> = (0..THROUGH_MAX).map(other).collect();
    let h = head(full);
    assert_eq!(h.through().len(), THROUGH_MAX, "the fixture's list is not at its bound");
    assert_eq!(h.witness_of(&me, Some(1)), engine::Witness::Unknown, "absent from a FULL list whose every entry is at or past our commit was read as not there");
    assert_eq!(h.witness_of(&me, None), engine::Witness::Unknown, "absent from a FULL list with no commit to compare was read as not there");
}

/// **A FULL LIST OF OLDER ENTRIES STILL WITNESSES `NotThere`** (review §3 on
/// sdk#295). A device is minted per page LOAD, so after `THROUGH_MAX` loads
/// the list is full for good; read as `Unknown`, every first commit that dies
/// would end `Unknown`. Eviction takes the lowest `last`: an entry left with
/// `last` below our commit's seq s proves ours was never evicted. Only a list
/// whose every entry is past s -- `THROUGH_MAX` other commits between our cut
/// and our read -- is `Unknown`; a tie at s is too (broken by device id).
#[test]
fn absent_from_a_full_list_of_older_entries_is_not_there() {
    use signer_proto::head::{value, Ledger, Through, THROUGH_MAX};
    let me = [7u8; 16];
    let at = |i: usize, last: u64| {
        let mut d = [0u8; 16];
        d[..8].copy_from_slice(&(i as u64 + 1000).to_le_bytes());
        Through { device: d, seq: 1, last }
    };
    let head = |through: Vec<Through>| page::HeadRead::from_value(500, &value(&[1; 32], &Ledger { through, ..Ledger::default() })).expect("a head");
    // Our commit would have landed at 300; the list's lasts run 200..200+MAX.
    let full = head((0..THROUGH_MAX).map(|i| at(i, 200 + i as u64)).collect());
    assert_eq!(full.through().len(), THROUGH_MAX, "the fixture's list is not at its bound");
    assert_eq!(full.witness_of(&me, Some(300)), engine::Witness::NotThere, "a full list holding entries older than our commit read as Unknown");
    // Every entry past our commit: ours may have been evicted.
    let past = head((0..THROUGH_MAX).map(|i| at(i, 301 + i as u64)).collect());
    assert_eq!(past.witness_of(&me, Some(300)), engine::Witness::Unknown, "every entry past our commit: absent was read as not there");
    // The lowest AT our commit's seq: a tie, broken by device id -- Unknown.
    let tie = head((0..THROUGH_MAX).map(|i| at(i, 300 + i as u64)).collect());
    assert_eq!(tie.witness_of(&me, Some(300)), engine::Witness::Unknown, "a tie at our seq was read as not there");
}
