use super::*;
use ed25519_dalek::{Signer, SigningKey};
use std::collections::HashMap;

const CODE: [u8; 32] = [9; 32];

fn key(n: u8) -> SigningKey {
    SigningKey::from_bytes(&[n; 32])
}
fn pk(n: u8) -> [u8; 32] {
    key(n).verifying_key().to_bytes()
}
fn def() -> Definition {
    Definition { code_hash: CODE, decimals: 8, min_send: 10, genesis_owner: pk(1), genesis_amount: 1000, name: b"T".to_vec(), nonce: [0; 16] }
}
fn params(owner: u8) -> ChainParams {
    ChainParams { code_hash: CODE, definition: id(&CODE, &def().encode()), owner: pk(owner) }
}
fn nonce(n: u8) -> [u8; 16] {
    [n; 16]
}

/// The balance after `body` on `s` (no checks: tests build wrong chains on purpose too).
fn after(s: &State, body: &Body) -> u128 {
    let b = s.balance();
    match body {
        Body::Genesis { amount } => *amount,
        Body::Send { amount, .. } | Body::Burn { amount } => b.wrapping_sub(*amount),
        Body::Receive { amount, .. } => b + amount,
    }
}
fn entry(s: &State, n: [u8; 16], body: Body) -> Entry {
    let balance = after(s, &body);
    Entry { seq: s.entries.len() as u64, prev: s.head(), nonce: n, body, balance, auth: Auth::Work(0) }
}
fn sign(mut e: Entry, by: u8) -> Entry {
    e.auth = Auth::Sig(key(by).sign(&e.message()).to_bytes());
    e
}
fn work(mut e: Entry) -> Entry {
    let m = e.message();
    let w = (0u64..).find(|w| work_ok(&m, *w)).unwrap();
    e.auth = Auth::Work(w);
    e
}
fn signed(s: &mut State, by: u8, n: u8, body: Body) {
    let e = sign(entry(s, nonce(n), body), by);
    s.entries.push(e);
}
fn received(s: &mut State, from: u8, n: u8, amount: u128) {
    let e = work(entry(s, nonce(n), Body::Receive { from: pk(from), nonce: nonce(n), amount }));
    s.entries.push(e);
}

struct World(HashMap<[u8; 32], State>);
impl World {
    fn check(&self, owner: u8) -> Result<(), Why> {
        let s = &self.0[&pk(owner)];
        check(&params(owner), &def(), s, &|o| self.0.get(o).cloned())
    }
}

/// A (1) holds the genesis and sends 300 to B (2); B receives it.
fn paid() -> World {
    let mut a = State::default();
    signed(&mut a, 1, 1, Body::Genesis { amount: 1000 });
    signed(&mut a, 1, 2, Body::Send { to: pk(2), amount: 300 });
    let mut b = State::default();
    received(&mut b, 1, 2, 300);
    World([(pk(1), a), (pk(2), b)].into_iter().collect())
}

#[test]
fn a_payment_is_well_formed_on_both_sides() {
    let w = paid();
    assert_eq!(w.check(1), Ok(()));
    assert_eq!(w.check(2), Ok(()));
    assert_eq!(w.0[&pk(1)].balance(), 700);
    assert_eq!(w.0[&pk(2)].balance(), 300);
}

#[test]
fn a_debit_over_the_balance_is_refused() {
    let mut w = paid();
    let a = w.0.get_mut(&pk(1)).unwrap();
    signed(a, 1, 3, Body::Send { to: pk(3), amount: 701 });
    assert_eq!(w.check(1), Err("a debit over the balance"));
    let mut w = paid();
    let b = w.0.get_mut(&pk(2)).unwrap();
    signed(b, 2, 4, Body::Burn { amount: 301 });
    assert_eq!(w.check(2), Err("a debit over the balance"));
}

#[test]
fn a_wrong_carried_balance_is_refused() {
    let mut w = paid();
    let a = w.0.get_mut(&pk(1)).unwrap();
    let mut e = entry(a, nonce(3), Body::Send { to: pk(3), amount: 100 });
    e.balance += 1;
    a.entries.push(sign(e, 1));
    assert_eq!(w.check(1), Err("a carried balance that does not add up"));
}

#[test]
fn a_send_under_the_minimum_or_to_itself_is_refused() {
    let mut w = paid();
    signed(w.0.get_mut(&pk(1)).unwrap(), 1, 3, Body::Send { to: pk(3), amount: 9 });
    assert_eq!(w.check(1), Err("a send under the minimum"));
    let mut w = paid();
    signed(w.0.get_mut(&pk(1)).unwrap(), 1, 3, Body::Send { to: pk(1), amount: 50 });
    assert_eq!(w.check(1), Err("a send to itself"));
}

#[test]
fn only_the_writer_signs_what_spends() {
    let mut w = paid();
    signed(w.0.get_mut(&pk(1)).unwrap(), 2, 3, Body::Send { to: pk(3), amount: 50 });
    assert_eq!(w.check(1), Err("an entry not signed by the chain's writer"));
}

#[test]
fn a_broken_link_or_seq_is_refused() {
    let mut w = paid();
    let a = w.0.get_mut(&pk(1)).unwrap();
    let mut e = entry(a, nonce(3), Body::Burn { amount: 10 });
    e.prev = [7; 32];
    a.entries.push(sign(e, 1));
    assert_eq!(w.check(1), Err("a broken link"));
    let mut w = paid();
    let a = w.0.get_mut(&pk(1)).unwrap();
    let mut e = entry(a, nonce(3), Body::Burn { amount: 10 });
    e.seq += 1;
    a.entries.push(sign(e, 1));
    assert_eq!(w.check(1), Err("a seq out of place"));
}

#[test]
fn only_the_definitions_genesis_comes_from_nothing() {
    let mut s = State::default();
    signed(&mut s, 3, 1, Body::Genesis { amount: 1000 });
    let w = World([(pk(3), s)].into_iter().collect());
    assert_eq!(w.check(3), Err("a genesis that is not the definition's"));
    let mut s = State::default();
    signed(&mut s, 1, 1, Body::Genesis { amount: 1001 });
    let w = World([(pk(1), s)].into_iter().collect());
    assert_eq!(w.check(1), Err("a genesis that is not the definition's"));
}

#[test]
fn a_receive_needs_its_work() {
    let mut w = paid();
    let b = w.0.get_mut(&pk(2)).unwrap();
    let mut e = b.entries.pop().unwrap();
    let m = e.message();
    e.auth = Auth::Work((0u64..).find(|x| !work_ok(&m, *x)).unwrap());
    b.entries.push(e);
    assert_eq!(w.check(2), Err("a receive without its work"));
}

#[test]
fn a_receive_must_find_its_send() {
    // No such send.
    let mut w = paid();
    received(w.0.get_mut(&pk(2)).unwrap(), 1, 9, 50);
    assert_eq!(w.check(2), Err("a receive whose send is not in its sender's chain"));
    // The send is to someone else.
    let mut w = paid();
    signed(w.0.get_mut(&pk(1)).unwrap(), 1, 3, Body::Send { to: pk(3), amount: 50 });
    received(w.0.get_mut(&pk(2)).unwrap(), 1, 3, 50);
    assert_eq!(w.check(2), Err("a receive whose send is not in its sender's chain"));
    // A different amount.
    let mut w = paid();
    signed(w.0.get_mut(&pk(1)).unwrap(), 1, 3, Body::Send { to: pk(2), amount: 50 });
    received(w.0.get_mut(&pk(2)).unwrap(), 1, 3, 60);
    assert_eq!(w.check(2), Err("a receive whose send is not in its sender's chain"));
    // No sender chain at all.
    let mut w = paid();
    received(w.0.get_mut(&pk(2)).unwrap(), 5, 1, 50);
    assert_eq!(w.check(2), Err("the sender's chain is not there"));
}

#[test]
fn a_send_is_received_once() {
    let mut w = paid();
    received(w.0.get_mut(&pk(2)).unwrap(), 1, 2, 300);
    assert_eq!(w.check(2), Err("a send received twice"));
}

#[test]
fn an_old_receive_is_not_proven_by_the_contract() {
    // TOKEN §3a: only the receives among the last RECENT entries are checked against their senders; an older one is
    // the wallets' to check. B receives a send that does not exist, then RECENT burns bury it.
    let mut w = paid();
    let b = w.0.get_mut(&pk(2)).unwrap();
    received(b, 1, 9, 50);
    for n in 0..RECENT as u8 {
        signed(b, 2, 100 + n, Body::Burn { amount: 1 });
    }
    assert_eq!(w.check(2), Ok(()), "the contract alone does not see an old receive");
    // One fewer burn: still recent, refused.
    let b = w.0.get_mut(&pk(2)).unwrap();
    b.entries.pop();
    assert_eq!(w.check(2), Err("a receive whose send is not in its sender's chain"));
}

#[test]
fn the_definition_must_be_this_standards() {
    let w = paid();
    let mut d = def();
    d.code_hash = [8; 32];
    assert_eq!(check(&params(1), &d, &w.0[&pk(1)], &|o| w.0.get(o).cloned()), Err("the definition is not this standard's"));
}

#[test]
fn encodings_round_trip_exactly() {
    let w = paid();
    for s in w.0.values() {
        assert_eq!(State::decode(&s.encode()).as_ref(), Some(s));
        let mut longer = s.encode();
        longer.push(0);
        assert_eq!(State::decode(&longer), None, "trailing bytes refused");
    }
    assert_eq!(Definition::decode(&def().encode()), Some(def()));
    assert_eq!(ChainParams::decode(&params(2).encode()), Some(params(2)));
}

/// Every order of merging gives one result.
fn all_orders(copies: &[State]) -> Vec<State> {
    let n = copies.len();
    let mut out = Vec::new();
    let mut idx: Vec<usize> = (0..n).collect();
    fn permute(k: usize, idx: &mut Vec<usize>, copies: &[State], out: &mut Vec<State>) {
        if k == idx.len() {
            let mut m = copies[idx[0]].clone();
            for &i in &idx[1..] {
                m = merge(&m, &copies[i]);
            }
            out.push(m);
            return;
        }
        for i in k..idx.len() {
            idx.swap(k, i);
            permute(k + 1, idx, copies, out);
            idx.swap(k, i);
        }
    }
    permute(0, &mut idx, copies, &mut out);
    out
}

#[test]
fn different_writers_at_one_seq_settle_the_same_everywhere() {
    // A's chain: the writer sends at seq 2 on one copy; on another, someone appends a receive at seq 2.
    let mut w = paid();
    let base = w.0[&pk(1)].clone();
    let mut by_owner = base.clone();
    signed(&mut by_owner, 1, 3, Body::Send { to: pk(3), amount: 50 });
    let mut by_anyone = base.clone();
    // (A receive appended by anyone; its sender is not checked by the merge.)
    received(&mut by_anyone, 4, 7, 20);
    w.0.insert(pk(1), base);
    let results = all_orders(&[by_owner.clone(), by_anyone.clone()]);
    assert!(results.windows(2).all(|r| r[0] == r[1]), "one result in every order");
    let lowest = if by_owner.entries[2].hash() < by_anyone.entries[2].hash() { by_owner } else { by_anyone };
    assert_eq!(results[0], lowest, "the lower hash's branch");
    assert!(results[0].fork.is_none(), "different writers never freeze a chain");
}

#[test]
fn the_writer_signing_twice_at_one_seq_freezes_the_chain() {
    let w = paid();
    let base = w.0[&pk(1)].clone();
    let mut x = base.clone();
    signed(&mut x, 1, 3, Body::Send { to: pk(2), amount: 600 });
    let mut y = base.clone();
    signed(&mut y, 1, 4, Body::Send { to: pk(3), amount: 600 });
    // One copy goes on past the fork.
    let mut y_longer = y.clone();
    signed(&mut y_longer, 1, 5, Body::Burn { amount: 10 });
    let results = all_orders(&[x.clone(), y_longer.clone(), base.clone()]);
    assert!(results.windows(2).all(|r| r[0] == r[1]), "one result in every order");
    let m = &results[0];
    assert_eq!(m.entries.len(), 2, "everything before the fork stays");
    let (lo, hi) = m.fork.as_ref().expect("frozen");
    assert!(lo.hash() < hi.hash());
    let mut frozen = w;
    frozen.0.insert(pk(1), m.clone());
    assert_eq!(frozen.check(1), Ok(()), "a frozen chain is well-formed: it carries its proof");
    // A fork whose second entry is not the writer's is not a fork.
    let mut fake = m.clone();
    let (a, b) = fake.fork.take().unwrap();
    let forged = sign(Entry { auth: Auth::Work(0), ..b.clone() }, 2);
    let pair = if a.hash() < forged.hash() { (a, forged) } else { (forged, a) };
    fake.fork = Some(pair);
    frozen.0.insert(pk(1), fake);
    assert_eq!(frozen.check(1), Err("a fork not signed twice by the chain's writer"));
}

#[test]
fn a_third_writer_entry_keeps_the_fork_with_the_two_lowest() {
    let w = paid();
    let base = w.0[&pk(1)].clone();
    let copies: Vec<State> = (3..6)
        .map(|n| {
            let mut c = base.clone();
            signed(&mut c, 1, n, Body::Send { to: pk(n), amount: 100 });
            c
        })
        .collect();
    let results = all_orders(&copies);
    assert!(results.windows(2).all(|r| r[0] == r[1]), "one result in every order");
    let mut hashes: Vec<[u8; 32]> = copies.iter().map(|c| c.entries[2].hash()).collect();
    hashes.sort();
    let (lo, hi) = results[0].fork.as_ref().unwrap();
    assert_eq!((lo.hash(), hi.hash()), (hashes[0], hashes[1]));
}

#[test]
fn a_send_after_its_senders_fork_is_never_received() {
    // TOKEN §3b strict: every debit after the fork is void — a receive citing one is refused.
    let mut w = paid();
    let base = w.0[&pk(1)].clone();
    let mut x = base.clone();
    signed(&mut x, 1, 3, Body::Send { to: pk(2), amount: 600 });
    let mut y = base.clone();
    signed(&mut y, 1, 4, Body::Send { to: pk(3), amount: 600 });
    w.0.insert(pk(1), merge(&x, &y));
    received(w.0.get_mut(&pk(2)).unwrap(), 1, 3, 600);
    assert_eq!(w.check(2), Err("a receive whose send is not in its sender's chain"));
}

#[test]
fn a_copy_that_extends_another_wins_and_merging_is_idempotent() {
    let w = paid();
    let a = w.0[&pk(1)].clone();
    let mut longer = a.clone();
    signed(&mut longer, 1, 3, Body::Burn { amount: 10 });
    assert_eq!(merge(&a, &longer), longer);
    assert_eq!(merge(&longer, &a), longer);
    assert_eq!(merge(&longer, &longer), longer);
}
