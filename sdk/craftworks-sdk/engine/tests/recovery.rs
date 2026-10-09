//! The restart gate (craftworks-sdk#29).
//!
//! This is the phase's promise: the engine is dropped at every state boundary
//! of every commit and restarted against a network that kept EXACTLY what had
//! been confirmed, and nothing published is lost, nothing is applied twice,
//! and no head ever names a root whose blocks are not all there.
//!
//! It is also the reason slice 1 is a pure state machine. A crash at a
//! precise moment is an ordinary value being dropped here; against a live
//! node it is a race nobody can schedule.

use engine::{ClientId, Effect, Engine, Epoch, Event, Expect, KeySource, Op, Params, State, WriteId};
use freenet_prolly::build::TreeBuilder;
use freenet_prolly::store::{Blocks, MemBlocks};
use freenet_prolly::Cid;
use std::collections::{BTreeMap, BTreeSet};

mod common;
use common::Store;

/// What the network kept. Only what was CONFIRMED is here: a put that was
/// emitted and not confirmed is exactly what a crash loses.
#[derive(Default, Clone)]
struct Network {
    blocks: MemBlocks,
    head: Option<(u64, Cid)>,
}

impl Network {
    /// A block whose put was confirmed. Packs are unpacked, because that is
    /// what the far side holds once it has one.
    fn confirm(&mut self, id: Cid, bytes: &[u8]) {
        for (mid, mbytes) in engine::pack::members(bytes) {
            self.blocks.insert(mid, &mbytes);
        }
        self.blocks.insert(id, bytes);
    }
}

/// Where a commit can be cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Cut {
    AfterAccept,
    AfterSomePacks,
    AfterAllPacks,
    AfterHeadEmitted,
    AfterHeadConfirmed,
    /// SAVED, not BACKED_UP (race put, COMMIT-LIFE §P): the head is signed
    /// and confirmed while some of the commit's PARITY was never acked, and
    /// the engine is dropped then. Before §P this was "mid-parity", reached
    /// by ticking; parity now goes with the data, so it is reached by
    /// withholding parity acks.
    SavedNotBackedUp,
}

const CUTS: [Cut; 6] = [
    Cut::AfterAccept,
    Cut::AfterSomePacks,
    Cut::AfterAllPacks,
    Cut::AfterHeadEmitted,
    Cut::AfterHeadConfirmed,
    Cut::SavedNotBackedUp,
];

fn boot(net: &Network, params: Params) -> Engine<Store> {
    let mut e = Engine::new(params, Store::default());
    let out = stepped!(
        e,
        Event::Start {
            key: KeySource::SecretStore,
            epochs: vec![Epoch(2), Epoch(1)],
        }
    );
    // Answer whatever head reads it asks for.
    let mut queue = out;
    while let Some(f) = queue.pop() {
        if let Effect::ReadHead { epoch } = f {
            let ev = match (epoch, net.head) {
                // The head lives under the current epoch in this harness.
                (Epoch(2), Some((seq, root))) => Event::HeadRead {
                    epoch: Epoch(2),
                    seq,
                    root,
                },
                _ => Event::HeadMissing,
            };
            queue.extend(stepped!(e, ev));
        }
    }
    e
}

/// Give a recovered engine the blocks the network kept, as a read would.
fn warm_from(e: &mut Engine<Store>, net: &Network) {
    for (id, bytes) in net.blocks.0.iter() {
        let _ = stepped!(
            e,
            Event::BlockArrived {
                id: *id,
                bytes: bytes.clone(),
            }
        );
    }
}

fn rebuild(records: &BTreeMap<Vec<u8>, Vec<u8>>) -> Cid {
    let mut t = TreeBuilder::new(|_, _: &[u8]| {});
    for (k, v) in records {
        t.push_bytes(k, v).unwrap();
    }
    t.finish().unwrap()
}

/// THE GATE.
#[test]
fn the_engine_survives_being_dropped_at_every_commit_boundary() {
    let mut cut_at: BTreeMap<Cut, usize> = BTreeMap::new();
    let mut checked_unknown = 0usize;

    for (batch_n, cut) in CUTS.iter().enumerate() {
        // A scripted workload: three commits, cut in the third.
        let mut net = Network::default();
        let mut published: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
        let mut e = boot(&net, Params::default());

        // One clock for the whole workload. Restarting it per commit sends
        // time BACKWARDS, and the parity timer then never fires — which is
        // how the mid-parity boundary came to be unreachable.
        let mut clock = 0u64;
        let mut next_write = 0u64;
        let mut accepted_only: BTreeSet<WriteId> = BTreeSet::new();

        for commit in 0..3u32 {
            let cutting = commit == 2;
            let will_publish: Vec<(Vec<u8>, Vec<u8>)> = (0..40u32)
                .map(|i| {
                    let k = format!("k/{commit}/{i:03}").into_bytes();
                    let v = vec![(i % 251) as u8; if i.is_multiple_of(4) { 1400 } else { 30 }];
                    (k, v)
                })
                .collect();
            let ops: Vec<(Vec<u8>, Op)> = will_publish
                .iter()
                .map(|(k, v)| (k.clone(), Op::Put(v.clone())))
                .collect();

            next_write += 1;
            let wid = WriteId(next_write);
            let mut queue = stepped!(
                e,
                Event::forced_write(ClientId(1), wid, ops)
            );
            accepted_only.insert(wid);

            if cutting && *cut == Cut::AfterAccept {
                *cut_at.entry(*cut).or_default() += 1;
                break;
            }

            // Drive the commit, cutting where the case says.
            let mut packs_confirmed = 0usize;
            let mut withheld_parity = 0usize;
            let mut dropped = false;
            let mut guard = 0;
            while let Some(f) = queue.pop() {
                guard += 1;
                assert!(guard < 200_000, "commit {commit} did not settle");
                match f {
                    // SavedNotBackedUp: the commit's parity is lost with the
                    // engine -- never acked. The head still signs (every
                    // group's members are in: k of k+m).
                    Effect::PutBlock { id, ref bytes, .. }
                        if cutting && *cut == Cut::SavedNotBackedUp && freenet_prolly::block_id(freenet_prolly::kind::PARITY, bytes) == id =>
                    {
                        withheld_parity += 1;
                    }
                    Effect::PutPack { id, bytes, .. } | Effect::PutBlock { id, bytes, .. } => {
                        net.confirm(id, &bytes);
                        let out = stepped!(e, Event::PutConfirmed(id));
                        packs_confirmed += 1;
                        if cutting && *cut == Cut::AfterSomePacks && packs_confirmed == 1 {
                            dropped = true;
                            break;
                        }
                        // Every pack is in and the engine has asked for the
                        // head to move — but the shell never acted on it, so
                        // the network never heard. Cut BEFORE that effect is
                        // processed, or the cut lands after the head is
                        // already confirmed and tests a different moment than
                        // it claims to.
                        let asks_for_head =
                            out.iter().any(|f| matches!(f, Effect::UpdateHead { .. }));
                        queue.extend(out);
                        if cutting && *cut == Cut::AfterAllPacks && asks_for_head {
                            dropped = true;
                            break;
                        }
                    }
                    Effect::UpdateHead { seq, root, .. } => {
                        if cutting && *cut == Cut::AfterHeadEmitted {
                            // Emitted and NOT confirmed: the network never
                            // learned of it, so recovery must not see it.
                            dropped = true;
                            break;
                        }
                        net.head = Some((seq, root));
                        queue.extend(stepped!(e, Event::HeadConfirmed(seq)));
                        if cutting && *cut == Cut::AfterHeadConfirmed {
                            published.extend(will_publish.iter().cloned());
                            accepted_only.remove(&wid);
                            dropped = true;
                            break;
                        }
                        if cutting && *cut == Cut::SavedNotBackedUp {
                            assert!(withheld_parity > 0, "{cut:?}: the commit put no parity, so there was nothing to lose with the engine");
                            published.extend(will_publish.iter().cloned());
                            accepted_only.remove(&wid);
                            dropped = true;
                            break;
                        }
                    }
                    Effect::Notify {
                        write_id,
                        state: State::Published,
                        ..
                    } => {
                        published.extend(will_publish.iter().cloned());
                        accepted_only.remove(&write_id);
                    }
                    _ => {}
                }
            }
            // Time passes between commits (parity went out WITH each commit,
            // §P, so there is nothing for a tick to put).
            if !dropped {
                for _ in 0..4 {
                    clock += 1;
                    let _ = stepped!(e, Event::Tick(clock));
                }
            }
            if dropped {
                *cut_at.entry(*cut).or_default() += 1;
                break;
            }
        }

        // ---- the drop ----
        drop(e);
        let mut e = boot(&net, Params::default());
        warm_from(&mut e, &net);

        // (b) the recovered root is the root of exactly the published writes.
        assert_eq!(
            e.published_root(),
            rebuild(&published),
            "{cut:?}: the recovered tree is not the tree of exactly what was published"
        );

        // (c) the head names nothing missing.
        if let Some((_, root)) = net.head {
            let mut stack = vec![root];
            let mut seen = BTreeSet::new();
            while let Some(cid) = stack.pop() {
                if !seen.insert(cid) {
                    continue;
                }
                let bytes = net
                    .blocks
                    .get(&cid)
                    .unwrap_or_else(|| panic!("{cut:?}: the head names block {cid:?}, which the network does not have"))
                    .to_vec();
                if let Ok(n) = freenet_prolly::node::Node::parse(&bytes) {
                    if !n.is_leaf() {
                        for i in 0..n.len() {
                            stack.push(n.child(i).0);
                        }
                    }
                }
            }
        }

        // (a) every published write is readable in the recovered tree.
        for (k, v) in &published {
            assert_eq!(
                freenet_prolly::read::get(&net.blocks, &e.published_root(), k)
                    .expect("the recovered tree reads")
                    .map(|val| match val {
                        freenet_prolly::node::Value::Inline(b) => b.to_vec(),
                        freenet_prolly::node::Value::Ref { cid, .. } =>
                            net.blocks.get(&cid).expect("the value block").to_vec(),
                    })
                    .as_ref(),
                Some(v),
                "{cut:?}: a published write is not in the recovered tree"
            );
        }

        // (d) an accepted-only write the restarted engine has no record of is
        // answered with NO verdict (sdk#196 review): not knowing is not
        // evidence -- the same answer is given for a write that published and
        // whose Published was lost, where `Lost` would roll back a write that
        // is in the tree. The client's own timeout decides.
        for wid in &accepted_only {
            let out = stepped!(
                e,
                Event::AskWrite {
                    client: ClientId(1),
                    write_id: *wid,
                }
            );
            assert!(
                !out.iter().any(|f| matches!(f, Effect::Notify { .. })),
                "{cut:?}: write {wid:?} is unknown to the restarted engine, and \
                 it was given a verdict anyway: {out:?}"
            );
            checked_unknown += 1;
        }
        let _ = batch_n;
    }

    // The gate counts what it cut, and fails below a floor: a run that cut
    // nowhere would otherwise be the greenest of all.
    assert_eq!(
        cut_at.len(),
        CUTS.len(),
        "only {} of {} boundaries were actually cut: {cut_at:?}",
        cut_at.len(),
        CUTS.len()
    );
    assert!(
        checked_unknown > 0,
        "no accepted-only write was ever asked after"
    );
    println!(
        "  cut at all {} boundaries; {checked_unknown} unknown writes given no verdict",
        CUTS.len()
    );
}

/// The control for (c): a head written before its packs leaves a root that
/// names blocks nobody has.
///
/// This is why pack read-back strictly precedes the head update, and why the
/// two cannot be parallelised for latency. Without the control, "(c) holds"
/// would be a claim about a window the test never opened.
#[test]
fn a_head_written_before_its_packs_names_blocks_nobody_has() {
    let mut net = Network::default();
    let mut e = boot(
        &net,
        Params {
            head_before_packs: true,
            // Values get their own PUT, so the commit is many blocks rather
            // than one pack. With a single pack, confirming one put confirms
            // everything and the window this control exists to open is never
            // open — which is what the assertion below caught.
            max_packed_value: 512,
            pack_on_write: true,
            ..Params::default()
        },
    );
    let ops: Vec<(Vec<u8>, Op)> = (0..40u32)
        .map(|i| {
            (
                format!("k/{i:03}").into_bytes(),
                Op::Put(vec![(i % 251) as u8; 1400]),
            )
        })
        .collect();
    let mut queue = stepped!(
        e,
        Event::forced_write(ClientId(1), WriteId(1), ops)
    );

    // Confirm ONE put, then take whatever head the engine offers.
    let mut confirmed_one = false;
    let mut head = None;
    let mut guard = 0;
    while let Some(f) = queue.pop() {
        guard += 1;
        assert!(guard < 100_000, "the commit did not settle");
        match f {
            Effect::PutPack { id, bytes, .. } | Effect::PutBlock { id, bytes, .. } => {
                if !confirmed_one {
                    net.confirm(id, &bytes);
                    confirmed_one = true;
                    queue.extend(stepped!(e, Event::PutConfirmed(id)));
                }
            }
            Effect::UpdateHead { seq, root, .. } => {
                head = Some((seq, root));
                break;
            }
            other => common::no_answer_owed(&other),
        }
    }
    let (_, root) = head.expect("the control must offer a head early, or it is no control");

    // The head names a root the network cannot walk.
    let mut stack = vec![root];
    let mut missing = 0;
    let mut seen = BTreeSet::new();
    while let Some(cid) = stack.pop() {
        if !seen.insert(cid) {
            continue;
        }
        match net.blocks.get(&cid) {
            None => missing += 1,
            Some(bytes) => {
                if let Ok(n) = freenet_prolly::node::Node::parse(bytes) {
                    if !n.is_leaf() {
                        for i in 0..n.len() {
                            stack.push(n.child(i).0);
                        }
                    }
                }
            }
        }
    }
    assert!(
        missing > 0,
        "the control's head named nothing missing, so (c) in the gate above is \
         asserted over a window this suite never opens"
    );
    println!("  control: a head written early names {missing} block(s) nobody has");
}

/// (e): a write never sits merely `accepted` in silence — and a stalled
/// write still gets saved, while a refused one leaves no trace.
///
/// `accepted` survives a refresh and not a restart, so it is exposure. A
/// commit that cannot publish leaves its own write sitting accepted, and at
/// T that write is told `Stalled`: still held, not saved, not moving.
///
/// `Stalled` is NOT `Failed`, and the difference is the whole point. The
/// edit is still in the tree and ships when the commit confirms, so reporting
/// `Failed` would be a false statement with consequences — the client
/// re-submits, the original publishes anyway, and a write someone else made
/// in between is overwritten by the re-submission.
///
/// Writes arriving BEHIND it are the other half (R-b): QUEUED, `Accepted` in
/// the step that submitted them, and -- behind a commit that is not moving --
/// told `Stalled` once each too: taken, not saved, not moving. When the
/// network answers, each publishes in its own commit, in order.
#[test]
fn a_stalled_write_is_reported_once_and_the_writes_queued_behind_it_publish() {
    let t = 8u64;
    let mut net = Network::default();
    let mut e = boot(
        &net,
        Params {
            max_accept_age: t,
            ..Params::default()
        },
    );

    // Write 1 opens a commit. Its puts are held back, so it cannot publish.
    let first = stepped!(
        e,
        Event::forced_write(ClientId(1), WriteId(1), vec![(b"a".to_vec(), Op::Put(vec![1u8; 40]))])
    );
    let held: Vec<(Cid, Vec<u8>)> = first
        .iter()
        .filter_map(|f| match f {
            Effect::PutPack { id, bytes, .. } | Effect::PutBlock { id, bytes, .. } => {
                Some((*id, bytes.clone()))
            }
            _ => None,
        })
        .collect();
    assert!(
        !held.is_empty(),
        "the first commit shipped nothing to hold back"
    );

    // Writes 2..6 arrive behind it.
    let mut seen: BTreeMap<WriteId, Vec<State>> = BTreeMap::new();
    let absorb = |seen: &mut BTreeMap<WriteId, Vec<State>>, fx: &[Effect]| {
        for f in fx {
            if let Effect::Notify {
                write_id, state, ..
            } = f
            {
                seen.entry(*write_id).or_default().push(*state);
            }
        }
    };
    absorb(&mut seen, &first);
    for n in 2..=6u64 {
        let out = stepped!(
            e,
            Event::forced_write(ClientId(1), WriteId(n), vec![(format!("k{n}").into_bytes(), Op::Put(vec![2u8; 40]))])
        );
        absorb(&mut seen, &out);
    }
    // Each of them was answered in the step that submitted it. Not eventually,
    // and not by a tick: a caller that got no reply has nothing to wait on.
    for n in 2..=6u64 {
        assert_eq!(
            seen.get(&WriteId(n)).map(Vec::as_slice),
            Some([State::Accepted].as_slice()),
            "write {n} arrived behind an open commit and was not taken in \
             the same step"
        );
    }

    // Time passes with the commit stuck. Its effects are KEPT: a tick may
    // settle the commit from fact (sdk#150) -- its blocks are held, so they
    // are confirmed and the head is sent in a tick's effects, which the
    // network answers below along with the rest.
    let mut later: Vec<Effect> = Vec::new();
    for tick in 1..=(t * 3) {
        let out = stepped!(e, Event::Tick(tick));
        absorb(&mut seen, &out);
        later.extend(out);
    }

    // Write 1 is the one that is genuinely held: it was accepted, its edit is
    // in the tree, and it cannot publish. That is what `Stalled` describes.
    let one = seen.get(&WriteId(1)).cloned().unwrap_or_default();
    assert!(
        one.contains(&State::Stalled),
        "write 1 sat accepted for {}+ ticks and was never reported Stalled: \
         {one:?}",
        t * 3
    );
    assert_eq!(
        one.iter().filter(|s| **s == State::Stalled).count(),
        1,
        "write 1 was told Stalled more than once; a notice repeated every \
         tick is one a caller learns to ignore"
    );
    assert!(
        !one.iter().any(|s| matches!(s, State::Failed { .. })),
        "write 1 was reported Failed while its edit is still in the tree"
    );
    // Queued behind the stuck commit: taken, and not moving either -- told
    // `Stalled` once each, never `Failed`.
    for n in 2..=6u64 {
        assert_eq!(
            seen.get(&WriteId(n)).map(Vec::as_slice),
            Some([State::Accepted, State::Stalled].as_slice()),
            "write {n}, queued behind a stuck commit, was not told Stalled once"
        );
    }

    // Now the network answers, and the stalled write gets saved.
    let mut queue = first;
    queue.extend(later);
    for (id, bytes) in &held {
        net.confirm(*id, bytes);
    }
    let mut guard = 0;
    while let Some(f) = queue.pop() {
        guard += 1;
        assert!(guard < 100_000, "the unblocked commit did not settle");
        match f {
            Effect::PutPack { id, bytes, .. } | Effect::PutBlock { id, bytes, .. } => {
                net.confirm(id, &bytes);
                let out = stepped!(e, Event::PutConfirmed(id));
                absorb(&mut seen, &out);
                queue.extend(out);
            }
            Effect::UpdateHead { seq, root, .. } => {
                net.head = Some((seq, root));
                let out = stepped!(e, Event::HeadConfirmed(seq));
                absorb(&mut seen, &out);
                queue.extend(out);
            }
            other => common::no_answer_owed(&other),
        }
    }

    let one = seen.get(&WriteId(1)).cloned().unwrap_or_default();
    assert!(
        one.contains(&State::Published),
        "write 1 was told Stalled and never reached Published: {one:?}. A \
         stalled write is still held and still gets saved — that is what \
         makes the notice honest"
    );
    assert!(
        valid_sequence(&one),
        "write 1 reported an impossible sequence: {one:?}"
    );

    // Every queued write published too, each in its own commit, in order:
    // the published tree is all six edits.
    let mut all_six: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
    all_six.insert(b"a".to_vec(), vec![1u8; 40]);
    for n in 2..=6u64 {
        let told = seen.get(&WriteId(n)).cloned().unwrap_or_default();
        assert!(told.contains(&State::Published), "queued write {n} never published: {told:?}");
        assert!(valid_sequence(&told), "write {n} reported an impossible sequence: {told:?}");
        all_six.insert(format!("k{n}").into_bytes(), vec![2u8; 40]);
    }
    let (seq, published) = net.head.expect("the commit never published a head");
    assert_eq!(published, rebuild(&all_six), "the published tree is not all six writes");
    // GROUP COMMIT (K9): write 1's commit, then the five queued behind it
    // cut together once it publishes.
    assert_eq!(seq, 2, "the writes queued behind the stuck commit did not go as one group");
    assert_eq!(e.commits_and_writes(), (2, 6), "writes per commit");
    assert_eq!(e.rebuild_differs(), 0, "K9: a rebuilt commit's root differed from its warm root");
    // ...and the comparison is sensitive: a tree missing one queued write's
    // key has a different root.
    let mut without_k6 = all_six.clone();
    without_k6.remove(b"k6".as_slice());
    assert_ne!(rebuild(&without_k6), published, "the root comparison cannot tell a missing write apart");

    // The control: with the bound off, nobody is told anything.
    let net2 = Network::default();
    let mut e2 = boot(
        &net2,
        Params {
            max_accept_age: t,
            bound_accept_age: false,
            ..Params::default()
        },
    );
    let mut stalled = 0;
    let _ = stepped!(
        e2,
        Event::forced_write(ClientId(1), WriteId(1), vec![(b"a".to_vec(), Op::Put(vec![1u8; 40]))])
    );
    for tick in 1..=(t * 3) {
        for f in stepped!(e2, Event::Tick(tick)) {
            if let Effect::Notify {
                state: State::Stalled,
                ..
            } = f
            {
                stalled += 1;
            }
        }
    }
    assert_eq!(
        stalled, 0,
        "the control reported {stalled} Stalled notice(s), so the bound is not \
         what produces them"
    );
    println!(
        "  write 1 stalled once and published; writes 2-6 refused once each \
         and left no key behind; control: 0 notices"
    );
}

/// Is this a sequence a write can legally report?
///
/// A prefix of `accepted (stalled)? published parity-complete`, or a terminal
/// (`failed` / `lost` / `busy`) with nothing after it. Written out rather than
/// derived from the enum's order, because the legal ORDER is a rule about the
/// protocol and the enum's order is an implementation detail that happens to
/// agree with it today.
fn valid_sequence(states: &[State]) -> bool {
    const HAPPY: [State; 4] = [
        State::Accepted,
        State::Stalled,
        State::Published,
        State::ParityComplete,
    ];
    let mut at = 0usize;
    for (i, s) in states.iter().enumerate() {
        if matches!(s, State::Failed { .. } | State::Lost | State::Busy) {
            // Terminal: nothing may follow it.
            return i + 1 == states.len();
        }
        // Must advance through the happy path, skipping the optional Stalled.
        match HAPPY[at..].iter().position(|h| h == s) {
            Some(k) => at += k + 1,
            None => return false,
        }
    }
    true
}

/// An engine's own head may sit under the PREVIOUS code epoch after an
/// upgrade, so recovery tries them newest first.
///
/// Rule 2. A new binary that looked only under its own epoch would decide its
/// device had never written and start an empty tree beside a real one — the
/// worst possible recovery, because it looks like a successful one.
#[test]
fn recovery_finds_a_head_left_under_the_previous_epoch() {
    let mut e = Engine::new(Params::default(), Store::default());
    let out = stepped!(
        e,
        Event::Start {
            key: KeySource::SecretStore,
            epochs: vec![Epoch(2), Epoch(1)],
        }
    );
    assert_eq!(
        out.iter()
            .filter_map(|f| match f {
                Effect::ReadHead { epoch } => Some(*epoch),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![Epoch(2)],
        "recovery must ask for the newest epoch first"
    );

    // Nothing under the current epoch.
    let out = stepped!(e, Event::HeadMissing);
    assert_eq!(
        out.iter()
            .filter_map(|f| match f {
                Effect::ReadHead { epoch } => Some(*epoch),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![Epoch(1)],
        "a missing head under the current epoch must fall back to the previous"
    );

    // The head is there, under the old epoch.
    let records: BTreeMap<Vec<u8>, Vec<u8>> = (0..50u32)
        .map(|i| (format!("k{i:03}").into_bytes(), vec![7u8; 30]))
        .collect();
    let root = rebuild(&records);
    let out = stepped!(
        e,
        Event::HeadRead {
            epoch: Epoch(1),
            seq: 41,
            root,
        }
    );
    assert!(
        out.is_empty(),
        "recovery walks nothing: reads warm it lazily"
    );
    assert_eq!(e.published_root(), root, "the recovered root is the head's");

    // And a device that has never written gets an empty tree, not a panic.
    let mut fresh = Engine::new(Params::default(), Store::default());
    let _ = stepped!(
        fresh,
        Event::Start {
            key: KeySource::Reissued,
            epochs: vec![Epoch(2)],
        }
    );
    let out = stepped!(fresh, Event::HeadMissing);
    assert!(out.is_empty(), "a brand-new device has nothing left to ask");
    assert_eq!(
        fresh.published_root(),
        rebuild(&BTreeMap::new()),
        "a device with no head starts from the empty tree"
    );
}

/// Two engines on one device key: the loser rebases, and never forks.
///
/// Rule 6. A restart racing the old process is the ordinary case, not an
/// exotic one — the Register refuses the lower or equal seq, and the engine
/// that loses must take the winner's head rather than publish a second
/// history under the same key.
///
/// A FOREIGN MOVE (R-b; COMMIT-LIFE § A write's stage): the dead commit's
/// write goes again at the FRONT, re-judged on the head that won, with one
/// try spent -- unless it is FORCED (`Expect::Any`), which falls `Lost`,
/// named, and is never re-applied (WRITE-PATH ⁷). Every write queued behind
/// it is re-applied in order onto the winner. Nothing is told twice.
#[test]
fn the_loser_of_a_head_conflict_rebases_and_never_forks() {
    for forced in [true, false] {
        let net = Network::default();
        let mut e = boot(&net, Params::default());
        let before = e.published_root();

        let mut answers: BTreeMap<WriteId, Vec<State>> = BTreeMap::new();
        for (n, key) in [(1u64, &b"mine"[..]), (2, &b"also-mine"[..])] {
            let ops = vec![(key.to_vec(), Op::Put(vec![n as u8; 40]))];
            // Write 1 (the commit) is the forced one, or a checked one that
            // read its key absent; write 2 is checked the same way.
            let ev = if forced && n == 1 {
                Event::forced_write(ClientId(1), WriteId(n), ops)
            } else {
                Event::Write { client: ClientId(1), write_id: WriteId(n), ops, reads: vec![(key.to_vec(), Expect::Absent)], deferred: false }
            };
            for f in stepped!(e, ev) {
                if let Effect::Notify { write_id, state, .. } = f {
                    answers.entry(write_id).or_default().push(state);
                }
            }
        }
        assert_ne!(e.root(), before, "the writes did not reach the warm tree");
        for n in 1..=2 {
            assert_eq!(answers[&WriteId(n)], vec![State::Accepted], "write {n} was not taken");
        }

        // The other engine got there first.
        let mut theirs: BTreeMap<Vec<u8>, Vec<u8>> = (0..30u32)
            .map(|i| (format!("theirs{i:02}").into_bytes(), vec![9u8; 30]))
            .collect();
        // The winner's tree is on the node (it published it), so re-applying
        // onto it reads blocks the node holds rather than parking.
        let (winner, their_blocks) = common::tree(&theirs);
        for (id, bytes) in their_blocks.0.iter() {
            e.blocks().put(*id, bytes);
        }
        let out = stepped!(e, Event::HeadConflict { seq: 99, root: winner });
        for f in &out {
            if let Effect::Notify { write_id, state, .. } = f {
                answers.entry(*write_id).or_default().push(*state);
            }
        }

        assert_eq!(e.published_root(), winner, "the loser did not take the winner's head");
        // The warm root is the WINNER plus what goes again -- never the
        // loser's own history beside it.
        if !forced {
            theirs.insert(b"mine".to_vec(), vec![1u8; 40]);
        }
        theirs.insert(b"also-mine".to_vec(), vec![2u8; 40]);
        assert_eq!(e.root(), rebuild(&theirs), "forced={forced}: the queue was not re-applied onto the winner, in order");
        let lost: Vec<WriteId> = answers.iter().filter(|(_, st)| st.contains(&State::Lost)).map(|(w, _)| *w).collect();
        if forced {
            assert_eq!(lost, vec![WriteId(1)], "the forced write in the dead commit did not fall Lost, alone");
            assert_eq!(e.lost_fell(), (1, 0), "the forced Lost was not counted");
        } else {
            assert!(lost.is_empty(), "a checked write fell Lost on its first dead commit: {lost:?}");
        }
        assert_eq!(answers[&WriteId(2)], vec![State::Accepted], "the queued write was told something twice");
        assert_eq!(e.queued_writes(), if forced { 1 } else { 2 }, "forced={forced}: the queue is not what goes again");
        println!("  conflict (forced={forced}): loser adopts the winner's head; lost {lost:?}; the rest re-applied on it");
    }
}

/// A write whose edit is still in the tree is never reported `Failed`.
///
/// `Failed` is terminal and means *this edit is not in the tree and never
/// will be*. Reporting it for a write that is merely blocked tells a client
/// to re-submit while the original is still on its way: the edit lands twice,
/// and a write someone else made in between is overwritten by the
/// re-submission. Kept as a regression test because the engine did exactly
/// this, and nothing else in the suite could see it.
#[test]
fn a_write_still_in_the_tree_is_never_reported_failed() {
    let net = Network::default();
    let mut e = boot(
        &net,
        Params {
            max_accept_age: 4,
            ..Params::default()
        },
    );
    // Write 1 opens a commit the network never confirms.
    let _ = stepped!(
        e,
        Event::forced_write(ClientId(1), WriteId(1), vec![(b"a".to_vec(), Op::Put(vec![1u8; 40]))])
    );
    // Write 2 folds behind it.
    let _ = stepped!(
        e,
        Event::forced_write(ClientId(1), WriteId(2), vec![(b"b".to_vec(), Op::Put(vec![2u8; 40]))])
    );
    let mut failed = false;
    for t in 1..=20u64 {
        for f in stepped!(e, Event::Tick(t)) {
            if let Effect::Notify {
                write_id: WriteId(2),
                state: State::Failed { .. },
                ..
            } = f
            {
                failed = true;
            }
        }
    }
    let in_tree = freenet_prolly::read::get(e.blocks(), &e.root(), b"b")
        .ok()
        .flatten()
        .is_some();
    println!("PROBE reported Failed = {failed}; edit still in the tree = {in_tree}");
    assert!(
        !(failed && in_tree),
        "a write was reported Failed while its edit is still in the tree and will publish"
    );
}

/// The context is lost while a head is in flight, and a client can still find
/// out what happened to its write.
///
/// The context cache is an in-process `DashMap` with a 10-minute TTL, never
/// written to disk: a node restart loses it outright, and so does ten idle
/// minutes. The engine that comes back has no memory of the write at all.
///
/// So the answer cannot come from the engine's memory — it has none — and it
/// must not be a guess. It comes from re-reading the HEAD, which is the one
/// durable record. `AskWrite` about a write the engine does not know gets NO
/// verdict (sdk#196 review): "unknown" is also what an engine says about a
/// write it published and forgot, so `Lost` there would be a guess. The
/// client's timeout is what hands the write back for re-submitting.
///
/// Both halves are asserted, because they differ in what a re-submit does:
///   (a) the head LANDED — re-submitting is a no-op against the same tree;
///   (b) it did NOT — re-submitting reproduces exactly the intended tree.
#[test]
fn a_context_lost_with_a_head_in_flight_leaves_the_write_recoverable() {
    for landed in [true, false] {
        let mut net = Network::default();
        let mut e = boot(&net, Params::default());
        let before = e.published_root();

        // A write, driven until the head is emitted but no further.
        let out = stepped!(
            e,
            Event::forced_write(ClientId(1), WriteId(1), vec![(b"k".to_vec(), Op::Put(vec![3u8; 40]))])
        );
        let mut queue = out;
        let mut head = None;
        let mut guard = 0;
        while let Some(f) = queue.pop() {
            guard += 1;
            assert!(guard < 100_000, "the commit did not reach a head");
            match f {
                Effect::PutPack { id, bytes, .. }
                | Effect::PutBlock { id, bytes, .. } => {
                    net.confirm(id, &bytes);
                    queue.extend(stepped!(e, Event::PutConfirmed(id)));
                }
                Effect::UpdateHead { seq, root, .. } => {
                    head = Some((seq, root));
                    // Deliberately NOT confirmed: this is the window.
                }
                other => common::no_answer_owed(&other),
            }
        }
        let (seq, root) = head.expect("the commit never emitted a head");
        let intended = root;
        assert_ne!(intended, before, "the write did not change the tree");

        // The head either landed on the Register or it did not. Either way
        // the context is gone: DROP the engine.
        if landed {
            net.head = Some((seq, root));
        }
        drop(e);

        // A fresh engine, with nothing but what the network holds.
        let mut e = boot(&net, Params::default());
        assert_eq!(
            e.published_root(),
            if landed { intended } else { before },
            "landed={landed}: the recovered engine did not take the head the \
             Register actually holds"
        );

        // The client asks. The engine has no record, so it says NOTHING:
        // `Lost` would be a guess (sdk#196 review).
        let out = stepped!(
            e,
            Event::AskWrite {
                client: ClientId(1),
                write_id: WriteId(1),
            }
        );
        assert_eq!(
            out.iter()
                .filter_map(|f| match f {
                    Effect::Notify { state, .. } => Some(*state),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            Vec::<State>::new(),
            "landed={landed}: a client asking about a write the engine has no \
             record of was given a verdict; not knowing is not evidence"
        );

        // And re-submitting is safe in both worlds. The write is the same
        // ops, so the tree it produces is the intended one either way — that
        // is what makes re-submitting after a timeout safe.
        warm_from(&mut e, &net);
        let _ = stepped!(
            e,
            Event::forced_write(ClientId(1), WriteId(2), vec![(b"k".to_vec(), Op::Put(vec![3u8; 40]))])
        );
        assert_eq!(
            e.root(),
            intended,
            "landed={landed}: re-submitting the lost write produced a \
             different tree from the one the lost commit would have published"
        );
        println!(
            "  head {}: recovered at {}, the unknown write given no verdict, re-submit \
             reproduces the intended tree",
            if landed { "landed" } else { "did NOT land" },
            if landed {
                "the new root"
            } else {
                "the old root"
            }
        );
    }
}

