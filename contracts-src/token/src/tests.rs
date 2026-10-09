use super::*;
use ed25519_dalek::{Signer, SigningKey};
use std::collections::HashMap;

const CODE: [u8; 32] = [9; 32];
const BEACON: [u8; 32] = [7; 32];
const IDLOG: [u8; 32] = [8; 32];

fn key(n: u8) -> SigningKey {
    SigningKey::from_bytes(&[n; 32])
}
/// Person n's DEVICE key (its node key: what signs its chains' entries).
fn pk(n: u8) -> [u8; 32] {
    key(n).verifying_key().to_bytes()
}
/// Person n's ACCOUNT: an owner key, the key it commits to next, its KEY LOG (an inception), its DID.
fn owner_seed(n: u8) -> [u8; 32] {
    [n.wrapping_add(100); 32]
}
fn next_seed(n: u8) -> [u8; 32] {
    [n.wrapping_add(150); 32]
}
fn log(n: u8) -> craftworks_idlog_contract::Log {
    let next = SigningKey::from_bytes(&next_seed(n)).verifying_key().to_bytes();
    let e = craftworks_idlog_contract::Event::signed(0, [0; 32], &owner_seed(n), craftworks_idlog_contract::commit(&next), [n; 32], [n; 32], vec![]).unwrap();
    craftworks_idlog_contract::Log { events: vec![e] }
}
/// Person n's log ROTATED once (its owner key now the pre-committed next one).
fn rotated(n: u8) -> craftworks_idlog_contract::Log {
    let mut l = log(n);
    let after = SigningKey::from_bytes(&[n.wrapping_add(200); 32]).verifying_key().to_bytes();
    let e = craftworks_idlog_contract::Event::signed(1, l.events[0].id(), &next_seed(n), craftworks_idlog_contract::commit(&after), [n; 32], [n; 32], vec![]).unwrap();
    l.events.push(e);
    l
}
fn did(n: u8) -> [u8; 32] {
    thread_local!(static DIDS: std::cell::RefCell<HashMap<u8, [u8; 32]>> = Default::default());
    DIDS.with(|m| *m.borrow_mut().entry(n).or_insert_with(|| log(n).events[0].id()))
}
/// Person n's device CREDENTIAL (the account's own format), signed by its owner key at seq 0.
fn cred(n: u8) -> Vec<u8> {
    craftworks_identity::credential(&did(n), &pk(n), &[], &owner_seed(n))
}
/// A DID's key log, as a node or a wallet reads it.
fn keys_of(d: &[u8; 32]) -> Option<craftworks_idlog_contract::Log> {
    (0..=255u8).find(|n| did(*n) == *d).map(log)
}
/// Before person `by` first signs a chain: its WRITER entry (device `by`, on key seq 0).
fn ensure_writer(s: &mut State, by: u8) {
    if s.fork.is_none() && writer_at(s, s.entries.len()).is_none() {
        let e = sign(entry(s, [0xee; 16], Body::Writer { cred: cred(by), key_seq: 0 }), by);
        s.entries.push(e);
    }
}
fn def() -> Definition {
    Definition { code_hash: CODE, decimals: 8, min_send: 10, genesis_owner: did(1), genesis_amount: 1000, name: b"T".to_vec(), nonce: [0; 16], beacon: BEACON, margin: 2, idlog: IDLOG, mint: None, freeze: None }
}
fn params(owner: u8) -> ChainParams {
    ChainParams { code_hash: CODE, definition: id(&CODE, &def().encode()), owner: did(owner), beacon: BEACON, idlog: IDLOG }
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
        Body::Receive { amount, .. } | Body::Mint { amount, .. } => b + amount,
        Body::Allow { .. } | Body::Offer { .. } | Body::Cut { .. } | Body::Writer { .. } => b,
        Body::Spend { amount, .. } | Body::Lock { amount, .. } | Body::Fill { amount, .. } => b.wrapping_sub(*amount),
        Body::Claim { amount, .. } => b + amount,
        Body::Refund { lock, .. } => {
            b + s.entries.iter().find_map(|x| match &x.body {
                Body::Lock { amount, .. } if &x.nonce == lock => Some(*amount),
                _ => None,
            }).unwrap_or(0)
        }
    }
}
fn delegate(mut e: Entry, by: u8) -> Entry {
    e.auth = Auth::Delegated { by: pk(by), sig: key(by).sign(&e.message()).to_bytes() };
    e
}
fn spent(s: &mut State, by: u8, n: u8, to: u8, amount: u128) {
    let e = delegate(entry(s, nonce(n), Body::Spend { to: did(to), amount }), by);
    s.entries.push(e);
}
fn entry(s: &State, n: [u8; 16], body: Body) -> Entry {
    let balance = after(s, &body);
    let stamp = s.entries.last().map_or(0, |e| e.stamp);
    Entry { seq: s.entries.last().map_or(0, |e| e.seq + 1), prev: s.head(), nonce: n, stamp, body, balance, auth: Auth::Work(0) }
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
    ensure_writer(s, by);
    let e = sign(entry(s, nonce(n), body), by);
    s.entries.push(e);
}
fn received(s: &mut State, from: u8, n: u8, amount: u128) {
    let e = work(entry(s, nonce(n), Body::Receive { from: did(from), nonce: nonce(n), amount, cited: 0, ev: Evidence::default() }));
    s.entries.push(e);
}

struct World(HashMap<[u8; 32], State>);
impl World {
    fn check(&self, owner: u8) -> Result<(), Why> {
        let s = &self.0[&did(owner)];
        let tok = params(1).definition;
        check(&params(owner), &def(), s, &|d, o| if *d == tok { self.0.get(o).cloned() } else { None }, &|_| None, Some(&log(owner)))
    }
}

/// A (1) holds the genesis and sends 300 to B (2); B receives it.
fn paid() -> World {
    let mut a = State::default();
    signed(&mut a, 1, 1, Body::Genesis { amount: 1000 });
    signed(&mut a, 1, 2, Body::Send { to: did(2), amount: 300 });
    let mut b = State::default();
    received(&mut b, 1, 2, 300);
    World([(did(1), a), (did(2), b)].into_iter().collect())
}

#[test]
fn a_payment_is_well_formed_on_both_sides() {
    let w = paid();
    assert_eq!(w.check(1), Ok(()));
    assert_eq!(w.check(2), Ok(()));
    assert_eq!(w.0[&did(1)].balance(), 700);
    assert_eq!(w.0[&did(2)].balance(), 300);
}

#[test]
fn a_debit_over_the_balance_is_refused() {
    let mut w = paid();
    let a = w.0.get_mut(&did(1)).unwrap();
    signed(a, 1, 3, Body::Send { to: did(3), amount: 701 });
    assert_eq!(w.check(1), Err("a debit over the balance"));
    let mut w = paid();
    let b = w.0.get_mut(&did(2)).unwrap();
    signed(b, 2, 4, Body::Burn { amount: 301 });
    assert_eq!(w.check(2), Err("a debit over the balance"));
}

#[test]
fn a_wrong_carried_balance_is_refused() {
    let mut w = paid();
    let a = w.0.get_mut(&did(1)).unwrap();
    let mut e = entry(a, nonce(3), Body::Send { to: did(3), amount: 100 });
    e.balance += 1;
    a.entries.push(sign(e, 1));
    assert_eq!(w.check(1), Err("a carried balance that does not add up"));
}

#[test]
fn a_send_under_the_minimum_or_to_itself_is_refused() {
    let mut w = paid();
    signed(w.0.get_mut(&did(1)).unwrap(), 1, 3, Body::Send { to: did(3), amount: 9 });
    assert_eq!(w.check(1), Err("a send under the minimum"));
    let mut w = paid();
    signed(w.0.get_mut(&did(1)).unwrap(), 1, 3, Body::Send { to: did(1), amount: 50 });
    assert_eq!(w.check(1), Err("a send to itself"));
}

#[test]
fn only_the_writer_signs_what_spends() {
    let mut w = paid();
    signed(w.0.get_mut(&did(1)).unwrap(), 2, 3, Body::Send { to: did(3), amount: 50 });
    assert_eq!(w.check(1), Err("an entry not signed by the chain's writer"));
}

#[test]
fn a_broken_link_or_seq_is_refused() {
    let mut w = paid();
    let a = w.0.get_mut(&did(1)).unwrap();
    let mut e = entry(a, nonce(3), Body::Burn { amount: 10 });
    e.prev = [7; 32];
    a.entries.push(sign(e, 1));
    assert_eq!(w.check(1), Err("a broken link"));
    let mut w = paid();
    let a = w.0.get_mut(&did(1)).unwrap();
    let mut e = entry(a, nonce(3), Body::Burn { amount: 10 });
    e.seq += 1;
    a.entries.push(sign(e, 1));
    assert_eq!(w.check(1), Err("a seq out of place"));
}

#[test]
fn only_the_definitions_genesis_comes_from_nothing() {
    let mut s = State::default();
    signed(&mut s, 3, 1, Body::Genesis { amount: 1000 });
    let w = World([(did(3), s)].into_iter().collect());
    assert_eq!(w.check(3), Err("a genesis that is not the definition's"));
    let mut s = State::default();
    signed(&mut s, 1, 1, Body::Genesis { amount: 1001 });
    let w = World([(did(1), s)].into_iter().collect());
    assert_eq!(w.check(1), Err("a genesis that is not the definition's"));
}

#[test]
fn a_receive_needs_its_work() {
    let mut w = paid();
    let b = w.0.get_mut(&did(2)).unwrap();
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
    received(w.0.get_mut(&did(2)).unwrap(), 1, 9, 50);
    assert_eq!(w.check(2), Err("a receive whose send is not in its sender's chain"));
    // The send is to someone else.
    let mut w = paid();
    signed(w.0.get_mut(&did(1)).unwrap(), 1, 3, Body::Send { to: did(3), amount: 50 });
    received(w.0.get_mut(&did(2)).unwrap(), 1, 3, 50);
    assert_eq!(w.check(2), Err("a receive whose send is not in its sender's chain"));
    // A different amount.
    let mut w = paid();
    signed(w.0.get_mut(&did(1)).unwrap(), 1, 3, Body::Send { to: did(2), amount: 50 });
    received(w.0.get_mut(&did(2)).unwrap(), 1, 3, 60);
    assert_eq!(w.check(2), Err("a receive whose send is not in its sender's chain"));
    // No sender chain at all.
    let mut w = paid();
    received(w.0.get_mut(&did(2)).unwrap(), 5, 1, 50);
    assert_eq!(w.check(2), Err("the sender's chain is not there"));
}

#[test]
fn a_send_is_received_once() {
    let mut w = paid();
    received(w.0.get_mut(&did(2)).unwrap(), 1, 2, 300);
    assert_eq!(w.check(2), Err("a send received twice"));
}

#[test]
fn an_old_receive_is_not_proven_by_the_contract() {
    // TOKEN §3a: only the receives among the last RECENT entries are checked against their senders; an older one is
    // the wallets' to check. B receives a send that does not exist, then RECENT burns bury it.
    let mut w = paid();
    let b = w.0.get_mut(&did(2)).unwrap();
    ensure_writer(b, 2); // (B's writer named before: only the burns bury the receive)
    received(b, 1, 9, 50);
    for n in 0..RECENT as u8 {
        signed(b, 2, 100 + n, Body::Burn { amount: 1 });
    }
    assert_eq!(w.check(2), Ok(()), "the contract alone does not see an old receive");
    // One fewer burn: still recent, refused.
    let b = w.0.get_mut(&did(2)).unwrap();
    b.entries.pop();
    assert_eq!(w.check(2), Err("a receive whose send is not in its sender's chain"));
}

#[test]
fn the_definition_must_be_this_standards() {
    let w = paid();
    let mut d = def();
    d.code_hash = [8; 32];
    assert_eq!(check(&params(1), &d, &w.0[&did(1)], &|_, o| w.0.get(o).cloned(), &|_| None, Some(&log(1))), Err("the definition is not this standard's"));
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
    let base = w.0[&did(1)].clone();
    let mut by_owner = base.clone();
    signed(&mut by_owner, 1, 3, Body::Send { to: did(3), amount: 50 });
    let mut by_anyone = base.clone();
    // (A receive appended by anyone; its sender is not checked by the merge.)
    received(&mut by_anyone, 4, 7, 20);
    w.0.insert(did(1), base.clone());
    let results = all_orders(&[by_owner.clone(), by_anyone.clone()]);
    assert!(results.windows(2).all(|r| r[0] == r[1]), "one result in every order");
    let at = base.entries.len();
    assert_ne!(by_owner.entries[at].hash(), by_anyone.entries[at].hash(), "two different entries at one seq");
    let lowest = if by_owner.entries[at].hash() < by_anyone.entries[at].hash() { by_owner } else { by_anyone };
    assert_eq!(results[0], lowest, "the lower hash's branch");
    assert!(results[0].fork.is_none(), "different writers never freeze a chain");
}

#[test]
fn the_writer_signing_twice_at_one_seq_freezes_the_chain() {
    let w = paid();
    let base = w.0[&did(1)].clone();
    let mut x = base.clone();
    signed(&mut x, 1, 3, Body::Send { to: did(2), amount: 600 });
    let mut y = base.clone();
    signed(&mut y, 1, 4, Body::Send { to: did(3), amount: 600 });
    // One copy goes on past the fork.
    let mut y_longer = y.clone();
    signed(&mut y_longer, 1, 5, Body::Burn { amount: 10 });
    let results = all_orders(&[x.clone(), y_longer.clone(), base.clone()]);
    assert!(results.windows(2).all(|r| r[0] == r[1]), "one result in every order");
    let m = &results[0];
    assert_eq!(m.entries.len(), base.entries.len(), "everything before the fork stays");
    let (lo, hi) = m.fork.as_ref().expect("frozen");
    assert!(lo.hash() < hi.hash());
    let mut frozen = w;
    frozen.0.insert(did(1), m.clone());
    assert_eq!(frozen.check(1), Ok(()), "a frozen chain is well-formed: it carries its proof");
    // A fork whose second entry is not the writer's is not a fork.
    let mut fake = m.clone();
    let (a, b) = fake.fork.take().unwrap();
    let forged = sign(Entry { auth: Auth::Work(0), ..b.clone() }, 2);
    let pair = if a.hash() < forged.hash() { (a, forged) } else { (forged, a) };
    fake.fork = Some(pair);
    frozen.0.insert(did(1), fake);
    assert_eq!(frozen.check(1), Err("a fork not signed twice by the chain's writer"));
}

#[test]
fn a_third_writer_entry_keeps_the_fork_with_the_two_lowest() {
    let w = paid();
    let base = w.0[&did(1)].clone();
    let copies: Vec<State> = (3..6)
        .map(|n| {
            let mut c = base.clone();
            signed(&mut c, 1, n, Body::Send { to: did(n), amount: 100 });
            c
        })
        .collect();
    let results = all_orders(&copies);
    assert!(results.windows(2).all(|r| r[0] == r[1]), "one result in every order");
    let mut hashes: Vec<[u8; 32]> = copies.iter().map(|c| c.entries.last().unwrap().hash()).collect();
    hashes.sort();
    let (lo, hi) = results[0].fork.as_ref().unwrap();
    assert_eq!((lo.hash(), hi.hash()), (hashes[0], hashes[1]));
}

#[test]
fn a_send_after_its_senders_fork_is_never_received() {
    // TOKEN §3b strict: every debit after the fork is void — a receive citing one is refused.
    let mut w = paid();
    let base = w.0[&did(1)].clone();
    let mut x = base.clone();
    signed(&mut x, 1, 3, Body::Send { to: did(2), amount: 600 });
    let mut y = base.clone();
    signed(&mut y, 1, 4, Body::Send { to: did(3), amount: 600 });
    w.0.insert(did(1), merge(&x, &y));
    received(w.0.get_mut(&did(2)).unwrap(), 1, 3, 600);
    assert_eq!(w.check(2), Err("a receive whose send is not in its sender's chain"));
}

#[test]
fn a_copy_that_extends_another_wins_and_merging_is_idempotent() {
    let w = paid();
    let a = w.0[&did(1)].clone();
    let mut longer = a.clone();
    signed(&mut longer, 1, 3, Body::Burn { amount: 10 });
    assert_eq!(merge(&a, &longer), longer);
    assert_eq!(merge(&longer, &a), longer);
    assert_eq!(merge(&longer, &longer), longer);
}

#[test]
fn a_receive_from_itself_is_refused_without_asking_for_itself() {
    let mut w = paid();
    received(w.0.get_mut(&did(2)).unwrap(), 2, 7, 20);
    assert_eq!(w.check(2), Err("a receive from itself"));
}

#[test]
fn a_definition_name_is_bounded() {
    let mut d = def();
    d.name = vec![b'x'; NAME_MAX];
    assert_eq!(Definition::decode(&d.encode()), Some(d.clone()));
    d.name = vec![b'x'; NAME_MAX + 1];
    assert_eq!(Definition::decode(&d.encode()), None);
}

/// A (1) allows S (5) to spend 200; S spends 150 to C (3); C receives it.
fn allowed() -> World {
    let mut w = paid();
    signed(w.0.get_mut(&did(1)).unwrap(), 1, 3, Body::Allow { spender: pk(5), limit: 200 });
    spent(w.0.get_mut(&did(1)).unwrap(), 5, 4, 3, 150);
    let mut c = State::default();
    received(&mut c, 1, 4, 150);
    w.0.insert(did(3), c);
    w
}

#[test]
fn a_spender_spends_within_its_allowance_and_it_is_received() {
    let w = allowed();
    assert_eq!(w.check(1), Ok(()));
    assert_eq!(w.check(3), Ok(()));
    assert_eq!(w.0[&did(1)].balance(), 550);
    assert_eq!(w.0[&did(3)].balance(), 150);
}

#[test]
fn a_spend_over_what_is_left_of_its_allowance_is_refused() {
    let mut w = allowed();
    spent(w.0.get_mut(&did(1)).unwrap(), 5, 5, 3, 51);
    assert_eq!(w.check(1), Err("a spend over its allowance"));
    let mut w = allowed();
    spent(w.0.get_mut(&did(1)).unwrap(), 5, 5, 3, 50);
    assert_eq!(w.check(1), Ok(()), "exactly what is left");
}

#[test]
fn only_an_allowed_key_spends_and_only_by_its_own_signature() {
    let mut w = allowed();
    spent(w.0.get_mut(&did(1)).unwrap(), 6, 5, 3, 20);
    assert_eq!(w.check(1), Err("a spend with no allowance"));
    // A spend signed as the owner is not a spend.
    let mut w = allowed();
    signed(w.0.get_mut(&did(1)).unwrap(), 1, 5, Body::Spend { to: did(3), amount: 20 });
    assert_eq!(w.check(1), Err("a spend not signed by a spender"));
    // Naming the spender but signed by someone else.
    let mut w = allowed();
    let a = w.0.get_mut(&did(1)).unwrap();
    let mut e = entry(a, nonce(5), Body::Spend { to: did(3), amount: 20 });
    e.auth = Auth::Delegated { by: pk(5), sig: key(6).sign(&e.message()).to_bytes() };
    a.entries.push(e);
    assert_eq!(w.check(1), Err("a spend not signed by a spender"));
}

#[test]
fn a_new_allowance_replaces_the_old_and_zero_revokes() {
    let mut w = allowed();
    signed(w.0.get_mut(&did(1)).unwrap(), 1, 5, Body::Allow { spender: pk(5), limit: 10 });
    spent(w.0.get_mut(&did(1)).unwrap(), 5, 6, 3, 11);
    assert_eq!(w.check(1), Err("a spend over its allowance"), "replaced, not added");
    let mut w = allowed();
    signed(w.0.get_mut(&did(1)).unwrap(), 1, 5, Body::Allow { spender: pk(5), limit: 0 });
    spent(w.0.get_mut(&did(1)).unwrap(), 5, 6, 3, 10);
    assert_eq!(w.check(1), Err("a spend with no allowance"), "revoked");
    let mut w = allowed();
    signed(w.0.get_mut(&did(1)).unwrap(), 1, 5, Body::Allow { spender: pk(1), limit: 10 });
    assert_eq!(w.check(1), Err("an allowance to itself"));
}

#[test]
fn an_owner_entry_and_a_spender_entry_at_one_seq_do_not_freeze_the_chain() {
    let w = allowed();
    let base = w.0[&did(1)].clone();
    let mut by_owner = base.clone();
    signed(&mut by_owner, 1, 7, Body::Send { to: did(2), amount: 20 });
    let mut by_spender = base.clone();
    spent(&mut by_spender, 5, 8, 3, 20);
    let r = all_orders(&[by_owner.clone(), by_spender.clone()]);
    assert!(r.windows(2).all(|x| x[0] == x[1]));
    assert!(r[0].fork.is_none(), "two writers, not an equivocation");
    let n = base.entries.len();
    let lowest = if by_owner.entries[n].hash() < by_spender.entries[n].hash() { by_owner } else { by_spender };
    assert_eq!(r[0], lowest);
}

// ---- step 5b: swaps (two tokens), refunds, the append rules ----

fn def_b() -> Definition {
    Definition { genesis_owner: did(2), name: b"B".to_vec(), nonce: [1; 16], ..def() }
}
fn id_a() -> [u8; 32] {
    id(&CODE, &def().encode())
}
fn id_b() -> [u8; 32] {
    id(&CODE, &def_b().encode())
}
fn at(token: [u8; 32], owner: u8) -> ChainParams {
    ChainParams { code_hash: CODE, definition: token, owner: did(owner), beacon: BEACON, idlog: IDLOG }
}

/// An entry at `stamp`, by the writer `by` (Some) or with work (None).
fn put_at(s: &mut State, n: u8, body: Body, by: Option<u8>, stamp: u64) {
    if let Some(k) = by {
        ensure_writer(s, k);
    }
    let mut e = entry(s, nonce(n), body);
    e.stamp = stamp;
    let e = match by {
        Some(k) => sign(e, k),
        None => work(e),
    };
    s.entries.push(e);
}

struct Two(HashMap<([u8; 32], [u8; 32]), State>);
impl Two {
    fn get(&self, token: [u8; 32], owner: u8) -> &State {
        &self.0[&(token, did(owner))]
    }
    fn chain(&mut self, token: [u8; 32], owner: u8) -> &mut State {
        self.0.entry((token, did(owner))).or_default()
    }
    fn check(&self, token: [u8; 32], owner: u8) -> Result<(), Why> {
        let d = if token == id_a() { def() } else { def_b() };
        check(&at(token, owner), &d, self.get(token, owner), &|t, o| self.0.get(&(*t, *o)).cloned(), &|_| None, Some(&log(owner)))
    }
    /// As a holder takes in the last `new` entries: their citations must be SHOWN (live, or by their evidence).
    fn strict(&self, token: [u8; 32], owner: u8, new: usize) -> Result<(), Why> {
        let d = if token == id_a() { def() } else { def_b() };
        let s = self.get(token, owner);
        walk(&at(token, owner), &d, s, &|t, o| self.0.get(&(*t, *o)).cloned(), &|_| None, Some(&log(owner)), new, s.entries.len() - new).map(|_| ()).map_err(|x| x.1)
    }
}

const M: u8 = 1; // the maker: holds A, offers 2 A for every 1 B
const T: u8 = 2; // the taker: holds B

/// M offers; T locks 100 B for it; the fill gives T 200 A; T receives it; M claims the 100 B.
fn swapped() -> Two {
    let mut w = Two(HashMap::new());
    put_at(w.chain(id_a(), M), 1, Body::Genesis { amount: 1000 }, Some(M), 0);
    put_at(w.chain(id_a(), M), 10, Body::Offer { get: id_b(), give: 2, per: 1, max: 400, until: 1000 }, Some(M), 0);
    put_at(w.chain(id_b(), T), 1, Body::Genesis { amount: 1000 }, Some(T), 0);
    put_at(w.chain(id_b(), T), 20, Body::Lock { to: did(M), amount: 100, until: 50, token: id_a(), offer: nonce(10) }, Some(T), 0);
    put_at(w.chain(id_a(), M), 30, Body::Fill { offer: nonce(10), taker: did(T), lock: nonce(20), amount: 200, cited: 0, until: 50, ev: Evidence::default() }, None, 0);
    put_at(w.chain(id_a(), T), 30, Body::Receive { from: did(M), nonce: nonce(30), amount: 200, cited: 0, ev: Evidence::default() }, None, 0);
    put_at(w.chain(id_b(), M), 40, Body::Claim { taker: did(T), lock: nonce(20), amount: 100, cited: 0, token: id_a(), ev: Evidence::default() }, None, 0);
    w
}

#[test]
fn a_swap_settles_on_all_four_chains() {
    let w = swapped();
    for (tok, o) in [(id_a(), M), (id_a(), T), (id_b(), T), (id_b(), M)] {
        assert_eq!(w.check(tok, o), Ok(()), "{o}");
    }
    assert_eq!((w.get(id_a(), M).balance(), w.get(id_a(), T).balance()), (800, 200));
    assert_eq!((w.get(id_b(), T).balance(), w.get(id_b(), M).balance()), (900, 100));
}

/// The swap with T's lock replaced by `lock`, then the fill and the rest as before.
fn swapped_with_lock(lock: Body) -> Two {
    let mut w = swapped();
    let tb = w.chain(id_b(), T);
    tb.entries.truncate(1);
    put_at(tb, 20, lock, Some(T), 0);
    w
}

#[test]
fn a_fill_must_be_paid_for_by_its_lock() {
    // 99 B pay for 198 A, not 200.
    let w = swapped_with_lock(Body::Lock { to: did(M), amount: 99, until: 50, token: id_a(), offer: nonce(10) });
    assert_eq!(w.check(id_a(), M), Err("a fill its lock does not pay for"));
    // A lock for another maker, another token, another offer, another deadline.
    for lock in [
        Body::Lock { to: did(9), amount: 100, until: 50, token: id_a(), offer: nonce(10) },
        Body::Lock { to: did(M), amount: 100, until: 50, token: [5; 32], offer: nonce(10) },
        Body::Lock { to: did(M), amount: 100, until: 50, token: id_a(), offer: nonce(11) },
        Body::Lock { to: did(M), amount: 100, until: 51, token: id_a(), offer: nonce(10) },
    ] {
        let w = swapped_with_lock(lock);
        assert_eq!(w.check(id_a(), M), Err("a fill its lock does not pay for"));
    }
}

#[test]
fn a_fill_is_bounded_by_its_offer_and_fills_a_lock_once() {
    let mut w = swapped();
    // Another lock of T's for 150 B → 300 A; only 200 are left of the offer.
    put_at(w.chain(id_b(), T), 21, Body::Lock { to: did(M), amount: 150, until: 50, token: id_a(), offer: nonce(10) }, Some(T), 0);
    let mut over = w.chain(id_a(), M).clone();
    put_at(&mut over, 31, Body::Fill { offer: nonce(10), taker: did(T), lock: nonce(21), amount: 300, cited: 0, until: 50, ev: Evidence::default() }, None, 0);
    *w.chain(id_a(), M) = over;
    assert_eq!(w.check(id_a(), M), Err("a fill past what is left of its offer"));
    let mut w = swapped();
    put_at(w.chain(id_a(), M), 31, Body::Fill { offer: nonce(10), taker: did(T), lock: nonce(20), amount: 10, cited: 0, until: 50, ev: Evidence::default() }, None, 0);
    assert_eq!(w.check(id_a(), M), Err("a lock filled twice"));
}

#[test]
fn a_claim_needs_the_fill_and_happens_once() {
    let mut w = swapped();
    // Remove the fill: the claim has nothing to stand on.
    let ma = w.chain(id_a(), M);
    ma.entries.truncate(2);
    assert_eq!(w.check(id_b(), M), Err("a claim of a lock that is not filled"));
    let mut w = swapped();
    put_at(w.chain(id_b(), M), 41, Body::Claim { taker: did(T), lock: nonce(20), amount: 100, cited: 0, token: id_a(), ev: Evidence::default() }, None, 0);
    assert_eq!(w.check(id_b(), M), Err("a lock claimed twice"));
    // A claim of more than the lock held.
    let mut w = swapped();
    let mb = w.chain(id_b(), M);
    mb.entries.clear();
    put_at(mb, 40, Body::Claim { taker: did(T), lock: nonce(20), amount: 101, cited: 0, token: id_a(), ev: Evidence::default() }, None, 0);
    assert_eq!(w.check(id_b(), M), Err("a claim of a lock that is not filled"));
}

#[test]
fn an_unfilled_lock_is_refunded_once_and_a_filled_one_never() {
    // A second lock nobody fills: refunded.
    let mut w = swapped();
    put_at(w.chain(id_b(), T), 21, Body::Lock { to: did(M), amount: 50, until: 50, token: id_a(), offer: nonce(10) }, Some(T), 0);
    put_at(w.chain(id_b(), T), 22, Body::Refund { lock: nonce(21), ev: Evidence::default() }, None, 0);
    assert_eq!(w.check(id_b(), T), Ok(()));
    assert_eq!(w.get(id_b(), T).balance(), 900, "the 50 back");
    put_at(w.chain(id_b(), T), 23, Body::Refund { lock: nonce(21), ev: Evidence::default() }, None, 0);
    assert_eq!(w.check(id_b(), T), Err("a lock refunded twice"));
    // The filled one.
    let mut w = swapped();
    put_at(w.chain(id_b(), T), 22, Body::Refund { lock: nonce(20), ev: Evidence::default() }, None, 0);
    assert_eq!(w.check(id_b(), T), Err("a refund of a filled lock"));
}

#[test]
fn a_receive_of_a_fill_names_its_stamp() {
    let mut w = swapped();
    let ta = w.chain(id_a(), T);
    ta.entries.clear();
    put_at(ta, 30, Body::Receive { from: did(M), nonce: nonce(30), amount: 200, cited: 1, ev: Evidence::default() }, None, 0);
    assert_eq!(w.check(id_a(), T), Err("a receive whose send is not in its sender's chain"));
}

#[test]
fn stamps_never_go_down() {
    let mut w = swapped();
    let tb = w.chain(id_b(), T);
    let last = tb.entries.len() - 1;
    tb.entries.truncate(last);
    put_at(tb, 20, Body::Lock { to: did(M), amount: 100, until: 50, token: id_a(), offer: nonce(10) }, Some(T), 0);
    let mut s = w.get(id_b(), T).clone();
    s.entries.truncate(1);
    s.entries[0].stamp = 5;
    let g = sign(Entry { auth: Auth::Work(0), ..s.entries[0].clone() }, T);
    s.entries[0] = g;
    put_at(&mut s, 21, Body::Burn { amount: 10 }, Some(T), 4);
    assert_eq!(check(&at(id_b(), T), &def_b(), &s, &|_, _| None, &|_| None, Some(&log(T))), Err("a stamp below its predecessor's"));
}

// The append rules: `held` → `merged`, now = the beacon's height. Margin 2.
fn base(stamp: u64) -> State {
    let mut s = State::default();
    put_at(&mut s, 1, Body::Genesis { amount: 1000 }, Some(M), stamp);
    s
}

#[test]
fn a_new_entry_is_stamped_near_the_beacon() {
    let held = base(0);
    let mut merged = held.clone();
    put_at(&mut merged, 2, Body::Burn { amount: 10 }, Some(M), 7);
    assert_eq!(append_check(&def(), 10, &held, &merged, Some(&log(M))), Err("a stamp far from the beacon"), "3 behind, margin 2");
    assert_eq!(append_check(&def(), 9, &held, &merged, Some(&log(M))), Ok(()), "2 behind");
    assert_eq!(append_check(&def(), 6, &held, &merged, Some(&log(M))), Err("a stamp far from the beacon"), "ahead of the beacon");
    // What the node already held is never checked again, however old.
    assert_eq!(append_check(&def(), 1000, &merged, &merged, Some(&log(M))), Ok(()));
}

#[test]
fn a_citation_waits_a_margin_after_what_it_cites() {
    let held = base(0);
    let mut merged = held.clone();
    put_at(&mut merged, 2, Body::Receive { from: did(9), nonce: nonce(2), amount: 50, cited: 10, ev: Evidence::default() }, None, 11);
    assert_eq!(append_check(&def(), 11, &held, &merged, Some(&log(M))), Err("an entry citing one not yet final"));
    let mut merged = held.clone();
    put_at(&mut merged, 2, Body::Receive { from: did(9), nonce: nonce(2), amount: 50, cited: 10, ev: Evidence::default() }, None, 12);
    assert_eq!(append_check(&def(), 12, &held, &merged, Some(&log(M))), Ok(()));
}

#[test]
fn a_fill_lands_a_margin_before_its_deadline_and_a_refund_after_it() {
    let held = base(0);
    let fill = |stamp| {
        let mut m = held.clone();
        put_at(&mut m, 2, Body::Fill { offer: nonce(1), taker: did(T), lock: nonce(9), amount: 10, cited: 5, until: 20, ev: Evidence::default() }, None, stamp);
        m
    };
    assert_eq!(append_check(&def(), 18, &held, &fill(18), Some(&log(M))), Err("a fill too close to its lock's deadline"));
    assert_eq!(append_check(&def(), 17, &held, &fill(17), Some(&log(M))), Ok(()));
    assert_eq!(append_check(&def(), 6, &held, &fill(6), Some(&log(M))), Err("an entry citing one not yet final"), "the lock not final yet");
    let mut locked = base(0);
    put_at(&mut locked, 3, Body::Lock { to: did(9), amount: 50, until: 20, token: [5; 32], offer: nonce(1) }, Some(M), 0);
    let refund = |stamp| {
        let mut m = locked.clone();
        put_at(&mut m, 4, Body::Refund { lock: nonce(3), ev: Evidence::default() }, None, stamp);
        m
    };
    assert_eq!(append_check(&def(), 19, &locked, &refund(19), Some(&log(M))), Err("a refund before its lock's deadline"));
    assert_eq!(append_check(&def(), 20, &locked, &refund(20), Some(&log(M))), Ok(()));
}

// ---- step 6: wallet verification ----
use crate::wallet::{Checkpoint, Fail, FraudProof, Source, Vouch, Wallet};

/// A verification's verdict on what is HELD: value, or its fraud proof (a test hands over every block: never `Need`).
fn fraud(r: Result<(), Fail>) -> Result<(), FraudProof> {
    r.map_err(|f| match f {
        Fail::Fraud(p) => p,
        Fail::Need(n) => panic!("blocks needed: {}", n.len()),
    })
}

fn defs(t: &[u8; 32]) -> Option<Definition> {
    if *t == id_a() {
        Some(def())
    } else if *t == id_b() {
        Some(def_b())
    } else {
        None
    }
}
fn verify(w: &Two, wallet: &mut Wallet, owner: u8) -> Result<(), FraudProof> {
    let chain = |p: &ChainParams| w.0.get(&(p.definition, p.owner)).cloned();
    let src = Source { chain: &chain, definition: &defs, block: &|_| None, keys: &keys_of, supply: &|_| None, authority: &|_| None };
    let last = w.get(id_a(), owner).entries.len() as u64 - 1;
    fraud(wallet.verify(&src, &at(id_a(), owner), last))
}
fn checkpoint(w: &Two, owner: u8, by: u8) -> Checkpoint {
    let s = w.get(id_a(), owner);
    let e = s.entries.last().unwrap();
    let chain = at(id_a(), owner);
    let m = Checkpoint::message(&chain, e.seq, &e.hash(), e.balance);
    Checkpoint { chain, seq: e.seq, head: e.hash(), balance: e.balance, by: pk(by), sig: key(by).sign(&m).to_bytes() }
}
fn vouch(c: &Checkpoint, by: u8) -> Vouch {
    Vouch { checkpoint: c.hash(), by: pk(by), sig: key(by).sign(&Vouch::message(&c.hash())).to_bytes() }
}

/// Token A: 1 (genesis) → 300 to 2 → 100 to 3.
fn three_hops() -> Two {
    let mut w = Two(HashMap::new());
    put_at(w.chain(id_a(), 1), 1, Body::Genesis { amount: 1000 }, Some(1), 0);
    put_at(w.chain(id_a(), 1), 2, Body::Send { to: did(2), amount: 300 }, Some(1), 0);
    put_at(w.chain(id_a(), 2), 2, Body::Receive { from: did(1), nonce: nonce(2), amount: 300, cited: 0, ev: Evidence::default() }, None, 0);
    put_at(w.chain(id_a(), 2), 3, Body::Send { to: did(3), amount: 100 }, Some(2), 0);
    put_at(w.chain(id_a(), 3), 3, Body::Receive { from: did(2), nonce: nonce(3), amount: 100, cited: 0, ev: Evidence::default() }, None, 0);
    w
}

/// 2 credits itself 5000 from a send that does not exist, buried under RECENT real receives (1 sends them), then pays
/// 3 out of it.
fn buried() -> Two {
    let mut w = Two(HashMap::new());
    put_at(w.chain(id_a(), 1), 1, Body::Genesis { amount: 1000 }, Some(1), 0);
    put_at(w.chain(id_a(), 2), 9, Body::Receive { from: did(1), nonce: nonce(9), amount: 5000, cited: 0, ev: Evidence::default() }, None, 0);
    for k in 0..RECENT as u8 {
        put_at(w.chain(id_a(), 1), 20 + k, Body::Send { to: did(2), amount: 10 }, Some(1), 0);
        put_at(w.chain(id_a(), 2), 20 + k, Body::Receive { from: did(1), nonce: nonce(20 + k), amount: 10, cited: 0, ev: Evidence::default() }, None, 0);
    }
    put_at(w.chain(id_a(), 2), 40, Body::Send { to: did(3), amount: 3000 }, Some(2), 0);
    put_at(w.chain(id_a(), 3), 40, Body::Receive { from: did(2), nonce: nonce(40), amount: 3000, cited: 0, ev: Evidence::default() }, None, 0);
    w
}

#[test]
fn a_wallet_verifies_an_honest_payment_back_to_genesis() {
    let w = three_hops();
    let mut wallet = Wallet::new(pk(3));
    assert_eq!(verify(&w, &mut wallet, 3), Ok(()));
    assert!(wallet.reads.get() >= 3, "it read the chains back to genesis");
}

#[test]
fn minting_by_burying_passes_the_contract_and_is_refused_by_the_wallet() {
    let w = buried();
    // The contract alone accepts the chain: the fake receive is past its window…
    assert_eq!(w.check(id_a(), 2), Ok(()));
    // …the wallet refuses it, naming the entry.
    let f = verify(&w, &mut Wallet::new(pk(5)), 2).unwrap_err();
    assert_eq!((f.chain, f.seq, f.why), (at(id_a(), 2), 0, "a receive whose send is not in its sender's chain"));
    // CONTROL: the same chain without the fake receive verifies.
    let mut honest = buried();
    let c2 = honest.chain(id_a(), 2);
    let rest: Vec<Entry> = c2.entries.drain(1..).collect();
    c2.entries.clear();
    let mut rebuilt = State::default();
    for e in rest {
        let body = e.body.clone();
        put_at(&mut rebuilt, e.nonce[0], body, if matches!(e.auth, Auth::Sig(_)) { Some(2) } else { None }, 0);
    }
    let last = rebuilt.entries.len() - 1;
    rebuilt.entries.truncate(last); // (the 3000 send is more than the honest balance: drop it)
    *honest.chain(id_a(), 2) = rebuilt;
    assert_eq!(verify(&honest, &mut Wallet::new(pk(5)), 2), Ok(()));
}

#[test]
fn a_payment_out_of_fake_money_is_refused_downstream_with_the_upstream_proof() {
    let w = buried();
    assert_eq!(w.check(id_a(), 3), Ok(()), "3's own chain is well-formed");
    let f = verify(&w, &mut Wallet::new(pk(3)), 3).unwrap_err();
    assert_eq!((f.chain, f.seq), (at(id_a(), 2), 0), "the root cause: 2's fake receive");
}

#[test]
fn a_citation_loop_is_money_from_nothing() {
    // 4 receives 50 from 5's send; 5 receives 50 from 4's send — each send made out of the other's receive.
    let mut w = Two(HashMap::new());
    put_at(w.chain(id_a(), 4), 51, Body::Receive { from: did(5), nonce: nonce(52), amount: 50, cited: 0, ev: Evidence::default() }, None, 0);
    put_at(w.chain(id_a(), 4), 53, Body::Send { to: did(5), amount: 50 }, Some(4), 0);
    put_at(w.chain(id_a(), 5), 54, Body::Receive { from: did(4), nonce: nonce(53), amount: 50, cited: 0, ev: Evidence::default() }, None, 0);
    put_at(w.chain(id_a(), 5), 52, Body::Send { to: did(4), amount: 50 }, Some(5), 0);
    assert_eq!(w.check(id_a(), 4), Ok(()));
    assert_eq!(w.check(id_a(), 5), Ok(()));
    let f = verify(&w, &mut Wallet::new(pk(9)), 4).unwrap_err();
    assert_eq!(f.why, "a citation loop: money from nothing");
}

#[test]
fn money_back_and_forth_is_not_a_loop() {
    // 1 → 300 to 2; 2 → 50 back to 1; 1 receives it; 1 → 100 to 3. Verifying 3 walks 1 (to its last send), which
    // walks 2, which walks 1 again — to an EARLIER entry: no loop.
    let mut w = three_hops();
    put_at(w.chain(id_a(), 2), 4, Body::Send { to: did(1), amount: 50 }, Some(2), 0);
    put_at(w.chain(id_a(), 1), 4, Body::Receive { from: did(2), nonce: nonce(4), amount: 50, cited: 0, ev: Evidence::default() }, None, 0);
    put_at(w.chain(id_a(), 1), 5, Body::Send { to: did(6), amount: 100 }, Some(1), 0);
    put_at(w.chain(id_a(), 6), 5, Body::Receive { from: did(1), nonce: nonce(5), amount: 100, cited: 0, ev: Evidence::default() }, None, 0);
    assert_eq!(verify(&w, &mut Wallet::new(pk(6)), 6), Ok(()));
}

#[test]
fn a_trusted_checkpoint_saves_the_walk() {
    let w = three_hops();
    let mut plain = Wallet::new(pk(3));
    verify(&w, &mut plain, 3).unwrap();
    let mut with = Wallet::new(pk(3));
    with.trusted = vec![pk(9)];
    with.checkpoints = vec![checkpoint(&w, 2, 9)];
    verify(&w, &mut with, 3).unwrap();
    assert!(with.reads.get() < plain.reads.get(), "{} reads with the checkpoint, {} without", with.reads.get(), plain.reads.get());
    // The owner's own checkpoint counts for nothing, trusted or not.
    let mut own = Wallet::new(pk(3));
    own.trusted = vec![pk(2)];
    own.checkpoints = vec![checkpoint(&w, 2, 2)];
    verify(&w, &mut own, 3).unwrap();
    assert_eq!(own.reads.get(), plain.reads.get(), "walked as if there were none");
}

#[test]
fn vouches_count_and_an_owners_vouch_does_not() {
    let w = three_hops();
    let c = checkpoint(&w, 2, 7); // 7: unknown to the wallet
    let reads = |wallet: &mut Wallet| {
        verify(&w, wallet, 3).unwrap();
        wallet.reads.get()
    };
    let mut none = Wallet::new(pk(3));
    none.checkpoints = vec![c.clone()];
    let walked = reads(&mut none);
    let mut by_trusted = Wallet::new(pk(3));
    by_trusted.trusted = vec![pk(8)];
    by_trusted.checkpoints = vec![c.clone()];
    by_trusted.vouches = vec![vouch(&c, 8)];
    assert!(reads(&mut by_trusted) < walked, "a trusted key's vouch");
    let mut by_count = Wallet::new(pk(3));
    by_count.min_vouches = 2;
    by_count.checkpoints = vec![c.clone()];
    by_count.vouches = vec![vouch(&c, 5), vouch(&c, 6)];
    assert!(reads(&mut by_count) < walked, "two vouches");
    let mut short = Wallet::new(pk(3));
    short.min_vouches = 2;
    short.checkpoints = vec![c.clone()];
    short.vouches = vec![vouch(&c, 5), vouch(&c, 2)]; // 2 owns the chain: does not count
    assert_eq!(reads(&mut short), walked, "the owner's vouch is not one");
}

#[test]
fn a_false_checkpoint_is_proven_and_its_maker_and_vouchers_are_skipped() {
    let w = buried();
    let c = checkpoint(&w, 2, 9);
    let v = vouch(&c, 8);
    // Trusting 9, a wallet accepts the payment — that is what trust is.
    let mut trusting = Wallet::new(pk(3));
    trusting.trusted = vec![pk(9)];
    trusting.checkpoints = vec![c.clone()];
    trusting.vouches = vec![v.clone()];
    assert_eq!(verify(&w, &mut trusting, 3), Ok(()));
    // Anyone AUDITS it: the proof names the maker and the voucher.
    let chain = |p: &ChainParams| w.0.get(&(p.definition, p.owner)).cloned();
    let src = Source { chain: &chain, definition: &defs, block: &|_| None, keys: &keys_of, supply: &|_| None, authority: &|_| None };
    let proof = fraud(trusting.audit(&src, &c)).unwrap_err();
    assert_eq!(proof.checkpoint.as_ref().map(|c| c.by), Some(pk(9)));
    assert_eq!(proof.vouchers, vec![pk(8)]);
    // Knowing the proof, a wallet skips both: the payment is refused.
    let mut wary = Wallet::new(pk(3));
    wary.trusted = vec![pk(9), pk(8)];
    wary.checkpoints = vec![c.clone()];
    wary.vouches = vec![v];
    wary.frauds = vec![proof];
    assert!(wary.distrusted(&pk(9)) && wary.distrusted(&pk(8)));
    assert!(verify(&w, &mut wary, 3).is_err());
    // An honest checkpoint survives its audit.
    let h = three_hops();
    let chain = |p: &ChainParams| h.0.get(&(p.definition, p.owner)).cloned();
    let src = Source { chain: &chain, definition: &defs, block: &|_| None, keys: &keys_of, supply: &|_| None, authority: &|_| None };
    assert_eq!(Wallet::new(pk(1)).audit(&src, &checkpoint(&h, 2, 9)), Ok(()));
}

#[test]
fn a_checkpoint_that_is_not_the_chain_is_not_used() {
    // A trusted key's signed checkpoint of the fake-funded chain — but naming a head the chain does not have (another
    // copy, an older story): the wallet does not stop there, it walks, and finds the fake receive.
    let w = buried();
    let s = w.get(id_a(), 2);
    let e = s.entries.last().unwrap();
    let chain = at(id_a(), 2);
    let wrong = [7u8; 32];
    let m = Checkpoint::message(&chain, e.seq, &wrong, e.balance);
    let c = Checkpoint { chain, seq: e.seq, head: wrong, balance: e.balance, by: did(9), sig: key(9).sign(&m).to_bytes() };
    let mut wallet = Wallet::new(pk(3));
    wallet.trusted = vec![pk(9)];
    wallet.checkpoints = vec![c];
    assert!(verify(&w, &mut wallet, 3).is_err());
}

#[test]
fn a_long_history_is_walked_without_deepening_the_stack() {
    // 400 accounts, each paying the next: verifying the last walks 400 chains — iteratively. (Run on a small thread
    // stack, as a browser wallet's: a recursive walk overflows it.)
    let r = std::thread::Builder::new().stack_size(256 * 1024).spawn(|| {
        let n: u8 = 200;
        // Owners 1..=n: 1 holds the genesis; k receives from k-1 and pays k+1.
        let mut w = Two(HashMap::new());
        put_at(w.chain(id_a(), 1), 1, Body::Genesis { amount: 1000 }, Some(1), 0);
        put_at(w.chain(id_a(), 1), 2, Body::Send { to: did(2), amount: 900 }, Some(1), 0);
        for k in 2..n {
            put_at(w.chain(id_a(), k), k, Body::Receive { from: did(k - 1), nonce: nonce(k), amount: 900 - (k as u128 - 2), cited: 0, ev: Evidence::default() }, None, 0);
            put_at(w.chain(id_a(), k), k + 1, Body::Send { to: did(k + 1), amount: 900 - (k as u128 - 1) }, Some(k), 0);
        }
        let last = n;
        put_at(w.chain(id_a(), last), last, Body::Receive { from: did(last - 1), nonce: nonce(last), amount: 900 - (last as u128 - 2), cited: 0, ev: Evidence::default() }, None, 0);
        let mut wallet = Wallet::new(pk(last));
        (verify(&w, &mut wallet, last), wallet.reads.get())
    }).unwrap().join().unwrap();
    assert_eq!(r.0, Ok(()));
    assert!(r.1 >= 199, "{} reads", r.1);
}

#[test]
fn the_shared_records_round_trip_exactly() {
    use crate::wallet::bytes;
    let w = buried();
    let c = checkpoint(&w, 2, 9);
    let v = vouch(&c, 8);
    assert_eq!(bytes::read_checkpoint_exact(&bytes::checkpoint(&c)), Some(c.clone()));
    assert_eq!(bytes::read_vouch(&bytes::vouch(&v)), Some(v.clone()));
    let chain = |p: &ChainParams| w.0.get(&(p.definition, p.owner)).cloned();
    let src = Source { chain: &chain, definition: &defs, block: &|_| None, keys: &keys_of, supply: &|_| None, authority: &|_| None };
    let mut wallet = Wallet::new(pk(1));
    wallet.vouches = vec![v];
    let proof = fraud(wallet.audit(&src, &c)).unwrap_err();
    let back = bytes::read_fraud(&bytes::fraud(&proof)).unwrap();
    assert_eq!((back.chain, back.seq, back.checkpoint.clone(), back.vouchers.clone()), (proof.chain, proof.seq, proof.checkpoint.clone(), proof.vouchers.clone()));
    // A claimed proof is CHECKED, not trusted: re-auditing its checkpoint gives the same verdict.
    assert!(wallet.audit(&src, back.checkpoint.as_ref().unwrap()).is_err());
    for b in [bytes::checkpoint(&c), bytes::vouch(&vouch(&c, 8)), bytes::fraud(&proof)] {
        let mut longer = b.clone();
        longer.push(0);
        assert!(bytes::read_checkpoint_exact(&longer).is_none() && bytes::read_vouch(&longer).is_none() && bytes::read_fraud(&longer).is_none());
    }
    assert!(crate::wallet::bag_name("checkpoints", &id_a()).starts_with("token checkpoints "));
}

// ---- foundation 1: the prune cut ----
use crate::cut;

/// `owner` cuts its chain of `token` at `stamp`: from its whole record (returned, the cut on it), the cut's tree blocks
/// added to `blocks`; the live chain is the cut on.
fn cut_at(w: &mut Two, whole: &State, token: [u8; 32], owner: u8, stamp: u64, blocks: &mut cut::Blocks) -> State {
    let d = if token == id_a() { def() } else { def_b() };
    // Who signs the cut is the chain's writer: named first, if the chain has none yet (one only received).
    let mut whole = whole.clone();
    ensure_writer(&mut whole, owner);
    let (body, b) = cut::make(&at(token, owner), &d, &whole, Some(&log(owner))).unwrap();
    blocks.extend(b);
    let mut s = whole.clone();
    put_at(&mut s, 99, body, Some(owner), stamp);
    *w.chain(token, owner) = prune(&s);
    s
}
fn roots(s: &State) -> ([u8; 32], [u8; 32]) {
    match &s.entries[0].body {
        Body::Cut { history, received, .. } => (*history, *received),
        _ => panic!("not a cut"),
    }
}
/// The evidence that `whole`'s entry `seq` (of chain `p`) is behind its live copy's cut.
fn behind(blocks: &cut::Blocks, live: &State, p: &ChainParams, whole: &State, seq: u64) -> ([u8; 32], u64, Vec<u8>, Vec<u8>) {
    let (h, _) = roots(live);
    (p.id(), seq, cut::proof(blocks, &h, &history_key(seq)).unwrap(), whole.entries[seq as usize].encode())
}
fn mine(blocks: &cut::Blocks, live: &State, kind: u8, who: u8, n: u8) -> Vec<u8> {
    cut::proof(blocks, &roots(live).1, &received_key(kind, &did(who), &nonce(n))).unwrap()
}

/// Token A: 1 (genesis) sends 300 to 2 (received), allows 5 up to 50; then 1 CUTS at stamp 5. Returns the world, 1's
/// whole record, the blocks.
fn cut_world() -> (Two, State, cut::Blocks) {
    let mut w = Two(HashMap::new());
    put_at(w.chain(id_a(), 1), 1, Body::Genesis { amount: 1000 }, Some(1), 0);
    put_at(w.chain(id_a(), 1), 2, Body::Send { to: did(2), amount: 300 }, Some(1), 0);
    put_at(w.chain(id_a(), 1), 3, Body::Allow { spender: pk(5), limit: 50 }, Some(1), 0);
    put_at(w.chain(id_a(), 1), 4, Body::Send { to: did(3), amount: 100 }, Some(1), 0);
    put_at(w.chain(id_a(), 2), 2, Body::Receive { from: did(1), nonce: nonce(2), amount: 300, cited: 0, ev: Evidence::default() }, None, 0);
    let mut blocks = cut::Blocks::new();
    let whole = w.get(id_a(), 1).clone();
    let whole = cut_at(&mut w, &whole, id_a(), 1, 5, &mut blocks);
    (w, whole, blocks)
}

#[test]
fn a_cut_carries_what_came_before_and_the_chain_lives_on_from_it() {
    let (mut w, whole, _) = cut_world();
    let live = w.get(id_a(), 1).clone();
    assert_eq!(live.entries.len(), 1, "only the cut is live");
    assert_eq!(live.entries[0].seq, 5);
    assert_eq!(check(&at(id_a(), 1), &def(), &whole, &|_, _| None, &|_| None, Some(&log(1))), Ok(()), "with what it summarises");
    assert_eq!(w.check(id_a(), 1), Ok(()), "from the cut alone");
    assert_eq!(live.balance(), 600);
    let Body::Cut { allowances, .. } = &live.entries[0].body else { panic!() };
    assert_eq!(allowances, &vec![(pk(5), 50)]);
    // The carried allowance is spent after the cut, and the chain goes on.
    spent(w.chain(id_a(), 1), 5, 6, 3, 40);
    assert_eq!(w.check(id_a(), 1), Ok(()));
    spent(w.chain(id_a(), 1), 5, 7, 3, 11);
    assert_eq!(w.check(id_a(), 1), Err("a spend over its allowance"));
}

#[test]
fn a_cut_must_carry_exactly_what_came_before_it() {
    let (_, whole, _) = cut_world();
    let n = whole.entries.len() - 1;
    let lie = |f: &dyn Fn(&mut Body)| {
        let mut s = whole.clone();
        let mut e = s.entries.pop().unwrap();
        f(&mut e.body);
        s.entries.push(sign(e, 1));
        check(&at(id_a(), 1), &def(), &s, &|_, _| None, &|_| None, Some(&log(1)))
    };
    assert_eq!(lie(&|_| {}), Ok(()), "control: the honest cut");
    assert_eq!(lie(&|b| if let Body::Cut { allowances, .. } = b { allowances[0].1 = 51 }), Err("a cut that is not what came before it"));
    assert_eq!(lie(&|b| if let Body::Cut { allowances, .. } = b { allowances.clear() }), Err("a cut that is not what came before it"));
    assert_eq!(lie(&|b| if let Body::Cut { offers, .. } = b { offers.push((nonce(9), id_b(), 1, 1, 1, 9)) }), Err("a cut that is not what came before it"));
    assert_eq!(lie(&|b| if let Body::Cut { locks, .. } = b { locks.push((nonce(9), did(2), 10, 9, id_b(), nonce(9), true)) }), Err("a cut that is not what came before it"));
    // A cut naming another device as the chain's writer (handing it the chain): not what came before it.
    assert_eq!(lie(&|b| if let Body::Cut { writer, .. } = b { *writer = Some((pk(9), 0)) }), Err("a cut that is not what came before it"));
    let mut s = whole.clone();
    let mut e = s.entries.pop().unwrap();
    e.balance += 1;
    s.entries.push(sign(e, 1));
    assert_eq!(check(&at(id_a(), 1), &def(), &s, &|_, _| None, &|_| None, Some(&log(1))), Err("a carried balance that does not add up"));
    // Only the writer cuts.
    let mut s = whole.clone();
    let e = s.entries.pop().unwrap();
    s.entries.push(sign(e, 5));
    assert_eq!(check(&at(id_a(), 1), &def(), &s, &|_, _| None, &|_| None, Some(&log(1))), Err("an entry not signed by the chain's writer"));
    assert_eq!(n, 5);
}

#[test]
fn a_chain_starts_at_genesis_or_at_its_cut() {
    let (_, whole, _) = cut_world();
    let mid = State { entries: whole.entries[2..].to_vec(), fork: None };
    assert_eq!(check(&at(id_a(), 1), &def(), &mid, &|_, _| None, &|_| None, Some(&log(1))), Err("a chain that starts mid-way without its cut"));
    assert_eq!(check(&at(id_a(), 1), &def(), &prune(&whole), &|_, _| None, &|_| None, Some(&log(1))), Ok(()));
}

#[test]
fn a_send_received_before_a_cut_is_never_received_again() {
    // 2 has received 1's send (nonce 2); 2 CUTS at stamp 5.
    let (mut w, _, mut blocks) = cut_world();
    let whole2 = w.get(id_a(), 2).clone();
    cut_at(&mut w, &whole2, id_a(), 2, 5, &mut blocks);
    let live2 = w.get(id_a(), 2).clone();
    // 1's chain shows the send as LIVE here (1's cut is undone for this test: only 2's received set is under test).
    let mut ww = Two(w.0.clone());
    ww.0.insert((id_a(), did(1)), State { entries: cut_world().1.entries[..5].to_vec(), fork: None });
    let again = |ev: Evidence| {
        let mut x = Two(ww.0.clone());
        put_at(x.chain(id_a(), 2), 2, Body::Receive { from: did(1), nonce: nonce(2), amount: 300, cited: 0, ev }, None, 6);
        x.strict(id_a(), 2, 1)
    };
    // The proof the writer CAN make is of presence: refused, as is none.
    let present = mine(&blocks, &live2, RECEIVED, 1, 2);
    assert_eq!(again(Evidence { mine: present, theirs: vec![] }), Err("a send received twice"));
    assert_eq!(again(Evidence::default()), Err("a send received twice"));
    // CONTROL: 1's other send (nonce 4, to 3) received by 3 after 3 cuts — not in 3's set: its absence proof passes.
    let mut x = Two(ww.0.clone());
    put_at(x.chain(id_a(), 3), 1, Body::Burn { amount: 0 }, Some(3), 0);
    x.chain(id_a(), 3).entries.clear();
    let mut whole3 = State::default();
    put_at(&mut whole3, 8, Body::Allow { spender: pk(5), limit: 1 }, Some(3), 0);
    *x.chain(id_a(), 3) = whole3.clone();
    let mut b3 = cut::Blocks::new();
    cut_at(&mut x, &whole3, id_a(), 3, 5, &mut b3);
    let live3 = x.get(id_a(), 3).clone();
    let absent = mine(&b3, &live3, RECEIVED, 1, 4);
    let mut ok = Two(x.0.clone());
    put_at(ok.chain(id_a(), 3), 4, Body::Receive { from: did(1), nonce: nonce(4), amount: 100, cited: 0, ev: Evidence { mine: absent, theirs: vec![] } }, None, 6);
    assert_eq!(ok.strict(id_a(), 3, 1), Ok(()));
    let mut no = Two(x.0.clone());
    put_at(no.chain(id_a(), 3), 4, Body::Receive { from: did(1), nonce: nonce(4), amount: 100, cited: 0, ev: Evidence::default() }, None, 6);
    assert_eq!(no.strict(id_a(), 3, 1), Err("a send received twice"), "stamped before the cut: the proof is required");
}

#[test]
fn a_send_behind_its_senders_cut_is_received_with_the_proof() {
    // 1 sent 100 to 3 (seq 3), then cut: the send is not live.
    let (w, whole1, blocks) = cut_world();
    let live1 = w.get(id_a(), 1).clone();
    let p1 = at(id_a(), 1);
    let recv = |theirs| {
        let mut x = Two(w.0.clone());
        put_at(x.chain(id_a(), 3), 4, Body::Receive { from: did(1), nonce: nonce(4), amount: 100, cited: 0, ev: Evidence { mine: vec![], theirs } }, None, 6);
        x.strict(id_a(), 3, 1)
    };
    assert_eq!(recv(vec![behind(&blocks, &live1, &p1, &whole1, 4)]), Ok(()));
    assert_eq!(recv(vec![]), Err("a receive whose send is not in its sender's chain"));
    // Another entry's bytes; a proof of another seq; another chain's id.
    let (id, seq, proof, _) = behind(&blocks, &live1, &p1, &whole1, 4);
    assert_eq!(recv(vec![(id, seq, proof.clone(), whole1.entries[2].encode())]), Err("a receive whose send is not in its sender's chain"));
    let bytes1 = whole1.entries[2].encode();
    assert_eq!(recv(vec![(id, 2, proof.clone(), bytes1)]), Err("a receive whose send is not in its sender's chain"));
    assert_eq!(recv(vec![(id, 4, proof[..proof.len() - 1].to_vec(), whole1.entries[4].encode())]), Err("a receive whose send is not in its sender's chain"));
    // A history that is not the cut's: entry 3 forged (to 3 for 150), proven against its own root.
    let mut forged = whole1.clone();
    forged.entries[4].body = Body::Send { to: did(3), amount: 150 };
    let mut fb = cut::Blocks::new();
    let ((fh, _), _) = cut::trees(&forged.entries[..5], &mut fb).unwrap();
    let fproof = cut::proof(&fb, &fh, &history_key(4)).unwrap();
    assert_eq!(recv(vec![(id, 4, fproof, forged.entries[4].encode())]), Err("a receive whose send is not in its sender's chain"));
    // The forged entry's bytes beside the REAL proof: the proof holds other bytes at that seq.
    assert_eq!(recv(vec![(id, 4, proof.clone(), forged.entries[4].encode())]), Err("a receive whose send is not in its sender's chain"));
    let mut also = whole1.entries[4].clone();
    also.body = Body::Send { to: did(3), amount: 100 };
    also.stamp = 0;
    assert_eq!(recv(vec![(id, 4, proof.clone(), also.encode())]), Ok(()), "control: the bytes the proof holds");
    // The same send with another carried balance: what the receive looks for, but not the bytes the history holds.
    let mut other = whole1.entries[4].clone();
    other.balance += 1;
    assert_eq!(recv(vec![(id, 4, proof.clone(), other.encode())]), Err("a receive whose send is not in its sender's chain"));
    assert_eq!(recv(vec![(at(id_a(), 2).id(), 4, proof, whole1.entries[4].encode())]), Err("a receive whose send is not in its sender's chain"));
}

#[test]
fn a_lock_filled_behind_the_makers_cut_is_never_refunded_or_filled_again() {
    let mut w = swapped();
    // T locks a second time (21), never filled; M then CUTS its chain of A at stamp 5 (the fill of 20 behind it).
    put_at(w.chain(id_b(), T), 21, Body::Lock { to: did(M), amount: 50, until: 50, token: id_a(), offer: nonce(10) }, Some(T), 0);
    let wholem = w.get(id_a(), M).clone();
    let mut blocks = cut::Blocks::new();
    cut_at(&mut w, &wholem, id_a(), M, 5, &mut blocks);
    let livem = w.get(id_a(), M).clone();
    assert_eq!(w.check(id_a(), M), Ok(()));
    let Body::Cut { offers, .. } = &livem.entries[0].body else { panic!() };
    assert_eq!(offers, &vec![(nonce(10), id_b(), 2, 1, 200, 1000)], "the open offer carried, what is left of it, its time");
    let mid = at(id_a(), M).id();
    let refund = |lock: u8, ev: Evidence| {
        let mut x = Two(w.0.clone());
        put_at(x.chain(id_b(), T), 60, Body::Refund { lock: nonce(lock), ev }, None, 60);
        x.strict(id_b(), T, 1)
    };
    let fill_proof = |lock: u8| cut::proof(&blocks, &roots(&livem).1, &received_key(FILLED, &did(T), &nonce(lock))).unwrap();
    // 20 was filled: its proof is of presence — refused, as is none.
    assert_eq!(refund(20, Evidence { mine: vec![], theirs: vec![(mid, 0, fill_proof(20), vec![])] }), Err("a refund of a lock the maker's history may have filled"));
    assert_eq!(refund(20, Evidence::default()), Err("a refund of a lock the maker's history may have filled"));
    // 21 was not: its absence proof refunds it.
    assert_eq!(refund(21, Evidence { mine: vec![], theirs: vec![(mid, 0, fill_proof(21), vec![])] }), Ok(()));
    assert_eq!(refund(21, Evidence::default()), Err("a refund of a lock the maker's history may have filled"));
    // Filling 20 again after the cut: the maker's own set shows it filled.
    let mut x = Two(w.0.clone());
    put_at(x.chain(id_a(), M), 31, Body::Fill { offer: nonce(10), taker: did(T), lock: nonce(20), amount: 10, cited: 0, until: 50, ev: Evidence { mine: fill_proof(20), theirs: vec![] } }, None, 6);
    assert_eq!(x.strict(id_a(), M, 1), Err("a lock filled twice"));
}

#[test]
fn a_holder_checks_a_new_cut_against_what_it_summarises_then_drops_it() {
    let (_, whole, _) = cut_world();
    let held = State { entries: whole.entries[..5].to_vec(), fork: None };
    let sent = prune(&whole);
    // The holder's merge keeps the prefix: the cut is checked against it.
    let m = merge(&held, &sent);
    assert_eq!(m, whole);
    assert_eq!(merge(&sent, &held), whole);
    assert_eq!(accept(&at(id_a(), 1), &def(), 6, &held, &m, &|_, _| None, &|_| None, Some(&log(1))), Ok(sent.clone()), "kept: from the cut");
    // A lying cut sent the same way is refused there.
    let mut bad = sent.clone();
    let mut e = bad.entries.pop().unwrap();
    if let Body::Cut { allowances, .. } = &mut e.body {
        allowances.clear();
    }
    bad.entries.push(sign(e, 1));
    assert_eq!(accept(&at(id_a(), 1), &def(), 6, &held, &merge(&held, &bad), &|_, _| None, &|_| None, Some(&log(1))), Err("a cut that is not what came before it"));
    // CONTROL: a holder that never had the prefix cannot check it (the wallet does).
    assert_eq!(accept(&at(id_a(), 1), &def(), 6, &State::default(), &bad, &|_, _| None, &|_| None, Some(&log(1))), Ok(bad.clone()));
    // Copies settle the same in every order (pruned).
    let r = all_orders(&[held.clone(), sent.clone(), whole.clone()]);
    assert!(r.iter().all(|x| prune(x) == sent));
    // A copy that ends well before the cut: the cut wins. A FORK below the cut: the fork stays.
    let early = State { entries: whole.entries[..2].to_vec(), fork: None };
    assert_eq!(merge(&early, &sent), sent);
    let mut f1 = early.clone();
    signed(&mut f1, 1, 50, Body::Burn { amount: 1 });
    let mut f2 = early.clone();
    signed(&mut f2, 1, 51, Body::Burn { amount: 2 });
    let frozen = merge(&f1, &f2);
    assert!(frozen.fork.is_some());
    assert_eq!(merge(&frozen, &sent), frozen);
    assert_eq!(merge(&sent, &frozen), frozen);
}

fn verify_with(w: &Two, blocks: &cut::Blocks, wallet: &mut Wallet, owner: u8) -> Result<(), FraudProof> {
    let chain = |p: &ChainParams| w.0.get(&(p.definition, p.owner)).cloned();
    let block = |c: &[u8; 32]| blocks.get(c).cloned();
    let src = Source { chain: &chain, definition: &defs, block: &block, keys: &keys_of, supply: &|_| None, authority: &|_| None };
    let s = w.get(id_a(), owner);
    let last = s.entries.last().unwrap().seq;
    fraud(wallet.verify(&src, &at(id_a(), owner), last))
}

#[test]
fn a_wallet_walks_across_a_cut_from_its_published_history() {
    let (mut w, whole1, blocks) = cut_world();
    let live1 = w.get(id_a(), 1).clone();
    let p1 = at(id_a(), 1);
    put_at(w.chain(id_a(), 3), 4, Body::Receive { from: did(1), nonce: nonce(4), amount: 100, cited: 0, ev: Evidence { mine: vec![], theirs: vec![behind(&blocks, &live1, &p1, &whole1, 3)] } }, None, 6);
    assert_eq!(verify_with(&w, &blocks, &mut Wallet::new(pk(9)), 3), Ok(()));
    // Without the history's blocks HELD the walk cannot cross: it says which it NEEDS (fetched by the caller through the
    // blocks door), never a verdict — and nothing is asked of any network here.
    let chain = |p: &ChainParams| w.0.get(&(p.definition, p.owner)).cloned();
    let src = Source { chain: &chain, definition: &defs, block: &|_| None, keys: &keys_of, supply: &|_| None, authority: &|_| None };
    let need = Wallet::new(pk(9)).verify(&src, &at(id_a(), 3), 0);
    let Err(Fail::Need(n)) = need else { panic!("{need:?}") };
    assert!(n.contains(&roots(&live1).0), "the history's root first");
    // A block that is not the one its id names.
    let mut forged = blocks.clone();
    let k = roots(&live1).0;
    forged.insert(k, b"not it".to_vec());
    let f = verify_with(&w, &forged, &mut Wallet::new(pk(9)), 3).unwrap_err();
    assert_eq!(f.why, "a history block that is not the one named");
}

#[test]
fn a_lying_cut_passes_the_contract_and_is_proven_by_the_wallet() {
    // 2 received 1's send, then cuts with a received set that LEAVES IT OUT — and receives it again.
    let (mut w, _, mut blocks) = cut_world();
    let mut ww = Two(w.0.clone());
    ww.0.insert((id_a(), did(1)), State { entries: cut_world().1.entries[..5].to_vec(), fork: None });
    w = ww;
    let whole2 = w.get(id_a(), 2).clone();
    let honest = cut_at(&mut Two(w.0.clone()), &whole2, id_a(), 2, 5, &mut blocks);
    let lie = |honest: &State, blocks: &mut cut::Blocks| {
        let mut s = honest.clone();
        let mut e = s.entries.pop().unwrap();
        let empty = freenet_prolly::build::build(std::iter::empty(), |c, b| {
            blocks.insert(c, b.to_vec());
        })
        .unwrap();
        if let Body::Cut { received, .. } = &mut e.body {
            *received = empty;
        }
        s.entries.push(sign(e, 2));
        s
    };
    let lying_whole = lie(&honest, &mut blocks);
    let live = prune(&lying_whole);
    let again = mine(&blocks, &live, RECEIVED, 1, 2);
    let mut lying = Two(w.0.clone());
    *lying.chain(id_a(), 2) = live.clone();
    put_at(lying.chain(id_a(), 2), 2, Body::Receive { from: did(1), nonce: nonce(2), amount: 300, cited: 0, ev: Evidence { mine: again, theirs: vec![] } }, None, 6);
    assert_eq!(lying.strict(id_a(), 2, 1), Ok(()), "the contract cannot see it: the received set is the cut's word");
    assert_eq!(lying.get(id_a(), 2).balance(), 600, "300 received twice");
    let f = verify_with(&lying, &blocks, &mut Wallet::new(pk(9)), 2).unwrap_err();
    assert_eq!((f.seq, f.why), (2, "a cut whose trees are not what came before it"));
    // CONTROL: the honest cut, walked the same way, is value.
    let mut fine = Two(w.0.clone());
    *fine.chain(id_a(), 2) = prune(&honest);
    assert_eq!(verify_with(&fine, &blocks, &mut Wallet::new(pk(9)), 2), Ok(()));
    // A lie about what is CARRIED (pruned: the contract cannot see it either) — the wallet's walk of the whole does.
    let mut s = honest.clone();
    let mut e = s.entries.pop().unwrap();
    if let Body::Cut { allowances, .. } = &mut e.body {
        allowances.push((did(5), 1_000));
    }
    s.entries.push(sign(e, 2));
    let mut carried = Two(w.0.clone());
    *carried.chain(id_a(), 2) = prune(&s);
    assert_eq!(carried.check(id_a(), 2), Ok(()));
    let f = verify_with(&carried, &blocks, &mut Wallet::new(pk(9)), 2).unwrap_err();
    assert_eq!((f.seq, f.why), (2, "a cut that is not what came before it"));
    // A cut naming parity its history does not have (a root no reader could rebuild): the audit finds it too.
    let mut s = honest.clone();
    let mut e = s.entries.pop().unwrap();
    if let Body::Cut { history_parity, .. } = &mut e.body {
        history_parity[0] = [7; 32];
    }
    s.entries.push(sign(e, 2));
    let mut parity = Two(w.0.clone());
    *parity.chain(id_a(), 2) = prune(&s);
    let f = verify_with(&parity, &blocks, &mut Wallet::new(pk(9)), 2).unwrap_err();
    assert_eq!((f.seq, f.why), (2, "a cut whose trees are not what came before it"));
}

#[test]
fn an_entry_over_1kb_behind_a_cut_is_proven_by_its_value_block() {
    // M fills T's second lock (21: 50 B → 100 A) carrying 2 KB of evidence, then CUTS: the fill is behind the cut, its
    // bytes over 1 KB — stored as a value block, proven by id and length.
    let mut w = swapped();
    put_at(w.chain(id_b(), T), 21, Body::Lock { to: did(M), amount: 50, until: 50, token: id_a(), offer: nonce(10) }, Some(T), 0);
    let pad = Evidence { mine: vec![], theirs: vec![([1; 32], 0, vec![7; 2048], vec![])] };
    put_at(w.chain(id_a(), M), 31, Body::Fill { offer: nonce(10), taker: did(T), lock: nonce(21), amount: 100, cited: 0, until: 50, ev: pad }, None, 0);
    assert_eq!(w.check(id_a(), M), Ok(()));
    let wholem = w.get(id_a(), M).clone();
    let seq = wholem.entries.len() as u64 - 1;
    assert!(wholem.entries[seq as usize].encode().len() > 1024);
    let mut blocks = cut::Blocks::new();
    let cutm = cut_at(&mut w, &wholem, id_a(), M, 5, &mut blocks);
    let livem = w.get(id_a(), M).clone();
    let (id, s1, proof, bytes) = behind(&blocks, &livem, &at(id_a(), M), &wholem, seq);
    assert!(matches!(freenet_prolly::proof::verify_bytes(&roots(&livem).0, &history_key(seq), &proof), Ok(freenet_prolly::proof::ProvenOwned::Present(freenet_prolly::proof::ProvenValue::Ref { .. }))));
    let recv = |bytes: Vec<u8>| {
        let mut x = Two(w.0.clone());
        put_at(x.chain(id_a(), T), 32, Body::Receive { from: did(M), nonce: nonce(31), amount: 100, cited: 0, ev: Evidence { mine: vec![], theirs: vec![(id, s1, proof.clone(), bytes)] } }, None, 6);
        x.strict(id_a(), T, 1)
    };
    assert_eq!(recv(bytes.clone()), Ok(()));
    // The same entry with other evidence (same length): another block.
    let mut other = wholem.entries[seq as usize].clone();
    if let Body::Fill { ev, .. } = &mut other.body {
        ev.theirs[0].2 = vec![8; 2048];
    }
    assert_eq!(other.encode().len(), bytes.len());
    assert_eq!(recv(other.encode()), Err("a receive whose send is not in its sender's chain"));
    // The wallet reads it back across the cut.
    assert!(cut::whole(&livem, &|c| blocks.get(c).cloned()).unwrap() == cutm);
}

#[test]
fn a_citation_shown_when_appended_stays_valid_after_its_chain_cuts() {
    // T received M's fill (30) — shown live then. M CUTS: the fill is no longer live.
    let mut w = swapped();
    let wholem = w.get(id_a(), M).clone();
    let mut blocks = cut::Blocks::new();
    cut_at(&mut w, &wholem, id_a(), M, 5, &mut blocks);
    // T's chain, checked from scratch, still holds: the citation is one the cut hid, not one never shown.
    assert_eq!(w.check(id_a(), T), Ok(()));
    // CONTROL: the same receive taken in as NEW must show it — refused without its evidence, taken with it.
    assert_eq!(w.strict(id_a(), T, 1), Err("a receive whose send is not in its sender's chain"));
    let livem = w.get(id_a(), M).clone();
    let mut x = Two(w.0.clone());
    let t = x.chain(id_a(), T);
    let mut e = t.entries.pop().unwrap();
    if let Body::Receive { ev, .. } = &mut e.body {
        ev.theirs = vec![behind(&blocks, &livem, &at(id_a(), M), &wholem, 3)];
    }
    t.entries.push(work(e));
    assert_eq!(x.strict(id_a(), T, 1), Ok(()));
    // A citation of a stamp AFTER the cut is never taken as hidden by it.
    let mut y = Two(w.0.clone());
    put_at(y.chain(id_a(), T), 77, Body::Receive { from: did(M), nonce: nonce(77), amount: 50, cited: 9, ev: Evidence::default() }, None, 12);
    assert_eq!(y.check(id_a(), T), Err("a receive whose send is not in its sender's chain"));
}

#[test]
fn a_history_read_back_is_every_seq_in_order_and_all_before_the_cut() {
    let (w, whole, _) = cut_world();
    let live = w.get(id_a(), 1).clone();
    let n = live.entries[0].seq as usize;
    // A history (its own root) whose leaves are not seq 0, 1, 2…: the key of seq 1 holding the entry of seq 2.
    let mut blocks = cut::Blocks::new();
    let mut b = freenet_prolly::build::TreeBuilder::new(|c, x: &[u8]| {
        blocks.insert(c, x.to_vec());
    });
    for (k, e) in [(0u64, 0usize), (1, 2), (2, 1), (3, 3)] {
        b.push_bytes(&history_key(k), &whole.entries[e].encode()).unwrap();
    }
    let shuffled = b.finish().unwrap();
    let with_root = |root: [u8; 32]| {
        let mut s = live.clone();
        if let Body::Cut { history, .. } = &mut s.entries[0].body {
            *history = root;
        }
        s
    };
    assert_eq!(cut::whole(&with_root(shuffled), &|c| blocks.get(c).cloned()).err(), Some(cut::Read::Refused("a history that is not every seq in order")));
    // A history that stops short of the cut (seq 0 to 2 of 0 to 3).
    let mut short = cut::Blocks::new();
    let ((h3, _), _) = cut::trees(&whole.entries[..n - 1], &mut short).unwrap();
    assert_eq!(cut::whole(&with_root(h3), &|c| short.get(c).cloned()).err(), Some(cut::Read::Refused("a cut whose history is not everything before it")));
    // CONTROL: the cut's own history reads back whole.
    let mut fine = cut::Blocks::new();
    let ((h, _), _) = cut::trees(&whole.entries[..n], &mut fine).unwrap();
    assert_eq!(cut::whole(&with_root(h), &|c| fine.get(c).cloned()).map(|s| s.entries.len()), Ok(n + 1));
}

#[test]
fn after_a_cut_a_fill_and_a_refund_still_name_and_find_what_they_need() {
    // M's offer and T's lock live on only in their chains' CUTS.
    let mut w = swapped();
    put_at(w.chain(id_b(), T), 21, Body::Lock { to: did(M), amount: 50, until: 50, token: id_a(), offer: nonce(10) }, Some(T), 0);
    let mut blocks = cut::Blocks::new();
    let (wm, wt) = (w.get(id_a(), M).clone(), w.get(id_b(), T).clone());
    cut_at(&mut w, &wm, id_a(), M, 5, &mut blocks);
    cut_at(&mut w, &wt, id_b(), T, 5, &mut blocks);
    // A FILL of the carried offer names the taker's chain (what validation asks the node for) …
    let mut m = w.get(id_a(), M).clone();
    put_at(&mut m, 32, Body::Fill { offer: nonce(10), taker: did(T), lock: nonce(21), amount: 100, cited: 0, until: 50, ev: Evidence::default() }, None, 6);
    assert!(cited_chains(&at(id_a(), M), &m).contains(&(id_b(), did(T))), "the taker's chain of the offer's token");
    // … and a REFUND of the carried lock names the maker's chain, and its deadline is found for the append rules.
    let held = w.get(id_b(), T).clone();
    let mut t = held.clone();
    put_at(&mut t, 60, Body::Refund { lock: nonce(21), ev: Evidence::default() }, None, 50);
    assert!(cited_chains(&at(id_b(), T), &t).contains(&(id_a(), did(M))));
    assert_eq!(append_check(&def_b(), 50, &held, &t, Some(&log(T))), Ok(()), "at its deadline");
    let mut early = held.clone();
    put_at(&mut early, 60, Body::Refund { lock: nonce(21), ev: Evidence::default() }, None, 49);
    assert_eq!(append_check(&def_b(), 49, &held, &early, Some(&log(T))), Err("a refund before its lock's deadline"));
}

#[test]
fn an_offer_is_filled_only_before_its_time() {
    // swapped(): M's offer runs until 1000.
    let fill = |stamp: u64| {
        let mut w = swapped();
        put_at(w.chain(id_b(), T), 21, Body::Lock { to: did(M), amount: 50, until: 2000, token: id_a(), offer: nonce(10) }, Some(T), 0);
        put_at(w.chain(id_a(), M), 31, Body::Fill { offer: nonce(10), taker: did(T), lock: nonce(21), amount: 100, cited: 0, until: 2000, ev: Evidence::default() }, None, stamp);
        w.check(id_a(), M)
    };
    assert_eq!(fill(999), Ok(()));
    assert_eq!(fill(1000), Err("a fill of an offer past its time"));
}

#[test]
fn a_lost_history_block_is_rebuilt_from_its_group_never_waited_on() {
    // A history big enough to branch (300 entries): its root and an inner node each lost in turn.
    let mut s = State::default();
    put_at(&mut s, 1, Body::Genesis { amount: 1000 }, Some(1), 0);
    for i in 0..299u32 {
        let mut e = entry(&s, nonce((i % 250) as u8), Body::Allow { spender: pk(5), limit: 10 + i as u128 });
        e.nonce[0] = (i / 250) as u8;
        s.entries.push(sign(e, 1));
    }
    let mut blocks = cut::Blocks::new();
    let ((root, parity), _) = cut::trees(&s.entries, &mut blocks).unwrap();
    struct Held<'a>(&'a cut::Blocks);
    impl freenet_prolly::store::Blocks for Held<'_> {
        fn get(&self, c: &[u8; 32]) -> Option<&[u8]> {
            self.0.get(c).map(Vec::as_slice)
        }
    }
    let rebuild = |held: &cut::Blocks, g: &engine::repair::Group| {
        let have: Vec<Option<Vec<u8>>> = g.slots.iter().enumerate().map(|(i, c)| held.get(c).map(|b| g.stored(i, b))).collect();
        engine::repair::rebuild(g, &have)
    };
    // THE ROOT lost: its group of one (the parity ids the cut carries) rebuilds it.
    assert!(!parity.is_empty());
    let mut held = blocks.clone();
    let lost = held.remove(&root).unwrap();
    assert_eq!(rebuild(&held, &engine::repair::root_group(root, &parity)).as_deref(), Ok(&lost[..]));
    // AN INNER NODE lost (a child the root lists): its group, found in the root, rebuilds it.
    let root_node = freenet_prolly::node::Node::parse(&blocks[&root]).unwrap();
    assert!(root_node.level() > 0, "a branching history");
    let child = root_node.child(0).0;
    let mut held = blocks.clone();
    let lost = held.remove(&child).unwrap();
    let g = engine::repair::find_group(&Held(&held), root, child).expect("a held node names its group");
    assert_eq!(rebuild(&held, &g).as_deref(), Ok(&lost[..]));
    // CONTROL: the same history built WITHOUT parity (`finish`, as the first cut code did).
    let mut bare = cut::Blocks::new();
    let mut tb = freenet_prolly::build::TreeBuilder::new(|c, b: &[u8]| {
        bare.insert(c, b.to_vec());
    });
    for e in &s.entries {
        tb.push_bytes(&history_key(e.seq), &e.encode()).unwrap();
    }
    let broot = tb.finish().unwrap();
    let bnode = freenet_prolly::node::Node::parse(&bare[&broot]).unwrap();
    let bchild = bnode.child(0).0;
    bare.remove(&bchild);
    // (Its nodes still NAME the group; its parity blocks were never made — nothing to rebuild from.)
    let g = engine::repair::find_group(&Held(&bare), broot, bchild).expect("the group is named");
    assert!(rebuild(&bare, &g).is_err(), "without its parity blocks a lost node stays lost");
}

#[test]
fn a_citation_appended_after_its_chains_cut_must_show_its_evidence_even_from_scratch() {
    // M cuts at stamp 5 (margin 2): the fill T's receive cites is behind it.
    let mut w = swapped();
    let wholem = w.get(id_a(), M).clone();
    let mut blocks = cut::Blocks::new();
    cut_at(&mut w, &wholem, id_a(), M, 5, &mut blocks);
    let receive = |stamp: u64| {
        let mut x = Two(w.0.clone());
        let t = x.chain(id_a(), T);
        t.entries.clear();
        put_at(t, 30, Body::Receive { from: did(M), nonce: nonce(30), amount: 200, cited: 0, ev: Evidence::default() }, None, stamp);
        x.check(id_a(), T)
    };
    // Appended before the cut (stamped within its margin): taken as shown then.
    assert_eq!(receive(7), Ok(()));
    // Stamped after the cut and its margin, with no evidence — a fresh copy checked from scratch refuses it.
    assert_eq!(receive(8), Err("a receive whose send is not in its sender's chain"));
    // A REFUND of T's lock, the maker cut since: appended before the cut it is taken; after it, it must show the
    // fill's absence (here: none shown).
    let mut w2 = swapped();
    put_at(w2.chain(id_b(), T), 21, Body::Lock { to: did(M), amount: 50, until: 6, token: id_a(), offer: nonce(10) }, Some(T), 0);
    let wm = w2.get(id_a(), M).clone();
    cut_at(&mut w2, &wm, id_a(), M, 5, &mut cut::Blocks::new());
    let refund = |stamp: u64| {
        let mut x = Two(w2.0.clone());
        put_at(x.chain(id_b(), T), 60, Body::Refund { lock: nonce(21), ev: Evidence::default() }, None, stamp);
        x.check(id_b(), T)
    };
    assert_eq!(refund(7), Ok(()));
    assert_eq!(refund(8), Err("a refund of a lock the maker's history may have filled"));
}

// ---- foundation 2: the owner is a DID, one device writes ----

/// A writer entry for chain `s` of person `n`: its device `node`'s credential signed by `owner_key` (seed), naming
/// key seq `k`, signed by `signer`.
fn writer_entry(s: &State, n: u8, node: u8, owner_key: &[u8; 32], k: u32, signer: u8) -> Entry {
    let c = craftworks_identity::credential(&did(n), &pk(node), &[], owner_key);
    sign(entry(s, [0xee; 16], Body::Writer { cred: c, key_seq: k }), signer)
}

#[test]
fn a_writer_is_a_device_of_the_owner_credentialed_by_its_key_log() {
    let p = at(id_a(), 1);
    let try_writer = |e: Entry| {
        let mut s = State::default();
        s.entries.push(e);
        check(&p, &def(), &s, &|_, _| None, &|_| None, Some(&log(1)))
    };
    let s0 = State::default();
    assert_eq!(try_writer(writer_entry(&s0, 1, 1, &owner_seed(1), 0, 1)), Ok(()), "control: its own device, its key at seq 0");
    assert_eq!(try_writer(writer_entry(&s0, 2, 1, &owner_seed(2), 0, 1)), Err("a writer credential for another account"));
    assert_eq!(try_writer(writer_entry(&s0, 1, 1, &[42; 32], 0, 1)), Err("a writer credential not signed by the account's key at that seq"));
    assert_eq!(try_writer(writer_entry(&s0, 1, 1, &owner_seed(1), 1, 1)), Err("a writer credential not signed by the account's key at that seq"), "no event at seq 1");
    assert_eq!(try_writer(writer_entry(&s0, 1, 1, &owner_seed(1), 0, 7)), Err("a writer entry not signed by its device"));
    let mut s = State::default();
    s.entries.push(writer_entry(&s, 1, 1, &owner_seed(1), 0, 1));
    assert_eq!(check(&p, &def(), &s, &|_, _| None, &|_| None, None), Err("the owner's key log is not there"), "the key log is required");
    // Genesis comes right after the writer — not after anything else.
    signed(&mut s, 1, 1, Body::Genesis { amount: 1000 });
    assert_eq!(check(&p, &def(), &s, &|_, _| None, &|_| None, Some(&log(1))), Ok(()));
    let mut late = State::default();
    signed(&mut late, 1, 2, Body::Allow { spender: pk(5), limit: 1 });
    signed(&mut late, 1, 1, Body::Genesis { amount: 1000 });
    assert_eq!(check(&p, &def(), &late, &|_, _| None, &|_| None, Some(&log(1))), Err("a genesis that is not the definition's"));
    // Another device of the same account, never named writer, does not sign.
    let mut other = s.clone();
    let e = sign(entry(&other, nonce(3), Body::Burn { amount: 1 }), 30);
    other.entries.push(e);
    assert_eq!(check(&p, &def(), &other, &|_, _| None, &|_| None, Some(&log(1))), Err("an entry not signed by the chain's writer"));
}

#[test]
fn a_rotation_revokes_older_devices_and_a_new_writer_stands_on_the_new_key() {
    // 1's chain written by device 1 on key seq 0; then the account ROTATES its key (seq 1).
    let mut held = State::default();
    signed(&mut held, 1, 1, Body::Genesis { amount: 1000 });
    let keys = rotated(1);
    let p = at(id_a(), 1);
    // The old device writes on: refused by the append rule (still well-formed from scratch: it was valid then).
    let mut old = held.clone();
    signed(&mut old, 1, 2, Body::Burn { amount: 1 });
    assert_eq!(append_check(&def(), 0, &held, &old, Some(&keys)), Err("an owner entry not on the account's current key"));
    assert_eq!(check(&p, &def(), &old, &|_, _| None, &|_| None, Some(&keys)), Ok(()));
    // A NEW writer (device 31) credentialed by the rotated key at seq 1: taken; it writes on.
    let mut new = held.clone();
    let e = writer_entry(&new, 1, 31, &next_seed(1), 1, 31);
    new.entries.push(e);
    let e = sign(entry(&new, nonce(4), Body::Burn { amount: 1 }), 31);
    new.entries.push(e);
    assert_eq!(append_check(&def(), 0, &held, &new, Some(&keys)), Ok(()));
    assert_eq!(check(&p, &def(), &new, &|_, _| None, &|_| None, Some(&keys)), Ok(()));
    // …and the revoked device's entries after it are not its writer's.
    let mut after = new.clone();
    let e = sign(entry(&after, nonce(5), Body::Burn { amount: 1 }), 1);
    after.entries.push(e);
    assert_eq!(check(&p, &def(), &after, &|_, _| None, &|_| None, Some(&keys)), Err("an entry not signed by the chain's writer"));
    // A writer stepping back to the older key: refused.
    let mut back = new.clone();
    let e = writer_entry(&back, 1, 1, &owner_seed(1), 0, 1);
    back.entries.push(e);
    assert_eq!(check(&p, &def(), &back, &|_, _| None, &|_| None, Some(&keys)), Err("a writer on an older key than the one before"));
}

#[test]
fn the_recovered_account_outranks_a_stolen_device_and_one_key_signing_twice_still_freezes() {
    let mut base = State::default();
    signed(&mut base, 1, 1, Body::Genesis { amount: 1000 });
    // The THIEF (device 1, key seq 0) spends at the next seq — twice (an equivocation of its own).
    let mut thief = base.clone();
    signed(&mut thief, 1, 2, Body::Send { to: did(9), amount: 900 });
    let mut thief2 = base.clone();
    signed(&mut thief2, 1, 3, Body::Send { to: did(9), amount: 800 });
    // The ACCOUNT, recovered by its words (rotated: key seq 1), names a new device writer at that seq.
    let mut rescue = base.clone();
    let e = writer_entry(&rescue, 1, 31, &next_seed(1), 1, 31);
    rescue.entries.push(e);
    let r = all_orders(&[thief.clone(), rescue.clone()]);
    assert!(r.windows(2).all(|x| x[0] == x[1]) && r[0] == rescue && r[0].fork.is_none(), "the recovery wins, nothing frozen");
    let frozen = merge(&thief, &thief2);
    assert!(frozen.fork.is_some(), "CONTROL: one key signing twice freezes");
    let r = all_orders(&[thief, thief2, rescue.clone()]);
    assert!(r.windows(2).all(|x| x[0] == x[1]), "one result in every order");
    assert_eq!(r[0], rescue, "the recovery outranks the thief's fork too");
    assert_eq!(check(&at(id_a(), 1), &def(), &r[0], &|_, _| None, &|_| None, Some(&rotated(1))), Ok(()));
}

#[test]
fn the_owners_own_keys_never_stand_behind_its_checkpoint() {
    let w = three_hops();
    let walked = {
        let mut x = Wallet::new(pk(3));
        verify(&w, &mut x, 3).unwrap();
        x.reads.get()
    };
    let chain = |p: &ChainParams| w.0.get(&(p.definition, p.owner)).cloned();
    let src = Source { chain: &chain, definition: &defs, block: &|_| None, keys: &keys_of, supply: &|_| None, authority: &|_| None };
    // 2's checkpoint of its own chain, signed by its KEY LOG's key (not its device's): trusted, and still not used.
    let s = w.get(id_a(), 2);
    let e = s.entries.last().unwrap();
    let chain2 = at(id_a(), 2);
    let m = Checkpoint::message(&chain2, e.seq, &e.hash(), e.balance);
    let ok = SigningKey::from_bytes(&owner_seed(2));
    let own = Checkpoint { chain: chain2, seq: e.seq, head: e.hash(), balance: e.balance, by: ok.verifying_key().to_bytes(), sig: ok.sign(&m).to_bytes() };
    assert!(crate::wallet::owner_keys(&src, &chain2).contains(&own.by));
    let mut x = Wallet::new(pk(3));
    x.trusted = vec![own.by];
    x.checkpoints = vec![own];
    verify(&w, &mut x, 3).unwrap();
    assert_eq!(x.reads.get(), walked, "walked in full: the owner's checkpoint did not count");
    // CONTROL: the same made by a key that is not the owner's, trusted: used.
    let mut y = Wallet::new(pk(3));
    y.trusted = vec![pk(9)];
    y.checkpoints = vec![checkpoint(&w, 2, 9)];
    verify(&w, &mut y, 3).unwrap();
    assert!(y.reads.get() < walked);
}

#[test]
fn a_holder_takes_a_receive_without_the_owners_key_log_but_never_a_new_writer() {
    // 1's chain (written by device 1) held; someone appends a receive: no key log needed — the writer was checked when
    // the holder took it.
    let mut held = State::default();
    signed(&mut held, 1, 1, Body::Genesis { amount: 1000 });
    let mut merged = held.clone();
    put_at(&mut merged, 2, Body::Receive { from: did(9), nonce: nonce(2), amount: 50, cited: 0, ev: Evidence::default() }, None, 0);
    let from = new_from(&held, &merged);
    let r = walk(&at(id_a(), 1), &def(), &merged, &|_, _| None, &|_| None, None, 0, from).map(|_| ()).map_err(|x| x.1);
    assert_eq!(r, Ok(()), "the held writer is not checked again");
    // A NEW writer without its key log: refused. From scratch without it: refused too.
    let mut neww = held.clone();
    let e = writer_entry(&neww, 1, 31, &owner_seed(1), 0, 31);
    neww.entries.push(e);
    let from = new_from(&held, &neww);
    assert_eq!(walk(&at(id_a(), 1), &def(), &neww, &|_, _| None, &|_| None, None, 0, from).map(|_| ()).map_err(|x| x.1), Err("the owner's key log is not there"));
    assert_eq!(check(&at(id_a(), 1), &def(), &held, &|_, _| None, &|_| None, None), Err("the owner's key log is not there"));
}

// ---- FOUNDATION 3: the mint authority, the supply shard, the Mint entry ----

/// The mint authority's key (seed 60) as a one-of-one set.
fn mint_set() -> authority::KeySet {
    authority::KeySet { k: 1, keys: vec![key(60).verifying_key().to_bytes()] }
}
/// A mint of `amount` to person `to`, nonce `n`, at beacon `at`, signed by key `by`.
fn a_mint(to: u8, n: u8, amount: u128, at: u64, by: u8) -> supply::Mint {
    let mut m = supply::Mint { to: did(to), amount, nonce: nonce(n), stamp: at, sigs: vec![] };
    m.sigs = vec![(0, key(by).sign(&m.message(&params(1).definition)).to_bytes())];
    m
}
fn minted(s: &mut State, n: u8, amount: u128, at: u64) {
    let mut e = entry(s, nonce(n), Body::Mint { nonce: nonce(n), amount, minted: at, ev: Evidence::default() });
    e.stamp = e.stamp.max(at);
    s.entries.push(work(e));
}

#[test]
fn a_mint_is_credited_only_if_its_supply_shard_already_holds_it() {
    let shard = supply::Shard { mints: vec![a_mint(2, 1, 500, 3, 60)] };
    let p = params(2);
    let ok = |s: &State, sh: &supply::Shard| check(&p, &def(), s, &|_, _| None, &|e| (e == 0).then(|| sh.clone()), Some(&log(2)));
    let mut b = State::default();
    minted(&mut b, 1, 500, 3);
    assert_eq!(b.balance(), 500);
    assert_eq!(ok(&b, &shard), Ok(()));
    assert_eq!(ok(&b, &supply::Shard::default()), Err("a mint that is not in its supply shard"));
    assert_eq!(check(&p, &def(), &b, &|_, _| None, &|_| None, Some(&log(2))), Err("the mint's supply shard is not there"));
    let other = supply::Shard { mints: vec![a_mint(3, 1, 500, 3, 60)] };
    assert_eq!(ok(&b, &other), Err("a mint that is not in its supply shard"), "minted to another");
    let more = supply::Shard { mints: vec![a_mint(2, 1, 501, 3, 60)] };
    assert_eq!(ok(&b, &more), Err("a mint that is not in its supply shard"), "another amount");
    let mut twice = b.clone();
    minted(&mut twice, 1, 500, 3);
    assert_eq!(ok(&twice, &shard), Err("a mint credited twice"));
    let mut early = State::default();
    minted(&mut early, 1, 500, 3);
    early.entries[0].stamp = 2;
    early.entries[0] = work(early.entries[0].clone());
    assert_eq!(ok(&early, &shard), Err("a mint credited before it was minted"));
    let mut signed_ = State::default();
    let e = sign(entry(&signed_, nonce(1), Body::Mint { nonce: nonce(1), amount: 500, minted: 0, ev: Evidence::default() }), 2);
    signed_.entries.push(e);
    assert_eq!(ok(&signed_, &supply::Shard { mints: vec![a_mint(2, 1, 500, 0, 60)] }), Err("a mint without its work"));
    // The shard of ITS epoch is the one read.
    let at = supply::EPOCH + 1;
    let mut c = State::default();
    minted(&mut c, 4, 70, at);
    let later = supply::Shard { mints: vec![a_mint(2, 4, 70, at, 60)] };
    assert_eq!(check(&p, &def(), &c, &|_, _| None, &|e| (e == 1).then(|| later.clone()), Some(&log(2))), Ok(()));
    assert_eq!(check(&p, &def(), &c, &|_, _| None, &|e| (e == 0).then(|| later.clone()), Some(&log(2))), Err("the mint's supply shard is not there"));
    // And it spends as any balance does.
    signed(&mut b, 2, 9, Body::Send { to: did(3), amount: 200 });
    assert_eq!(ok(&b, &shard), Ok(()));
}

#[test]
fn an_update_reads_the_shard_of_each_new_mint() {
    let shard = supply::Shard { mints: vec![a_mint(2, 1, 500, 3, 60)] };
    let held = State::default();
    let mut b = State::default();
    minted(&mut b, 1, 500, 3);
    let take = |sh: Option<supply::Shard>| accept(&params(2), &def(), 4, &held, &b, &|_, _| None, &|e| if e == 0 { sh.clone() } else { None }, Some(&log(2)));
    assert!(take(Some(shard)).is_ok());
    assert_eq!(take(None), Err("the mint's supply shard is not there"));
    assert_eq!(minted_epochs_from(&b, 0), vec![0]);
    let mut late = b.clone();
    minted(&mut late, 2, 7, supply::EPOCH + 1);
    assert_eq!(minted_epochs_from(&late, 0), vec![0, 1]);
    assert_eq!(minted_epochs_from(&late, 1), vec![1]);
}

#[test]
fn a_frozen_account_appends_no_debit_but_still_takes_credits() {
    let freezer = Definition { freeze: Some(mint_set()), ..def() };
    let w = paid();
    let b = w.0[&did(2)].clone();
    let mut sent = b.clone();
    signed(&mut sent, 2, 5, Body::Send { to: did(3), amount: 100 });
    let mut credited = b.clone();
    received(&mut credited, 1, 7, 50);
    let word = |frozen: bool| {
        let mut r = freeze::Record { owner: did(2), frozen, seq: frozen as u32 ^ 1, stamp: 0, sigs: vec![] };
        r.sigs = vec![(0, key(60).sign(&r.message(&params(1).definition)).to_bytes())];
        freeze::List { records: vec![r] }
    };
    let (frozen, thawed) = (word(true), word(false));
    let p = params(2);
    assert_eq!(freeze_check(&p, &freezer, &b, &sent, &frozen), Err("a debit of a frozen account"));
    assert_eq!(freeze_check(&p, &freezer, &b, &credited, &frozen), Ok(()), "a credit still lands");
    assert_eq!(freeze_check(&p, &freezer, &b, &sent, &thawed), Ok(()), "thawed");
    assert_eq!(freeze_check(&p, &def(), &b, &sent, &frozen), Ok(()), "a token with no freeze authority");
    assert_eq!(freeze_check(&params(3), &freezer, &b, &sent, &frozen), Ok(()), "another account");
    assert_eq!(freeze_check(&p, &freezer, &sent, &sent, &frozen), Ok(()), "a debit held before the freeze stays");
    let mut allowed = b.clone();
    signed(&mut allowed, 2, 6, Body::Allow { spender: pk(4), limit: 10 });
    assert_eq!(freeze_check(&p, &freezer, &b, &allowed, &frozen), Ok(()), "a permission is not a debit (its spend is)");
    assert!(debits_from(&sent, b.entries.len()) && !debits_from(&credited, b.entries.len()));
}

#[test]
fn a_wallet_counts_a_mint_only_against_the_shard_and_authority_chain_it_holds() {
    let md = Definition { mint: Some(mint_set()), ..def() };
    let mid = id(&CODE, &md.encode());
    let signed_mint = |n: u8, at_: u64, by: u8| {
        let mut m = supply::Mint { to: did(2), amount: 500, nonce: nonce(n), stamp: at_, sigs: vec![] };
        m.sigs = vec![(0, key(by).sign(&m.message(&mid)).to_bytes())];
        m
    };
    // B's chain credits a mint by key 60 at stamp 3.
    let mut b = State::default();
    minted(&mut b, 1, 500, 3);
    let chain = at(mid, 2);
    let honest = supply::Shard { mints: vec![signed_mint(1, 3, 60)] };
    // The mint set later moved 60 → 61 at stamp 5, sealing epoch 0 as it held nothing then.
    let moved = |closed: Vec<(u64, [u8; 32])>| {
        let mut c = authority::Change { seq: 0, role: authority::Role::Mint, next: Some(authority::KeySet { k: 1, keys: vec![pk(61)] }), stamp: 5, closed, cap: 0, sigs: vec![] };
        c.sigs = vec![(0, key(60).sign(&c.message(&mid)).to_bytes())];
        authority::Chain { changes: vec![c] }
    };
    let run = |shard: Option<supply::Shard>, auth: Option<authority::Chain>| {
        let chains = |p: &ChainParams| (p.definition == mid && p.owner == did(2)).then(|| b.clone());
        let defs = |t: &[u8; 32]| (*t == mid).then(|| md.clone());
        let sh = |_: &supply::Params| shard.clone();
        let au = |_: &authority::Params| auth.clone();
        let src = Source { chain: &chains, definition: &defs, block: &|_| None, keys: &keys_of, supply: &sh, authority: &au };
        fraud(Wallet::new([0; 32]).verify(&src, &chain, b.entries.len() as u64 - 1)).map_err(|f| f.why)
    };
    assert_eq!(run(Some(honest.clone()), Some(authority::Chain::default())), Ok(()));
    assert_eq!(run(Some(honest.clone()), None), Err("the mint's supply shard is not there"), "the authority chain not held: not counted");
    assert_eq!(run(None, Some(authority::Chain::default())), Err("the mint's supply shard is not there"));
    // Key 60, replaced, back-dates the mint into the epoch its own change sealed (empty): refused.
    assert_eq!(run(Some(honest.clone()), Some(moved(vec![]))), Err("the mint's supply shard is not there"));
    assert_eq!(run(Some(honest.clone()), Some(moved(vec![(0, supply::seal(&honest, 5).unwrap())]))), Ok(()), "sealed as it holds");
}
