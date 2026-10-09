//! A SITE THROUGH THE ONE SIGN VERB (builder#117): `Sign { prev, next, label: Site { app, contract } }` is decided
//! by the head's own rule (`decide`), under the SAME authority relabelled `site:<app>`, with its own record. No
//! site verb, record type or stage machine of its own.
use contract_keys::site::{frame, site_params};
use signer::*;
use std::collections::BTreeMap;

const RCODE: &[u8] = b"a register contract's code, as provisioned";
const BCODE: &[u8] = b"a block contract's code, as provisioned";
const SITE: [u8; 32] = [0x51; 32];

#[derive(Default, Clone)]
struct Mem {
    secrets: BTreeMap<Vec<u8>, Vec<u8>>,
    states: BTreeMap<[u8; 32], Vec<u8>>,
}
impl Host for Mem {
    fn get_secret(&self, k: &[u8]) -> Option<Vec<u8>> {
        self.secrets.get(k).cloned()
    }
    fn set_secret(&mut self, k: &[u8], v: &[u8]) -> bool {
        self.secrets.insert(k.to_vec(), v.to_vec());
        true
    }
    fn contract_state(&self, id: &[u8; 32]) -> Option<Vec<u8>> {
        self.states.get(id).cloned()
    }
}

fn provisioned() -> (Mem, Vec<u8>) {
    let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
    let params = wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME);
    let mut host = Mem::default();
    let req = Request::Provision { signing_key: sk.to_bytes().to_vec(), register_code: RCODE.to_vec(), register_params: params.clone(), block_code: BCODE.to_vec() };
    assert_eq!(serve(&mut host, &encode_request(1, &req), Origin::Local), Answer::Provisioned);
    (host, params)
}

fn web(n: u8) -> Vec<u8> {
    vec![n; 100]
}
fn bundle(n: u8) -> [u8; 32] {
    *blake3::hash(&web(n)).as_bytes()
}
fn site(app: &str) -> Label {
    Label::Site { app: app.into(), contract: SITE }
}
fn ask(host: &mut Mem, prev: (u64, [u8; 32]), seq: u64, n: u8, label: Label, origin: Origin) -> Answer {
    let req = Request::Sign { prev: Head { seq: prev.0, root: prev.1 }, next: Next { seq, root: bundle(n), ledger: vec![] }, label };
    serve(host, &encode_request(2, &req), origin)
}
const GENESIS: (u64, [u8; 32]) = (0, [0u8; 32]);

/// **A site is signed as a Register record under the relabelled params, kept under its OWN record, and the head's
/// record is untouched.**
#[test]
fn a_site_is_signed_under_its_own_label_and_record() {
    let (mut host, params) = provisioned();
    let Answer::Signed(state) = ask(&mut host, GENESIS, 1, 1, site("notes"), Origin::Local) else { panic!("the site was not signed") };
    let sp = site_params(&params, "notes").expect("site params");
    assert_eq!(verified_record(&sp, &state), Some((1, bundle(1).to_vec())), "the signature does not verify as the site's record (1, blake3(web))");
    assert_eq!(verified_record(&params, &state), None, "a site signature verified under the HEAD's params: the label does not separate them");
    assert!(host.get_secret(&record_name(&site("notes"))).is_some(), "the site's record was not kept");
    assert!(host.get_secret(RECORD).is_none(), "a site signature wrote the HEAD's record");
    // One record per label: another app, and the head, each start from their own genesis.
    assert!(matches!(ask(&mut host, GENESIS, 1, 9, site("other"), Origin::Local), Answer::Signed(_)), "another app's site shares this one's record");
}

/// **Rule 13: a served app never gets a site signed** (it could republish its visitor's site), and nothing is
/// recorded. The head's label is left as it was (#318). Mutant "no origin check" -> red.
#[test]
fn a_served_origin_is_refused_a_site_and_nothing_is_recorded() {
    let (mut host, _) = provisioned();
    assert_eq!(ask(&mut host, GENESIS, 1, 1, site("notes"), Origin::Served), Answer::Refused(Why::FromApp));
    assert!(host.get_secret(&record_name(&site("notes"))).is_none(), "a refused site request left a record");
    // THE CONTROL: the same request from the person's own tools is signed.
    assert!(matches!(ask(&mut host, GENESIS, 1, 1, site("notes"), Origin::Local), Answer::Signed(_)));
}

/// A label naming no app id is refused by name (the ONE rule, `core_types::name::app_ok`).
#[test]
fn a_site_label_with_no_app_id_is_refused() {
    let (mut host, _) = provisioned();
    for bad in ["", "Notes", "a b", "x".repeat(33).as_str()] {
        assert_eq!(ask(&mut host, GENESIS, 1, 1, site(bad), Origin::Local), Answer::Refused(Why::BadLabel), "app id {bad:?}");
    }
}

/// **THE LIVELOCK (open item 1) ends through the head's own rule, with no new answer.** The signer recorded v1 with
/// bundle A, and A never landed. A publish of B reads the site at genesis and asks for v1: rule (b) answers A's
/// record (AlreadySigned). The page, seeing a value that is not B's, asks FROM the record (v1, A): rule (c) signs
/// v2 with B. Two asks; each moves prev up to a seq the signer recorded. Mutant (the page) "ask the same prev
/// again" never gets past the first answer -- asserted here as: the same prev is answered A's record every time.
#[test]
fn a_recorded_version_that_never_landed_is_passed_by_asking_from_it() {
    let (mut host, params) = provisioned();
    let Answer::Signed(a) = ask(&mut host, GENESIS, 1, 1, site("notes"), Origin::Local) else { panic!("v1 A") };
    // B, from the node's view (genesis): the record's bytes come back, not a signature for B.
    for _ in 0..3 {
        assert_eq!(ask(&mut host, GENESIS, 1, 2, site("notes"), Origin::Local), Answer::AlreadySigned(a.clone()), "the same prev must keep answering the first record");
    }
    let (seq, value) = verified_record(&site_params(&params, "notes").unwrap(), &a).expect("A's record verifies");
    let Answer::Signed(b) = ask(&mut host, (seq, value.as_slice().try_into().unwrap()), 2, 2, site("notes"), Origin::Local) else {
        panic!("asking from the record did not sign the next version")
    };
    assert_eq!(verified_record(&site_params(&params, "notes").unwrap(), &b), Some((2, bundle(2).to_vec())));
}

/// **Q1 (the architect): the site contract the page names is truth ONLY if what it holds VERIFIES.** An unsigned
/// record there with a huge seq is ignored (the signer signs from its record); a genuinely signed newer one is the
/// truth (a legitimate skip: NotNext to it). Mutant "skip the verify" -> red on the unsigned case.
#[test]
fn the_named_site_contract_counts_only_when_its_record_verifies() {
    let (mut host, params) = provisioned();
    let sp = site_params(&params, "notes").unwrap();
    // An UNSIGNED record claiming seq 99 (the right layout, a zero signature).
    let mut forged = b"RG01".to_vec();
    forged.extend_from_slice(&[0x01, 0]);
    forged.extend_from_slice(&99u64.to_le_bytes());
    forged.extend_from_slice(&32u16.to_le_bytes());
    forged.extend_from_slice(&bundle(9));
    forged.extend_from_slice(&[0u8; 64]);
    host.states.insert(SITE, frame(&forged, &web(9)));
    assert!(matches!(ask(&mut host, GENESIS, 1, 1, site("notes"), Origin::Local), Answer::Signed(_)), "an UNSIGNED record at the named contract was taken as the truth");
    // A genuinely signed v5 (another device of the same authority): the truth.
    let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
    let v5 = contract_keys::register::head_state(&sp, &sk.to_bytes(), 5, &bundle(5)).expect("signs");
    host.states.insert(SITE, frame(&v5, &web(5)));
    let (mut fresh, _) = provisioned();
    fresh.states.insert(SITE, frame(&v5, &web(5)));
    assert_eq!(ask(&mut fresh, GENESIS, 1, 1, site("notes"), Origin::Local), Answer::NotNext { current: Head { seq: 5, root: bundle(5) } }, "a genuinely signed newer site was not the truth");
}

/// FAIL CLOSED: a record that is there and does not decode refuses, rather than reading as "none" and signing
/// any version again (sdk#332's review).
#[test]
fn an_undecodable_site_record_refuses() {
    let (mut host, _) = provisioned();
    host.secrets.insert(record_name(&site("notes")), b"garbage".to_vec());
    assert_eq!(ask(&mut host, GENESIS, 1, 1, site("notes"), Origin::Local), Answer::Refused(Why::CannotSign));
}

/// THE NODE'S ENTRY PASSES THE ATTESTED ORIGIN (the delegate cannot run natively, so its source is read): an
/// attested origin -- a served web app, or a delegate that may be relaying one -- is `Served`; none is `Local`.
/// Mutant "the entry passes Local always" -> red.
#[test]
fn the_delegate_entry_passes_the_attested_origin() {
    let src = include_str!("../src/delegate.rs");
    assert!(src.contains("fn process("), "THE CONTROL: the reader did not find the delegate's entry");
    assert!(src.contains("let who = if origin.is_some() { crate::Origin::Served } else { crate::Origin::Local };"), "the entry does not map the node's attested origin");
    assert!(src.contains("crate::serve_full(&mut Ctx(ctx), &m.payload, who)"), "the entry does not pass the origin it read");
    assert!(!src.contains("_origin"), "the entry ignores the origin again");
}
