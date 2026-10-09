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
/// Before person `by` first signs a chain: its WRITER entry (device `by`, on key seq 0).
fn ensure_writer(s: &mut State, by: u8) {
    if s.fork.is_none() && writer_at(s, s.entries.len()).is_none() {
        let e = sign(entry(s, [0xee; 16], Body::Writer { cred: cred(by), key_seq: 0 }), by);
        s.entries.push(e);
    }
}
fn def() -> Definition {
    Definition { code_hash: CODE, decimals: 8, min_send: 10, genesis_owner: did(1), genesis_amount: 1000, name: b"T".to_vec(), nonce: [0; 16], beacon: BEACON, margin: 2, idlog: IDLOG, mint: None, freeze: None, ids: Ids::None, royalty: None }
}
fn params(owner: u8) -> ChainParams {
    ChainParams { code_hash: CODE, definition: id(&CODE, &def().encode()), owner: did(owner), beacon: BEACON, idlog: IDLOG, item: [0; 32] }
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
        Body::Receive { amount, .. } | Body::Mint { amount, .. } | Body::Vested { amount, .. } => b + amount,
        Body::Vest { schedule, .. } => b.wrapping_sub(schedule.iter().map(|x| x.1).sum::<u128>()),
        Body::Stake { amount } => b.wrapping_sub(*amount),
        Body::Unstake { .. } | Body::Slash { .. } => b,
        Body::Release { unstake } => b + unbonding_of(s).into_iter().find(|u| &u.0 == unstake).map_or(0, |u| u.1),
        Body::Allow { .. } | Body::Offer { .. } | Body::Cut { .. } | Body::Writer { .. } | Body::Members { .. } => b,
        Body::Spend { amount, .. } | Body::Fill { amount, .. } => b.wrapping_sub(*amount),
        Body::Lock { amount, fee, .. } => b.wrapping_sub(amount + fee.map_or(0, |f| f.1)),
        Body::Claim { amount, .. } => b + amount,
        Body::Refund { lock, .. } => {
            b + s.entries.iter().find_map(|x| match &x.body {
                Body::Lock { amount, fee, .. } if &x.nonce == lock => Some(amount + fee.map_or(0, |f| f.1)),
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
        check(&params(owner), &def(), s, &|d, o| if *d == tok.into() { self.0.get(o).cloned() } else { None }, &|_| None, Some(&log(owner)))
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
    ChainParams { code_hash: CODE, definition: token, owner: did(owner), beacon: BEACON, idlog: IDLOG, item: [0; 32] }
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
        check(&at(token, owner), &d, self.get(token, owner), &|t, o| self.0.get(&(t.definition, *o)).cloned(), &|_| None, Some(&log(owner)))
    }
    /// As a holder takes in the last `new` entries: their citations must be SHOWN (live, or by their evidence).
    fn strict(&self, token: [u8; 32], owner: u8, new: usize) -> Result<(), Why> {
        let d = if token == id_a() { def() } else { def_b() };
        let s = self.get(token, owner);
        walk(&at(token, owner), &d, s, &|t, o| self.0.get(&(t.definition, *o)).cloned(), &|_| None, Some(&log(owner)), new, s.entries.len() - new).map(|_| ()).map_err(|x| x.1)
    }
}

const M: u8 = 1; // the maker: holds A, offers 2 A for every 1 B
const T: u8 = 2; // the taker: holds B

/// M offers; T locks 100 B for it; the fill gives T 200 A; T receives it; M claims the 100 B.
fn swapped() -> Two {
    let mut w = Two(HashMap::new());
    put_at(w.chain(id_a(), M), 1, Body::Genesis { amount: 1000 }, Some(M), 0);
    put_at(w.chain(id_a(), M), 10, Body::Offer { get: id_b().into(), give: 2, per: 1, max: 400, until: 1000 }, Some(M), 0);
    put_at(w.chain(id_b(), T), 1, Body::Genesis { amount: 1000 }, Some(T), 0);
    put_at(w.chain(id_b(), T), 20, Body::Lock { to: did(M), amount: 100, until: 50, token: id_a().into(), offer: nonce(10), fee: None }, Some(T), 0);
    put_at(w.chain(id_a(), M), 30, Body::Fill { offer: nonce(10), taker: did(T), lock: nonce(20), amount: 200, cited: 0, until: 50, ev: Evidence::default() }, None, 0);
    put_at(w.chain(id_a(), T), 30, Body::Receive { from: did(M), nonce: nonce(30), amount: 200, cited: 0, ev: Evidence::default() }, None, 0);
    put_at(w.chain(id_b(), M), 40, Body::Claim { taker: did(T), lock: nonce(20), amount: 100, cited: 0, token: id_a().into(), maker: did(M), ev: Evidence::default() }, None, 0);
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
    let w = swapped_with_lock(Body::Lock { to: did(M), amount: 99, until: 50, token: id_a().into(), offer: nonce(10), fee: None });
    assert_eq!(w.check(id_a(), M), Err("a fill its lock does not pay for"));
    // A lock for another maker, another token, another offer, another deadline.
    for lock in [
        Body::Lock { to: did(9), amount: 100, until: 50, token: id_a().into(), offer: nonce(10), fee: None },
        Body::Lock { to: did(M), amount: 100, until: 50, token: [5; 32].into(), offer: nonce(10), fee: None },
        Body::Lock { to: did(M), amount: 100, until: 50, token: id_a().into(), offer: nonce(11), fee: None },
        Body::Lock { to: did(M), amount: 100, until: 51, token: id_a().into(), offer: nonce(10), fee: None },
    ] {
        let w = swapped_with_lock(lock);
        assert_eq!(w.check(id_a(), M), Err("a fill its lock does not pay for"));
    }
}

#[test]
fn a_fill_is_bounded_by_its_offer_and_fills_a_lock_once() {
    let mut w = swapped();
    // Another lock of T's for 150 B → 300 A; only 200 are left of the offer.
    put_at(w.chain(id_b(), T), 21, Body::Lock { to: did(M), amount: 150, until: 50, token: id_a().into(), offer: nonce(10), fee: None }, Some(T), 0);
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
    put_at(w.chain(id_b(), M), 41, Body::Claim { taker: did(T), lock: nonce(20), amount: 100, cited: 0, token: id_a().into(), maker: did(M), ev: Evidence::default() }, None, 0);
    assert_eq!(w.check(id_b(), M), Err("a lock claimed twice"));
    // A claim of more than the lock held.
    let mut w = swapped();
    let mb = w.chain(id_b(), M);
    mb.entries.clear();
    put_at(mb, 40, Body::Claim { taker: did(T), lock: nonce(20), amount: 101, cited: 0, token: id_a().into(), maker: did(M), ev: Evidence::default() }, None, 0);
    assert_eq!(w.check(id_b(), M), Err("a claim of a lock that is not filled"));
}

#[test]
fn an_unfilled_lock_is_refunded_once_and_a_filled_one_never() {
    // A second lock nobody fills: refunded.
    let mut w = swapped();
    put_at(w.chain(id_b(), T), 21, Body::Lock { to: did(M), amount: 50, until: 50, token: id_a().into(), offer: nonce(10), fee: None }, Some(T), 0);
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
    put_at(tb, 20, Body::Lock { to: did(M), amount: 100, until: 50, token: id_a().into(), offer: nonce(10), fee: None }, Some(T), 0);
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
    put_at(&mut locked, 3, Body::Lock { to: did(9), amount: 50, until: 20, token: [5; 32].into(), offer: nonce(1), fee: None }, Some(M), 0);
    let refund = |stamp| {
        let mut m = locked.clone();
        put_at(&mut m, 4, Body::Refund { lock: nonce(3), ev: Evidence::default() }, None, stamp);
        m
    };
    assert_eq!(append_check(&def(), 19, &locked, &refund(19), Some(&log(M))), Err("a refund before its lock's deadline"));
    assert_eq!(append_check(&def(), 20, &locked, &refund(20), Some(&log(M))), Ok(()));
}

// ---- wallet fixtures ----

/// Token A: 1 (genesis) → 300 to 2 → 100 to 3.
fn three_hops() -> Two {
    let mut w = Two(HashMap::new());
    put_at(w.chain(id_a(), 1), 1, Body::Genesis { amount: 1000 }, Some(1), 0);
    put_at(w.chain(id_a(), 1), 2, Body::Send { to: did(2), amount: 300 }, Some(1), 0);
    put_at(w.chain(id_a(), 2), 2, Body::Receive { from: did(1), nonce: nonce(2), amount: 300, cited: 0, ev: Evidence::default() }, None, 2);
    put_at(w.chain(id_a(), 2), 3, Body::Send { to: did(3), amount: 100 }, Some(2), 2);
    put_at(w.chain(id_a(), 3), 3, Body::Receive { from: did(2), nonce: nonce(3), amount: 100, cited: 2, ev: Evidence::default() }, None, 4);
    w
}

/// 2 credits itself 5000 from a send that does not exist, buried under RECENT real receives (1 sends them), then pays
/// 3 out of it.
fn buried() -> Two {
    let mut w = Two(HashMap::new());
    put_at(w.chain(id_a(), 1), 1, Body::Genesis { amount: 1000 }, Some(1), 0);
    put_at(w.chain(id_a(), 2), 9, Body::Receive { from: did(1), nonce: nonce(9), amount: 5000, cited: 0, ev: Evidence::default() }, None, 2);
    for k in 0..RECENT as u8 {
        put_at(w.chain(id_a(), 1), 20 + k, Body::Send { to: did(2), amount: 10 }, Some(1), 0);
        put_at(w.chain(id_a(), 2), 20 + k, Body::Receive { from: did(1), nonce: nonce(20 + k), amount: 10, cited: 0, ev: Evidence::default() }, None, 2);
    }
    put_at(w.chain(id_a(), 2), 40, Body::Send { to: did(3), amount: 3000 }, Some(2), 2);
    put_at(w.chain(id_a(), 3), 40, Body::Receive { from: did(2), nonce: nonce(40), amount: 3000, cited: 2, ev: Evidence::default() }, None, 4);
    w
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
    assert_eq!(lie(&|b| if let Body::Cut { offers, .. } = b { offers.push((nonce(9), id_b().into(), 1, 1, 1, 9)) }), Err("a cut that is not what came before it"));
    assert_eq!(lie(&|b| if let Body::Cut { locks, .. } = b { locks.push((nonce(9), did(2), 10, 9, id_b().into(), nonce(9), true)) }), Err("a cut that is not what came before it"));
    // A cut naming another device as the chain's writer (handing it the chain): not what came before it.
    assert_eq!(lie(&|b| if let Body::Cut { writer, .. } = b { *writer = Some(crate::Signer::Device(pk(9), 0)) }), Err("a cut that is not what came before it"));
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
    put_at(w.chain(id_b(), T), 21, Body::Lock { to: did(M), amount: 50, until: 50, token: id_a().into(), offer: nonce(10), fee: None }, Some(T), 0);
    let wholem = w.get(id_a(), M).clone();
    let mut blocks = cut::Blocks::new();
    cut_at(&mut w, &wholem, id_a(), M, 5, &mut blocks);
    let livem = w.get(id_a(), M).clone();
    assert_eq!(w.check(id_a(), M), Ok(()));
    let Body::Cut { offers, .. } = &livem.entries[0].body else { panic!() };
    assert_eq!(offers, &vec![(nonce(10), id_b().into(), 2, 1, 200, 1000)], "the open offer carried, what is left of it, its time");
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






#[test]
fn an_entry_over_1kb_behind_a_cut_is_proven_by_its_value_block() {
    // M fills T's second lock (21: 50 B → 100 A) carrying 2 KB of evidence, then CUTS: the fill is behind the cut, its
    // bytes over 1 KB — stored as a value block, proven by id and length.
    let mut w = swapped();
    put_at(w.chain(id_b(), T), 21, Body::Lock { to: did(M), amount: 50, until: 50, token: id_a().into(), offer: nonce(10), fee: None }, Some(T), 0);
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
    put_at(w.chain(id_b(), T), 21, Body::Lock { to: did(M), amount: 50, until: 50, token: id_a().into(), offer: nonce(10), fee: None }, Some(T), 0);
    let mut blocks = cut::Blocks::new();
    let (wm, wt) = (w.get(id_a(), M).clone(), w.get(id_b(), T).clone());
    cut_at(&mut w, &wm, id_a(), M, 5, &mut blocks);
    cut_at(&mut w, &wt, id_b(), T, 5, &mut blocks);
    // A FILL of the carried offer names the taker's chain (what validation asks the node for) …
    let mut m = w.get(id_a(), M).clone();
    put_at(&mut m, 32, Body::Fill { offer: nonce(10), taker: did(T), lock: nonce(21), amount: 100, cited: 0, until: 50, ev: Evidence::default() }, None, 6);
    assert!(cited_chains(&at(id_a(), M), &m).contains(&(id_b().into(), did(T))), "the taker's chain of the offer's token");
    // … and a REFUND of the carried lock names the maker's chain, and its deadline is found for the append rules.
    let held = w.get(id_b(), T).clone();
    let mut t = held.clone();
    put_at(&mut t, 60, Body::Refund { lock: nonce(21), ev: Evidence::default() }, None, 50);
    assert!(cited_chains(&at(id_b(), T), &t).contains(&(id_a().into(), did(M))));
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
        put_at(w.chain(id_b(), T), 21, Body::Lock { to: did(M), amount: 50, until: 2000, token: id_a().into(), offer: nonce(10), fee: None }, Some(T), 0);
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
    put_at(w2.chain(id_b(), T), 21, Body::Lock { to: did(M), amount: 50, until: 6, token: id_a().into(), offer: nonce(10), fee: None }, Some(T), 0);
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
    let mut m = supply::Mint { to: did(to), item: [0; 32], amount, nonce: nonce(n), stamp: at, sigs: vec![] };
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



// ---- FOUNDATION 4: a MULTISIG owner (a k-of-n key set) ----

fn set_of(k: u8, seeds: &[u8]) -> authority::KeySet {
    authority::KeySet { k, keys: seeds.iter().map(|n| pk(*n)).collect() }
}
/// Entry `e` signed by the keys at indexes `by` of `seeds`.
fn msign(mut e: Entry, seeds: &[u8], by: &[u8]) -> Entry {
    let m = e.message();
    e.auth = Auth::Multi(by.iter().map(|i| (*i, key(seeds[*i as usize]).sign(&m).to_bytes())).collect());
    e
}
const TRIO: [u8; 3] = [70, 71, 72];
/// The set chain's params: owner = the 2-of-3 set's id.
fn set_params() -> ChainParams {
    ChainParams { owner: set_id(&set_of(2, &TRIO)), ..params(1) }
}
/// A (1): genesis, 300 to the set; the set's chain: its members, then the receive.
fn set_world() -> (State, State) {
    let mut a = State::default();
    signed(&mut a, 1, 1, Body::Genesis { amount: 1000 });
    signed(&mut a, 1, 2, Body::Send { to: set_params().owner, amount: 300 });
    let mut m = State::default();
    let e = msign(entry(&m, nonce(40), Body::Members { set: set_of(2, &TRIO), set_seq: 0 }), &TRIO, &[0, 2]);
    m.entries.push(e);
    let e = work(entry(&m, nonce(2), Body::Receive { from: did(1), nonce: nonce(2), amount: 300, cited: 0, ev: Evidence::default() }));
    m.entries.push(e);
    (a, m)
}

#[test]
fn a_key_set_owns_a_chain_and_k_of_it_sign_its_spends() {
    let (a, m) = set_world();
    let p = set_params();
    let ck = |s: &State| check(&p, &def(), s, &|_, o| (*o == did(1)).then(|| a.clone()), &|_| None, None);
    assert_eq!(ck(&m), Ok(()), "no key log asked for: the set names its own signers");
    assert_eq!(m.balance(), 300);
    let mut two = m.clone();
    let e = msign(entry(&two, nonce(3), Body::Send { to: did(3), amount: 100 }), &TRIO, &[1, 2]);
    two.entries.push(e);
    assert_eq!(ck(&two), Ok(()), "2 of 3");
    let mut one = m.clone();
    let e = msign(entry(&one, nonce(3), Body::Send { to: did(3), amount: 100 }), &TRIO, &[1]);
    one.entries.push(e);
    assert_eq!(ck(&one), Err("an entry not signed by the chain's writer"), "1 of 3");
    let mut device = m.clone();
    let e = sign(entry(&device, nonce(3), Body::Send { to: did(3), amount: 100 }), 70);
    device.entries.push(e);
    assert_eq!(ck(&device), Err("an entry not signed by the chain's writer"), "one member's plain signature");
    // The first members must be the set the owner id names, signed by k of it.
    let mut other = State::default();
    let e = msign(entry(&other, nonce(40), Body::Members { set: set_of(2, &[70, 71, 73]), set_seq: 0 }), &[70, 71, 73], &[0, 1]);
    other.entries.push(e);
    assert_eq!(ck(&other), Err("a key set that is not the chain's owner"));
    let mut lone = State::default();
    let e = msign(entry(&lone, nonce(40), Body::Members { set: set_of(2, &TRIO), set_seq: 0 }), &TRIO, &[0]);
    lone.entries.push(e);
    assert_eq!(ck(&lone), Err("an entry not signed by the chain's writer"), "the first members signed by 1 of 3");
    // A device writer on a set's chain; a set on a device's chain.
    let mut w = m.clone();
    let e = sign(entry(&w, [0xee; 16], Body::Writer { cred: cred(2), key_seq: 0 }), 2);
    w.entries.push(e);
    assert_eq!(ck(&w), Err("a writer credential for another account"));
    // A credential naming the SET as its account, under some key log handed in: still no device on a set's chain.
    let mut forced = m.clone();
    let c = craftworks_identity::credential(&p.owner, &pk(2), &[], &owner_seed(2));
    let e = sign(entry(&forced, [0xee; 16], Body::Writer { cred: c, key_seq: 0 }), 2);
    forced.entries.push(e);
    assert_eq!(check(&p, &def(), &forced, &|_, o| (*o == did(1)).then(|| a.clone()), &|_| None, Some(&log(2))), Err("a device writer on a key set's chain"));
    let mut b = paid().0[&did(2)].clone();
    let e = msign(entry(&b, nonce(41), Body::Members { set: set_of(2, &TRIO), set_seq: 0 }), &TRIO, &[0, 1]);
    let mut first = b.clone();
    first.entries.push(e);
    assert_eq!(check(&params(2), &def(), &first, &|_, o| paid().0.get(o).cloned(), &|_| None, Some(&log(2))), Err("a key set that is not the chain's owner"), "a DID's chain names no set");
    ensure_writer(&mut b, 2);
    let e = msign(entry(&b, nonce(41), Body::Members { set: set_of(2, &TRIO), set_seq: 1 }), &TRIO, &[0, 1]);
    b.entries.push(e);
    assert_eq!(check(&params(2), &def(), &b, &|_, o| paid().0.get(o).cloned(), &|_| None, Some(&log(2))), Err("a key set on a device's chain"));
}

#[test]
fn the_members_change_by_k_of_the_set_before_and_the_change_outranks_the_old_set() {
    let (a, m) = set_world();
    let p = set_params();
    let ck = |s: &State| check(&p, &def(), s, &|_, o| (*o == did(1)).then(|| a.clone()), &|_| None, None);
    const NEXT: [u8; 2] = [73, 74];
    let mut moved = m.clone();
    let e = msign(entry(&moved, nonce(41), Body::Members { set: set_of(2, &NEXT), set_seq: 1 }), &TRIO, &[0, 1]);
    moved.entries.push(e);
    assert_eq!(ck(&moved), Ok(()));
    let mut by_new = moved.clone();
    let e = msign(entry(&by_new, nonce(3), Body::Send { to: did(3), amount: 100 }), &NEXT, &[0, 1]);
    by_new.entries.push(e);
    assert_eq!(ck(&by_new), Ok(()), "the new set signs");
    const THIRD: [u8; 2] = [75, 76];
    let mut again = moved.clone();
    let e = msign(entry(&again, nonce(42), Body::Members { set: set_of(1, &THIRD), set_seq: 2 }), &NEXT, &[0, 1]);
    again.entries.push(e);
    let e = msign(entry(&again, nonce(3), Body::Send { to: did(3), amount: 100 }), &THIRD, &[1]);
    again.entries.push(e);
    assert_eq!(ck(&again), Ok(()), "a second change, then the third set signs");
    let mut by_old = moved.clone();
    let e = msign(entry(&by_old, nonce(3), Body::Send { to: did(3), amount: 100 }), &TRIO, &[0, 1]);
    by_old.entries.push(e);
    assert_eq!(ck(&by_old), Err("an entry not signed by the chain's writer"), "the old set no longer");
    let mut skip = m.clone();
    let e = msign(entry(&skip, nonce(41), Body::Members { set: set_of(2, &NEXT), set_seq: 2 }), &TRIO, &[0, 1]);
    skip.entries.push(e);
    assert_eq!(ck(&skip), Err("a key set change out of place"));
    let mut by_next = m.clone();
    let e = msign(entry(&by_next, nonce(41), Body::Members { set: set_of(2, &NEXT), set_seq: 1 }), &NEXT, &[0, 1]);
    by_next.entries.push(e);
    assert_eq!(ck(&by_next), Err("an entry not signed by the chain's writer"), "a change signed by the incoming set alone");
    // At one seq: the change (set seq 1) outranks the old set's send (set seq 0); two sends by the set freeze it.
    let mut old_send = m.clone();
    let e = msign(entry(&old_send, nonce(5), Body::Send { to: did(4), amount: 200 }), &TRIO, &[1, 2]);
    old_send.entries.push(e);
    assert_eq!(merge(&old_send, &moved), moved);
    assert_eq!(merge(&moved, &old_send), moved);
    let mut other_send = m.clone();
    let e = msign(entry(&other_send, nonce(6), Body::Send { to: did(5), amount: 200 }), &TRIO, &[0, 2]);
    other_send.entries.push(e);
    let f = merge(&old_send, &other_send);
    assert!(f.fork.is_some(), "an equivocation by the set");
    assert_eq!(ck(&f), Ok(()), "the frozen chain still holds its shape");
    // An update needs no key log for a set's chain.
    let mut fresh = by_new.clone();
    for e in fresh.entries.iter_mut() {
        e.stamp = 5;
    }
    let mut resigned = State::default();
    for (i, e) in fresh.entries.iter().enumerate() {
        let mut x = entry(&resigned, e.nonce, e.body.clone());
        x.stamp = 5;
        let x = match i {
            0 => msign(x, &TRIO, &[0, 1]),
            1 => work(x),
            2 => msign(x, &TRIO, &[0, 1]),
            _ => msign(x, &NEXT, &[0, 1]),
        };
        resigned.entries.push(x);
    }
    assert_eq!(append_check(&def(), 6, &State::default(), &resigned, None), Ok(()));
}

#[test]
fn a_set_chain_cuts_and_carries_its_set() {
    let (a, m) = set_world();
    let p = set_params();
    let mut blocks = cut::Blocks::new();
    let (body, b) = cut::make(&p, &def(), &m, None).unwrap();
    blocks.extend(b);
    match &body {
        Body::Cut { writer, .. } => assert_eq!(writer, &Some(crate::Signer::Set(set_of(2, &TRIO), 0))),
        _ => panic!("not a cut"),
    }
    let mut s = m.clone();
    let e = msign(entry(&s, nonce(99), body), &TRIO, &[0, 1]);
    s.entries.push(e);
    let pruned = prune(&s);
    let ck = |s: &State| check(&p, &def(), s, &|_, o| (*o == did(1)).then(|| a.clone()), &|_| None, None);
    assert_eq!(ck(&pruned), Ok(()));
    let mut after = pruned.clone();
    let e = msign(entry(&after, nonce(3), Body::Send { to: did(3), amount: 100 }), &TRIO, &[0, 2]);
    after.entries.push(e);
    assert_eq!(ck(&after), Ok(()), "the cut's set signs on");
    assert!(is_set_chain(&after) && !is_set_chain(&paid().0[&did(2)]));
}

// ---- FOUNDATION 5: ids (unique / many) and royalties ----

/// U: a UNIQUE token (NFTs) minted by key 60, a 5% royalty to its creator (person 7); one of an item is a send.
fn def_u() -> Definition {
    Definition { ids: Ids::Unique, mint: Some(mint_set()), royalty: Some((500, did(7))), min_send: 1, name: b"U".to_vec(), nonce: [2; 16], ..def() }
}
fn id_u() -> [u8; 32] {
    id(&CODE, &def_u().encode())
}
/// The item minted at epoch 0 under nonce 1.
fn item_u() -> Tok {
    Tok { definition: id_u(), item: unique_item(&id_u(), 0, &nonce(1)) }
}
fn at_t(t: Tok, owner: u8) -> ChainParams {
    ChainParams { item: t.item, ..at(t.definition, owner) }
}
struct Ids5(HashMap<(Tok, [u8; 32]), State>);
impl Ids5 {
    fn chain(&mut self, t: Tok, owner: u8) -> &mut State {
        self.0.entry((t, did(owner))).or_default()
    }
    fn check(&self, t: Tok, owner: u8) -> Result<(), Why> {
        let d = if t.definition == id_u() { def_u() } else if t.definition == id_a() { def() } else { def_b() };
        let shard = supply::Shard { mints: vec![u_mint(M, 1)] };
        check(&at_t(t, owner), &d, &self.0[&(t, did(owner))], &|c, o| self.0.get(&(*c, *o)).cloned(), &|e| (e == 0).then(|| shard.clone()), Some(&log(owner)))
    }
}
/// The mint of U's item (nonce `n`, epoch 0) to `to`.
fn u_mint(to: u8, n: u8) -> supply::Mint {
    let mut m = supply::Mint { to: did(to), item: unique_item(&id_u(), 0, &nonce(n)), amount: 1, nonce: nonce(n), stamp: 0, sigs: vec![] };
    m.sigs = vec![(0, key(60).sign(&m.message(&id_u())).to_bytes())];
    m
}
/// M holds U's item (minted) and offers it for 100 B; T locks 100 B (+ `fee`) for it; the fill; T receives the item.
fn nft_sale(fee: Option<([u8; 32], u128)>) -> Ids5 {
    let mut w = Ids5(HashMap::new());
    let it = item_u();
    put_at(w.chain(it, M), 1, Body::Mint { nonce: nonce(1), amount: 1, minted: 0, ev: Evidence::default() }, None, 0);
    put_at(w.chain(it, M), 10, Body::Offer { get: id_b().into(), give: 1, per: 100, max: 1, until: 1000 }, Some(M), 0);
    put_at(w.chain(id_b().into(), T), 1, Body::Genesis { amount: 1000 }, Some(T), 0);
    put_at(w.chain(id_b().into(), T), 20, Body::Lock { to: did(M), amount: 100, until: 50, token: it, offer: nonce(10), fee }, Some(T), 0);
    put_at(w.chain(it, M), 30, Body::Fill { offer: nonce(10), taker: did(T), lock: nonce(20), amount: 1, cited: 0, until: 50, ev: Evidence::default() }, None, 0);
    put_at(w.chain(it, T), 30, Body::Receive { from: did(M), nonce: nonce(30), amount: 1, cited: 0, ev: Evidence::default() }, None, 0);
    w
}

#[test]
fn an_item_is_its_own_chain_of_its_tokens_kind() {
    let mut w = nft_sale(Some((did(7), 5)));
    let it = item_u();
    assert_eq!(w.check(it, M), Ok(()), "minted, offered, filled");
    assert_eq!(w.check(it, T), Ok(()), "the item received: T owns it");
    assert_eq!(w.0[&(it, did(T))].balance(), 1);
    // A fungible token holds no item; a token of ids holds nothing but items; no genesis for a token of ids.
    let mut a = State::default();
    signed(&mut a, 1, 1, Body::Genesis { amount: 1000 });
    assert_eq!(check(&at_t(Tok { definition: id_a(), item: [1; 32] }, 1), &def(), &a, &|_, _| None, &|_| None, Some(&log(1))), Err("an item that is not its token's kind"));
    assert_eq!(check(&at(id_u(), 1), &def_u(), &a, &|_, _| None, &|_| None, Some(&log(1))), Err("an item that is not its token's kind"));
    let g = Definition { genesis_owner: did(M), ..def_u() };
    assert_eq!(check(&at_t(it, M), &g, &a, &|_, _| None, &|_| None, Some(&log(1))), Err("a genesis that is not the definition's"));
    // A mint credited to another item's chain is not its mint.
    let other = Tok { item: [9; 32], ..it };
    let mut x = State::default();
    put_at(&mut x, 1, Body::Mint { nonce: nonce(1), amount: 1, minted: 0, ev: Evidence::default() }, None, 0);
    *w.chain(other, M) = x;
    assert_eq!(w.check(other, M), Err("a mint that is not in its supply shard"));
}

#[test]
fn a_unique_item_is_minted_once_as_one() {
    let sp = supply::Params { code_hash: CODE, definition: id_u(), beacon: BEACON, epoch: 0 };
    let first = (Some(mint_set()), None);
    let holds = |m: supply::Mint| supply::holds(&sp, (&first.0, &first.1), Ids::Unique, &authority::Chain::default(), &supply::Shard { mints: vec![m] });
    assert_eq!(holds(u_mint(M, 1)), Ok(()));
    let resign = |mut m: supply::Mint| {
        m.sigs = vec![(0, key(60).sign(&m.message(&id_u())).to_bytes())];
        m
    };
    assert_eq!(holds(resign(supply::Mint { amount: 2, ..u_mint(M, 1) })), Err("a mint of an item not its token's kind"), "two of a unique item");
    assert_eq!(holds(resign(supply::Mint { item: [5; 32], ..u_mint(M, 1) })), Err("a mint of an item not its token's kind"), "an item it did not make");
    assert_eq!(supply::holds(&sp, (&first.0, &first.1), Ids::None, &authority::Chain::default(), &supply::Shard { mints: vec![u_mint(M, 1)] }), Err("a mint of an item not its token's kind"), "an item of a fungible token");
    assert_eq!(supply::holds(&sp, (&first.0, &first.1), Ids::Many, &authority::Chain::default(), &supply::Shard { mints: vec![resign(supply::Mint { item: [5; 32], amount: 30, ..u_mint(M, 1) })] }), Ok(()), "many: any item, any amount");
    assert_eq!(supply::holds(&sp, (&first.0, &first.1), Ids::Many, &authority::Chain::default(), &supply::Shard { mints: vec![resign(supply::Mint { item: [0; 32], amount: 30, ..u_mint(M, 1) })] }), Err("a mint of an item not its token's kind"), "many: never no item");
    assert_ne!(unique_item(&id_u(), 0, &nonce(1)), unique_item(&id_u(), 1, &nonce(1)), "another epoch, another item");
}

#[test]
fn a_sale_of_a_royalty_token_pays_its_creator_who_claims_it() {
    let it = item_u();
    assert_eq!(nft_sale(Some((did(7), 5))).check(it, M), Ok(()), "5% of 100");
    for (fee, why) in [(None, "no fee"), (Some((did(7), 4)), "under 5%"), (Some((did(8), 5)), "to another than the creator")] {
        assert_eq!(nft_sale(fee).check(it, M), Err("a fill its lock does not pay for"), "{why}");
    }
    // M claims the 100; the creator claims its 5 — each from T's lock, the fill in M's chain.
    let mut w = nft_sale(Some((did(7), 5)));
    put_at(w.chain(id_b().into(), M), 40, Body::Claim { taker: did(T), lock: nonce(20), amount: 100, cited: 0, token: it, maker: did(M), ev: Evidence::default() }, None, 0);
    put_at(w.chain(id_b().into(), 7), 41, Body::Claim { taker: did(T), lock: nonce(20), amount: 5, cited: 0, token: it, maker: did(M), ev: Evidence::default() }, None, 0);
    assert_eq!(w.check(id_b().into(), M), Ok(()));
    assert_eq!(w.check(id_b().into(), 7), Ok(()));
    assert_eq!(w.check(id_b().into(), T), Ok(()));
    assert_eq!(w.0[&(id_b().into(), did(T))].balance(), 895, "100 + 5 locked");
    let mut greedy = State::default();
    put_at(&mut greedy, 41, Body::Claim { taker: did(T), lock: nonce(20), amount: 100, cited: 0, token: it, maker: did(M), ev: Evidence::default() }, None, 0);
    *w.chain(id_b().into(), 7) = greedy;
    assert_eq!(w.check(id_b().into(), 7), Err("a claim of a lock that is not filled"), "the creator claims the maker's share");
    // A lock's fee to its maker or itself; the maker who IS the creator needs none.
    let mut bad = State::default();
    put_at(&mut bad, 1, Body::Genesis { amount: 1000 }, Some(T), 0);
    put_at(&mut bad, 20, Body::Lock { to: did(M), amount: 100, until: 50, token: it, offer: nonce(10), fee: Some((did(M), 5)) }, Some(T), 0);
    assert_eq!(check(&at(id_b(), T), &def_b(), &bad, &|_, _| None, &|_| None, Some(&log(T))), Err("a lock's fee to no one else"));
    // The share exactly: 1 bp of 10,001 is owed as 1.0001 — 1 is short, 2 pays.
    let tiny = Definition { royalty: Some((1, did(7))), ..def_u() };
    assert!(!royalty_paid(&tiny, &did(M), 10_001, Some((did(7), 1))) && royalty_paid(&tiny, &did(M), 10_001, Some((did(7), 2))));
    assert!(royalty_paid(&tiny, &did(M), 10_000, Some((did(7), 1))), "exactly its share");
    // A lock with a fee, never filled, refunds the whole of it.
    let mut back = State::default();
    put_at(&mut back, 1, Body::Genesis { amount: 1000 }, Some(T), 0);
    put_at(&mut back, 20, Body::Lock { to: did(M), amount: 100, until: 5, token: it, offer: nonce(10), fee: Some((did(7), 5)) }, Some(T), 0);
    put_at(&mut back, 21, Body::Refund { lock: nonce(20), ev: Evidence::default() }, None, 6);
    assert_eq!(back.balance(), 1000);
    assert_eq!(check(&at(id_b(), T), &def_b(), &back, &|_, _| None, &|_| None, Some(&log(T))), Ok(()), "100 + 5 back");
    let own = Definition { royalty: Some((500, did(M))), ..def_u() };
    assert!(royalty_paid(&own, &did(M), 100, None) && !royalty_paid(&def_u(), &did(M), 100, None));
}


// ---- FOUNDATION 6a: STAKE, its release after a delay, and its burn on a witness's equivocation ----

/// A (1) with its genesis (1000), its writer device 1.
fn staker() -> State {
    let mut a = State::default();
    signed(&mut a, 1, 1, Body::Genesis { amount: 1000 });
    a
}
fn wsig(by: u8, chain: u8, seq: u64, entry: u8) -> witness::WitnessSig {
    let (c, h) = ([chain; 32], [entry; 32]);
    witness::WitnessSig { chain: c, seq, entry: h, sig: key(by).sign(&witness::WitnessSig::message(&c, seq, &h)).to_bytes() }
}
fn at_stamp(s: &mut State, n: u8, body: Body, by: Option<u8>, stamp: u64) {
    let mut e = entry(s, nonce(n), body);
    e.stamp = stamp;
    s.entries.push(match by {
        Some(k) => sign(e, k),
        None => work(e),
    });
}

#[test]
fn stake_is_locked_out_of_the_balance_and_comes_back_only_after_its_delay() {
    let ck = |s: &State| check(&params(1), &def(), s, &|_, _| None, &|_| None, Some(&log(1)));
    let mut a = staker();
    signed(&mut a, 1, 2, Body::Stake { amount: 600 });
    assert_eq!(ck(&a), Ok(()));
    assert_eq!(a.balance(), 400, "staked is not spendable");
    let mut over = a.clone();
    signed(&mut over, 1, 3, Body::Send { to: did(2), amount: 401 });
    assert_eq!(ck(&over), Err("a debit over the balance"));
    let mut zero = staker();
    signed(&mut zero, 1, 2, Body::Stake { amount: 0 });
    assert_eq!(ck(&zero), Err("a stake of nothing"));
    let mut too_much = staker();
    signed(&mut too_much, 1, 2, Body::Stake { amount: 1001 });
    assert_eq!(ck(&too_much), Err("a debit over the balance"));
    // Unstake 200 at stamp 10: released at 10 + UNBOND, not a step before.
    at_stamp(&mut a, 3, Body::Unstake { amount: 200 }, Some(1), 10);
    assert_eq!(ck(&a), Ok(()));
    let mut early = a.clone();
    at_stamp(&mut early, 4, Body::Release { unstake: nonce(3) }, None, 10 + UNBOND - 1);
    assert_eq!(ck(&early), Err("a release before its delay"));
    let mut back = a.clone();
    at_stamp(&mut back, 4, Body::Release { unstake: nonce(3) }, None, 10 + UNBOND);
    assert_eq!(ck(&back), Ok(()));
    assert_eq!(back.balance(), 600, "400 + the 200 released");
    let mut twice = back.clone();
    at_stamp(&mut twice, 5, Body::Release { unstake: nonce(3) }, None, 10 + UNBOND);
    assert_eq!(ck(&twice), Err("a release of nothing unbonding"));
    let mut over_stake = a.clone();
    at_stamp(&mut over_stake, 6, Body::Unstake { amount: 401 }, Some(1), 11);
    assert_eq!(ck(&over_stake), Err("an unstake over the stake"), "400 left staked");
    // The append rule: the BEACON past the release height, not the entry's stamp.
    let held = a.clone();
    assert_eq!(append_check(&def(), 10 + UNBOND - 1, &held, &early, Some(&log(1))).err(), Some("a release before its delay"), "stamped now, the beacon one short");
    assert_eq!(append_check(&def(), 10 + UNBOND, &held, &back, Some(&log(1))), Ok(()));
}

#[test]
fn a_witness_equivocation_burns_its_stake_and_its_unbonding() {
    let ck = |s: &State| check(&params(1), &def(), s, &|_, _| None, &|_| None, Some(&log(1)));
    let mut a = staker();
    signed(&mut a, 1, 2, Body::Stake { amount: 600 });
    at_stamp(&mut a, 3, Body::Unstake { amount: 100 }, Some(1), 0);
    // Device 1 (this chain's writer) signed two entries at one place of chain 9.
    let mut slashed = a.clone();
    at_stamp(&mut slashed, 4, Body::Slash { key: pk(1), a: wsig(1, 9, 5, 1), b: wsig(1, 9, 5, 2) }, None, 0);
    assert_eq!(ck(&slashed), Ok(()));
    let mut nothing_back = slashed.clone();
    at_stamp(&mut nothing_back, 5, Body::Release { unstake: nonce(3) }, None, UNBOND);
    assert_eq!(ck(&nothing_back), Err("a release of nothing unbonding"), "the unbonding burned too");
    let mut unstake = slashed.clone();
    at_stamp(&mut unstake, 5, Body::Unstake { amount: 1 }, Some(1), 0);
    assert_eq!(ck(&unstake), Err("an unstake over the stake"), "the stake burned");
    assert_eq!(slashed.balance(), 400, "the balance itself untouched");
    // No equivocation: the same entry twice; two places; another key's signatures; a key not this chain's.
    for (body, why) in [
        (Body::Slash { key: pk(1), a: wsig(1, 9, 5, 1), b: wsig(1, 9, 5, 1) }, "a slash with no equivocation"),
        (Body::Slash { key: pk(1), a: wsig(1, 9, 5, 1), b: wsig(1, 9, 6, 2) }, "a slash with no equivocation"),
        (Body::Slash { key: pk(1), a: wsig(2, 9, 5, 1), b: wsig(2, 9, 5, 2) }, "a slash with no equivocation"),
        (Body::Slash { key: pk(2), a: wsig(2, 9, 5, 1), b: wsig(2, 9, 5, 2) }, "a slash of a key that is not this chain's"),
    ] {
        let mut x = a.clone();
        at_stamp(&mut x, 4, body, None, 0);
        assert_eq!(ck(&x), Err(why));
    }
}

#[test]
fn a_cut_carries_the_stake_and_the_unbonding() {
    let mut a = staker();
    signed(&mut a, 1, 2, Body::Stake { amount: 600 });
    at_stamp(&mut a, 3, Body::Unstake { amount: 100 }, Some(1), 7);
    let mut blocks = cut::Blocks::new();
    let (body, b) = cut::make(&params(1), &def(), &a, Some(&log(1))).unwrap();
    blocks.extend(b);
    match &body {
        Body::Cut { staked, unbonding, .. } => {
            assert_eq!(*staked, 500);
            assert_eq!(unbonding, &vec![(nonce(3), 100, 7 + UNBOND)]);
        }
        _ => panic!("not a cut"),
    }
    let mut s = a.clone();
    at_stamp(&mut s, 99, body.clone(), Some(1), 7);
    let pruned = prune(&s);
    let ck = |s: &State| check(&params(1), &def(), s, &|_, _| None, &|_| None, Some(&log(1)));
    assert_eq!(ck(&pruned), Ok(()));
    let mut back = pruned.clone();
    at_stamp(&mut back, 4, Body::Release { unstake: nonce(3) }, None, 7 + UNBOND);
    assert_eq!(ck(&back), Ok(()), "released across the cut");
    // The stake itself across the cut: all 500 can be unstaked, not one more.
    let mut un = pruned.clone();
    at_stamp(&mut un, 5, Body::Unstake { amount: 500 }, Some(1), 8);
    assert_eq!(ck(&un), Ok(()));
    let mut over = pruned.clone();
    at_stamp(&mut over, 5, Body::Unstake { amount: 501 }, Some(1), 8);
    assert_eq!(ck(&over), Err("an unstake over the stake"));
    // A cut that drops the stake is not what came before it.
    let mut lie = s.clone();
    if let Some(Body::Cut { staked, .. }) = lie.entries.last_mut().map(|e| &mut e.body) {
        *staked = 0;
    }
    let last = lie.entries.pop().unwrap();
    let e = sign(Entry { auth: Auth::Work(0), ..last }, 1);
    lie.entries.push(e);
    assert_eq!(ck(&lie), Err("a cut that is not what came before it"));
}

// ---- FOUNDATION 6c: an entry accepted once enough of its committee witnessed it ----

#[test]
fn an_entry_counts_once_enough_of_its_committee_signed_it_with_real_stake() {
    use draw::StakeClaim;
    let def_id = params(1).definition;
    // Ten stakers (keys 60..70), stake 100 each, epoch 3.
    let claim = |n: u8, amount: u128| {
        let k = key(n);
        let kb = k.verifying_key().to_bytes();
        StakeClaim { owner: [n; 32], amount, epoch: 3, key: kb, sig: k.sign(&StakeClaim::message(&def_id, &[n; 32], amount, 3, &kb)).to_bytes() }
    };
    let claims: Vec<StakeClaim> = (60..70).map(|n| claim(n, 100)).collect();
    let (chain, seq, entry, out) = ([7u8; 32], 12u64, [1u8; 32], [9u8; 32]);
    let (k, need) = (5, 4);
    let members = draw::committee(&def_id, &out, &chain, seq, 3, &claims, k);
    let sig_by = |n: u8, e: [u8; 32]| witness::WitnessSig { chain, seq, entry: e, sig: key(n).sign(&witness::WitnessSig::message(&chain, seq, &e)).to_bytes() };
    let member_keys: Vec<u8> = members.iter().map(|m| claims[*m].owner[0]).collect();
    let outsiders: Vec<u8> = (60..70).filter(|n| !member_keys.contains(n)).collect();
    // Each staker's chain: 100 staked, its writer key the claim's.
    let real = |o: &[u8; 32]| Some((100u128, vec![pk(o[0])]));
    let acc = |sigs: &[witness::WitnessSig], stake: &dyn Fn(&[u8; 32]) -> Option<witness::Stake>| witness::accepted(&def_id, &chain, seq, &entry, &out, 3, &claims, sigs, k, need, stake);
    let four: Vec<_> = member_keys[..4].iter().map(|n| sig_by(*n, entry)).collect();
    assert_eq!(acc(&four, &real), Ok(4));
    assert_eq!(acc(&four[..3], &real), Err("an entry its committee has not witnessed"), "three of four needed");
    // Outsiders' signatures, the same member twice, signatures for another entry: count for nothing.
    let mut padded = four[..3].to_vec();
    padded.extend(outsiders.iter().map(|n| sig_by(*n, entry)));
    padded.push(sig_by(member_keys[0], entry));
    padded.push(sig_by(member_keys[3], [2; 32]));
    assert_eq!(acc(&padded, &real), Err("an entry its committee has not witnessed"));
    // A member whose chain holds less stake than it claimed: not counted.
    let short = |o: &[u8; 32]| Some((if o[0] == member_keys[0] { 99 } else { 100 }, vec![pk(o[0])]));
    assert_eq!(acc(&four, &short), Err("an entry its committee has not witnessed"));
    let unknown = |o: &[u8; 32]| (o[0] != member_keys[1]).then(|| (100, vec![pk(o[0])]));
    assert_eq!(acc(&four, &unknown), Err("an entry its committee has not witnessed"), "a chain not readable: not counted");
    // A member whose claim's key is not one its chain's stake burns for (another writer): not counted.
    let other_key = |o: &[u8; 32]| Some((100, vec![if o[0] == member_keys[2] { pk(1) } else { pk(o[0]) }]));
    assert_eq!(acc(&four, &other_key), Err("an entry its committee has not witnessed"));
}

// ---- FOUNDATION 6d: a wallet takes an entry its committee accepted; a witness signs only on an accepted past ----

/// The lot for the wallet tests: 20 stakers (keys 60..80), 100 each, in the pool `id_a()`, epoch 0; the beacon drawn up
/// to height `drawn`; the witness signatures as `sigs` holds them.
struct Lot {
    claims: Vec<draw::StakeClaim>,
    sigs: HashMap<([u8; 32], u64), Vec<witness::WitnessSig>>,
    drawn: u64,
    /// The beacon's height as the witnesses read it.
    now: u64,
}
impl Lot {
    fn new() -> Lot {
        let pool = id_a();
        let claims = (60..80u8)
            .map(|n| {
                let kb = pk(n);
                draw::StakeClaim { owner: [n; 32], amount: 100, epoch: 0, key: kb, sig: key(n).sign(&draw::StakeClaim::message(&pool, &[n; 32], 100, 0, &kb)).to_bytes() }
            })
            .collect();
        Lot { claims, sigs: HashMap::new(), drawn: 100, now: 100 }
    }
    fn out(h: u64) -> [u8; 32] {
        *blake3::hash(&h.to_le_bytes()).as_bytes()
    }
    fn src(&self) -> wallet::Source<'_> {
        wallet::Source {
            pool: id_a(),
            claims: Box::leak(Box::new(|e: u64| if e == 0 { self.claims.clone() } else { vec![] })),
            sigs: Box::leak(Box::new(|c: &[u8; 32], seq: u64| self.sigs.get(&(*c, seq)).cloned().unwrap_or_default())),
            out: Box::leak(Box::new(|h: u64| (h <= self.drawn).then(|| Lot::out(h)))),
            // Every staker's stake as held at epoch 0's start (height 0) — its claims' epoch.
            stake_of: &|o, h| (h == 0).then(|| (100, vec![pk(o[0])])),
        }
    }
    /// `n` of the entry's committee sign it (`entry`: what they sign).
    fn sign(&mut self, chain: &ChainParams, e: &Entry, n: usize, entry: [u8; 32]) {
        let (epoch, h) = wallet::lot_of(e.stamp);
        let id = chain.id();
        let members = draw::committee(&id_a(), &Lot::out(h), &id, e.seq, epoch, &self.claims, wallet::COMMITTEE);
        let v = self.sigs.entry((id, e.seq)).or_default();
        for m in members.into_iter().take(n) {
            let k = self.claims[m].owner[0];
            v.push(witness::WitnessSig { chain: id, seq: e.seq, entry, sig: key(k).sign(&witness::WitnessSig::message(&id, e.seq, &entry)).to_bytes() });
        }
    }
    /// The committee witnesses every entry of `owner`'s chain of token A, in order — each only if the rule lets it.
    fn witness(&mut self, w: &Two, owner: u8) -> Result<(), Why> {
        let p = at(id_a(), owner);
        let s = w.get(id_a(), owner).clone();
        for e in &s.entries {
            let h = self.may(w, owner, e.seq)?;
            self.sign(&p, e, wallet::NEED, h);
        }
        Ok(())
    }
    fn may(&self, w: &Two, owner: u8, seq: u64) -> Result<[u8; 32], Why> {
        let p = at(id_a(), owner);
        let holds = |s: &State| check(&p, &def(), s, &|t, o| w.0.get(&(t.definition, *o)).cloned(), &|_| None, Some(&log(owner)));
        wallet::may_witness(&self.src(), &p, &def(), w.get(id_a(), owner), seq, self.now, &holds, &|t, o| w.0.get(&(t.definition, *o)).cloned())
    }
    fn accept(&self, w: &Two, owner: u8, seq: u64) -> Result<usize, Why> {
        let e = w.get(id_a(), owner).entries.iter().find(|e| e.seq == seq).unwrap().clone();
        wallet::accept(&self.src(), &at(id_a(), owner), &e)
    }
}

#[test]
fn a_payment_is_value_once_enough_of_its_committee_signed_it() {
    let w = three_hops();
    let mut lot = Lot::new();
    lot.witness(&w, 1).unwrap();
    lot.witness(&w, 2).unwrap();
    let (p, e) = (at(id_a(), 3), w.get(id_a(), 3).entries[0].clone());
    assert_eq!(lot.accept(&w, 3, 0), Err("an entry its committee has not witnessed"), "nobody signed it yet");
    lot.sign(&p, &e, 10, e.hash());
    assert_eq!(lot.accept(&w, 3, 0), Err("an entry its committee has not witnessed"), "10 of 16: one short of 11");
    lot.sign(&p, &e, wallet::NEED, [9; 32]);
    assert_eq!(lot.accept(&w, 3, 0), Err("an entry its committee has not witnessed"), "signatures for another entry");
    lot.sigs.clear();
    lot.witness(&w, 1).unwrap();
    lot.witness(&w, 2).unwrap();
    lot.witness(&w, 3).unwrap();
    assert_eq!(lot.accept(&w, 3, 0), Ok(11));
    // Before the beacon output after its stamp exists, nobody knows its committee.
    lot.drawn = 0;
    assert_eq!(lot.accept(&w, 3, 0), Err("an entry whose lot is not drawn yet"));
}

#[test]
fn a_witness_signs_only_on_an_accepted_past() {
    let w = three_hops();
    let mut lot = Lot::new();
    // 2's receive cites 1's send: not witnessable until that send is accepted.
    assert_eq!(lot.may(&w, 2, 0), Err("an entry its committee has not witnessed"), "the cited send not accepted");
    lot.witness(&w, 1).unwrap();
    let h = lot.may(&w, 2, 0).unwrap();
    assert_eq!(h, w.get(id_a(), 2).entries[0].hash());
    // 2's next entry (its send to 3): not until the receive before it is accepted.
    assert_eq!(lot.may(&w, 2, 1), Err("an entry its committee has not witnessed"), "the predecessor not accepted");
    let e = w.get(id_a(), 2).entries[0].clone();
    lot.sign(&at(id_a(), 2), &e, wallet::NEED, h);
    assert!(lot.may(&w, 2, 1).is_ok());
}

#[test]
fn a_fake_receive_buried_under_real_ones_is_never_witnessed_nor_anything_after_it() {
    let w = buried();
    // The contract takes it (it checks only the last RECENT citations)…
    assert_eq!(w.check(id_a(), 2), Ok(()));
    let mut lot = Lot::new();
    lot.witness(&w, 1).unwrap();
    // …no witness signs it: checked as the last entry of its chain, the send it cites is in no chain.
    assert_eq!(lot.may(&w, 2, 0), Err("a receive whose send is not in its sender's chain"));
    assert_eq!(lot.witness(&w, 2), Err("a receive whose send is not in its sender's chain"));
    // So nothing after it is witnessable either: 3's receive of the 3000 never becomes value.
    let last = w.get(id_a(), 2).entries.last().unwrap().seq;
    assert_eq!(lot.may(&w, 2, last), Err("an entry its committee has not witnessed"));
    assert_eq!(lot.may(&w, 3, 0), Err("an entry its committee has not witnessed"));
}

#[test]
fn a_witness_refuses_a_forked_chain_and_what_it_does_not_hold() {
    let mut w = three_hops();
    let lot = Lot::new();
    assert_eq!(lot.may(&w, 1, 7), Err("an entry not held"));
    // A chain held from past its start (a cut) with no predecessor in hand.
    let mut tail = w.get(id_a(), 1).clone();
    tail.entries.remove(0);
    let p = at(id_a(), 1);
    assert_eq!(wallet::may_witness(&lot.src(), &p, &def(), &tail, 1, 100, &|_| Ok(()), &|_, _| None), Err("an entry whose predecessor is not held"));
    // Two entries signed at one seq: frozen — nobody witnesses past the fork.
    let base = State { entries: w.get(id_a(), 1).entries[..1].to_vec(), fork: None };
    let (mut f1, mut f2) = (base.clone(), base.clone());
    put_at(&mut f1, 50, Body::Burn { amount: 1 }, Some(1), 0);
    put_at(&mut f2, 51, Body::Burn { amount: 2 }, Some(1), 0);
    *w.chain(id_a(), 1) = merge(&f1, &f2);
    assert!(w.get(id_a(), 1).fork.is_some());
    assert_eq!(lot.may(&w, 1, 0), Err("a forked chain"));
}

#[test]
fn the_shared_lot_records_round_trip_exactly() {
    let lot = Lot::new();
    let c = &lot.claims[3];
    let b = wallet::bytes::stake_claim(c);
    assert_eq!(wallet::bytes::read_stake_claim(&b).as_ref(), Some(c));
    assert!(wallet::bytes::read_stake_claim(&[&b[..], &[0]].concat()).is_none());
    assert!(wallet::bytes::read_stake_claim(&b[..b.len() - 1]).is_none());
    let s = wsig(60, 7, 3, 1);
    let b = wallet::bytes::witness_sig(&s);
    assert_eq!(wallet::bytes::read_witness_sig(&b).as_ref(), Some(&s));
    assert!(wallet::bytes::read_witness_sig(&[&b[..], &[0]].concat()).is_none());
}

#[test]
fn a_buried_receive_of_a_real_send_to_someone_else_is_never_witnessed() {
    // 1 sends 10 to 4 (accepted). 2 credits ITSELF 5000 citing that very send, buried under RECENT real receives.
    let mut w = Two(HashMap::new());
    put_at(w.chain(id_a(), 1), 1, Body::Genesis { amount: 1000 }, Some(1), 0);
    put_at(w.chain(id_a(), 1), 8, Body::Send { to: did(4), amount: 10 }, Some(1), 0);
    put_at(w.chain(id_a(), 2), 9, Body::Receive { from: did(1), nonce: nonce(8), amount: 5000, cited: 0, ev: Evidence::default() }, None, 2);
    for k in 0..RECENT as u8 {
        put_at(w.chain(id_a(), 1), 20 + k, Body::Send { to: did(2), amount: 10 }, Some(1), 0);
        put_at(w.chain(id_a(), 2), 20 + k, Body::Receive { from: did(1), nonce: nonce(20 + k), amount: 10, cited: 0, ev: Evidence::default() }, None, 2);
    }
    assert_eq!(w.check(id_a(), 2), Ok(()), "the contract, checking only the last RECENT citations, takes it");
    let mut lot = Lot::new();
    lot.witness(&w, 1).unwrap();
    // The send it names exists and is accepted — but it is 10 to 4, not 5000 to 2.
    assert_eq!(lot.may(&w, 2, 0), Err("a receive whose send is not in its sender's chain"));
    let last = w.get(id_a(), 2).entries.last().unwrap().seq;
    assert_eq!(lot.may(&w, 2, last), Err("an entry its committee has not witnessed"));
}

// ---- FOUNDATION 6d: stake answers for its keys, as held at its epoch's start ----

#[test]
fn a_writer_key_retired_while_staked_stays_burnable_across_a_cut() {
    let ck = |s: &State| check(&params(1), &def(), s, &|_, _| None, &|_| None, Some(&log(1)));
    let slash = |s: &State, k: u8| {
        let mut x = s.clone();
        let last = x.entries.last().map_or(0, |e| e.stamp);
        at_stamp(&mut x, 9, Body::Slash { key: pk(k), a: wsig(k, 9, 5, 1), b: wsig(k, 9, 5, 2) }, None, last);
        ck(&x)
    };
    // Staked, then a new writer (device 2): device 1's key still burns the stake.
    let mut a = staker();
    signed(&mut a, 1, 2, Body::Stake { amount: 600 });
    let e = writer_entry(&a, 1, 2, &owner_seed(1), 0, 2);
    a.entries.push(e);
    assert_eq!(ck(&a), Ok(()));
    assert_eq!(slash(&a, 1), Ok(()), "the retired key");
    assert_eq!(slash(&a, 2), Ok(()), "the writer's");
    assert_eq!(slash(&a, 3), Err("a slash of a key that is not this chain's"));
    // CONTROL: the same rotation with nothing staked retires nothing.
    let mut b = staker();
    let e = writer_entry(&b, 1, 2, &owner_seed(1), 0, 2);
    b.entries.push(e);
    assert_eq!(ck(&b), Ok(()));
    assert_eq!(slash(&b, 1), Err("a slash of a key that is not this chain's"));
    // A cut carries the retired key: the chain starting at it still burns for it.
    let (body, _) = cut::make(&params(1), &def(), &a, Some(&log(1))).unwrap();
    assert!(matches!(&body, Body::Cut { retired, .. } if retired == &vec![pk(1)]));
    let mut s = a.clone();
    at_stamp(&mut s, 99, body.clone(), Some(2), 0);
    assert_eq!(ck(&s), Ok(()));
    let pruned = prune(&s);
    assert_eq!(slash(&pruned, 1), Ok(()), "across the cut");
    assert_eq!(Entry::decode(&pruned.entries[0].encode()).as_ref(), Some(&pruned.entries[0]), "the cut round-trips with its retired keys");
    // A cut dropping it is not what came before it.
    let mut lie = a.clone();
    let mut dropped = body.clone();
    if let Body::Cut { retired, .. } = &mut dropped {
        retired.clear();
    }
    at_stamp(&mut lie, 99, dropped, Some(2), 0);
    assert_eq!(ck(&lie), Err("a cut that is not what came before it"));
    // Once nothing is staked or unbonding any more, nothing is left to burn: the retired keys go.
    let mut out = a.clone();
    at_stamp(&mut out, 3, Body::Unstake { amount: 600 }, Some(2), 10);
    at_stamp(&mut out, 4, Body::Release { unstake: nonce(3) }, None, 10 + UNBOND);
    assert_eq!(ck(&out), Ok(()));
    assert_eq!(slash(&out, 1), Err("a slash of a key that is not this chain's"));
}

#[test]
fn stake_counts_as_held_before_a_height_with_the_keys_it_burns_for() {
    let (p, d) = (params(1), def());
    let mut a = staker(); // genesis at 0, writer device 1
    at_stamp(&mut a, 2, Body::Stake { amount: 600 }, Some(1), 10);
    at_stamp(&mut a, 3, Body::Unstake { amount: 200 }, Some(1), 20);
    let e = writer_entry(&a, 1, 2, &owner_seed(1), 0, 2);
    a.entries.push(Entry { stamp: 30, ..e });
    let fix = |mut s: State| {
        // Re-sign the writer entry after its stamp moved.
        let i = s.entries.len() - 1;
        s.entries[i] = sign(s.entries[i].clone(), 2);
        s
    };
    let a = fix(a);
    assert_eq!(check(&p, &d, &a, &|_, _| None, &|_| None, Some(&log(1))), Ok(()));
    assert_eq!(wallet::stake_at(&p, &d, &a, 10), Ok((0, vec![pk(1)])), "the stake at 10 is not before 10");
    assert_eq!(wallet::stake_at(&p, &d, &a, 11), Ok((600, vec![pk(1)])));
    assert_eq!(wallet::stake_at(&p, &d, &a, 21), Ok((400, vec![pk(1)])), "unbonding is not counted");
    assert_eq!(wallet::stake_at(&p, &d, &a, 31), Ok((400, vec![pk(1), pk(2)])), "the retired key and the writer's");
    // A chain held from a cut stamped after the height cannot say: read it whole.
    let (body, _) = cut::make(&p, &d, &a, Some(&log(1))).unwrap();
    let mut s = a.clone();
    at_stamp(&mut s, 99, body, Some(2), 40);
    let pruned = prune(&s);
    assert_eq!(wallet::stake_at(&p, &d, &pruned, 31), Err("a stake before what is held"));
    assert_eq!(wallet::stake_at(&p, &d, &pruned, 41), Ok((400, vec![pk(1), pk(2)])));
}

#[test]
fn stake_counted_for_an_epoch_stays_burnable_a_full_epoch_after_it_ends() {
    let ck = |s: &State| check(&params(1), &def(), s, &|_, _| None, &|_| None, Some(&log(1)));
    let e1 = supply::EPOCH; // epoch 1 starts here; claims of epoch 1 count the stake held before it
    let mut a = staker();
    at_stamp(&mut a, 2, Body::Stake { amount: 600 }, Some(1), e1 - 1);
    assert_eq!(wallet::stake_at(&params(1), &def(), &a, e1), Ok((600, vec![pk(1)])));
    // Unstaked the moment epoch 1 starts: still unbonding — burnable — to the last step of epoch 2.
    at_stamp(&mut a, 3, Body::Unstake { amount: 600 }, Some(1), e1);
    let mut early = a.clone();
    at_stamp(&mut early, 4, Body::Release { unstake: nonce(3) }, None, 3 * supply::EPOCH - 1);
    assert_eq!(ck(&early), Err("a release before its delay"));
    let mut slashed = a.clone();
    at_stamp(&mut slashed, 4, Body::Slash { key: pk(1), a: wsig(1, 9, 5, 1), b: wsig(1, 9, 5, 2) }, None, 3 * supply::EPOCH - 1);
    assert_eq!(ck(&slashed), Ok(()), "burned for what it signed in epoch 1");
}

// ---- VESTING (TOKEN §4a): a debit now, released in steps, never taken back ----

fn vest_of(steps: &[(u64, u128)]) -> Body {
    Body::Vest { to: did(2), schedule: steps.to_vec() }
}
/// 1: genesis, then vests 100 at 10 and 200 at 20 to 2 (stamp 1).
fn vesting() -> Two {
    let mut w = Two(HashMap::new());
    put_at(w.chain(id_a(), 1), 1, Body::Genesis { amount: 1000 }, Some(1), 0);
    put_at(w.chain(id_a(), 1), 5, vest_of(&[(10, 100), (20, 200)]), Some(1), 1);
    w
}

#[test]
fn a_vest_is_a_debit_of_its_whole_schedule_now() {
    let w = vesting();
    assert_eq!(w.check(id_a(), 1), Ok(()));
    assert_eq!(w.get(id_a(), 1).balance(), 700);
    let try_vest = |b: Body| {
        let mut x = Two(HashMap::new());
        put_at(x.chain(id_a(), 1), 1, Body::Genesis { amount: 1000 }, Some(1), 0);
        put_at(x.chain(id_a(), 1), 5, b, Some(1), 1);
        x.check(id_a(), 1)
    };
    assert_eq!(try_vest(vest_of(&[])), Err("a vesting schedule that is not rising steps"));
    assert_eq!(try_vest(vest_of(&[(10, 100), (10, 100)])), Err("a vesting schedule that is not rising steps"));
    assert_eq!(try_vest(vest_of(&[(20, 100), (10, 100)])), Err("a vesting schedule that is not rising steps"));
    assert_eq!(try_vest(vest_of(&[(10, 100), (20, 9)])), Err("a send under the minimum"));
    assert_eq!(try_vest(vest_of(&[(10, 600), (20, 401)])), Err("a debit over the balance"));
    assert_eq!(try_vest(vest_of(&[(10, 600), (20, 400)])), Ok(()), "the whole balance");
    assert_eq!(try_vest(Body::Vest { to: did(1), schedule: vec![(10, 100)] }), Err("a vest to itself"));
    assert_eq!(try_vest(vest_of(&(0..=VEST_STEPS as u64).map(|i| (i + 1, 10)).collect::<Vec<_>>())), Err("a vesting schedule that is not rising steps"), "past the step cap");
    assert_eq!(try_vest(vest_of(&(0..VEST_STEPS as u64).map(|i| (i + 1, 10)).collect::<Vec<_>>())), Ok(()), "at the cap");
    // Never taken back: the giver has no entry that undoes it — a refund of it names no lock.
    let mut back = vesting();
    put_at(back.chain(id_a(), 1), 6, Body::Refund { lock: nonce(5), ev: Evidence::default() }, None, 30);
    assert_eq!(back.check(id_a(), 1), Err("a refund of a lock that is not here"));
}

#[test]
fn each_vesting_step_is_received_once_as_its_schedule_says() {
    let mut w = vesting();
    put_at(w.chain(id_a(), 2), 10, Body::Vested { from: did(1), vest: nonce(5), step: 0, amount: 100, at: 10, cited: 1, ev: Evidence::default() }, None, 10);
    assert_eq!(w.check(id_a(), 2), Ok(()));
    assert_eq!(w.get(id_a(), 2).balance(), 100);
    let with = |b: Body| {
        let mut x = Two(w.0.clone());
        put_at(x.chain(id_a(), 2), 11, b, None, 20);
        x.check(id_a(), 2)
    };
    let v = |step: u32, amount: u128, at: u64, vest: u8, cited: u64| Body::Vested { from: did(1), vest: nonce(vest), step, amount, at, cited, ev: Evidence::default() };
    assert_eq!(with(v(1, 200, 20, 5, 1)), Ok(()), "the second step");
    assert_eq!(with(v(0, 100, 10, 5, 1)), Err("a vesting step received twice"));
    assert_eq!(with(v(1, 201, 20, 5, 1)), Err("a vesting step that is not in its giver's chain"), "another amount");
    assert_eq!(with(v(1, 200, 19, 5, 1)), Err("a vesting step that is not in its giver's chain"), "another time");
    assert_eq!(with(v(2, 200, 20, 5, 1)), Err("a vesting step that is not in its giver's chain"), "no such step");
    assert_eq!(with(v(1, 200, 20, 6, 1)), Err("a vesting step that is not in its giver's chain"), "no such vest");
    assert_eq!(with(v(1, 200, 20, 5, 0)), Err("a vesting step that is not in its giver's chain"), "another stamp");
    // Only its beneficiary: the same step credited to 3 is not in the giver's chain.
    let mut x = Two(w.0.clone());
    put_at(x.chain(id_a(), 3), 10, v(1, 200, 20, 5, 1), None, 20);
    assert_eq!(x.check(id_a(), 3), Err("a vesting step that is not in its giver's chain"));
}

#[test]
fn a_vesting_step_counts_only_from_its_time_even_in_a_first_copy() {
    // Checked from scratch (as a node takes a FIRST copy, with no append rules): stamped before its step, refused.
    let w = vesting();
    let at = |stamp: u64| {
        let mut x = Two(w.0.clone());
        put_at(x.chain(id_a(), 2), 10, Body::Vested { from: did(1), vest: nonce(5), step: 1, amount: 200, at: 20, cited: 1, ev: Evidence::default() }, None, stamp);
        x.check(id_a(), 2)
    };
    assert_eq!(at(19), Err("a vesting step before its time"));
    assert_eq!(at(20), Ok(()));
    // A step stamped at its time but written BEFORE it (a stamp from the future): no witness signs it.
    let mut x = Two(w.0.clone());
    put_at(x.chain(id_a(), 2), 10, Body::Vested { from: did(1), vest: nonce(5), step: 1, amount: 200, at: 20, cited: 1, ev: Evidence::default() }, None, 20);
    let mut lot = Lot::new();
    lot.witness(&x, 1).unwrap();
    lot.now = 19;
    assert_eq!(lot.may(&x, 2, 0), Err("an entry stamped ahead of the beacon"));
    lot.now = 20;
    assert!(lot.may(&x, 2, 0).is_ok());
    // And appended: citing a vest not yet final, refused.
    let held = base(0);
    let early = |stamp: u64| {
        let mut m = held.clone();
        put_at(&mut m, 2, Body::Vested { from: did(9), vest: nonce(5), step: 0, amount: 100, at: 0, cited: 10, ev: Evidence::default() }, None, stamp);
        m
    };
    assert_eq!(append_check(&def(), 11, &held, &early(11), Some(&log(M))), Err("an entry citing one not yet final"));
    assert_eq!(append_check(&def(), 12, &held, &early(12), Some(&log(M))), Ok(()));
}

#[test]
fn vesting_round_trips_and_is_witnessed_on_its_vest() {
    let w = vesting();
    let e = &w.get(id_a(), 1).entries.last().unwrap();
    assert_eq!(Entry::decode(&e.encode()).as_ref(), Some(*e));
    let mut w = w;
    put_at(w.chain(id_a(), 2), 10, Body::Vested { from: did(1), vest: nonce(5), step: 1, amount: 200, at: 20, cited: 1, ev: Evidence::default() }, None, 20);
    let e2 = w.get(id_a(), 2).entries.last().unwrap().clone();
    assert_eq!(Entry::decode(&e2.encode()), Some(e2));
    // A step stands on its vest: not witnessable until the vest is accepted.
    let mut lot = Lot::new();
    assert_eq!(lot.may(&w, 2, 0), Err("an entry its committee has not witnessed"));
    lot.witness(&w, 1).unwrap();
    assert!(lot.may(&w, 2, 0).is_ok());
    // In a cut's received set: a cut chain refuses the same step again without its absence proof.
    assert!(cut::received_keys(&w.get(id_a(), 2).entries).contains(&vested_key(&did(1), &nonce(5), 1)));
}

#[test]
fn a_witness_applies_the_timed_rules_at_the_entrys_own_stamp() {
    // A chain taken as a FIRST copy (no append rules): its refund stamped before its lock's deadline, its fill too close
    // to its deadline, its receive before what it cites is final — the contract from scratch takes all three; no
    // witness signs them.
    let lot = Lot::new();
    let p = at(id_a(), 1);
    let ok = |_: &State| Ok(());
    let may = |s: &State| wallet::may_witness(&lot.src(), &p, &def(), s, s.entries.last().unwrap().seq, 100, &ok, &|_, _| None);
    let mut locked = base(0);
    put_at(&mut locked, 3, Body::Lock { to: did(9), amount: 50, until: 20, token: [5; 32].into(), offer: nonce(1), fee: None }, Some(M), 0);
    let refund = |stamp: u64| {
        let mut m = locked.clone();
        put_at(&mut m, 4, Body::Refund { lock: nonce(3), ev: Evidence::default() }, None, stamp);
        m
    };
    assert_eq!(may(&refund(19)), Err("a refund before its lock's deadline"));
    assert_eq!(may(&refund(20)).err(), Some("an entry its committee has not witnessed"), "past the timed rule: its predecessor");
    let recv = |stamp: u64| {
        let mut m = base(0);
        put_at(&mut m, 2, Body::Receive { from: did(9), nonce: nonce(2), amount: 50, cited: 10, ev: Evidence::default() }, None, stamp);
        m
    };
    assert_eq!(may(&recv(11)), Err("an entry citing one not yet final"));
    assert_eq!(may(&recv(12)).err(), Some("an entry its committee has not witnessed"));
    // A release before its delay, at its own stamp.
    let mut a = staker();
    signed(&mut a, 1, 2, Body::Stake { amount: 600 });
    at_stamp(&mut a, 3, Body::Unstake { amount: 200 }, Some(1), 10);
    let rel = |stamp: u64| {
        let mut m = a.clone();
        at_stamp(&mut m, 4, Body::Release { unstake: nonce(3) }, None, stamp);
        m
    };
    let pp = params(1);
    let mayp = |s: &State| wallet::may_witness(&lot.src(), &pp, &def(), s, s.entries.last().unwrap().seq, u64::MAX, &ok, &|_, _| None);
    assert_eq!(mayp(&rel(10 + UNBOND - 1)), Err("a release before its delay"));
    assert_eq!(mayp(&rel(10 + UNBOND)).err(), Some("an entry its committee has not witnessed"));
}
