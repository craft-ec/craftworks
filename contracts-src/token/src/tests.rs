use super::*;
use ed25519_dalek::{Signer, SigningKey};
use std::collections::HashMap;

const CODE: [u8; 32] = [9; 32];
const BEACON: [u8; 32] = [7; 32];

fn key(n: u8) -> SigningKey {
    SigningKey::from_bytes(&[n; 32])
}
fn pk(n: u8) -> [u8; 32] {
    key(n).verifying_key().to_bytes()
}
fn def() -> Definition {
    Definition { code_hash: CODE, decimals: 8, min_send: 10, genesis_owner: pk(1), genesis_amount: 1000, name: b"T".to_vec(), nonce: [0; 16], beacon: BEACON, margin: 2 }
}
fn params(owner: u8) -> ChainParams {
    ChainParams { code_hash: CODE, definition: id(&CODE, &def().encode()), owner: pk(owner), beacon: BEACON }
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
        Body::Allow { .. } | Body::Offer { .. } => b,
        Body::Spend { amount, .. } | Body::Lock { amount, .. } | Body::Fill { amount, .. } => b.wrapping_sub(*amount),
        Body::Claim { amount, .. } => b + amount,
        Body::Refund { lock } => {
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
    let e = delegate(entry(s, nonce(n), Body::Spend { to: pk(to), amount }), by);
    s.entries.push(e);
}
fn entry(s: &State, n: [u8; 16], body: Body) -> Entry {
    let balance = after(s, &body);
    let stamp = s.entries.last().map_or(0, |e| e.stamp);
    Entry { seq: s.entries.len() as u64, prev: s.head(), nonce: n, stamp, body, balance, auth: Auth::Work(0) }
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
    let e = work(entry(s, nonce(n), Body::Receive { from: pk(from), nonce: nonce(n), amount, cited: 0 }));
    s.entries.push(e);
}

struct World(HashMap<[u8; 32], State>);
impl World {
    fn check(&self, owner: u8) -> Result<(), Why> {
        let s = &self.0[&pk(owner)];
        let tok = params(1).definition;
        check(&params(owner), &def(), s, &|d, o| if *d == tok { self.0.get(o).cloned() } else { None })
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
    assert_eq!(check(&params(1), &d, &w.0[&pk(1)], &|_, o| w.0.get(o).cloned()), Err("the definition is not this standard's"));
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

#[test]
fn a_receive_from_itself_is_refused_without_asking_for_itself() {
    let mut w = paid();
    received(w.0.get_mut(&pk(2)).unwrap(), 2, 7, 20);
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
    signed(w.0.get_mut(&pk(1)).unwrap(), 1, 3, Body::Allow { spender: pk(5), limit: 200 });
    spent(w.0.get_mut(&pk(1)).unwrap(), 5, 4, 3, 150);
    let mut c = State::default();
    received(&mut c, 1, 4, 150);
    w.0.insert(pk(3), c);
    w
}

#[test]
fn a_spender_spends_within_its_allowance_and_it_is_received() {
    let w = allowed();
    assert_eq!(w.check(1), Ok(()));
    assert_eq!(w.check(3), Ok(()));
    assert_eq!(w.0[&pk(1)].balance(), 550);
    assert_eq!(w.0[&pk(3)].balance(), 150);
}

#[test]
fn a_spend_over_what_is_left_of_its_allowance_is_refused() {
    let mut w = allowed();
    spent(w.0.get_mut(&pk(1)).unwrap(), 5, 5, 3, 51);
    assert_eq!(w.check(1), Err("a spend over its allowance"));
    let mut w = allowed();
    spent(w.0.get_mut(&pk(1)).unwrap(), 5, 5, 3, 50);
    assert_eq!(w.check(1), Ok(()), "exactly what is left");
}

#[test]
fn only_an_allowed_key_spends_and_only_by_its_own_signature() {
    let mut w = allowed();
    spent(w.0.get_mut(&pk(1)).unwrap(), 6, 5, 3, 20);
    assert_eq!(w.check(1), Err("a spend with no allowance"));
    // A spend signed as the owner is not a spend.
    let mut w = allowed();
    signed(w.0.get_mut(&pk(1)).unwrap(), 1, 5, Body::Spend { to: pk(3), amount: 20 });
    assert_eq!(w.check(1), Err("a spend not signed by a spender"));
    // Naming the spender but signed by someone else.
    let mut w = allowed();
    let a = w.0.get_mut(&pk(1)).unwrap();
    let mut e = entry(a, nonce(5), Body::Spend { to: pk(3), amount: 20 });
    e.auth = Auth::Delegated { by: pk(5), sig: key(6).sign(&e.message()).to_bytes() };
    a.entries.push(e);
    assert_eq!(w.check(1), Err("a spend not signed by a spender"));
}

#[test]
fn a_new_allowance_replaces_the_old_and_zero_revokes() {
    let mut w = allowed();
    signed(w.0.get_mut(&pk(1)).unwrap(), 1, 5, Body::Allow { spender: pk(5), limit: 10 });
    spent(w.0.get_mut(&pk(1)).unwrap(), 5, 6, 3, 11);
    assert_eq!(w.check(1), Err("a spend over its allowance"), "replaced, not added");
    let mut w = allowed();
    signed(w.0.get_mut(&pk(1)).unwrap(), 1, 5, Body::Allow { spender: pk(5), limit: 0 });
    spent(w.0.get_mut(&pk(1)).unwrap(), 5, 6, 3, 10);
    assert_eq!(w.check(1), Err("a spend with no allowance"), "revoked");
    let mut w = allowed();
    signed(w.0.get_mut(&pk(1)).unwrap(), 1, 5, Body::Allow { spender: pk(1), limit: 10 });
    assert_eq!(w.check(1), Err("an allowance to itself"));
}

#[test]
fn an_owner_entry_and_a_spender_entry_at_one_seq_do_not_freeze_the_chain() {
    let w = allowed();
    let base = w.0[&pk(1)].clone();
    let mut by_owner = base.clone();
    signed(&mut by_owner, 1, 7, Body::Send { to: pk(2), amount: 20 });
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
    Definition { genesis_owner: pk(2), name: b"B".to_vec(), nonce: [1; 16], ..def() }
}
fn id_a() -> [u8; 32] {
    id(&CODE, &def().encode())
}
fn id_b() -> [u8; 32] {
    id(&CODE, &def_b().encode())
}
fn at(token: [u8; 32], owner: u8) -> ChainParams {
    ChainParams { code_hash: CODE, definition: token, owner: pk(owner), beacon: BEACON }
}

/// An entry at `stamp`, by the writer `by` (Some) or with work (None).
fn put_at(s: &mut State, n: u8, body: Body, by: Option<u8>, stamp: u64) {
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
        &self.0[&(token, pk(owner))]
    }
    fn chain(&mut self, token: [u8; 32], owner: u8) -> &mut State {
        self.0.entry((token, pk(owner))).or_default()
    }
    fn check(&self, token: [u8; 32], owner: u8) -> Result<(), Why> {
        let d = if token == id_a() { def() } else { def_b() };
        check(&at(token, owner), &d, self.get(token, owner), &|t, o| self.0.get(&(*t, *o)).cloned())
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
    put_at(w.chain(id_b(), T), 20, Body::Lock { to: pk(M), amount: 100, until: 50, token: id_a(), offer: nonce(10) }, Some(T), 0);
    put_at(w.chain(id_a(), M), 30, Body::Fill { offer: nonce(10), taker: pk(T), lock: nonce(20), amount: 200, cited: 0, until: 50 }, None, 0);
    put_at(w.chain(id_a(), T), 30, Body::Receive { from: pk(M), nonce: nonce(30), amount: 200, cited: 0 }, None, 0);
    put_at(w.chain(id_b(), M), 40, Body::Claim { taker: pk(T), lock: nonce(20), amount: 100, cited: 0, token: id_a() }, None, 0);
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
    let w = swapped_with_lock(Body::Lock { to: pk(M), amount: 99, until: 50, token: id_a(), offer: nonce(10) });
    assert_eq!(w.check(id_a(), M), Err("a fill its lock does not pay for"));
    // A lock for another maker, another token, another offer, another deadline.
    for lock in [
        Body::Lock { to: pk(9), amount: 100, until: 50, token: id_a(), offer: nonce(10) },
        Body::Lock { to: pk(M), amount: 100, until: 50, token: [5; 32], offer: nonce(10) },
        Body::Lock { to: pk(M), amount: 100, until: 50, token: id_a(), offer: nonce(11) },
        Body::Lock { to: pk(M), amount: 100, until: 51, token: id_a(), offer: nonce(10) },
    ] {
        let w = swapped_with_lock(lock);
        assert_eq!(w.check(id_a(), M), Err("a fill its lock does not pay for"));
    }
}

#[test]
fn a_fill_is_bounded_by_its_offer_and_fills_a_lock_once() {
    let mut w = swapped();
    // Another lock of T's for 150 B → 300 A; only 200 are left of the offer.
    put_at(w.chain(id_b(), T), 21, Body::Lock { to: pk(M), amount: 150, until: 50, token: id_a(), offer: nonce(10) }, Some(T), 0);
    let mut over = w.chain(id_a(), M).clone();
    put_at(&mut over, 31, Body::Fill { offer: nonce(10), taker: pk(T), lock: nonce(21), amount: 300, cited: 0, until: 50 }, None, 0);
    *w.chain(id_a(), M) = over;
    assert_eq!(w.check(id_a(), M), Err("a fill past what is left of its offer"));
    let mut w = swapped();
    put_at(w.chain(id_a(), M), 31, Body::Fill { offer: nonce(10), taker: pk(T), lock: nonce(20), amount: 10, cited: 0, until: 50 }, None, 0);
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
    put_at(w.chain(id_b(), M), 41, Body::Claim { taker: pk(T), lock: nonce(20), amount: 100, cited: 0, token: id_a() }, None, 0);
    assert_eq!(w.check(id_b(), M), Err("a lock claimed twice"));
    // A claim of more than the lock held.
    let mut w = swapped();
    let mb = w.chain(id_b(), M);
    mb.entries.clear();
    put_at(mb, 40, Body::Claim { taker: pk(T), lock: nonce(20), amount: 101, cited: 0, token: id_a() }, None, 0);
    assert_eq!(w.check(id_b(), M), Err("a claim of a lock that is not filled"));
}

#[test]
fn an_unfilled_lock_is_refunded_once_and_a_filled_one_never() {
    // A second lock nobody fills: refunded.
    let mut w = swapped();
    put_at(w.chain(id_b(), T), 21, Body::Lock { to: pk(M), amount: 50, until: 50, token: id_a(), offer: nonce(10) }, Some(T), 0);
    put_at(w.chain(id_b(), T), 22, Body::Refund { lock: nonce(21) }, None, 0);
    assert_eq!(w.check(id_b(), T), Ok(()));
    assert_eq!(w.get(id_b(), T).balance(), 900, "the 50 back");
    put_at(w.chain(id_b(), T), 23, Body::Refund { lock: nonce(21) }, None, 0);
    assert_eq!(w.check(id_b(), T), Err("a lock refunded twice"));
    // The filled one.
    let mut w = swapped();
    put_at(w.chain(id_b(), T), 22, Body::Refund { lock: nonce(20) }, None, 0);
    assert_eq!(w.check(id_b(), T), Err("a refund of a filled lock"));
}

#[test]
fn a_receive_of_a_fill_names_its_stamp() {
    let mut w = swapped();
    let ta = w.chain(id_a(), T);
    ta.entries.clear();
    put_at(ta, 30, Body::Receive { from: pk(M), nonce: nonce(30), amount: 200, cited: 1 }, None, 0);
    assert_eq!(w.check(id_a(), T), Err("a receive whose send is not in its sender's chain"));
}

#[test]
fn stamps_never_go_down() {
    let mut w = swapped();
    let tb = w.chain(id_b(), T);
    let last = tb.entries.len() - 1;
    tb.entries.truncate(last);
    put_at(tb, 20, Body::Lock { to: pk(M), amount: 100, until: 50, token: id_a(), offer: nonce(10) }, Some(T), 0);
    let mut s = w.get(id_b(), T).clone();
    s.entries.truncate(1);
    s.entries[0].stamp = 5;
    let g = sign(Entry { auth: Auth::Work(0), ..s.entries[0].clone() }, T);
    s.entries[0] = g;
    put_at(&mut s, 21, Body::Burn { amount: 10 }, Some(T), 4);
    assert_eq!(check(&at(id_b(), T), &def_b(), &s, &|_, _| None), Err("a stamp below its predecessor's"));
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
    assert_eq!(append_check(&def(), 10, &held, &merged), Err("a stamp far from the beacon"), "3 behind, margin 2");
    assert_eq!(append_check(&def(), 9, &held, &merged), Ok(()), "2 behind");
    assert_eq!(append_check(&def(), 6, &held, &merged), Err("a stamp far from the beacon"), "ahead of the beacon");
    // What the node already held is never checked again, however old.
    assert_eq!(append_check(&def(), 1000, &merged, &merged), Ok(()));
}

#[test]
fn a_citation_waits_a_margin_after_what_it_cites() {
    let held = base(0);
    let mut merged = held.clone();
    put_at(&mut merged, 2, Body::Receive { from: pk(9), nonce: nonce(2), amount: 50, cited: 10 }, None, 11);
    assert_eq!(append_check(&def(), 11, &held, &merged), Err("an entry citing one not yet final"));
    let mut merged = held.clone();
    put_at(&mut merged, 2, Body::Receive { from: pk(9), nonce: nonce(2), amount: 50, cited: 10 }, None, 12);
    assert_eq!(append_check(&def(), 12, &held, &merged), Ok(()));
}

#[test]
fn a_fill_lands_a_margin_before_its_deadline_and_a_refund_after_it() {
    let held = base(0);
    let fill = |stamp| {
        let mut m = held.clone();
        put_at(&mut m, 2, Body::Fill { offer: nonce(1), taker: pk(T), lock: nonce(9), amount: 10, cited: 5, until: 20 }, None, stamp);
        m
    };
    assert_eq!(append_check(&def(), 18, &held, &fill(18)), Err("a fill too close to its lock's deadline"));
    assert_eq!(append_check(&def(), 17, &held, &fill(17)), Ok(()));
    assert_eq!(append_check(&def(), 6, &held, &fill(6)), Err("an entry citing one not yet final"), "the lock not final yet");
    let mut locked = base(0);
    put_at(&mut locked, 3, Body::Lock { to: pk(9), amount: 50, until: 20, token: [5; 32], offer: nonce(1) }, Some(M), 0);
    let refund = |stamp| {
        let mut m = locked.clone();
        put_at(&mut m, 4, Body::Refund { lock: nonce(3) }, None, stamp);
        m
    };
    assert_eq!(append_check(&def(), 19, &locked, &refund(19)), Err("a refund before its lock's deadline"));
    assert_eq!(append_check(&def(), 20, &locked, &refund(20)), Ok(()));
}

// ---- step 6: wallet verification ----
use crate::wallet::{Checkpoint, Source, Vouch, Wallet};

fn defs(t: &[u8; 32]) -> Option<Definition> {
    if *t == id_a() {
        Some(def())
    } else if *t == id_b() {
        Some(def_b())
    } else {
        None
    }
}
fn verify(w: &Two, wallet: &mut Wallet, owner: u8) -> Result<(), crate::wallet::FraudProof> {
    let chain = |p: &ChainParams| w.0.get(&(p.definition, p.owner)).cloned();
    let src = Source { chain: &chain, definition: &defs };
    let last = w.get(id_a(), owner).entries.len() as u64 - 1;
    wallet.verify(&src, &at(id_a(), owner), last)
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
    put_at(w.chain(id_a(), 1), 2, Body::Send { to: pk(2), amount: 300 }, Some(1), 0);
    put_at(w.chain(id_a(), 2), 2, Body::Receive { from: pk(1), nonce: nonce(2), amount: 300, cited: 0 }, None, 0);
    put_at(w.chain(id_a(), 2), 3, Body::Send { to: pk(3), amount: 100 }, Some(2), 0);
    put_at(w.chain(id_a(), 3), 3, Body::Receive { from: pk(2), nonce: nonce(3), amount: 100, cited: 0 }, None, 0);
    w
}

/// 2 credits itself 5000 from a send that does not exist, buried under RECENT real receives (1 sends them), then pays
/// 3 out of it.
fn buried() -> Two {
    let mut w = Two(HashMap::new());
    put_at(w.chain(id_a(), 1), 1, Body::Genesis { amount: 1000 }, Some(1), 0);
    put_at(w.chain(id_a(), 2), 9, Body::Receive { from: pk(1), nonce: nonce(9), amount: 5000, cited: 0 }, None, 0);
    for k in 0..RECENT as u8 {
        put_at(w.chain(id_a(), 1), 20 + k, Body::Send { to: pk(2), amount: 10 }, Some(1), 0);
        put_at(w.chain(id_a(), 2), 20 + k, Body::Receive { from: pk(1), nonce: nonce(20 + k), amount: 10, cited: 0 }, None, 0);
    }
    put_at(w.chain(id_a(), 2), 40, Body::Send { to: pk(3), amount: 3000 }, Some(2), 0);
    put_at(w.chain(id_a(), 3), 40, Body::Receive { from: pk(2), nonce: nonce(40), amount: 3000, cited: 0 }, None, 0);
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
    put_at(w.chain(id_a(), 4), 51, Body::Receive { from: pk(5), nonce: nonce(52), amount: 50, cited: 0 }, None, 0);
    put_at(w.chain(id_a(), 4), 53, Body::Send { to: pk(5), amount: 50 }, Some(4), 0);
    put_at(w.chain(id_a(), 5), 54, Body::Receive { from: pk(4), nonce: nonce(53), amount: 50, cited: 0 }, None, 0);
    put_at(w.chain(id_a(), 5), 52, Body::Send { to: pk(4), amount: 50 }, Some(5), 0);
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
    put_at(w.chain(id_a(), 2), 4, Body::Send { to: pk(1), amount: 50 }, Some(2), 0);
    put_at(w.chain(id_a(), 1), 4, Body::Receive { from: pk(2), nonce: nonce(4), amount: 50, cited: 0 }, None, 0);
    put_at(w.chain(id_a(), 1), 5, Body::Send { to: pk(6), amount: 100 }, Some(1), 0);
    put_at(w.chain(id_a(), 6), 5, Body::Receive { from: pk(1), nonce: nonce(5), amount: 100, cited: 0 }, None, 0);
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
    let src = Source { chain: &chain, definition: &defs };
    let proof = trusting.audit(&src, &c).unwrap_err();
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
    let src = Source { chain: &chain, definition: &defs };
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
    let c = Checkpoint { chain, seq: e.seq, head: wrong, balance: e.balance, by: pk(9), sig: key(9).sign(&m).to_bytes() };
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
        put_at(w.chain(id_a(), 1), 2, Body::Send { to: pk(2), amount: 900 }, Some(1), 0);
        for k in 2..n {
            put_at(w.chain(id_a(), k), k, Body::Receive { from: pk(k - 1), nonce: nonce(k), amount: 900 - (k as u128 - 2), cited: 0 }, None, 0);
            put_at(w.chain(id_a(), k), k + 1, Body::Send { to: pk(k + 1), amount: 900 - (k as u128 - 1) }, Some(k), 0);
        }
        let last = n;
        put_at(w.chain(id_a(), last), last, Body::Receive { from: pk(last - 1), nonce: nonce(last), amount: 900 - (last as u128 - 2), cited: 0 }, None, 0);
        let mut wallet = Wallet::new(pk(last));
        (verify(&w, &mut wallet, last), wallet.reads.get())
    }).unwrap().join().unwrap();
    assert_eq!(r.0, Ok(()));
    assert!(r.1 >= 199, "{} reads", r.1);
}
