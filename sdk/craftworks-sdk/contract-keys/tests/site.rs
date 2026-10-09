//! A site's params: the ONE relabelling (builder#117). Reading a site record is the signer's (`signer::verified_record`,
//! through the Register crate); these are the params it is verified under.
use contract_keys::site::{relabel, site_params};

fn params() -> (Vec<u8>, [u8; 32]) {
    let sk = ed25519_dalek::SigningKey::from_bytes(&[3u8; 32]);
    let mut p = b"RG01".to_vec();
    p.push(0);
    p.extend_from_slice(&sk.verifying_key().to_bytes());
    p.extend_from_slice(b"head");
    (p, sk.to_bytes())
}

#[test]
fn relabel_keeps_the_authority_and_refuses_what_it_cannot_sign() {
    let (head, _) = params();
    let site = relabel(&head, b"site:notes").expect("mode 0");
    assert_eq!(&site[..37], &head[..37], "the authority changed");
    assert_eq!(&site[37..], b"site:notes");
    let mut quorum = head.clone();
    quorum[4] = 1;
    assert_eq!(relabel(&quorum, b"site:notes"), None, "a k-of-n keyset was relabelled (Phase 6)");
    assert_eq!(site_params(&head, "Not An App"), None, "an app id outside the one rule was taken");
}
