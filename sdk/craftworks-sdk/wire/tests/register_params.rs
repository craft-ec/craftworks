//! THE ONE RG01 WRITER outside the Register crate (sdk#364): `wire::register_params`. The crate parses params but
//! has no encoder, so they are laid out here -- and read back by the crate's own parser, or the id every page derives
//! from them addresses a register the contract cannot parse.
use craftec_register_contract::wire::{Authority, Params};

#[test]
fn the_one_params_writer_round_trips_through_the_register_crate() {
    for (seed, label) in [(1u8, wire::HEAD_NAME), (2, b"site:notes".as_slice()), (3, b"".as_slice())] {
        let key = ed25519_dalek::SigningKey::from_bytes(&[seed; 32]).verifying_key().to_bytes();
        let bytes = wire::register_params(&key, label);
        let p = Params::parse(&bytes).expect("the Register crate does not parse the SDK's params");
        match p.authority {
            Authority::One(k) => assert_eq!(k.to_bytes(), key, "another writer key"),
            other => panic!("not mode 0: {other:?}"),
        }
        assert_eq!(p.label, label, "another label");
        assert_eq!(p.hash, *blake3::hash(&bytes).as_bytes(), "the hash signatures bind to is not of these bytes");
    }
}

/// THE OBSERVATION TREE's HEAD has ONE derivation (sdk#399): the page's `register_params(identity, OBS_NAME)` and the
/// signer's relabel of the person's head params (`contract_keys::site::obs_params`, which can't depend on wire) are the
/// same bytes, and so the same contract. A drift would sign one register and read another.
#[test]
fn the_obs_head_params_are_one_derivation_on_both_sides() {
    for seed in [1u8, 2, 3] {
        let key = ed25519_dalek::SigningKey::from_bytes(&[seed; 32]).verifying_key().to_bytes();
        let head = wire::register_params(&key, wire::HEAD_NAME);
        let obs = wire::register_params(&key, wire::OBS_NAME);
        assert_eq!(contract_keys::site::obs_params(&head), Some(obs.clone()), "the signer derives another obs register");
        assert_ne!(obs, head, "the obs head is the data head");
    }
}
