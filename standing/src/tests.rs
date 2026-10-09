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
    let by = |n: u8| Attest::new(&key(n), "55qLzrXAazYagArUE", &fy, 3);
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
