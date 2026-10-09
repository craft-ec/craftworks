//! Owed parity over a LARGE tree, and a zero that can be believed
//! (craftworks-sdk#181, and #119's first half).
//!
//! #181: `recompute_owed` restarted at the root on every call and gave up
//! after `max_parity_scan_blocks` nodes of a rightmost-first walk, so a group
//! listed deep on the LEFT of a big tree — an OLD record, since ids sort by
//! creation time — was never recovered, never put, and its writer never told
//! `ParityComplete`. Executed on 12,000 keys: the leftmost edit ended at
//! `Published`. The walk now RESUMES where it stopped.
//!
//! #119: `owed_groups() == 0` meant both "every group is put" and "this engine
//! never looked". `parity_scan()` says which.

use engine::{ClientId, Effect, Engine, Epoch, Event, KeySource, Op, ParityScan, Params, State, WriteId};
use freenet_prolly::store::MemBlocks;
use freenet_prolly::Cid;
use std::collections::BTreeMap;

mod common;
use common::Store;

/// What the network kept: every confirmed put, and the head.
#[derive(Default)]
struct Network {
    blocks: MemBlocks,
    head: Option<(u64, Cid)>,
}

fn boot(net: &Network, params: Params) -> Engine<Store> {
    let mut e = Engine::new(params, Store::default());
    let mut queue = stepped!(
        e,
        Event::Start {
            key: KeySource::SecretStore,
            epochs: vec![Epoch(1)],
        }
    );
    while let Some(f) = queue.pop() {
        if let Effect::ReadHead { .. } = f {
            let ev = match net.head {
                Some((seq, root)) => Event::HeadRead { epoch: Epoch(1), seq, root },
                None => Event::HeadMissing,
            };
            queue.extend(stepped!(e, ev));
        }
    }
    e
}

/// Answer every effect as a node that holds everything would, and record
/// every state each write is told.
fn drive(e: &mut Engine<Store>, net: &mut Network, first: Vec<Effect>, told: &mut BTreeMap<u64, Vec<State>>) {
    let mut queue = first;
    let mut guard = 0usize;
    while let Some(f) = queue.pop() {
        guard += 1;
        assert!(guard < 5_000_000, "the engine never settled");
        match f {
            Effect::PutPack { id, bytes, .. } | Effect::PutBlock { id, bytes, .. } => {
                for (mid, mbytes) in engine::pack::members(&bytes) {
                    net.blocks.insert(mid, &mbytes);
                }
                net.blocks.insert(id, &bytes);
                queue.extend(stepped!(e, Event::PutConfirmed(id)));
            }
            // A changed group's other member: held, if the network holds it.
            Effect::ConfirmHeld { id } => {
                if freenet_prolly::store::Blocks::get(&net.blocks, &id).is_some() {
                    queue.extend(stepped!(e, Event::PutConfirmed(id)));
                }
            }
            Effect::UpdateHead { seq, root, .. } => {
                net.head = Some((seq, root));
                queue.extend(stepped!(e, Event::HeadConfirmed(seq)));
            }
            Effect::FetchBlock { id, .. } => {
                if let Some(bytes) = freenet_prolly::store::Blocks::get(&net.blocks, &id) {
                    let bytes = bytes.to_vec();
                    queue.extend(stepped!(e, Event::BlockArrived { id, bytes }));
                }
            }
            Effect::Notify { write_id, state, .. } => told.entry(write_id.0).or_default().push(state),
            other => common::no_answer_owed(&other),
        }
    }
}

fn write(e: &mut Engine<Store>, net: &mut Network, id: u64, ops: Vec<(Vec<u8>, Op)>, told: &mut BTreeMap<u64, Vec<State>>) {
    let out = stepped!(e, Event::forced_write(ClientId(1), WriteId(id), ops));
    drive(e, net, out, told);
}

fn ticks(e: &mut Engine<Store>, net: &mut Network, from: u64, n: u64, told: &mut BTreeMap<u64, Vec<State>>) {
    for t in from..from + n {
        let out = stepped!(e, Event::Tick(t));
        drive(e, net, out, told);
    }
}

fn key(i: u32) -> Vec<u8> {
    format!("d/notes/{i:06}").into_bytes()
}

/// **#181: editing the OLDEST record of a big tree gets its redundancy.**
/// Under race put (COMMIT-LIFE §P) a commit's parity goes WITH it, so there is
/// no walk to reach a group far down the left of the tree: the edit's own
/// commit carries its group's parity, and its writer is told `ParityComplete`
/// once every block of the commit is acked. Control: the RIGHTMOST edit.
#[test]
fn editing_the_leftmost_key_of_a_big_tree_reaches_parity_complete() {
    let params = Params::default();
    let mut net = Network::default();
    let mut e = boot(&net, params);
    let mut told = BTreeMap::new();
    let n: u32 = 24_000;
    let mut id = 1u64;
    for chunk in (0..n).collect::<Vec<_>>().chunks(100) {
        let ops = chunk.iter().map(|i| (key(*i), Op::Put(vec![(*i % 251) as u8; 1_000]))).collect();
        write(&mut e, &mut net, id, ops, &mut told);
        id += 1;
    }
    ticks(&mut e, &mut net, 1, 20, &mut told);
    let blocks = net.blocks.0.len();
    let mut t = 1_000u64;
    for (label, k) in [("RIGHTMOST", n - 1), ("LEFTMOST", 0)] {
        let w = id;
        id += 1;
        let out = stepped!(e, Event::forced_write(ClientId(1), WriteId(w), vec![(key(k), Op::Put(b"edited".to_vec()))]));
        drive(&mut e, &mut net, out, &mut told);
        ticks(&mut e, &mut net, t, 5, &mut told);
        t += 1_000;
        let states = told.get(&w).cloned().unwrap_or_default();
        println!("  {blocks} blocks | edit {label} key: {states:?}");
        assert!(
            states.contains(&State::ParityComplete),
            "editing the {label} key of a {n}-key tree ({blocks} blocks) was told {states:?}: its commit's parity never completed"
        );
    }
    assert_eq!(e.backing(), 0, "a commit is still putting after every block was acked");
}

/// **#119: a zero that can be believed.** An engine that tracked the tree from
/// empty says `Done`; a second engine booting on that non-empty head knows
/// nothing about its parity and says `NotScanned` — while `owed_groups()` is 0
/// in BOTH, which is exactly why the count alone must never be reported.
#[test]
fn owed_zero_is_done_only_for_an_engine_that_tracked_the_tree() {
    let params = Params::default();
    let mut net = Network::default();
    let mut e = boot(&net, params);
    let mut told = BTreeMap::new();
    assert!(matches!(e.parity_scan(), ParityScan::Done { .. }), "an empty tree is known in full: {:?}", e.parity_scan());
    let ops = (0..300u32).map(|i| (key(i), Op::Put(vec![7u8; 1400]))).collect();
    write(&mut e, &mut net, 1, ops, &mut told);
    ticks(&mut e, &mut net, 1, 200, &mut told);
    let root = net.head.unwrap_or_else(|| panic!("no head was written; the write was told {told:?}")).1;
    assert_eq!(e.parity_scan(), &ParityScan::Done { root }, "its own commits keep it Done, at the new root");
    assert_eq!(e.owed_groups(), 0, "the control: this engine's zero is 'all put'");

    let fresh = boot(&net, params);
    assert_eq!(fresh.owed_groups(), 0);
    assert_eq!(
        fresh.parity_scan(),
        &ParityScan::NotScanned,
        "a fresh engine on a non-empty head reports 0 owed without having looked — it must say NotScanned"
    );
}
