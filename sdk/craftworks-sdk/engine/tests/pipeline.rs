//! Acceptance for the write pipeline (craftworks-sdk#24).
//!
//! Everything here is deterministic: no node, no clock, no randomness that is
//! not seeded and printed. That is the whole point of a sans-IO core — an
//! interleaving a live network produces once a week is an ordinary test here.

use engine::{ClientId, Effect, Engine, Event, Op, Params, State, WriteId, PARITY};
use freenet_prolly::Cid;
use std::collections::{BTreeMap, BTreeSet};

mod common;
use common::{rebuild, Harness, Store};

fn w(n: u64) -> WriteId {
    WriteId(n)
}
fn c(n: u64) -> ClientId {
    ClientId(n)
}

fn write(client: u64, id: u64, ops: Vec<(Vec<u8>, Op)>) -> Event {
    Event::forced_write(c(client), w(id), ops)
}

fn put(k: &str, v: &[u8]) -> (Vec<u8>, Op) {
    (k.as_bytes().to_vec(), Op::Put(v.to_vec()))
}

/// The states reported for each write, in the order they were reported.
#[derive(Default, PartialEq, Eq, Debug)]
struct Seen(BTreeMap<(ClientId, WriteId), Vec<State>>);

impl Seen {
    fn absorb(&mut self, effects: &[Effect]) {
        for e in effects {
            if let Effect::Notify {
                client,
                write_id,
                state,
            } = e
            {
                self.0.entry((*client, *write_id)).or_default().push(*state);
            }
        }
    }
    fn of(&self, client: u64, id: u64) -> &[State] {
        self.0
            .get(&(c(client), w(id)))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

/// Is this a sequence a write can legally report?
///
/// A prefix of `accepted (stalled)? published parity-complete`, or a terminal
/// (`failed` / `lost` / `busy`) with nothing after it. Each state at most
/// once: a caller that is told `Published` twice cannot tell a retry from
/// progress.
///
/// Written out rather than derived from the enum's declaration order, because
/// the legal ORDER is a rule about the protocol and the enum's order is an
/// implementation detail that happens to agree with it today. It is also what
/// catches the shape that was wrong here before: `Failed` followed by
/// `Published`, which is two contradictory answers to one question.
pub fn valid_sequence(states: &[State]) -> bool {
    const HAPPY: [State; 4] = [
        State::Accepted,
        State::Stalled,
        State::Published,
        State::ParityComplete,
    ];
    let mut at = 0usize;
    for (i, s) in states.iter().enumerate() {
        if matches!(s, State::Failed { .. } | State::Lost | State::Busy) {
            return i + 1 == states.len();
        }
        match HAPPY[at..].iter().position(|h| h == s) {
            Some(k) => at += k + 1,
            None => return false,
        }
    }
    true
}

fn ids(effects: &[Effect]) -> Vec<Cid> {
    effects
        .iter()
        .filter_map(|e| match e {
            // (A changed group's other member the node is asked about is answered by the harness at once, sdk#416:
            // never a PUT this sweep may fail.)
            Effect::PutPack { id, .. } | Effect::PutBlock { id, .. } => Some(*id),
            _ => None,
        })
        .collect()
}

fn pack_ids(effects: &[Effect]) -> Vec<Cid> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::PutPack { id, .. } => Some(*id),
            _ => None,
        })
        .collect()
}

fn head_of(effects: &[Effect]) -> Option<(u64, Cid, Vec<Cid>)> {
    effects.iter().find_map(|e| match e {
        Effect::UpdateHead { seq, root, after, .. } => Some((*seq, *root, after.clone())),
        _ => None,
    })
}

fn rng(seed: u64) -> impl FnMut() -> u64 {
    let mut s = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    }
}

/// RACE PUT (COMMIT-LIFE §P): the head is signed when the ROOT is acked and
/// every changed group has k of its k+m -- not when every block is. Two
/// values too large to ride in the leaf, so the leaf's value group has k = 2
/// members and PARITY parity; the leaf IS the root, a group of ONE (sdk#335)
/// with its own PARITY parity, so any one of the root's 1 + PARITY must be acked.
#[test]
fn one_write_reaches_published_when_its_root_and_groups_are_recoverable() {
    let mut e = common::new_store_params(Params {
        max_packed_value: 1024,
        ..Params::default()
    });
    let mut seen = Seen::default();

    let out = stepped!(
        e,
        write(
            1,
            1,
            vec![
                (b"a".to_vec(), Op::Put(vec![1u8; 4000])),
                (b"b".to_vec(), Op::Put(vec![2u8; 4000])),
            ],
        )
    );
    seen.absorb(&out);
    assert_eq!(seen.of(1, 1), &[State::Accepted], "a write is accepted before anything is on the network");
    let blocks: Vec<(Cid, Vec<u8>)> = out
        .iter()
        .filter_map(|f| match f {
            Effect::PutBlock { id, bytes, .. } => Some((*id, bytes.clone())),
            _ => None,
        })
        .collect();
    let is = |k: u8, (id, b): &(Cid, Vec<u8>)| freenet_prolly::block_id(k, b) == *id;
    let root: Vec<Cid> = blocks.iter().filter(|x| is(freenet_prolly::kind::TREE_NODE, x)).map(|x| x.0).collect();
    let values: Vec<Cid> = blocks.iter().filter(|x| is(freenet_prolly::kind::RAW, x)).map(|x| x.0).collect();
    let parity: Vec<Cid> = blocks.iter().filter(|x| is(freenet_prolly::kind::PARITY, x)).map(|x| x.0).collect();
    // THE FIRST WAVE (#378 P1-hybrid): ONE parity per changed group -- the value group's and the root's (sdk#335).
    assert_eq!((root.len(), values.len(), parity.len()), (1, 2, 2), "the first send is not root + 2 values + ONE parity per group");
    assert!(head_of(&out).is_none(), "the head moved before a single block was confirmed");

    let rp_all: BTreeSet<Cid> = e.root_parity_of(&root[0]).into_iter().collect();
    assert_eq!(rp_all.len(), PARITY, "the root's parity is not PARITY blocks");
    let fw_rp: Vec<Cid> = parity.iter().copied().filter(|p| rp_all.contains(p)).collect();
    let fw_gp: Vec<Cid> = parity.iter().copied().filter(|p| !rp_all.contains(p)).collect();
    assert_eq!((fw_gp.len(), fw_rp.len()), (1, 1), "not one parity of each group in the first wave");

    // The value group at k = 2 (a value and its first-wave parity), the root group at 0 of its 1 + PARITY: NO head.
    for id in [values[0], fw_gp[0]] {
        let out = stepped!(e, Event::PutConfirmed(id));
        seen.absorb(&out);
        assert!(head_of(&out).is_none(), "the head moved with none of the root group acked");
    }
    // The root's first-wave parity lands, the root itself still out: the head goes out (any 1 of its 1 + PARITY).
    let out = stepped!(e, Event::PutConfirmed(fw_rp[0]));
    seen.absorb(&out);
    let (seq, head_root, after) = head_of(&out).expect("root group and value group at k: the head must move (race put)");
    assert_eq!(seq, 1);
    assert_eq!(head_root, e.root());
    let acked: BTreeSet<Cid> = [values[0], fw_gp[0], fw_rp[0]].into_iter().collect();
    let named: BTreeSet<Cid> = after.into_iter().collect();
    assert!(named.is_subset(&acked), "UpdateHead names a block that is not acked: {:?}", named.difference(&acked).collect::<Vec<_>>());
    assert!(named.contains(&fw_rp[0]), "UpdateHead does not name the root parity it depends on");
    assert!(puts_of(&out).is_empty(), "a follow-up parity PUT went with the Sign: the head's UPDATE would queue behind it");
    assert_eq!(seen.of(1, 1), &[State::Accepted]);

    let out = stepped!(e, Event::HeadConfirmed(seq));
    seen.absorb(&out);
    assert_eq!(seen.of(1, 1), &[State::Accepted, State::Published], "SAVED at k, not BACKED_UP");
    // THE REST OF THE PARITY follows when the head LANDS: every group's other m - 1.
    let follow = puts_of(&out);
    assert_eq!(follow.len(), 2 * (PARITY - 1), "not every group's other m - 1 parity followed the head");
    assert!(follow.iter().all(|p| !parity.contains(p)), "a first-wave parity was sent again");
    // The stragglers land -- the root, the other value and the follow-up parity: BACKED_UP.
    let rest: Vec<Cid> = [values[1], root[0]].into_iter().chain(follow).collect();
    for id in rest {
        let out = stepped!(e, Event::PutConfirmed(id));
        seen.absorb(&out);
    }
    assert_eq!(seen.of(1, 1), &[State::Accepted, State::Published, State::ParityComplete], "states must arrive once each, in order");
}

/// The blocks `out` puts, in order.
fn puts_of(out: &[Effect]) -> Vec<Cid> {
    out.iter().filter_map(|f| if let Effect::PutBlock { id, .. } = f { Some(*id) } else { None }).collect()
}

/// Confirm every put and head the engine asks for, until it asks for none.
fn drive(e: &mut Engine<Store>, seen: &mut Seen, first: Vec<Effect>) {
    let mut queue = first;
    let mut guard = 0;
    while let Some(f) = queue.pop() {
        guard += 1;
        assert!(guard < 100_000, "the engine did not settle");
        let ev = match f {
            Effect::PutBlock { id, .. } | Effect::PutPack { id, .. } | Effect::ConfirmHeld { id } => Event::PutConfirmed(id),
            Effect::UpdateHead { seq, .. } => Event::HeadConfirmed(seq),
            _ => continue,
        };
        let out = stepped!(e, ev);
        seen.absorb(&out);
        queue.extend(out);
    }
}

/// A write during a commit is QUEUED (R-b; COMMIT-LIFE § A write's stage):
/// `Accepted` into the warm root at once, nothing put on the network, and
/// committed -- one write per commit, rebuilt on the root the first one
/// published -- when the first publishes. It used to be refused `Busy`, and
/// an outbox re-sent it for ever.
#[test]
fn a_write_during_a_commit_is_queued_and_commits_after_it() {
    let mut e = common::new_store_params(Params::default());
    let mut seen = Seen::default();

    let first = stepped!(e, write(1, 1, vec![put("k/1", b"one")]));
    seen.absorb(&first);
    assert!(!ids(&first).is_empty(), "the first write shipped nothing");
    let published_before = e.published_root();

    let mut expected: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
    expected.insert(b"k/1".to_vec(), b"one".to_vec());
    for (client, id, key) in [(1u64, 2u64, "k/2"), (2, 3, "k/3")] {
        let out = stepped!(e, write(client, id, vec![put(key, b"v")]));
        seen.absorb(&out);
        assert_eq!(seen.of(client, id), &[State::Accepted], "a write during a commit was not taken");
        assert!(ids(&out).is_empty(), "a queued write put blocks on the network before its turn");
        assert!(head_of(&out).is_none(), "a queued write moved a head before its turn");
        expected.insert(key.as_bytes().to_vec(), b"v".to_vec());
        assert_eq!(e.root(), common::rebuild(&expected), "the warm root is not every write taken, in order");
        assert_eq!(e.published_root(), published_before, "a queued write moved the published root");
    }

    // Every write publishes, each in its own commit, in order.
    drive(&mut e, &mut seen, first);
    for (client, id) in [(1u64, 1u64), (1, 2), (2, 3)] {
        assert!(
            seen.of(client, id).contains(&State::Published),
            "write ({client}, {id}) did not publish: {:?}",
            seen.of(client, id)
        );
    }
    assert_eq!(e.published_root(), common::rebuild(&expected), "the published tree is not every write");
    // GROUP COMMIT (K9): the first write's commit, then the two queued behind
    // it cut together when it publishes -- two commits for three writes.
    assert_eq!(e.published_seq(), 2, "the two writes queued behind the first did not go as one group");
    assert_eq!(e.commits_and_writes(), (2, 3), "writes per commit");
    assert_eq!(e.rebuild_differs(), 0, "K9: a rebuilt commit's root differed from its warm root");
    assert_eq!(e.own_publish_rederived(), 0, "the warm root was re-derived on own publish");
    assert_eq!(e.queued_writes(), 0, "the queue did not drain");
}

/// One seed of the interleaving sweep. The rng is seeded per call, so a seed
/// is one fixed sequence of writes, failures, duplicates and strangers.
struct SweepSeed {
    published: Cid,
    expected: Cid,
    seen: Seen,
    live: Vec<(u64, u64)>,
    pack_failures: usize,
    direct_failures: usize,
    directs: usize,
    retries: usize,
}

fn sweep_seed(params: Params, seed: u64) -> SweepSeed {
    let mut r = rng(seed);
    let mut h = Harness::new(params, Store::fresh());
    let mut seen = Seen::default();
    let mut records: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
    let mut live: Vec<(u64, u64)> = Vec::new();
    let mut next_id = 0u64;
    let mut pack_failures = 0usize;
    let mut direct_failures = 0usize;
    let mut directs = 0usize;
    let mut retries_seen = 0usize;

    for _round in 0..6 {
        // A batch of writes, sometimes overwriting earlier keys.
        let n = 1 + (r() % 4) as usize;
        let mut ops = Vec::new();
        for _ in 0..n {
            let k = format!("k/{:04}", r() % 40).into_bytes();
            // Most values ride in the pack; one in eight is over
            // `max_packed_value` and is PUT on its own. Every value used to be
            // under 2 KiB against a 64 KiB threshold, so the direct branch --
            // the one whose failure leaves a commit with an outstanding put
            // that is not in any pack -- was never entered by this sweep at
            // all, and `direct_failures` below is the floor that says so.
            let big = r().is_multiple_of(8);
            let len = if big {
                params.max_packed_value + 1 + (r() % 4096) as usize
            } else {
                1 + (r() % 2000) as usize
            };
            let v = vec![(r() % 251) as u8; len];
            records.insert(k.clone(), v.clone());
            ops.push((k, Op::Put(v)));
        }
        next_id += 1;
        let client = 1 + (r() % 2);
        let out = h.step(write(client, next_id, ops));
        seen.absorb(&out);
        live.push((client, next_id));

        // Answer whatever is outstanding, in a shuffled order, with
        // duplicates and strangers mixed in.
        let mut pending = ids(&out);
        for i in (1..pending.len()).rev() {
            pending.swap(i, (r() % (i as u64 + 1)) as usize);
        }
        let packs: BTreeSet<Cid> = pack_ids(&out).into_iter().collect();
        directs += ids(&out).iter().filter(|i| !packs.contains(*i)).count();
        let mut queue: Vec<Cid> = Vec::new();
        for id in &pending {
            // A failure first, at a position the seed chooses.
            if r().is_multiple_of(3) {
                let again = h.step(Event::PutFailed(*id));
                seen.absorb(&again);
                // A failed put MUST produce a retry, or the commit waits
                // for ever on something that already failed.
                assert!(
                    !again.is_empty(),
                    "seed {seed}: PutFailed re-emitted nothing, so \
                     the commit can never complete"
                );
                if packs.contains(id) {
                    pack_failures += 1;
                } else {
                    direct_failures += 1;
                }
                retries_seen += 1;
                queue.extend(ids(&again));
            }
            queue.push(*id);
            if r().is_multiple_of(4) {
                queue.push(*id); // a duplicate confirmation
            }
            if r().is_multiple_of(5) {
                // A confirmation for a block nothing is waiting on.
                queue.push([(r() % 251) as u8; 32]);
            }
        }
        for id in queue {
            let out = h.step(Event::PutConfirmed(id));
            seen.absorb(&out);
            if let Some((seq, _, _)) = head_of(&out) {
                let out = h.step(Event::HeadConfirmed(seq));
                seen.absorb(&out);
                // A published commit can open the next one; the parity that follows the head (#378 P1-hybrid)
                // goes out as it lands, and is answered too.
                let mut more = ids(&out);
                for i in (1..more.len()).rev() {
                    more.swap(i, (r() % (i as u64 + 1)) as usize);
                }
                for id in more {
                    let out = h.step(Event::PutConfirmed(id));
                    seen.absorb(&out);
                    if let Some((seq, _, _)) = head_of(&out) {
                        let out = h.step(Event::HeadConfirmed(seq));
                        seen.absorb(&out);
                    }
                }
            }
        }
    }

    // Drain anything still in flight so every write can settle.
    for _ in 0..8 {
        let out = h.step(Event::Tick(1000));
        seen.absorb(&out);
    }

    SweepSeed {
        published: h.published_root(),
        expected: rebuild(&records),
        seen,
        live,
        pack_failures,
        direct_failures,
        directs,
        retries: retries_seen,
    }
}

/// Every interleaving publishes the root a rebuild produces.
#[test]
fn any_interleaving_publishes_the_root_a_rebuild_produces() {
    // Counted across the whole sweep and asserted at the end. A failure
    // injected into a value block exercises a different branch from one
    // injected into a PACK, and only the pack branch could leave a commit with
    // nothing outstanding and no retry. This sweep missed that defect the
    // first time because it confirmed every id it had failed, regardless of
    // whether the retry produced anything.
    let mut pack_failures = 0usize;
    let mut direct_failures = 0usize;
    let mut directs = 0usize;
    let mut retries_seen = 0usize;
    // BOTH write paths. Phase 3 ships members only; Phase 4 (#39) turns the
    // pack back on, and the format has to keep working until then — a sweep
    // over one of them would let the other rot with nothing to say so.
    for (arm, params) in [
        ("members", Params::default()),
        (
            "packed",
            Params {
                pack_on_write: true,
                ..Params::default()
            },
        ),
    ] {
        for seed in 1..=24u64 {
            let l = sweep_seed(params, seed);
            assert_eq!(
                l.published, l.expected,
                "seed {seed}: the published root is not the root a rebuild \
             produces"
            );
            // And no write was silently dropped, or reported an impossible life.
            for (client, id) in &l.live {
                let states = l.seen.of(*client, *id);
                assert!(
                    !states.is_empty(),
                    "seed {seed}: write {id} was never reported at all"
                );
                assert!(
                    valid_sequence(states),
                    "seed {seed}: write {id} reported an impossible sequence: \
                 {states:?}"
                );
            }
            for (client, id) in &l.live {
                let states = l.seen.of(*client, *id);
                if states.contains(&State::Published) {
                    assert!(states.contains(&State::ParityComplete), "seed {seed}: write {id} published and every block was acked, but it was never BACKED_UP: {states:?}");
                }
            }
            pack_failures += l.pack_failures;
            direct_failures += l.direct_failures;
            directs += l.directs;
            retries_seen += l.retries;
            println!("  {arm} seed {seed:2}: matches a rebuild");
        }
    }
    // Without this the sweep can inject failures that never land on a pack and
    // report green over a branch it never entered.
    // The pack arm must have hit the pack path. With packing off the write
    // path this counts zero for the members arm, which is correct and is why
    // the sweep runs both.
    assert!(
        pack_failures > 0,
        "{retries_seen} put failures were injected and NONE hit a pack across \
         BOTH arms: the pack retry path was not exercised, and Phase 4 is \
         where it comes back"
    );
    // The other half of the same floor. A pack retry and a direct-block retry
    // are different branches, and a sweep whose values are all small emits no
    // direct block to fail -- which is what this one did until the fixture
    // above was widened.
    assert!(
        directs > 0,
        "the sweep emitted no direct PutBlock at all, so no value exceeded \
         max_packed_value and the pack/direct split was never taken"
    );
    assert!(
        direct_failures > 0,
        "{directs} direct block puts were emitted and NONE was failed: the \
         direct retry path was not exercised"
    );
    println!(
        "  {retries_seen} injected failures: {pack_failures} on packs, \
         {direct_failures} on direct blocks (of {directs} emitted)"
    );
}






/// The core never looks inside a value.
///
/// Sealed values are coming (§11), and the engine must not be the thing that
/// has to change: it moves bytes it cannot read. Opaque bytes go in and the
/// exact same bytes come out, whether they ride in a pack or get their own
/// PUT.
#[test]
fn an_opaque_value_passes_through_untouched() {
    let mut e = common::new_store_params(Params {
        // Small enough that the large value takes its own PUT, so both paths
        // are exercised in one test — which needs the pack path ON, since
        // Phase 3 leaves it off and there would otherwise be only one path.
        max_packed_value: 4096,
        pack_on_write: true,
        ..Params::default()
    });
    let mut seen = Seen::default();
    let sealed: Vec<u8> = (0..9000u32)
        .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
        .collect();
    let small: Vec<u8> = (0..2000u32).map(|i| (i ^ 0x5a) as u8).collect();

    let out = stepped!(
        e,
        write(
            1,
            1,
            vec![
                (b"sealed".to_vec(), Op::Put(sealed.clone())),
                (b"small".to_vec(), Op::Put(small.clone())),
            ],
        )
    );
    seen.absorb(&out);

    let direct: Vec<&Vec<u8>> = out
        .iter()
        .filter_map(|e| match e {
            Effect::PutBlock { bytes, .. } => Some(bytes),
            _ => None,
        })
        .collect();
    assert!(
        direct.iter().any(|b| **b == sealed),
        "the large opaque value is not among the blocks put, byte for byte"
    );
    // And the small one is inside a pack, unaltered.
    let packed: Vec<u8> = out
        .iter()
        .filter_map(|e| match e {
            Effect::PutPack { bytes, .. } => Some(bytes.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(
        packed.windows(small.len()).any(|w| w == small.as_slice()),
        "the small opaque value was altered on its way into a pack"
    );
}

/// No sequence of events makes the core panic.
///
/// A delegate is re-entered with whatever the network hands it, including
/// answers to questions nobody asked. The count is printed and asserted so a
/// run that explored nothing cannot read as a pass.
#[test]
fn no_event_sequence_panics() {
    let mut cases = 0;
    for seed in 1..=40u64 {
        let mut r = rng(seed);
        let mut e = common::new_store_params(Params {
            max_write_bytes: 4096,
            ..Params::default()
        });
        let mut known: Vec<Cid> = Vec::new();
        for _ in 0..60 {
            let ev = match r() % 6 {
                0 => write(
                    1 + r() % 3,
                    r() % 20,
                    vec![(
                        format!("k/{}", r() % 30).into_bytes(),
                        if r().is_multiple_of(5) {
                            Op::Delete
                        } else {
                            Op::Put(vec![(r() % 251) as u8; (r() % 3000) as usize])
                        },
                    )],
                ),
                1 => Event::PutConfirmed(pick(&mut r, &known)),
                2 => Event::PutFailed(pick(&mut r, &known)),
                3 => Event::HeadConfirmed(r() % 8),
                _ => Event::Tick(r() % 16),
            };
            let out = stepped!(e, ev);
            known.extend(ids(&out));
            if known.len() > 200 {
                known.drain(..100);
            }
            cases += 1;
        }
    }
    assert!(cases >= 2000, "only {cases} events were driven");
    println!("  {cases} events across 40 seeds, no panic");
}

fn pick(r: &mut impl FnMut() -> u64, known: &[Cid]) -> Cid {
    if known.is_empty() || r().is_multiple_of(4) {
        [(r() % 251) as u8; 32]
    } else {
        known[(r() % known.len() as u64) as usize]
    }
}

/// Beyond the QUEUE's BYTE bound a write is told `QueueFull` (R-b; COMMIT-LIFE
/// K1): nothing applied, nothing put, the warm root as it was -- backpressure
/// the SDK waits on, not a verdict. There is NO count bound (K9: 300 small
/// writes all queue). And a session with nothing queued is always taken: one
/// tab's burst cannot starve another tab's write (its fair share).
#[test]
fn past_the_queue_byte_bound_a_write_waits_and_another_session_is_still_taken() {
    let mut e = common::new_store_params(Params { max_queue_bytes: 4000, ..Params::default() });
    let mut seen = Seen::default();
    let out = stepped!(e, write(1, 1, vec![(b"a1".to_vec(), Op::Put(vec![1u8; 3000]))]));
    seen.absorb(&out);
    assert_eq!(seen.of(1, 1).first(), Some(&State::Accepted), "a write under the bound was not taken");
    let root_before = e.root();
    // What the queue holds (sdk#450): write 1's ops AND the warm-apply blocks it pins -- the ONE sum.
    let (_, held) = e.queue_load();
    assert!(held > 2 + 3000 + 2 + 33, "THE SETUP: the queue's bytes do not count write 1's warm blocks");
    let out = stepped!(e, write(1, 2, vec![(b"a2".to_vec(), Op::Put(vec![2u8; 3000]))]));
    seen.absorb(&out);
    assert_eq!(seen.of(1, 2), &[State::QueueFull { bytes: held, limit: 4000 }], "past the byte bound the write was not told QueueFull, with the bytes the queue holds");
    assert_eq!(e.root(), root_before, "a write past the bound changed the warm root");
    assert!(ids(&out).is_empty(), "a write past the bound put blocks on the network");
    // ANOTHER session, nothing queued: taken, over the bound or not.
    let out = stepped!(e, write(2, 1, vec![(b"b1".to_vec(), Op::Put(vec![3u8; 3000]))]));
    seen.absorb(&out);
    assert_eq!(seen.of(2, 1).first(), Some(&State::Accepted), "one session's burst starved another session's first write");
    // No COUNT bound: many small writes all queue.
    let mut e = common::new_store_params(Params::default());
    let mut seen = Seen::default();
    for id in 1..=300u64 {
        let out = stepped!(e, write(1, id, vec![(format!("k{id}").into_bytes(), Op::Put(b"v".to_vec()))]));
        seen.absorb(&out);
        assert_eq!(seen.of(1, id).first(), Some(&State::Accepted), "write {id} of 300 small ones was not taken: a count cap");
    }
}


/// Settings a caller can choose must not panic the core three steps later.
///
/// `max_packed_value` above `max_pack` describes a value that must ride in a
/// pack and cannot fit in one. The planner met that as an `unreachable!`,
/// far from the setting that caused it and with nothing in the message
/// pointing back at it.
#[test]
fn impossible_params_are_refused_where_they_are_set() {
    let bad = Params {
        max_pack: 4096,
        max_packed_value: 8192,
        ..Params::default()
    };
    let panicked = std::panic::catch_unwind(move || Engine::new(bad, Store::default()));
    assert!(
        panicked.is_err(),
        "a pack size smaller than the values that must fit in it was accepted"
    );
    // And the pair that only just fits is accepted, so the refusal is the
    // relationship and not a blanket rejection of small packs.
    let ok = Params {
        max_pack: 4096,
        max_packed_value: 4096 - 11,
        ..Params::default()
    };
    let _ = Engine::new(ok, Store::default());
}

/// The limit that bites a packed member is its KIND's, not the pack's.
///
/// The check above bounded `max_packed_value` against `max_pack` — the
/// container — and packed members are `RAW`, whose ceiling is 262,208. So
/// every setting from 262,209 up to `max_pack - 11` satisfied it: with a
/// 1 MiB pack that is a **786,357-wide band** the engine accepted and that
/// builds packs the contract refuses PER MEMBER.
///
/// That band is worse than a panic. The refusal happens at the NODE, remotely,
/// where nothing local says why (F48) — the diagnosable local failure existed
/// and was unreachable, so what survived was the least diagnosable one.
///
/// Driven at the boundary rather than deep in the band: an off-by-one here
/// either refuses a setting the network accepts or admits one it does not, and
/// a value picked from the middle cannot tell those apart.
#[test]
fn a_packed_value_over_its_kinds_limit_is_refused_where_it_is_set() {
    let raw_limit = engine::pack::max_body(core_types::kind::BlockKind::Raw);
    assert_eq!(raw_limit, 262_208, "the contract's RAW ceiling");

    // Inside the old band: it fits a 1 MiB pack, so the pack-size check passes
    // and ONLY the per-member one can catch it.
    let bad = Params {
        max_pack: 1024 * 1024,
        max_packed_value: raw_limit + 1,
        ..Params::default()
    };
    assert!(
        std::panic::catch_unwind(move || Engine::new(bad, Store::default())).is_err(),
        "max_packed_value {} fits the pack but not a RAW member, and was accepted",
        raw_limit + 1,
    );

    // And exactly at the limit is accepted, so this is the relationship and
    // not a blanket refusal of large values.
    let ok = Params {
        max_pack: 1024 * 1024,
        max_packed_value: raw_limit,
        ..Params::default()
    };
    let _ = Engine::new(ok, Store::default());
}


/// A tab can close at any moment. Nothing wakes the engine afterwards (no
/// tick, no wake-ups), so everything a write is promised must happen on its
/// own confirmations. Under race put (COMMIT-LIFE §P) a commit's parity goes
/// out IN THE SAME ROUND as its data, so every write reaches `Published` AND
/// `ParityComplete` with no tick and no `Flush` at all, and a `Flush` after
/// the commits puts nothing. The parity is COUNTED in the commit rounds, so
/// `ParityComplete` cannot pass over a commit that coded none.
#[test]
fn after_a_disconnect_every_write_publishes_and_is_backed_up_with_no_tick_and_no_flush() {
    let run = |flush: bool| -> (Seen, usize, usize) {
        let store = Store::fresh();
        let mut e = Engine::new(Params::default(), store.clone());
        // A NEW tree, and the engine is told so (sdk#223).
        let _ = e.step(Event::HeadMissing);
        let mut seen = Seen::default();
        let mut commit_parity = 0usize;
        // Values by reference, so the leaves carry parity over them.
        let mut queue = Vec::new();
        for n in 1..=3u64 {
            let ops: Vec<(Vec<u8>, Op)> = (0..16u32).map(|i| (format!("k/{n}/{i:04}").into_bytes(), Op::Put(vec![(i % 251) as u8; 1400]))).collect();
            let out = stepped!(e, write(1, n, ops));
            seen.absorb(&out);
            queue.extend(out.clone());
            let mut guard = 0;
            while let Some(f) = queue.pop() {
                guard += 1;
                assert!(guard < 100_000, "the commit did not settle");
                let o = match &f {
                    Effect::PutBlock { id, bytes, .. } => {
                        if freenet_prolly::block_id(freenet_prolly::kind::PARITY, bytes) == *id {
                            commit_parity += 1;
                        }
                        stepped!(e, Event::PutConfirmed(*id))
                    }
                    Effect::PutPack { id, .. } | Effect::ConfirmHeld { id } => stepped!(e, Event::PutConfirmed(*id)),
                    Effect::UpdateHead { seq, .. } => stepped!(e, Event::HeadConfirmed(*seq)),
                    _ => Vec::new(),
                };
                seen.absorb(&o);
                queue.extend(o);
            }
        }
        // The client goes away. No tick is sent here, or ever.
        let mut flush_puts = 0usize;
        if flush {
            let out = stepped!(e, Event::Flush);
            seen.absorb(&out);
            flush_puts = out.iter().filter(|f| matches!(f, Effect::PutBlock { .. } | Effect::PutPack { .. })).count();
        }
        (seen, commit_parity, flush_puts)
    };

    for flush in [false, true] {
        let (seen, commit_parity, flush_puts) = run(flush);
        for n in 1..=3u64 {
            let states = seen.of(1, n);
            assert!(states.contains(&State::Published), "flush={flush}: write {n} did not publish: {states:?}");
            assert!(states.contains(&State::ParityComplete), "flush={flush}: write {n} published but was never BACKED_UP, and no tick is ever coming: {states:?}");
            assert!(valid_sequence(states), "flush={flush}: write {n} reported an impossible sequence: {states:?}");
        }
        assert!(commit_parity > 0, "flush={flush}: the commits put no parity at all, so ParityComplete above says nothing");
        assert_eq!(flush_puts, 0, "flush={flush}: a Flush after the commits put {flush_puts} block(s): parity is still paid late");
    }
}

