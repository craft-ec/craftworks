//! CRAFTWORKS' SITUATION, randomized (owner 10-17: "forks should not happen"; no server, nobody must stay online, delay
//! accepted). A space's member LIST (its acts) changes at random; every member holding the group reconciles its own view
//! to the list by itself — adding who is listed and not placed, removing who is placed and not listed — so several
//! members act at once on stale views, the race the MLS design forked on. Members rotate their keys now and then. Every
//! operation reaches each replica after its own random delay, only in causal order (a replica applies an operation once
//! it holds its predecessors), and replicas go offline. A member added gets its replica from the operations it is sent.
//!
//! CHECKED at the end, after a quiet period (everyone online, everything delivered, one member reconciles to the list and
//! rotates, delivered everywhere): every replica's tree valid and IDENTICAL (no fork); every listed member derives the
//! SAME root key; no unlisted (removed) member derives one; the group places EXACTLY the list.
use crate::{
    cgka::Cgka,
    error::CgkaError,
    id::{MemberId, TreeId},
    keys::ShareKeyMap,
    operation::CgkaOperation,
    test_utils::{member, Member, ADD_AUTH, REMOVE_AUTH},
    transact::{Fork, Merge},
};
use alloc::{collections::BTreeSet, format, string::String, sync::Arc, vec, vec::Vec};
use future_form::Local;
use keyhive_crypto::{digest::Digest, share_key::ShareSecretKey, signed::Signed, signer::memory::MemorySigner, verifiable::Verifiable};
use rand::{rngs::StdRng, Rng, SeedableRng};

const PEOPLE: usize = 8;
const START: usize = 3;
const TICKS: u64 = 300;

type Op = Arc<Signed<CgkaOperation>>;

struct Sim {
    rng: StdRng,
    doc: TreeId,
    people: Vec<Member>,
    /// Each person's replica once they have one.
    replicas: Vec<Option<Cgka>>,
    /// THE LIST (the acts): who belongs.
    listed: BTreeSet<usize>,
    /// Every operation made, with when each person sees it.
    sent: Vec<(Op, Vec<u64>)>,
    /// Per person: the operations applied (by digest).
    applied: Vec<BTreeSet<Digest<Signed<CgkaOperation>>>>,
    offline_until: Vec<u64>,
    now: u64,
}

impl Sim {
    async fn new(seed: u64) -> Sim {
        let mut rng = StdRng::seed_from_u64(seed);
        let doc = TreeId(MemorySigner::generate(&mut rng).verifying_key());
        let people: Vec<Member> = (0..PEOPLE).map(|_| member(&mut rng)).collect();
        let mut s = Sim {
            rng,
            doc,
            replicas: (0..PEOPLE).map(|_| None).collect(),
            listed: (0..START).collect(),
            sent: Vec::new(),
            applied: vec![BTreeSet::new(); PEOPLE],
            offline_until: vec![0; PEOPLE],
            now: 0,
            people,
        };
        // Person 0 makes the group: its replica places itself, then the first members (as Keyhive's own harness).
        s.replicas[0] = Some(s.fresh(0));
        for i in 0..START {
            let (id, pk) = (s.people[i].id, s.people[i].pk);
            let op = s.replicas[0].as_mut().unwrap().add::<Local, _>(id, pk, ADD_AUTH, &s.people[0].signer).await.unwrap().map(Arc::new);
            if let Some(op) = op {
                s.send(0, op);
            }
        }
        s
    }
    fn fresh(&self, i: usize) -> Cgka {
        let mut sks = ShareKeyMap::new();
        sks.insert(self.people[i].pk, self.people[i].sk);
        Cgka::new(self.doc, self.people[i].id, sks)
    }
    fn send(&mut self, author: usize, op: Op) {
        let now = self.now;
        let seen: Vec<u64> = (0..PEOPLE).map(|p| if p == author { now } else { now + 1 + self.rng.gen_range(0..15) }).collect();
        self.applied[author].insert(Digest::hash(op.as_ref()));
        self.sent.push((op, seen));
    }
    /// Deliver to `i` everything due (causal: an op whose predecessors it lacks waits). A person placed in the tree but
    /// with no replica yet gets one from every operation sent (what it is sent once it is added).
    fn deliver(&mut self, i: usize, all: bool) {
        if self.replicas[i].is_none() {
            let placed = self.sent.iter().any(|(op, seen)| (all || seen[i] <= self.now) && matches!(&op.payload, CgkaOperation::Add { added_id, .. } if *added_id == self.people[i].id));
            if !placed {
                return;
            }
            self.replicas[i] = Some(self.fresh(i));
        }
        loop {
            let mut moved = false;
            for k in 0..self.sent.len() {
                let (op, seen) = (self.sent[k].0.clone(), self.sent[k].1[i]);
                let d = Digest::hash(op.as_ref());
                if (!all && seen > self.now) || self.applied[i].contains(&d) {
                    continue;
                }
                if self.replicas[i].as_mut().unwrap().merge_concurrent_operation(op).is_ok() {
                    self.applied[i].insert(d);
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
    }
    /// Person `i` reconciles its own view of the group to the list (adds, removes), or rotates.
    async fn act(&mut self, i: usize) -> Result<(), String> {
        let Some(r) = self.replicas[i].as_mut() else { return Ok(()) };
        if !r.tree.contains_id(&self.people[i].id) {
            return Ok(());
        }
        let placed: BTreeSet<MemberId> = r.tree.member_ids().collect();
        let signer = self.people[i].signer.clone();
        let mut ops = Vec::new();
        for p in 0..PEOPLE {
            let id = self.people[p].id;
            if self.listed.contains(&p) && !placed.contains(&id) && self.rng.gen_bool(0.5) {
                let pk = self.people[p].pk;
                if let Some(op) = self.replicas[i].as_mut().unwrap().add::<Local, _>(id, pk, ADD_AUTH, &signer).await.map_err(|e| format!("add: {e:?}"))? {
                    ops.push(Arc::new(op));
                }
            } else if !self.listed.contains(&p) && placed.contains(&id) && p != i && self.rng.gen_bool(0.5) {
                if let Some(op) = self.replicas[i].as_mut().unwrap().remove::<Local, _>(id, REMOVE_AUTH, &signer).await.map_err(|e| format!("remove: {e:?}"))? {
                    ops.push(Arc::new(op));
                }
            }
        }
        if ops.is_empty() && self.rng.gen_bool(0.2) {
            let sk = ShareSecretKey::generate(&mut self.rng);
            let pk = sk.share_key();
            match self.replicas[i].as_mut().unwrap().update::<Local, _, StdRng>(pk, sk, &signer, &mut self.rng).await {
                Ok((_, op, _)) => ops.push(Arc::new(op)),
                Err(CgkaError::IdentifierNotFound) | Err(CgkaError::NoMembers) => {}
                Err(e) => return Err(format!("rotate: {e:?}")),
            }
        }
        for op in ops {
            self.send(i, op);
        }
        Ok(())
    }
}

/// STRONG CONVERGENCE, every tick: two replicas that applied the same operations hold the same tree once each is put
/// in its canonical form (a full replay of its graph — the crate's own `assert_trees_match_replay`), and derive the same
/// root key when both derive one. (`incremental`: how many pairs differed BEFORE the replay — reported, not failed.)
fn same_ops_same_tree(s: &mut Sim, incremental: &mut usize) -> Result<(), String> {
    let canon = |c: &Cgka| {
        let mut r = c.clone();
        let f = r.fork();
        r.merge(f);
        r
    };
    for a in 0..PEOPLE {
        for b in a + 1..PEOPLE {
            if s.applied[a] != s.applied[b] {
                continue;
            }
            if let (Some(x), Some(y)) = (s.replicas[a].clone(), s.replicas[b].clone()) {
                if x.tree != y.tree {
                    *incremental += 1;
                }
                let (mut cx, mut cy) = (canon(&x), canon(&y));
                // THE KEY AS APPLIED (no replay) against the replayed one: what a product deriving from its live
                // replica would get.
                let (mut lx, mut ly) = (x.clone(), y.clone());
                if let (Ok(a1), Ok(b1)) = (lx.decrypt_tree_root_secret(), ly.decrypt_tree_root_secret()) {
                    if a1 != b1 {
                        LIVE_KEY_DIFF.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    }
                }
                if cx.tree != cy.tree {
                    return Err(format!("FORK — persons {a} and {b} applied the same operations into different trees, replayed too"));
                }
                if let (Ok(kx), Ok(ky)) = (cx.decrypt_tree_root_secret(), cy.decrypt_tree_root_secret()) {
                    if kx != ky {
                        return Err(format!("FORK — persons {a} and {b} applied the same operations and derive different root keys"));
                    }
                }
            }
        }
    }
    Ok(())
}

async fn run(seed: u64) -> Result<usize, String> {
    run_with(seed, false).await
}

/// `withhold`: the CONTROL — one operation never delivered to person 1 (the checks must see it).
async fn run_with(seed: u64, withhold: bool) -> Result<usize, String> {
    let mut s = Sim::new(seed).await;
    let mut incremental = 0usize;
    for _ in 0..TICKS {
        s.now += 1;
        // THE LIST changes (an act): someone listed or taken out (never person 0, the group's maker).
        if s.rng.gen_bool(0.05) {
            let p = s.rng.gen_range(1..PEOPLE);
            if !s.listed.insert(p) {
                s.listed.remove(&p);
            }
        }
        for p in 0..PEOPLE {
            if s.rng.gen_bool(0.02) {
                s.offline_until[p] = s.now + s.rng.gen_range(1..40);
            }
            if s.offline_until[p] > s.now {
                continue;
            }
            s.deliver(p, false);
            if s.listed.contains(&p) && s.rng.gen_bool(0.1) {
                s.act(p).await.map_err(|e| format!("seed {seed}: {e}"))?;
            }
        }
        let now = s.now;
        same_ops_same_tree(&mut s, &mut incremental).map_err(|e| format!("seed {seed} tick {now}: {e}"))?;
    }
    // QUIET: everyone online, everything delivered; person 0 reconciles to the list (all of it) and rotates; delivered.
    s.now += 1000;
    for p in 0..PEOPLE {
        s.deliver(p, true);
    }
    for _ in 0..3 {
        let r0 = s.replicas[0].as_mut().unwrap();
        let placed: BTreeSet<MemberId> = r0.tree.member_ids().collect();
        let signer = s.people[0].signer.clone();
        for p in 0..PEOPLE {
            let id = s.people[p].id;
            let r0 = s.replicas[0].as_mut().unwrap();
            let op = if s.listed.contains(&p) && !placed.contains(&id) {
                r0.add::<Local, _>(id, s.people[p].pk, ADD_AUTH, &signer).await.map_err(|e| format!("seed {seed}: quiet add: {e:?}"))?
            } else if !s.listed.contains(&p) && placed.contains(&id) {
                r0.remove::<Local, _>(id, REMOVE_AUTH, &signer).await.map_err(|e| format!("seed {seed}: quiet remove: {e:?}"))?
            } else {
                None
            };
            if let Some(op) = op {
                s.send(0, Arc::new(op));
            }
        }
        let sk = ShareSecretKey::generate(&mut s.rng);
        let pk = sk.share_key();
        let (_, op, _) = s.replicas[0].as_mut().unwrap().update::<Local, _, StdRng>(pk, sk, &signer, &mut s.rng).await.map_err(|e| format!("seed {seed}: quiet rotate: {e:?}"))?;
        s.send(0, Arc::new(op));
        for p in 0..PEOPLE {
            s.deliver(p, true);
        }
    }
    // THE CONTROL: person 1 misses the last operation it applied — rebuilt from every other one.
    if withhold && s.replicas[1].is_some() {
        let ops: Vec<Op> = s.sent.iter().map(|(o, _)| o.clone()).collect();
        let last = ops.len() - 1;
        let mut r = s.fresh(1);
        let mut done = BTreeSet::new();
        loop {
            let mut moved = false;
            for (k, op) in ops.iter().enumerate() {
                if k == last || done.contains(&k) {
                    continue;
                }
                if r.merge_concurrent_operation(op.clone()).is_ok() {
                    done.insert(k);
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
        s.replicas[1] = Some(r);
    }
    // THE CHECKS.
    let r0tree = s.replicas[0].as_ref().unwrap().tree.clone();
    for p in 0..PEOPLE {
        let Some(r) = s.replicas[p].as_ref() else {
            if s.listed.contains(&p) {
                return Err(format!("seed {seed}: person {p} listed but holds no replica"));
            }
            continue;
        };
        let v = r.tree.invariant_violations();
        if !v.is_empty() {
            return Err(format!("seed {seed}: person {p}'s tree is corrupt: {}", v.join(". ")));
        }
        if r.tree != r0tree {
            return Err(format!("seed {seed}: FORK — person {p}'s tree differs from person 0's"));
        }
    }
    let placed: BTreeSet<MemberId> = s.replicas[0].as_ref().unwrap().tree.member_ids().collect();
    let want: BTreeSet<MemberId> = s.listed.iter().map(|&p| s.people[p].id).collect();
    if placed != want {
        return Err(format!("seed {seed}: the group places {} people, the list names {}", placed.len(), want.len()));
    }
    let mut agreed = None;
    for p in 0..PEOPLE {
        let Some(r) = s.replicas[p].as_mut() else { continue };
        let key = r.decrypt_tree_root_secret();
        if !s.listed.contains(&p) {
            if key.is_ok() {
                return Err(format!("seed {seed}: person {p} was removed but still derives the root key"));
            }
            continue;
        }
        let key = key.map_err(|e| format!("seed {seed}: person {p} is listed but cannot derive the root key: {e:?}"))?;
        match agreed {
            None => agreed = Some(key),
            Some(k) if k != key => return Err(format!("seed {seed}: FORK — person {p} derives a different root key")),
            _ => {}
        }
    }
    if incremental > 0 {
        INCREMENTAL.fetch_add(incremental, core::sync::atomic::Ordering::Relaxed);
    }
    Ok(s.sent.len())
}
static INCREMENTAL: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
static LIVE_KEY_DIFF: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

#[tokio::test(flavor = "current_thread")]
async fn craftworks_situation_never_forks_and_follows_the_list() {
    let runs: u64 = std::env::var("SIM_RUNS").ok().and_then(|x| x.parse().ok()).unwrap_or(100);
    let (mut ops, mut failed) = (0, Vec::new());
    for seed in 1..=runs {
        match run(seed * 6151).await {
            Ok(n) => ops += n,
            Err(f) => failed.push(f),
        }
    }
    std::println!("beekem, craftworks' situation: {runs} runs, {} failed, {ops} operations in all; pairs with the same operations whose trees differed before a replay: {}; whose LIVE root keys differed: {}", failed.len(), INCREMENTAL.load(core::sync::atomic::Ordering::Relaxed), LIVE_KEY_DIFF.load(core::sync::atomic::Ordering::Relaxed));
    for f in failed.iter().take(12) {
        std::println!("  {f}");
    }
    assert!(failed.is_empty(), "{} of {runs} runs broke a property", failed.len());
}

#[tokio::test(flavor = "current_thread")]
async fn control_a_missed_operation_is_seen() {
    let runs: u64 = 30;
    let mut seen = 0;
    for seed in 1..=runs {
        if run_with(seed * 6151, true).await.is_err() {
            seen += 1;
        }
    }
    std::println!("control (one operation withheld from one person): {seen} of {runs} runs caught");
    assert!(seen > 0, "the checks must see a replica that missed an operation");
}

/// THE COST OF A REPLAY (keys derived only from a replayed replica): N members added by one, then ROTATIONS by members
/// in turn (each delivered to the measured replica). Timed: one full replay (fork + merge) and one root-key derivation.
#[tokio::test(flavor = "current_thread")]
async fn replay_cost_by_group_size() {
    for &(n, rotations) in &[(8usize, 200usize), (64, 200), (256, 200), (1024, 200)] {
        let mut rng = StdRng::seed_from_u64(n as u64);
        let doc = TreeId(MemorySigner::generate(&mut rng).verifying_key());
        let people: Vec<Member> = (0..n).map(|_| member(&mut rng)).collect();
        let replica = |i: usize| {
            let mut sks = ShareKeyMap::new();
            sks.insert(people[i].pk, people[i].sk);
            Cgka::new(doc, people[i].id, sks)
        };
        let mut owner = replica(0);
        let mut ops: Vec<Op> = Vec::new();
        for p in &people {
            if let Some(op) = owner.add::<Local, _>(p.id, p.pk, ADD_AUTH, &people[0].signer).await.unwrap() {
                ops.push(Arc::new(op));
            }
        }
        // A few members' own replicas rotate in turn (each catches up first): ops by many authors.
        let authors = 8.min(n);
        let mut reps: Vec<Cgka> = (0..authors).map(replica).collect();
        let mut seen = vec![0usize; authors];
        for k in 0..rotations {
            let a = k % authors;
            for op in &ops[seen[a]..] {
                reps[a].merge_concurrent_operation(op.clone()).ok();
            }
            seen[a] = ops.len();
            let sk = ShareSecretKey::generate(&mut rng);
            let pk = sk.share_key();
            let (_, op, _) = reps[a].update::<Local, _, StdRng>(pk, sk, &people[a].signer, &mut rng).await.unwrap();
            ops.push(Arc::new(op));
            seen[a] = ops.len();
        }
        let mut measured = replica(0);
        for op in &ops {
            measured.merge_concurrent_operation(op.clone()).unwrap();
        }
        // (Its own rotation resolves the concurrent ones: then it derives the key — the main simulation's settle.)
        let sk = ShareSecretKey::generate(&mut rng);
        let pk = sk.share_key();
        measured.update::<Local, _, StdRng>(pk, sk, &people[0].signer, &mut rng).await.unwrap();
        let t = std::time::Instant::now();
        let mut r = measured.clone();
        let f = r.fork();
        r.merge(f);
        let replay = t.elapsed();
        let t = std::time::Instant::now();
        r.decrypt_tree_root_secret().unwrap();
        let key = t.elapsed();
        std::println!("replay cost: {n} members, {} operations — replay {:?}, key {:?}", ops.len(), replay, key);
    }
}

/// LARGE GROUPS: N members added by one, then its own rotation (with every inner node blank, a first update encrypts to
/// a resolution that can be all of them), a second rotation, a full replay, a key. Each timed.
#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn large_group_costs() {
    let sizes: Vec<usize> = std::env::var("SIZES").ok().map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or(vec![10_000, 100_000]);
    for n in sizes {
        let mut rng = StdRng::seed_from_u64(n as u64);
        let doc = TreeId(MemorySigner::generate(&mut rng).verifying_key());
        let people: Vec<Member> = (0..n).map(|_| member(&mut rng)).collect();
        let mut sks = ShareKeyMap::new();
        sks.insert(people[0].pk, people[0].sk);
        let mut g = Cgka::new(doc, people[0].id, sks);
        let t = std::time::Instant::now();
        let mut ops = 0usize;
        let (mut first100, mut last100) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
        for (k, p) in people.iter().enumerate() {
            let t1 = std::time::Instant::now();
            if g.add::<Local, _>(p.id, p.pk, ADD_AUTH, &people[0].signer).await.unwrap().is_some() {
                ops += 1;
            }
            if k < 100 {
                first100 += t1.elapsed();
            } else if k >= n - 100 {
                last100 += t1.elapsed();
            }
        }
        let adding = t.elapsed();
        let mut rotate = Vec::new();
        for _ in 0..2 {
            let sk = ShareSecretKey::generate(&mut rng);
            let pk = sk.share_key();
            let t = std::time::Instant::now();
            let (_, op, _) = g.update::<Local, _, StdRng>(pk, sk, &people[0].signer, &mut rng).await.unwrap();
            let sz = match bincode::serialize(&op) { Ok(v) => v.len(), Err(e) => { std::println!("  (size: {e})"); 0 } };
            rotate.push((t.elapsed(), sz));
        }
        let t = std::time::Instant::now();
        let mut r = g.clone();
        let f = r.fork();
        r.merge(f);
        let replay = t.elapsed();
        let t = std::time::Instant::now();
        r.decrypt_tree_root_secret().unwrap();
        let key = t.elapsed();
        std::println!("large: {n} members ({ops} adds) — ONE add: {:?} into an empty group, {:?} into a full one (each, mean of 100); setup {adding:?}; ", first100 / 100, last100 / 100); std::println!("  first rotation {:?} ({} KB, bincode), second {:?} ({} KB); replay {replay:?}; key {key:?}", rotate[0].0, rotate[0].1 / 1024, rotate[1].0, rotate[1].1 / 1024);
    }
}

/// THE STEADY STATE: every member has rotated once (each replica caught up first), then one more rotation, and a
/// removal followed by the remover's rotation — each timed and sized (bincode). What a long-lived group pays.
#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn steady_state_costs() {
    let sizes: Vec<usize> = std::env::var("SIZES").ok().map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or(vec![1000, 4000]);
    for n in sizes {
        let mut rng = StdRng::seed_from_u64(n as u64 + 1);
        let doc = TreeId(MemorySigner::generate(&mut rng).verifying_key());
        let people: Vec<Member> = (0..n).map(|_| member(&mut rng)).collect();
        let fresh = |i: usize| {
            let mut sks = ShareKeyMap::new();
            sks.insert(people[i].pk, people[i].sk);
            Cgka::new(doc, people[i].id, sks)
        };
        let mut ops: Vec<Op> = Vec::new();
        let mut g0 = fresh(0);
        for p in &people {
            if let Some(op) = g0.add::<Local, _>(p.id, p.pk, ADD_AUTH, &people[0].signer).await.unwrap() {
                ops.push(Arc::new(op));
            }
        }
        // Each member, in turn, takes every op so far and rotates (one replica reused: rebuilt per member is O(n²)).
        let mut sizes_seen = Vec::new();
        for i in 0..n {
            let mut r = fresh(i);
            for op in &ops {
                r.merge_concurrent_operation(op.clone()).unwrap();
            }
            let sk = ShareSecretKey::generate(&mut rng);
            let pk = sk.share_key();
            let (_, op, _) = r.update::<Local, _, StdRng>(pk, sk, &people[i].signer, &mut rng).await.unwrap();
            sizes_seen.push(bincode::serialize(&op).unwrap().len());
            ops.push(Arc::new(op));
        }
        // One more rotation, by member 0, everyone having rotated.
        let mut r = fresh(0);
        for op in &ops {
            r.merge_concurrent_operation(op.clone()).unwrap();
        }
        let sk = ShareSecretKey::generate(&mut rng);
        let pk = sk.share_key();
        let t = std::time::Instant::now();
        let (_, op, _) = r.update::<Local, _, StdRng>(pk, sk, &people[0].signer, &mut rng).await.unwrap();
        let took = t.elapsed();
        // A SECOND rotation by the same replica (its history already worked through), then a key: is the first one slow
        // only once?
        let sk2 = ShareSecretKey::generate(&mut rng);
        let pk2 = sk2.share_key();
        let t2 = std::time::Instant::now();
        let (_, op2, _) = r.update::<Local, _, StdRng>(pk2, sk2, &people[0].signer, &mut rng).await.unwrap();
        std::println!("  second rotation by the same replica: {} B in {:?}", bincode::serialize(&op2).unwrap().len(), t2.elapsed());
        let at = |k: usize| sizes_seen.get(k - 1).copied().unwrap_or(0);
        std::println!("steady: {n} members — rotation size (bytes) as members rotate in turn: 1st {}, 100th {}, {}th {}, {}th {}; then one more: {} B in {took:?}", at(1), at(100), n / 2, at(n / 2), n, at(n), bincode::serialize(&op).unwrap().len());
    }
}

/// THE REAL CASE: each member KEEPS its replica (its device keeps its state), takes each new operation as it comes,
/// and rotates in turn. Timed: each rotation; reported: the slowest and the last, and how many updates are unchained.
#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn kept_replicas_costs() {
    let sizes: Vec<usize> = std::env::var("SIZES").ok().map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or(vec![300]);
    for n in sizes {
        let mut rng = StdRng::seed_from_u64(n as u64 + 7);
        let doc = TreeId(MemorySigner::generate(&mut rng).verifying_key());
        let people: Vec<Member> = (0..n).map(|_| member(&mut rng)).collect();
        let mut reps: Vec<Cgka> = (0..n)
            .map(|i| {
                let mut sks = ShareKeyMap::new();
                sks.insert(people[i].pk, people[i].sk);
                Cgka::new(doc, people[i].id, sks)
            })
            .collect();
        let mut ops: Vec<Op> = Vec::new();
        for p in &people {
            if let Some(op) = reps[0].add::<Local, _>(p.id, p.pk, ADD_AUTH, &people[0].signer).await.unwrap() {
                ops.push(Arc::new(op));
            }
        }
        let mut seen = vec![0usize; n];
        seen[0] = ops.len();
        let (mut slowest, mut last) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
        for round in 0..2 {
            for i in 0..n {
                for op in &ops[seen[i]..] {
                    reps[i].merge_concurrent_operation(op.clone()).unwrap();
                }
                let sk = ShareSecretKey::generate(&mut rng);
                let pk = sk.share_key();
                let t = std::time::Instant::now();
                let (_, op, _) = reps[i].update::<Local, _, StdRng>(pk, sk, &people[i].signer, &mut rng).await.unwrap();
                let took = t.elapsed();
                slowest = slowest.max(took);
                if round == 1 && i == n - 1 {
                    last = took;
                }
                ops.push(Arc::new(op));
                seen[i] = ops.len();
            }
        }
        std::println!("kept replicas: {n} members, {} operations — slowest rotation {slowest:?}, the last {last:?}; member 0 holds {} unchained updates", ops.len(), reps[0].unchained_updates.len());
    }
}
