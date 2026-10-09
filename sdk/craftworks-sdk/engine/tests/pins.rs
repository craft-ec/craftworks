//! THE ONE PIN RULE (sdk#411): `Engine::pins` names every block whose page copy may not be dropped. These pin the
//! two READ-side classes where a dropped block would be fetched again, successfully, for ever (a livelock): a
//! parked READ's held and waited blocks (P) and a parked WRITE's (PW). Each is the class's own assertion: its
//! mutant ("unpinned") is red here.

use engine::{ClientId, Effect, Engine, Event, Op, PagePins, Params, Pin, WriteId};
use freenet_prolly::store::{Blocks, MemBlocks};
use freenet_prolly::Cid;
use std::collections::{BTreeMap, BTreeSet};

mod common;
use common::{new_store_params, tree, Store};

fn fixture(n: u32) -> (Cid, MemBlocks) {
    let records: BTreeMap<Vec<u8>, Vec<u8>> = (0..n).map(|i| (format!("k/{i:05}").into_bytes(), vec![(i % 251) as u8; 40])).collect();
    tree(&records)
}

/// An engine on `root`, through the fixture, its store empty.
fn started(root: Cid) -> Engine<Store> {
    let mut e = new_store_params(Params::default());
    let _ = e.step(Event::Start { key: engine::KeySource::SecretStore, epochs: vec![engine::Epoch(1)] });
    let _ = e.step(Event::HeadRead { epoch: engine::Epoch(1), seq: 1, root });
    e
}

fn fetches(fx: &[Effect]) -> Vec<Cid> {
    fx.iter().filter_map(|f| match f { Effect::FetchBlock { id, .. } => Some(*id), _ => None }).collect()
}

fn no_page_facts() -> BTreeSet<Cid> {
    BTreeSet::new()
}

/// **P: a parked READ's blocks are pinned** -- the one it waits on, and every one it has been handed (`held`): its
/// next attempt re-descends through them. Mutant "P unpinned" -> red.
#[test]
fn a_parked_reads_waited_and_held_blocks_are_pinned() {
    let (root, all) = fixture(400);
    let mut e = started(root);
    let out = e.step(Event::Get { client: ClientId(1), req_id: engine::read::ReqId(1), key: b"k/00100".to_vec() });
    let first = fetches(&out);
    assert_eq!(first, vec![root], "THE SETUP: the cold read did not ask for the root first");
    let k = no_page_facts();
    let pins = |e: &Engine<Store>| e.pins(&PagePins { held_asks: &k });
    assert_eq!(pins(&e).get(&root), Some(&Pin::ParkedRead), "the block a parked read WAITS on is not pinned");
    // The root lands: the read holds it and parks on the next level.
    let bytes = all.get(&root).expect("the root");
    e.blocks().put(root, bytes);
    let out = e.step(Event::BlockArrived { id: root, bytes: bytes.to_vec() });
    let next = fetches(&out);
    assert!(!next.is_empty() && !next.contains(&root), "THE SETUP: the read did not park on a deeper level");
    let now = pins(&e);
    assert_eq!(now.get(&root), Some(&Pin::ParkedRead), "a block a parked read HOLDS (its path) is not pinned");
    // The next level: the child the read itself waits on is pinned (race get may ask its group's other members
    // too -- those land in the repair's own memory, not the store, and are not the read's).
    assert!(next.iter().any(|id| now.get(id) == Some(&Pin::ParkedRead)), "the next-level block a parked read waits on is not pinned");
}

/// **PW: a parked WRITE's blocks are pinned** -- those it waits on (`needs`) and those that landed for it (`held`):
/// its re-apply walks them. Mutant "PW unpinned" -> red. And gone with it: once it applies, nothing of it is pinned.
#[test]
fn a_parked_writes_waited_and_held_blocks_are_pinned_until_it_applies() {
    let (root, all) = fixture(400);
    let mut e = started(root);
    e.blocks().put(root, all.get(&root).expect("the root"));
    let k = no_page_facts();
    let pins = |e: &Engine<Store>| e.pins(&PagePins { held_asks: &k });
    let out = e.step(Event::forced_write(ClientId(1), WriteId(1), vec![(b"k/00100".to_vec(), Op::Put(b"cold".to_vec()))]));
    let mut queue = fetches(&out);
    assert!(!queue.is_empty(), "THE SETUP: the write onto a cold path asked for nothing");
    assert!(queue.iter().all(|id| pins(&e).get(id) == Some(&Pin::ParkedWrite)), "a block a parked write WAITS on is not pinned");
    let mut landed_while_parked = Vec::new();
    let mut rounds = 0;
    while let Some(id) = queue.pop() {
        rounds += 1;
        assert!(rounds < 1000, "the parked write never settled");
        let bytes = all.get(&id).expect("the fixture holds every block");
        e.blocks().put(id, bytes);
        let out = e.step(Event::BlockArrived { id, bytes: bytes.to_vec() });
        let more = fetches(&out);
        if !more.is_empty() {
            // Still parked: what landed is HELD, and pinned.
            landed_while_parked.push(id);
            assert_eq!(pins(&e).get(&id), Some(&Pin::ParkedWrite), "a block that landed for a still-parked write is not pinned");
        }
        queue.extend(more);
    }
    assert!(!landed_while_parked.is_empty(), "THE SETUP: the write never parked twice, so nothing was HELD");
    assert!(e.parked_write_is_live(), "the parked write's owner is not live after it settled");
    let after = pins(&e);
    assert!(landed_while_parked.iter().all(|id| after.get(id) != Some(&Pin::ParkedWrite)), "a parked write's pins outlived it (it applied)");
}
