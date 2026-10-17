//! The door to BeeKEM: each property with its control.
use super::*;
use rand::{rngs::StdRng, SeedableRng};
use std::collections::{BTreeMap, BTreeSet};

const SPACE: [u8; 32] = [0x5a; 32];

/// The list, as a test sets it: member id → account, listed accounts, acts → (add?, target).
#[derive(Default)]
struct List {
    accounts: BTreeMap<[u8; 32], String>,
    listed: BTreeSet<String>,
    acts: BTreeMap<[u8; 32], (bool, [u8; 32])>,
}
impl Check for List {
    fn account_of(&self, member: &[u8; 32]) -> Option<String> {
        self.accounts.get(member).cloned()
    }
    fn listed(&self, did: &str) -> bool {
        self.listed.contains(did)
    }
    fn allows(&self, act: &[u8; 32], add: bool, target: &[u8; 32]) -> bool {
        self.acts.get(act) == Some(&(add, *target))
    }
}

fn account(n: u8) -> Account {
    Account::of(&[n; 32])
}
fn id(a: &Account) -> [u8; 32] {
    a.id.0.to_bytes()
}
fn list(of: &[(&Account, &str)]) -> List {
    let mut l = List::default();
    for (a, did) in of {
        l.accounts.insert(id(a), did.to_string());
        l.listed.insert(did.to_string());
        // (The maker's own placement names no act: allowed for the first member.)
        l.acts.insert([0; 32], (true, id(of[0].0)));
    }
    l
}
fn merge_all(g: &mut Group, ops: &[Vec<u8>], l: &impl Check) {
    for op in ops {
        assert_eq!(g.merge(op, l), Merged::Applied);
    }
}

#[test]
fn a_member_added_reads_and_writes_with_the_maker() {
    let mut rng = StdRng::seed_from_u64(1);
    let (a, b) = (account(1), account(2));
    let mut l = list(&[(&a, "did:a"), (&b, "did:b")]);
    l.acts.insert([7; 32], (true, id(&b)));
    let (mut ga, first) = Group::create(&SPACE, a.clone()).unwrap();
    let add = ga.add_member(&id(&b), &b.leaf_key(), &[7; 32]).unwrap().unwrap();
    let (rot, _) = ga.rotate(&mut rng).unwrap();
    let mut gb = Group::empty(&SPACE, b.clone());
    merge_all(&mut gb, &[first, add, rot], &l);
    assert!(gb.placed());
    let (sealed, op) = ga.seal([1; 32], b"hello b", &mut rng).unwrap();
    if let Some(op) = op {
        assert_eq!(gb.merge(&op, &l), Merged::Applied);
    }
    assert_eq!(gb.open(&sealed).unwrap(), b"hello b");
    let (back, op) = gb.seal([2; 32], b"hello a", &mut rng).unwrap();
    if let Some(op) = op {
        assert_eq!(ga.merge(&op, &l), Merged::Applied);
    }
    assert_eq!(ga.open(&back).unwrap(), b"hello a");
}

#[test]
fn our_check_refuses_an_outsiders_operation_and_an_add_of_an_unlisted_account() {
    let (a, b, x) = (account(1), account(2), account(9));
    let l = list(&[(&a, "did:a"), (&b, "did:b")]);
    let (mut ga, first) = Group::create(&SPACE, a.clone()).unwrap();
    // X (no listed account publishes its id) holds the operations and adds itself.
    let mut gx = Group::empty(&SPACE, x.clone());
    gx.merge(&first, &l);
    let evil = gx.add_member(&id(&x), &x.leaf_key(), &[7; 32]).unwrap().unwrap();
    assert!(matches!(ga.merge(&evil, &l), Merged::Refused(_)), "an outsider's operation");
    // A (listed) adds C, whose card is known but whom the list does not name: refused, whatever act it names; control —
    // C listed: taken (K7: an add of a listed account needs no act).
    let c = account(3);
    let mut unlisted = list(&[(&a, "did:a"), (&b, "did:b")]);
    unlisted.accounts.insert(id(&c), "did:c".into());
    let mut ga2 = Group::empty(&SPACE, a.clone());
    ga2.merge(&first, &unlisted);
    let add_c = ga.add_member(&id(&c), &c.leaf_key(), &[0; 32]).unwrap().unwrap();
    assert!(matches!(ga2.merge(&add_c, &unlisted), Merged::Refused(_)), "an add of an account not listed");
    let listed = list(&[(&a, "did:a"), (&b, "did:b"), (&c, "did:c")]);
    let mut ga3 = Group::empty(&SPACE, a.clone());
    ga3.merge(&first, &listed);
    assert_eq!(ga3.merge(&add_c, &listed), Merged::Applied, "control: listed, taken");
}

#[test]
fn reconcile_places_the_list_and_takes_out_whom_an_act_put_out() {
    let (a, b, c) = (account(1), account(2), account(3));
    let cards: Vec<Card> = [(&a, "did:a"), (&b, "did:b"), (&c, "did:c")].iter().map(|(p, d)| (d.to_string(), id(p), *p.leaf_key().as_bytes())).collect();
    let (mut ga, _) = Group::create(&SPACE, a.clone()).unwrap();
    let mut listed: Vec<&str> = vec!["did:a", "did:b", "did:c"];
    let r = ga.reconcile(&cards, &|d| listed.contains(&d), &Default::default(), &Default::default(), [5; 32]);
    assert_eq!((r.added.len(), r.removed.len(), r.ops.len()), (2, 0, 3), "B and C added, one key change: {:?}", r.not);
    assert_eq!(ga.members().len(), 3);
    // Again: nothing to do, nothing sent.
    assert!(ga.reconcile(&cards, &|d| listed.contains(&d), &Default::default(), &Default::default(), [6; 32]).ops.is_empty(), "the list placed: nothing");
    // C off the list: without the act putting it out, left (nothing decided); with it, removed.
    listed.retain(|d| *d != "did:c");
    assert!(ga.reconcile(&cards, &|d| listed.contains(&d), &Default::default(), &Default::default(), [7; 32]).removed.is_empty(), "control: no act, not removed");
    let outs = std::collections::BTreeMap::from([("did:c".to_string(), [4u8; 32])]);
    let r = ga.reconcile(&cards, &|d| listed.contains(&d), &Default::default(), &outs, [8; 32]);
    assert_eq!(r.removed, vec!["did:c".to_string()]);
    assert_eq!(ga.members().len(), 2);
}

#[test]
fn an_operation_ahead_of_its_predecessors_waits() {
    let (a, b) = (account(1), account(2));
    let mut l = list(&[(&a, "did:a"), (&b, "did:b")]);
    l.acts.insert([7; 32], (true, id(&b)));
    let (mut ga, first) = Group::create(&SPACE, a.clone()).unwrap();
    let add = ga.add_member(&id(&b), &b.leaf_key(), &[7; 32]).unwrap().unwrap();
    let mut gb = Group::empty(&SPACE, b);
    assert_eq!(gb.merge(&add, &l), Merged::Waiting, "its predecessor not held");
    assert_eq!(gb.merge(&first, &l), Merged::Applied);
    assert_eq!(gb.merge(&add, &l), Merged::Applied, "offered again: taken");
}

#[test]
fn a_removed_member_cannot_open_what_is_sealed_after() {
    let mut rng = StdRng::seed_from_u64(2);
    let (a, b) = (account(1), account(2));
    let mut l = list(&[(&a, "did:a"), (&b, "did:b")]);
    l.acts.insert([7; 32], (true, id(&b)));
    l.acts.insert([6; 32], (false, id(&b)));
    let (mut ga, first) = Group::create(&SPACE, a.clone()).unwrap();
    let add = ga.add_member(&id(&b), &b.leaf_key(), &[7; 32]).unwrap().unwrap();
    let (rot, _) = ga.rotate(&mut rng).unwrap();
    let mut gb = Group::empty(&SPACE, b.clone());
    merge_all(&mut gb, &[first, add, rot], &l);
    let remove = ga.remove_member(&id(&b), &[6; 32]).unwrap().unwrap();
    let (rot2, _) = ga.rotate(&mut rng).unwrap();
    let (sealed, _) = ga.seal([3; 32], b"after", &mut rng).unwrap();
    gb.merge(&remove, &l);
    gb.merge(&rot2, &l);
    assert!(gb.open(&sealed).is_err(), "removed: cannot open");
    // Control: a member still in (a second device of A) opens it.
    let mut ga_copy = Group::load(&ga.save().unwrap(), a).unwrap();
    assert_eq!(ga_copy.open(&sealed).unwrap(), b"after");
}

#[test]
fn an_accounts_devices_are_one_member_and_derive_each_others_new_leaf() {
    let mut rng = StdRng::seed_from_u64(3);
    let (a, b) = (account(1), account(2));
    assert_eq!(id(&Account::of(&[2; 32])), id(&b), "every device of an account derives the same member");
    let mut l = list(&[(&a, "did:a"), (&b, "did:b")]);
    l.acts.insert([7; 32], (true, id(&b)));
    let (mut ga, first) = Group::create(&SPACE, a.clone()).unwrap();
    let add = ga.add_member(&id(&b), &b.leaf_key(), &[7; 32]).unwrap().unwrap();
    let (rot, _) = ga.rotate(&mut rng).unwrap();
    let ops = [first, add, rot];
    let (mut b1, mut b2) = (Group::empty(&SPACE, b.clone()), Group::empty(&SPACE, b.clone()));
    merge_all(&mut b1, &ops, &l);
    merge_all(&mut b2, &ops, &l);
    // Device 1 (or the account's node) changes B's leaf; A seals after it.
    let (brot, _) = b1.rotate(&mut rng).unwrap();
    ga.merge(&brot, &l);
    let (sealed, op) = ga.seal([4; 32], b"to b", &mut rng).unwrap();
    let after: Vec<Vec<u8>> = op.into_iter().collect();
    // Control: device 2 WITHOUT that change cannot open what is sealed after it.
    let mut b2_before = Group::load(&b2.save().unwrap(), b.clone()).unwrap();
    for o in &after {
        b2_before.merge(o, &l);
    }
    assert!(b2_before.open(&sealed).is_err(), "control: without the change, device 2 cannot read");
    // Merging it, device 2 DERIVES the new leaf secret (nothing handed over) and reads.
    for g in [&mut b1, &mut b2] {
        g.merge(&brot, &l);
        for o in &after {
            g.merge(o, &l);
        }
    }
    assert_eq!(b1.open(&sealed).unwrap(), b"to b");
    assert_eq!(b2.open(&sealed).unwrap(), b"to b", "device 2 derives the account's new leaf and reads");
    // And ANOTHER account's device merging the same change derives nothing of B's: control on the derivation.
    let c = account(3);
    l.accounts.insert(id(&c), "did:c".into());
    let mut gc = Group::empty(&SPACE, c);
    for o in ops.iter().chain([&brot]).chain(after.iter()) {
        gc.merge(o, &l);
    }
    assert!(gc.open(&sealed).is_err(), "control: an account not in the group derives nothing");
}

#[test]
fn a_group_saved_and_loaded_is_the_same() {
    let mut rng = StdRng::seed_from_u64(4);
    let a = account(1);
    let (mut ga, _) = Group::create(&SPACE, a.clone()).unwrap();
    let (sealed, _) = ga.seal([5; 32], b"kept", &mut rng).unwrap();
    let mut again = Group::load(&ga.save().unwrap(), a).unwrap();
    assert_eq!(again.open(&sealed).unwrap(), b"kept");
}

#[test]
fn generations_agree_reach_history_and_end_at_a_removal() {
    let mut rng = StdRng::seed_from_u64(9);
    let (a, b, c) = (account(1), account(2), account(3));
    let mut l = list(&[(&a, "did:a"), (&b, "did:b"), (&c, "did:c")]);
    l.acts.insert([7; 32], (true, id(&b)));
    l.acts.insert([8; 32], (true, id(&c)));
    l.acts.insert([6; 32], (false, id(&b)));
    let (mut ga, first) = Group::create(&SPACE, a.clone()).unwrap();
    let add_b = ga.add_member(&id(&b), &b.leaf_key(), &[7; 32]).unwrap().unwrap();
    let (rot1, _) = ga.rotate(&mut rng).unwrap();
    let mut gb = Group::empty(&SPACE, b.clone());
    merge_all(&mut gb, &[first.clone(), add_b.clone(), rot1.clone()], &l);
    // THE SAME GENERATION, number and secret, for every member.
    let (na, ka, op) = ga.current(&mut rng).unwrap();
    assert!(op.is_none(), "a single head: no key change needed");
    let (nb, kb, _) = gb.current(&mut rng).unwrap();
    assert_eq!((na, ka), (nb, kb), "A and B write with one generation");
    // A NEWCOMER (C, added later) reaches the earlier generation too (history); control: before its add, it reaches none.
    let add_c = ga.add_member(&id(&c), &c.leaf_key(), &[8; 32]).unwrap().unwrap();
    let (rot2, _) = ga.rotate(&mut rng).unwrap();
    let mut gc = Group::empty(&SPACE, c.clone());
    merge_all(&mut gc, &[first.clone(), add_b.clone(), rot1.clone()], &l);
    assert!(gc.generations().is_empty(), "control: not added yet, nothing reached");
    merge_all(&mut gc, &[add_c, rot2.clone()], &l);
    assert!(gc.generations().iter().any(|(n, k)| (*n, *k) == (na, ka)), "C reaches the generation from before its add");
    // B REMOVED, A rotates: B does not reach the new generation; control — C does.
    gb.merge(&rot2, &l);
    let remove = ga.remove_member(&id(&b), &[6; 32]).unwrap().unwrap();
    let (rot3, _) = ga.rotate(&mut rng).unwrap();
    let (n_after, k_after, _) = ga.current(&mut rng).unwrap();
    for g in [&mut gb, &mut gc] {
        g.merge(&remove, &l);
        g.merge(&rot3, &l);
    }
    assert!(!gb.generations().iter().any(|(n, _)| *n == n_after), "removed: the generation after is not reached");
    assert!(gc.generations().contains(&(n_after, k_after)), "control: a member still in reaches it");
}

#[test]
fn a_later_generation_has_a_higher_number() {
    let mut rng = StdRng::seed_from_u64(11);
    let a = account(1);
    let (mut ga, _) = Group::create(&SPACE, a).unwrap();
    let mut last = 0;
    for _ in 0..5 {
        ga.rotate(&mut rng).unwrap();
        let (n, _, _) = ga.current(&mut rng).unwrap();
        assert!(n > last, "the newest is the highest: {n} after {last}");
        last = n;
    }
}
