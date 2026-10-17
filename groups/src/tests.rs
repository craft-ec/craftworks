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
fn our_check_refuses_an_outsiders_operation_and_an_add_no_act_allows() {
    let (a, b, x) = (account(1), account(2), account(9));
    let mut l = list(&[(&a, "did:a"), (&b, "did:b")]);
    l.acts.insert([7; 32], (true, id(&b)));
    let (mut ga, first) = Group::create(&SPACE, a.clone()).unwrap();
    // X (no listed account publishes its id) holds the operations and adds itself.
    let mut gx = Group::empty(&SPACE, x.clone());
    gx.merge(&first, &l);
    let evil = gx.add_member(&id(&x), &x.leaf_key(), &[7; 32]).unwrap().unwrap();
    assert!(matches!(ga.merge(&evil, &l), Merged::Refused(_)), "an outsider's operation");
    // A's add of B naming an act that allows something else: refused; control — the act that allows it: taken.
    let mut ga2 = Group::empty(&SPACE, a.clone());
    ga2.merge(&first, &l);
    let wrong = ga.add_member(&id(&b), &b.leaf_key(), &[8; 32]).unwrap().unwrap();
    assert!(matches!(ga2.merge(&wrong, &l), Merged::Refused(_)), "an add no act allows");
    let mut l2 = list(&[(&a, "did:a"), (&b, "did:b")]);
    l2.acts.insert([8; 32], (true, id(&b)));
    let mut ga3 = Group::empty(&SPACE, a.clone());
    ga3.merge(&first, &l2);
    assert_eq!(ga3.merge(&wrong, &l2), Merged::Applied, "control: with the act that allows it");
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
fn an_accounts_devices_are_one_member_and_share_a_new_leaf_through_the_account() {
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
    // Device 1 rotates B's leaf; A seals after it.
    let (brot, new_leaf) = b1.rotate(&mut rng).unwrap();
    ga.merge(&brot, &l);
    b2.merge(&brot, &l);
    let (sealed, op) = ga.seal([4; 32], b"to b", &mut rng).unwrap();
    for g in [&mut b1, &mut b2] {
        if let Some(op) = &op {
            g.merge(op, &l);
        }
    }
    assert_eq!(b1.open(&sealed).unwrap(), b"to b");
    assert!(b2.open(&sealed).is_err(), "control: device 2 without the account's new leaf");
    b2.take_leaf(new_leaf);
    assert_eq!(b2.open(&sealed).unwrap(), b"to b", "handed through the account: it reads");
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
