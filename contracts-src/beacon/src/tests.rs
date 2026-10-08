use super::*;

fn p() -> Params {
    Params { k: 64, segs: 8, sample: 2, window: 2, keep: 5, seed: [3; 32] }
}

fn chain(n: usize) -> State {
    let mut s = State::default();
    for _ in 0..n {
        s = s.extend(&p());
    }
    s
}

/// A FAKE next step that passes the contract's sampled check: point `j` forged, the root re-made until the sample
/// misses both segments it touches — what a cheater does. `j` the last point: the OUTPUT is forged too.
fn fake_at(s: &State, j: usize) -> State {
    let p = p();
    let honest = Step::compute(&p, s.out(&p), s.height() + 1);
    for t in 0u32.. {
        let mut st = honest.clone();
        let pts = st.points.as_mut().unwrap();
        pts[j] = *blake3::hash(&t.to_le_bytes()).as_bytes();
        st.root = root(pts);
        st.out = *pts.last().unwrap();
        let picked = samples(&p, &st.root, &st.prev, st.height);
        if !picked.contains(&j) && !picked.contains(&(j + 1)) {
            let mut f = s.clone();
            f.steps.push(st);
            return normalize(&p, f);
        }
    }
    unreachable!()
}
fn fake_next(s: &State) -> (State, usize) {
    (fake_at(s, 3), 3)
}

#[test]
fn honest_steps_are_well_formed_and_carry_points_only_in_the_window() {
    let s = chain(4);
    assert_eq!(check(&p(), &s), Ok(()));
    assert_eq!(s.height(), 4);
    let with: Vec<bool> = s.steps.iter().map(|x| x.points.is_some()).collect();
    assert_eq!(with, vec![false, false, true, true]);
}

#[test]
fn two_extenders_compute_the_same_step() {
    let a = chain(2).extend(&p());
    let b = chain(2).extend(&p());
    assert_eq!(a, b);
}

#[test]
fn it_keeps_only_its_newest_steps_and_a_cut_chain_still_checks() {
    let s = chain(7);
    assert_eq!(s.steps.len(), 5);
    assert_eq!(s.steps[0].height, 3);
    assert_eq!(check(&p(), &s), Ok(()));
}

#[test]
fn a_sampled_segment_that_does_not_compute_is_refused() {
    // Forge EVERY point: whatever the sample picks is wrong.
    let s = chain(2);
    let mut st = Step::compute(&p(), s.out(&p()), 3);
    let pts: Vec<[u8; 32]> = (0..8u8).map(|i| [i; 32]).collect();
    st.root = root(&pts);
    st.out = pts[7];
    st.points = Some(pts);
    let mut f = s.clone();
    f.steps.push(st);
    assert_eq!(check(&p(), &normalize(&p(), f)), Err("a sampled segment that does not compute"));
}

#[test]
fn the_sample_is_only_a_speed_bump_and_a_fraud_proof_is_the_safety() {
    let s = chain(2);
    let (fake, j) = fake_next(&s);
    // The cheater's step passes the contract's own check…
    assert_eq!(check(&p(), &fake), Ok(()), "a ground root slips past the sample");
    // …but anyone recomputing it finds the forged segment and proves it.
    let top = fake.steps.last().unwrap();
    let fr = top.find_fraud(&p()).expect("a verifier finds it");
    assert!(fr.j as usize == j || fr.j as usize == j + 1);
    assert!(fr.holds(&p()));
    let mut proven = fake.clone();
    proven.frauds.push(fr.clone());
    assert_eq!(check(&p(), &proven), Err("a step proven fraudulent"));
    // An honest step has no fraud to find.
    assert!(chain(3).steps.last().unwrap().find_fraud(&p()).is_none());
}

#[test]
fn a_fraud_proof_of_an_honest_segment_proves_nothing() {
    let s = chain(3);
    let top = s.steps.last().unwrap();
    let not_fraud = Fraud::of(top, 2);
    assert!(!not_fraud.holds(&p()));
    let mut x = s.clone();
    x.frauds.push(not_fraud);
    assert_eq!(check(&p(), &x), Err("a fraud proof that does not prove fraud"));
}

fn all_orders(copies: &[State]) -> Vec<State> {
    fn go(k: usize, idx: &mut Vec<usize>, c: &[State], out: &mut Vec<State>) {
        if k == idx.len() {
            let mut m = c[idx[0]].clone();
            for &i in &idx[1..] {
                m = merge(&p(), &m, &c[i]);
            }
            out.push(m);
            return;
        }
        for i in k..idx.len() {
            idx.swap(k, i);
            go(k + 1, idx, c, out);
            idx.swap(k, i);
        }
    }
    let mut idx: Vec<usize> = (0..copies.len()).collect();
    let mut out = Vec::new();
    go(0, &mut idx, copies, &mut out);
    out
}

#[test]
fn the_longer_honest_copy_wins_in_every_order() {
    let r = all_orders(&[chain(2), chain(5), chain(3)]);
    assert!(r.windows(2).all(|w| w[0] == w[1]));
    assert_eq!(r[0], chain(5));
}

#[test]
fn once_proven_a_fake_is_gone_in_every_order_and_stays_gone() {
    let s = chain(2);
    // The fake forges its OUTPUT: everything built on it is fake too.
    let fake = fake_at(&s, 7);
    let fr = fake.steps.last().unwrap().find_fraud(&p()).unwrap();
    // It even runs ahead (it is cheap to make)…
    let fake_longer = fake.extend(&p()).extend(&p());
    assert_eq!(fake_longer.height(), 5);
    let honest = s.extend(&p());
    let mut proof_only = s.clone();
    proof_only.frauds.push(fr.clone());
    let r = all_orders(&[fake_longer.clone(), honest.clone(), proof_only.clone()]);
    assert!(r.windows(2).all(|w| w[0] == w[1]), "one result in every order");
    assert_eq!(r[0].steps, honest.steps, "the honest chain, the fake cut away");
    assert_eq!(r[0].frauds, vec![fr.clone()], "the proof kept");
    assert_eq!(check(&p(), &r[0]), Ok(()));
    // Offered again later, the fake does not come back.
    assert_eq!(merge(&p(), &r[0], &fake_longer).steps, honest.steps);
    // CONTROL: without the proof, the fake (longer) is the chain — the proof is what removed it.
    let no_proof = all_orders(&[fake_longer.clone(), honest.clone()]);
    assert!(no_proof.windows(2).all(|w| w[0] == w[1]));
    assert_eq!(no_proof[0].height(), 5, "unproven, the cheap fake chain leads");
}

#[test]
fn a_fake_that_kept_the_true_output_loses_only_itself() {
    // Forged inside, its output the true one: the steps built on it are honest and stay, under the honest step.
    let s = chain(2);
    let (fake, _) = fake_next(&s);
    let fr = fake.steps.last().unwrap().find_fraud(&p()).unwrap();
    let on_top = fake.extend(&p()).extend(&p());
    let honest = s.extend(&p());
    let mut proof_only = s.clone();
    proof_only.frauds.push(fr);
    let r = all_orders(&[on_top, honest.clone(), proof_only]);
    assert!(r.windows(2).all(|w| w[0] == w[1]), "one result in every order");
    assert_eq!(r[0].height(), 5);
    assert_eq!(r[0].steps[2].root, honest.steps[2].root, "the honest step at the fake's height");
    assert_eq!(r[0].steps, honest.extend(&p()).extend(&p()).steps, "exactly the honest chain");
    assert_eq!(r[0].frauds.len(), 1, "the proof kept");
}

#[test]
fn links_and_the_window_are_enforced() {
    let mut s = chain(3);
    s.steps[2].prev = [9; 32];
    assert_eq!(check(&p(), &s), Err("a broken link"));
    let mut s = chain(3);
    s.steps[0].points = Some(s.steps[2].points.clone().unwrap());
    assert_eq!(check(&p(), &s), Err("points kept past the fraud window"));
    let mut s = chain(3);
    s.steps[2].points = None;
    assert_eq!(check(&p(), &s), Err("a step in the fraud window without its points"));
    let mut s = chain(1);
    s.steps[0].prev = [1; 32];
    assert_eq!(check(&p(), &s), Err("a first step not after the genesis"));
}

#[test]
fn params_must_be_checkable_within_a_validation() {
    let ok = p();
    assert_eq!(Params::decode(&ok.encode()), Some(ok));
    assert_eq!(Params::decode(&Params { segs: 6, ..ok }.encode()), None, "segments a power of two");
    assert_eq!(Params::decode(&Params { k: (SEG_MAX + 1) * 8, ..ok }.encode()), None, "a segment too long to check");
    assert_eq!(Params::decode(&Params { k: 65, ..ok }.encode()), None, "whole segments");
    // The calibrated beacon (TOKEN §5a): ~1.07e9 iterations, 2048 segments, 8 sampled.
    let real = Params { k: 2048 * 524_288, segs: 2048, sample: 8, window: 2, keep: 1440, seed: [0; 32] };
    assert_eq!(Params::decode(&real.encode()), Some(real));
}

#[test]
fn bytes_round_trip_exactly() {
    let s = chain(4);
    let mut withfraud = s.clone();
    let (fake, _) = fake_next(&chain(2));
    withfraud.frauds.push(fake.steps.last().unwrap().find_fraud(&p()).unwrap());
    for x in [s, withfraud] {
        let b = wire::encode(&x);
        assert_eq!(wire::decode(&b), Some(x));
        let mut longer = b.clone();
        longer.push(0);
        assert_eq!(wire::decode(&longer), None);
    }
}


#[test]
fn a_full_length_beacon_merges() {
    // As kept for real (1440 steps): merging long copies stays linear and shallow.
    let q = Params { keep: 1440, ..p() };
    let mut s = State::default();
    for _ in 0..1500 {
        s = s.extend(&q);
    }
    assert_eq!(s.steps.len(), 1440);
    let longer = s.extend(&q);
    assert_eq!(merge(&q, &s, &longer), longer);
    assert_eq!(merge(&q, &longer, &s), longer);
    assert_eq!(check(&q, &longer), Ok(()));
}
