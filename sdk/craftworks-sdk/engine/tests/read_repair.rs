//! A READ REPAIRS THROUGH PARITY (Phase 4, DURABILITY-STATE gap 1; the
//! owner's test: data survives when the nodes that held it go away).
//!
//! A real tree, written by a real engine with its parity, is read cold by a
//! second engine from a "network" that has LOST blocks of one sibling group:
//! * up to 3 of the group gone (members, or members and parity): every read
//!   answers exactly what the fully-held tree holds -- the lost blocks are
//!   rebuilt from the group, verified by hash, and kept;
//! * 4 gone: the reads that needed them end `Unavailable` naming the block,
//!   and the engine says why (the group could not reach k);
//! * THE CONTROL: repair off, ONE block gone -- `Unavailable`. So the passes
//!   above are the repair's, not a network that still had the block.

use engine::read::{ReadResult, ReqId};
use engine::{ClientId, Effect, Engine, Event, Params, PARITY};
use freenet_prolly::node::Node;
use freenet_prolly::store::{Blocks, MemBlocks};
use freenet_prolly::Cid;
use std::collections::{BTreeMap, BTreeSet};

mod common;
use common::{cold_reader, tree, Store};

/// The records: enough small rows for a branch whose children (leaves) form
/// full sibling groups.
fn records() -> BTreeMap<Vec<u8>, Vec<u8>> {
    (0..6000u32).map(|i| (format!("k/{i:06}").into_bytes(), format!("value {i}").into_bytes())).collect()
}

/// A node one level above the leaves, and one of its groups: `(members,
/// parity)`, members being leaves. The node's parity BLOCKS are put on the
/// network too -- computed with `blocks_of`, the function the writer puts them
/// with, and checked to be exactly the ids the node lists (the fixture writer
/// stops at `Published`, before its parity is due).
fn a_leaf_group(all: &mut MemBlocks, root: Cid) -> (Vec<Cid>, Vec<Cid>) {
    let mut at = root;
    loop {
        let bytes = all.get(&at).expect("held").to_vec();
        let n = Node::parse(&bytes).expect("a node");
        assert!(!n.is_leaf(), "the tree is one leaf: no groups to lose");
        if n.level() == 1 {
            let ids: Vec<Cid> = n.parity().collect();
            let blocks = freenet_prolly::parity::blocks_of(&n, &*all).expect("every member held");
            assert_eq!(blocks.iter().map(|(id, _)| *id).collect::<Vec<_>>(), ids, "the parity computed is not what the node lists");
            for (id, b) in blocks {
                all.insert(id, &b);
            }
            let groups = freenet_prolly::parity::group_members(&n);
            let (g, (_, members)) = groups.into_iter().enumerate().max_by_key(|(_, (_, m))| m.len()).expect("a group");
            assert!(ids.len() >= PARITY * (g + 1), "the node lists no parity for its groups: pcount {}", ids.len());
            return (members, ids[PARITY * g..PARITY * (g + 1)].to_vec());
        }
        at = n.child(0).0;
    }
}

/// Every key held in `leaves`.
fn keys_in(all: &MemBlocks, leaves: &[Cid]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for l in leaves {
        let n = Node::parse(all.get(l).expect("held")).expect("a leaf");
        for i in 0..n.len() {
            out.push(n.key(i));
        }
    }
    out
}

/// What a cold reader answers for `keys`, from a network holding `all` minus
/// `lost`; the page keeps what the engine rebuilt (`Keep`).
///
/// The engine re-asks a missed block with NO count that ends it (rule 7);
/// the PAGE paces those re-asks on a backoff. This harness has no clock, so
/// it plays the pacing as "a block asked more than [`PACED`] times is not
/// asked again yet": its later re-asks are held, as a paced page holds them
/// until due. A read that is still waiting then is a read the page would
/// show as "not answering" -- never one that ended.
const PACED: usize = 3;

fn read_cold(root: Cid, all: &MemBlocks, lost: &BTreeSet<Cid>, keys: &[Vec<u8>], params: Params) -> (BTreeMap<Vec<u8>, ReadResult>, Engine<Store>) {
    let (mut e, store) = cold_reader(root, params);
    let mut answers = BTreeMap::new();
    for (n, key) in keys.iter().enumerate() {
        // Paced per read: each read is its own stretch of time.
        let mut asked: BTreeMap<Cid, usize> = BTreeMap::new();
        let mut queue = e.step(Event::Get { client: ClientId(1), req_id: ReqId(n as u64), key: key.clone() });
        let mut steps = 0;
        while let Some(f) = queue.pop() {
            steps += 1;
            assert!(steps < 20_000, "a read did not settle");
            match f {
                Effect::FetchBlock { id, .. } => {
                    let times = asked.entry(id).or_insert(0);
                    *times += 1;
                    if *times > PACED {
                        continue;
                    }
                    let ev = match all.get(&id).filter(|_| !lost.contains(&id)) {
                        Some(b) => {
                            store.put(id, b);
                            Event::BlockArrived { id, bytes: b.to_vec() }
                        }
                        None => Event::BlockMissed(id),
                    };
                    queue.extend(e.step(ev));
                }
                Effect::Keep { id, bytes } => store.put(id, &bytes),
                Effect::Reply { req_id, result, .. } if req_id == ReqId(n as u64) => {
                    answers.insert(key.clone(), result);
                }
                other => common::no_answer_owed(&other),
            }
        }
    }
    (answers, e)
}

fn right(answers: &BTreeMap<Vec<u8>, ReadResult>, records: &BTreeMap<Vec<u8>, Vec<u8>>) -> Vec<String> {
    answers
        .iter()
        .filter_map(|(k, r)| match r {
            ReadResult::Value(Some(v)) if Some(v) == records.get(k) => None,
            other => Some(format!("{}: {other:?}", String::from_utf8_lossy(k))),
        })
        .collect()
}

#[test]
fn up_to_parity_lost_blocks_of_a_group_are_rebuilt_and_every_read_is_right() {
    let records = records();
    let (root, mut all) = tree(&records);
    let (members, parity) = a_leaf_group(&mut all, root);
    let k = members.len();
    assert!(k > PARITY, "the group has {k} members: too small to lose PARITY of them");
    // PARITY lost, spread evenly from the group's first member to its last.
    let spread: Vec<Cid> = (0..PARITY).map(|i| members[i * (k - 1) / (PARITY - 1)]).collect();
    let mixed: Vec<Cid> = members[1..PARITY].iter().copied().chain(std::iter::once(parity[0])).collect();
    for (case, lost) in [
        ("1 member".to_string(), vec![members[0]]),
        (format!("{PARITY} members"), spread),
        (format!("{} members and 1 parity", PARITY - 1), mixed),
    ] {
        let lost: BTreeSet<Cid> = lost.into_iter().collect();
        let lost_leaves: Vec<Cid> = members.iter().copied().filter(|m| lost.contains(m)).collect();
        let keys = keys_in(&all, &lost_leaves);
        assert!(!keys.is_empty(), "{case}: nothing to read in the lost leaves");
        let (answers, e) = read_cold(root, &all, &lost, &keys, Params::default());
        let wrong = right(&answers, &records);
        let (started, rebuilt, given_up) = e.repair_counts();
        println!("  {case}: {} reads over {} lost leaves -- repairs started {started}, rebuilt {rebuilt}, given up {given_up}", keys.len(), lost_leaves.len());
        assert!(wrong.is_empty(), "{case}: {} of {} reads wrong after losing {} blocks of a group: {:?}", wrong.len(), keys.len(), lost.len(), &wrong[..wrong.len().min(3)]);
        assert_eq!(answers.len(), keys.len(), "{case}: a read never answered");
        assert_eq!(rebuilt, lost_leaves.len() as u64, "{case}: the lost leaves were not each rebuilt once");
        assert_eq!(given_up, 0, "{case}");
    }
}

/// Past the group's reach the read WAITS (rule 7): it is never answered
/// `Unavailable`, and the engine still awaits the block, so the page keeps
/// asking for it and shows "not answering for N s". The block may come back
/// (a keeper's repair, a late PUT); `Unavailable` would have thrown away the
/// read that could then be answered.
#[test]
fn parity_plus_one_lost_blocks_of_a_group_wait_and_are_never_answered_unavailable() {
    let records = records();
    let (root, mut all) = tree(&records);
    let (members, _) = a_leaf_group(&mut all, root);
    let lost: BTreeSet<Cid> = members[..PARITY + 1].iter().copied().collect();
    let keys = keys_in(&all, &members[..1]);
    let (answers, e) = read_cold(root, &all, &lost, &keys, Params::default());
    assert!(answers.is_empty(), "past the group's reach a read ended: {answers:?}");
    assert!(e.readers_of(&members[0]).any(), "the engine stopped waiting on the lost block: nothing would ask for it again");
    assert_eq!(e.repair_counts().1, 0, "a group short of k rebuilt something");
    println!("  {} lost: {} read(s) waiting, block still awaited, repairs {:?}", PARITY + 1, keys.len(), e.repair_counts());
}

/// THE CONTROL: the same network, repair off -- one lost member is a read
/// that is not answered (it waits on the block). The passes above are the
/// repair's.
#[test]
fn control_with_repair_off_one_lost_block_is_not_read() {
    let records = records();
    let (root, mut all) = tree(&records);
    let (members, _) = a_leaf_group(&mut all, root);
    let lost: BTreeSet<Cid> = [members[0]].into_iter().collect();
    let keys = keys_in(&all, &members[..1]);
    let (answers, e) = read_cold(root, &all, &lost, &keys, Params { repair_reads: false, ..Params::default() });
    assert!(answers.is_empty(), "with repair off a lost block was answered: {answers:?}");
    assert!(e.readers_of(&members[0]).any());
    assert_eq!(e.repair_counts(), (0, 0, 0));
}

/// A NETWORK THAT SENDS RUBBISH for a group block cannot plant a value: a
/// block whose bytes do not hash to its slot counts as missing, and the
/// rebuilt block is kept only if it hashes to the id asked for.
#[test]
fn a_group_block_that_does_not_hash_to_its_slot_is_not_used() {
    let records = records();
    let (root, mut all) = tree(&records);
    let (members, parity) = a_leaf_group(&mut all, root);
    // Every parity block answers with the wrong bytes, and one member is lost:
    // the member alone is k-1 of the members, so without honest parity the
    // group cannot reach k.
    let mut forged = MemBlocks::default();
    let mut ids = BTreeSet::new();
    let mut stack = vec![root];
    while let Some(c) = stack.pop() {
        if !ids.insert(c) {
            continue;
        }
        if let Some(b) = all.get(&c) {
            forged.insert(c, b);
            if let Ok(n) = Node::parse(b) {
                if !n.is_leaf() {
                    stack.extend((0..n.len()).map(|i| n.child(i).0));
                }
            }
        }
    }
    // Plausible forgeries: the REAL parity with one byte flipped -- the same
    // length, so the solve succeeds and yields a wrong member. Only the hash
    // checks can refuse it (the slot's, then the rebuilt block's).
    for p in &parity {
        let mut b = all.get(p).expect("the group's parity").to_vec();
        let last = b.len() - 1;
        b[last] ^= 0x5A;
        forged.insert(*p, &b);
    }
    let lost: BTreeSet<Cid> = [members[0]].into_iter().collect();
    let keys = keys_in(&all, &members[..1]);
    let (answers, e) = read_cold(root, &forged, &lost, &keys, Params::default());
    assert!(answers.is_empty(), "forged parity answered a read: {answers:?}");
    assert_eq!(e.repair_counts().1, 0, "a block was rebuilt from forged parity");
    assert!(e.readers_of(&members[0]).any(), "the lost block is no longer awaited");
}

/// sdk#405, RULE 11 FOR A WRITE (rule 4: one read path): a write whose path needs a block the network LOST is
/// applied from the block REBUILT from its group, as a read of it would be -- it never waits on the straggler.
/// A cold engine over the tree writes a key in a lost leaf: the leaf is answered NotFound every time it is asked
/// (it never arrives), and the write is Published with the leaf rebuilt from the other members and the parity.
/// THE CONTROL: repair off, the same write stays parked (the page would keep re-asking).
#[test]
fn a_write_whose_path_lost_a_block_is_applied_from_its_group() {
    let records = records();
    let (root, mut all) = tree(&records);
    let (members, _) = a_leaf_group(&mut all, root);
    let lost: BTreeSet<Cid> = BTreeSet::from([members[0]]);
    let key = keys_in(&all, &members[..1]).into_iter().next().expect("a key in the lost leaf");
    for repair in [true, false] {
        let params = Params { repair_reads: repair, ..Params::default() };
        let (mut e, store) = common::cold_reader(root, params);
        let mut asked: BTreeMap<Cid, usize> = BTreeMap::new();
        let mut published = false;
        let mut arrived_lost = false;
        let mut queue = e.step(Event::forced_write(ClientId(1), engine::WriteId(1), vec![(key.clone(), engine::Op::Put(b"written over a lost leaf".to_vec()))]));
        let mut steps = 0;
        while let Some(f) = queue.pop() {
            steps += 1;
            assert!(steps < 50_000, "repair={repair}: the write did not settle");
            match f {
                Effect::FetchBlock { id, .. } => {
                    let times = asked.entry(id).or_insert(0);
                    *times += 1;
                    if *times > PACED {
                        continue;
                    }
                    let ev = match all.get(&id).filter(|_| !lost.contains(&id)) {
                        Some(b) => {
                            store.put(id, b);
                            Event::BlockArrived { id, bytes: b.to_vec() }
                        }
                        None => Event::BlockMissed(id),
                    };
                    arrived_lost |= lost.contains(&id) && matches!(ev, Event::BlockArrived { .. });
                    queue.extend(e.step(ev));
                }
                Effect::Keep { id, bytes } => store.put(id, &bytes),
                Effect::PutBlock { id, bytes, .. } => {
                    store.put(id, &bytes);
                    queue.extend(e.step(Event::PutConfirmed(id)));
                }
                Effect::PutRepaired { id, bytes } => store.put(id, &bytes),
                // #424's question, answered as the PAGE does (the architect on #413 x #424): held when the node holds it
                // (a confirmed or Held-present block counts toward k), else unknown (its group's deferred parity goes).
                Effect::ConfirmHeld { id } => {
                    let held = store.node_holds(&id) || (all.get(&id).is_some() && !lost.contains(&id));
                    queue.extend(e.step(if held { Event::PutConfirmed(id) } else { Event::HeldUnknown(id) }));
                }
                Effect::UpdateHead { seq, .. } => queue.extend(e.step(Event::HeadConfirmed(seq))),
                Effect::Notify { state: engine::State::Published, .. } => published = true,
                other => common::no_answer_owed(&other),
            }
        }
        let (started, rebuilt, _) = e.repair_counts();
        println!("repair={repair}: published {published}; repairs started {started}, rebuilt {rebuilt}; the lost leaf asked {} time(s)", asked.get(&members[0]).copied().unwrap_or(0));
        assert!(!arrived_lost, "THE SETUP: the lost leaf arrived");
        if repair {
            assert!(published, "a write over a lost leaf waited on the straggler instead of rebuilding it from its group");
            assert!(rebuilt >= 1, "published without a rebuild: the leaf was not needed, the test is vacuous");
        } else {
            assert!(!published, "THE CONTROL: with repair off the write published without the lost leaf");
        }
    }
}

/// THE TABLE CELL (the architect on #413 x #424): a changed group's member THIS page rebuilt and re-put is ABSENT until a
/// Held says present -- its repair PUT's answer confirms nothing (ruling (b)) -- and counts toward k from then. Cold
/// reads of keys in two lost leaves rebuild them from their group (`PutRepaired`); then a write in a SIBLING leaf of the
/// same group changes the group and keeps the rebuilt leaves as UNCHANGED other members, which the commit asks the node
/// about (`ConfirmHeld`), answered as the page does.
/// * (i) the repair PUTs landed: the node holds them -> Held present -> they count -> Published;
/// * (ii) they did not land: Held unknown -> the group's deferred parity is released instead -> still Published (#424's
///   no-deadlock property), and the rebuilt leaves were counted absent.
///
/// In both, ConfirmHeld named the rebuilt leaves (non-vacuity: a write OVER a lost leaf replaces it, and never asks).
#[test]
fn a_member_this_page_rebuilt_counts_toward_k_once_the_node_says_it_holds_it() {
    let records = records();
    let (root, mut all) = tree(&records);
    let (members, _) = a_leaf_group(&mut all, root);
    // TWO lost leaves: with one absent member the write's first wave (its new leaf + one parity) already reaches k, so
    // the released parity would not be needed (measured: the no-release mutant survived); two absent needs it.
    let lost: BTreeSet<Cid> = BTreeSet::from([members[0], members[2]]);
    let rebuilt = [members[0], members[2]];
    let read_keys: Vec<Vec<u8>> = rebuilt.iter().map(|m| keys_in(&all, &[*m]).into_iter().next().expect("a key in a lost leaf")).collect();
    let write_key = keys_in(&all, &members[1..2]).into_iter().next().expect("a key in a sibling leaf");
    for landed in [true, false] {
        let (mut e, store) = common::cold_reader(root, Params { repair_reads: true, ..Params::default() });
        let answer = |id: &Cid, store: &Store| match all.get(id).filter(|_| !lost.contains(id)) {
            Some(b) => {
                store.put(*id, b);
                Event::BlockArrived { id: *id, bytes: b.to_vec() }
            }
            None => Event::BlockMissed(*id),
        };
        // 1. The reads: each lost leaf is rebuilt from its group and re-put (landing on the node only in case (i)).
        let mut repaired = Vec::new();
        for (n, read_key) in read_keys.iter().enumerate() {
        let req = ReqId(n as u64 + 1);
        let mut asked: BTreeMap<Cid, usize> = BTreeMap::new();
        let mut got = None;
        let mut queue = e.step(Event::Get { client: ClientId(1), req_id: req, key: read_key.clone() });
        while let Some(f) = queue.pop() {
            match f {
                Effect::FetchBlock { id, .. } => {
                    let times = asked.entry(id).or_insert(0);
                    *times += 1;
                    if *times <= PACED {
                        queue.extend(e.step(answer(&id, &store)));
                    }
                }
                Effect::Keep { id, bytes } => store.put(id, &bytes),
                // The repair's re-put reaches the NODE only in case (i). (The engine's own store is this `store` too, so
                // the node is modelled apart: `node` below.)
                Effect::PutRepaired { id, .. } => repaired.push(id),
                Effect::Reply { req_id, result, .. } if req_id == req => got = Some(result),
                other => common::no_answer_owed(&other),
            }
        }
        assert!(matches!(got, Some(ReadResult::Value(Some(_)))), "landed={landed}: THE SETUP: the read of a lost leaf was not answered from its group: {got:?}");
        }
        assert!(rebuilt.iter().all(|m| repaired.contains(m)), "landed={landed}: THE SETUP: a lost leaf was not rebuilt and re-put");
        // What the NODE holds: the network less the lost leaf, plus whatever this page put since, plus the rebuilt leaf
        // only if its repair PUT landed.
        let mut put_since: BTreeSet<Cid> = BTreeSet::new();
        let node = |id: &Cid, put_since: &BTreeSet<Cid>| (all.get(id).is_some() && !lost.contains(id)) || put_since.contains(id) || (landed && rebuilt.contains(id));
        // 2. The write in a sibling leaf; the commit asks the node about the group's other members.
        let mut named = Vec::new();
        let mut published = false;
        let mut queue = e.step(Event::forced_write(ClientId(1), engine::WriteId(1), vec![(write_key.clone(), engine::Op::Put(b"written beside a rebuilt leaf".to_vec()))]));
        let mut steps = 0;
        while let Some(f) = queue.pop() {
            steps += 1;
            assert!(steps < 50_000, "landed={landed}: the write did not settle");
            match f {
                Effect::FetchBlock { id, .. } => queue.extend(e.step(answer(&id, &store))),
                Effect::Keep { id, bytes } => store.put(id, &bytes),
                Effect::PutBlock { id, bytes, .. } => {
                    store.put(id, &bytes);
                    put_since.insert(id);
                    queue.extend(e.step(Event::PutConfirmed(id)));
                }
                // As the page answers: held when the node holds it, else unknown.
                Effect::ConfirmHeld { id } => {
                    named.push(id);
                    let held = node(&id, &put_since);
                    queue.extend(e.step(if held { Event::PutConfirmed(id) } else { Event::HeldUnknown(id) }));
                }
                Effect::UpdateHead { seq, .. } => queue.extend(e.step(Event::HeadConfirmed(seq))),
                Effect::Notify { state: engine::State::Published, .. } => published = true,
                other => common::no_answer_owed(&other),
            }
        }
        assert!(rebuilt.iter().all(|m| named.contains(m)), "landed={landed}: VACUOUS: the commit never asked about a rebuilt leaf (asked {} others)", named.len());
        assert!(published, "landed={landed}: a write beside a rebuilt leaf never published");
    }
}

/// DAMAGED IS REACHED IN THE ORDER THE RACE GIVES (the architect on sdk#524): a race asks all `k + m` at once, so the
/// lost slots' NotFounds land BEFORE the read's own block is answered NotFound -- while the repair is still a RACE.
/// `absent` records ANSWERS in every state, so the moment the read's own block misses, the group is named DAMAGED
/// (`Engine::damaged`: `j = k - 1` of its slots not answered NotFound), with no re-ask needed first. With `m` lost it
/// never is. Mutant "record absent only when missed" -> red (the racing NotFounds are forgotten; j over-counted).
#[test]
fn m_plus_one_lost_is_named_damaged_even_when_every_slot_misses_before_the_block_itself() {
    let records = records();
    let (root, mut all) = tree(&records);
    let (members, parity) = a_leaf_group(&mut all, root);
    let k = members.len();
    let own = members[0];
    let key = keys_in(&all, &[own])[0].clone();
    for (lost_n, want_damaged) in [(PARITY, false), (PARITY + 1, true)] {
        // `lost_n` of the group lost: the read's own member first, then members, then parity.
        let lost: BTreeSet<Cid> = members.iter().chain(parity.iter()).take(lost_n).copied().collect();
        assert!(lost.contains(&own));
        let (mut e, store) = cold_reader(root, Params::default());
        let mut queue: Vec<Effect> = e.step(Event::Get { client: ClientId(1), req_id: ReqId(0), key: key.clone() });
        let mut own_asked = false;
        let mut steps = 0;
        // Answer EVERYTHING except the read's own block; that one waits until nothing else is left.
        while let Some(f) = queue.pop() {
            steps += 1;
            assert!(steps < 20_000, "the read did not settle");
            match f {
                Effect::FetchBlock { id, .. } if id == own => own_asked = true,
                Effect::FetchBlock { id, .. } => {
                    let ev = match all.get(&id).filter(|_| !lost.contains(&id)) {
                        Some(b) => {
                            store.put(id, b);
                            Event::BlockArrived { id, bytes: b.to_vec() }
                        }
                        None => Event::BlockMissed(id),
                    };
                    queue.extend(e.step(ev));
                }
                Effect::Keep { id, bytes } => store.put(id, &bytes),
                other => common::no_answer_owed(&other),
            }
        }
        assert!(own_asked, "THE SETUP: the read never asked its own block");
        assert!(e.damaged().is_empty(), "{lost_n} lost: named damaged BEFORE the read's own block was answered");
        // Now the read's own block misses: the race becomes a repair. Checked in THIS step, before any re-ask lands.
        let _ = e.step(Event::BlockMissed(own));
        let damaged = e.damaged();
        if want_damaged {
            assert_eq!(damaged.len(), 1, "m + 1 lost, every other slot answered first: not named damaged: {damaged:?}");
            assert_eq!((damaged[0].block, damaged[0].j, damaged[0].k), (own, k + PARITY - lost_n, k), "the wrong block, or the wrong j of k");
            assert!(damaged[0].j < k);
        } else {
            assert!(damaged.is_empty(), "m lost is recoverable, yet named damaged: {damaged:?}");
        }
        println!("  {lost_n} lost of k = {k} + {PARITY}: damaged {damaged:?}");
    }
}

/// A tree that is ONE leaf of referenced values (1400 bytes each; the records in `same` all hold the SAME value, so
/// one id fills several slots of the leaf's group), with the leaf's parity put on the network: the records, the root,
/// every block, and the group's `k` data slots then its parity.
/// The records, the root, every block, the group's slots (k data, then parity), and k.
type ValueGroup = (BTreeMap<Vec<u8>, Vec<u8>>, Cid, MemBlocks, Vec<Cid>, usize);

fn a_value_group(same: &[u32]) -> ValueGroup {
    let records: BTreeMap<Vec<u8>, Vec<u8>> = (0..12u32)
        .map(|i| {
            let mut v = vec![b'a' + (i % 20) as u8; 1400];
            if same.contains(&i) {
                v = vec![b'z'; 1400];
            } else {
                v[..4].copy_from_slice(&i.to_be_bytes());
            }
            (format!("v/{i:04}").into_bytes(), v)
        })
        .collect();
    let (root, mut all) = tree(&records);
    let bytes = all.get(&root).expect("held").to_vec();
    let n = Node::parse(&bytes).expect("a node");
    assert!(n.is_leaf(), "THE SETUP: the tree is not one leaf");
    let ids: Vec<Cid> = n.parity().collect();
    for (id, b) in freenet_prolly::parity::blocks_of(&n, &all).expect("every member held") {
        all.insert(id, &b);
    }
    let (g, (_, members)) = freenet_prolly::parity::group_members(&n).into_iter().enumerate().max_by_key(|(_, (_, m))| m.len()).expect("a group");
    let k = members.len();
    let mut slots = members;
    slots.extend_from_slice(&ids[PARITY * g..PARITY * (g + 1)]);
    (records, root, all, slots, k)
}

/// Reads of `keys` (one each, `ReqId` = its index) on a cold reader that already holds the root (so no read has had a
/// block arrive): every block asked is answered from `all` -- NotFound when in `lost` -- except each read's OWN block
/// (`owns`), held until nothing else is left, and the blocks in `pending`, never answered (still in flight). Then each
/// own block is answered NotFound, one step each, nothing after. The engine, as it stands then.
fn reads_holding_their_own(root: Cid, all: &MemBlocks, lost: &BTreeSet<Cid>, pending: &BTreeSet<Cid>, keys: &[Vec<u8>], owns: &[Cid]) -> Engine<Store> {
    let (mut e, store) = cold_reader(root, Params::default());
    store.put(root, all.get(&root).expect("the root"));
    let mut queue: Vec<Effect> = Vec::new();
    for (n, key) in keys.iter().enumerate() {
        queue.extend(e.step(Event::Get { client: ClientId(1), req_id: ReqId(n as u64), key: key.clone() }));
    }
    let mut asked = BTreeSet::new();
    let mut steps = 0;
    while let Some(f) = queue.pop() {
        steps += 1;
        assert!(steps < 20_000, "the reads did not settle");
        match f {
            Effect::FetchBlock { id, .. } if owns.contains(&id) || pending.contains(&id) => {
                asked.insert(id);
            }
            Effect::FetchBlock { id, .. } => {
                // A lost block is answered NotFound ONCE (its re-ask is paced: held here, as a page holds it).
                if !asked.insert(id) && lost.contains(&id) {
                    continue;
                }
                let ev = match all.get(&id).filter(|_| !lost.contains(&id)) {
                    Some(b) => {
                        store.put(id, b);
                        Event::BlockArrived { id, bytes: b.to_vec() }
                    }
                    None => Event::BlockMissed(id),
                };
                queue.extend(e.step(ev));
            }
            Effect::Keep { id, bytes } => store.put(id, &bytes),
            other => common::no_answer_owed(&other),
        }
    }
    for own in owns {
        assert!(asked.contains(own), "THE SETUP: a read never asked its own block");
        let _ = e.step(Event::BlockMissed(*own));
    }
    e
}

/// The first `n` SLOTS' ids, taking `first` first: every slot holding a taken id counts.
fn lose_slots(slots: &[Cid], first: &[Cid], n: usize) -> BTreeSet<Cid> {
    let mut lost = BTreeSet::new();
    let mut taken = 0;
    for id in first.iter().chain(slots.iter()) {
        let holds = slots.iter().filter(|s| *s == id).count();
        if !lost.contains(id) && taken + holds <= n {
            lost.insert(*id);
            taken += holds;
        }
    }
    assert_eq!(taken, n, "THE SETUP: {n} slots could not be lost");
    lost
}

/// ONE ID IN TWO SLOTS, ABSENT (Codex on sdk#542): identical values are one block, so a group can list the same id in
/// two slots, and a NotFound for it answers BOTH. m + 1 SLOTS lost with the read's own block the repeated one: DAMAGED
/// at j = k - 1 the moment the own block misses; m slots lost: nothing named.
#[test]
fn an_id_in_two_slots_missing_is_absent_in_both() {
    let (records, root, all, slots, k) = a_value_group(&[0, 5]);
    let key = b"v/0000".to_vec();
    let own = freenet_prolly::block_id(freenet_prolly::kind::RAW, &records[&key]);
    assert_eq!(slots[..k].iter().filter(|s| **s == own).count(), 2, "THE SETUP: the read's own value is not in two slots");
    for (n, want) in [(PARITY, false), (PARITY + 1, true)] {
        let lost = lose_slots(&slots, &[own], n);
        let e = reads_holding_their_own(root, &all, &lost, &BTreeSet::new(), std::slice::from_ref(&key), &[own]);
        let damaged = e.damaged();
        if want {
            assert_eq!(damaged.len(), 1, "m + 1 slots lost with one id in two of them: not named damaged: {damaged:?}");
            assert_eq!((damaged[0].block, damaged[0].j, damaged[0].k), (own, k - 1, k));
        } else {
            assert!(damaged.is_empty(), "m slots lost is recoverable, yet named damaged: {damaged:?}");
        }
    }
}

/// ONE ID IN TWO SLOTS, HELD: a block that arrives fills EVERY slot of its id. Exactly `k` slots survive, two of them
/// the repeated id: the lost member is rebuilt and read right -- counted once, the group would stay one short for ever.
#[test]
fn an_id_in_two_slots_arriving_fills_both() {
    let (records, root, all, slots, k) = a_value_group(&[0, 5]);
    let twice = freenet_prolly::block_id(freenet_prolly::kind::RAW, &records[&b"v/0000".to_vec()]);
    let key = b"v/0001".to_vec();
    let own = freenet_prolly::block_id(freenet_prolly::kind::RAW, &records[&key]);
    assert!(own != twice && slots[..k].contains(&own), "THE SETUP: the read's own value is not a member");
    let others: Vec<Cid> = slots.iter().filter(|s| **s != twice && **s != own).copied().collect();
    let mut lost = BTreeSet::from([own]);
    lost.extend(others.iter().take(PARITY - 1));
    let left = slots.iter().filter(|s| !lost.contains(*s)).count();
    assert_eq!((left, slots.iter().filter(|s| !lost.contains(*s) && **s == twice).count()), (k, 2), "THE SETUP: not exactly k left with the repeated id twice");
    let (answers, _) = read_cold(root, &all, &lost, std::slice::from_ref(&key), Params::default());
    assert!(answers.contains_key(&key) && right(&answers, &records).is_empty(), "a repeated id counted once: {answers:?}");
}

/// A REPAIR NOBODY READS IS NOT NAMED (Codex on sdk#542): two sibling reads, each own block lost with m + 1 of the
/// group, each repair asking the other's block -- so after both reads are superseded, each repair still "wants" a
/// block of the other's and neither ends. `damaged()` names only a repair a live READ waits on.
#[test]
fn a_repair_with_no_read_left_is_not_named_damaged() {
    let (records, root, all, slots, k) = a_value_group(&[]);
    let keys = [b"v/0001".to_vec(), b"v/0002".to_vec()];
    let owns: Vec<Cid> = keys.iter().map(|key| freenet_prolly::block_id(freenet_prolly::kind::RAW, &records[key])).collect();
    assert!(owns.iter().all(|o| slots[..k].contains(o)), "THE SETUP: the two values are not members of the group");
    let lost = lose_slots(&slots, &owns, PARITY + 1);
    let mut e = reads_holding_their_own(root, &all, &lost, &BTreeSet::new(), &keys, &owns);
    assert_eq!(e.damaged().len(), 2, "THE SETUP: both reads' groups are not named damaged: {:?}", e.damaged());
    for r in 0..2 {
        assert!(e.supersede_read(ReqId(r)).is_some(), "THE SETUP: read {r} was not superseded");
    }
    let damaged = e.damaged();
    assert!(damaged.is_empty(), "named damaged with no read left: {damaged:?}");
}

/// THE RECOVERABLE BOUNDARY WHILE SURVIVORS ARE STILL PENDING (Codex on sdk#542): m slots answered NotFound, the k
/// survivors not yet answered at all -- `present` (not answered NotFound) is exactly k, margin 0: DEGRADED, never
/// DAMAGED. (The other test answers the survivors first, so its repair has already rebuilt; this one has not.)
/// Mutant "margin 0 is DAMAGED" -> red here. m + 1 with the survivors pending: DAMAGED at j = k - 1.
#[test]
fn m_lost_with_every_survivor_pending_is_not_damaged() {
    let (records, root, all, slots, k) = a_value_group(&[]);
    let key = b"v/0003".to_vec();
    let own = freenet_prolly::block_id(freenet_prolly::kind::RAW, &records[&key]);
    for (n, want) in [(PARITY, false), (PARITY + 1, true)] {
        let lost = lose_slots(&slots, &[own], n);
        let pending: BTreeSet<Cid> = slots.iter().filter(|s| !lost.contains(*s)).copied().collect();
        let e = reads_holding_their_own(root, &all, &lost, &pending, std::slice::from_ref(&key), &[own]);
        let damaged = e.damaged();
        if want {
            assert_eq!(damaged.iter().map(|d| (d.block, d.j, d.k)).collect::<Vec<_>>(), vec![(own, k - 1, k)], "m + 1 lost, survivors pending");
        } else {
            assert!(damaged.is_empty(), "m lost, the k survivors pending (margin 0): named damaged: {damaged:?}");
        }
    }
}

/// PARITY LOST, EVERY MEMBER THERE (engineer3 for sdk#479): the node lost 2 of a group's parity blocks and none of its
/// members. A cold reader reading every key the group's members hold races the group, sees the two parity blocks
/// answered NotFound, and -- once it holds all k members -- re-encodes and PUTs both back, byte for byte what the save
/// wrote. No other parity block is PUT; any other PUT is a racing read's rebuild of a member (the race's own: a read
/// whose k other slots land before its own block rebuilds it), byte for byte the block the save wrote. `parity_counts`
/// = (2 missing, 2 put, 0 mismatched). Mutants: the owed record dropped, or `put_owed_parity` never called -> red.
#[test]
fn two_parity_lost_with_every_member_there_are_put_back_byte_for_byte() {
    let records = records();
    let (root, mut all) = tree(&records);
    let (members, parity) = a_leaf_group(&mut all, root);
    let lost: BTreeSet<Cid> = parity.iter().take(2).copied().collect();
    let keys = keys_in(&all, &members);
    let (mut e, store) = cold_reader(root, Params::default());
    let mut put: BTreeMap<Cid, Vec<u8>> = BTreeMap::new();
    let mut seen: BTreeSet<Cid> = BTreeSet::new();
    for (n, key) in keys.iter().enumerate() {
        // Paced per read, as `read_cold` paces: each read is its own stretch of time.
        let mut asked: BTreeMap<Cid, usize> = BTreeMap::new();
        let mut queue = e.step(Event::Get { client: ClientId(1), req_id: ReqId(n as u64), key: key.clone() });
        let mut steps = 0;
        while let Some(f) = queue.pop() {
            steps += 1;
            assert!(steps < 20_000, "a read did not settle");
            match f {
                Effect::FetchBlock { id, .. } => {
                    seen.insert(id);
                    let times = asked.entry(id).or_insert(0);
                    *times += 1;
                    if *times > PACED {
                        continue;
                    }
                    let ev = match all.get(&id).filter(|_| !lost.contains(&id)) {
                        Some(b) => {
                            store.put(id, b);
                            Event::BlockArrived { id, bytes: b.to_vec() }
                        }
                        None => Event::BlockMissed(id),
                    };
                    queue.extend(e.step(ev));
                }
                Effect::Keep { id, bytes } => store.put(id, &bytes),
                Effect::PutRepaired { id, bytes } => {
                    assert!(put.insert(id, bytes).is_none(), "block {:?} PUT back twice", &id[..4]);
                }
                Effect::Reply { .. } => {}
                other => common::no_answer_owed(&other),
            }
        }
    }
    assert!(lost.iter().all(|p| seen.contains(p)), "THE SETUP: the reads never asked the lost parity");
    let want: BTreeMap<Cid, Vec<u8>> = lost.iter().map(|p| (*p, all.get(p).expect("the save wrote it").to_vec())).collect();
    let short = |c: &Cid| core_types::hex::encode(&c[..4]);
    for (id, bytes) in &want {
        assert_eq!(put.get(id), Some(bytes), "lost parity {} was not PUT back byte for byte; counts {:?}", short(id), e.parity_counts());
    }
    for (id, bytes) in put.iter().filter(|(id, _)| !lost.contains(*id)) {
        assert!(!parity.contains(id), "parity {} the node HOLDS was PUT back", short(id));
        assert_eq!(Some(bytes.as_slice()), all.get(id), "{} was PUT back with other bytes than the save wrote", short(id));
    }
    assert_eq!(e.parity_counts(), (2, 2, 0));
}
