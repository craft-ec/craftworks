use super::*;

fn key(n: u8) -> SigningKey {
    SigningKey::from_bytes(&[n; 32])
}
fn id(n: u32) -> [u8; 32] {
    *blake3::hash(&n.to_le_bytes()).as_bytes()
}

#[test]
fn a_location_is_the_cores_own() {
    // Measured live 10-09: fx (127.0.0.1:37691) and fy (:37692) report these.
    assert_eq!(millionths(ring_of_addr(&"127.0.0.1:37691".parse().unwrap())), 596_280);
    assert_eq!(millionths(ring_of_addr(&"127.0.0.1:37692".parse().unwrap())), 487_930);
    // One /24, one place (the core's sybil limit); another /24, another.
    let a = ring_of_addr(&"86.38.75.158:1".parse().unwrap());
    assert_eq!(a, ring_of_addr(&"86.38.75.3:2".parse().unwrap()));
    assert_ne!(a, ring_of_addr(&"86.38.76.158:1".parse().unwrap()));
    // A contract: its id as a fraction of the ring.
    let mut c = [0u8; 32];
    c[0] = 0x80;
    assert_eq!(millionths(ring_of_contract(&c)), 500_000);
}

#[test]
fn a_nodes_arc_runs_midway_to_its_nearest_peers_round_the_ring() {
    let q = 1u64 << 62; // a quarter of the ring
    assert_eq!(arc(2 * q, &[q, 3 * q]), Some((q + q / 2, q)));
    // Round the top: the node at 0.95 with peers at 0.15 and 0.75 holds [0.85, 0.05) — across 0.
    let f = |x: f64| (x * u64::MAX as f64) as u64;
    let a = arc(f(0.95), &[f(0.15), f(0.75)]).unwrap();
    for inside in [0.86, 0.95, 0.999, 0.0, 0.04] {
        assert!(in_arc(f(inside), a), "{inside}");
    }
    for outside in [0.06, 0.15, 0.5, 0.75, 0.84] {
        assert!(!in_arc(f(outside), a), "{outside}");
    }
    assert_eq!(arc(5, &[]), None);
    assert_eq!(arc(5, &[5]), None, "a peer at its own place is no neighbour");
}

#[test]
fn a_place_is_what_enough_other_peers_say() {
    let fy: SocketAddr = "127.0.0.1:37692".parse().unwrap();
    let by = |n: u8| Attest::new(&key(n), "55qLzrXAazYagArUE", &fy, n as u64 * (u64::MAX / 8), 3);
    let ring = ring_of_addr(&fy);
    let all = vec![by(1), by(2), by(3)];
    assert_eq!(located("55qLzrXAazYagArUE", 3, &all, &[], 3), Some(ring));
    assert_eq!(located("55qLzrXAazYagArUE", 3, &all, &[], 4), None, "short of a quorum");
    assert_eq!(located("55qLzrXAazYagArUE", 3, &[by(1), by(1), by(1)], &[], 2), None, "one attester counted once");
    assert_eq!(located("55qLzrXAazYagArUE", 3, &all, &[key(1).verifying_key().to_bytes()], 3), None, "the peer's own key does not vouch");
    assert_eq!(located("55qLzrXAazYagArUE", 4, &all, &[], 1), None, "another epoch");
    let mut forged = by(4);
    forged.ring ^= 1;
    assert_eq!(located("55qLzrXAazYagArUE", 3, &[by(1), forged], &[], 2), None, "a changed place is not signed");
    // Bindings hold only as signed.
    let b = Bind::new(&key(9), "55qLzrXAazYagArUE", 3);
    assert!(b.holds());
    assert!(!Bind { peer: "other".into(), ..b }.holds());
}

#[test]
fn every_entry_proves_under_its_root() {
    for n in [1usize, 2, 3, 5, 8, 13] {
        let ids: Vec<[u8; 32]> = (0..n as u32).map(id).collect();
        let r = root(&ids);
        for i in 0..n {
            assert!(verify(&r, n as u32, i, &ids[i], &path(&ids, i)), "{n} entries, entry {i}");
            assert!(!verify(&r, n as u32, i, &id(999), &path(&ids, i)), "another id");
            if n > 1 {
                assert!(!verify(&r, n as u32, (i + 1) % n, &ids[i], &path(&ids, i)), "another place");
            }
        }
        assert!(!verify(&r, n as u32, n, &ids[0], &path(&ids, 0)), "past the count");
    }
}

#[test]
fn an_answer_holds_only_for_the_challenged_contract_in_the_arc_with_its_state() {
    let k = key(7);
    let whole = (0, u64::MAX);
    let hosted: Vec<([u8; 32], u64)> = (0..50).map(|n| (id(n), 100 + n as u64)).collect();
    let (h, ids) = Holdings::new(&k, "5cCFQqQwLPNKX4zwK", epoch(5000), &hosted, whole);
    assert_eq!((h.count, h.bytes), (50, (0..50).map(|n| 100 + n).sum::<u64>()));
    let state = |i: &[u8; 32]| Some(i.repeat(3));
    let out = [42u8; 32];
    let a = Answer::new(&k, &h, &ids, 5000, &out, &state).unwrap();
    let s = state(&a.id).unwrap();
    assert_eq!(check(&h, &a, &out, whole, Some(&s)), Ok(()));
    assert_eq!(check(&h, &a, &out, whole, None), Ok(()), "everything but the hash, without the state");
    assert_eq!(check(&h, &a, &out, whole, Some(b"another state")), Err("an answer that is not the contract's state"));
    assert_eq!(check(&h, &a, &[43; 32], whole, Some(&s)).is_err(), true, "another beacon output");
    // Another entry than the one asked for, re-signed: refused.
    let j = (a.index as usize + 1) % ids.len();
    let mut other = Answer { index: j as u32, id: ids[j], path: path(&ids, j), ..a.clone() };
    other.sig = k.sign(&other.message()).to_bytes();
    assert_eq!(check(&h, &other, &out, whole, None), Err("an answer to another challenge"));
    // The arc: an answer about a contract outside the node's place.
    let r = ring_of_contract(&a.id);
    let elsewhere = (r.wrapping_add(1), 1 << 40);
    assert_eq!(check(&h, &a, &out, elsewhere, None), Err("a holding outside the node's arc"));
    // Holdings built for an arc leave out what lies outside it.
    let (half, _) = Holdings::new(&k, "5cCFQqQwLPNKX4zwK", epoch(5000), &hosted, (0, 1 << 63));
    assert_eq!(half.count as usize, hosted.iter().filter(|(i, _)| ring_of_contract(i) < 1 << 63).count());
    // Another epoch's holdings; another key's answer.
    assert_eq!(check(&h, &Answer { height: 5000 + EPOCH, ..a.clone() }, &out, whole, None), Err("an answer for other holdings"));
    let mut stolen = a.clone();
    stolen.key = key(8).verifying_key().to_bytes();
    assert_eq!(check(&h, &stolen, &out, whole, None), Err("an answer for other holdings"));
    assert_eq!(Answer::new(&k, &h, &ids, 5000, &out, &|_| None), None, "the state not here: no answer");
}

#[test]
fn the_edges_of_an_arc_a_path_and_an_answer() {
    let q = 1u64 << 62;
    assert_eq!(arc(2 * q, &[2 * q, q, 3 * q]), arc(2 * q, &[q, 3 * q]), "a peer at the node's own place is ignored");
    let a = arc(2 * q, &[q, 3 * q]).unwrap();
    assert!(in_arc(a.0, a) && in_arc(a.0 + a.1 - 1, a) && !in_arc(a.0 + a.1, a), "the arc's end is not in it");
    // A path that claims "no sibling" where there is one is refused (5 entries: entry 0's first sibling exists).
    let ids: Vec<[u8; 32]> = (0..5u32).map(id).collect();
    let mut p = path(&ids, 0);
    p[0] = None;
    assert!(!verify(&root(&ids), 5, 0, &ids[0], &p));
    // No node's answer is another's: the peer is in the hash.
    assert_ne!(answer_hash(b"s", &[1; 32], "5cCFQqQwLPNKX4zwK"), answer_hash(b"s", &[1; 32], "55qLzrXAazYagArUE"));
}

// ---- the record: everything from scratch against the beacon ----

mod record_tests {
    use super::*;
    use crate::record::{holds, merge, prune, weight, Epoch, State, EPOCHS_MAX};

    const P: &str = "5cCFQqQwLPNKX4zwK";
    /// The beacon's output at a height (a fixed stand-in: unpredictable enough for sampling).
    fn out(h: u64) -> [u8; 32] {
        *blake3::hash(&h.to_le_bytes()).as_bytes()
    }
    /// Peer P at the middle of the ring, its three attesters a quarter either side; key 7 bound; holdings over 40
    /// contracts across the whole ring (only those in its arc kept); an answer at each sampled height of epoch 2 up to
    /// `upto`.
    fn world(upto: u64) -> (State, Vec<[u8; 32]>) {
        let e = 2;
        let mid = u64::MAX / 2;
        let addr: SocketAddr = "86.38.75.1:1".parse().unwrap();
        let ring = ring_of_addr(&addr);
        let at = |n: u64| ring.wrapping_add(if n % 2 == 0 { u64::MAX / 4 } else { (u64::MAX / 4).wrapping_neg() });
        let attests: Vec<Attest> = (1..=3u8).map(|n| Attest::new(&key(n), P, &addr, at(n as u64), e)).collect();
        let _ = mid;
        let k = key(7);
        let binds = vec![Bind::new(&k, P, e)];
        let (_, arc_) = place(P, e, &attests, &[k.verifying_key().to_bytes()], 3).unwrap();
        let hosted: Vec<([u8; 32], u64)> = (0..40).map(|n| (id(n), 1000)).collect();
        let (h, ids) = Holdings::new(&k, P, e, &hosted, arc_);
        assert!(h.count > 0, "THE SETUP: something held in the arc");
        let state_of = |i: &[u8; 32]| Some(i.to_vec());
        let answers: Vec<Answer> = (e * EPOCH..=upto).filter(|x| sampled(&out(*x), P)).filter_map(|x| Answer::new(&k, &h, &ids, x, &out(x), &state_of)).collect();
        (State { epochs: vec![Epoch { epoch: e, attests, binds, holdings: vec![h], answers }] }, ids)
    }
    fn beacon(upto: u64) -> impl Fn(u64) -> Option<[u8; 32]> {
        move |h| (h <= upto).then(|| out(h))
    }

    #[test]
    fn a_record_holds_from_scratch_and_reads_back_exactly() {
        let top = 2 * EPOCH + 600;
        let (s, _) = world(top);
        assert!(!s.epochs[0].answers.is_empty(), "THE SETUP: some heights sampled");
        assert_eq!(holds(P, &s, &beacon(top)), Ok(()));
        assert_eq!(crate::codec::decode(&crate::codec::encode(&s)), Some(s.clone()));
        assert_eq!(crate::codec::decode(&[crate::codec::encode(&s), vec![0]].concat()), None, "a trailing byte");
        assert_eq!(holds("another", &s, &beacon(top)), Err("an attestation that does not hold here"));
    }

    #[test]
    fn what_does_not_hold_is_refused() {
        let top = 2 * EPOCH + 600;
        let (s, _) = world(top);
        // An answer at a height the beacon no longer keeps.
        let first = s.epochs[0].answers.iter().map(|a| a.height).min().unwrap();
        assert_eq!(holds(P, &s, &|h| (h > first).then(|| out(h))), Err("an answer outside the beacon's window"));
        // An answer at a height not sampled for this peer.
        let mut bad = s.clone();
        let a = bad.epochs[0].answers[0].clone();
        let h = bad.epochs[0].holdings[0].clone();
        let unsampled = (2 * EPOCH..top).find(|x| !sampled(&out(*x), P)).unwrap();
        let mut moved = Answer { height: unsampled, ..a.clone() };
        moved.sig = key(7).sign(&moved.message()).to_bytes();
        bad.epochs[0].answers = vec![moved];
        assert_eq!(holds(P, &bad, &beacon(top)), Err("an answer at a height not sampled for this peer"));
        // Holdings by a key not bound to the peer.
        let mut unbound = s.clone();
        unbound.epochs[0].binds.clear();
        assert_eq!(holds(P, &unbound, &beacon(top)), Err("holdings not by a key bound to the peer"));
        // Short of a quorum of attesters: no place, so no answer counts.
        let mut alone = s.clone();
        alone.epochs[0].attests.truncate(2);
        assert_eq!(holds(P, &alone, &beacon(top)), Err("answers with no place for the peer"));
        // The peer's own bound key cannot be one of its attesters.
        let mut selfish = s.clone();
        let addr: SocketAddr = "86.38.75.1:1".parse().unwrap();
        selfish.epochs[0].attests[2] = Attest::new(&key(7), P, &addr, 1, 2);
        assert_eq!(holds(P, &selfish, &beacon(top)), Err("answers with no place for the peer"));
        let _ = h;
        // An answer for another entry than the one the beacon asked for, re-signed by the bound key: refused by its check.
        let mut wrong = s.clone();
        let ids = world(top).1;
        let a0 = wrong.epochs[0].answers[0].clone();
        let j = (a0.index as usize + 1) % ids.len();
        let mut other = Answer { index: j as u32, id: ids[j], path: path(&ids, j), ..a0 };
        other.sig = key(7).sign(&other.message()).to_bytes();
        wrong.epochs[0].answers[0] = other;
        assert_eq!(holds(P, &wrong, &beacon(top)), Err("an answer to another challenge"));
        // Epochs out of order; an attestation of another epoch inside one.
        let two = State { epochs: vec![Epoch { epoch: 3, ..Default::default() }, Epoch { epoch: 2, ..Default::default() }] };
        assert_eq!(holds(P, &two, &beacon(top)), Err("epochs out of order"));
        let mut stray = s.clone();
        stray.epochs[0].attests.push(Attest::new(&key(9), P, &addr, 1, 3));
        assert_eq!(holds(P, &stray, &beacon(top)), Err("an attestation that does not hold here"));
    }

    #[test]
    fn standing_is_the_bytes_held_times_the_samples_answered() {
        let top = 2 * EPOCH + 600;
        let (s, _) = world(top);
        let h = &s.epochs[0].holdings[0];
        let due = (2 * EPOCH..=top).filter(|x| sampled(&out(*x), P)).count() as u128;
        assert_eq!(weight(P, &s, &beacon(top), top), h.bytes as u128 * s.epochs[0].answers.len() as u128 / due);
        assert_eq!(s.epochs[0].answers.len() as u128, due, "every sampled height answered: all of its bytes");
        // Half the answers: about half the standing.
        let mut half = s.clone();
        let n = half.epochs[0].answers.len() / 2;
        half.epochs[0].answers.truncate(n);
        assert_eq!(weight(P, &half, &beacon(top), top), h.bytes as u128 * n as u128 / due);
        assert_eq!(weight(P, &State::default(), &beacon(top), top), 0);
    }

    #[test]
    fn merge_unions_and_keeps_the_newest_epochs_and_prune_drops_the_old() {
        let top = 2 * EPOCH + 600;
        let (s, _) = world(top);
        let mut a = s.clone();
        let mut b = s.clone();
        let n = a.epochs[0].answers.len();
        a.epochs[0].answers.truncate(n / 2);
        b.epochs[0].answers.drain(..n / 2);
        assert_eq!(merge(&a, &b).epochs[0].answers.len(), n);
        assert_eq!(merge(&a, &b), merge(&b, &a), "the same bytes either way");
        let many = State { epochs: (0..9).map(|e| Epoch { epoch: e, ..Default::default() }).collect() };
        assert_eq!(merge(&many, &State::default()).epochs.len(), EPOCHS_MAX);
        assert_eq!(merge(&many, &State::default()).epochs[0].epoch, 9 - EPOCHS_MAX as u64);
        assert_eq!(prune(&many, 5 * EPOCH).epochs.iter().map(|e| e.epoch).collect::<Vec<_>>(), vec![5, 6, 7, 8]);
    }

}
