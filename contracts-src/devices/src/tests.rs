use super::*;
use craftworks_idlog_contract::{commit, Event, Log};
use ed25519_dalek::SigningKey;

fn pubk(seed: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(seed).verifying_key().to_bytes()
}
/// An account: key 0 = seed [1], committing to [2]; rotated once: key 1 = [2], committing to [3].
fn log0() -> Log {
    Log { events: vec![Event::signed(0, [0; 32], &[1; 32], commit(&pubk(&[2; 32])), [5; 32], [6; 32], vec![]).unwrap()] }
}
fn log1() -> Log {
    let mut l = log0();
    let e = Event::signed(1, l.events[0].id(), &[2; 32], commit(&pubk(&[3; 32])), [5; 32], [6; 32], vec![]).unwrap();
    l.events.push(e);
    l
}
fn did() -> [u8; 32] {
    log0().events[0].id()
}
fn cred(node: u8, owner_seed: &[u8; 32]) -> Vec<u8> {
    craftworks_identity::credential(&did(), &[node; 32], &[], owner_seed)
}

#[test]
fn the_set_round_trips_exactly_and_refuses_disorder() {
    let s = vec![cred(1, &[1; 32]), cred(2, &[1; 32])];
    let b = encode(&s);
    let mut sorted = s.clone();
    sorted.sort();
    assert_eq!(decode(&b), Some(sorted.clone()));
    let mut longer = b.clone();
    longer.push(0);
    assert_eq!(decode(&longer), None);
    // Out of order (as written by hand): refused — one set has one encoding.
    let mut v = MAGIC.to_vec();
    v.extend_from_slice(&2u16.to_le_bytes());
    for c in sorted.iter().rev() {
        v.extend_from_slice(&(c.len() as u16).to_le_bytes());
        v.extend_from_slice(c);
    }
    assert_eq!(decode(&v), None);
}

#[test]
fn only_the_dids_credentials_signed_by_its_keys_are_valid() {
    assert!(valid(&did(), &log0(), &[cred(1, &[1; 32])]));
    assert!(!valid(&did(), &log0(), &[cred(1, &[9; 32])]), "a key the log never had");
    assert!(!valid(&did(), &log0(), &[cred(1, &[2; 32])]), "the next key, before the rotation reveals it");
    assert!(valid(&did(), &log1(), &[cred(1, &[1; 32]), cred(2, &[2; 32])]), "after it: both keys the log has had");
    let other = craftworks_identity::credential(&[7; 32], &[1; 32], &[], &[1; 32]);
    assert!(!valid(&did(), &log0(), &[other]), "another account's");
}

#[test]
fn nothing_is_ever_taken_out_and_a_rotation_leaves_only_reissued_devices_counting() {
    // Devices 1 and 2 credentialed on key 0.
    let before = vec![cred(1, &[1; 32]), cred(2, &[1; 32])];
    // Device 2 REMOVED: the key rotated, device 1 re-issued on key 1 — 2 gets nothing.
    let reissued = vec![cred(1, &[2; 32])];
    let merged = union(&before, &reissued).unwrap();
    assert_eq!(merged.len(), 3, "the union: nothing taken out");
    // A device holding the data key cannot shrink the set: the merge of the set with an empty one is the set.
    assert_eq!(union(&merged, &[]).unwrap(), merged);
    let now: Vec<[u8; 32]> = current(&did(), &log1(), &merged).into_iter().map(|(n, _)| n).collect();
    assert_eq!(now, vec![[1; 32]], "only the re-issued device counts");
    // CONTROL: before the rotation both count.
    let then: Vec<[u8; 32]> = current(&did(), &log0(), &before).into_iter().map(|(n, _)| n).collect();
    assert_eq!(then, vec![[1; 32], [2; 32]]);
}

#[test]
fn the_set_is_bounded() {
    let many: Vec<Vec<u8>> = (0..=MAX).map(|i| craftworks_identity::credential(&did(), &[(i % 256) as u8; 32], &[(i / 256) as u8], &[1; 32])).collect();
    assert!(union(&many[..MAX], &[]).is_some());
    assert!(union(&many, &[]).is_none());
}
