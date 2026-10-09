//! RACE GET on the page (sdk#303): what the engine WITHDRAWS, the page stops asking.
//!
//! A reader page reads the keys of one member the node never answers. Its race rebuilds that member from the
//! group; one of the group's parity blocks is silent too, and the rebuild did not need it. Before withdrawal the
//! page re-sent that parity block's GET on the RTO for ever, for a read that was answered long ago.

use engine::{ClientId, Effect, Event, Op as WriteOp, Params, WriteId};
use freenet_prolly::node::Node;
use freenet_prolly::Cid;
use page::{Answer, Ms, Op, Page, PutPath};
use std::collections::{BTreeMap, BTreeSet};

/// The node: every block PUT to it, and the head.
#[derive(Default)]
struct Node_ {
    blocks: BTreeMap<Cid, Vec<u8>>,
    head: Option<(u64, Cid)>,
    /// Answers on their way back, with when they land: kept ACROSS drive calls, or every answer still in flight at
    /// the end of a call would be lost (which, at 300 ms, is most of them).
    due: Vec<(u64, Answer)>,
    /// A FORGETFUL node: it drops every `forget_every`-th block it serves, the moment it has served it (0: never).
    forget_every: usize,
    served: usize,
}

/// Drive `p` against `node` until nothing is waiting (or `for_ms` passes), answering after 5 ms; `silent` blocks
/// are never answered. Returns the GETs sent per block, with the time of each.
fn drive(
    p: &mut Page,
    node: &mut Node_,
    silent: &BTreeSet<Cid>,
    now: &mut u64,
    for_ms: u64,
    gets: &mut Vec<(u64, Cid)>,
) {
    drive_slow(p, node, silent, &BTreeSet::new(), now, for_ms, gets)
}

/// [`drive`], with the `slow` blocks answered after 300 ms instead of 5.
fn drive_slow(
    p: &mut Page,
    node: &mut Node_,
    silent: &BTreeSet<Cid>,
    slow: &BTreeSet<Cid>,
    now: &mut u64,
    for_ms: u64,
    gets: &mut Vec<(u64, Cid)>,
) {
    let end = *now + for_ms;
    while *now < end {
        for op in p.take_ops() {
            let a = match op {
                Op::Put { id, bytes } => {
                    node.blocks.insert(id, bytes);
                    Some(Answer::PutOk(id))
                }
                Op::Get { id } => {
                    gets.push((*now, id));
                    if silent.contains(&id) {
                        None
                    } else {
                        Some(match node.blocks.get(&id).cloned() {
                            Some(bytes) => {
                                node.served += 1;
                                if node.forget_every > 0 && node.served.is_multiple_of(node.forget_every) {
                                    node.blocks.remove(&id);
                                }
                                Answer::Got { id, bytes }
                            }
                            None => Answer::GetMissed(id),
                        })
                    }
                }
                Op::ReadHead { .. } => Some(Answer::Head { label: page::Label::Head, read: node.head.map(Into::into) }),
                // Page-io's own requests (signer, register): not this test's subject.
                Op::Ext(_) => None,
                Op::Sign { id, seq, root, .. } => {
                    node.head = Some((seq, root));
                    let record = [seq.to_le_bytes().as_slice(), &root].concat();
                    Some(Answer::Signer {
                        id,
                        answer: signer_proto::Answer::Signed(record),
                    })
                }
                Op::Update { .. } => Some(Answer::Updated { label: page::Label::Head }),
                Op::AskHeld { batch, ids } => Some(Answer::Held {
                    batch,
                    present: ids.iter().map(|id| node.blocks.contains_key(id)).collect(),
                }),
                Op::PutApp { key } => Some(Answer::AppPutOk(key)),
            };
            if let Some(a) = a {
                let delay = match &a {
                    Answer::Got { id, .. } | Answer::GetMissed(id) if slow.contains(id) => 300,
                    _ => 5,
                };
                node.due.push((*now + delay, a));
            }
        }
        *now += 1;
        let (ready, later): (Vec<_>, Vec<_>) = std::mem::take(&mut node.due).into_iter().partition(|(t, _)| *t <= *now);
        node.due = later;
        for (_, a) in ready {
            p.answer(a, Ms(*now));
        }
        p.tick(Ms(*now));
    }
}

/// A level-1 node's largest group, `(members, parity)`.
fn a_leaf_group(blocks: &BTreeMap<Cid, Vec<u8>>, root: Cid) -> (Vec<Cid>, Vec<Cid>) {
    let mut at = root;
    loop {
        let n = Node::parse(&blocks[&at]).expect("a node");
        assert!(!n.is_leaf(), "the tree is one leaf: no groups");
        if n.level() == 1 {
            let ids: Vec<Cid> = n.parity().collect();
            let groups = freenet_prolly::parity::group_members(&n);
            let m = ids.len() / groups.len();
            let (g, (_, members)) = groups
                .into_iter()
                .enumerate()
                .max_by_key(|(_, (_, m))| m.len())
                .expect("a group");
            return (members, ids[m * g..m * g + m].to_vec());
        }
        at = n.child(0).0;
    }
}

/// A writer page's real tree (6000 rows), with its parity, on a fake node: the node, the root, and the clock.
fn written() -> (Node_, Cid, u64) {
    written_with(500, |i| format!("value {i}").into_bytes())
}

/// [`written`], `per_batch` rows a write (a commit has a block cap), each row's value from `value_of(row)`.
fn written_with(per_batch: u64, value_of: impl Fn(u64) -> Vec<u8>) -> (Node_, Cid, u64) {
    // The writer: a real tree, with its parity, on the node.
    let mut node = Node_::default();
    let mut now = 1_000u64;
    let mut gets = Vec::new();
    let mut w = Page::new(Params::default(), PutPath::Page);
    for batch in 0..6000 / per_batch {
        let ops = (0..per_batch)
            .map(|i| {
                (
                    format!("k/{:06}", batch * per_batch + i).into_bytes(),
                    WriteOp::Put(value_of(batch * per_batch + i)),
                )
            })
            .collect();
        w.write(ClientId(1), WriteId(batch + 1), ops);
        drive(
            &mut w,
            &mut node,
            &BTreeSet::new(),
            &mut now,
            3_000,
            &mut gets,
        );
    }
    let Some((_, root)) = node.head else {
        let notices = w.take_notices();
        panic!("the writer did not publish: unusable {:?}, waiting {}, last notices {:?}", w.unusable(), w.waiting(), notices.iter().rev().take(3).collect::<Vec<_>>())
    };
    (node, root, now)
}

/// The keys of one leaf.
fn keys_of(node: &Node_, leaf: Cid) -> Vec<Vec<u8>> {
    let n = Node::parse(&node.blocks[&leaf]).expect("a leaf");
    (0..n.len()).map(|i| n.key(i)).collect()
}

/// A cold reader page reads `keys` from `node` (`silent` never answered) until every read is answered or 60 s
/// pass; returns the page, the GETs it sent, and how many reads were answered.
fn read_all(node: &mut Node_, now: &mut u64, keys: &[Vec<u8>], silent: &BTreeSet<Cid>) -> (Page, Vec<(u64, Cid)>, usize) {
    let mut r = Page::new(Params::default(), PutPath::Page);
    let mut gets = Vec::new();
    drive(&mut r, node, silent, now, 500, &mut gets);
    for (n, key) in keys.iter().enumerate() {
        r.event(Event::Get { client: ClientId(2), req_id: engine::read::ReqId(n as u64), key: key.clone() });
    }
    let mut replied = BTreeSet::new();
    for _ in 0..60 {
        drive(&mut r, node, silent, now, 1_000, &mut gets);
        for e in r.take_client() {
            if let Effect::Reply { client: ClientId(2), req_id, .. } = e {
                replied.insert(req_id.0);
            }
        }
        if replied.len() == keys.len() && !r.waiting() {
            break;
        }
    }
    (r, gets, replied.len())
}

/// REPAIR IS A WRITE (the owner, on sdk#331): a member the node has LOST (NotFound) is read through its group,
/// rebuilt, and PUT back by the commit's own PUT -- so the NEXT reader, fresh, gets it straight from the node
/// with no repair at all.
#[test]
fn a_rebuilt_block_is_put_back_and_the_next_reader_gets_it_directly() {
    let (mut node, root, mut now) = written();
    let (members, _) = a_leaf_group(&node.blocks, root);
    let member = members[members.len() / 2];
    let lost = node.blocks.remove(&member).expect("on the node");
    let keys = keys_of(&Node_ { blocks: [(member, lost.clone())].into_iter().collect(), ..Node_::default() }, member);

    let (a, _, answered) = read_all(&mut node, &mut now, &keys, &BTreeSet::new());
    let (started, rebuilt, _) = a.repair_counts();
    println!("  first reader: {answered} of {} reads answered; repairs started {started}, rebuilt {rebuilt}; the node has the member again: {}", keys.len(), node.blocks.contains_key(&member));
    assert_eq!(answered, keys.len(), "the first reader was not answered");
    assert_eq!(rebuilt, 1, "the member was not rebuilt");
    assert_eq!(node.blocks.get(&member), Some(&lost), "the rebuilt member was not PUT back (acked) as its own bytes");
    assert!(!a.waiting(), "the first reader still waits (its PUT unacked?)");

    let (b, gets, answered) = read_all(&mut node, &mut now, &keys, &BTreeSet::new());
    // Racing starts a race on every first want, so "started" is not the measure: REBUILT is.
    println!("  second reader: {answered} answered; races started {}, rebuilt {}; GETs of the member {}", b.repair_counts().0, b.repair_counts().1, gets.iter().filter(|(_, id)| *id == member).count());
    assert_eq!(answered, keys.len(), "the second reader was not answered");
    assert_eq!(b.repair_counts().1, 0, "the second reader had to rebuild the member: the network is still damaged");
}

/// A FORGETFUL node does not starve a read on the page (the production shape of engine/tests/reads.rs'
/// `a_forgetful_store_still_answers_every_read`, sdk#331): the engine reads the page's own memory (PageBlocks),
/// which keeps every block that arrived, so a node that drops what it serves -- every block, or every third --
/// still answers EVERY read, a value stored by reference included (it needs its leaf and its value block at once).
#[test]
fn a_forgetful_node_does_not_starve_a_read_on_the_page() {
    for forget_every in [1usize, 3] {
        // Every third row's value is 1400 bytes: stored by reference, in its own block.
        let (mut node, _root, mut now) = written_with(100, |i| if i.is_multiple_of(3) { vec![(i % 251) as u8; 1400] } else { format!("value {i}").into_bytes() });
        node.forget_every = forget_every;
        let keys: Vec<Vec<u8>> = (0..6000u64).step_by(400).map(|i| format!("k/{i:06}").into_bytes()).collect();
        let by_ref = (0..6000u64).step_by(400).filter(|i| i.is_multiple_of(3)).count();
        let (_, gets, answered) = read_all(&mut node, &mut now, &keys, &BTreeSet::new());
        println!("  node forgets every {forget_every}: {answered} of {} reads answered ({by_ref} by reference), {} GETs", keys.len(), gets.len());
        assert!(by_ref > 0, "no read by reference: the test proves nothing about the pair");
        assert_eq!(answered, keys.len(), "forget every {forget_every}: a read was left waiting on the page");
    }
}

#[test]
fn a_withdrawn_get_is_not_asked_again() {
    withdrawn_gets(false);
}

/// The same with every OTHER leaf read first and answered slowly (300 ms, packs included), so the race runs on a
/// window the slow GETs have shrunk (to 3): still every read answered, nothing withdrawn re-sent, nothing left
/// waiting. It does NOT leave a group block queued when the group resolves (the window refills before the engine
/// hears the resolving answer); the queue clause of `end_unneeded_gets` is tested inside the page crate.
#[test]
fn withdrawal_holds_with_slow_reads_in_flight() {
    withdrawn_gets(true);
}

fn withdrawn_gets(busy: bool) {
    let (mut node, root, mut now) = written();
    let (members, parity) = a_leaf_group(&node.blocks, root);
    assert!(
        parity.iter().all(|p| node.blocks.contains_key(p)),
        "the writer's parity is not on the node"
    );
    let (member, spare) = (members[members.len() / 2], parity[0]);
    let silent: BTreeSet<Cid> = [member, spare].into_iter().collect();
    let leaf = Node::parse(&node.blocks[&member]).expect("a leaf");
    let keys: Vec<Vec<u8>> = (0..leaf.len()).map(|i| leaf.key(i)).collect();
    let member_keys = keys.len();
    // Busy: one key from each OTHER leaf too, those leaves answered slowly, so their GETs hold the window and the
    // race's slots wait in the queue behind them.
    let mut slow = BTreeSet::new();
    let member_keys_v = keys;
    let mut keys = Vec::new();
    if busy {
        let mut at = vec![root];
        while let Some(id) = at.pop() {
            let n = Node::parse(&node.blocks[&id]).expect("a node");
            if n.is_leaf() {
                if !members.contains(&id) {
                    keys.push(n.key(0));
                }
            } else {
                at.extend((0..n.len()).map(|i| n.child(i).0));
            }
        }
        // Slow: every block but the member's path (the root, its parent) and its group -- packs included, which is
        // how the other leaves mostly come.
        // The path a_leaf_group took: child 0 down to the level-1 node that holds the group.
        let mut path = BTreeSet::new();
        let mut at = root;
        loop {
            path.insert(at);
            let n = Node::parse(&node.blocks[&at]).expect("a node");
            if n.level() == 1 {
                break;
            }
            at = n.child(0).0;
        }
        slow = node.blocks.keys().copied().filter(|id| !path.contains(id) && !members.contains(id) && !parity.contains(id)).collect();
    }
    // The member's reads LAST: the race's slots then queue behind the slow leaves' GETs, and go out one freed place
    // at a time -- so some are still queued when the first k have come back.
    keys.extend(member_keys_v);

    // The reader, cold.
    let mut r = Page::new(Params::default(), PutPath::Page);
    let mut gets = Vec::new();
    drive(&mut r, &mut node, &silent, &mut now, 500, &mut gets);
    let first = keys.len() - member_keys;
    for (n, key) in keys.iter().enumerate() {
        // Busy: the other leaves' slow GETs hold the window and fill the queue BEFORE the member is read.
        if n == first && busy {
            drive_slow(&mut r, &mut node, &silent, &slow, &mut now, 400, &mut gets);
        }
        r.event(Event::Get {
            client: ClientId(2),
            req_id: engine::read::ReqId(n as u64),
            key: key.clone(),
        });
    }
    let mut replied: BTreeSet<u64> = BTreeSet::new();
    let mut answered_at = None;
    for _ in 0..60 {
        drive_slow(&mut r, &mut node, &silent, &slow, &mut now, 1_000, &mut gets);
        for e in r.take_client() {
            if let Effect::Reply { client: ClientId(2), req_id, .. } = e {
                replied.insert(req_id.0);
            }
        }
        if replied.len() == keys.len() && answered_at.is_none() {
            answered_at = Some(now);
        }
    }
    let answered = replied.len();
    let answered_at = answered_at.unwrap_or_else(|| {
        let missing: Vec<String> = (0..keys.len() as u64)
            .filter(|n| !replied.contains(n))
            .map(|n| format!("#{n} {}", String::from_utf8_lossy(&keys[n as usize])))
            .collect();
        panic!("{answered} of {} reads answered in 60 s (the member's are the last {}); unanswered: {missing:?}; waiting {}", keys.len(), member_keys, r.waiting())
    });
    let spare_gets = gets.iter().filter(|(_, id)| *id == spare).count();
    // A GET the RTO had already timed out may go once more before the engine withdrew it; after that, none.
    let late: Vec<u64> = gets
        .iter()
        .filter(|(t, id)| *id == spare && *t > answered_at + 1_000)
        .map(|(t, _)| t - answered_at)
        .collect();
    println!("  {} reads answered; the unneeded silent parity block GET {spare_gets} times, {} of them after the reads were answered: {late:?} ms", keys.len(), late.len());
    assert!(
        late.is_empty(),
        "a withdrawn GET was asked again {} times after its race finished: {late:?} ms after",
        late.len()
    );
    let group_gets: BTreeMap<Cid, usize> = gets.iter().filter(|(_, id)| members.contains(id) || parity.contains(id)).fold(BTreeMap::new(), |mut m, (_, id)| {
        *m.entry(*id).or_insert(0) += 1;
        m
    });
    println!("  group of {} + {}: {} of its blocks asked, {} GETs", members.len(), parity.len(), group_gets.len(), group_gets.values().sum::<usize>());
    // The rebuilt member is not asked again either, and nothing is left waiting or withdrawn.
    let member_late = gets.iter().filter(|(t, id)| *id == member && *t > answered_at + 1_000).count();
    assert_eq!(member_late, 0, "the rebuilt member was still asked for after its reads were answered");
    // (No "withdrawn left over" count: the engine stores no withdrawn set -- the withdrawal is an `Unwanted` effect,
    // carried in the step that made it, WANTED-LIFE.)
    assert!(!r.waiting(), "the reader still waits, with every read answered (a withdrawn GET kept its deadline?)");

    // A LATE answer to the withdrawn GET, with bytes that are not its id: nothing re-sent, nothing left over.
    let before = gets.len();
    r.answer(Answer::Got { id: spare, bytes: b"not the block".to_vec() }, Ms(now));
    drive(&mut r, &mut node, &silent, &mut now, 30_000, &mut gets);
    assert_eq!(gets.len(), before, "a late wrong answer to a withdrawn GET was asked again: {:?}", &gets[before..]);
    assert!(!r.waiting());
}

/// A PARITY BLOCK A READ FETCHES IS A BLOCK LIKE ANY OTHER (engine `read::matches_id`, main's report from engineer2's
/// step-3 model): a member LOST (NotFound) is rebuilt from its group. The one member lost needs ONE parity block,
/// and the parity block the race took is KEPT in the page's memory under its id -- not judged "not its id" and
/// dropped. The group's other parity blocks come after the race resolved and are WITHDRAWN (a finished race keeps
/// nothing late, sdk#303), so they are not kept; every parity block is asked ONCE. RED on 200ce38: none kept.
#[test]
fn a_parity_block_a_read_fetches_is_kept_and_asked_once() {
    let (mut node, root, mut now) = written();
    let (members, parity) = a_leaf_group(&node.blocks, root);
    let member = members[members.len() / 2];
    let lost = node.blocks.remove(&member).expect("on the node");
    let keys = keys_of(&Node_ { blocks: [(member, lost)].into_iter().collect(), ..Node_::default() }, member);
    let (r, gets, answered) = read_all(&mut node, &mut now, &keys, &BTreeSet::new());
    assert_eq!(answered, keys.len(), "THE SETUP: the reads over a lost member were not answered");
    assert_eq!(r.repair_counts().1, 1, "THE SETUP: the member was not rebuilt from its group");
    let fetched: Vec<Cid> = parity.iter().copied().filter(|p| gets.iter().any(|(_, id)| id == p)).collect();
    assert!(!fetched.is_empty(), "THE SETUP: the race fetched no parity block");
    let asked: Vec<usize> = fetched.iter().map(|p| gets.iter().filter(|(_, id)| id == p).count()).collect();
    let kept: Vec<bool> = fetched.iter().map(|p| freenet_prolly::store::Blocks::get(r.blocks(), p).is_some()).collect();
    println!("  parity fetched {}: GETs each {asked:?}, kept {kept:?}", fetched.len());
    assert!(kept.iter().any(|k| *k), "no parity block the race took was kept under its id: {kept:?}");
    assert!(asked.iter().all(|n| *n == 1), "a parity block the node answered was asked again: {asked:?}");
}
