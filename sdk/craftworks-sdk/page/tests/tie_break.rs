//! `page::beats` IS the Register contract's equal-seq rule: pinned against the
//! contract's OWN merge (main's condition on the ledger format), on values that
//! differ in root, and on values that share a root and differ only in ledger.

use craftec_register_contract::Register;
use contract_keys::register::head_state;
use freenet_stdlib::prelude::*;
use signer_proto::head::{record_of, value, Ledger};

fn params() -> Vec<u8> {
    let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
    wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME)
}

/// The value the contract keeps when `a` is held and `b` arrives (and the other way round).
fn contract_winner(a: &[u8], b: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let p = params();
    let sa = head_state(&p, &[7u8; 32], 5, a).expect("signs");
    let sb = head_state(&p, &[7u8; 32], 5, b).expect("signs");
    let merge = |held: &[u8], arriving: &[u8]| {
        let m = <Register as ContractInterface>::update_state(
            Parameters::from(p.clone()),
            State::from(held.to_vec()),
            vec![UpdateData::State(State::from(arriving.to_vec()))],
        )
        .expect("merges")
        .new_state
        .expect("a state");
        record_of(m.as_ref()).expect("a record").1.to_vec()
    };
    (merge(&sa, &sb), merge(&sb, &sa))
}

#[test]
fn the_page_picks_the_value_the_register_keeps() {
    let mut pairs: Vec<(Vec<u8>, Vec<u8>)> = (0u8..24).map(|i| (vec![i; 32], vec![i.wrapping_add(101); 32])).collect();
    // One root, two ledgers (the architect's #1: two devices landing the same tree).
    for i in 0u8..8 {
        let root = [i; 32];
        let a = value(&root, &Ledger { prev: Some(signer_proto::Head { seq: 4, root: [1; 32] }), ..Ledger::default() });
        let b = value(&root, &Ledger { prev: Some(signer_proto::Head { seq: 4, root: [2; 32] }), ..Ledger::default() });
        pairs.push((a, b));
    }
    let mut each_side = [0, 0];
    for (a, b) in pairs {
        let (w1, w2) = contract_winner(&a, &b);
        assert_eq!(w1, w2, "the contract's merge depends on order");
        let page_pick = if page::beats(&a, &b) { a.clone() } else { b.clone() };
        assert_eq!(page_pick, w1, "page::beats disagrees with the Register");
        each_side[usize::from(page_pick == a)] += 1;
    }
    assert!(each_side[0] > 0 && each_side[1] > 0, "every pair went one way: the test cannot tell a rule from a constant");
}
