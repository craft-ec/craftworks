//! ONE DEFINITION OF "IS THIS BLOCK STILL WANTED" (sdk#480, the structure sweep's Tier-1 #1). A block is wanted
//! while a read waits on it, a repair needs it as a slot, or the parked write needs it. A block nobody wants is
//! WITHDRAWN: the page ends its GET and does not keep it if it arrives late. Withdrawing a block that is still
//! wanted drops the GET its owner is waiting on.

use engine::read::ReqId;
use engine::{ClientId, Effect, Event, Op, Params, WriteId};
use freenet_prolly::node::Node;
use freenet_prolly::store::Blocks;
use freenet_prolly::Cid;
use std::collections::BTreeMap;

mod common;
use common::{cold_reader, tree};

/// Did these effects tell `id` Unwanted (WANTED-LIFE: the page then ends its GET)?
fn unwanted(fx: &[Effect], id: Cid) -> bool {
    fx.iter().any(|f| matches!(f, Effect::Unwanted { id: u } if *u == id))
}

fn fetches(fx: &[Effect]) -> Vec<Cid> {
    fx.iter().filter_map(|f| if let Effect::FetchBlock { id, .. } = f { Some(*id) } else { None }).collect()
}

/// **A block the parked write still needs is NOT withdrawn when a read that also waited on it is superseded.**
/// The write parks on a cold block and asks for it; a read of the same key waits on the same block (one fetch,
/// shared); the read is superseded. Before sdk#480 `withdraw_if_unwanted` looked only at waiting READS, so the
/// block was withdrawn under the parked write: its GET ended, and the write waited on a fetch nobody would make.
#[test]
fn a_block_the_parked_write_needs_is_not_withdrawn_when_a_read_on_it_is_superseded() {
    let records: BTreeMap<Vec<u8>, Vec<u8>> = (0..400u32).map(|i| (format!("k/{i:05}").into_bytes(), vec![(i % 251) as u8; 40])).collect();
    let (root, all) = tree(&records);
    let (mut e, store) = cold_reader(root, Params::default());
    store.put(root, all.get(&root).expect("the root"));
    let out = e.step(Event::forced_write(ClientId(1), WriteId(1), vec![(b"k/00100".to_vec(), Op::Put(b"new".to_vec()))]));
    let asked = fetches(&out);
    assert!(!asked.is_empty(), "THE SETUP: the write did not park on a cold block");
    let needed = asked[0];
    let _ = e.step(Event::Get { client: ClientId(2), req_id: ReqId(7), key: b"k/00100".to_vec() });
    let fx = e.supersede_read(ReqId(7)).expect("THE SETUP: the read waiting on the write's block was not superseded");
    assert!(!unwanted(&fx, needed), "a block the parked write still needs was WITHDRAWN when a read on it was superseded");
}

/// A level-1 node's largest group: its members and its parity ids (the parity blocks are NOT put anywhere).
fn a_leaf_group(all: &freenet_prolly::store::MemBlocks, root: Cid) -> (Vec<Cid>, Vec<Cid>) {
    let mut at = root;
    loop {
        let n = Node::parse(all.get(&at).expect("held")).expect("a node");
        assert!(!n.is_leaf(), "the tree is one leaf: no groups");
        if n.level() == 1 {
            let ids: Vec<Cid> = n.parity().collect();
            let groups = freenet_prolly::parity::group_members(&n);
            let m = ids.len() / groups.len();
            let (g, (_, members)) = groups.into_iter().enumerate().max_by_key(|(_, (_, m))| m.len()).expect("a group");
            return (members, ids[m * g..m * g + m].to_vec());
        }
        at = n.child(0).0;
    }
}

/// **The REPAIR of a block the parked write needs stands when the read that raced it is superseded.** A member is
/// cold; the write on its key parks on it; a read of the same key waits on it and RACES its group (its parity is not
/// held, so each parity slot is asked for the repair). The read is superseded: the member is still the parked
/// write's, so its repair -- and the repair's asks -- go on. Mutant "supersede_read ends the repair when no READ
/// waits" -> red: the repair ends and its slots are freed under the write.
#[test]
fn a_repair_the_parked_write_needs_stands_when_the_read_that_raced_it_is_superseded() {
    let records: BTreeMap<Vec<u8>, Vec<u8>> = (0..6000u32).map(|i| (format!("k/{i:06}").into_bytes(), format!("value {i}").into_bytes())).collect();
    let (root, all) = tree(&records);
    let (members, parity) = a_leaf_group(&all, root);
    let member = members[members.len() / 2];
    let (mut e, store) = cold_reader(root, Params::default());
    for (id, bytes) in all.0.iter() {
        if *id != member && !parity.contains(id) {
            store.put(*id, bytes);
        }
    }
    let key = Node::parse(all.get(&member).expect("held")).expect("a leaf").key(0);
    let out = e.step(Event::forced_write(ClientId(1), WriteId(1), vec![(key.clone(), Op::Put(b"new".to_vec()))]));
    assert!(fetches(&out).contains(&member), "THE SETUP: the write did not park on the cold member");
    let _ = e.step(Event::Get { client: ClientId(2), req_id: ReqId(7), key });
    assert!(parity.iter().all(|p| e.readers_of(p).repairs), "THE SETUP: the read did not race the member's group");
    let fx = e.supersede_read(ReqId(7)).expect("THE SETUP: the read was not superseded");
    assert!(parity.iter().all(|p| e.readers_of(p).repairs), "the repair of a block the parked write needs ended when the read that raced it was superseded");
    assert!(parity.iter().all(|p| !unwanted(&fx, *p)), "a slot of a repair the parked write needs was withdrawn");
}

/// **A FREED repair slot the parked write needs is NOT withdrawn.** Members A and B of one group are cold; the write
/// on A's key parks on A; a read of B's key races B's group, so A is also a SLOT of B's repair. B arrives: its repair
/// ends and frees slot A -- which the parked write still needs. Before sdk#480 `withdraw_if_unwanted` saw no READ on
/// A and withdrew it under the write. Mutant "withdraw looks only at reads" -> red.
#[test]
fn a_freed_repair_slot_the_parked_write_needs_is_not_withdrawn() {
    let records: BTreeMap<Vec<u8>, Vec<u8>> = (0..6000u32).map(|i| (format!("k/{i:06}").into_bytes(), format!("value {i}").into_bytes())).collect();
    let (root, all) = tree(&records);
    let (members, parity) = a_leaf_group(&all, root);
    let (a, b) = (members[0], members[members.len() - 1]);
    let (mut e, store) = cold_reader(root, Params::default());
    for (id, bytes) in all.0.iter() {
        if *id != a && *id != b && !parity.contains(id) {
            store.put(*id, bytes);
        }
    }
    let key_of = |m: &Cid| Node::parse(all.get(m).expect("held")).expect("a leaf").key(0);
    let out = e.step(Event::forced_write(ClientId(1), WriteId(1), vec![(key_of(&a), Op::Put(b"new".to_vec()))]));
    assert!(fetches(&out).contains(&a), "THE SETUP: the write did not park on A");
    let _ = e.step(Event::Get { client: ClientId(2), req_id: ReqId(7), key: key_of(&b) });
    // The write's own need of A is shown by its FetchBlock above, never read back through `readers_of` here: the
    // property below is what must see a `readers_of` that ignores the write (the architect on #495).
    assert!(e.readers_of(&a).repairs, "THE SETUP: A is not a slot of B's race");
    let bytes = all.get(&b).expect("held").to_vec();
    store.put(b, &bytes);
    let out = e.step(Event::BlockArrived { id: b, bytes });
    assert!(!e.readers_of(&a).repairs, "THE SETUP: B's repair did not end and free slot A");
    assert!(!unwanted(&out, a), "a freed repair slot the parked write still needs was WITHDRAWN");
}

/// **T3 (WANTED-LIFE, the architect): Unwanted is NET at the call's end.** In ONE step a race frees slot Y and the
/// parked write then parks on Y. X and Y are adjacent leaves of one group. The write DELETES X's last key -- the key
/// that ends X's chunk -- so its apply reads X first and only then finds it must read Y too (X's rest joins Y's chunk).
/// A read of X races X's group, so Y is a slot. X arrives: its race ends and frees Y, and the write -- X now held --
/// re-applies and parks on Y, in the same step. Y is wanted at the step's end: no `Unwanted{Y}`, and the write's
/// fetch of Y goes out. Before WANTED-LIFE, Y went into `withdrawn` at the free and `park_write` never cleared it, so
/// the page's take ended the write's fresh GET. Mutant "Unwanted decided at the drop, not re-checked at the end" -> red.
#[test]
fn a_slot_freed_and_parked_on_in_one_step_is_not_unwanted() {
    let records: BTreeMap<Vec<u8>, Vec<u8>> = (0..6000u32).map(|i| (format!("k/{i:06}").into_bytes(), format!("value {i}").into_bytes())).collect();
    let (root, all) = tree(&records);
    let (members, parity) = a_leaf_group(&all, root);
    let (x, y) = (members[0], members[1]);
    let (mut e, store) = cold_reader(root, Params::default());
    for (id, bytes) in all.0.iter() {
        if *id != x && *id != y && !parity.contains(id) {
            store.put(*id, bytes);
        }
    }
    let leaf = |m: &Cid| Node::parse(all.get(m).expect("held")).expect("a leaf");
    let x_last = leaf(&x).key(leaf(&x).len() - 1);
    let out = e.step(Event::forced_write(ClientId(1), WriteId(1), vec![(x_last, Op::Delete)]));
    assert!(fetches(&out).contains(&x) && !fetches(&out).contains(&y), "THE SETUP: the write did not park on X alone first: {:?}", fetches(&out));
    let _ = e.step(Event::Get { client: ClientId(2), req_id: ReqId(7), key: leaf(&x).key(0) });
    assert!(e.readers_of(&y).repairs, "THE SETUP: Y is not a slot of X's race");
    let bytes = all.get(&x).expect("held").to_vec();
    store.put(x, &bytes);
    let out = e.step(Event::BlockArrived { id: x, bytes });
    assert!(!e.readers_of(&y).repairs, "THE SETUP: X's race did not end and free slot Y");
    assert!(fetches(&out).contains(&y), "THE SETUP: the write did not park on Y in the same step: {:?}", fetches(&out));
    assert!(!unwanted(&out, y), "Y, freed and parked on in one step, was told Unwanted: the page would end the write's GET");
}

/// **A RELEASED parked write's needs are told Unwanted** (WANTED-LIFE part 3: `release_write` drops each need, and a
/// block nobody else wants is `Unwanted` at the call's end). The write parks on a cold block X that no read wants;
/// it hears nothing for its idle bound and is released `Failed` -- and X, now wanted by nobody, is `Unwanted` in that
/// same step, so the page ends its GET rather than re-asking it until its own due-time check. Mutant "release does not
/// drop the write's needs" -> red.
#[test]
fn a_released_parked_writes_needs_are_unwanted() {
    let records: BTreeMap<Vec<u8>, Vec<u8>> = (0..400u32).map(|i| (format!("k/{i:05}").into_bytes(), vec![(i % 251) as u8; 40])).collect();
    let (root, all) = tree(&records);
    let (mut e, store) = cold_reader(root, Params::default());
    store.put(root, all.get(&root).expect("the root"));
    let _ = e.step(Event::Tick(1_000));
    let out = e.step(Event::forced_write(ClientId(1), WriteId(1), vec![(b"k/00100".to_vec(), Op::Put(b"new".to_vec()))]));
    let x = *fetches(&out).first().expect("THE SETUP: the write did not park on a cold block");
    let mut released = Vec::new();
    for t in 1..=10_000u64 {
        let out = e.step(Event::Tick(1_000 + t));
        if out.iter().any(|f| matches!(f, Effect::Notify { state: engine::State::Failed { why: engine::FailWhy::SilentRelease }, .. })) {
            released = out;
            break;
        }
    }
    assert!(!released.is_empty(), "THE SETUP: the silent parked write was never released");
    assert!(unwanted(&released, x), "a released parked write's need, wanted by nobody now, was not told Unwanted: its GET runs on");
}
