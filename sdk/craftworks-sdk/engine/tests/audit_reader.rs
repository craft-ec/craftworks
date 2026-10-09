//! THE AUDIT IS A READER (sdk#530; the architect's H1; WANTED-LIFE W1, W7, W8, A1-A3): the assets audit's pass is one
//! more holder of a block's (block, GET) key, so no other reader's end can end the GET it waits on; only the NODE's
//! answer serves it -- never bytes the page holds or rebuilt; and it pins nothing.
use engine::{AuditVerdict, ClientId, Effect, Event, PagePins, PassId, Params};
use freenet_prolly::store::Blocks;
use freenet_prolly::Cid;
use std::collections::{BTreeMap, BTreeSet};

mod common;
use common::{cold_reader, tree};

fn records() -> BTreeMap<Vec<u8>, Vec<u8>> {
    (0..600u32).map(|i| (format!("a/{i:05}").into_bytes(), format!("value {i}").into_bytes())).collect()
}

fn fetched(fx: &[Effect]) -> Vec<Cid> {
    fx.iter().filter_map(|f| if let Effect::FetchBlock { id, .. } = f { Some(*id) } else { None }).collect()
}

fn unwanted(fx: &[Effect], id: Cid) -> bool {
    fx.iter().any(|f| matches!(f, Effect::Unwanted { id: x } if *x == id))
}

fn answered(fx: &[Effect]) -> Vec<(Cid, Vec<PassId>, AuditVerdict)> {
    fx.iter()
        .filter_map(|f| if let Effect::AuditAnswered { id, passes, verdict } = f { Some((*id, passes.clone(), *verdict)) } else { None })
        .collect()
}

/// A cold reader whose read of one key is parked on the root: the block it waits on, and the read's id.
fn a_parked_read() -> (engine::Engine<common::Store>, Cid, engine::read::ReqId) {
    let (root, _) = tree(&records());
    let (mut e, _) = cold_reader(root, Params::default());
    let req = engine::read::ReqId(1);
    let fx = e.step(Event::Get { client: ClientId(1), req_id: req, key: b"a/00001".to_vec() });
    let wait = *fetched(&fx).first().expect("THE SETUP: the read asked no block");
    (e, wait, req)
}

/// W1, BOTH ORDERS: an app read and an audit on one block. Whichever leaves first, the GET survives (no `Unwanted`,
/// the other still a reader); the last to leave ends it. Mutant "the audit not counted in any()" -> red.
#[test]
fn a_read_and_an_audit_on_one_block_each_keep_the_get_until_both_leave() {
    for read_leaves_first in [true, false] {
        let (mut e, b, req) = a_parked_read();
        let joined = e.step(Event::AuditWant { id: b, pass: PassId(7) });
        assert!(fetched(&joined).is_empty(), "the audit asked a second GET of a block a read already asks (W5)");
        let first = if read_leaves_first { e.supersede_read(req).expect("the read is superseded") } else { e.step(Event::AuditDrop { id: b, pass: PassId(7) }) };
        assert!(!unwanted(&first, b), "read_leaves_first={read_leaves_first}: the GET ended while the other reader still waits");
        assert!(e.readers_of(&b).any(), "read_leaves_first={read_leaves_first}: the block lost its remaining reader");
        assert_eq!(e.readers_of(&b).audits, read_leaves_first, "read_leaves_first={read_leaves_first}: the wrong reader remains");
        let last = if read_leaves_first { e.step(Event::AuditDrop { id: b, pass: PassId(7) }) } else { e.supersede_read(req).expect("the read is superseded") };
        assert!(unwanted(&last, b), "read_leaves_first={read_leaves_first}: the last reader left and the GET was not ended");
    }
}

/// A2: the NODE's answer answers every pass waiting, found, and takes them. Mutant "served not taking the audits" ->
/// red (the pass never hears).
#[test]
fn the_nodes_answer_answers_the_audit_found() {
    let (root, all) = tree(&records());
    let (mut e, _) = cold_reader(root, Params::default());
    let fx = e.step(Event::AuditWant { id: root, pass: PassId(3) });
    assert_eq!(fetched(&fx), vec![root], "the audit's want asked no GET");
    let bytes = all.get(&root).expect("the root").to_vec();
    let fx = e.step(Event::BlockArrived { id: root, bytes });
    assert_eq!(answered(&fx), vec![(root, vec![PassId(3)], AuditVerdict::Present)], "the node's answer did not answer the audit");
    assert!(!e.readers_of(&root).audits, "the answered audit is still a reader");
}

/// A3: a NotFound answers the audit ABSENT and takes it; the read waiting on the same block keeps waiting (rule 8).
#[test]
fn a_not_found_answers_the_audit_absent_and_the_read_keeps_waiting() {
    let (mut e, b, _) = a_parked_read();
    let _ = e.step(Event::AuditWant { id: b, pass: PassId(4) });
    let fx = e.step(Event::BlockMissed(b));
    assert_eq!(answered(&fx), vec![(b, vec![PassId(4)], AuditVerdict::Absent)], "the NotFound did not answer the audit absent");
    assert!(!e.readers_of(&b).audits);
    assert!(e.readers_of(&b).reads, "the read stopped waiting on a NotFound");
}

/// W7: a HELD block's audit still asks the node (held never suppresses an audit's ask), and a LOCAL landing -- the page
/// serving held bytes, or a rebuild -- answers NO audit; only the node's answer does. Mutant "E2r answers audits" -> red.
#[test]
fn only_the_nodes_answer_serves_an_audit_never_held_bytes() {
    let (root, all) = tree(&records());
    let (mut e, store) = cold_reader(root, Params::default());
    let bytes = all.get(&root).expect("the root").to_vec();
    store.put(root, &bytes);
    let fx = e.step(Event::AuditWant { id: root, pass: PassId(5) });
    assert_eq!(fetched(&fx), vec![root], "a held block's audit asked no GET: held suppressed it");
    let local = e.step(Event::BlockLanded { id: root, bytes: bytes.clone() });
    assert!(answered(&local).is_empty(), "a LOCAL landing answered the audit: {:?}", answered(&local));
    assert!(e.readers_of(&root).audits, "the audit stopped waiting on a local landing");
    let node = e.step(Event::BlockArrived { id: root, bytes });
    assert_eq!(answered(&node), vec![(root, vec![PassId(5)], AuditVerdict::Present)], "the node's answer did not answer the audit");
}

/// W8: an audit pins nothing -- an audit of N blocks leaves the engine's pins as they were.
#[test]
fn an_audit_pins_nothing() {
    let (root, all) = tree(&records());
    let (mut e, _) = cold_reader(root, Params::default());
    let none = BTreeSet::new();
    let before = e.pins(&PagePins { held_asks: &none });
    let n = freenet_prolly::node::Node::parse(all.get(&root).expect("the root")).expect("a node");
    let blocks: Vec<Cid> = std::iter::once(root).chain((0..n.len().min(5)).filter(|_| !n.is_leaf()).map(|i| n.child(i).0)).collect();
    for (i, b) in blocks.iter().enumerate() {
        let _ = e.step(Event::AuditWant { id: *b, pass: PassId(i as u64) });
        let bytes = all.get(b).expect("held in the fixture").to_vec();
        let _ = e.step(Event::BlockArrived { id: *b, bytes });
    }
    assert_eq!(e.pins(&PagePins { held_asks: &none }), before, "an audit of {} blocks changed the pins", blocks.len());
}

/// A PACK's answer is the node's answer for the PACK only (the second reviewer on #538): a member expanded from it is a
/// LOCAL landing for an audit (E2r). Audit member M; the node answers a pack P holding M while M's own GET is
/// unanswered: M's audit is NOT answered -- it still waits for the node's answer to M.
#[test]
fn a_packs_answer_does_not_answer_its_members_audit() {
    let member = b"a member block".to_vec();
    let m = freenet_prolly::block_id(freenet_prolly::kind::RAW, &member);
    let pack = engine::pack::build(&[(freenet_prolly::kind::RAW, member)]).expect("a pack");
    let p = engine::pack::pack_id(&pack);
    let (root, _) = tree(&records());
    let (mut e, _) = cold_reader(root, Params::default());
    let _ = e.step(Event::AuditWant { id: m, pass: PassId(9) });
    let fx = e.step(Event::BlockArrived { id: p, bytes: pack });
    assert!(answered(&fx).is_empty(), "the pack's answer answered its member's audit: {:?}", answered(&fx));
    assert!(e.readers_of(&m).audits, "the member's audit stopped waiting on the pack's answer");
}

/// A REBUILD is a LOCAL landing (E2r) for an audit (the second reviewer on #538: land_rebuilt -> on_arrived(.., true)
/// must go red). A read's block M is answered NotFound and its group rebuilds it; an audit that joined M meanwhile is
/// NOT answered by the rebuild -- it waits for the node's answer to M.
#[test]
fn a_rebuild_does_not_answer_an_audit() {
    let records: BTreeMap<Vec<u8>, Vec<u8>> = (0..6000u32).map(|i| (format!("k/{i:06}").into_bytes(), format!("value {i}").into_bytes())).collect();
    let (root, mut all) = tree(&records);
    // A leaf group with its parity on the "network", as read_repair's fixture does.
    let mut at = root;
    let (members, _) = loop {
        let bytes = all.get(&at).expect("held").to_vec();
        let n = freenet_prolly::node::Node::parse(&bytes).expect("a node");
        if n.level() == 1 {
            for (id, b) in freenet_prolly::parity::blocks_of(&n, &all).expect("every member held") {
                all.insert(id, &b);
            }
            let (_, (_, m)) = freenet_prolly::parity::group_members(&n).into_iter().enumerate().max_by_key(|(_, (_, m))| m.len()).expect("a group");
            break (m, ());
        }
        at = n.child(0).0;
    };
    let (mut e, store) = cold_reader(root, Params::default());
    // The read of every key under the group's first member; the block LOST is whichever group member the read asks
    // first (so it is one the read needs).
    let first = freenet_prolly::node::Node::parse(all.get(&members[0]).expect("held")).expect("a leaf");
    let mut queue = e.step(Event::Get { client: ClientId(1), req_id: engine::read::ReqId(1), key: first.key(0) });
    let mut lost: Option<Cid> = None;
    let mut joined = false;
    let mut rebuilt_answered = Vec::new();
    let mut guard = 0;
    let mut asked: BTreeMap<Cid, usize> = BTreeMap::new();
    loop {
        // THE ORDER THAT MATTERS: the first time the read asks a member of the group (a race asks its slots in the
        // same step), that member is answered NotFound FIRST and the audit joins it -- before any slot is answered, so
        // the rebuild lands while the audit waits.
        if lost.is_none() {
            if let Some(m) = queue.iter().find_map(|f| match f {
                Effect::FetchBlock { id, .. } if members.contains(id) => Some(*id),
                _ => None,
            }) {
                lost = Some(m);
                queue.retain(|f| !matches!(f, Effect::FetchBlock { id, .. } if *id == m));
                let fx = e.step(Event::BlockMissed(m));
                rebuilt_answered.extend(answered(&fx));
                queue.extend(fx);
                joined = true;
                // From here the node is SILENT on the lost block (its re-asks held, as a paced page holds them): the
                // audit waits, and the only thing that can land it is the rebuild.
                asked.insert(m, usize::MAX / 2);
                let fx = e.step(Event::AuditWant { id: m, pass: PassId(11) });
                queue.extend(fx);
            }
        }
        let Some(f) = queue.pop() else { break };
        guard += 1;
        assert!(guard < 20_000, "the read did not settle");
        match f {
            Effect::FetchBlock { id, .. } => {
                // Paced as read_repair's harness paces a page's re-asks: past 3 asks of one block, the later ones are
                // held (a silent node), never answered in a loop.
                let times = asked.entry(id).or_insert(0usize);
                *times += 1;
                if *times > 3 {
                    continue;
                }
                let ev = if Some(id) == lost {
                    Event::BlockMissed(id)
                } else {
                    let b = all.get(&id).expect("the fixture holds it").to_vec();
                    store.put(id, &b);
                    Event::BlockArrived { id, bytes: b }
                };
                let fx = e.step(ev);
                rebuilt_answered.extend(answered(&fx));
                if e.repair_counts().1 > 0 {
                    break;
                }
                queue.extend(fx);
            }
            Effect::Keep { id, bytes } => store.put(id, &bytes),
            other => common::no_answer_owed(&other),
        }
    }
    let lost = lost.expect("THE SETUP: the read asked no member of the group");
    assert!(joined, "THE SETUP: the lost block was never asked");
    assert!(e.repair_counts().1 > 0, "THE SETUP: nothing was rebuilt");
    assert!(rebuilt_answered.iter().all(|(id, _, _)| *id != lost), "a REBUILD answered the audit: {rebuilt_answered:?}");
    assert!(e.readers_of(&lost).audits, "the audit stopped waiting on a rebuild");
}

/// W8 while PENDING (the second reviewer on #538): pins are unchanged with audits waiting -- after their wants, and
/// after a local landing that answers none of them.
#[test]
fn pending_audits_pin_nothing_even_after_a_local_landing() {
    let (root, all) = tree(&records());
    let (mut e, _) = cold_reader(root, Params::default());
    let none = BTreeSet::new();
    let before = e.pins(&PagePins { held_asks: &none });
    let n = freenet_prolly::node::Node::parse(all.get(&root).expect("the root")).expect("a node");
    let blocks: Vec<Cid> = std::iter::once(root).chain((0..n.len().min(4)).filter(|_| !n.is_leaf()).map(|i| n.child(i).0)).collect();
    for (i, b) in blocks.iter().enumerate() {
        let _ = e.step(Event::AuditWant { id: *b, pass: PassId(20 + i as u64) });
    }
    assert_eq!(e.pins(&PagePins { held_asks: &none }), before, "pending audits changed the pins");
    for b in &blocks {
        let _ = e.step(Event::BlockLanded { id: *b, bytes: all.get(b).expect("held").to_vec() });
        assert!(e.readers_of(b).audits, "a local landing answered an audit");
    }
    assert_eq!(e.pins(&PagePins { held_asks: &none }), before, "audits pending after a local landing changed the pins");
}
