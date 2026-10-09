use super::*;
use craftworks_standing::record::{Epoch, State};
use craftworks_standing::{place, sampled, seal, Answer, Attest, Bind, Holdings, EPOCH};
use ed25519_dalek::SigningKey;
use std::net::SocketAddr;

const P: &str = "5cCFQqQwLPNKX4zwK";

fn key(n: u8) -> SigningKey {
    SigningKey::from_bytes(&[n; 32])
}
fn id(n: u32) -> [u8; 32] {
    *blake3::hash(&n.to_le_bytes()).as_bytes()
}

/// A REAL beacon run to `top` (keeping 600 steps), and its outputs log as the extenders fill it.
fn beacon(top: u64) -> (B::Params, B::State, Vec<B::log::Log>) {
    let p = B::Params { k: 64, segs: 8, sample: 2, window: 2, keep: 600, seed: [3; 32] };
    let mut s = B::State::default();
    let mut logs = vec![B::log::Log::default(); (top / B::log::SPAN + 1) as usize];
    for i in 0..top {
        s = s.extend(&p, *blake3::hash(&i.to_le_bytes()).as_bytes());
        for (idx, l) in logs.iter_mut().enumerate() {
            let lp = B::log::Params { beacon: [9; 32], index: idx as u64 };
            *l = B::log::take(&lp, l, &B::log::record(&lp, &s), &s).unwrap();
        }
    }
    (p, s, logs)
}

/// Peer P placed by three attesters, key 7 bound, holdings over 40 contracts, an answer at each height of epoch 2
/// sampled for it (by the REAL beacon's outputs).
fn record(out: &dyn Fn(u64) -> [u8; 32], upto: u64) -> State {
    let e = 2;
    let addr: SocketAddr = "86.38.75.1:1".parse().unwrap();
    let ring = craftworks_standing::ring_of_addr(&addr);
    let at = |n: u64| ring.wrapping_add(if n.is_multiple_of(2) { u64::MAX / 4 } else { (u64::MAX / 4).wrapping_neg() });
    let attests: Vec<Attest> = (1..=3u8).map(|n| Attest::new(&key(n), P, &addr, at(n as u64), e)).collect();
    let k = key(7);
    let binds = vec![Bind::new(&k, P, e)];
    let (_, arc_) = place(P, e, &attests, &[k.verifying_key().to_bytes()], 3).unwrap();
    let hosted: Vec<([u8; 32], u64)> = (0..40).map(|n| (id(n), 1000)).collect();
    let (h, ids) = Holdings::new(&k, P, e, &hosted, arc_);
    let kb = k.verifying_key().to_bytes();
    let replica = |i: &[u8; 32]| Some(seal(&kb, i, i));
    let answers: Vec<Answer> = (e * EPOCH..=upto).filter(|x| sampled(&out(*x), P)).filter_map(|x| Answer::new(&k, &h, &ids, x, &out(x), &replica)).collect();
    State { epochs: vec![Epoch { epoch: e, attests, binds, holdings: vec![h], answers }] }
}

#[test]
fn a_standing_record_is_checked_against_the_beacon_and_its_outputs_log() {
    let top = 2 * EPOCH + 1000;
    let (_, b, logs) = beacon(top);
    let truth = |h: u64| b.steps.iter().find(|s| s.height == h).map(|s| s.out).or_else(|| logs[(h / B::log::SPAN) as usize].get(h)).unwrap();
    let s = record(&truth, top - 5);
    let first = s.epochs[0].answers.iter().map(|a| a.height).min().unwrap();
    let oldest = b.steps.first().unwrap().height;
    assert!(first < oldest, "THE SETUP: an answer older than the beacon keeps ({first} < {oldest})");
    let p = Params { beacon_code: [8; 32], beacon: [9; 32], peer: P.into() };
    assert_eq!(Params::decode(&p.encode()), Some(p.clone()));
    let logs_of = |h: u64| logs.get((h / B::log::SPAN) as usize).cloned();
    assert_eq!(check(&p, &s, Some(&b), &logs_of), Ok(()));
    // Only recent answers: checked from the beacon alone, with no log at all.
    let mut recent = s.clone();
    recent.epochs[0].answers.retain(|a| a.height >= oldest);
    assert!(!recent.epochs[0].answers.is_empty(), "THE SETUP: answers the beacon still keeps");
    assert_eq!(check(&p, &recent, Some(&b), &|_| None), Ok(()));
    // Without the log: its oldest answers cannot be checked.
    assert_eq!(check(&p, &s, Some(&b), &|_| None), Err("an answer outside the beacon's window"));
    // A log that says another output: the answer does not hold.
    let lying = |h: u64| logs_of(h).map(|mut l| {
        for e in &mut l.entries {
            e.1 = [7; 32];
        }
        l
    });
    assert!(check(&p, &s, Some(&b), &lying).is_err());
    // What a check reads: the beacon, and the log of every answered span.
    let w = wanted(&p, &s);
    assert_eq!(w[0], p.beacon);
    assert!(w.contains(&p.log_id(first)));
    assert_eq!(w.iter().filter(|x| **x == p.log_id(first)).count(), 1, "each log once");
    // Another peer's contract: its record does not hold there.
    let q = Params { peer: "another".into(), ..p.clone() };
    assert!(check(&q, &s, Some(&b), &logs_of).is_err());
}
