use super::*;
use craftec_register_contract::wire::{Params, Signed};
use ed25519_dalek::{Signature, Verifier};
use std::collections::HashMap;

#[derive(Default)]
struct Map {
    s: HashMap<Vec<u8>, Vec<u8>>,
    /// Refuse every write (a full disk).
    refuse: bool,
}

impl Host for Map {
    fn get_secret(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.s.get(key).cloned()
    }
    fn set_secret(&mut self, key: &[u8], value: &[u8]) -> bool {
        if self.refuse {
            return false;
        }
        self.s.insert(key.to_vec(), value.to_vec());
        true
    }
}

/// Alice and Bob share a computer; each is a member (a node key) of their own DID.
const ALICE: [u8; 32] = [7; 32];
const BOB: [u8; 32] = [8; 32];
/// A second member of Alice's DID on the same node.
const ALICE2: [u8; 32] = [9; 32];
const ALICE_DID: [u8; 32] = [0xD1; 32];
const BOB_DID: [u8; 32] = [0xD2; 32];
const ALICE_PIN: &str = "432101";
const ALICE2_PIN: &str = "432102";
const BOB_PIN: &str = "987601";
/// The app (site) that makes the members, and another app on the same node.
const APP: [u8; 32] = [0xA1; 32];
const OTHER: [u8; 32] = [0xB2; 32];

fn public(seed: [u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(&seed).verifying_key().to_bytes()
}

fn opened(seed: [u8; 32], did: [u8; 32]) -> Answer {
    Answer::Unlocked { public: public(seed), did, data: None }
}

fn alice() -> Answer {
    opened(ALICE, ALICE_DID)
}

fn bob() -> Answer {
    opened(BOB, BOB_DID)
}

/// One-key Register params: `RG01 ‖ 0 ‖ key ‖ label`.
fn params(key: [u8; 32], label: &[u8]) -> Vec<u8> {
    [&b"RG01"[..], &[0], &key, label].concat()
}

fn provision(m: &mut Map, seed: [u8; 32], did: [u8; 32], pin: &str, app: [u8; 32]) -> Answer {
    serve(m, Request::Provision { seed, did, pin: pin.into(), data: Vec::new() }, app)
}

/// Alice's member made from `app`, which then holds a session on it.
fn provisioned(app: [u8; 32]) -> Map {
    let mut m = Map::default();
    assert_eq!(provision(&mut m, ALICE, ALICE_DID, ALICE_PIN, app), alice());
    m
}

fn unlock(m: &mut Map, pin: &str, app: [u8; 32]) -> Answer {
    serve(m, Request::Unlock { pin: pin.into() }, app)
}

fn who(m: &mut Map, app: [u8; 32]) -> Answer {
    serve(m, Request::Who, app)
}

fn lock(m: &mut Map, app: [u8; 32]) -> Answer {
    serve(m, Request::Lock, app)
}

fn sign(m: &mut Map, app: [u8; 32], p: &[u8], seq: u64, v: [u8; 32]) -> Answer {
    serve(m, Request::Sign { params: p.to_vec(), seq, value_hash: v }, app)
}

fn export(m: &mut Map, app: [u8; 32]) -> Answer {
    serve(m, Request::Export, app)
}

/// A record of `app`'s, under `key`.
fn record(key: [u8; 32], app: [u8; 32]) -> Vec<u8> {
    params(key, &[&app[..], b"tail"].concat())
}

#[test]
fn the_gate_answers_only_a_web_app_the_node_names() {
    let mut m = provisioned(APP);
    // No origin (the node cannot say who asks), or another delegate: refused before anything is read or done.
    let out = serve_bytes(&mut m, &encode_request(9, &Request::Who), None);
    assert_eq!(decode_answer(&out), Some((9, Answer::Refused(Why::NotAttested))));
    let out = serve_bytes(&mut m, &encode_request(10, &Request::Export), None);
    assert_eq!(decode_answer(&out), Some((10, Answer::Refused(Why::NotAttested))));
    // The same request from the app the node names is served.
    let out = serve_bytes(&mut m, &encode_request(11, &Request::Who), Some(APP));
    assert_eq!(decode_answer(&out), Some((11, alice())));
    // Garbage is refused, never a panic.
    for junk in [&b""[..], b"ID01", b"ID01\xff\xff\xff\xff\xff", b"SG02whatever"] {
        assert_eq!(decode_answer(&serve_bytes(&mut m, junk, Some(APP))), Some((0, Answer::Refused(Why::Unreadable))));
    }
}

#[test]
fn same_node_same_pin_opens_the_same_member_another_pin_another_person() {
    let mut m = provisioned(APP);
    assert_eq!(provision(&mut m, BOB, BOB_DID, BOB_PIN, APP), bob());
    assert_eq!(who(&mut m, APP), bob(), "the session is Bob's now");
    assert_eq!(lock(&mut m, APP), Answer::LoggedOut);
    assert_eq!(who(&mut m, APP), Answer::Refused(Why::NoSession));
    for _ in 0..2 {
        assert_eq!(unlock(&mut m, ALICE_PIN, APP), alice());
        assert_eq!(who(&mut m, APP), alice());
        assert_eq!(unlock(&mut m, BOB_PIN, APP), bob());
        assert_eq!(who(&mut m, APP), bob());
    }
    // The session is the app's on this node: every later ask is still Bob until he logs out. Another app has none.
    assert_eq!(who(&mut m, APP), bob());
    assert_eq!(who(&mut m, OTHER), Answer::Refused(Why::NoSession));
}

#[test]
fn two_pins_on_one_node_can_be_two_members_of_one_did() {
    let mut m = provisioned(APP);
    assert_eq!(provision(&mut m, ALICE2, ALICE_DID, ALICE2_PIN, APP), opened(ALICE2, ALICE_DID));
    assert_eq!(unlock(&mut m, ALICE_PIN, APP), alice());
    assert_eq!(unlock(&mut m, ALICE2_PIN, APP), opened(ALICE2, ALICE_DID));
    assert_eq!(sign(&mut m, APP, &record(public(ALICE), APP), 1, [1; 32]), Answer::Refused(Why::NotThisKey));
    assert!(matches!(sign(&mut m, APP, &record(public(ALICE2), APP), 1, [1; 32]), Answer::Signed { .. }));
}

#[test]
fn a_pin_already_used_is_refused_and_counted_as_a_guess() {
    let mut m = provisioned(APP);
    assert_eq!(provision(&mut m, BOB, BOB_DID, ALICE_PIN, APP), Answer::Refused(Why::PinTaken));
    for _ in 0..3 {
        assert!(matches!(unlock(&mut m, "000000", APP), Answer::WrongPin { .. }));
    }
    assert_eq!(unlock(&mut m, "000000", APP), Answer::Locked);
    assert_eq!(provision(&mut m, BOB, BOB_DID, BOB_PIN, APP), Answer::Locked);
    assert_eq!(provision(&mut Map::default(), BOB, BOB_DID, "1234", APP), Answer::Refused(Why::BadPin));
}

#[test]
fn five_wrong_pins_lock_the_node_until_a_key_file_resets_a_pin() {
    let mut m = provisioned(APP);
    assert_eq!(provision(&mut m, BOB, BOB_DID, BOB_PIN, APP), bob());
    for left in (1..MAX_TRIES).rev() {
        assert_eq!(unlock(&mut m, "000000", APP), Answer::WrongPin { tries_left: left });
    }
    assert_eq!(unlock(&mut m, "000000", APP), Answer::Locked);
    assert_eq!(unlock(&mut m, ALICE_PIN, APP), Answer::Locked);
    assert_eq!(unlock(&mut m, BOB_PIN, APP), Answer::Locked);
    // Alice's key file sets her a new PIN, from any app; her old PIN no longer opens anything.
    assert_eq!(provision(&mut m, ALICE, ALICE_DID, "555555", OTHER), alice());
    assert_eq!(unlock(&mut m, BOB_PIN, APP), bob(), "the lock is cleared for everyone");
    assert_eq!(unlock(&mut m, "555555", APP), alice());
    assert_eq!(unlock(&mut m, ALICE_PIN, APP), Answer::WrongPin { tries_left: MAX_TRIES - 1 });
    // Her home did not move with the key file.
    assert_eq!(unlock(&mut m, "555555", APP), alice());
    assert_eq!(export(&mut m, APP), Answer::Exported { seed: ALICE });
    assert_eq!(provision(&mut m, ALICE, BOB_DID, "666666", APP), Answer::Refused(Why::OtherDid));
}

#[test]
fn a_wrong_pin_that_cannot_be_counted_is_not_answered() {
    let mut m = provisioned(APP);
    m.refuse = true;
    assert_eq!(unlock(&mut m, "000000", APP), Answer::Refused(Why::NotSaved));
}

#[test]
fn the_signature_verifies_as_a_register_record_of_this_key() {
    let mut m = provisioned(APP);
    let p = record(public(ALICE), APP);
    let v = [3; 32];
    let Answer::Signed { sig } = sign(&mut m, APP, &p, 1, v) else { panic!("not signed") };
    let parsed = Params::parse(&p).unwrap();
    let Authority::One(k) = parsed.authority.clone() else { unreachable!() };
    let sig = Signature::from_bytes(&sig.try_into().unwrap());
    assert!(k.verify(&parsed.signed_message(false, 1, &v), &sig).is_ok());
    let signed = Signed { terminal: false, seq: 1, value_hash: v, bitmap: 0, sigs: vec![sig.to_bytes()] };
    assert!(signed.verify(&parsed));
    assert!(!Signed { seq: 2, ..signed }.verify(&parsed));
}

#[test]
fn an_app_signs_only_records_under_its_own_id() {
    let mut m = provisioned(APP);
    let theirs = record(public(ALICE), OTHER);
    assert_eq!(sign(&mut m, APP, &theirs, 1, [1; 32]), Answer::Refused(Why::NotYourRecord));
    assert_eq!(sign(&mut m, OTHER, &theirs, 1, [1; 32]), Answer::Refused(Why::NoSession));
    assert_eq!(unlock(&mut m, ALICE_PIN, OTHER), alice());
    assert!(matches!(sign(&mut m, OTHER, &theirs, 1, [1; 32]), Answer::Signed { .. }));
    // No app signs a label outside its own id, whatever the key.
    assert_eq!(sign(&mut m, APP, &params(public(ALICE), b"head"), 1, [1; 32]), Answer::Refused(Why::NotYourRecord));
}

#[test]
fn only_its_own_single_key_is_signed_for() {
    let mut m = provisioned(APP);
    assert_eq!(sign(&mut m, APP, &record(public([3; 32]), APP), 1, [1; 32]), Answer::Refused(Why::NotThisKey));
    assert_eq!(sign(&mut m, APP, b"nonsense", 1, [1; 32]), Answer::Refused(Why::BadParams));
    let mut ks = [public(ALICE), public([3; 32])];
    ks.sort();
    let quorum = [&b"RG01"[..], &[1, 1, 2], &ks[0], &ks[1], &APP].concat();
    assert_eq!(sign(&mut m, APP, &quorum, 1, [1; 32]), Answer::Refused(Why::NotThisKey));
}

#[test]
fn never_two_values_at_one_seq_never_backwards_a_reask_is_answered_the_same() {
    let mut m = provisioned(APP);
    let p = record(public(ALICE), APP);
    let first = sign(&mut m, APP, &p, 5, [1; 32]);
    assert!(matches!(first, Answer::Signed { .. }));
    assert_eq!(sign(&mut m, APP, &p, 5, [1; 32]), first, "identical re-ask");
    assert_eq!(sign(&mut m, APP, &p, 5, [2; 32]), Answer::Refused(Why::WouldFork { last_seq: 5 }));
    assert_eq!(sign(&mut m, APP, &p, 4, [9; 32]), Answer::Refused(Why::WouldFork { last_seq: 5 }));
    assert!(matches!(sign(&mut m, APP, &p, 6, [2; 32]), Answer::Signed { .. }));
    let q = params(public(ALICE), &[&APP[..], b"other"].concat());
    assert!(matches!(sign(&mut m, APP, &q, 1, [2; 32]), Answer::Signed { .. }));
    // A corrupt guard signs nothing.
    let r = params(public(ALICE), &[&APP[..], b"r"].concat());
    m.s.insert([GUARD, &Params::parse(&r).unwrap().hash[..]].concat(), vec![1, 2, 3]);
    assert!(matches!(sign(&mut m, APP, &r, 1, [1; 32]), Answer::Refused(Why::WouldFork { .. })));
}

#[test]
fn nothing_is_signed_unless_the_guard_was_saved() {
    let mut m = provisioned(APP);
    m.refuse = true;
    let p = record(public(ALICE), APP);
    assert_eq!(sign(&mut m, APP, &p, 1, [1; 32]), Answer::Refused(Why::NotSaved));
    m.refuse = false;
    assert!(matches!(sign(&mut m, APP, &p, 1, [2; 32]), Answer::Signed { .. }));
}

#[test]
fn only_the_home_exports_a_key() {
    let mut m = provisioned(APP);
    assert_eq!(unlock(&mut m, ALICE_PIN, OTHER), alice());
    assert_eq!(export(&mut m, OTHER), Answer::Refused(Why::NotHome));
    assert_eq!(export(&mut m, APP), Answer::Exported { seed: ALICE });
    assert_eq!(lock(&mut m, APP), Answer::LoggedOut);
    assert_eq!(export(&mut m, APP), Answer::Refused(Why::NoSession));
}

#[test]
fn every_node_of_an_account_signs_its_data_with_the_one_data_key() {
    const DATA: [u8; 32] = [0xDA; 32];
    let mut m = Map::default();
    let give = |m: &mut Map, seed, pin: &str| {
        serve(m, Request::Provision { seed, did: ALICE_DID, pin: pin.into(), data: DATA.to_vec() }, APP)
    };
    let data_public = public(DATA);
    assert_eq!(give(&mut m, ALICE, ALICE_PIN), Answer::Unlocked { public: public(ALICE), did: ALICE_DID, data: Some(data_public) });
    let table = record(data_public, APP);
    assert!(matches!(sign(&mut m, APP, &table, 1, [1; 32]), Answer::Signed { .. }));
    assert!(matches!(give(&mut m, ALICE2, ALICE2_PIN), Answer::Unlocked { .. }));
    assert!(matches!(sign(&mut m, APP, &table, 2, [2; 32]), Answer::Signed { .. }));
    assert_eq!(sign(&mut m, APP, &record(data_public, OTHER), 1, [1; 32]), Answer::Refused(Why::NotYourRecord));
    assert_eq!(provision(&mut m, BOB, BOB_DID, BOB_PIN, APP), bob());
    assert_eq!(sign(&mut m, APP, &table, 3, [3; 32]), Answer::Refused(Why::NotThisKey));
    let r = serve(&mut m, Request::Provision { seed: [4; 32], did: BOB_DID, pin: "888888".into(), data: vec![1; 20] }, APP);
    assert_eq!(r, Answer::Refused(Why::BadDataKey));
    let keep = Answer::Unlocked { public: public(ALICE), did: ALICE_DID, data: Some(data_public) };
    assert_eq!(unlock(&mut m, ALICE_PIN, APP), keep);
    let r = serve(&mut m, Request::Provision { seed: ALICE, did: ALICE_DID, pin: "777777".into(), data: vec![] }, APP);
    assert_eq!(r, keep, "a key file brings no data key, and does not erase the one held");
}

#[test]
fn a_member_is_handed_to_the_next_version_on_its_pin_to_its_home_only() {
    const DATA: [u8; 32] = [0xDA; 32];
    let mut old = Map::default();
    let r = serve(&mut old, Request::Provision { seed: ALICE, did: ALICE_DID, pin: ALICE_PIN.into(), data: DATA.to_vec() }, APP);
    assert!(matches!(r, Answer::Unlocked { .. }));
    assert_eq!(provision(&mut old, BOB, BOB_DID, BOB_PIN, APP), bob());
    // Another app, even with the right PIN, gets nothing: it never held this member.
    let from_other = serve(&mut old, Request::Handover { pin: ALICE_PIN.into() }, OTHER);
    assert_eq!(from_other, Answer::Refused(Why::NotHome));
    // A wrong PIN is a counted try, like an unlock.
    let wrong = serve(&mut old, Request::Handover { pin: "000000".into() }, APP);
    assert_eq!(wrong, Answer::WrongPin { tries_left: MAX_TRIES - 1 });
    // The right PIN, from the home: exactly that member, and nobody else's.
    let handed = serve(&mut old, Request::Handover { pin: ALICE_PIN.into() }, APP);
    assert_eq!(handed, Answer::Handed { seed: ALICE, did: ALICE_DID, data: Some(DATA) });
    // The next version takes it in with a Provision: the same member, the same PIN, the same data key.
    let Answer::Handed { seed, did, data } = handed else { unreachable!() };
    let mut new = Map::default();
    let r = serve(&mut new, Request::Provision { seed, did, pin: ALICE_PIN.into(), data: data.unwrap().to_vec() }, APP);
    assert_eq!(r, Answer::Unlocked { public: public(ALICE), did: ALICE_DID, data: Some(public(DATA)) });
    assert_eq!(unlock(&mut new, BOB_PIN, APP), Answer::WrongPin { tries_left: MAX_TRIES - 1 }, "Bob was not carried");
}
