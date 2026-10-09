//! `webapp::params` and the framing, judged by the WEBAPP CONTRACT ITSELF
//! (`craftec_webapp_contract::check`, the one check its every entry point
//! runs). Our address derivation and our manifest both go through `params`,
//! so they agree with each other whatever `params` computes; only the
//! contract can say a PUT would be Valid.

use craftec_webapp_contract::check;
use wire::webapp::{app_container, container, params};

fn containers() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("an app container", app_container(&[("index.html", b"<!doctype html>".as_slice()), ("app.json", b"{}")]).expect("packs")),
        ("a container with metadata", container(b"{\"v\":1}", b"any web bytes")),
    ]
}

#[test]
fn the_contract_accepts_what_this_crate_publishes() {
    for (what, state) in containers() {
        assert!(check(&params(&state), &state), "the webapp contract refuses {what} under our params");
    }
}

/// THE CONTROLS: the contract refuses a params one bit off, and another
/// state's params — so "accepted" above is a check that can fail.
#[test]
fn control_the_contract_refuses_wrong_params() {
    let (_, state) = &containers()[0];
    let mut p = params(state);
    p[0] ^= 1;
    assert!(!check(&p, state), "a params one bit off was accepted");
    let (_, other) = &containers()[1];
    assert!(!check(&params(other), state), "another state's params were accepted");
}
