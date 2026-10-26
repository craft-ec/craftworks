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
    creator: Option<String>,
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
    fn creator(&self) -> Option<String> {
        self.creator.clone()
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
    // (The first listed made the tree.)
    l.creator = of.first().map(|(_, d)| d.to_string());
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

/// MEASURED (10-18, the v401 delegate's budget): what a node round does for one space — the tree rebuilt from every
/// operation (`Group::empty` + merge all), then every generation derived — at sizes of the live spaces.
#[test]
#[ignore]
fn cost_of_a_rebuild_round() {
    use std::time::Instant;
    for &(n, rounds) in &[(10usize, 3usize), (30, 3), (60, 3), (60, 10)] {
        let mut rng = StdRng::seed_from_u64(n as u64);
        let people: Vec<Account> = (0..n).map(|i| account(i as u8 + 1)).collect();
        let mut l = List::default();
        for (i, p) in people.iter().enumerate() {
            l.accounts.insert(id(p), format!("did:{i}"));
            l.listed.insert(format!("did:{i}"));
        }
        let (mut g0, first) = Group::create(&SPACE, people[0].clone()).unwrap();
        let mut ops = vec![first];
        for p in &people[1..] {
            ops.push(g0.add_member(&id(p), &p.leaf_key(), &[0; 32]).unwrap().unwrap());
        }
        // Rotations by members in turn (each reconcile on a page makes one).
        let mut reps: Vec<Group> = people.iter().map(|p| Group::empty(&SPACE, p.clone())).collect();
        for _ in 0..rounds {
            for (i, r) in reps.iter_mut().enumerate().take(n.min(8)) {
                for op in &ops {
                    r.merge(op, &l);
                }
                if r.placed() {
                    ops.push(r.rotate(&mut rng).unwrap().0);
                }
                let _ = i;
            }
        }
        let t = Instant::now();
        let mut g = Group::empty(&SPACE, people[0].clone());
        let mut pending = ops.clone();
        for _ in 0..64 {
            let before = pending.len();
            pending.retain(|op| g.merge(op, &l) == Merged::Waiting);
            if pending.is_empty() || pending.len() == before {
                break;
            }
        }
        let merged = t.elapsed();
        let gens = g.generations().len();
        let total = t.elapsed();
        println!("{n} members, {} ops: rebuild {:?}, + {gens} generations {:?} (native)", ops.len(), merged, total);
    }
}

/// 10-19, live: TWO TREES in one space — a listed member's page made one (it read no operations yet) while the owner's
/// made another; the owner's operations waited forever on a root that lost, and nobody was ever added. Only the
/// space's CREATOR (its owner) makes the root: another member's root is refused by every reader, the owner's tree is
/// the one every reader holds, and the owner adds the other maker into it. Control: with the other maker as creator,
/// the owner's root is the one refused — the rule decides, not the order the operations arrive in.
#[test]
fn only_the_creators_root_is_taken_two_makers_one_tree() {
    let mut rng = StdRng::seed_from_u64(9);
    let (owner, member, reader) = (account(11), account(12), account(13));
    let l = list(&[(&owner, "did:owner"), (&member, "did:member"), (&reader, "did:reader")]);
    // Both make a tree, each seeing none.
    let (mut go, owners_root) = Group::create(&SPACE, owner.clone()).unwrap();
    let (_gm, members_root) = Group::create_by(&SPACE, member.clone(), &[0; 32]).unwrap();
    // The member's root arrives FIRST at the reader.
    let mut gr = Group::empty(&SPACE, reader.clone());
    assert!(matches!(gr.merge(&members_root, &l), Merged::Refused(ref w) if w.contains("not the space's creator")), "the member's root refused");
    assert_eq!(gr.merge(&owners_root, &l), Merged::Applied, "the owner's root taken");
    // The owner adds both; every reader is on the owner's tree.
    let add_m = go.add_member(&id(&member), &member.leaf_key(), &[0; 32]).unwrap().unwrap();
    let add_r = go.add_member(&id(&reader), &reader.leaf_key(), &[0; 32]).unwrap().unwrap();
    let (rot, _) = go.rotate(&mut rng).unwrap();
    for op in [&add_m, &add_r, &rot] {
        assert_eq!(gr.merge(op, &l), Merged::Applied);
    }
    assert!(gr.placed(), "the reader is placed in the owner's tree");
    let mut gm2 = Group::empty(&SPACE, member.clone());
    assert!(matches!(gm2.merge(&members_root, &l), Merged::Refused(_)), "the member's own page refuses its root too");
    for op in [&owners_root, &add_m, &add_r, &rot] {
        assert_eq!(gm2.merge(op, &l), Merged::Applied);
    }
    assert!(gm2.placed(), "the other maker is placed in the owner's tree");
    assert_eq!(gr.current_now().map(|x| x.0), gm2.current_now().map(|x| x.0), "one tree: the same generation for both");

    // CONTROL: the member as creator — the owner's root is the one refused.
    let mut lc = list(&[(&member, "did:member"), (&owner, "did:owner"), (&reader, "did:reader")]);
    lc.creator = Some("did:member".into());
    let mut gc = Group::empty(&SPACE, reader.clone());
    assert!(matches!(gc.merge(&owners_root, &lc), Merged::Refused(_)), "control: the owner's root refused when it is not the creator");
    assert_eq!(gc.merge(&members_root, &lc), Merged::Applied);
}

/// OUR CHECK FROM THE LIST (`ListCheck`, page and node): an add allowed by the act that names the member's account; the
/// owner's own placement by no act; the creator the owner (else the lowest listed); a refusal kept with what it saw.
#[test]
fn list_check_allows_by_the_act_and_keeps_what_a_refusal_saw() {
    let (a, b) = ([1u8; 32], [2u8; 32]);
    let mut l = ListCheck { owner: "did:o".into(), listed: vec!["did:b".into()], ..ListCheck::default() };
    l.by_member.insert(a, "did:o".into());
    l.by_member.insert(b, "did:b".into());
    l.acts.insert([7; 32], (true, "did:b".into()));
    assert!(l.allows(&[7; 32], true, &b), "the act letting B in");
    assert!(!l.allows(&[7; 32], false, &b), "not a removal");
    assert!(l.allows(&[0; 32], true, &a), "the owner's own placement, by no act");
    assert!(!l.allows(&[0; 32], true, &b), "nobody else's");
    assert_eq!(l.misses.borrow().len(), 2);
    assert_eq!(l.creator().as_deref(), Some("did:o"));
    assert_eq!(ListCheck { listed: vec!["did:z".into(), "did:c".into()], ..ListCheck::default() }.creator().as_deref(), Some("did:c"));
}

#[test]
fn a_held_operation_no_feed_holds_is_handed_back_and_places_the_member() {
    let (a, b) = (account(1), account(2));
    let mut l = list(&[(&a, "did:a"), (&b, "did:b")]);
    l.acts.insert([7; 32], (true, id(&b)));
    let (mut ga, root) = Group::create(&SPACE, a.clone()).unwrap();
    let add = ga.add_member(&id(&b), &b.leaf_key(), &[7; 32]).unwrap().unwrap();
    // THE NETWORK lost the root: only the add is published — B waits on it forever.
    let mut gb = Group::empty(&SPACE, b.clone());
    assert_eq!(gb.merge_all(&[add.clone()], &l).waiting, 1);
    // The maker's tree holds it: handed back (control — published whole, nothing is).
    let back = ga.unpublished(&[add.clone()], &l);
    assert_eq!(back, vec![root.clone()]);
    assert!(ga.unpublished(&[root.clone(), add.clone()], &l).is_empty());
    let all: Vec<Vec<u8>> = back.into_iter().chain([add]).collect();
    assert_eq!(gb.merge_all(&all, &l).waiting, 0);
    assert!(gb.placed());
    // An operation our check refuses is never handed back: the root's maker taken off the list.
    let mut off = list(&[(&b, "did:b")]);
    off.accounts.insert(id(&a), "did:a".into());
    assert!(ga.unpublished(&[], &off).is_empty());
}


/// K8.5 THE ACCOUNT's CHECK: the account's tree's members are its devices, the devices list its word. A current device
/// is added and reads; a device not current is refused; one no longer current is taken out and does not read what is
/// sealed after; the root is the lowest current device that wrote. Each with its control.
#[test]
fn the_accounts_tree_follows_its_devices_list() {
    let mut rng = StdRng::seed_from_u64(9);
    let (d1, d2, d3) = (account(0x31), account(0x32), account(0x33));
    let mut chk = AccountCheck { current: vec![id(&d1), id(&d2)], writers: vec![id(&d1)] };
    // The ROOT: d1, current and a writer.
    let (mut g1, root) = Group::create(&SPACE, d1.clone()).unwrap();
    let mut g2 = Group::empty(&SPACE, d2.clone());
    assert_eq!(g2.merge(&root, &chk), Merged::Applied, "d1's root taken");
    // CONTROL: the same root, d1 not a writer and d2 the only one — refused (one maker).
    let other = AccountCheck { current: vec![id(&d1), id(&d2)], writers: vec![id(&d2)] };
    assert!(matches!(Group::empty(&SPACE, d3.clone()).merge(&root, &other), Merged::Refused(_)), "not the creator's root");
    // A CURRENT device added: placed, reads what d1 seals.
    let add = g1.add_member(&id(&d2), &d2.leaf_key(), &[0; 32]).unwrap().expect("an add");
    assert_eq!(g2.merge(&add, &chk), Merged::Applied);
    assert!(g2.placed(), "d2 placed");
    let (sealed, op) = g1.seal([1; 32], b"the account's row", &mut rng).unwrap();
    if let Some(op) = op {
        g2.merge(&op, &chk);
    }
    assert_eq!(g2.open(&sealed).unwrap(), b"the account's row");
    // A device NOT current: its add refused.
    // (Made on a copy of d1's tree: d1 itself goes on without it.)
    let add3 = Group::load(&g1.save().unwrap(), d1.clone()).unwrap().add_member(&id(&d3), &d3.leaf_key(), &[0; 32]).unwrap().expect("an add");
    assert!(matches!(g2.merge(&add3, &chk), Merged::Refused(_)), "d3 is not a current device");
    // CONTROL: d3 current — the same add taken.
    let with3 = AccountCheck { current: vec![id(&d1), id(&d2), id(&d3)], writers: vec![id(&d1)] };
    assert_eq!(Group::load(&g2.save().unwrap(), d2.clone()).unwrap().merge(&add3, &with3), Merged::Applied);
    // d2 REMOVED (the key log rotated, d2 not credentialed again): taken out; reads nothing sealed after.
    chk.current = vec![id(&d1)];
    let out = g1.remove_member(&id(&d2), &[0; 32]).unwrap().expect("a removal");
    // CONTROL: d2 still current — the removal refused.
    let still = AccountCheck { current: vec![id(&d1), id(&d2)], writers: vec![id(&d1)] };
    assert!(matches!(Group::load(&g2.save().unwrap(), d2.clone()).unwrap().merge(&out, &still), Merged::Refused(_)), "a current device is not taken out");
    assert_eq!(g2.merge(&out, &chk), Merged::Applied);
    let (after, op) = g1.seal([2; 32], b"after the removal", &mut rng).unwrap();
    if let Some(op) = op {
        let _ = g2.merge(&op, &chk);
    }
    assert!(g2.open(&after).is_err(), "the removed device reads nothing sealed after");
}

/// K8.5 A DEVICE's LEAF is taken only signed by its member: a leaf signed by another member, or changed, is refused.
#[test]
fn a_devices_leaf_is_taken_only_signed_by_it() {
    let (d1, d2) = (account(0x41), account(0x42));
    let (leaf, sig) = d1.signed_leaf();
    assert!(leaf_checks(&id(&d1), &leaf, &sig), "its own, signed");
    assert_eq!(leaf, d1.leaf_key().to_bytes(), "the leaf an add encrypts to");
    // CONTROLS: another member's signature; a changed leaf.
    let (_, other) = d2.signed_leaf();
    assert!(!leaf_checks(&id(&d1), &leaf, &other), "signed by another member");
    let mut changed = leaf;
    changed[0] ^= 1;
    assert!(!leaf_checks(&id(&d1), &changed, &sig), "a leaf put in its place");
}

