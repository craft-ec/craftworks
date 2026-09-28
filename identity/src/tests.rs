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
/// The accounts' data keys (every member of one account holds its account's).
const ALICE_DATA: [u8; 32] = [0xDA; 32];
const BOB_DATA: [u8; 32] = [0xDB; 32];

fn public(seed: [u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(&seed).verifying_key().to_bytes()
}

fn data_of(did: [u8; 32]) -> [u8; 32] {
    if did == BOB_DID { BOB_DATA } else { ALICE_DATA }
}

fn opened(seed: [u8; 32], did: [u8; 32]) -> Answer {
    Answer::Unlocked { public: public(seed), did, data: Some(public(data_of(did))) }
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
    serve(m, Request::Provision { seed, did, pin: pin.into(), data: data_of(did).to_vec() }, app)
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
    serve(m, Request::Sign { params: p.to_vec(), seq, value_hash: v, space: None }, app)
}

fn export(m: &mut Map, app: [u8; 32]) -> Answer {
    serve(m, Request::Export, app)
}

/// The page's answer bytes (a request that is not a grant is always answered at once).
fn now(o: Out) -> Vec<u8> {
    match o {
        Out::Answer(a) => a,
        Out::Ask(p) => panic!("asked the person: {p:?}"),
    }
}

/// Table `table` of an account, under its data key.
fn table(data: [u8; 32], table: &str) -> Vec<u8> {
    params(public(data), &[&TABLE[..], table.as_bytes()].concat())
}

/// Ask for a grant, and answer the node's prompt with `choice`: the page's answer.
fn ask(m: &mut Map, app: [u8; 32], t: &str, choice: &str) -> Answer {
    asks(m, app, &[t], choice)
}

fn asks(m: &mut Map, app: [u8; 32], ts: &[&str], choice: &str) -> Answer {
    let t = ts.join(" ");
    match serve_bytes(m, &encode_request(77, &Request::Grant { tables: ts.iter().map(|t| t.to_string()).collect() }), Some(app)) {
        Out::Answer(a) => decode_answer(&a).unwrap().1,
        Out::Ask(p) => {
            for one in t.split(' ') {
                assert!(p.message.contains(one), "the prompt names {one}: {}", p.message);
            }
            assert_eq!(p.choices, [ALLOW, DENY]);
            let (id, answer) = decode_answer(&serve_answer(m, p.id, p.choices_bytes(choice)).unwrap()).unwrap();
            assert_eq!(id, 77, "the page that asked is answered, under its own id");
            answer
        }
    }
}

impl Prompt {
    fn choices_bytes<'a>(&self, c: &'a str) -> &'a [u8] {
        c.as_bytes()
    }
}

#[test]
fn the_gate_answers_only_a_web_app_the_node_names() {
    let mut m = provisioned(APP);
    // No origin (the node cannot say who asks), or another delegate: refused before anything is read or done.
    let out = serve_bytes(&mut m, &encode_request(9, &Request::Who), None);
    assert_eq!(decode_answer(&now(out)), Some((9, Answer::Refused(Why::NotAttested))));
    let out = serve_bytes(&mut m, &encode_request(10, &Request::Export), None);
    assert_eq!(decode_answer(&now(out)), Some((10, Answer::Refused(Why::NotAttested))));
    // The same request from the app the node names is served.
    let out = serve_bytes(&mut m, &encode_request(11, &Request::Who), Some(APP));
    assert_eq!(decode_answer(&now(out)), Some((11, alice())));
    // Garbage is refused, never a panic.
    for junk in [&b""[..], b"ID01", b"ID01\xff\xff\xff\xff\xff", b"SG02whatever"] {
        assert_eq!(decode_answer(&now(serve_bytes(&mut m, junk, Some(APP)))), Some((0, Answer::Refused(Why::Unreadable))));
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
fn the_signature_verifies_as_a_register_record_of_the_data_key() {
    let mut m = provisioned(APP);
    assert_eq!(ask(&mut m, APP, "notes", ALLOW), Answer::Granted { tables: vec!["notes".into()] });
    let p = table(ALICE_DATA, "notes");
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
fn a_site_writes_a_table_only_with_the_persons_grant() {
    let mut m = provisioned(APP);
    assert_eq!(unlock(&mut m, ALICE_PIN, OTHER), alice());
    let notes = table(ALICE_DATA, "notes");
    // Nothing is signed for a table the site was never allowed.
    assert_eq!(sign(&mut m, OTHER, &notes, 1, [1; 32]), Answer::Refused(Why::NotGranted { table: "notes".into() }));
    // The person says no: still nothing.
    assert_eq!(ask(&mut m, OTHER, "notes", DENY), Answer::Refused(Why::Denied));
    assert_eq!(sign(&mut m, OTHER, &notes, 1, [1; 32]), Answer::Refused(Why::NotGranted { table: "notes".into() }));
    // The person allows: signed; and asking again is answered at once, no prompt.
    assert_eq!(ask(&mut m, OTHER, "notes", ALLOW), Answer::Granted { tables: vec!["notes".into()] });
    assert!(matches!(sign(&mut m, OTHER, &notes, 1, [1; 32]), Answer::Signed { .. }));
    let again = serve_bytes(&mut m, &encode_request(5, &Request::Grant { tables: vec!["notes".into()] }), Some(OTHER));
    assert!(matches!(again, Out::Answer(_)), "an existing grant needs no prompt");
    // A grant is for one table.
    assert_eq!(sign(&mut m, OTHER, &table(ALICE_DATA, "pins"), 2, [1; 32]), Answer::Refused(Why::NotGranted { table: "pins".into() }));
    // The home (the site the member was made with) is granted without a prompt, and writes the same table.
    let home = serve_bytes(&mut m, &encode_request(6, &Request::Grant { tables: vec!["notes".into()] }), Some(APP));
    assert!(matches!(home, Out::Answer(_)), "the home is not prompted");
    assert!(matches!(sign(&mut m, APP, &notes, 2, [2; 32]), Answer::Signed { .. }));
    // A prompt nobody waits on is ignored; a bad name is refused without one.
    assert_eq!(serve_answer(&mut m, 999, ALLOW.as_bytes()), None);
    assert_eq!(ask(&mut m, OTHER, "Bad Name!", ALLOW), Answer::Refused(Why::BadTable));
}

#[test]
fn grants_are_per_person_and_the_home_lists_and_revokes_them() {
    let mut m = provisioned(APP);
    assert_eq!(ask(&mut m, APP, "notes", ALLOW), Answer::Granted { tables: vec!["notes".into()] });
    assert_eq!(unlock(&mut m, ALICE_PIN, OTHER), alice());
    assert_eq!(ask(&mut m, OTHER, "notes", ALLOW), Answer::Granted { tables: vec!["notes".into()] });
    // Bob on the same node has none of Alice's grants.
    assert_eq!(provision(&mut m, BOB, BOB_DID, BOB_PIN, APP), bob());
    assert_eq!(sign(&mut m, APP, &table(BOB_DATA, "notes"), 1, [1; 32]), Answer::Refused(Why::NotGranted { table: "notes".into() }));
    assert_eq!(serve(&mut m, Request::Grants, APP), Answer::Grants { list: vec![] });
    // Alice's home lists all of hers; another site sees only its own.
    assert_eq!(unlock(&mut m, ALICE_PIN, APP), alice());
    assert_eq!(serve(&mut m, Request::Grants, APP), Answer::Grants { list: vec![(APP, "notes".into()), (OTHER, "notes".into())] });
    assert_eq!(serve(&mut m, Request::Grants, OTHER), Answer::Grants { list: vec![(OTHER, "notes".into())] });
    // Another site may not revoke the home's grant; the home revokes any.
    assert_eq!(serve(&mut m, Request::Revoke { app: APP, table: "notes".into() }, OTHER), Answer::Refused(Why::NotHome));
    assert_eq!(serve(&mut m, Request::Revoke { app: OTHER, table: "notes".into() }, APP), Answer::Revoked);
    assert_eq!(sign(&mut m, OTHER, &table(ALICE_DATA, "notes"), 1, [1; 32]), Answer::Refused(Why::NotGranted { table: "notes".into() }));
    assert_eq!(serve(&mut m, Request::Grants, APP), Answer::Grants { list: vec![(APP, "notes".into())] });
}

#[test]
fn only_the_members_own_feed_or_the_accounts_data_key_signs_and_only_a_table() {
    let mut m = provisioned(APP);
    assert_eq!(ask(&mut m, APP, "notes", ALLOW), Answer::Granted { tables: vec!["notes".into()] });
    // The member's own key signs its own FEED of a table: a signature that verifies under that key.
    let own = params(public(ALICE), b"t/notes");
    let Answer::Signed { sig } = sign(&mut m, APP, &own, 1, [1; 32]) else { panic!("its own feed is signed") };
    let vk = ed25519_dalek::VerifyingKey::from_bytes(&public(ALICE)).unwrap();
    let msg = Params::parse(&own).unwrap().signed_message(false, 1, &[1; 32]);
    assert!(vk.verify(&msg, &Signature::from_slice(&sig).unwrap()).is_ok());
    // Its own key, but not a table label.
    assert_eq!(sign(&mut m, APP, &params(public(ALICE), b"head"), 1, [1; 32]), Answer::Refused(Why::NotATable));
    // Anyone else's key signs nothing.
    assert_eq!(sign(&mut m, APP, &table([3; 32], "notes"), 1, [1; 32]), Answer::Refused(Why::NotThisKey));
    assert_eq!(sign(&mut m, APP, b"nonsense", 1, [1; 32]), Answer::Refused(Why::BadParams));
    // The data key, but not a table label.
    assert_eq!(sign(&mut m, APP, &params(public(ALICE_DATA), b"head"), 1, [1; 32]), Answer::Refused(Why::NotATable));
    let mut ks = [public(ALICE_DATA), public([3; 32])];
    ks.sort();
    let quorum = [&b"RG01"[..], &[1, 1, 2], &ks[0], &ks[1], b"t/notes"].concat();
    assert_eq!(sign(&mut m, APP, &quorum, 1, [1; 32]), Answer::Refused(Why::NotThisKey));
    // A member given no data key signs nothing.
    let mut n = Map::default();
    let r = serve(&mut n, Request::Provision { seed: BOB, did: BOB_DID, pin: BOB_PIN.into(), data: vec![] }, APP);
    assert!(matches!(r, Answer::Unlocked { data: None, .. }));
    assert_eq!(sign(&mut n, APP, &table(BOB_DATA, "notes"), 1, [1; 32]), Answer::Refused(Why::NoDataKey));
}

#[test]
fn never_two_values_at_one_seq_never_backwards_a_reask_is_answered_the_same() {
    let mut m = provisioned(APP);
    assert_eq!(ask(&mut m, APP, "notes", ALLOW), Answer::Granted { tables: vec!["notes".into()] });
    assert_eq!(ask(&mut m, APP, "other", ALLOW), Answer::Granted { tables: vec!["other".into()] });
    let p = table(ALICE_DATA, "notes");
    let first = sign(&mut m, APP, &p, 5, [1; 32]);
    assert!(matches!(first, Answer::Signed { .. }));
    assert_eq!(sign(&mut m, APP, &p, 5, [1; 32]), first, "identical re-ask");
    assert_eq!(sign(&mut m, APP, &p, 5, [2; 32]), Answer::Refused(Why::WouldFork { last_seq: 5 }));
    assert_eq!(sign(&mut m, APP, &p, 4, [9; 32]), Answer::Refused(Why::WouldFork { last_seq: 5 }));
    assert!(matches!(sign(&mut m, APP, &p, 6, [2; 32]), Answer::Signed { .. }));
    assert!(matches!(sign(&mut m, APP, &table(ALICE_DATA, "other"), 1, [2; 32]), Answer::Signed { .. }));
    // A corrupt guard signs nothing.
    m.s.insert([GUARD, &Params::parse(&table(ALICE_DATA, "other")).unwrap().hash[..]].concat(), vec![1, 2, 3]);
    assert!(matches!(sign(&mut m, APP, &table(ALICE_DATA, "other"), 2, [1; 32]), Answer::Refused(Why::WouldFork { .. })));
}

#[test]
fn nothing_is_signed_unless_the_guard_was_saved() {
    let mut m = provisioned(APP);
    assert_eq!(ask(&mut m, APP, "notes", ALLOW), Answer::Granted { tables: vec!["notes".into()] });
    m.refuse = true;
    let p = table(ALICE_DATA, "notes");
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
    let mut m = provisioned(APP);
    assert_eq!(ask(&mut m, APP, "notes", ALLOW), Answer::Granted { tables: vec!["notes".into()] });
    let notes = table(ALICE_DATA, "notes");
    assert!(matches!(sign(&mut m, APP, &notes, 1, [1; 32]), Answer::Signed { .. }));
    // A second member of the account (another node's, here on one node) signs the next write of the same table.
    assert_eq!(provision(&mut m, ALICE2, ALICE_DID, ALICE2_PIN, APP), opened(ALICE2, ALICE_DID));
    assert_eq!(ask(&mut m, APP, "notes", ALLOW), Answer::Granted { tables: vec!["notes".into()] });
    assert!(matches!(sign(&mut m, APP, &notes, 2, [2; 32]), Answer::Signed { .. }));
    let r = serve(&mut m, Request::Provision { seed: [4; 32], did: BOB_DID, pin: "888888".into(), data: vec![1; 20] }, APP);
    assert_eq!(r, Answer::Refused(Why::BadDataKey));
    // A key file brings no data key, and does not erase the one held.
    let r = serve(&mut m, Request::Provision { seed: ALICE, did: ALICE_DID, pin: "777777".into(), data: vec![] }, APP);
    assert_eq!(r, alice());
}

#[test]
fn a_member_is_handed_to_the_next_version_on_its_pin_to_its_home_only() {
    const DATA: [u8; 32] = ALICE_DATA;
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
    let _ = DATA;
}

#[test]
fn one_prompt_for_every_table_an_app_uses_and_later_only_for_a_new_one() {
    let mut m = provisioned(APP);
    assert_eq!(unlock(&mut m, ALICE_PIN, OTHER), alice());
    // One prompt names both.
    let out = serve_bytes(&mut m, &encode_request(8, &Request::Grant { tables: vec!["notes".into(), "pins".into()] }), Some(OTHER));
    let Out::Ask(p) = out else { panic!("expected one prompt") };
    assert!(p.message.contains("“notes” and “pins”"), "{}", p.message);
    let (_, a) = decode_answer(&serve_answer(&mut m, p.id, ALLOW.as_bytes()).unwrap()).unwrap();
    assert_eq!(a, Answer::Granted { tables: vec!["notes".into(), "pins".into()] });
    assert!(matches!(sign(&mut m, OTHER, &table(ALICE_DATA, "notes"), 1, [1; 32]), Answer::Signed { .. }));
    assert!(matches!(sign(&mut m, OTHER, &table(ALICE_DATA, "pins"), 1, [1; 32]), Answer::Signed { .. }));
    // The same list again: no prompt. A new table: a prompt for it alone.
    assert!(matches!(serve_bytes(&mut m, &encode_request(9, &Request::Grant { tables: vec!["notes".into(), "pins".into()] }), Some(OTHER)), Out::Answer(_)));
    let out = serve_bytes(&mut m, &encode_request(10, &Request::Grant { tables: vec!["notes".into(), "labels".into()] }), Some(OTHER));
    let Out::Ask(p) = out else { panic!("expected a prompt for the new table") };
    assert!(p.message.contains("“labels”") && !p.message.contains("notes"), "{}", p.message);
    // A list that is empty or too long, or a bad name in it, is refused without a prompt.
    assert_eq!(asks(&mut m, OTHER, &[], ALLOW), Answer::Refused(Why::BadTable));
    assert_eq!(asks(&mut m, OTHER, &["ok", "Bad!"], ALLOW), Answer::Refused(Why::BadTable));
}

#[test]
fn the_catalog_is_written_by_the_home_site_or_a_site_allowed_some_table() {
    let mut m = provisioned(APP);
    // The home site: no grant asked.
    assert!(matches!(sign(&mut m, APP, &table(ALICE_DATA, CATALOG), 1, [1; 32]), Answer::Signed { .. }));
    // Another site: not until the person allows it some table.
    assert_eq!(unlock(&mut m, ALICE_PIN, OTHER), alice());
    assert_eq!(sign(&mut m, OTHER, &table(ALICE_DATA, CATALOG), 2, [2; 32]), Answer::Refused(Why::NotGranted { table: CATALOG.into() }));
    assert_eq!(ask(&mut m, OTHER, "notes", ALLOW), Answer::Granted { tables: vec!["notes".into()] });
    assert!(matches!(sign(&mut m, OTHER, &table(ALICE_DATA, CATALOG), 2, [2; 32]), Answer::Signed { .. }));
}

#[test]
fn a_table_key_is_given_to_the_home_site_and_to_a_site_only_once_granted() {
    let mut m = provisioned(APP);
    let key = |m: &mut Map, app, t: &str, gen| serve(m, Request::TableKey { table: t.into(), gen }, app);
    let Answer::TableKey { key: home } = key(&mut m, APP, "notes", 0) else { panic!("the home site holds its tables") };
    assert_eq!(home, table_key(&ALICE_DATA, "notes", 0), "derived from the data key");
    assert_ne!(home, table_key(&ALICE_DATA, "pins", 0), "each table its own");
    assert_ne!(home, table_key(&ALICE_DATA, "notes", 1), "each generation its own");
    // Another site: nothing until the person allows it that table; then the same key.
    assert_eq!(unlock(&mut m, ALICE_PIN, OTHER), alice());
    assert_eq!(key(&mut m, OTHER, "notes", 0), Answer::Refused(Why::NotGranted { table: "notes".into() }));
    assert_eq!(ask(&mut m, OTHER, "notes", ALLOW), Answer::Granted { tables: vec!["notes".into()] });
    assert_eq!(key(&mut m, OTHER, "notes", 0), Answer::TableKey { key: home });
    assert_eq!(key(&mut m, OTHER, "pins", 0), Answer::Refused(Why::NotGranted { table: "pins".into() }), "only what was granted");
    assert_eq!(key(&mut m, OTHER, "Bad!", 0), Answer::Refused(Why::BadTable));
}

#[test]
fn the_vault_keeps_mls_state_for_the_home_site_and_gives_epoch_table_keys_to_granted_sites() {
    let mut m = provisioned(APP);
    let at = |m: &mut Map, app, t: &str, epoch| serve(m, Request::TableKeyAt { table: t.into(), epoch, space: None }, app);
    assert_eq!(at(&mut m, APP, "notes", None), Answer::Refused(Why::NoEpoch), "nothing before the group exists");
    assert_eq!(serve(&mut m, Request::MlsLoad { space: None }, APP), Answer::MlsState { state: None });
    assert_eq!(serve(&mut m, Request::MlsSave { space: None, state: b"s1".to_vec(), epoch: 1, secret: [1; 32] }, APP), Answer::MlsSaved);
    assert_eq!(serve(&mut m, Request::MlsSave { space: None, state: b"s2".to_vec(), epoch: 2, secret: [2; 32] }, APP), Answer::MlsSaved);
    assert_eq!(serve(&mut m, Request::MlsLoad { space: None }, APP), Answer::MlsState { state: Some(b"s2".to_vec()) });
    // The newest epoch by default; an older one by name (blocks sealed then).
    assert_eq!(at(&mut m, APP, "notes", None), Answer::TableKeyAt { epoch: 2, key: epoch_table_key(&[2; 32], "notes") });
    assert_eq!(at(&mut m, APP, "notes", Some(1)), Answer::TableKeyAt { epoch: 1, key: epoch_table_key(&[1; 32], "notes") });
    assert_eq!(at(&mut m, APP, "notes", Some(7)), Answer::Refused(Why::NoEpoch));
    // An earlier epoch recovered from escrow: kept, readable by its number, the newest unchanged.
    assert_eq!(serve(&mut m, Request::EpochKeep { space: None, epoch: 0, secret: [9; 32] }, APP), Answer::MlsSaved);
    assert_eq!(at(&mut m, APP, "notes", Some(0)), Answer::TableKeyAt { epoch: 0, key: epoch_table_key(&[9; 32], "notes") });
    assert_eq!(at(&mut m, APP, "notes", None), Answer::TableKeyAt { epoch: 2, key: epoch_table_key(&[2; 32], "notes") });
    // Another site: no state at all, and table keys only for what it was granted.
    assert_eq!(unlock(&mut m, ALICE_PIN, OTHER), alice());
    assert_eq!(serve(&mut m, Request::MlsLoad { space: None }, OTHER), Answer::Refused(Why::NotHome));
    assert_eq!(serve(&mut m, Request::MlsSave { space: None, state: vec![], epoch: 3, secret: [3; 32] }, OTHER), Answer::Refused(Why::NotHome));
    assert_eq!(serve(&mut m, Request::EpochKeep { space: None, epoch: 3, secret: [3; 32] }, OTHER), Answer::Refused(Why::NotHome));
    assert_eq!(at(&mut m, OTHER, "notes", None), Answer::Refused(Why::NotGranted { table: "notes".into() }));
    assert_eq!(ask(&mut m, OTHER, "notes", ALLOW), Answer::Granted { tables: vec!["notes".into()] });
    assert_eq!(at(&mut m, OTHER, "notes", None), Answer::TableKeyAt { epoch: 2, key: epoch_table_key(&[2; 32], "notes") });
}

#[test]
fn a_removed_node_forgets_its_member_and_every_key_it_held() {
    let mut m = provisioned(APP);
    let at = |m: &mut Map, app, epoch| serve(m, Request::TableKeyAt { table: "notes".into(), epoch, space: None }, app);
    assert_eq!(serve(&mut m, Request::MlsSave { space: None, state: b"s".to_vec(), epoch: 2, secret: [2; 32] }, APP), Answer::MlsSaved);
    assert_eq!(serve(&mut m, Request::EpochKeep { space: None, epoch: 0, secret: [9; 32] }, APP), Answer::MlsSaved);
    assert_eq!(unlock(&mut m, ALICE_PIN, OTHER), alice());
    assert_eq!(ask(&mut m, OTHER, "notes", ALLOW), Answer::Granted { tables: vec!["notes".into()] });
    // Bob on the same node, as a control: untouched.
    assert_eq!(provision(&mut m, BOB, BOB_DID, BOB_PIN, APP), bob());
    assert_eq!(unlock(&mut m, ALICE_PIN, APP), alice());
    // Only the home site forgets.
    assert_eq!(unlock(&mut m, ALICE_PIN, OTHER), alice());
    assert_eq!(serve(&mut m, Request::Forget, OTHER), Answer::Refused(Why::NotHome));
    assert!(matches!(at(&mut m, APP, Some(0)), Answer::TableKeyAt { .. }), "control: the keys are there before");
    assert_eq!(serve(&mut m, Request::Forget, APP), Answer::LoggedOut);
    // Gone: its PIN opens nothing, no session anywhere, no state, no epoch, no grant.
    assert!(matches!(unlock(&mut m, ALICE_PIN, APP), Answer::WrongPin { .. }));
    assert_eq!(who(&mut m, OTHER), Answer::Refused(Why::NoSession));
    let alice_key = public(ALICE);
    for (k, v) in &m.s {
        if k.windows(32).any(|w| w == alice_key) {
            assert!(v.is_empty(), "a secret still names the forgotten member: {:?}", String::from_utf8_lossy(&k[..k.len().min(24)]));
        }
    }
    assert!(!m.s.values().any(|v| v.windows(32).any(|w| w == [2; 32] || w == [9; 32])), "an epoch secret is still held");
    // Bob still opens.
    assert_eq!(unlock(&mut m, BOB_PIN, APP), bob());
}

#[test]
fn a_members_keys_go_to_the_next_build_on_its_pin_to_its_home_only() {
    let mut m = provisioned(APP);
    assert_eq!(serve(&mut m, Request::MlsSave { space: None, state: b"group".to_vec(), epoch: 3, secret: [3; 32] }, APP), Answer::MlsSaved);
    assert_eq!(serve(&mut m, Request::EpochKeep { space: None, epoch: 1, secret: [1; 32] }, APP), Answer::MlsSaved);
    let keys = |m: &mut Map, pin: &str, app| serve(m, Request::HandoverKeys { pin: pin.into() }, app);
    assert_eq!(
        keys(&mut m, ALICE_PIN, APP),
        Answer::HandedKeys { mls: Some(b"group".to_vec()), epochs: vec![(1, [1; 32]), (3, [3; 32])] }
    );
    assert_eq!(keys(&mut m, ALICE_PIN, OTHER), Answer::Refused(Why::NotHome));
    assert!(matches!(keys(&mut m, "000000", APP), Answer::WrongPin { .. }), "a wrong PIN is a guess, counted");
    // A member with no group yet: nothing to hand.
    assert_eq!(provision(&mut m, BOB, BOB_DID, BOB_PIN, APP), bob());
    assert_eq!(keys(&mut m, BOB_PIN, APP), Answer::HandedKeys { mls: None, epochs: vec![] });
}

#[test]
fn the_members_table_is_read_with_any_grant_and_written_by_the_home_only() {
    let mut m = provisioned(APP);
    assert_eq!(unlock(&mut m, ALICE_PIN, OTHER), alice());
    let key = |m: &mut Map, app| serve(m, Request::TableKey { table: MEMBERS.into(), gen: 0 }, app);
    // Another site with no grant at all: nothing. With a grant for something else: the members' key too.
    assert_eq!(key(&mut m, OTHER), Answer::Refused(Why::NotGranted { table: MEMBERS.into() }));
    assert_eq!(ask(&mut m, OTHER, "notes", ALLOW), Answer::Granted { tables: vec!["notes".into()] });
    assert!(matches!(key(&mut m, OTHER), Answer::TableKey { .. }));
    // Writing it: the home site signs its own feed of it; another site, even granted, does not.
    let feed = params(public(ALICE), b"t/members");
    assert!(matches!(sign(&mut m, APP, &feed, 1, [1; 32]), Answer::Signed { .. }));
    assert_eq!(sign(&mut m, OTHER, &feed, 2, [2; 32]), Answer::Refused(Why::NotGranted { table: MEMBERS.into() }));
}

#[test]
fn an_epochs_log_is_signed_only_with_that_epochs_secret_by_the_home_site() {
    let mut m = provisioned(APP);
    assert_eq!(serve(&mut m, Request::MlsSave { space: None, state: b"s".to_vec(), epoch: 2, secret: [2; 32] }, APP), Answer::MlsSaved);
    assert_eq!(ask(&mut m, APP, "mls", ALLOW), Answer::Granted { tables: vec!["mls".into()] });
    let log = |secret: [u8; 32]| params(epoch_log_key(&secret).verifying_key().to_bytes(), b"t/mls");
    // Epoch 2's log: signed, and the signature holds under that log's key.
    let Answer::Signed { sig } = sign(&mut m, APP, &log([2; 32]), 1, [1; 32]) else { panic!("epoch 2's log is signed") };
    let vk = epoch_log_key(&[2; 32]).verifying_key();
    assert!(vk.verify(&Params::parse(&log([2; 32])).unwrap().signed_message(false, 1, &[1; 32]), &Signature::from_slice(&sig).unwrap()).is_ok());
    // An epoch whose secret it does not hold (a removed node never gets the next one): refused.
    assert_eq!(sign(&mut m, APP, &log([3; 32]), 1, [1; 32]), Answer::Refused(Why::NotThisKey));
    // Another site, even for a held epoch: refused.
    assert_eq!(unlock(&mut m, ALICE_PIN, OTHER), alice());
    assert_eq!(ask(&mut m, OTHER, "mls", ALLOW), Answer::Granted { tables: vec!["mls".into()] });
    assert_eq!(sign(&mut m, OTHER, &log([2; 32]), 2, [2; 32]), Answer::Refused(Why::NotThisKey));
}

#[test]
fn a_joiner_signs_the_log_of_an_epoch_it_kept_before_it_has_a_group() {
    let mut m = provisioned(APP);
    assert_eq!(ask(&mut m, APP, "mls", ALLOW), Answer::Granted { tables: vec!["mls".into()] });
    let log = |secret: [u8; 32]| params(epoch_log_key(&secret).verifying_key().to_bytes(), b"t/mls");
    // Control: nothing kept, nothing signed.
    assert_eq!(sign(&mut m, APP, &log([4; 32]), 1, [1; 32]), Answer::Refused(Why::NotThisKey));
    // Epochs 3 and 4 kept from the walk (no group state yet): epoch 4's log is signed, and 3's too.
    assert_eq!(serve(&mut m, Request::EpochKeep { space: None, epoch: 3, secret: [3; 32] }, APP), Answer::MlsSaved);
    assert_eq!(serve(&mut m, Request::EpochKeep { space: None, epoch: 4, secret: [4; 32] }, APP), Answer::MlsSaved);
    assert!(matches!(sign(&mut m, APP, &log([4; 32]), 1, [1; 32]), Answer::Signed { .. }));
    assert!(matches!(sign(&mut m, APP, &log([3; 32]), 1, [1; 32]), Answer::Signed { .. }));
    // Keeping an OLDER epoch later does not move the newest back.
    assert_eq!(serve(&mut m, Request::EpochKeep { space: None, epoch: 1, secret: [1; 32] }, APP), Answer::MlsSaved);
    assert_eq!(serve(&mut m, Request::TableKeyAt { table: "mls".into(), epoch: None, space: None }, APP), Answer::TableKeyAt { epoch: 4, key: epoch_table_key(&[4; 32], "mls") });
}

#[test]
fn each_space_keeps_its_own_group_and_epochs_apart_and_forget_clears_them_all() {
    let mut m = provisioned(APP);
    let (x, y) = (Some([0x51; 32]), Some([0x52; 32]));
    let save = |m: &mut Map, space, state: &[u8], epoch, secret| serve(m, Request::MlsSave { space, state: state.to_vec(), epoch, secret }, APP);
    let load = |m: &mut Map, space| serve(m, Request::MlsLoad { space }, APP);
    let key = |m: &mut Map, space, epoch| serve(m, Request::TableKeyAt { table: "chat".into(), epoch, space }, APP);
    assert_eq!(save(&mut m, None, b"account", 4, [4; 32]), Answer::MlsSaved);
    assert_eq!(save(&mut m, x, b"server x", 1, [0x11; 32]), Answer::MlsSaved);
    assert_eq!(save(&mut m, y, b"server y", 7, [0x77; 32]), Answer::MlsSaved);
    // Each its own state, newest epoch and keys.
    assert_eq!(load(&mut m, None), Answer::MlsState { state: Some(b"account".to_vec()) });
    assert_eq!(load(&mut m, x), Answer::MlsState { state: Some(b"server x".to_vec()) });
    assert_eq!(key(&mut m, x, None), Answer::TableKeyAt { epoch: 1, key: epoch_table_key(&[0x11; 32], "chat") });
    assert_eq!(key(&mut m, y, None), Answer::TableKeyAt { epoch: 7, key: epoch_table_key(&[0x77; 32], "chat") });
    assert_eq!(key(&mut m, x, Some(4)), Answer::Refused(Why::NoEpoch), "the account's epoch 4 is not server x's");
    // Another site: no space's keys, even a granted table name.
    assert_eq!(unlock(&mut m, ALICE_PIN, OTHER), alice());
    assert_eq!(ask(&mut m, OTHER, "chat", ALLOW), Answer::Granted { tables: vec!["chat".into()] });
    assert_eq!(serve(&mut m, Request::TableKeyAt { table: "chat".into(), epoch: None, space: x }, OTHER), Answer::Refused(Why::NotGranted { table: "chat".into() }));
    // A space's epoch log is signed in its own space only.
    let log = params(epoch_log_key(&[0x11; 32]).verifying_key().to_bytes(), b"t/chat");
    assert!(matches!(serve(&mut m, Request::Sign { params: log.clone(), seq: 1, value_hash: [1; 32], space: x }, APP), Answer::Signed { .. }));
    assert_eq!(serve(&mut m, Request::Sign { params: log, seq: 2, value_hash: [2; 32], space: y }, APP), Answer::Refused(Why::NotThisKey));
    // Forget: every space's group goes with the member.
    assert_eq!(unlock(&mut m, ALICE_PIN, APP), alice());
    assert_eq!(serve(&mut m, Request::Forget, APP), Answer::LoggedOut);
    assert!(!m.s.values().any(|v| v.windows(8).any(|w| w == b"server x" || w == b"server y")), "a space's state is still held");
    assert!(!m.s.values().any(|v| v.windows(32).any(|w| w == [0x11; 32] || w == [0x77; 32])), "a space's epoch secret is still held");
}

#[test]
fn the_inbox_is_opened_by_the_accounts_nodes_for_the_home_site_only() {
    let mut m = provisioned(APP);
    let Answer::InboxKey { public } = serve(&mut m, Request::InboxKey, APP) else { panic!("an inbox key") };
    assert_eq!(public, inbox_public(&ALICE_DATA), "from the account's data key: the same on every node");
    let sealed = seal_to(&public, b"a welcome", [3; 32]);
    let other = seal_to(&inbox_public(&BOB_DATA), b"not alice's", [4; 32]);
    let open = |m: &mut Map, app| serve(m, Request::InboxOpen { items: vec![sealed.clone(), other.clone(), b"junk".to_vec()] }, app);
    assert_eq!(open(&mut m, APP), Answer::Opened { items: vec![Some(b"a welcome".to_vec()), None, None] });
    assert_eq!(unlock(&mut m, ALICE_PIN, OTHER), alice());
    assert_eq!(open(&mut m, OTHER), Answer::Refused(Why::NotHome));
}

#[test]
fn a_members_spaces_go_to_the_next_build_on_its_pin_to_its_home_only() {
    let mut m = provisioned(APP);
    let x = [0x51; 32];
    assert_eq!(serve(&mut m, Request::MlsSave { space: Some(x), state: b"server x".to_vec(), epoch: 2, secret: [2; 32] }, APP), Answer::MlsSaved);
    assert_eq!(serve(&mut m, Request::EpochKeep { space: Some(x), epoch: 0, secret: [9; 32] }, APP), Answer::MlsSaved);
    // The account's own group is not a space: not in the list.
    assert_eq!(serve(&mut m, Request::MlsSave { space: None, state: b"account".to_vec(), epoch: 1, secret: [1; 32] }, APP), Answer::MlsSaved);
    let spaces = |m: &mut Map, pin: &str, app| serve(m, Request::HandoverSpaces { pin: pin.into() }, app);
    assert_eq!(
        spaces(&mut m, ALICE_PIN, APP),
        Answer::HandedSpaces { spaces: vec![(x, Some(b"server x".to_vec()), vec![(0, [9; 32]), (2, [2; 32])])] }
    );
    assert_eq!(spaces(&mut m, ALICE_PIN, OTHER), Answer::Refused(Why::NotHome));
    assert!(matches!(spaces(&mut m, "000000", APP), Answer::WrongPin { .. }));
}
