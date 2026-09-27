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

/// Alice and Bob share a computer; each is a member (a device key) of their own DID.
const ALICE: [u8; 32] = [7; 32];
const BOB: [u8; 32] = [8; 32];
/// A second member of Alice's DID on the same device (e.g. her work profile).
const ALICE2: [u8; 32] = [9; 32];
const ALICE_DID: [u8; 32] = [0xD1; 32];
const BOB_DID: [u8; 32] = [0xD2; 32];
const ALICE_PIN: &str = "432101";
const ALICE2_PIN: &str = "432102";
const BOB_PIN: &str = "987601";
const APP: [u8; 32] = [0xA1; 32];
const OTHER: [u8; 32] = [0xB2; 32];
/// The session token every test page holds.
const TOKEN: [u8; 32] = [0x5E; 32];

fn public(seed: [u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(&seed).verifying_key().to_bytes()
}

fn opened(seed: [u8; 32], did: [u8; 32]) -> Answer {
    Answer::Unlocked { public: public(seed), did }
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

fn provision(m: &mut Map, seed: [u8; 32], did: [u8; 32], pin: &str, origin: Origin) -> Answer {
    serve(m, Request::Provision { seed, did, pin: pin.into(), session: TOKEN }, origin)
}

/// Alice's member made by "log in with this device" from `origin`, which then holds a session on it.
fn provisioned(origin: Origin) -> Map {
    let mut m = Map::default();
    assert_eq!(provision(&mut m, ALICE, ALICE_DID, ALICE_PIN, origin), alice());
    m
}

fn unlock(m: &mut Map, pin: &str, origin: Origin) -> Answer {
    serve(m, Request::Unlock { pin: pin.into(), session: TOKEN }, origin)
}

fn who(m: &mut Map, origin: Origin) -> Answer {
    serve(m, Request::Who { session: TOKEN }, origin)
}

fn lock(m: &mut Map, origin: Origin) -> Answer {
    serve(m, Request::Lock, origin)
}

fn sign(m: &mut Map, origin: Origin, p: &[u8], seq: u64, v: [u8; 32]) -> Answer {
    serve(m, Request::Sign { session: TOKEN, params: p.to_vec(), seq, value_hash: v }, origin)
}

fn export(m: &mut Map, origin: Origin) -> Answer {
    serve(m, Request::Export { session: TOKEN }, origin)
}

#[test]
fn same_device_same_pin_opens_the_same_member_another_pin_another_person() {
    let mut m = provisioned(Origin::App(APP));
    assert_eq!(provision(&mut m, BOB, BOB_DID, BOB_PIN, Origin::App(APP)), bob());
    assert_eq!(who(&mut m, Origin::App(APP)), bob(), "the session is Bob's now");
    assert_eq!(lock(&mut m, Origin::App(APP)), Answer::LoggedOut);
    assert_eq!(who(&mut m, Origin::App(APP)), Answer::Refused(Why::NoSession));
    // Later visits: each PIN opens its own member, again and again.
    for _ in 0..2 {
        assert_eq!(unlock(&mut m, ALICE_PIN, Origin::App(APP)), alice());
        assert_eq!(who(&mut m, Origin::App(APP)), alice());
        assert_eq!(unlock(&mut m, BOB_PIN, Origin::App(APP)), bob());
        assert_eq!(who(&mut m, Origin::App(APP)), bob());
    }
    // Another page's token is not this session.
    let stranger = serve(&mut m, Request::Who { session: [1; 32] }, Origin::App(APP));
    assert_eq!(stranger, Answer::Refused(Why::NoSession));
}

#[test]
fn two_pins_on_one_device_can_be_two_members_of_one_did() {
    let mut m = provisioned(Origin::App(APP));
    assert_eq!(provision(&mut m, ALICE2, ALICE_DID, ALICE2_PIN, Origin::App(APP)), opened(ALICE2, ALICE_DID));
    // Each PIN opens its own member key; both are the same DID.
    assert_eq!(unlock(&mut m, ALICE_PIN, Origin::App(APP)), alice());
    assert_eq!(unlock(&mut m, ALICE2_PIN, Origin::App(APP)), opened(ALICE2, ALICE_DID));
    // Each member signs only with its own key.
    let first = params(public(ALICE), &APP);
    assert_eq!(sign(&mut m, Origin::App(APP), &first, 1, [1; 32]), Answer::Refused(Why::NotThisKey));
    assert!(matches!(sign(&mut m, Origin::App(APP), &params(public(ALICE2), &APP), 1, [1; 32]), Answer::Signed { .. }));
}

#[test]
fn a_session_signs_only_with_its_own_member() {
    let mut m = provisioned(Origin::App(APP));
    assert_eq!(provision(&mut m, BOB, BOB_DID, BOB_PIN, Origin::App(APP)), bob());
    let alices = params(public(ALICE), &APP);
    let bobs = params(public(BOB), &APP);
    assert_eq!(sign(&mut m, Origin::App(APP), &alices, 1, [1; 32]), Answer::Refused(Why::NotThisKey));
    assert!(matches!(sign(&mut m, Origin::App(APP), &bobs, 1, [1; 32]), Answer::Signed { .. }));
    assert_eq!(unlock(&mut m, ALICE_PIN, Origin::App(APP)), alice());
    assert!(matches!(sign(&mut m, Origin::App(APP), &alices, 1, [1; 32]), Answer::Signed { .. }));
    assert_eq!(sign(&mut m, Origin::App(APP), &bobs, 2, [1; 32]), Answer::Refused(Why::NotThisKey));
}

#[test]
fn a_pin_already_used_is_refused_and_counted_as_a_guess() {
    let mut m = provisioned(Origin::App(APP));
    assert_eq!(provision(&mut m, BOB, BOB_DID, ALICE_PIN, Origin::App(APP)), Answer::Refused(Why::PinTaken));
    // It cost a try: four wrong unlocks now lock the device.
    for _ in 0..3 {
        assert!(matches!(unlock(&mut m, "000000", Origin::App(APP)), Answer::WrongPin { .. }));
    }
    assert_eq!(unlock(&mut m, "000000", Origin::App(APP)), Answer::Locked);
    // Locked: no new member is made (making one is a guess too).
    assert_eq!(provision(&mut m, BOB, BOB_DID, BOB_PIN, Origin::App(APP)), Answer::Locked);
    // Too short a PIN is refused before anything.
    assert_eq!(provision(&mut Map::default(), BOB, BOB_DID, "1234", Origin::App(APP)), Answer::Refused(Why::BadPin));
}

#[test]
fn five_wrong_pins_lock_the_device_until_a_key_file_resets_a_pin() {
    let mut m = provisioned(Origin::App(APP));
    assert_eq!(provision(&mut m, BOB, BOB_DID, BOB_PIN, Origin::App(APP)), bob());
    for left in (1..MAX_TRIES).rev() {
        assert_eq!(unlock(&mut m, "000000", Origin::App(APP)), Answer::WrongPin { tries_left: left });
    }
    assert_eq!(unlock(&mut m, "000000", Origin::App(APP)), Answer::Locked);
    // Locked for everyone: even right PINs.
    assert_eq!(unlock(&mut m, ALICE_PIN, Origin::App(APP)), Answer::Locked);
    assert_eq!(unlock(&mut m, BOB_PIN, Origin::App(APP)), Answer::Locked);
    // Alice's key file sets her a new PIN, from any app; her old PIN no longer opens anything.
    assert_eq!(provision(&mut m, ALICE, ALICE_DID, "555555", Origin::App(OTHER)), alice());
    assert_eq!(unlock(&mut m, BOB_PIN, Origin::App(APP)), bob(), "the lock is cleared for everyone");
    assert_eq!(unlock(&mut m, "555555", Origin::App(APP)), alice());
    assert_eq!(unlock(&mut m, ALICE_PIN, Origin::App(APP)), Answer::WrongPin { tries_left: MAX_TRIES - 1 });
    // Her home did not move with the key file.
    assert_eq!(unlock(&mut m, "555555", Origin::App(APP)), alice());
    assert_eq!(export(&mut m, Origin::App(APP)), Answer::Exported { seed: ALICE });
    // A member key belongs to one DID: its key file under another is refused.
    assert_eq!(provision(&mut m, ALICE, BOB_DID, "666666", Origin::App(APP)), Answer::Refused(Why::OtherDid));
}

#[test]
fn a_wrong_pin_that_cannot_be_counted_is_not_answered() {
    let mut m = provisioned(Origin::App(APP));
    m.refuse = true;
    assert_eq!(unlock(&mut m, "000000", Origin::App(APP)), Answer::Refused(Why::NotSaved));
}

#[test]
fn the_signature_verifies_as_a_register_record_of_this_key() {
    let mut m = provisioned(Origin::App(APP));
    let p = params(public(ALICE), &[&APP[..], b"tail"].concat());
    let v = [3; 32];
    let Answer::Signed { sig } = sign(&mut m, Origin::App(APP), &p, 1, v) else { panic!("not signed") };
    let parsed = Params::parse(&p).unwrap();
    let Authority::One(k) = parsed.authority.clone() else { unreachable!() };
    let sig = Signature::from_bytes(&sig.try_into().unwrap());
    assert!(k.verify(&parsed.signed_message(false, 1, &v), &sig).is_ok());
    // Through the Register's own verifier (verify_strict), as a record the contract would accept.
    let signed = Signed { terminal: false, seq: 1, value_hash: v, bitmap: 0, sigs: vec![sig.to_bytes()] };
    assert!(signed.verify(&parsed));
    // The control: the same signature at another seq does not verify.
    assert!(!Signed { seq: 2, ..signed }.verify(&parsed));
}

#[test]
fn an_app_signs_only_records_under_its_own_id_local_tools_any() {
    let mut m = provisioned(Origin::App(APP));
    let theirs = params(public(ALICE), &[&OTHER[..], b"tail"].concat());
    assert_eq!(sign(&mut m, Origin::App(APP), &theirs, 1, [1; 32]), Answer::Refused(Why::NotYourRecord));
    // OTHER needs its own unlock, then signs its own record.
    assert_eq!(sign(&mut m, Origin::App(OTHER), &theirs, 1, [1; 32]), Answer::Refused(Why::NoSession));
    assert_eq!(unlock(&mut m, ALICE_PIN, Origin::App(OTHER)), alice());
    assert!(matches!(sign(&mut m, Origin::App(OTHER), &theirs, 1, [1; 32]), Answer::Signed { .. }));
    let any = params(public(ALICE), b"head");
    assert_eq!(sign(&mut m, Origin::App(APP), &any, 1, [1; 32]), Answer::Refused(Why::NotYourRecord));
    assert_eq!(unlock(&mut m, ALICE_PIN, Origin::Local), alice());
    assert!(matches!(sign(&mut m, Origin::Local, &any, 1, [1; 32]), Answer::Signed { .. }));
}

#[test]
fn only_its_own_single_key_is_signed_for() {
    let mut m = provisioned(Origin::Local);
    let someone = params(public([3; 32]), b"x");
    assert_eq!(sign(&mut m, Origin::Local, &someone, 1, [1; 32]), Answer::Refused(Why::NotThisKey));
    assert_eq!(sign(&mut m, Origin::Local, b"nonsense", 1, [1; 32]), Answer::Refused(Why::BadParams));
    // A quorum naming this key among others is not this member's record.
    let mut ks = [public(ALICE), public([3; 32])];
    ks.sort();
    let quorum = [&b"RG01"[..], &[1, 1, 2], &ks[0], &ks[1]].concat();
    assert_eq!(sign(&mut m, Origin::Local, &quorum, 1, [1; 32]), Answer::Refused(Why::NotThisKey));
}

#[test]
fn never_two_values_at_one_seq_never_backwards_a_reask_is_answered_the_same() {
    let mut m = provisioned(Origin::Local);
    let p = params(public(ALICE), b"r");
    let first = sign(&mut m, Origin::Local, &p, 5, [1; 32]);
    assert!(matches!(first, Answer::Signed { .. }));
    assert_eq!(sign(&mut m, Origin::Local, &p, 5, [1; 32]), first, "identical re-ask");
    assert_eq!(sign(&mut m, Origin::Local, &p, 5, [2; 32]), Answer::Refused(Why::WouldFork { last_seq: 5 }));
    assert_eq!(sign(&mut m, Origin::Local, &p, 4, [9; 32]), Answer::Refused(Why::WouldFork { last_seq: 5 }));
    assert!(matches!(sign(&mut m, Origin::Local, &p, 6, [2; 32]), Answer::Signed { .. }));
    // Another record has its own guard.
    let q = params(public(ALICE), b"q");
    assert!(matches!(sign(&mut m, Origin::Local, &q, 1, [2; 32]), Answer::Signed { .. }));
}

#[test]
fn nothing_is_signed_unless_the_guard_was_saved() {
    let mut m = provisioned(Origin::Local);
    m.refuse = true;
    let p = params(public(ALICE), b"r");
    assert_eq!(sign(&mut m, Origin::Local, &p, 1, [1; 32]), Answer::Refused(Why::NotSaved));
    m.refuse = false;
    // The refused ask left no guard: the same seq with another value still signs.
    assert!(matches!(sign(&mut m, Origin::Local, &p, 1, [2; 32]), Answer::Signed { .. }));
}

#[test]
fn only_the_home_or_own_tools_export_a_key() {
    let mut m = provisioned(Origin::App(APP));
    assert_eq!(unlock(&mut m, ALICE_PIN, Origin::App(OTHER)), alice());
    assert_eq!(export(&mut m, Origin::App(OTHER)), Answer::Refused(Why::NotHome));
    assert_eq!(export(&mut m, Origin::App(APP)), Answer::Exported { seed: ALICE });
    assert_eq!(export(&mut m, Origin::Local), Answer::Refused(Why::NoSession));
    assert_eq!(unlock(&mut m, ALICE_PIN, Origin::Local), alice());
    assert_eq!(export(&mut m, Origin::Local), Answer::Exported { seed: ALICE });
    assert_eq!(lock(&mut m, Origin::App(APP)), Answer::LoggedOut);
    assert_eq!(export(&mut m, Origin::App(APP)), Answer::Refused(Why::NoSession));
}

#[test]
fn delegates_get_nothing_and_garbage_is_refused_not_a_panic() {
    let mut m = provisioned(Origin::Local);
    assert_eq!(who(&mut m, Origin::Delegate), Answer::Refused(Why::NotForDelegates));
    let out = serve_bytes(&mut m, &encode_request(42, &Request::Who { session: TOKEN }), Origin::Local);
    assert_eq!(decode_answer(&out), Some((42, alice())));
    for junk in [&b""[..], b"ID01", b"ID01\xff\xff\xff\xff\xff", b"SG02whatever"] {
        let out = serve_bytes(&mut m, junk, Origin::Local);
        assert_eq!(decode_answer(&out), Some((0, Answer::Refused(Why::Unreadable))));
    }
    // A corrupt guard signs nothing.
    let p = params(public(ALICE), b"r");
    let hash = Params::parse(&p).unwrap().hash;
    m.s.insert([GUARD, &hash[..]].concat(), vec![1, 2, 3]);
    assert!(matches!(sign(&mut m, Origin::Local, &p, 1, [1; 32]), Answer::Refused(Why::WouldFork { .. })));
}
