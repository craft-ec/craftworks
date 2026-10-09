//! A site record VERIFIES only as exactly what it claims (builder#117 Q1): `verified_record` is what lets the
//! signer take a page-named contract as truth, so every way to forge one must read as nothing -- and a record that
//! survived a same-version race (the Register keeps its fork evidence) still reads.
use contract_keys::register::head_state;
use contract_keys::site::site_params;
use signer::verified_record;

fn params() -> (Vec<u8>, [u8; 32]) {
    let sk = ed25519_dalek::SigningKey::from_bytes(&[3u8; 32]);
    let mut p = b"RG01".to_vec();
    p.push(0);
    p.extend_from_slice(&sk.verifying_key().to_bytes());
    p.extend_from_slice(b"head");
    (p, sk.to_bytes())
}

#[test]
fn a_signed_site_record_verifies_under_its_own_params_only() {
    let (head, key) = params();
    let site = site_params(&head, "notes").expect("site params");
    let value = [9u8; 32];
    let st = head_state(&site, &key, 4, &value).expect("signs");
    assert_eq!(verified_record(&site, &st), Some((4, value.to_vec())), "THE CONTROL: a real record does not verify");
    assert_eq!(verified_record(&head, &st), None, "verified under the head's params");
    assert_eq!(verified_record(&site_params(&head, "other").unwrap(), &st), None, "verified under another app's params");
    let other = { let sk = ed25519_dalek::SigningKey::from_bytes(&[4u8; 32]); let mut p = b"RG01".to_vec(); p.push(0); p.extend_from_slice(&sk.verifying_key().to_bytes()); p.extend_from_slice(b"site:notes"); p };
    assert_eq!(verified_record(&other, &st), None, "verified under another authority");
    // Tampered: the value, the seq, the signature, a trailing byte, the evidence flag, a terminal record.
    let at_value = 4 + 1 + 1 + 8 + 2;
    for (what, i) in [("value", at_value), ("seq", 6), ("signature", st.len() - 1)] {
        let mut t = st.clone();
        t[i] ^= 1;
        assert_eq!(verified_record(&site, &t), None, "a record with its {what} changed verified");
    }
    let mut longer = st.clone();
    longer.push(0);
    assert_eq!(verified_record(&site, &longer), None, "a record with a trailing byte verified");
    let mut evidence = st.clone();
    evidence[4] = 0b11;
    assert_eq!(verified_record(&site, &evidence), None, "a record claiming fork evidence it does not carry verified");
    let mut terminal = st.clone();
    terminal[5] = 1;
    assert_eq!(verified_record(&site, &terminal), None, "a record whose terminal flag was flipped under its signature verified");
}

/// The Register's own merge of two records at ONE version: the winner and the (sticky) fork evidence.
fn raced(site: &[u8], key: &[u8; 32], seq: u64) -> (Vec<u8>, [u8; 32]) {
    use freenet_stdlib::prelude::*;
    let (a, b) = ([1u8; 32], [2u8; 32]);
    let (sa, sb) = (head_state(site, key, seq, &a).expect("signs"), head_state(site, key, seq, &b).expect("signs"));
    let merged = <craftec_register_contract::Register as ContractInterface>::update_state(
        Parameters::from(site.to_vec()),
        State::from(sa),
        vec![UpdateData::State(State::from(sb))],
    )
    .expect("merges")
    .new_state
    .expect("a state")
    .as_ref()
    .to_vec();
    let winner = if blake3::hash(&a).as_bytes() < blake3::hash(&b).as_bytes() { a } else { b };
    (merged, winner)
}

/// **AFTER A SAME-VERSION RACE THE SITE STILL READS** (architect, sdk#363): the Register keeps the winner AND the
/// fork evidence, for ever. A reader that refused any state carrying evidence would leave this device's signer
/// unable to read its site again -- signing at or below the live version, Superseded on every publish. The record
/// counts when it verifies; the evidence does not stop it. Mutant "reject a state with evidence" -> red.
#[test]
fn a_verified_record_beside_fork_evidence_counts() {
    let (head, key) = params();
    let site = site_params(&head, "notes").expect("site params");
    let (merged, winner) = raced(&site, &key, 5);
    let (_, s) = craftec_register_contract::read(&site, &merged).expect("THE SETUP: the Register does not read its own merge");
    assert!(s.evidence.is_some(), "THE SETUP: the race left no fork evidence");
    assert_eq!(verified_record(&site, &merged), Some((5, winner.to_vec())), "a site that survived a race does not read");
    // A FORGED record beside the same evidence: its value changed under its signature.
    let mut forged = merged.clone();
    forged[4 + 1 + 1 + 8 + 2] ^= 1;
    assert_eq!(verified_record(&site, &forged), None, "a forged record beside real evidence counted");
    // And under another app's params nothing in it verifies.
    assert_eq!(verified_record(&site_params(&head, "other").unwrap(), &merged), None);
}
