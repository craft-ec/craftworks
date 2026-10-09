//! The head value's ONE format (signer_proto::head): what every reader reads and every writer writes.

use signer_proto::head::*;
use signer_proto::Head;

const ROOT: [u8; 32] = [0xAB; 32];

fn dev(n: u8) -> [u8; 16] {
    [n; 16]
}

fn full() -> Ledger {
    Ledger {
        prev: Some(Head { seq: 41, root: [7; 32] }),
        parity: Some(b"k/0042".to_vec()),
        through: vec![Through { device: dev(2), seq: 40, last: 41 }, Through { device: dev(1), seq: 12, last: 30 }],
    }
}

#[test]
fn an_empty_ledger_is_the_bare_root_and_reads_back_as_it_always_did() {
    let v = value(&ROOT, &Ledger::default());
    assert_eq!(v, ROOT.to_vec(), "an empty ledger must be the pre-format head, byte for byte");
    let h = read_value(&v).expect("a head");
    assert_eq!((h.root, h.ledger.is_empty(), h.refused), (ROOT, true, false));
    assert_eq!(check(&v), Ok(()));
}

#[test]
fn a_full_ledger_round_trips_normalised() {
    let v = value(&ROOT, &full());
    assert_eq!(&v[..32], &ROOT, "the root is FIRST");
    let h = read_value(&v).expect("a head");
    assert!(!h.refused);
    let mut want = full();
    want.through.sort_by_key(|t| t.device);
    assert_eq!(h.ledger, want);
    assert_eq!(check(&v), Ok(()));
}

#[test]
fn prev_is_omitted_at_the_genesis_not_zeros() {
    let v = value(&ROOT, &Ledger { parity: Some(b"p".to_vec()), ..Ledger::default() });
    assert!(read_value(&v).expect("a head").ledger.prev.is_none());
    assert!(!v[33..].windows(1).next().is_some_and(|t| t[0] == TAG_PREV), "a PREV field was written at the genesis");
}

/// main's ruling (3): from this format on EVERY reader is tolerant. A value whose ledger carries a tag this build does
/// not know still yields its root AND the fields it does know.
#[test]
fn an_unknown_trailing_field_is_skipped_and_the_root_and_known_fields_still_read() {
    let mut v = value(&ROOT, &Ledger { prev: Some(Head { seq: 3, root: [9; 32] }), ..Ledger::default() });
    v.extend_from_slice(&[200, 4, 0, 1, 2, 3, 4]); // tag 200, 4 bytes
    let h = read_value(&v).expect("a head");
    assert_eq!((h.root, h.refused, h.ledger.prev.map(|p| p.seq)), (ROOT, false, Some(3)));
    assert_eq!(check(&v), Ok(()), "the signer refuses a field only for its shape, not for being new");
}

#[test]
fn an_unknown_version_reads_root_only_never_no_head() {
    let mut v = ROOT.to_vec();
    // 3: the first version this build does not know (2 is race put's).
    v.extend_from_slice(&[3, TAG_PREV, 40, 0]);
    v.extend_from_slice(&[0; 40]);
    let h = read_value(&v).expect("a head, never 'no head'");
    assert_eq!((h.root, h.refused, h.ledger.is_empty()), (ROOT, true, true));
    assert_eq!(check(&v), Err(Malformed::Version(3)));
}

/// main's ruling (1): a refused ledger is NO prev, so the merge's conservative cell, never "built on mine". A prev
/// that WAS in the bytes before the fault must not survive into the reading.
#[test]
fn a_malformed_ledger_never_yields_a_prev_whatever_came_before_the_fault() {
    let good = value(&ROOT, &Ledger { prev: Some(Head { seq: 5, root: [5; 32] }), ..Ledger::default() });
    let mut cases: Vec<(Vec<u8>, Malformed)> = Vec::new();
    // truncated mid-field
    cases.push((good[..good.len() - 1].to_vec(), Malformed::Truncated));
    // a known field of the wrong shape after a good prev
    let mut bad = good.clone();
    bad.extend_from_slice(&[TAG_THROUGH, 3, 0, 1, 0, 9]);
    cases.push((bad, Malformed::ThroughShape));
    // tags out of order: prev after parity
    let mut v = ROOT.to_vec();
    v.extend_from_slice(&[LEDGER_VERSION, TAG_PARITY, 1, 0, b'x', TAG_PREV, 40, 0]);
    v.extend_from_slice(&[1; 40]);
    cases.push((v, Malformed::OutOfOrder));
    // a duplicated tag
    let mut v = good.clone();
    v.extend_from_slice(&[TAG_PREV, 40, 0]);
    v.extend_from_slice(&[1; 40]);
    cases.push((v, Malformed::OutOfOrder));
    // an unsorted device list
    let mut v = good.clone();
    let mut e = vec![TAG_THROUGH, 66, 0, 2, 0];
    for d in [3u8, 1] {
        e.extend_from_slice(&[d; 16]);
        e.extend_from_slice(&[0; 16]);
    }
    v.extend_from_slice(&e);
    cases.push((v, Malformed::ThroughUnsorted));
    for (v, why) in cases {
        let h = read_value(&v).expect("a head");
        assert!(h.refused, "{why:?} was read as a ledger");
        assert_eq!(h.root, ROOT, "{why:?} lost the root");
        assert!(h.ledger.prev.is_none(), "{why:?}: a refused ledger still gave a prev — 'built on mine' from bytes nobody can vouch for");
        assert_eq!(check(&v), Err(why));
    }
}

#[test]
fn shorter_than_a_root_is_not_a_head() {
    assert!(read_value(&[1; 31]).is_none());
    assert_eq!(check(&[1; 31]), Err(Malformed::ShortRoot));
}

/// The bound, derived and checked: a ledger with EVERY field at its largest fits the Register's value cap.
#[test]
fn the_device_bound_is_the_one_where_a_full_ledger_still_fits() {
    assert_eq!(THROUGH_MAX, 117);
    let l = Ledger {
        prev: Some(Head { seq: 1, root: [1; 32] }),
        parity: Some(vec![b'z'; PARITY_MAX]),
        through: (0..THROUGH_MAX as u16).map(|i| Through { device: { let mut d = [0; 16]; d[..2].copy_from_slice(&i.to_be_bytes()); d }, seq: 1, last: 1 }).collect(),
    };
    let v = value(&ROOT, &l);
    assert!(v.len() <= VALUE_MAX, "{} > {VALUE_MAX}", v.len());
    assert_eq!(check(&v), Ok(()));
}

/// Past the bound: EVICT the least recently written, deterministically, never refuse. And a READER takes a longer
/// list than this build writes (a newer writer's), which only the signer's check holds to the bound.
#[test]
fn past_the_bound_the_least_recently_written_is_evicted_and_a_longer_list_is_still_read() {
    let mut through: Vec<Through> = (0..=THROUGH_MAX as u16)
        .map(|i| Through { device: { let mut d = [0; 16]; d[..2].copy_from_slice(&i.to_be_bytes()); d }, seq: 10, last: 100 + i as u64 })
        .collect();
    // Two equally old: the lower device id goes.
    through[5].last = 1;
    through[9].last = 1;
    let l = Ledger { through: through.clone(), ..Ledger::default() };
    let h = read_value(&value(&ROOT, &l)).expect("a head");
    assert_eq!(h.ledger.through.len(), THROUGH_MAX);
    assert!(!h.ledger.through.iter().any(|t| t.device == through[5].device), "the least recently written was not evicted");
    assert!(h.ledger.through.iter().any(|t| t.device == through[9].device), "the tie was not broken by the lower device id");
    // A longer list, written by hand: read, but not signable by this build.
    let mut v = ROOT.to_vec();
    let n = THROUGH_MAX + 1;
    let mut body = (n as u16).to_le_bytes().to_vec();
    for t in &through {
        body.extend_from_slice(&t.device);
        body.extend_from_slice(&t.seq.to_le_bytes());
        body.extend_from_slice(&t.last.to_le_bytes());
    }
    v.push(LEDGER_VERSION);
    v.push(TAG_THROUGH);
    v.extend_from_slice(&(body.len() as u16).to_le_bytes());
    v.extend_from_slice(&body);
    let h = read_value(&v).expect("a head");
    assert_eq!((h.refused, h.ledger.through.len()), (false, n), "a reader refused a longer list");
    assert_eq!(check(&v), Err(Malformed::ThroughTooMany));
}

/// LEDGERS MERGE: per device the max, the parity mark further on, the winner's prev.
#[test]
fn ledgers_merge_per_device_max_the_further_parity_and_the_winners_prev() {
    let w = Ledger {
        prev: Some(Head { seq: 9, root: [9; 32] }),
        parity: Some(b"k/10".to_vec()),
        through: vec![Through { device: dev(1), seq: 9, last: 9 }, Through { device: dev(2), seq: 3, last: 4 }],
    };
    let l = Ledger {
        prev: Some(Head { seq: 9, root: [8; 32] }),
        parity: Some(b"k/20".to_vec()),
        through: vec![Through { device: dev(2), seq: 8, last: 9 }, Through { device: dev(3), seq: 2, last: 2 }],
    };
    let m = merge(&w, &l);
    assert_eq!(m.prev, w.prev);
    assert_eq!(m.parity.as_deref(), Some(&b"k/20"[..]));
    assert_eq!(
        m.through,
        vec![
            Through { device: dev(1), seq: 9, last: 9 },
            Through { device: dev(2), seq: 8, last: 9 },
            Through { device: dev(3), seq: 2, last: 2 },
        ],
        "a loser's device entry vanished or was not the max"
    );
    // The per-device part does not depend on who won.
    assert_eq!(merge(&l, &w).through, m.through);
}

#[test]
fn record_of_reads_the_registers_framing() {
    let mut st = b"RG01".to_vec();
    st.extend_from_slice(&[0b01, 0]);
    st.extend_from_slice(&7u64.to_le_bytes());
    st.extend_from_slice(&32u16.to_le_bytes());
    st.extend_from_slice(&ROOT);
    st.extend_from_slice(&[0; 64]);
    let (seq, v) = record_of(&st).expect("a record");
    assert_eq!((seq, v), (7, &ROOT[..]));
    assert!(record_of(b"RG02").is_none());
}

/// LEDGER VERSION 2 (race put, COMMIT-LIFE §P): `TAG_PARITY`'s meaning changed
/// to the §P mark, so the version bumped. This build WRITES v2; a reader still
/// READS v1 but drops its parity field (#119's scan front, not the mark); the
/// SIGNER's strict check signs only v2.
#[test]
fn ledger_v2_writes_the_mark_and_a_v1_parity_field_is_dropped_on_read() {
    let v = value(&ROOT, &Ledger { parity: Some(Vec::new()), ..Ledger::default() });
    assert_eq!(v[32], LEDGER_VERSION);
    assert_eq!(LEDGER_VERSION, 2);
    assert_eq!(read_value(&v).expect("a head").ledger.parity, Some(Vec::new()), "the mark does not read back");
    assert!(check(&v).is_ok());

    let mut v1 = ROOT.to_vec();
    v1.extend_from_slice(&[LEDGER_VERSION_V1, TAG_PARITY, 1, 0, b'x']);
    let h = read_value(&v1).expect("a head");
    assert!(!h.refused, "a v1 ledger is still READ");
    assert_eq!(h.ledger.parity, None, "a v1 parity field (the scan front) was taken for the §P mark");
    assert_eq!(check(&v1), Err(Malformed::Version(LEDGER_VERSION_V1)), "the signer would sign a v1 ledger");
}
