//! THE ROOT IS A GROUP OF ONE (sdk#335): a fresh head's root block had no
//! second source, and a reader on another node waited on it alone (measured:
//! ~4 min silent then NotFound; ~60 s silent). Now the root is coded like any
//! group -- k = 1, `PARITY` parity blocks, distinct bytes at distinct ids --
//! and its parity ids ride in the head:
//!
//! * RACE PUT signs when ANY 1 of the root's `1 + PARITY` is acked (the root
//!   group recoverable), the root itself silent or not;
//! * RACE GET asks the root and its parity at once and finishes on the FIRST;
//! * a head that lists no root parity is read as before: the root alone;
//! * a root move withdraws the old root's parity with the old root.

use engine::read::{ReadResult, ReqId};
use engine::{ClientId, Effect, Engine, Epoch, Event, Op, Params, State, WriteId, PARITY};
use freenet_prolly::{block_id, kind, parity, Cid};
use std::collections::{BTreeMap, BTreeSet};

mod common;
use common::Store;

fn put(k: &str, v: &[u8]) -> (Vec<u8>, Op) {
    (k.as_bytes().to_vec(), Op::Put(v.to_vec()))
}

fn puts(fx: &[Effect]) -> BTreeMap<Cid, Vec<u8>> {
    fx.iter()
        .filter_map(|f| match f {
            Effect::PutBlock { id, bytes, .. } => Some((*id, bytes.clone())),
            _ => None,
        })
        .collect()
}

fn head(fx: &[Effect]) -> Option<u64> {
    fx.iter().find_map(|f| match f {
        Effect::UpdateHead { seq, .. } => Some(*seq),
        _ => None,
    })
}

fn states(fx: &[Effect], id: u64) -> Vec<State> {
    fx.iter()
        .filter_map(|f| match f {
            Effect::Notify { write_id, state, .. } if *write_id == WriteId(id) => Some(*state),
            _ => None,
        })
        .collect()
}

/// Ack every PUT the first send and every later step emit except `skip`, and land the head when it goes (the parity
/// that follows it, #378 P1-hybrid, is emitted as it lands). Every effect, in order.
fn ack_all_but(e: &mut Engine<Store>, fx: &[Effect], skip: &BTreeSet<Cid>) -> Vec<Effect> {
    let mut all = fx.to_vec();
    let mut queue: Vec<Cid> = puts(fx).into_keys().collect();
    // A changed group's other members the node is asked about (sdk#416): all held here.
    queue.extend(fx.iter().filter_map(|f| if let Effect::ConfirmHeld { id } = f { Some(*id) } else { None }));
    let mut done: BTreeSet<Cid> = BTreeSet::new();
    let mut landed = false;
    loop {
        while let Some(id) = queue.pop() {
            if skip.contains(&id) || !done.insert(id) {
                continue;
            }
            let more = e.step(Event::PutConfirmed(id));
            queue.extend(puts(&more).into_keys());
            queue.extend(more.iter().filter_map(|f| if let Effect::ConfirmHeld { id } = f { Some(*id) } else { None }));
            all.extend(more);
        }
        match head(&all) {
            Some(seq) if !landed => {
                landed = true;
                let more = e.step(Event::HeadConfirmed(seq));
                queue.extend(puts(&more).into_keys());
                // A changed group's other members the node is asked about (class 2): all held here.
                queue.extend(more.iter().filter_map(|f| if let Effect::ConfirmHeld { id } = f { Some(*id) } else { None }));
                all.extend(more);
            }
            _ => return all,
        }
    }
}

/// The root's parity blocks as the tree's own rule codes a group of one node.
fn coded(root_bytes: &[u8]) -> Vec<Cid> {
    let mut st = vec![kind::TREE_NODE];
    st.extend_from_slice(root_bytes);
    parity::encode_group(&[st]).expect("k = 1 codes").iter().map(|p| block_id(kind::PARITY, p)).collect()
}

/// A writer's first commit of `n` small rows; nothing acked yet.
fn first_commit(n: u32) -> (Engine<Store>, Vec<Effect>) {
    let mut e = common::new_store_params(Params::default());
    let ops: Vec<(Vec<u8>, Op)> = (0..n).map(|i| put(&format!("k/{i:06}"), &[(i % 251) as u8; 20])).collect();
    let fx = e.step(Event::forced_write(ClientId(1), WriteId(1), ops));
    e.blocks().absorb(&fx);
    (e, fx)
}

#[test]
fn the_root_is_coded_as_a_group_of_one_and_one_of_its_parity_is_in_the_first_send() {
    let (e, fx) = first_commit(600);
    let sent = puts(&fx);
    let root = e.root();
    let ids = e.root_parity_of(&root);
    assert_eq!(ids.len(), PARITY, "the root's parity is not PARITY blocks");
    assert_eq!(ids, coded(&sent[&root]), "the root's parity is not the tree's k = 1 code of the root");
    assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), PARITY, "the root's parity blocks are not distinct");
    // THE FIRST WAVE (#378 P1-hybrid): ONE of the root's parity in the first send; the rest follow the Sign.
    assert_eq!(ids.iter().filter(|p| sent.contains_key(*p)).count(), 1, "not exactly one root parity in the first send");
    assert!(!ids.contains(&root));
}

/// §P at k = 1: with the ROOT silent, any ONE of its parity acked signs the
/// head; with none of the root's 1 + PARITY acked, nothing does.
#[test]
fn the_head_signs_on_one_root_parity_ack_with_the_root_silent() {
    let (mut e, fx) = first_commit(600);
    let root = e.root();
    let rp: BTreeSet<Cid> = e.root_parity_of(&root).into_iter().collect();
    let mut all = fx.clone();
    for id in puts(&fx).keys().filter(|id| **id != root && !rp.contains(*id)) {
        all.extend(e.step(Event::PutConfirmed(*id)));
    }
    assert!(head(&all).is_none(), "the head signed with none of the root's 1 + PARITY acked");
    let one = *rp.iter().next().unwrap();
    let more = e.step(Event::PutConfirmed(one));
    assert!(head(&more).is_some(), "one root parity acked (root silent) did not sign the head");
}

/// BACKED_UP needs all 1 + PARITY of the root group, like every group.
#[test]
fn backed_up_waits_for_every_block_of_the_root_group() {
    let (mut e, fx) = first_commit(600);
    let root = e.root();
    let last = *e.root_parity_of(&root).last().unwrap();
    let all = ack_all_but(&mut e, &fx, &BTreeSet::from([last]));
    assert!(states(&all, 1).contains(&State::Published));
    assert!(!states(&all, 1).contains(&State::ParityComplete), "BACKED_UP with a root parity block out");
    let more = e.step(Event::PutConfirmed(last));
    assert!(states(&more, 1).contains(&State::ParityComplete), "the last root parity landed and the write was not BACKED_UP");
}

/// A root move withdraws the old root's parity, still out, with the old root.
#[test]
fn a_root_move_withdraws_the_old_roots_parity() {
    let (mut e, fx) = first_commit(600);
    let r1 = e.root();
    let rp1 = e.root_parity_of(&r1);
    let straggler = rp1[0];
    ack_all_but(&mut e, &fx, &BTreeSet::from([straggler]));
    let fx2 = e.step(Event::forced_write(ClientId(1), WriteId(2), vec![put("k/000100", b"two")]));
    e.blocks().absorb(&fx2);
    let all2 = ack_all_but(&mut e, &fx2, &BTreeSet::new());
    assert_ne!(e.root(), r1);
    let withdrawn: BTreeSet<Cid> = all2.iter().filter_map(|f| match f { Effect::Withdraw { id } => Some(*id), _ => None }).collect();
    assert!(withdrawn.contains(&straggler), "the superseded root's parity was not withdrawn");
    assert!(states(&all2, 1).contains(&State::ParityComplete), "write 1 was not BACKED_UP with the newer root");
}

/// A cold reader of a head. `mark` is what the head's ledger lists for the
/// root; the ROOT itself is silent on the network. Returns the answer to one
/// read, and how often each block was asked.
fn read_with_silent_root(mark: Option<Vec<Cid>>) -> (Option<ReadResult>, BTreeMap<Cid, usize>, Vec<u8>) {
    let (mut w, fx) = first_commit(600);
    let net = puts(&fx);
    let root = w.root();
    let mut all = fx.clone();
    for id in net.keys() {
        all.extend(w.step(Event::PutConfirmed(*id)));
    }
    all.extend(w.step(Event::HeadConfirmed(head(&all).expect("signed"))));
    let mark = mark.map(|m| if m.is_empty() { m } else { w.root_parity_of(&root) });

    let mut e = common::fresh_reader(Params::default());
    e.set_head_mark(mark);
    let mut q = e.step(Event::HeadRead { epoch: Epoch(1), seq: 1, root });
    q.extend(e.step(Event::Get { client: ClientId(1), req_id: ReqId(7), key: b"k/000100".to_vec() }));
    let mut asked: BTreeMap<Cid, usize> = BTreeMap::new();
    let mut unwanted: std::collections::BTreeSet<Cid> = std::collections::BTreeSet::new();
    let mut answer = None;
    let mut steps = 0;
    while let Some(f) = q.pop() {
        steps += 1;
        assert!(steps < 50_000, "the read did not settle");
        match f {
            Effect::FetchBlock { id, .. } => {
                *asked.entry(id).or_insert(0) += 1;
                if id == root || (unwanted.contains(&id) && !e.readers_of(&id).any()) {
                    continue; // the root is SILENT
                }
                if let Some(b) = net.get(&id) {
                    e.blocks().put(id, b);
                    q.extend(e.step(Event::BlockArrived { id, bytes: b.clone() }));
                }
            }
            Effect::Keep { id, bytes } => e.blocks().put(id, &bytes),
            Effect::Unwanted { id } => {
                unwanted.insert(id);
            }
            Effect::Reply { req_id: ReqId(7), result, .. } => answer = Some(result),
            other => common::no_answer_owed(&other),
        }
    }
    // Row 100's value, as `first_commit` wrote it.
    (answer, asked, vec![100u8; 20])
}

#[test]
fn a_reader_reads_through_a_silent_root_from_its_parity() {
    let (answer, asked, want) = read_with_silent_root(Some(vec![[0; 32]]));
    assert_eq!(answer, Some(ReadResult::Value(Some(want))), "the read through a silent root was not answered from its parity");
    assert!(asked.len() > 1 + PARITY, "the read did not go on below the root");
}

/// THE CONTROL: a head listing no root parity (an empty mark) is read as
/// before, the root alone -- and a silent root holds the read.
#[test]
fn a_head_listing_no_root_parity_waits_on_the_root_alone() {
    let (answer, asked, _) = read_with_silent_root(Some(Vec::new()));
    assert!(answer.is_none(), "a read through a silent root was answered with no parity to rebuild it from");
    assert_eq!(asked.len(), 1, "a head with no root parity asked for more than its root: {asked:?}");
}
