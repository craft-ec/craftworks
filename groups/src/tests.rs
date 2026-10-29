//! The door to BeeKEM: each property with its control.
use super::*;
use rand::{rngs::StdRng, SeedableRng};
use std::collections::{BTreeMap, BTreeSet};

const SPACE: [u8; 32] = [0x5a; 32];

/// The list, as a test sets it: member id → account, listed accounts, acts → (add?, target).
#[derive(Default)]
struct List {
    accounts: BTreeMap<[u8; 32], String>,
    leaves: BTreeMap<[u8; 32], [u8; 32]>,
    listed: BTreeSet<String>,
    acts: BTreeMap<[u8; 32], (bool, [u8; 32])>,
    creator: Option<String>,
}
impl List {
    fn act(&mut self, h: [u8; 32], mv: (bool, [u8; 32])) {
        self.acts.insert(h, mv);
    }
}
impl Check for List {
    /// As the real list: listed, or let in by an act on record.
    fn ever_in(&self, did: &str) -> bool {
        self.listed.contains(did) || self.acts.values().any(|(i, t)| *i && self.accounts.get(t).is_some_and(|d| d == did))
    }
    fn account_of(&self, member: &[u8; 32]) -> Option<String> {
        self.accounts.get(member).cloned()
    }
    fn listed(&self, did: &str) -> bool {
        self.listed.contains(did)
    }
    /// As the real list: an act names an ACCOUNT — any of its members (a replaced one's successor too) it lets in.
    fn allows(&self, act: &[u8; 32], add: bool, target: &[u8; 32]) -> bool {
        self.acts.get(act).is_some_and(|(i, t)| *i == add && (t == target || self.accounts.get(t).is_some_and(|d| self.accounts.get(target) == Some(d))))
    }
    fn creator(&self) -> Option<String> {
        self.creator.clone()
    }
    fn leaf_of(&self, member: &[u8; 32]) -> Option<[u8; 32]> {
        self.leaves.get(member).copied()
    }
}

fn account(n: u8) -> Account {
    Account::of(&[n; 32])
}
fn id(a: &Account) -> [u8; 32] {
    a.id.0.to_bytes()
}
/// Each account's published leaf (its first: what a card names), keyed by its member id.
fn lv(of: &[&Account]) -> BTreeMap<[u8; 32], [u8; 32]> {
    of.iter().map(|a| (id(a), a.leaf_key().to_bytes())).collect()
}
/// The act letting an account in (each listed one has its own, as the real roster's `member`/`join` acts).
fn act_in(a: &Account) -> [u8; 32] {
    let mut x = id(a);
    x[31] ^= 0xAC;
    x
}
fn list(of: &[(&Account, &str)]) -> List {
    let mut l = List::default();
    for (a, did) in of {
        l.accounts.insert(id(a), did.to_string());
        l.leaves.insert(id(a), a.leaf_key().to_bytes());
        l.listed.insert(did.to_string());
        l.act(act_in(a), (true, id(a)));
        // (The maker's own placement names no act: allowed for the first member.)
        l.acts.insert([0; 32], (true, id(of[0].0)));
    }
    // (The first listed made the tree.)
    l.creator = of.first().map(|(_, d)| d.to_string());
    l
}
/// NOT WRITTEN FOR (FOUNDATION "WHO RECEIVES IS DECIDED BY THE ONE WHO SEALS"): `honest`, having taken `rogue_op`
/// (admission judges fixed facts only), places `rogue` — and is NOT written with (`writable`: none) until it takes it out
/// (`out_act`) and changes the key; then it is, and `rogue` (the tree as it stood, merging everything after) opens
/// nothing sealed with it. CONTROL: sealed with the tree as it stands while it places the rogue, the rogue reads it —
/// the reason it is not written with.
fn not_written_for(honest: &mut Group, check: &impl Check, rogue_op: &[u8], rogue: &Account, out_act: [u8; 32], seed: u64) {
    let mut rng = StdRng::seed_from_u64(seed);
    assert_eq!(honest.merge(rogue_op, check), Merged::Applied, "taken: admission judges fixed facts only");
    assert!(honest.members().contains(&id(rogue)), "the rogue placed");
    // (A key change made as it stands: a CURRENT generation exists — the precondition that makes `writable` decide.)
    let (rot0, _) = honest.rotate(&mut rng).unwrap();
    assert!(honest.current_now().is_some(), "a current generation exists");
    assert!(honest.writable(check).is_none(), "placing one the list puts out: not written with");
    let mut seen = Group::load(&honest.save().unwrap(), rogue.clone()).unwrap();
    // CONTROL: written with as it stands — the rogue would read it.
    let (leak, op) = Group::load(&honest.save().unwrap(), honest.account.clone()).unwrap().seal([9; 32], b"as it stands", &mut rng).unwrap();
    let mut seen_c = Group::load(&seen.save().unwrap(), rogue.clone()).unwrap();
    for o in [Some(rot0), op].into_iter().flatten() {
        seen_c.merge(&o, check);
    }
    assert_eq!(seen_c.open(&leak).as_deref().ok(), Some(&b"as it stands"[..]), "control: a tree placing the rogue, written with, reaches it");
    // TAKEN OUT, the key changed: written with again — the rogue opens nothing.
    let out = honest.remove_member(&id(rogue), &out_act).unwrap().expect("its removal");
    let (rot, _) = honest.rotate(&mut rng).unwrap();
    let (n, _) = honest.writable(check).expect("clean: written with");
    let (sealed, op) = honest.seal([8; 32], b"after", &mut rng).unwrap();
    assert!(honest.current_now().is_some_and(|(m, _)| m == n), "sealed with the writable generation");
    for o in [Some(out), Some(rot), op].into_iter().flatten() {
        seen.merge(&o, check);
    }
    assert!(seen.open(&sealed).is_err(), "the rogue opens nothing written after");
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
    l.act([7; 32], (true, id(&b)));
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
    let add_c = ga.add_member(&id(&c), &c.leaf_key(), &act_in(&c)).unwrap().unwrap();
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
    l.act([7; 32], (true, id(&b)));
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
    l.act([7; 32], (true, id(&b)));
    l.act([6; 32], (false, id(&b)));
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
    l.act([7; 32], (true, id(&b)));
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
    l.act([7; 32], (true, id(&b)));
    l.act([8; 32], (true, id(&c)));
    l.act([6; 32], (false, id(&b)));
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
            ops.push(g0.add_member(&id(p), &p.leaf_key(), &act_in(&p)).unwrap().unwrap());
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
    let add_m = go.add_member(&id(&member), &member.leaf_key(), &act_in(&member)).unwrap().unwrap();
    let add_r = go.add_member(&id(&reader), &reader.leaf_key(), &act_in(&reader)).unwrap().unwrap();
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
    l.act([7; 32], (true, id(&b)));
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
    let mut chk = AccountCheck { ever: vec![], current: vec![id(&d1), id(&d2)], writers: vec![id(&d1)], leaves: lv(&[&d1, &d2]) };
    // The ROOT: d1, current and a writer.
    let (mut g1, root) = Group::create(&SPACE, d1.clone()).unwrap();
    let mut g2 = Group::empty(&SPACE, d2.clone());
    assert_eq!(g2.merge(&root, &chk), Merged::Applied, "d1's root taken");
    // CONTROL: the same root, d1 not a writer and d2 the only one — refused (one maker).
    let other = AccountCheck { ever: vec![], current: vec![id(&d1), id(&d2)], writers: vec![id(&d2)], leaves: lv(&[&d1, &d2]) };
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
    let with3 = AccountCheck { ever: vec![], current: vec![id(&d1), id(&d2), id(&d3)], writers: vec![id(&d1)], leaves: lv(&[&d1, &d2, &d3]) };
    assert_eq!(Group::load(&g2.save().unwrap(), d2.clone()).unwrap().merge(&add3, &with3), Merged::Applied);
    // d2 REMOVED (the key log rotated, d2 not credentialed again): taken out; reads nothing sealed after.
    chk.current = vec![id(&d1)];
    let out = g1.remove_member(&id(&d2), &[0; 32]).unwrap().expect("a removal");
    // CONTROL: d2 still current — the removal refused.
    let still = AccountCheck { ever: vec![], current: vec![id(&d1), id(&d2)], writers: vec![id(&d1)], leaves: lv(&[&d1, &d2]) };
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

/// K8.5 A REMOVED DEVICE's HISTORY STILL COUNTS: d1 made the root and added d2; d1 is removed (no longer current, but
/// ever the account's) — a fresh tree of d2 still takes the root and the add, and d2 takes d1 out. Control: judged by
/// the current list alone, the root is refused (what fx/fy showed 10-26).
#[test]
fn a_removed_devices_history_still_counts() {
    let (d1, d2) = (account(0x61), account(0x62));
    let both = AccountCheck { current: vec![id(&d1), id(&d2)], ever: vec![id(&d1), id(&d2)], writers: vec![id(&d1)], leaves: lv(&[&d1, &d2]) };
    let (mut g1, root) = Group::create(&SPACE, d1.clone()).unwrap();
    let add = g1.add_member(&id(&d2), &d2.leaf_key(), &[0; 32]).unwrap().expect("an add");
    // d1 REMOVED: d2 alone current.
    let after = AccountCheck { current: vec![id(&d2)], ever: vec![id(&d1), id(&d2)], writers: vec![id(&d1)], leaves: lv(&[&d2, &d1]) };
    // (d2 merged them while d1 was current.)
    let mut g2 = Group::empty(&SPACE, d2.clone());
    assert_eq!(g2.merge(&root, &both), Merged::Applied, "the root");
    assert_eq!(g2.merge(&add, &both), Merged::Applied, "its add of d2");
    assert!(g2.placed());
    let out = g2.remove_member(&id(&d1), &[0; 32]).unwrap().expect("a removal");
    assert_eq!(Group::load(&g2.save().unwrap(), d2.clone()).unwrap().merge(&out, &after), Merged::Applied, "d2 takes d1 out");
    // A FRESH MERGE after it: d1's root and add are HISTORY — taken with d1's removal on record.
    let fresh = Group::empty(&SPACE, account(0x3E)).merge_all(&[out.clone(), add.clone(), root.clone()], &after);
    assert!(fresh.refused.is_empty() && fresh.waiting == 0, "a removed device's history, its removal on record: {fresh:?}");
    // …its root alone too, with no removal of it held: history is judged by fixed facts, never by what else is held.
    assert_eq!(Group::empty(&SPACE, account(0x3D)).merge(&root, &after), Merged::Applied, "its root, alone");
    // A REMOVED DEVICE ADDING ITSELF BACK (its own view, its removal merged — 10-28 review C1): taken, and never
    // written for.
    let mut g1b = Group::empty(&SPACE, d1.clone());
    for op in [&root, &add] { let _ = g1b.merge(op, &both); }
    assert_eq!(g1b.merge(&out, &after), Merged::Applied, "its removal, judged after it");
    assert!(!g1b.placed(), "d1, its removal merged, is out");
    let back = g1b.add_member(&id(&d1), &d1.leaf_key(), &[0; 32]).unwrap().expect("its own add");
    let mut g2b = Group::load(&g2.save().unwrap(), d2.clone()).unwrap();
    g2b.merge(&out, &after);
    not_written_for(&mut g2b, &after, &back, &d1, [0; 32], 61);
    // CONTROL: the current list alone — the root refused.
    let current_only = AccountCheck { current: vec![id(&d2)], ever: vec![], writers: vec![id(&d1)], leaves: lv(&[&d2, &d1]) };
    assert!(matches!(Group::empty(&SPACE, d2.clone()).merge(&root, &current_only), Merged::Refused(_)), "judged by now alone");
}


/// K8.6 A DEVICE ENTERS THE ACCOUNT's TREE BY ITSELF (a words login, every placed device gone): current, unplaced, it adds
/// its own leaf and changes the key; the old device's tree takes both, and reads what the new one seals.
#[test]
fn a_current_device_enters_the_accounts_tree_by_itself() {
    let mut rng = StdRng::seed_from_u64(11);
    let (d1, d3) = (account(0x41), account(0x43));
    let chk = AccountCheck { ever: vec![], current: vec![id(&d1), id(&d3)], writers: vec![id(&d1)], leaves: lv(&[&d1, &d3]) };
    let (mut g1, root) = Group::create(&SPACE, d1.clone()).unwrap();
    let mut g3 = Group::empty(&SPACE, d3.clone());
    assert_eq!(g3.merge(&root, &chk), Merged::Applied);
    assert!(!g3.placed(), "control: not placed before");
    let add = g3.add_member(&id(&d3), &d3.leaf_key(), &[0; 32]).unwrap().expect("its own add");
    let (rot, _) = g3.rotate(&mut rng).unwrap();
    assert!(g3.placed(), "placed by itself");
    assert_eq!(g1.merge(&add, &chk), Merged::Applied, "its add taken: it is current");
    assert_eq!(g1.merge(&rot, &chk), Merged::Applied, "its key change taken");
    let (sealed, op) = g3.seal([3; 32], b"written by the new device", &mut rng).unwrap();
    if let Some(op) = op {
        g1.merge(&op, &chk);
    }
    assert_eq!(g1.open(&sealed).unwrap(), b"written by the new device");
    // CONTROL: a device NOT current cannot enter.
    let (d4, not) = (account(0x44), AccountCheck { ever: vec![], current: vec![id(&d1)], writers: vec![id(&d1)], leaves: lv(&[&d1]) });
    let mut g4 = Group::empty(&SPACE, d4.clone());
    g4.merge(&root, &not);
    let add4 = g4.add_member(&id(&d4), &d4.leaf_key(), &[0; 32]).unwrap().expect("an add");
    assert!(matches!(Group::load(&g1.save().unwrap(), d1.clone()).unwrap().merge(&add4, &not), Merged::Refused(_)), "not current: refused");
}

/// K8.6 THE RACE: a placed device adds a device while it enters by itself — two adds of one member, concurrent. Both
/// trees take both, the new device is placed, and each reads what the other seals.
#[test]
fn a_device_added_while_it_enters_by_itself_converges() {
    let mut rng = StdRng::seed_from_u64(12);
    let (d1, d3) = (account(0x51), account(0x53));
    let chk = AccountCheck { ever: vec![], current: vec![id(&d1), id(&d3)], writers: vec![id(&d1)], leaves: lv(&[&d1, &d3]) };
    let (mut g1, root) = Group::create(&SPACE, d1.clone()).unwrap();
    let mut g3 = Group::empty(&SPACE, d3.clone());
    g3.merge(&root, &chk);
    let by1 = g1.add_member(&id(&d3), &d3.leaf_key(), &[0; 32]).unwrap().expect("d1's add");
    let self_add = g3.add_member(&id(&d3), &d3.leaf_key(), &[0; 32]).unwrap().expect("its own add");
    let (rot, _) = g3.rotate(&mut rng).unwrap();
    let a = g1.merge_all(&[self_add.clone(), rot.clone()], &chk);
    let b = g3.merge_all(&[by1.clone()], &chk);
    eprintln!("d1 took {a:?}; d3 took {b:?}; members d1 {} d3 {}", g1.members().len(), g3.members().len());
    assert!(g3.placed());
    let (s3, op3) = g3.seal([4; 32], b"from d3", &mut rng).unwrap();
    if let Some(op) = op3 {
        g1.merge(&op, &chk);
    }
    assert_eq!(g1.open(&s3).unwrap(), b"from d3", "d1 reads d3");
    let (s1, op1) = g1.seal([5; 32], b"from d1", &mut rng).unwrap();
    if let Some(op) = op1 {
        g3.merge(&op, &chk);
    }
    assert_eq!(g3.open(&s1).unwrap(), b"from d1", "d3 reads d1");
}

/// K8.6 KEY OPERATIONS AUTHENTICATED: an operation is taken only SIGNED BY THE MEMBER IT NAMES — the same operation with
/// another member named as its issuer (what a device that can write where we read could forge) is refused. CONTROL: the
/// genuine one merges; and one already held is not judged again.
#[test]
fn an_operation_signed_by_another_than_it_names_is_refused() {
    let (d1, d2) = (account(0x61), account(0x62));
    let chk = AccountCheck { ever: vec![], current: vec![id(&d1), id(&d2)], writers: vec![id(&d1)], leaves: lv(&[&d1, &d2]) };
    let (mut g1, root) = Group::create(&SPACE, d1.clone()).unwrap();
    let add = g1.add_member(&id(&d2), &d2.leaf_key(), &[0; 32]).unwrap().expect("d1's add");
    // FORGED: d1's add, named as d2's.
    let mut forged: Signed<CgkaOperation> = bincode::deserialize(&add).unwrap();
    forged.issuer = ed25519_dalek::VerifyingKey::from_bytes(&id(&d2)).unwrap();
    let forged = bincode::serialize(&forged).unwrap();
    let mut g = Group::empty(&SPACE, account(0x63));
    assert_eq!(g.merge(&root, &chk), Merged::Applied);
    assert!(matches!(g.merge(&forged, &chk), Merged::Refused(w) if w.contains("signature")), "forged: refused");
    // CONTROL: the genuine one merges — and again, held, it is simply taken.
    assert_eq!(g.merge(&add, &chk), Merged::Applied, "genuine: merged");
    assert_eq!(g.merge(&add, &chk), Merged::Applied, "held: taken again without a second judgement");
}

/// BEEKEM's OWN RULE, measured (K8.7): a member REMOVED while it made a key change CONCURRENTLY (the same predecessors)
/// reads nothing sealed after both are merged — on the remover's tree and on the removed one's. CONTROL: without the
/// removal, the same key change leaves it reading.
#[test]
fn a_key_change_concurrent_with_its_own_removal_reads_nothing_after() {
    let mut rng = StdRng::seed_from_u64(21);
    let (a, b) = (account(0x71), account(0x72));
    let chk = AccountCheck { ever: vec![], current: vec![id(&a), id(&b)], writers: vec![id(&a)], leaves: lv(&[&a, &b]) };
    let out = AccountCheck { ever: vec![id(&b)], current: vec![id(&a)], writers: vec![id(&a)], leaves: lv(&[&b, &a]) };
    let (mut ga, root) = Group::create(&SPACE, a.clone()).unwrap();
    let add = ga.add_member(&id(&b), &b.leaf_key(), &act_in(&b)).unwrap().unwrap();
    let (rot_a, _) = ga.rotate(&mut rng).unwrap();
    let mut gb = Group::empty(&SPACE, b.clone());
    gb.merge_all(&[root.clone(), add.clone(), rot_a.clone()], &chk);
    assert!(gb.placed());
    // CONTROL: b's key change alone — b still reads.
    let mut ca = Group::load(&ga.save().unwrap(), a.clone()).unwrap();
    let mut cb = Group::load(&gb.save().unwrap(), b.clone()).unwrap();
    let (cu, _) = cb.rotate(&mut rng).unwrap();
    ca.merge(&cu, &chk);
    let (s, op) = ca.seal([1; 32], b"control", &mut rng).unwrap();
    if let Some(op) = op {
        cb.merge(&op, &chk);
    }
    assert_eq!(cb.open(&s).unwrap(), b"control", "control: a key change alone leaves b reading");
    // CONCURRENT: a removes b and changes the key; b changes its key from the same heads.
    let rm = ga.remove_member(&id(&b), &[0; 32]).unwrap().unwrap();
    let (rot2, _) = ga.rotate(&mut rng).unwrap();
    let (bu, _) = gb.rotate(&mut rng).unwrap();
    let ma = ga.merge(&bu, &out);
    let mb = gb.merge_all(&[rm.clone(), rot2.clone()], &out);
    eprintln!("a took b's concurrent key change: {ma:?}; b took the removal: {mb:?}");
    let (sealed, op) = ga.seal([2; 32], b"after the removal", &mut rng).unwrap();
    if let Some(op) = op {
        let _ = gb.merge(&op, &out);
    }
    assert!(gb.open(&sealed).is_err(), "b reads nothing sealed after its removal, its concurrent key change merged");
}

/// H3 AN ADD CARRIES THE LEAF ITS MEMBER PUBLISHED: an add of b with a leaf the adder chose is refused (else the adder,
/// once removed, reads on through b's leaf); one of a member whose leaf is not known is refused too. CONTROL: b's own
/// published leaf — taken.
#[test]
fn an_add_with_a_leaf_its_member_did_not_publish_is_refused() {
    let (a, b, c) = (account(0x81), account(0x82), account(0x83));
    let l = list(&[(&a, "did:a"), (&b, "did:b")]);
    let (mut ga, root) = Group::create(&SPACE, a.clone()).unwrap();
    let planted = Group::load(&ga.save().unwrap(), a.clone()).unwrap().add_member(&id(&b), &c.leaf_key(), &act_in(&b)).unwrap().unwrap();
    let mut g = Group::empty(&SPACE, account(0x84));
    assert_eq!(g.merge(&root, &l), Merged::Applied);
    assert!(matches!(g.merge(&planted, &l), Merged::Refused(w) if w.contains("did not publish")), "a planted leaf: refused");
    let mut unknown = list(&[(&a, "did:a"), (&b, "did:b")]);
    unknown.leaves.remove(&id(&b));
    let real = ga.add_member(&id(&b), &b.leaf_key(), &act_in(&b)).unwrap().unwrap();
    assert_eq!(Group::load(&g.save().unwrap(), account(0x84)).unwrap().merge(&real, &unknown), Merged::Waiting, "a leaf not known yet: it waits");
    assert_eq!(g.merge(&real, &l), Merged::Applied, "control: its published leaf — taken");
}

/// K8.7 A MEMBER ITS ACCOUNT REPLACED: taken out with NO act (`retire_superseded`), its earlier operations still its
/// account's (a fresh merge keeps them); out, it reads nothing sealed after. CONTROL: not marked superseded — the same
/// removal with no act is refused.
#[test]
fn a_superseded_member_is_retired_with_no_act_and_keeps_its_history() {
    let mut rng = StdRng::seed_from_u64(31);
    let (a, old, new) = (account(0x91), account(0x92), account(0x93));
    let mut l = list(&[(&a, "did:a"), (&old, "did:b")]);
    let (mut ga, root) = Group::create(&SPACE, a.clone()).unwrap();
    let add = ga.add_member(&id(&old), &old.leaf_key(), &act_in(&old)).unwrap().unwrap();
    let mut gold = Group::empty(&SPACE, old.clone());
    gold.merge_all(&[root.clone(), add.clone()], &l);
    let (old_rot, _) = gold.rotate(&mut rng).unwrap();
    ga.merge(&old_rot, &l);
    // b's account REPLACES its member: the card names `new`; `old` superseded.
    l.accounts.insert(id(&new), "did:b".into());
    l.leaves.insert(id(&new), new.leaf_key().to_bytes());
    let control = Group::load(&ga.save().unwrap(), a.clone()).unwrap().remove_member(&id(&old), &[0; 32]).unwrap().unwrap();
    assert!(matches!(Group::load(&ga.save().unwrap(), a.clone()).unwrap().merge(&control, &l), Merged::Refused(_)), "control: not superseded — no act, refused");
    let mut sup = SupList(l, BTreeSet::new());
    sup.1.insert(id(&old));
    let ops = ga.retire_superseded(&[id(&old)], [5; 32]).unwrap();
    assert_eq!(ops.len(), 2, "its removal and a key change");
    let add_new = ga.add_member(&id(&new), &new.leaf_key(), &act_in(&old)).unwrap().unwrap();
    // A FRESH MERGE of everything: its history still authored, the retirement taken.
    let mut fresh = Group::empty(&SPACE, new.clone());
    let all: Vec<Vec<u8>> = [root, add, old_rot].into_iter().chain(ops.clone()).chain([add_new]).collect();
    let r = fresh.merge_all(&all, &sup);
    assert!(r.refused.is_empty() && r.waiting == 0, "all taken: {r:?}");
    assert!(fresh.placed(), "the new member placed");
    // OUT: the old member reads nothing sealed after.
    gold.merge_all(&ops, &sup);
    let (sealed, op) = ga.seal([6; 32], b"after", &mut rng).unwrap();
    if let Some(op) = op {
        let _ = gold.merge(&op, &sup);
    }
    assert!(gold.open(&sealed).is_err(), "the replaced member reads nothing after");
}
/// A test check with members marked superseded.
struct SupList(List, BTreeSet<[u8; 32]>);
impl Check for SupList {
    fn account_of(&self, m: &[u8; 32]) -> Option<String> {
        self.0.account_of(m)
    }
    fn listed(&self, d: &str) -> bool {
        self.0.listed(d)
    }
    fn allows(&self, act: &[u8; 32], add: bool, t: &[u8; 32]) -> bool {
        self.0.allows(act, add, t)
    }
    fn creator(&self) -> Option<String> {
        self.0.creator()
    }
    fn leaf_of(&self, m: &[u8; 32]) -> Option<[u8; 32]> {
        self.0.leaf_of(m)
    }
    fn superseded(&self, m: &[u8; 32]) -> bool {
        self.1.contains(m)
    }
}

/// K8.7 A TREE SAVED BY THE ACCOUNT's OLD MEMBER, loaded by its NEW one: the operations kept, the new member NOT placed
/// (the old one's keys are not its own), and it enters by itself — reading what is sealed after. CONTROL: loaded by the
/// same member, it is placed as saved.
#[test]
fn a_tree_saved_by_the_old_member_loads_for_the_new_one_unplaced() {
    let mut rng = StdRng::seed_from_u64(41);
    let (a, old, new) = (account(0xA1), account(0xA2), account(0xA3));
    let l = list(&[(&a, "did:a"), (&old, "did:b"), (&new, "did:b")]);
    let (mut ga, root) = Group::create(&SPACE, a.clone()).unwrap();
    let add = ga.add_member(&id(&old), &old.leaf_key(), &act_in(&old)).unwrap().unwrap();
    let mut gold = Group::empty(&SPACE, old.clone());
    gold.merge_all(&[root, add], &l);
    assert!(gold.placed());
    let saved = gold.save().unwrap();
    assert!(Group::load(&saved, old.clone()).unwrap().placed(), "control: the same member — placed");
    let mut gnew = Group::load(&saved, new.clone()).unwrap();
    assert!(!gnew.placed(), "the new member: not placed by the old one's save");
    let entry = gnew.add_member(&id(&new), &new.leaf_key(), &act_in(&new)).unwrap().unwrap();
    let (rot, _) = gnew.rotate(&mut rng).unwrap();
    assert!(gnew.placed(), "entered by itself");
    ga.merge_all(&[entry, rot], &l);
    let (sealed, op) = ga.seal([7; 32], b"after", &mut rng).unwrap();
    if let Some(op) = op {
        gnew.merge(&op, &l);
    }
    assert_eq!(gnew.open(&sealed).unwrap(), b"after");
}

/// 10-28 REVIEW C1: a SUPERSEDED member (its account replaced it — it keeps the old seed) adding ITSELF back is refused,
/// though it still authors its history. CONTROL: the account's current member's add is taken.
#[test]
fn a_superseded_member_never_adds_itself_back() {
    let (a, old, new) = (account(0xB1), account(0xB2), account(0xB3));
    let mut l = list(&[(&a, "did:a"), (&old, "did:b")]);
    let (mut ga, root) = Group::create(&SPACE, a.clone()).unwrap();
    let add = ga.add_member(&id(&old), &old.leaf_key(), &act_in(&old)).unwrap().unwrap();
    l.accounts.insert(id(&new), "did:b".into());
    l.leaves.insert(id(&new), new.leaf_key().to_bytes());
    let sup = SupList(l, [id(&old)].into_iter().collect());
    let out = ga.retire_superseded(&[id(&old)], [9; 32]).unwrap();
    let mut gold = Group::empty(&SPACE, old.clone());
    gold.merge_all(&[root.clone(), add.clone()], &sup);
    gold.merge_all(&out, &sup);
    assert!(!gold.placed());
    let back = gold.add_member(&id(&old), &old.leaf_key(), &act_in(&old)).unwrap().expect("its own add");
    not_written_for(&mut Group::load(&ga.save().unwrap(), a.clone()).unwrap(), &sup, &back, &old, [0; 32], 62);
    let real = ga.add_member(&id(&new), &new.leaf_key(), &act_in(&new)).unwrap().unwrap();
    assert_eq!(Group::load(&ga.save().unwrap(), a.clone()).unwrap().merge(&real, &sup), Merged::Applied, "control: the current member's add");
}

/// BeeKEM's OWN RULE (measured 10-28, asserted since): a removed member's add of ITSELF made CONCURRENT with its removal
/// (predecessors from before its first add) is taken and leaves it OUT — it reads nothing sealed after.
#[test]
fn a_concurrent_re_add_after_removal_leaves_the_member_out() {
    let mut rng = StdRng::seed_from_u64(51);
    let (a, b) = (account(0xC1), account(0xC2));
    let l = list(&[(&a, "did:a"), (&b, "did:b")]);
    let (mut ga, root) = Group::create(&SPACE, a.clone()).unwrap();
    let a1 = ga.add_member(&id(&b), &b.leaf_key(), &act_in(&b)).unwrap().unwrap();
    let (ar, _) = ga.rotate(&mut rng).unwrap();
    let mut gb = Group::empty(&SPACE, b.clone());
    gb.merge_all(&[root.clone(), a1.clone(), ar.clone()], &l);
    let rm = ga.remove_member(&id(&b), &[0; 32]).unwrap().unwrap();
    let (rr, _) = ga.rotate(&mut rng).unwrap();
    // b's forged re-add: from a tree that saw only the root.
    let mut gb0 = Group::empty(&SPACE, b.clone());
    gb0.merge(&root, &l);
    let a2 = gb0.add_member(&id(&b), &b.leaf_key(), &act_in(&b)).unwrap().unwrap();
    let took = ga.merge(&a2, &l);
    let in_a = ga.members().contains(&id(&b));
    let (sealed, op) = ga.seal([8; 32], b"after", &mut rng).unwrap();
    gb.merge_all(&[rm, rr, a2.clone()], &l);
    if let Some(op) = op {
        gb.merge(&op, &l);
    }
    assert_eq!(took, Merged::Applied, "taken: fixed facts only");
    assert!(!in_a, "concurrent with its removal: left out");
    assert!(gb.open(&sealed).is_err(), "it reads nothing sealed after");
}

/// 10-28 RE-REVIEW N1: a member REMOVED and then ADMITTED AGAIN (a space: kicked, then listed again — its member id the
/// same) is added back, by its adder and by a fresh merge alike: no fork. CONTROL: still not listed — refused.
#[test]
fn a_member_removed_and_admitted_again_is_added_back_everywhere() {
    let (a, b) = (account(0xD1), account(0xD2));
    let mut l = list(&[(&a, "did:a"), (&b, "did:b")]);
    l.act([5; 32], (false, id(&b)));
    let (mut ga, root) = Group::create(&SPACE, a.clone()).unwrap();
    // AN ADD NAMING NO ACT — refused though its account is listed now (judged by "now", it would go unauthorized once
    // its member left, and every later fresh merge would fork).
    let noact = Group::load(&ga.save().unwrap(), a.clone()).unwrap().add_member(&id(&b), &b.leaf_key(), &[0; 32]).unwrap().unwrap();
    assert!(matches!(Group::load(&ga.save().unwrap(), a.clone()).unwrap().merge(&noact, &l), Merged::Refused(w) if w.contains("no act")), "an add naming no act — refused");
    let add = ga.add_member(&id(&b), &b.leaf_key(), &act_in(&b)).unwrap().unwrap();
    let out = ga.remove_member(&id(&b), &[5; 32]).unwrap().unwrap();
    l.act([6; 32], (true, id(&b)));
    let back = ga.add_member(&id(&b), &b.leaf_key(), &[6; 32]).unwrap().expect("re-added by the act letting it in again");
    assert!(ga.placed() && ga.members().contains(&id(&b)), "the adder places b again");
    let ops = vec![root, add, out, back.clone()];
    let fresh = Group::empty(&SPACE, account(0xD3)).merge_all(&ops, &l);
    assert!(fresh.refused.is_empty() && fresh.waiting == 0, "a fresh merge takes it too: {fresh:?}");
    // KICKED AGAIN: b removed a second time, no longer listed — the history (its re-add by its act) still merges whole.
    let out2 = ga.remove_member(&id(&b), &[7; 32]).unwrap().unwrap();
    let mut again = list(&[(&a, "did:a")]);
    again.accounts.insert(id(&b), "did:b".into());
    again.leaves.insert(id(&b), b.leaf_key().to_bytes());
    for (k, v) in [(act_in(&b), (true, id(&b))), ([5; 32], (false, id(&b))), ([6; 32], (true, id(&b))), ([7; 32], (false, id(&b)))] {
        again.act(k, v);
    }
    let all: Vec<Vec<u8>> = ops.iter().cloned().chain([out2]).collect();
    let r2 = Group::empty(&SPACE, account(0xD5)).merge_all(&all, &again);
    assert!(r2.refused.is_empty() && r2.waiting == 0, "kicked twice: the history still whole: {r2:?}");
    // A KICKED MEMBER ADDED BACK by an act letting it in BEFORE its kick (its first act-in, still on record), by a
    // listed member's pre-kick view (10-28 re-reviews: history-judged refusal read evidence the sender chooses): taken,
    // and never written for.
    let all_g = || Group::load(&ga.save().unwrap(), a.clone()).unwrap();
    let mut gz = Group::empty(&SPACE, a.clone());
    let took = gz.merge_all(&ops[..3], &l);
    assert!(took.refused.is_empty() && took.waiting == 0, "a's view up to the kick: {took:?}");
    let rogue = gz.add_member(&id(&b), &b.leaf_key(), &act_in(&b)).unwrap().expect("an add");
    // (Concurrent with b's second removal: BeeKEM's own rule leaves it out — nothing placed to guard.)
    let mut h = all_g();
    assert_eq!(h.merge(&rogue, &again), Merged::Applied, "taken: fixed facts only");
    assert!(!h.members().contains(&id(&b)), "concurrent with its later removal: left out");
    let mut gz2 = Group::empty(&SPACE, a.clone());
    let took2 = gz2.merge_all(&all, &again);
    assert!(took2.refused.is_empty() && took2.waiting == 0 && !gz2.members().contains(&id(&b)), "a's view after the second kick: {took2:?}");
    let view2 = gz2.save().unwrap();
    // CONTROL: a THIRD invite, after the second kick — taken.
    again.act([8; 32], (true, id(&b)));
    let back3 = Group::load(&view2, a.clone()).unwrap().add_member(&id(&b), &b.leaf_key(), &[8; 32]).unwrap().expect("an add");
    assert_eq!(all_g().merge(&back3, &again), Merged::Applied, "control: an invite after the kick lets it back");
}


/// A DEVICE REMOVED BEFORE IT WAS EVER PLACED (10-28 re-review: it added itself and read): its own add is taken — and
/// never written for; a current device's, taken (control).
#[test]
fn a_device_removed_before_it_was_placed_is_never_written_for() {
    let (a, y, c) = (account(0x71), account(0x72), account(0x73));
    let chk = AccountCheck { current: vec![id(&a), id(&c)], ever: vec![id(&a), id(&y), id(&c)], writers: vec![id(&a)], leaves: lv(&[&a, &y, &c]) };
    let (ga, root) = Group::create(&SPACE, a.clone()).unwrap();
    let mut gy = Group::empty(&SPACE, y.clone());
    gy.merge(&root, &chk);
    let rogue = gy.add_member(&id(&y), &y.leaf_key(), &[0; 32]).unwrap().expect("its own add");
    not_written_for(&mut Group::load(&ga.save().unwrap(), a.clone()).unwrap(), &chk, &rogue, &y, [0; 32], 65);
    let mut gat = Group::load(&ga.save().unwrap(), a.clone()).unwrap();
    // CONTROL: a current device's own add — taken.
    let mut gc = Group::empty(&SPACE, c.clone());
    gc.merge(&root, &chk);
    let enter = gc.add_member(&id(&c), &c.leaf_key(), &[0; 32]).unwrap().expect("its own add");
    assert_eq!(gat.merge(&enter, &chk), Merged::Applied, "control: a current device enters");
}

/// A KICKED MEMBER's OWN OPERATIONS stay history (10-28 re-review): b added c and changed the key, then was kicked; a
/// fresh merge — b no longer listed — takes every one and places the next member; b signing AFTER its own removal: refused.
#[test]
fn a_kicked_members_operations_stay_history() {
    let mut rng = StdRng::seed_from_u64(41);
    let (a, b, c, d, e2) = (account(0xE1), account(0xE2), account(0xE3), account(0xE4), account(0xE5));
    let mut l = list(&[(&a, "did:a"), (&b, "did:b"), (&c, "did:c"), (&d, "did:d"), (&e2, "did:e")]);
    l.act([5; 32], (false, id(&b)));
    let (mut ga, root) = Group::create(&SPACE, a.clone()).unwrap();
    let add_b = ga.add_member(&id(&b), &b.leaf_key(), &act_in(&b)).unwrap().unwrap();
    let mut gb = Group::empty(&SPACE, b.clone());
    gb.merge_all(&[root.clone(), add_b.clone()], &l);
    let add_c = gb.add_member(&id(&c), &c.leaf_key(), &act_in(&c)).unwrap().unwrap();
    let (rot_b, _) = gb.rotate(&mut rng).unwrap();
    ga.merge_all(&[add_c.clone(), rot_b.clone()], &l);
    let out_b = ga.remove_member(&id(&b), &[5; 32]).unwrap().unwrap();
    let add_d = ga.add_member(&id(&d), &d.leaf_key(), &act_in(&d)).unwrap().unwrap();
    // b KICKED: not listed now (its acts still on record).
    l.listed.remove("did:b");
    let all = [root, add_b, add_c, rot_b, out_b.clone(), add_d];
    let mut gd = Group::empty(&SPACE, d.clone());
    let r = gd.merge_all(&all, &l);
    assert!(r.refused.is_empty() && r.waiting == 0 && gd.placed(), "the kicked member's history taken, d placed: {r:?}");
    // b, its removal merged, adding a listed member after it: refused.
    gb.merge_all(&all, &l);
    // (b, out, adding itself back: taken — and never written for.)
    let late = gb.add_member(&id(&b), &b.leaf_key(), &act_in(&b)).unwrap().expect("an add");
    not_written_for(&mut Group::load(&gd.save().unwrap(), d.clone()).unwrap(), &l, &late, &b, [5; 32], 64);
    // CONTROL: a never-listed account's operation — refused.
    let z = account(0xE9);
    l.accounts.insert(id(&z), "did:z".into());
    let mut gz = Group::empty(&SPACE, z.clone());
    gz.merge_all(&all, &l);
    let zop = gz.add_member(&id(&e2), &e2.leaf_key(), &act_in(&e2)).unwrap().expect("an add");
    assert!(matches!(gd.merge(&zop, &l), Merged::Refused(w) if w.contains("never on the list")), "control: never let in");
}

/// THE FOUNDER IS LISTED AS ANYONE (10-28 re-review): out of the roster, not listed — still EVER in (its root counts).
#[test]
fn the_founder_put_out_is_not_listed() {
    let l = ListCheck { owner: "did:o".into(), listed: vec!["did:n".into()], ..ListCheck::default() };
    assert!(!l.listed("did:o"), "the founder, off the roster: not listed");
    assert!(l.ever_in("did:o"), "…ever in: its first placement counts");
    assert!(!l.ever_in("did:x"));
    // An account let in by an act, off the roster since (kicked): ever in.
    let k = ListCheck { owner: "did:o".into(), acts: [([1; 32], (true, "did:k".to_string())), ([2; 32], (false, "did:k".to_string()))].into_iter().collect(), ..ListCheck::default() };
    assert!(!k.listed("did:k") && k.ever_in("did:k"), "kicked: not listed, ever in");
    assert!(!k.ever_in("did:x"), "never let in");
}

/// A KICKED ACCOUNT's NEW MEMBER added by its first act-in (10-28 re-review: it read what was sealed after the kick): taken —
/// and never written for; listed again, an add of it is (control).
#[test]
fn a_kicked_accounts_new_member_is_never_written_for() {
    let (a, b, b2) = (account(0xF1), account(0xF2), account(0xF3));
    let mut l = list(&[(&a, "did:a"), (&b, "did:b")]);
    l.act([5; 32], (false, id(&b)));
    let (mut ga, _root) = Group::create(&SPACE, a.clone()).unwrap();
    ga.add_member(&id(&b), &b.leaf_key(), &act_in(&b)).unwrap().unwrap();
    ga.remove_member(&id(&b), &[5; 32]).unwrap().unwrap();
    // b's account REPLACES its member: b2 on its card.
    l.accounts.insert(id(&b2), "did:b".into());
    l.leaves.insert(id(&b2), b2.leaf_key().to_bytes());
    let base = ga.save().unwrap();
    // (b KICKED: not listed now.)
    l.listed.remove("did:b");
    let old = Group::load(&base, a.clone()).unwrap().add_member(&id(&b2), &b2.leaf_key(), &act_in(&b)).unwrap().unwrap();
    not_written_for(&mut Group::load(&base, a.clone()).unwrap(), &l, &old, &b2, [5; 32], 66);
    l.listed.insert("did:b".into());
    l.act([6; 32], (true, id(&b)));
    let again = Group::load(&base, a.clone()).unwrap().add_member(&id(&b2), &b2.leaf_key(), &[6; 32]).unwrap().unwrap();
    assert_eq!(Group::load(&base, a.clone()).unwrap().merge(&again, &l), Merged::Applied, "control: an act after the kick");
}

/// THE KEY LOG MOVED, THE DEVICES NOT CREDENTIALED AGAIN YET (no current device): the account's check judges nothing —
/// every operation waits, none refused; once they are, all taken (control).
#[test]
fn an_account_check_with_no_current_device_judges_nothing() {
    let (a, b) = (account(0x81), account(0x82));
    let both = AccountCheck { current: vec![id(&a), id(&b)], ever: vec![id(&a), id(&b)], writers: vec![id(&a)], leaves: lv(&[&a, &b]) };
    let (mut ga, root) = Group::create(&SPACE, a.clone()).unwrap();
    let add = ga.add_member(&id(&b), &b.leaf_key(), &[0; 32]).unwrap().unwrap();
    let window = AccountCheck { current: vec![], ever: both.ever.clone(), writers: both.writers.clone(), leaves: both.leaves.clone() };
    let mut g = Group::empty(&SPACE, account(0x8F));
    let r = g.merge_all(&[root.clone(), add.clone()], &window);
    assert!(r.refused.is_empty() && r.applied == 0 && r.waiting == 2, "the window: all wait, none refused: {r:?}");
    let r = g.merge_all(&[root, add], &both);
    assert!(r.refused.is_empty() && r.applied == 2, "control: credentialed again — taken: {r:?}");
}

/// A KICKED MEMBER WHOSE CARD NO LONGER NAMES IT (10-28 review: it rewrote its card without its member — nothing could
/// take it out, and the tree was never clean): judged by WHOSE it was when taken (`known`, kept with the tree), not
/// written for, and taken out by its account's act. CONTROL: a tree saved before (no record) loads.
#[test]
fn a_kicked_member_that_drops_its_card_is_still_taken_out() {
    let mut rng = StdRng::seed_from_u64(71);
    let (a, b) = (account(0xA1), account(0xA2));
    let mut l = list(&[(&a, "did:a"), (&b, "did:b")]);
    l.act([5; 32], (false, id(&b)));
    let (mut ga, _root) = Group::create(&SPACE, a.clone()).unwrap();
    // (Added by THIS tree's own reconcile — the adder: its add comes back held, never judged here again — 10-28 review.)
    let ins: BTreeMap<String, [u8; 32]> = [("did:b".to_string(), act_in(&b))].into_iter().collect();
    let r0 = ga.reconcile(&[("did:b".to_string(), id(&b), b.leaf_key().to_bytes())], &|d| l.listed.contains(d), &ins, &BTreeMap::new(), [3; 32]);
    assert_eq!(r0.added, vec!["did:b".to_string()]);
    for op in &r0.ops {
        assert_eq!(ga.merge(op, &l), Merged::Applied, "its own, held");
    }
    ga.rotate(&mut rng).unwrap();
    assert!(ga.writable(&l).is_some(), "control: b listed — written with");
    // b KICKED, and its card no longer names its member.
    l.listed.remove("did:b");
    l.accounts.remove(&id(&b));
    let mut g = Group::load(&ga.save().unwrap(), a.clone()).unwrap();
    assert_eq!(g.whose(&id(&b)), None, "the tree's own save carries no record (every build reads it)");
    g.take_known(&ga.save_known());
    assert_eq!(g.whose(&id(&b)), Some("did:b"), "whose it was, saved beside the tree");
    assert!(g.writable(&l).is_none(), "placing it: not written with");
    let outs: BTreeMap<String, [u8; 32]> = [("did:b".to_string(), [5; 32])].into_iter().collect();
    assert!(g.due(&[], &|d| l.listed.contains(d), &outs), "its removal due — no card names it");
    let r = g.reconcile(&[], &|d| l.listed.contains(d), &BTreeMap::new(), &outs, [7; 32]);
    assert_eq!(r.removed, vec!["did:b".to_string()], "taken out by its account's act: {r:?}");
    assert!(g.writable(&l).is_some(), "clean again: written with");
    // (The tree's save is the cgka alone, as every earlier build saves and reads it.)
    assert_eq!(ga.save().unwrap(), bincode::serialize(&ga.cgka).unwrap(), "the save every build reads");
}

/// 10-29, live (Craftworks): three operations of the owner's device waited, said as "a predecessor not held" — read
/// as LOST operations. They were built on a stray root the creator rule refuses (taken on a page before the rule): a
/// waiting operation says which it waits on — REFUSED (for good), or on no feed read here (CONTROL: the predecessor
/// held back, then given: it applies).
#[test]
fn a_waiting_operation_says_its_predecessor_was_refused() {
    let (owner, member, reader) = (account(21), account(22), account(23));
    let l = list(&[(&owner, "did:owner"), (&member, "did:member"), (&reader, "did:reader")]);
    let (_go, owners_root) = Group::create(&SPACE, owner.clone()).unwrap();
    // The member's own tree on its stray root, the reader added on it.
    let (mut gm, members_root) = Group::create_by(&SPACE, member.clone(), &[0; 32]).unwrap();
    let on_stray = gm.add_member(&id(&reader), &reader.leaf_key(), &act_in(&reader)).unwrap().unwrap();
    let mut gr = Group::empty(&SPACE, reader.clone());
    let said = gr.merge_all(&[owners_root.clone(), members_root.clone(), on_stray.clone()], &l);
    assert_eq!((said.applied, said.waiting, said.refused.len()), (1, 1, 1), "{said:?}");
    assert!(said.waits[0].contains("built on a REFUSED operation"), "{said:?}");
    // CONTROL: the stray root never given — the same add waits on a predecessor on no feed read here.
    let mut gc = Group::empty(&SPACE, reader.clone());
    let said = gc.merge_all(&[owners_root, on_stray], &l);
    assert_eq!(said.waiting, 1, "{said:?}");
    assert!(said.waits[0].contains("on no feed read here"), "{said:?}");
}

/// An add waiting on its member's CARD names the act that let them in (`waiting_cards`): the node reads that account's
/// card next round, not after its 10-minute "asked, none there" wait (10-29, fz). CONTROL: the card known, the add applies
/// and nothing is named.
#[test]
fn an_add_waiting_on_a_card_names_its_act() {
    let (owner, member) = (account(31), account(32));
    let full = list(&[(&owner, "did:owner"), (&member, "did:member")]);
    let (mut go, root) = Group::create(&SPACE, owner.clone()).unwrap();
    let add = go.add_member(&id(&member), &member.leaf_key(), &act_in(&member)).unwrap().unwrap();
    // The member's card not read: whose it is unknown.
    let mut no_card = list(&[(&owner, "did:owner"), (&member, "did:member")]);
    no_card.accounts.remove(&id(&member));
    let mut g = Group::empty(&SPACE, owner.clone());
    let said = g.merge_all(&[root.clone(), add.clone()], &no_card);
    assert_eq!(said.waiting, 1, "{said:?}");
    assert_eq!(said.waiting_cards, vec![act_in(&member)], "{said:?}");
    let mut gc = Group::empty(&SPACE, owner);
    let said = gc.merge_all(&[root, add], &full);
    assert_eq!((said.waiting, said.waiting_cards.len()), (0, 0), "{said:?}");
}

/// A REMOVAL LEAVES THE TREE ROOTLESS — the removed member cannot make the key change and every other one is in step:
/// `due` says so and `reconcile` makes the one key change, after which the tree is writable (10-29, fz: a self-leave held
/// the node's re-key forever).
#[test]
fn a_tree_a_removal_left_rootless_is_due_its_key_change() {
    let mut rng = StdRng::seed_from_u64(9);
    let (a, b) = (account(1), account(2));
    let mut l = list(&[(&a, "did:a")]);
    l.act([7; 32], (true, id(&b)));
    l.act([6; 32], (false, id(&b)));
    let (mut ga, _) = Group::create(&SPACE, a.clone()).unwrap();
    ga.add_member(&id(&b), &b.leaf_key(), &[7; 32]).unwrap().unwrap();
    ga.rotate(&mut rng).unwrap();
    ga.remove_member(&id(&b), &[6; 32]).unwrap().unwrap();
    let cards: Vec<Card> = vec![("did:a".to_string(), id(&a), a.leaf_key().to_bytes())];
    let outs = std::collections::BTreeMap::new();
    let listed = |d: &str| d == "did:a";
    assert!(ga.writable(&l).is_none(), "rootless: nothing to write with");
    assert!(ga.rootless());
    assert!(ga.due(&cards, &listed, &outs), "a rootless tree is due its key change, the list already placed");
    let r = ga.reconcile(&cards, &listed, &std::collections::BTreeMap::new(), &outs, [4; 32]);
    assert_eq!(r.ops.len(), 1, "one key change, nothing added or removed: {:?}", r.not);
    assert!(ga.writable(&l).is_some(), "rooted again: writable");
    assert!(!ga.due(&cards, &listed, &outs), "in step: nothing due");
}
