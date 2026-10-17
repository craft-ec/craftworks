//! CRAFTWORKS' FLOWS on BeeKEM (proposal `docs/proposals/members-and-keys-beekem.md`, "Flows"): each the model's own
//! case, with the control that shows the check can fail.
use crate::{
    cgka::Cgka,
    id::TreeId,
    keys::ShareKeyMap,
    operation::CgkaOperation,
    test_utils::{member, Member, ADD_AUTH, REMOVE_AUTH},
};
use alloc::{sync::Arc, vec::Vec};
use future_form::Local;
use keyhive_crypto::{digest::Digest, share_key::ShareSecretKey, signed::Signed, signer::memory::MemorySigner, verifiable::Verifiable};
use rand::{rngs::StdRng, SeedableRng};

type Op = Arc<Signed<CgkaOperation>>;

fn replica(doc: TreeId, m: &Member) -> Cgka {
    let mut sks = ShareKeyMap::new();
    sks.insert(m.pk, m.sk);
    Cgka::new(doc, m.id, sks)
}
fn feed(r: &mut Cgka, ops: &[Op]) {
    for op in ops {
        r.merge_concurrent_operation(op.clone()).unwrap();
    }
}
async fn rotate(r: &mut Cgka, m: &Member, rng: &mut StdRng) -> (Op, ShareSecretKey) {
    let sk = ShareSecretKey::generate(rng);
    let pk = sk.share_key();
    let (_, op, _) = r.update::<Local, _, StdRng>(pk, sk, &m.signer, rng).await.unwrap();
    (Arc::new(op), sk)
}

/// OUR CHECK before BeeKEM (BeeKEM merges what it is given — authorization is the layer above): an operation counts
/// only when its signer is a member the list names (`listed`), the account's key being its member id.
fn admitted(op: &Op, listed: &[&Member]) -> bool {
    listed.iter().any(|m| *op.issuer() == m.id.0)
}

/// FLOW 1 + HISTORY: a newcomer added after several updates derives the root key at once (the add's invitation) and
/// works back to the earlier updates' root secrets (the chain).
#[tokio::test]
async fn a_newcomer_reads_at_once_and_reads_history() {
    let mut rng = StdRng::seed_from_u64(0xf1);
    let doc = TreeId(MemorySigner::generate(&mut rng).verifying_key());
    let (a, b, c) = (member(&mut rng), member(&mut rng), member(&mut rng));
    let mut ra = replica(doc, &a);
    let mut ops: Vec<Op> = Vec::new();
    for m in [&a, &b] {
        ops.push(Arc::new(ra.add::<Local, _>(m.id, m.pk, ADD_AUTH, &a.signer).await.unwrap().unwrap()));
    }
    let mut rb = replica(doc, &b);
    feed(&mut rb, &ops);
    let mut earlier = Vec::new();
    for k in 0..3 {
        let (op, _) = if k % 2 == 0 { rotate(&mut ra, &a, &mut rng).await } else { rotate(&mut rb, &b, &mut rng).await };
        earlier.push(Digest::hash(op.as_ref()));
        ra.merge_concurrent_operation(op.clone()).unwrap();
        rb.merge_concurrent_operation(op.clone()).unwrap();
        ops.push(op);
    }
    let add_c = Arc::new(ra.add::<Local, _>(c.id, c.pk, ADD_AUTH, &a.signer).await.unwrap().unwrap());
    ops.push(add_c);
    // The adder rotates right after adding (proposal, model 7): the tree's root, blanked by the add, has a key again.
    let (rot, _) = rotate(&mut ra, &a, &mut rng).await;
    ops.push(rot);
    let mut rc = replica(doc, &c);
    feed(&mut rc, &ops);
    let key_a = ra.decrypt_tree_root_secret().ok();
    let key_c = rc.decrypt_tree_root_secret().ok();
    // Each earlier update's secret, as its author recorded it, reached by the newcomer (the chain, or its update's path).
    let readable: usize = earlier
        .iter()
        .filter(|h| {
            let want = ra.root_secrets.get(h).or_else(|| rb.root_secrets.get(h));
            let got = rc.root_secret_from_chain(h).or_else(|| rc.root_secret_from_update_path(h));
            want.is_some() && got == want
        })
        .count();
    std::println!("newcomer: root key now {}; earlier updates' secrets reached {readable} of {}", if key_c.is_some() && key_c == key_a { "the group's" } else { "NOT the group's" }, earlier.len());
    assert!(key_c.is_some() && key_c == key_a, "the newcomer reads at once");
    assert_eq!(readable, earlier.len(), "the newcomer reads the history");
}

/// FLOW 10: an outsider's operation — BeeKEM alone takes it (control: the layer above must check); our check refuses it.
#[tokio::test]
async fn an_outsiders_operation_is_refused_by_our_check() {
    let mut rng = StdRng::seed_from_u64(0xf2);
    let doc = TreeId(MemorySigner::generate(&mut rng).verifying_key());
    let (a, b, x) = (member(&mut rng), member(&mut rng), member(&mut rng));
    let mut ra = replica(doc, &a);
    let mut ops: Vec<Op> = Vec::new();
    for m in [&a, &b] {
        ops.push(Arc::new(ra.add::<Local, _>(m.id, m.pk, ADD_AUTH, &a.signer).await.unwrap().unwrap()));
    }
    // X, holding the operations (they are public) but not listed, writes a removal of B.
    let mut rx = replica(doc, &x);
    feed(&mut rx, &ops);
    let evil = Arc::new(rx.remove::<Local, _>(b.id, REMOVE_AUTH, &x.signer).await.unwrap().unwrap());
    let mut control = ra.clone();
    let taken = control.merge_concurrent_operation(evil.clone()).is_ok() && !control.tree.contains_id(&b.id);
    std::println!("outsider: BeeKEM alone {} it; our check {}", if taken { "TAKES" } else { "refuses" }, if admitted(&evil, &[&a, &b]) { "takes it" } else { "refuses it" });
    assert!(!admitted(&evil, &[&a, &b]), "our check refuses an outsider's operation");
    assert!(admitted(&ops[0], &[&a, &b]), "control: a listed member's is taken");
}

/// OPEN QUESTION 1 (one slot per account): two DEVICES of one account share its leaf. After one rotates, the other
/// cannot derive the root until the account hands it the new leaf secret — then it can.
#[tokio::test]
async fn an_accounts_devices_share_its_leaf_through_the_account() {
    let mut rng = StdRng::seed_from_u64(0xf3);
    let doc = TreeId(MemorySigner::generate(&mut rng).verifying_key());
    let (a, b) = (member(&mut rng), member(&mut rng));
    let mut ra = replica(doc, &a);
    let mut ops: Vec<Op> = Vec::new();
    for m in [&a, &b] {
        ops.push(Arc::new(ra.add::<Local, _>(m.id, m.pk, ADD_AUTH, &a.signer).await.unwrap().unwrap()));
    }
    let mut rb1 = replica(doc, &b);
    let mut rb2 = replica(doc, &b);
    feed(&mut rb1, &ops);
    feed(&mut rb2, &ops);
    let (op, new_sk) = rotate(&mut rb1, &b, &mut rng).await;
    ra.merge_concurrent_operation(op.clone()).unwrap();
    rb2.merge_concurrent_operation(op).unwrap();
    let before = rb2.decrypt_tree_root_secret().ok();
    rb2.owner_sks.insert(new_sk.share_key(), new_sk);
    let after = rb2.decrypt_tree_root_secret().ok();
    let group = ra.decrypt_tree_root_secret().ok();
    std::println!("second device: without the account's new leaf secret {}, with it {}", if before == group { "reads" } else { "cannot read" }, if after == group { "reads" } else { "cannot read" });
    assert!(before != group, "control: a device without the new leaf secret cannot read");
    assert!(after.is_some() && after == group, "with it (handed through the account) it reads");
}

/// FLOW 4: a device that LOST its state, restored from the account's leaf secrets and the group's operations: reads,
/// and rotates at the speed of a kept replica.
#[tokio::test]
async fn a_restored_device_reads_and_rotates_fast() {
    let mut rng = StdRng::seed_from_u64(0xf4);
    let doc = TreeId(MemorySigner::generate(&mut rng).verifying_key());
    let people: Vec<Member> = (0..40).map(|_| member(&mut rng)).collect();
    let mut reps: Vec<Cgka> = people.iter().map(|m| replica(doc, m)).collect();
    let mut ops: Vec<Op> = Vec::new();
    for m in &people {
        ops.push(Arc::new(reps[0].add::<Local, _>(m.id, m.pk, ADD_AUTH, &people[0].signer).await.unwrap().unwrap()));
    }
    let mut seen = alloc::vec![0usize; people.len()];
    seen[0] = ops.len();
    for round in 0..3 {
        for i in 0..people.len() {
            for op in &ops[seen[i]..] {
                reps[i].merge_concurrent_operation(op.clone()).unwrap();
            }
            let (op, _) = rotate(&mut reps[i], &people[i], &mut rng).await;
            ops.push(op);
            seen[i] = ops.len();
            let _ = round;
        }
    }
    // Member 5's device loses its state: a new one, with the account's leaf secrets (what the account holds).
    let account_sks = reps[5].owner_sks.clone();
    let mut restored = Cgka::new(doc, people[5].id, account_sks);
    feed(&mut restored, &ops);
    for op in &ops[seen[0]..] {
        reps[0].merge_concurrent_operation(op.clone()).unwrap();
    }
    let reads = restored.decrypt_tree_root_secret().ok() == reps[0].decrypt_tree_root_secret().ok();
    let t = std::time::Instant::now();
    let _ = rotate(&mut restored, &people[5], &mut rng).await;
    let took = t.elapsed();
    std::println!("restored device: {} the group's key; its rotation {took:?} ({} operations of history)", if reads { "reads" } else { "does NOT read" }, ops.len());
    assert!(reads, "a restored device reads");
    assert!(took.as_millis() < 500, "and rotates without rebuilding every earlier update: {took:?}");
}

/// FLOW 6: a removed member derives nothing after its removal, and its later operations are refused by our check.
#[tokio::test]
async fn a_removed_member_reads_nothing_new() {
    let mut rng = StdRng::seed_from_u64(0xf6);
    let doc = TreeId(MemorySigner::generate(&mut rng).verifying_key());
    let (a, b, c) = (member(&mut rng), member(&mut rng), member(&mut rng));
    let mut ra = replica(doc, &a);
    let mut ops: Vec<Op> = Vec::new();
    for m in [&a, &b, &c] {
        ops.push(Arc::new(ra.add::<Local, _>(m.id, m.pk, ADD_AUTH, &a.signer).await.unwrap().unwrap()));
    }
    let mut rc = replica(doc, &c);
    feed(&mut rc, &ops);
    let remove = Arc::new(ra.remove::<Local, _>(c.id, REMOVE_AUTH, &a.signer).await.unwrap().unwrap());
    let (rot, _) = rotate(&mut ra, &a, &mut rng).await;
    // (C's copy from before its removal: what a C that ignores its removal writes from.)
    let mut rc_before = rc.clone();
    rc.merge_concurrent_operation(remove).unwrap();
    rc.merge_concurrent_operation(rot).unwrap();
    let c_key = rc.decrypt_tree_root_secret().ok();
    let a_key = ra.decrypt_tree_root_secret().ok();
    std::println!("removed member: {}", if c_key.is_some() && c_key == a_key { "STILL READS" } else { "reads nothing new" });
    assert!(a_key.is_some() && c_key != a_key, "the removed member does not derive the group's new key");
    // C's own LATER operation (a rotation from its replica, which still holds the group as it was): signed with C's key,
    // no longer a listed member's — refused; control: A's own operation is taken.
    let (late, _) = rotate(&mut rc_before, &c, &mut rng).await;
    assert!(!admitted(&late, &[&a, &b]), "C, no longer listed: its operations are refused by our check");
    assert!(admitted(&ops[0], &[&a, &b]), "control: A's is taken");
}
