//! A FINAL SIGN REFUSAL ENDS THE COMMIT (PUBLISH-LIFE ⁵, E8; COMMIT-LIFE's Heading x head refused, Committing x head
//! refused). The signer's final refusal is a real answer (rule 8): the page steps `Event::HeadRefused { seq }` and the
//! engine ends that commit at once -- every write of its cut `Failed` -- never leaving it to hang until its own clock
//! calls it `Stalled`. A refusal naming any other seq is stale and changes nothing.
use engine::{ClientId, Effect, Event, Op, Params, State, WriteId};

mod common;
use common::{Harness, Store};

const T0: u64 = 1_790_000_000;

fn write(id: u64, ops: Vec<(Vec<u8>, Op)>) -> Event {
    Event::forced_write(ClientId(1), WriteId(id), ops)
}

fn told(fx: &[Effect], id: u64) -> Vec<State> {
    fx.iter()
        .filter_map(|f| match f {
            Effect::Notify { write_id, state, .. } if write_id.0 == id => Some(*state),
            _ => None,
        })
        .collect()
}

fn small(tag: u8) -> Vec<(Vec<u8>, Op)> {
    (0..8u8).map(|i| (format!("{tag}/{i}").into_bytes(), Op::Put(vec![i; 700]))).collect()
}

/// A commit for write `id` with every block confirmed and its head SENT (the engine is Heading): the head's seq.
fn heading(h: &mut Harness, id: u64) -> u64 {
    let mut fx = h.step(write(id, small(id as u8)));
    for _ in 0..50 {
        let mut next = Vec::new();
        for f in &fx {
            match f {
                Effect::PutBlock { id, .. } | Effect::PutPack { id, .. } => {
                    h.store.remember(id);
                    next.extend(h.step(Event::PutConfirmed(*id)));
                }
                Effect::UpdateHead { seq, .. } => return *seq,
                other => common::no_answer_owed(other),
            }
        }
        fx = next;
    }
    panic!("THE SETUP: the commit never sent its head");
}

fn harness() -> Harness {
    let mut h = Harness::new(Params::default(), Store::fresh());
    let _ = h.step(Event::Tick(T0));
    h
}

/// **Heading x head refused (final), this seq:** every write of the cut is `Failed` IN THAT STEP -- no tick run, so
/// nothing reaches a stall bound -- and the engine is released: the next write commits. Mutant "the refusal ends
/// nothing" -> red (the write would go `Stalled` on the clock instead).
#[test]
fn a_final_head_refusal_ends_the_commit_failed_at_once() {
    let mut h = harness();
    let seq = heading(&mut h, 1);
    let fx = h.step(Event::HeadRefused { seq });
    assert_eq!(told(&fx, 1), vec![State::Failed { why: engine::FailWhy::SignerRefused }], "the refused commit's write was not Failed at once, as the signer's refusal: {:?}", told(&fx, 1));
    let next = h.step(write(2, small(2)));
    assert!(next.iter().any(|f| matches!(f, Effect::PutBlock { .. } | Effect::PutPack { .. })), "the engine was not released: the next write did not commit");
}

/// **A stale seq changes nothing:** a refusal naming a seq that is not the commit in flight's (its head already dead)
/// emits nothing, and the commit in flight still publishes on its own confirmation.
#[test]
fn a_head_refusal_for_another_seq_is_stale_and_changes_nothing() {
    let mut h = harness();
    let seq = heading(&mut h, 1);
    let fx = h.step(Event::HeadRefused { seq: seq + 1 });
    assert!(fx.is_empty(), "a stale refusal did something: {fx:?}");
    let done = h.step(Event::HeadConfirmed(seq));
    assert!(told(&done, 1).contains(&State::Published), "the commit in flight did not publish after a stale refusal: {:?}", told(&done, 1));
}

/// **The dead commit's ops are withdrawn** (the architect on #502). A refusal comes AFTER the head was sent: the
/// commit's first wave is out and its held-back parity was never sent. Every one of its blocks not yet confirmed is
/// `Withdraw`n in the refusal's step, and no PutBlock or ConfirmHeld for any of them goes out afterwards, however long
/// the page ticks. Mutant "fail_commit withdraws nothing" -> red.
#[test]
fn a_refused_commits_unconfirmed_blocks_are_withdrawn_and_never_put_again() {
    let mut h = harness();
    let (seq, confirmed) = heading_confirmed(&mut h, 1);
    let fx = h.step(Event::HeadRefused { seq });
    let withdrawn: Vec<freenet_prolly::Cid> = fx.iter().filter_map(|f| if let Effect::Withdraw { id } = f { Some(*id) } else { None }).collect();
    assert!(!withdrawn.is_empty(), "the refused commit withdrew nothing: its held-back parity and unconfirmed PUTs go on");
    assert!(withdrawn.iter().all(|id| !confirmed.contains(id)), "a block the node had already confirmed was withdrawn");
    let mut later = Vec::new();
    for t in 1..=2_000u64 {
        later.extend(h.step(Event::Tick(T0 + t)));
    }
    let again: Vec<&Effect> = later.iter().filter(|f| matches!(f, Effect::PutBlock { id, .. } | Effect::ConfirmHeld { id } if withdrawn.contains(id))).collect();
    assert!(again.is_empty(), "a withdrawn block of the refused commit was put or asked about again: {again:?}");
}

/// [`heading`], and the ids the node confirmed on the way.
fn heading_confirmed(h: &mut Harness, id: u64) -> (u64, Vec<freenet_prolly::Cid>) {
    let mut fx = h.step(write(id, small(id as u8)));
    let mut confirmed = Vec::new();
    for _ in 0..50 {
        let mut next = Vec::new();
        for f in &fx {
            match f {
                Effect::PutBlock { id, .. } | Effect::PutPack { id, .. } => {
                    h.store.remember(id);
                    confirmed.push(*id);
                    next.extend(h.step(Event::PutConfirmed(*id)));
                }
                Effect::UpdateHead { seq, .. } => return (*seq, confirmed),
                other => common::no_answer_owed(other),
            }
        }
        fx = next;
    }
    panic!("THE SETUP: the commit never sent its head");
}

/// **The dead commit's ConfirmHeld asks are withdrawn too** (the architect and main on #502). A write changes ONE leaf
/// of a published 6000-row tree: its commit asks the node about the group's other members (`ConfirmHeld`, sdk#416),
/// and its head goes out once k of the group are counted -- with some other members still unconfirmed and the
/// held-back parity never sent. The head is refused: every still-unconfirmed member and parity block is `Withdraw`n
/// in that step (the page ends its PUT/Held waits), and nothing of that commit goes out afterwards. Mutant
/// "fail_commit withdraws nothing" -> red.
#[test]
fn a_refused_commits_unconfirmed_members_and_parity_are_withdrawn() {
    use std::collections::BTreeMap;
    let records: BTreeMap<Vec<u8>, Vec<u8>> = (0..6000u32).map(|i| (format!("k/{i:06}").into_bytes(), format!("value {i}").into_bytes())).collect();
    let (root, all) = common::tree(&records);
    let (mut e, store) = common::cold_reader(root, Params::default());
    for (id, bytes) in all.0.iter() {
        store.put(*id, bytes);
    }
    let _ = e.step(Event::Tick(T0));
    let mut fx = e.step(Event::forced_write(ClientId(1), WriteId(1), vec![(b"k/003000".to_vec(), Op::Put(b"changed".to_vec()))]));
    let (mut asked, mut seq) = (Vec::new(), None);
    for _ in 0..200 {
        if let Some(s) = fx.iter().find_map(|f| if let Effect::UpdateHead { seq, .. } = f { Some(*seq) } else { None }) {
            seq = Some(s);
            break;
        }
        asked.extend(fx.iter().filter_map(|f| if let Effect::ConfirmHeld { id } = f { Some(*id) } else { None }));
        let puts: Vec<freenet_prolly::Cid> = fx.iter().filter_map(|f| if let Effect::PutBlock { id, .. } = f { Some(*id) } else { None }).collect();
        let mut next = Vec::new();
        if !puts.is_empty() {
            for id in puts {
                next.extend(e.step(Event::PutConfirmed(id)));
            }
        } else if let Some(m) = asked.pop() {
            // The node confirms the group's other members ONE AT A TIME, until the head goes: some stay unconfirmed.
            next.extend(e.step(Event::PutConfirmed(m)));
        }
        fx = next;
    }
    let seq = seq.expect("THE SETUP: the commit never sent its head");
    assert!(!asked.is_empty(), "THE SETUP: every other member was confirmed before the head: no ConfirmHeld ask is still out");
    let out = e.step(Event::HeadRefused { seq });
    let withdrawn: Vec<freenet_prolly::Cid> = out.iter().filter_map(|f| if let Effect::Withdraw { id } = f { Some(*id) } else { None }).collect();
    let missed: Vec<&freenet_prolly::Cid> = asked.iter().filter(|m| !withdrawn.contains(m)).collect();
    assert!(missed.is_empty(), "{} ConfirmHeld ask(s) of the refused commit were left out: {missed:?}", missed.len());
    assert!(out.iter().any(|f| matches!(f, Effect::Notify { state: State::Failed { why: engine::FailWhy::SignerRefused }, .. })), "THE SETUP: the refusal did not end the commit");
    let mut later = Vec::new();
    for t in 1..=2_000u64 {
        later.extend(e.step(Event::Tick(T0 + t)));
    }
    assert!(!later.iter().any(|f| matches!(f, Effect::PutBlock { id, .. } | Effect::ConfirmHeld { id } if withdrawn.contains(id))), "a withdrawn block of the refused commit went out again");
}
