//! RACE GET (the owner's rule 11, sdk#303): a read asks ALL `k + m` blocks of a sibling group at once and
//! finishes on the first `k`.
//!
//! The case racing exists for is SILENCE: a member the network never answers -- no NotFound, nothing. Before
//! racing, the group was asked only after the member was answered NotFound, so a silent member held its read for
//! ever. Here a member is silent:
//! * racing on: every read is answered right, the member rebuilt from its group and verified;
//! * THE CONTROL, racing off: the read on the silent member is never answered.
//!
//! And the race's hygiene (the architect's review, docs/reviews/sdk303-race-get-attack.md):
//! * each block is asked ONCE per read, however many members of its group race it;
//! * what a finished race no longer wants is WITHDRAWN (the page drops its GET);
//! * a slot whose bytes do not hash to it never counts toward `k`.

use engine::read::{ReadResult, ReqId};
use engine::{ClientId, Effect, Engine, Event, Params};
use freenet_prolly::node::Node;
use freenet_prolly::store::{Blocks, MemBlocks};
use freenet_prolly::Cid;
use std::collections::{BTreeMap, BTreeSet};

mod common;
use common::{cold_reader, tree, Store};

fn records() -> BTreeMap<Vec<u8>, Vec<u8>> {
    (0..6000u32)
        .map(|i| {
            (
                format!("k/{i:06}").into_bytes(),
                format!("value {i}").into_bytes(),
            )
        })
        .collect()
}

/// A level-1 node's largest group, `(members, parity)`, with the node's parity blocks put on the network (as
/// `read_repair.rs` does).
fn a_leaf_group(all: &mut MemBlocks, root: Cid) -> (Vec<Cid>, Vec<Cid>) {
    let mut at = root;
    loop {
        let bytes = all.get(&at).expect("held").to_vec();
        let n = Node::parse(&bytes).expect("a node");
        assert!(!n.is_leaf(), "the tree is one leaf: no groups");
        if n.level() == 1 {
            let ids: Vec<Cid> = n.parity().collect();
            for (id, b) in freenet_prolly::parity::blocks_of(&n, &*all).expect("every member held")
            {
                all.insert(id, &b);
            }
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

fn keys_in(all: &MemBlocks, leaves: &[Cid]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for l in leaves {
        let n = Node::parse(all.get(l).expect("held")).expect("a leaf");
        out.extend((0..n.len()).map(|i| n.key(i)));
    }
    out
}

/// How the network answers one GET.
#[derive(Clone, Copy, PartialEq)]
enum Net {
    Serve,
    /// Never answers at all: no event.
    Silent,
    /// Answers ONCE with bytes that do not hash to the id, then is silent (a re-ask is paced by the page's RTO;
    /// answering every re-ask at once would only spin this loop).
    Forged,
    /// The node does not have it: NotFound.
    Lost,
}

struct Run {
    /// Blocks the engine told `Unwanted` (WANTED-LIFE), over the whole run.
    unwanted: BTreeSet<Cid>,
    answers: BTreeMap<Vec<u8>, ReadResult>,
    /// Blocks the engine put back (`PutRepaired`), in order.
    repaired: Vec<(Cid, Vec<u8>)>,
    /// FetchBlock effects per block id, over the whole run.
    asked: BTreeMap<Cid, usize>,
    e: Engine<Store>,
}

/// Read `keys` cold from `all`, each block answered as `net` says (default `Serve`).
fn read_cold(
    root: Cid,
    all: &MemBlocks,
    net: &BTreeMap<Cid, Net>,
    keys: &[Vec<u8>],
    params: Params,
) -> Run {
    read_cold_as(root, all, net, keys, params, false)
}

/// `together`: every read is asked before any answer comes (reads in flight at once), instead of one read settled
/// before the next is asked.
fn read_cold_as(
    root: Cid,
    all: &MemBlocks,
    net: &BTreeMap<Cid, Net>,
    keys: &[Vec<u8>],
    params: Params,
    together: bool,
) -> Run {
    let (mut e, store) = cold_reader(root, params);
    let mut answers = BTreeMap::new();
    let mut asked: BTreeMap<Cid, usize> = BTreeMap::new();
    let mut forged: BTreeSet<Cid> = BTreeSet::new();
    let mut unwanted: BTreeSet<Cid> = BTreeSet::new();
    let mut repaired = Vec::new();
    let rounds: Vec<Vec<usize>> = if together {
        vec![(0..keys.len()).collect()]
    } else {
        (0..keys.len()).map(|n| vec![n]).collect()
    };
    for round in rounds {
        let mut queue = Vec::new();
        for &n in &round {
            queue.extend(e.step(Event::Get {
                client: ClientId(1),
                req_id: ReqId(n as u64),
                key: keys[n].clone(),
            }));
        }
        let mut steps = 0;
        while let Some(f) = queue.pop() {
            steps += 1;
            assert!(steps < 50_000, "a read did not settle");
            match f {
                Effect::FetchBlock { id, .. } => {
                    *asked.entry(id).or_insert(0) += 1;
                    // As the page does: a GET the engine told `Unwanted` has ended, and is not answered -- unless it is
                    // wanted again since (a new ask).
                    if unwanted.contains(&id) && !e.readers_of(&id).any() {
                        continue;
                    }
                    match (net.get(&id).copied().unwrap_or(Net::Serve), all.get(&id)) {
                        (Net::Silent, _) => {}
                        (Net::Lost, _) => queue.extend(e.step(Event::BlockMissed(id))),
                        (Net::Forged, _) if !forged.insert(id) => {}
                        (Net::Forged, Some(b)) => {
                            let mut bad = b.to_vec();
                            let last = bad.len() - 1;
                            bad[last] ^= 0x5A;
                            queue.extend(e.step(Event::BlockArrived { id, bytes: bad }));
                        }
                        (_, Some(b)) => {
                            store.put(id, b);
                            queue.extend(e.step(Event::BlockArrived {
                                id,
                                bytes: b.to_vec(),
                            }));
                        }
                        (_, None) => queue.extend(e.step(Event::BlockMissed(id))),
                    }
                }
                Effect::Keep { id, bytes } => store.put(id, &bytes),
                Effect::PutRepaired { id, bytes } => repaired.push((id, bytes)),
                Effect::Unwanted { id } => {
                    unwanted.insert(id);
                }
                Effect::Reply { req_id, result, .. } if round.contains(&(req_id.0 as usize)) => {
                    answers.insert(keys[req_id.0 as usize].clone(), result);
                }
                other => common::no_answer_owed(&other),
            }
        }
    }
    Run { unwanted, answers, repaired, asked, e }
}

fn wrong(
    answers: &BTreeMap<Vec<u8>, ReadResult>,
    records: &BTreeMap<Vec<u8>, Vec<u8>>,
) -> Vec<String> {
    answers
        .iter()
        .filter_map(|(k, r)| match r {
            ReadResult::Value(Some(v)) if Some(v) == records.get(k) => None,
            other => Some(format!("{}: {other:?}", String::from_utf8_lossy(k))),
        })
        .collect()
}

#[test]
fn a_silent_member_does_not_hold_its_read_the_group_answers_it() {
    let records = records();
    let (root, mut all) = tree(&records);
    let (members, parity) = a_leaf_group(&mut all, root);
    let k = members.len();
    assert!(
        k >= 7 && parity.len() >= 3,
        "a group of {k} + {}: too small to be a real one",
        parity.len()
    );
    let silent = members[k / 2];
    let net: BTreeMap<Cid, Net> = [(silent, Net::Silent)].into_iter().collect();
    let keys = keys_in(&all, &[silent]);
    let run = read_cold(root, &all, &net, &keys, Params::default());
    let bad = wrong(&run.answers, &records);
    let (started, rebuilt, given_up) = run.e.repair_counts();
    println!("  silent member: {} reads, {} answered; races/repairs started {started}, rebuilt {rebuilt}, given up {given_up}", keys.len(), run.answers.len());
    assert_eq!(
        run.answers.len(),
        keys.len(),
        "a read on the silent member was never answered"
    );
    assert!(
        bad.is_empty(),
        "{} reads wrong: {:?}",
        bad.len(),
        &bad[..bad.len().min(3)]
    );
    assert_eq!(rebuilt, 1, "the silent member was not rebuilt exactly once");
    // Every other block of its group was asked -- the race -- and each of them ONCE.
    for s in members.iter().chain(&parity).filter(|s| **s != silent) {
        assert_eq!(
            run.asked.get(s).copied(),
            Some(1),
            "group block {} asked {:?} times (want 1)",
            hex(s),
            run.asked.get(s)
        );
    }
}

/// Reads of EVERY member of a group in flight at once: each member's first want races the same group, and still each
/// block goes out once -- a race asks what is not already asked, whether a read or another race asked it.
#[test]
fn reads_in_flight_together_ask_each_block_of_the_group_once() {
    let records = records();
    let (root, mut all) = tree(&records);
    let (members, parity) = a_leaf_group(&mut all, root);
    let keys: Vec<Vec<u8>> = members
        .iter()
        .map(|m| keys_in(&all, &[*m])[0].clone())
        .collect();
    let run = read_cold_as(root, &all, &BTreeMap::new(), &keys, Params::default(), true);
    assert_eq!(run.answers.len(), keys.len(), "not every read was answered");
    assert!(wrong(&run.answers, &records).is_empty());
    let twice: Vec<String> = members
        .iter()
        .chain(&parity)
        .filter(|s| run.asked.get(*s).copied().unwrap_or(0) > 1)
        .map(|s| format!("{} x{}", hex(s), run.asked[s]))
        .collect();
    let total: usize = members
        .iter()
        .chain(&parity)
        .map(|s| run.asked.get(s).copied().unwrap_or(0))
        .sum();
    println!(
        "  {} reads together over a group of {} + {}: {total} asks for its blocks",
        keys.len(),
        members.len(),
        parity.len()
    );
    assert!(
        twice.is_empty(),
        "group blocks asked more than once: {twice:?}"
    );
}

/// The same one-ask rule ACROSS steps, where no step sees the other's asks: read A is asked and raced, its group's
/// blocks held unanswered; read B (a member A's race already asked) comes in a LATER step; then B's GET is answered
/// NotFound, which the read and the race both want re-asked. Each block goes out once, and B once more for the miss.
#[test]
fn a_block_raced_and_read_in_different_steps_is_asked_once() {
    let records = records();
    let (root, mut all) = tree(&records);
    let (members, parity) = a_leaf_group(&mut all, root);
    let group: BTreeSet<Cid> = members.iter().chain(&parity).copied().collect();
    let (a, b) = (members[0], members[1]);
    let (mut e, store) = cold_reader(root, Params::default());
    let mut asked: BTreeMap<Cid, usize> = BTreeMap::new();
    let mut held: Vec<Cid> = Vec::new();
    let mut answered = 0;
    // Answer everything but the group's blocks, which are held back.
    let mut pump = |e: &mut Engine<Store>,
                    mut queue: Vec<Effect>,
                    held: &mut Vec<Cid>,
                    answered: &mut usize| {
        while let Some(f) = queue.pop() {
            match f {
                Effect::FetchBlock { id, .. } => {
                    *asked.entry(id).or_insert(0) += 1;
                    if group.contains(&id) {
                        held.push(id);
                    } else {
                        let bytes = all.get(&id).expect("held").to_vec();
                        store.put(id, &bytes);
                        queue.extend(e.step(Event::BlockArrived { id, bytes }));
                    }
                }
                Effect::Keep { id, bytes } => store.put(id, &bytes),
                Effect::Reply { .. } => *answered += 1,
                other => common::no_answer_owed(&other),
            }
        }
        asked.clone()
    };
    let key_a = keys_in(&all, &[a])[0].clone();
    let key_b = keys_in(&all, &[b])[0].clone();
    let q = e.step(Event::Get {
        client: ClientId(1),
        req_id: ReqId(0),
        key: key_a,
    });
    pump(&mut e, q, &mut held, &mut answered);
    let q = e.step(Event::Get {
        client: ClientId(1),
        req_id: ReqId(1),
        key: key_b,
    });
    let after_b = pump(&mut e, q, &mut held, &mut answered);
    let q = e.step(Event::BlockMissed(b));
    let after_miss = pump(&mut e, q, &mut held, &mut answered);
    println!(
        "  asks for a {:?}, b {:?} (then {:?} after its NotFound)",
        after_b.get(&a),
        after_b.get(&b),
        after_miss.get(&b)
    );
    let twice: Vec<String> = group
        .iter()
        .filter(|s| after_b.get(*s).copied().unwrap_or(0) > 1)
        .map(hex)
        .collect();
    assert!(twice.is_empty(), "asked twice before any answer: {twice:?}");
    assert_eq!(
        after_miss.get(&b).copied(),
        Some(2),
        "b's NotFound re-asked it {:?} times (want once more)",
        after_miss.get(&b).map(|n| n - 1)
    );
    assert_eq!(answered, 0, "a read answered with its group held back");
}

/// A RACE beside a healthy read DROPS a slot the node does not have: a tree whose parity is not on the network
/// (written before parity, or its parity lost) answers every read, and each parity block is asked ONCE -- not
/// re-asked beside a read that needs nothing from it.
#[test]
fn a_race_drops_a_slot_the_node_does_not_have() {
    let records = records();
    let (root, mut all) = tree(&records);
    let (members, parity) = a_leaf_group(&mut all, root);
    for p in &parity {
        all.0.remove(p);
    }
    let keys = keys_in(&all, &[members[0]]);
    let run = read_cold(root, &all, &BTreeMap::new(), &keys, Params::default());
    assert_eq!(run.answers.len(), keys.len(), "a read was not answered");
    assert!(wrong(&run.answers, &records).is_empty());
    let asked: Vec<usize> = parity.iter().map(|p| run.asked.get(p).copied().unwrap_or(0)).collect();
    println!("  parity not on the network: {} reads answered; each parity block asked {asked:?} times", keys.len());
    assert!(asked.iter().all(|n| *n == 1), "a parity block the node does not have was re-asked beside a healthy read: {asked:?}");
}

/// ...and once the raced block ITSELF is answered NotFound, the race is a REPAIR: a slot it dropped is asked
/// again, now until answered (the node may have it back).
#[test]
fn a_race_whose_block_is_missed_asks_its_dropped_slots_again() {
    let records = records();
    let (root, mut all) = tree(&records);
    let (members, parity) = a_leaf_group(&mut all, root);
    let (a, p) = (members[0], parity[0]);
    let (mut e, store) = cold_reader(root, Params::default());
    let group: BTreeSet<Cid> = members.iter().chain(&parity).copied().collect();
    // Serve the path; hold the group's blocks.
    let mut held = Vec::new();
    let mut queue = e.step(Event::Get { client: ClientId(1), req_id: ReqId(0), key: keys_in(&all, &[a])[0].clone() });
    while let Some(f) = queue.pop() {
        match f {
            Effect::FetchBlock { id, .. } if group.contains(&id) => held.push(id),
            Effect::FetchBlock { id, .. } => {
                let bytes = all.get(&id).expect("held").to_vec();
                store.put(id, &bytes);
                queue.extend(e.step(Event::BlockArrived { id, bytes }));
            }
            other => common::no_answer_owed(&other),
        }
    }
    assert!(held.contains(&p) && held.contains(&a), "the race did not ask the group");
    // The parity block: NotFound. Dropped, not re-asked.
    let out = e.step(Event::BlockMissed(p));
    assert!(!out.iter().any(|f| matches!(f, Effect::FetchBlock { id, .. } if *id == p)), "the race re-asked a NotFound slot beside a healthy read");
    // The block itself: NotFound. Now a repair: the dropped slot is asked again.
    let out = e.step(Event::BlockMissed(a));
    assert!(out.iter().any(|f| matches!(f, Effect::FetchBlock { id, .. } if *id == p)), "the repair did not ask the slot its race dropped");
    assert!(e.readers_of(&p).any(), "the repair does not wait on the slot");
}

/// THE CONTROL: racing off (today's one-at-a-time), the same silent member -- the read is never answered, and the
/// engine still waits on the block. So the pass above is the race's.
#[test]
fn control_with_racing_off_a_silent_member_holds_its_read() {
    let records = records();
    let (root, mut all) = tree(&records);
    let (members, _) = a_leaf_group(&mut all, root);
    let silent = members[members.len() / 2];
    let net: BTreeMap<Cid, Net> = [(silent, Net::Silent)].into_iter().collect();
    let keys = keys_in(&all, &[silent]);
    let run = read_cold(
        root,
        &all,
        &net,
        &keys,
        Params {
            race_get: false,
            ..Params::default()
        },
    );
    assert!(
        run.answers.is_empty(),
        "with racing off a read on a silent member was answered: {:?}",
        run.answers.len()
    );
    assert!(run.e.readers_of(&silent).any());
    assert_eq!(
        run.e.repair_counts(),
        (0, 0, 0),
        "a group was asked without racing"
    );
}

/// Once the group resolved, what it no longer wants is WITHDRAWN: here a silent PARITY block, which the rebuild
/// did not need -- the page drops its GET instead of re-asking it for ever.
#[test]
fn a_finished_race_withdraws_what_it_no_longer_wants() {
    let records = records();
    let (root, mut all) = tree(&records);
    let (members, parity) = a_leaf_group(&mut all, root);
    let silent = members[0];
    let net: BTreeMap<Cid, Net> = [(silent, Net::Silent), (parity[0], Net::Silent)]
        .into_iter()
        .collect();
    let keys = keys_in(&all, &[silent]);
    let run = read_cold(root, &all, &net, &keys, Params::default());
    assert_eq!(run.answers.len(), keys.len(), "the read was not answered");
    assert!(wrong(&run.answers, &records).is_empty());
    assert!(
        run.unwanted.contains(&parity[0]),
        "the silent parity block was not told Unwanted after its group resolved"
    );
    assert!(
        !run.e.readers_of(&parity[0]).any(),
        "the engine still awaits a block nobody needs"
    );
    // And it STAYS resolved: N more ticks ask nothing of the group, and what the engine awaits of it does not grow.
    let awaited = |e: &Engine<Store>| members.iter().chain(&parity).filter(|s| e.readers_of(s).any()).count();
    let mut e = run.e;
    let before = awaited(&e);
    let mut asked_after = 0;
    for t in 0..50u64 {
        asked_after += e.step(Event::Tick(10_000 + t)).iter().filter(|f| matches!(f, Effect::FetchBlock { .. })).count();
    }
    assert_eq!(asked_after, 0, "the resolved group was asked again on later ticks");
    assert!(awaited(&e) <= before, "what the engine awaits of the resolved group grew: {before} -> {}", awaited(&e));
    let run = Run { e, ..run };
    // Withdrawn means NOT WANTED: never a block the engine still awaits, and never the member the read needed.
    assert!(
        !run.unwanted.contains(&silent),
        "the rebuilt member was told Unwanted"
    );
    for s in members.iter().chain(&parity) {
        assert!(
            !(run.unwanted.contains(s) && run.e.readers_of(s).any()),
            "{} is both withdrawn and awaited",
            hex(s)
        );
    }
}

/// A slot answered with bytes that do not hash to it never counts toward `k`: the rebuild uses honest slots only,
/// and every read is still right.
#[test]
fn a_forged_slot_does_not_count_toward_k() {
    let records = records();
    let (root, mut all) = tree(&records);
    let (members, parity) = a_leaf_group(&mut all, root);
    let silent = members[0];
    // Forged: every parity block but one, and one member. What is left honest is still >= k.
    let mut net: BTreeMap<Cid, Net> = [(silent, Net::Silent), (members[1], Net::Forged)]
        .into_iter()
        .collect();
    for p in &parity[1..] {
        net.insert(*p, Net::Forged);
    }
    let honest = members.len() - 2 + 1;
    assert!(
        honest < members.len(),
        "the fixture must need the one honest parity block"
    );
    let keys = keys_in(&all, &[silent]);
    let run = read_cold(root, &all, &net, &keys, Params::default());
    let (_, rebuilt, given_up) = run.e.repair_counts();
    // k needs `members.len()` honest slots; with one member silent and one forged, the honest members are k-2 and
    // one honest parity makes k-1: NOT enough, so the read waits -- a forged slot counted would have "reached" k
    // and rebuilt garbage (or failed verification).
    assert!(
        run.answers.is_empty(),
        "a read was answered from a group that has only k-1 honest blocks: {:?}",
        run.answers.len()
    );
    assert_eq!(
        (rebuilt, given_up),
        (0, 0),
        "a forged slot counted toward k"
    );
    assert!(run.e.readers_of(&silent).any(), "the read stopped waiting");
}

fn hex(c: &Cid) -> String {
    core_types::hex::encode(&c[..6])
}

/// **A MISSING PARITY BLOCK IS PUT BACK** (sdk#479; the owner via core dev: a group whose parity is never replaced
/// erodes to its last copy). The node has lost one parity block of a group; reading the group's keys races the group,
/// sees that parity NotFound, and -- once every member is held -- re-encodes it with the save's own code and PUTs it
/// back, byte for byte the block the save made. THE CONTROL: nothing lost, nothing put back.
#[test]
fn a_parity_block_the_reads_find_missing_is_re_encoded_and_put_back_byte_for_byte() {
    let records = records();
    let (root, mut all) = tree(&records);
    let (members, parity) = a_leaf_group(&mut all, root);
    let keys = keys_in(&all, &members);
    let lost = parity[1];
    let net: BTreeMap<Cid, Net> = [(lost, Net::Lost)].into_iter().collect();
    let run = read_cold(root, &all, &net, &keys, Params::default());
    assert!(wrong(&run.answers, &records).is_empty(), "THE SETUP: a read of the group went wrong");
    let original = all.get(&lost).expect("the save made it").to_vec();
    let back: Vec<&(Cid, Vec<u8>)> = run.repaired.iter().filter(|(id, _)| *id == lost).collect();
    assert_eq!(back.len(), 1, "the lost parity was not put back exactly once: {} times", back.len());
    assert_eq!(back[0].1, original, "the parity put back is not the block the save made");
    // THE CONTROL: the same reads with the parity on the network. (The fixture puts only THIS node's parity on the
    // network, so other groups' parity is missing in both runs: the difference is this one block.)
    let clean = read_cold(root, &all, &BTreeMap::new(), &keys, Params::default());
    // (A MEMBER may be put back in either run: when `k` of its group answer before it, the race rebuilds it -- main's
    // race get, not this change.)
    assert!(clean.repaired.iter().all(|(id, _)| !parity.contains(id)), "THE CONTROL: a whole group put one of its parity blocks back");
    let (m, p) = (run.e.parity_counts(), clean.e.parity_counts());
    println!("parity (missing, put back, mismatched): lost one {m:?}, whole {p:?}");
    assert_eq!((m.0 - p.0, m.1 - p.1, m.2 - p.2), (1, 1, 0), "the lost parity is not the one difference in (missing, put back, mismatched)");
}
