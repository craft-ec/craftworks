use super::*;
use craftec_register_contract::testing::{keyset, keyset_seeded, World};
use ed25519_dalek::Signer;

/// The writer's side: sign the body that results at `seq`, with the first `k` keys of the keyset.
fn sign(w: &World, seq: u64, hash: [u8; HASH_LEN]) -> Signed {
    let msg = message(&w.params, seq, &hash);
    let (bitmap, sigs) = match &w.auth {
        Authority::One(_) => (0u16, vec![w.signers[0].sign(&msg).to_bytes()]),
        Authority::Quorum { .. } => (
            (0..w.k).fold(0u16, |m, i| m | (1 << i)),
            (0..w.k)
                .map(|i| w.signers[i].sign(&msg).to_bytes())
                .collect(),
        ),
    };
    Signed {
        terminal: false,
        seq,
        value_hash: hash,
        bitmap,
        sigs,
    }
}

/// A writer: its current tail, and the delta that moves it on.
fn step(w: &World, held: &Option<Tail>, ops: Vec<Op>) -> (Vec<u8>, Tail) {
    let seq = held.as_ref().map_or(0, |t| t.seq()) + 1;
    let base = held.as_ref().map(|t| t.body.clone()).unwrap_or_default();
    let body = base.apply(&ops, seq).expect("a legal step");
    let signed = sign(w, seq, body.hash());
    (encode_delta(&w.auth, &signed, &ops), Tail { signed, body })
}

fn set(k: &str, v: &str) -> Op {
    Op::Set {
        key: k.as_bytes().to_vec(),
        value: v.as_bytes().to_vec(),
    }
}

fn host(w: &World, held: &[u8], cands: &[&[u8]]) -> Vec<u8> {
    let mut t = parse_unverified(held, &w.params).expect("held parses");
    for c in cands {
        t = absorb(t, c, &w.params);
    }
    t.map(|t| t.encode(&w.auth)).unwrap_or_default()
}

fn valid(w: &World, s: &[u8]) -> bool {
    read(&w.params_bytes, s).is_some()
}

#[test]
fn an_empty_tail_is_valid_and_a_first_delta_is_adopted() {
    let w = keyset(1, 1, true);
    assert!(valid(&w, &[]));
    let (d, t) = step(&w, &None, vec![set("a", "1")]);
    let s = host(&w, &[], &[&d]);
    assert_eq!(s, t.encode(&w.auth));
    assert!(valid(&w, &s));
    let (_, got) = read(&w.params_bytes, &s).unwrap();
    assert_eq!(got.unwrap().body.get(b"a"), Some(Some(&b"1"[..])));
}

#[test]
fn a_delta_applies_only_as_the_exact_next_seq() {
    let w = keyset(1, 1, true);
    let (d1, t1) = step(&w, &None, vec![set("a", "1")]);
    let (d2, t2) = step(&w, &Some(t1.clone()), vec![set("b", "2")]);
    // Out of order: d2 before d1 is ignored; the host stays empty, then takes d1, then d2.
    assert_eq!(host(&w, &[], &[&d2]), Vec::<u8>::new());
    let s1 = host(&w, &[], &[&d1]);
    assert_eq!(s1, t1.encode(&w.auth));
    let s2 = host(&w, &s1, &[&d2]);
    assert_eq!(s2, t2.encode(&w.auth));
    // A replay of an old delta changes nothing.
    assert_eq!(host(&w, &s2, &[&d1, &d2]), s2);
}

/// The body hash already refuses almost every out-of-order delta (it covers the result). The next-seq rule decides
/// the one case the hash cannot: a delta that SKIPS a number yet produces the same body from the older state, because
/// it overwrote everything the skipped one wrote. `seq` counts deltas, so the host waits for the whole state.
#[test]
fn a_delta_that_skips_a_seq_is_ignored_even_when_its_body_would_match() {
    let w = keyset(1, 1, true);
    let (d1, t1) = step(&w, &None, vec![set("a", "1")]);
    let (_d2, t2) = step(&w, &Some(t1.clone()), vec![set("b", "2")]);
    let (d3, t3) = step(&w, &Some(t2), vec![set("b", "3")]);
    // Precondition: applied to t1 at seq 3, d3's ops give exactly t3's body.
    assert_eq!(t1.body.apply(&[set("b", "3")], 3), Some(t3.body.clone()));
    let s1 = host(&w, &[], &[&d1]);
    assert_eq!(host(&w, &s1, &[&d3]), s1);
}

#[test]
fn a_delta_signed_by_another_key_or_over_another_body_is_ignored() {
    let w = keyset(1, 1, true);
    let other = keyset_seeded(9, 1, 1, true);
    // Same params (so the same contract), a stranger's signature.
    let body = Body::default().apply(&[set("a", "1")], 1).unwrap();
    let forged = encode_delta(&w.auth, &sign(&other, 1, body.hash()), &[set("a", "1")]);
    assert_eq!(host(&w, &[], &[&forged]), Vec::<u8>::new());
    // The writer's signature, but over a body the ops do not produce.
    let wrong = Body::default().apply(&[set("a", "2")], 1).unwrap();
    let lie = encode_delta(&w.auth, &sign(&w, 1, wrong.hash()), &[set("a", "1")]);
    assert_eq!(host(&w, &[], &[&lie]), Vec::<u8>::new());
    // A good candidate in the same batch still lands.
    let (good, t) = step(&w, &None, vec![set("a", "1")]);
    assert_eq!(host(&w, &[], &[&forged, &lie, &good]), t.encode(&w.auth));
}

#[test]
fn a_flush_moves_the_root_and_drops_only_the_covered_rows() {
    let w = keyset(1, 1, true);
    let (d1, t1) = step(&w, &None, vec![set("a", "1")]);
    let (d2, t2) = step(&w, &Some(t1), vec![set("b", "2")]);
    let root = [5u8; 32];
    let (d3, t3) = step(&w, &Some(t2), vec![Op::Flush { root, through: 1 }]);
    let s = host(&w, &[], &[&d1, &d2, &d3]);
    assert_eq!(s, t3.encode(&w.auth));
    let t = read(&w.params_bytes, &s).unwrap().1.unwrap();
    assert_eq!(t.body.root, Some(root));
    assert_eq!(t.body.get(b"a"), None, "in the tree now");
    assert_eq!(t.body.get(b"b"), Some(Some(&b"2"[..])));
}

#[test]
fn a_host_that_missed_deltas_catches_up_from_a_whole_state() {
    let w = keyset(1, 1, true);
    let (d1, t1) = step(&w, &None, vec![set("a", "1")]);
    let (_d2, t2) = step(&w, &Some(t1), vec![set("b", "2")]);
    let (_d3, t3) = step(&w, &Some(t2), vec![set("c", "3")]);
    let behind = host(&w, &[], &[&d1]);
    let ahead = t3.encode(&w.auth);
    // The sync: the ahead host answers the behind host's summary with its whole state.
    let behind_t = parse_unverified(&behind, &w.params).unwrap();
    let ahead_t = parse_unverified(&ahead, &w.params).unwrap();
    assert_ne!(summary_of(&behind_t), summary_of(&ahead_t));
    assert_eq!(host(&w, &behind, &[&ahead]), ahead);
    // And a whole state never moves a host backwards.
    assert_eq!(host(&w, &ahead, &[&behind]), ahead);
}

#[test]
fn two_bodies_at_one_seq_resolve_the_same_way_in_either_order() {
    let w = keyset(1, 1, true);
    let (_, x) = step(&w, &None, vec![set("a", "x")]);
    let (_, y) = step(&w, &None, vec![set("a", "y")]);
    let (xs, ys) = (x.encode(&w.auth), y.encode(&w.auth));
    let xy = host(&w, &xs, &[&ys]);
    let yx = host(&w, &ys, &[&xs]);
    assert_eq!(xy, yx, "the merge is order-independent");
    let lower = if x.signed.value_hash < y.signed.value_hash {
        xs
    } else {
        ys
    };
    assert_eq!(
        xy, lower,
        "the Register's order: the lower body hash wins a tie"
    );
}

#[test]
fn a_two_of_three_keyset_writes_a_tail() {
    let w = keyset(2, 3, false);
    let (d1, t1) = step(&w, &None, vec![set("a", "1")]);
    let s = host(&w, &[], &[&d1]);
    assert_eq!(s, t1.encode(&w.auth));
    assert!(valid(&w, &s));
}

#[test]
fn a_state_is_refused_unless_its_signature_covers_its_body() {
    let w = keyset(1, 1, true);
    let (_, t) = step(&w, &None, vec![set("a", "1")]);
    assert!(valid(&w, &t.encode(&w.auth)));
    // Swap in another body under the same signature.
    let other = Body::default().apply(&[set("a", "2")], 1).unwrap();
    assert!(!valid(&w, &encode_state(&w.auth, &t.signed, &other)));
    // A terminal record is not a tail state (moved-to is not built yet).
    let mut term = t.signed.clone();
    term.terminal = true;
    assert!(!valid(&w, &encode_state(&w.auth, &term, &t.body)));
    // Garbage.
    assert!(!valid(&w, b"TL01"));
    assert!(!valid(&w, b"nonsense"));
}

#[test]
fn a_host_never_ends_on_a_state_that_fails_validation() {
    let w = keyset(1, 1, true);
    let mut tails = vec![None];
    let mut deltas = Vec::new();
    for (i, ops) in [
        vec![set("a", "1")],
        vec![set("b", "2"), Op::Delete { key: b"a".to_vec() }],
        vec![Op::Flush {
            root: [3; 32],
            through: 1,
        }],
        vec![set("c", "3")],
    ]
    .into_iter()
    .enumerate()
    {
        let (d, t) = step(&w, &tails[i], ops);
        deltas.push(d);
        tails.push(Some(t));
    }
    let states: Vec<Vec<u8>> = tails
        .iter()
        .map(|t| t.as_ref().map(|t| t.encode(&w.auth)).unwrap_or_default())
        .collect();
    for s in &states {
        for c in states.iter().chain(deltas.iter()) {
            let out = host(&w, s, &[c]);
            assert!(valid(&w, &out));
            // Never backwards.
            let before = parse_unverified(s, &w.params).unwrap();
            let after = parse_unverified(&out, &w.params).unwrap();
            assert!(rank(&after) >= rank(&before));
        }
    }
}
