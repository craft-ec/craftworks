//! sdk#135: a COLD delta on a node that EVICTS what it serves. Before apply-and-resume the engine discarded every
//! diff page that reported `need` and restarted, so it needed the diff's whole working set co-resident, and on a
//! node retaining 40 blocks it re-fetched ANSWERED blocks for ever (engineer1: no reply after 20,000 fetches).
//! Now the parked delta keeps each correct prefix (freenet_prolly::diff's page contract, prolly#51) and resumes
//! after it; below the node's retention floor, two cycles of answered re-fetches answer `FullReloadRequired`.
//!
//! The node here is the issue's probe: it answers every block of the two trees, answers any other id (race GET's
//! parity asks) NotFound, and RETAINS at most `cap` blocks FIFO -- roots included, as a real node does.
mod common;
use engine::read::{ReadResult, ReqId};
use engine::subs::SubRange;
use engine::{ClientId, Effect, Engine, Event, Params};
use freenet_prolly::node::Value;
use freenet_prolly::store::{Blocks, MemBlocks};
use freenet_prolly::{build, Cid};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Bound;
use std::rc::Rc;

type Changes = Vec<(Vec<u8>, Option<Vec<u8>>)>;

/// A node that RETAINS at most `cap` fetched blocks (FIFO). Bytes are leaked so a borrow handed out before an
/// eviction stays valid -- test scaffolding, as in the engine's own tests.
#[derive(Clone)]
struct Node {
    held: Rc<RefCell<HashMap<Cid, &'static [u8]>>>,
    fifo: Rc<RefCell<VecDeque<Cid>>>,
    cap: usize,
}
impl Blocks for Node {
    fn get(&self, c: &Cid) -> Option<&[u8]> {
        self.held.borrow().get(c).copied()
    }
}
impl Node {
    fn new(cap: usize) -> Node {
        Node { held: Default::default(), fifo: Default::default(), cap }
    }
    fn put(&self, c: Cid, b: &[u8]) {
        self.held.borrow_mut().insert(c, Box::leak(b.to_vec().into_boxed_slice()));
        let mut f = self.fifo.borrow_mut();
        f.push_back(c);
        while f.len() > self.cap {
            let old = f.pop_front().expect("non-empty");
            if !f.contains(&old) {
                self.held.borrow_mut().remove(&old);
            }
        }
    }
}

/// Two trees of `n` keys, `b` changing every `every`-th key: every block of both, the roots, and the TRUE diff.
fn trees(n: u32, every: u32) -> (MemBlocks, Cid, Cid, Changes) {
    let val = |i: u32, g: u8| vec![g.wrapping_add((i % 251) as u8); if i.is_multiple_of(5) { 700 } else { 24 }];
    let a: Vec<(Vec<u8>, Vec<u8>)> = (0..n).map(|i| (format!("k/{i:06}").into_bytes(), val(i, 0))).collect();
    let b: Vec<(Vec<u8>, Vec<u8>)> = (0..n).map(|i| (format!("k/{i:06}").into_bytes(), val(i, if i % every == 7 { 9 } else { 0 }))).collect();
    let truth: Changes = (0..n).filter(|i| i % every == 7).map(|i| (b[i as usize].0.clone(), Some(b[i as usize].1.clone()))).collect();
    let mut all = MemBlocks::default();
    let ra = build::build(a.iter().map(|(k, v)| (&k[..], Value::Inline(&v[..]))), |c, x| all.insert(c, x)).unwrap();
    let rb = build::build(b.iter().map(|(k, v)| (&k[..], Value::Inline(&v[..]))), |c, x| all.insert(c, x)).unwrap();
    (all, ra, rb, truth)
}

struct Run {
    answer: Option<ReadResult>,
    fetched: usize,
    missed: usize,
    /// Every step's prefix was a correct prefix of the truth, and never moved backwards.
    prefix_ok: bool,
    /// The blocks asked, in order (to pick a block to make silent).
    asked: Vec<Cid>,
    /// The delta answered while a `slow` block it asked was still out.
    replied_while_held: bool,
}

/// Drive one cold delta to its answer (or `limit` fetches). `slow`: a block the node answers only when nothing else is
/// left to answer -- a slow network, where every other ask of the cycle lands while this one is still out.
#[allow(clippy::too_many_arguments)]
fn run(all: &MemBlocks, ra: Cid, rb: Cid, truth: &Changes, cap: usize, params: Params, limit: usize, slow: Option<Cid>) -> Run {
    let node = Node::new(cap);
    let mut e = Engine::new(params, node.clone());
    e.adopt_root_for_test(rb);
    let req = ReqId(1);
    let mut queue: VecDeque<Effect> = e
        .step(Event::ChangesSince {
            client: ClientId(1),
            req_id: req,
            from: ra,
            range: SubRange { lo: Bound::Unbounded, hi: Bound::Unbounded },
            max_entries: usize::MAX,
        })
        .into();
    let (mut fetched, mut missed, mut answer, mut prefix_ok) = (0, 0, None, true);
    let mut asked = Vec::new();
    let mut held_back: VecDeque<Cid> = VecDeque::new();
    let mut replied_while_held = false;
    let mut last_len = 0;
    let check = |e: &Engine<Node>, prefix_ok: &mut bool, last_len: &mut usize| {
        if let Some((_, acc)) = e.delta_prefix_for_test(req) {
            if acc.len() < *last_len || acc[..] != truth[..acc.len()] {
                *prefix_ok = false;
            }
            *last_len = acc.len();
        }
    };
    let mut tick = 0u64;
    while answer.is_none() && fetched < limit {
        let Some(f) = queue.pop_front() else {
            // Nothing else to answer: the held-back block lands now.
            match held_back.pop_front() {
                Some(id) => {
                    let b = all.get(&id).expect("a tree block");
                    node.put(id, b);
                    fetched += 1;
                    let out = e.step(Event::BlockArrived { id, bytes: b.to_vec() });
                    check(&e, &mut prefix_ok, &mut last_len);
                    queue.extend(out);
                }
                None => {
                    tick += 10_000;
                    let out = e.step(Event::Tick(tick));
                    if out.is_empty() || tick > 2_000_000 {
                        break;
                    }
                    queue.extend(out);
                }
            }
            continue;
        };
        match f {
            Effect::FetchBlock { id, .. } => {
                asked.push(id);
                if Some(id) == slow {
                    if !held_back.contains(&id) {
                        held_back.push_back(id);
                    }
                    continue;
                }
                fetched += 1;
                let out = match all.get(&id) {
                    Some(b) => {
                        node.put(id, b);
                        e.step(Event::BlockArrived { id, bytes: b.to_vec() })
                    }
                    None => {
                        missed += 1;
                        e.step(Event::BlockMissed(id))
                    }
                };
                check(&e, &mut prefix_ok, &mut last_len);
                queue.extend(out);
            }
            Effect::Reply { result, .. } => {
                replied_while_held |= !held_back.is_empty();
                answer = Some(result);
            }
            other => common::no_answer_owed(&other),
        }
    }
    Run { answer, fetched, missed, prefix_ok, asked, replied_while_held }
}

fn said(r: &Run, truth: &Changes) -> String {
    match &r.answer {
        Some(ReadResult::Delta { changes, cursor, .. }) => format!(
            "Delta: {} changes (truth {}), {}, cursor {}",
            changes.len(),
            truth.len(),
            if changes == truth { "EXACT" } else { "WRONG" },
            cursor.is_some()
        ),
        Some(other) => format!("{other:?}").split(['(', '{']).next().unwrap_or("").trim().to_string(),
        None => "NO REPLY".into(),
    }
}

#[test]
fn measure_the_floor() {
    let (all, ra, rb, truth) = trees(20_000, 333);
    println!("61-change delta of 20,000 keys, default params; node retains N blocks FIFO, roots evictable:");
    for cap in [20usize, 30, 40, 50, 60, 80, 120, 200, usize::MAX] {
        let r = run(&all, ra, rb, &truth, cap, Params::default(), 20_000, None);
        println!("  N = {cap:>20}: fetched {:>6} (NotFound {:>5}) · {} · every step a prefix: {}", r.fetched, r.missed, said(&r, &truth), r.prefix_ok);
    }
}

/// The 61-change delta: the issue's case.
fn case() -> (MemBlocks, Cid, Cid, Changes) {
    trees(20_000, 333)
}

/// Every block of the tree at `root` (what a full reload of the whole range reads, at least).
fn blocks_of(all: &MemBlocks, root: Cid) -> usize {
    let (mut seen, mut stack) = (HashSet::new(), vec![root]);
    while let Some(c) = stack.pop() {
        if !seen.insert(c) {
            continue;
        }
        let n = freenet_prolly::node::Node::parse(all.get(&c).expect("a block of the tree")).expect("a node");
        if !n.is_leaf() {
            stack.extend((0..n.len()).map(|i| n.child(i).0));
        }
    }
    seen.len()
}

#[test]
fn a_cold_delta_finishes_at_the_retention_floor_of_a_node_that_evicts() {
    // THE FLOOR, measured (measure_the_floor): 60 blocks with roots evictable (engineer2's 40 fails, 60 works). Each
    // attempt re-opens both roots and walks their spines to the resume key, plus one fetch batch.
    let (all, ra, rb, truth) = case();
    let r = run(&all, ra, rb, &truth, 60, Params::default(), 20_000, None);
    println!("N = 60: fetched {} (NotFound {}), a full reload reads at least {} blocks of b · {}", r.fetched, r.missed, blocks_of(&all, rb), said(&r, &truth));
    match &r.answer {
        Some(ReadResult::Delta { changes, cursor: None, new_root }) => {
            assert_eq!(changes, &truth, "not the true delta");
            assert_eq!(*new_root, rb);
        }
        other => panic!("the delta did not finish at the floor: {other:?} after {} fetches", r.fetched),
    }
    assert!(r.prefix_ok, "a step's established prefix was not a prefix of the true delta");
}

#[test]
fn every_step_is_a_correct_prefix_and_the_answer_exact_above_the_floor() {
    let (all, ra, rb, truth) = case();
    for cap in [80usize, 200, usize::MAX] {
        let r = run(&all, ra, rb, &truth, cap, Params::default(), 20_000, None);
        assert!(r.prefix_ok, "N = {cap}: a step was not a prefix");
        assert!(matches!(&r.answer, Some(ReadResult::Delta { changes, cursor: None, .. }) if *changes == truth), "N = {cap}: {}", said(&r, &truth));
    }
}

#[test]
fn below_the_floor_it_answers_full_reload_after_answered_refetches_never_a_wrong_or_partial_delta() {
    let (all, ra, rb, truth) = case();
    for cap in [20usize, 40, 50] {
        let r = run(&all, ra, rb, &truth, cap, Params::default(), 20_000, None);
        println!("N = {cap}: fetched {} · {}", r.fetched, said(&r, &truth));
        assert!(matches!(r.answer, Some(ReadResult::FullReloadRequired { new_root }) if new_root == rb), "N = {cap}: {}", said(&r, &truth));
        assert!(r.prefix_ok, "N = {cap}: a step was not a prefix");
    }
}

#[test]
fn a_deep_descent_with_everything_held_never_trips_the_stall_rule() {
    // Between two changes of a sparse delta the diff descends several levels, one answered fetch cycle each, with
    // (after, acc) unchanged: NOT a stall, because every cycle asks NEW blocks.
    for every in [5_000u32, 333] {
        let (all, ra, rb, truth) = trees(20_000, every);
        let r = run(&all, ra, rb, &truth, usize::MAX, Params::default(), 20_000, None);
        assert!(matches!(&r.answer, Some(ReadResult::Delta { changes, .. }) if *changes == truth), "{} changes: {}", truth.len(), said(&r, &truth));
    }
}

#[test]
fn an_unanswered_block_is_never_a_stall_the_read_never_answers_while_one_is_out() {
    // Below the floor (it WILL stall), with b's ROOT -- re-asked every cycle -- answered LAST, after every other ask of
    // its cycle has landed (a slow network). While it is out the cycle has not ended, so no arrival may count toward
    // the stall: the read answers only once nothing it asked is outstanding. Silence is re-asked (rule 8), never
    // taken for "missing".
    let (all, ra, rb, truth) = case();
    let r = run(&all, ra, rb, &truth, 40, Params::default(), 20_000, Some(rb));
    println!("N = 40, b's root always last: fetched {} · {} · asked the root {} times", r.fetched, said(&r, &truth), r.asked.iter().filter(|c| **c == rb).count());
    assert!(!r.replied_while_held, "the delta answered while a block it asked was still unanswered");
    assert!(r.answer.is_some(), "THE SETUP: the read never ended");
    assert!(r.prefix_ok);
}

#[test]
fn controls_warm_answers_in_one_step_an_unobtainable_old_root_reloads_promptly_and_a_page_limit_is_a_cursor() {
    let (all, ra, rb, truth) = case();
    // Warm: every block held, the answer in the same step (I4).
    let node = Node::new(usize::MAX);
    for c in [ra, rb] {
        let (mut seen, mut stack) = (HashSet::new(), vec![c]);
        while let Some(x) = stack.pop() {
            if !seen.insert(x) {
                continue;
            }
            let b = all.get(&x).expect("block");
            node.put(x, b);
            let n = freenet_prolly::node::Node::parse(b).expect("node");
            if !n.is_leaf() {
                stack.extend((0..n.len()).map(|i| n.child(i).0));
            }
        }
    }
    let mut e = Engine::new(Params::default(), node);
    e.adopt_root_for_test(rb);
    let out = e.step(Event::ChangesSince { client: ClientId(1), req_id: ReqId(1), from: ra, range: SubRange { lo: Bound::Unbounded, hi: Bound::Unbounded }, max_entries: usize::MAX });
    assert!(out.iter().any(|f| matches!(f, Effect::Reply { result: ReadResult::Delta { changes, .. }, .. } if *changes == truth)), "a warm delta did not reply in the same step");

    // An old root nobody holds: NotFound, and the full read at once.
    let mut only_b = MemBlocks::default();
    let (mut seen, mut stack) = (HashSet::new(), vec![rb]);
    while let Some(x) = stack.pop() {
        if !seen.insert(x) {
            continue;
        }
        let b = all.get(&x).expect("block");
        only_b.insert(x, b);
        let n = freenet_prolly::node::Node::parse(b).expect("node");
        if !n.is_leaf() {
            stack.extend((0..n.len()).map(|i| n.child(i).0));
        }
    }
    let r = run(&only_b, ra, rb, &truth, usize::MAX, Params::default(), 20_000, None);
    assert!(matches!(r.answer, Some(ReadResult::FullReloadRequired { .. })), "{}", said(&r, &truth));
    assert!(r.fetched <= 16, "an unobtainable old root took {} fetches to give up", r.fetched);

    // A page limit reached by prefixes: exactly the first `max_entries` changes, with a cursor.
    let node = Node::new(60);
    let mut e = Engine::new(Params::default(), node.clone());
    e.adopt_root_for_test(rb);
    let mut q: VecDeque<Effect> = e.step(Event::ChangesSince { client: ClientId(1), req_id: ReqId(2), from: ra, range: SubRange { lo: Bound::Unbounded, hi: Bound::Unbounded }, max_entries: 10 }).into();
    let mut reply = None;
    let mut n = 0;
    while let Some(f) = q.pop_front() {
        match f {
            Effect::FetchBlock { id, .. } => {
                n += 1;
                assert!(n < 20_000, "no reply");
                q.extend(match all.get(&id) {
                    Some(b) => {
                        node.put(id, b);
                        e.step(Event::BlockArrived { id, bytes: b.to_vec() })
                    }
                    None => e.step(Event::BlockMissed(id)),
                });
            }
            Effect::Reply { result, .. } => {
                reply = Some(result);
                break;
            }
            other => common::no_answer_owed(&other),
        }
    }
    match reply {
        Some(ReadResult::Delta { changes, cursor: Some(_), .. }) => assert_eq!(changes, truth[..10].to_vec()),
        other => panic!("a page limit did not answer 10 changes with a cursor: {other:?}"),
    }
}
