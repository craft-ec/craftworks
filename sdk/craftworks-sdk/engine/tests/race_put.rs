//! RACE PUT's accounting (COMMIT-LIFE §P), driven through the engine's own
//! events with the acks under the test's control:
//!
//! * SUPERSEDED stragglers: a later commit that re-codes a group an earlier
//!   commit is still putting WITHDRAWS the earlier commit's blocks there, and
//!   the earlier write is BACKED_UP with the newer version of the group --
//!   not before it is full, and not never.
//! * VALUE blocks are never withdrawn: the same value under two keys is one
//!   block, so a removed leaf's value may still be needed.
//! * The head's race set: an earlier commit's straggler is NEVER counted
//!   present toward a group's k, and `after` names only this commit's acked
//!   blocks.

use engine::{ClientId, Effect, Engine, Event, Op, Params, State, WriteId, PARITY};
use freenet_prolly::node::Node;
use freenet_prolly::{parity, Cid};
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

fn head(fx: &[Effect]) -> Option<(u64, Vec<Cid>)> {
    fx.iter().find_map(|f| match f {
        Effect::UpdateHead { seq, after, .. } => Some((*seq, after.clone())),
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

/// The foreign blocks a published commit asks the node about before BACKED_UP (safety gap class 2).
fn asked_held(fx: &[Effect]) -> BTreeSet<Cid> {
    fx.iter()
        .filter_map(|f| match f {
            Effect::ConfirmHeld { id } => Some(*id),
            _ => None,
        })
        .collect()
}

fn withdrawn(fx: &[Effect]) -> BTreeSet<Cid> {
    fx.iter()
        .filter_map(|f| match f {
            Effect::Withdraw { id } => Some(*id),
            _ => None,
        })
        .collect()
}

/// An engine over a node that holds what it is sent, and every block it was
/// asked to put, by id (the test's view of the tree).
struct Rig {
    e: Engine<Store>,
    known: BTreeMap<Cid, Vec<u8>>,
    /// Leave `ConfirmHeld` unanswered (a test that plays the page itself); else the node, which holds every
    /// block, answers each at once, as the page does for one it confirmed (sdk#416).
    withhold_held: bool,
}

impl Rig {
    /// A published base tree of 600 small rows, every block acked.
    fn base() -> Rig {
        let mut r = Rig { e: common::new_store_params(Params::default()), known: BTreeMap::new(), withhold_held: false };
        let ops: Vec<(Vec<u8>, Op)> = (0..600u32).map(|i| put(&format!("k/{i:06}"), &[(i % 251) as u8; 20])).collect();
        let all = r.commit(1, ops, |_, _| true);
        assert!(states(&all, 1).contains(&State::Published), "the base write did not publish");
        r
    }

    fn step(&mut self, ev: Event) -> Vec<Effect> {
        let mut fx = self.e.step(ev);
        self.e.blocks().absorb(&fx);
        if !self.withhold_held {
            common::answer_held(&mut self.e, &mut fx);
        }
        self.known.extend(puts(&fx));
        fx
    }

    /// Commit `ops` as write `id`, acking every block `ack` allows; land the
    /// head once it is asked for. Every effect, in order.
    fn commit(&mut self, id: u64, ops: Vec<(Vec<u8>, Op)>, ack: impl Fn(&Cid, &[u8]) -> bool) -> Vec<Effect> {
        let fx = self.step(Event::forced_write(ClientId(1), WriteId(id), ops));
        let mut all = fx.clone();
        // Every PUT any step emits, answered by `ack` -- the first wave AND the parity that follows the head
        // (#378 P1-hybrid), which is emitted as the head lands.
        let mut queue: std::collections::VecDeque<(Cid, Vec<u8>)> = puts(&fx).into_iter().collect();
        let mut done: BTreeSet<Cid> = BTreeSet::new();
        let mut landed = false;
        loop {
            while let Some((bid, bytes)) = queue.pop_front() {
                if !done.insert(bid) || !ack(&bid, &bytes) {
                    continue;
                }
                let more = self.step(Event::PutConfirmed(bid));
                queue.extend(puts(&more));
                all.extend(more);
            }
            match head(&all) {
                Some((seq, _)) if !landed => {
                    landed = true;
                    let more = self.step(Event::HeadConfirmed(seq));
                    queue.extend(puts(&more));
                    // A changed group's other member asked about as the head lands: answered as the page does (sdk#454).
                    for id in asked_held(&more) {
                        let held = self.e.blocks().node_holds(&id);
                        all.extend(self.step(if held { Event::PutConfirmed(id) } else { Event::HeldUnknown(id) }));
                    }
                    all.extend(more);
                }
                _ => return all,
            }
        }
    }
}

/// The new LEAF a commit put, holding `key`.
fn leaf_with(fx: &[Effect], key: &[u8]) -> Cid {
    puts(fx)
        .into_iter()
        .find(|(_, b)| Node::parse(b).is_ok_and(|n| n.is_leaf() && (0..n.len()).any(|i| n.key(i) == key)))
        .map(|(id, _)| id)
        .expect("the commit put a leaf holding the key")
}

/// The leaf write 2 (`k/000100` := "two") puts, learned on a probe engine:
/// the tree is deterministic, so a fresh engine puts the same block.
fn l2() -> Cid {
    let mut p = Rig::base();
    let fx = p.step(Event::forced_write(ClientId(1), WriteId(2), vec![put("k/000100", b"two")]));
    leaf_with(&fx, b"k/000100")
}

/// Write 2 published with ITS LEAF as the only straggler (race put: the head
/// signs at k of k+m).
fn with_straggler() -> (Rig, Cid) {
    let l2 = l2();
    let mut r = Rig::base();
    let all = r.commit(2, vec![put("k/000100", b"two")], |id, _| *id != l2);
    assert!(states(&all, 2).contains(&State::Published), "write 2 did not publish with one straggler");
    assert!(!states(&all, 2).contains(&State::ParityComplete), "write 2 BACKED_UP with its leaf un-acked");
    (r, l2)
}

#[test]
fn a_later_commit_re_coding_the_group_withdraws_the_straggler_and_backs_up_the_earlier_write() {
    let (mut r, l2) = with_straggler();
    // Write 3 re-codes the SAME leaf and its group: l2 is garbage now.
    let all = r.commit(3, vec![put("k/000100", b"three")], |_, _| true);
    let w = withdrawn(&all);
    assert!(w.contains(&l2), "the superseded straggler was not withdrawn: {w:?}");
    assert!(states(&all, 3).contains(&State::ParityComplete), "write 3 is fully acked and not BACKED_UP");
    assert!(states(&all, 2).contains(&State::ParityComplete), "write 2's data lives in write 3's full group and it is not BACKED_UP: the straggler is waited on for ever");
    assert_eq!(r.e.backing(), 0, "a Backing still waits on a withdrawn block");
}

#[test]
fn an_earlier_write_is_not_backed_up_while_its_current_group_has_a_hole() {
    let (mut probe, _) = with_straggler();
    let l3 = leaf_with(&probe.step(Event::forced_write(ClientId(1), WriteId(3), vec![put("k/000100", b"three")])), b"k/000100");
    let (mut r, l2) = with_straggler();
    // Write 3 re-codes the group, but ITS new leaf is the straggler now.
    let all = r.commit(3, vec![put("k/000100", b"three")], |id, _| *id != l3);
    assert!(withdrawn(&all).contains(&l2), "the superseded straggler was not withdrawn");
    assert!(states(&all, 3).contains(&State::Published));
    assert!(!states(&all, 2).contains(&State::ParityComplete), "write 2 is BACKED_UP while the current version of its group misses a block");
    assert!(!states(&all, 3).contains(&State::ParityComplete));
    // The hole fills: both are BACKED_UP.
    let fx = r.step(Event::PutConfirmed(l3));
    assert!(states(&fx, 2).contains(&State::ParityComplete), "write 2 never BACKED_UP after its group filled");
    assert!(states(&fx, 3).contains(&State::ParityComplete));
}

#[test]
fn a_value_block_shared_by_an_unchanged_leaf_is_never_withdrawn() {
    // A value over MAX_INLINE is a block of its own; the SAME bytes under two
    // keys far apart (two leaves) are ONE block.
    let big = vec![7u8; freenet_prolly::node::MAX_INLINE + 100];
    let v = freenet_prolly::block_id(freenet_prolly::kind::RAW, &big);
    let mut r = Rig::base();
    let all = r.commit(2, vec![put("k/000050", &big), put("k/000550", &big)], |id, _| *id != v);
    assert!(puts(&all).contains_key(&v), "the shared value was not put as its own block");
    assert!(states(&all, 2).contains(&State::Published));
    assert!(!states(&all, 2).contains(&State::ParityComplete));
    // Write 3 rewrites ONE of the keys: its leaf is removed; the other leaf
    // still references the value.
    let all = r.commit(3, vec![put("k/000050", b"small")], |_, _| true);
    assert!(!withdrawn(&all).contains(&v), "a value block still referenced by an unchanged leaf was WITHDRAWN");
    assert!(!states(&all, 2).contains(&State::ParityComplete), "write 2 BACKED_UP with its value block never acked");
    let fx = r.step(Event::PutConfirmed(v));
    assert!(states(&fx, 2).contains(&State::ParityComplete), "write 2 never BACKED_UP after its value landed");
}

#[test]
fn an_earlier_straggler_is_never_counted_present_and_after_names_only_this_commits_acked_blocks() {
    let (mut r, l2) = with_straggler();
    // l2's group, as its parent lists it.
    let parent = r
        .known
        .values()
        .filter_map(|b| Node::parse(b).ok())
        .find(|n| !n.is_leaf() && parity::group_members(n).iter().any(|(_, m)| m.contains(&l2)))
        .expect("l2's parent was put");
    let (_, members) = parity::group_members(&parent).into_iter().find(|(_, m)| m.contains(&l2)).unwrap();
    // A key in a SIBLING leaf of the same group: write 3 changes that leaf,
    // so l2 is an unchanged member of a group write 3 changes -- un-acked.
    let sibling = *members.iter().find(|m| **m != l2).unwrap();
    let key = Node::parse(&r.known[&sibling]).unwrap().key(0);
    let fx3 = r.step(Event::forced_write(ClientId(1), WriteId(3), vec![(key, Op::Put(b"three".to_vec()))]));
    let new3 = puts(&fx3);
    let pn = new3
        .values()
        .filter_map(|b| Node::parse(b).ok())
        .find(|n| !n.is_leaf() && parity::group_members(n).iter().any(|(_, m)| m.contains(&l2)))
        .expect("write 3 re-codes l2's parent");
    let g = parity::group_members(&pn).into_iter().position(|(_, m)| m.contains(&l2)).unwrap();
    let pids: Vec<Cid> = pn.parity().collect();
    let (_, gm) = parity::group_members(&pn).into_iter().nth(g).unwrap();
    let group_new: BTreeSet<Cid> = gm.iter().chain(&pids[PARITY * g..PARITY * (g + 1)]).copied().filter(|c| new3.contains_key(c)).collect();
    // Write 3's own blocks in that group: its new leaf and PARITY new parity.
    assert!(group_new.len() >= 2, "write 3 put {} block(s) of the group", group_new.len());
    // Ack everything of write 3 EXCEPT all but one of its blocks in that
    // group. Counting l2 would make it (k-2 present) + 1 (l2) + 1 = k: the
    // head would sign. Not counting it: k-1, no head.
    let keep_one = *group_new.iter().next().unwrap();
    let mut all = fx3.clone();
    for id in new3.keys() {
        if !group_new.contains(id) || *id == keep_one {
            all.extend(r.step(Event::PutConfirmed(*id)));
        }
    }
    assert!(head(&all).is_none(), "the head was signed with an earlier commit's UN-ACKED straggler counted present");
    // The straggler lands: the group has k now, and the head signs.
    let more = r.step(Event::PutConfirmed(l2));
    let (_, after) = head(&more).expect("with the earlier straggler acked, the group reaches k and the head signs");
    // `after` names only blocks write 3 put AND that were acked.
    for id in &after {
        assert!(new3.contains_key(id), "after names a block write 3 did not put: {id:?}");
        assert!(!group_new.contains(id) || *id == keep_one, "after names a block of write 3 that was never acked");
    }
    assert!(!after.is_empty(), "after is empty: the check above is vacuous");
}

/// SAVED AND BACKED_UP NEED THE NODE'S WORD (class 2, sdk#416; the architect's rulings): a commit over a FOREIGN
/// tree -- another engine's commit, adopted -- changes a group whose other members include the leaf the other
/// engine put. Every other member is asked about at the commit's START (`ConfirmHeld`); the page answers at once for
/// what it confirmed, and says `HeldUnknown` for the foreign leaf, which counts ABSENT toward k and releases one
/// more of the group's parity into the first wave. Here this page's own new leaf in that group also STALLS, so the
/// group is at k - 1 on the node until that extra parity lands:
/// * no head before it (a foreign member presumed held would sign at k - 1 really there: seed 7's shape);
/// * the head once it lands -- the foreign PUT never landing does not deadlock the commit;
/// * BACKED_UP only when the node has answered for the foreign leaf too.
#[test]
fn a_changed_groups_unconfirmed_member_counts_absent_until_the_node_holds_it() {
    let (foreign, foreign_leaf) = {
        let mut p = Rig::base();
        let all = p.commit(2, vec![put("k/000100", b"foreign")], |_, _| true);
        assert!(states(&all, 2).contains(&State::Published));
        (p.e.published_root(), leaf_with(&all, b"k/000100"))
    };
    let mut r = Rig::base();
    r.withhold_held = true;
    let _ = r.step(Event::HeadRead { epoch: engine::Epoch(1), seq: 2, root: foreign });
    assert_eq!(r.e.published_root(), foreign, "THE SETUP: the foreign head was not adopted");
    // A commit in a SIBLING leaf of the foreign change's, in the same parent group.
    let first = r.step(Event::forced_write(ClientId(1), WriteId(3), vec![put("k/000160", b"three")]));
    let asked: BTreeSet<Cid> = asked_held(&first);
    assert!(asked.contains(&foreign_leaf), "the leaf the other engine put, a member of a changed group, was not asked about at the commit's start");
    let own_leaf = leaf_with(&first, b"k/000160");
    // The page: every other member it confirmed, at once; the foreign leaf it cannot.
    let mut all = first.clone();
    for id in asked.iter().filter(|id| **id != foreign_leaf) {
        all.extend(r.step(Event::PutConfirmed(*id)));
    }
    let extra_fx = r.step(Event::HeldUnknown(foreign_leaf));
    let extra: Vec<Cid> = puts(&extra_fx).into_keys().collect();
    assert_eq!(extra.len(), 1, "HeldUnknown did not release exactly one more of the group's parity");
    all.extend(extra_fx);
    // Every PUT acked but the stalled own leaf and the extra parity: k - 1 of the group on the node.
    let held_back: BTreeSet<Cid> = [own_leaf, extra[0]].into_iter().collect();
    for id in puts(&all).into_keys().filter(|id| !held_back.contains(id)).collect::<Vec<_>>() {
        all.extend(r.step(Event::PutConfirmed(id)));
    }
    assert!(head(&all).is_none(), "the head signed with k - 1 of the group on the node: the unconfirmed foreign leaf was counted");
    // The extra parity lands: k, from this page's own PUTs.
    let signed = r.step(Event::PutConfirmed(extra[0]));
    let (seq, _) = head(&signed).expect("the extra first-wave parity brought the group to k, and no head: the commit deadlocks on a foreign PUT");
    let mut fx = r.step(Event::HeadConfirmed(seq));
    assert!(states(&fx, 3).contains(&State::Published), "not SAVED");
    for id in puts(&fx).into_keys().filter(|id| *id != own_leaf).collect::<Vec<_>>() {
        fx.extend(r.step(Event::PutConfirmed(id)));
    }
    fx.extend(r.step(Event::PutConfirmed(own_leaf)));
    assert!(!states(&fx, 3).contains(&State::ParityComplete), "BACKED_UP with the foreign leaf never confirmed on the node");
    let last = r.step(Event::PutConfirmed(foreign_leaf));
    assert!(states(&last, 3).contains(&State::ParityComplete), "every member held, and not BACKED_UP");
}

/// THE HARNESS ANSWERS AS THE PAGE DOES (sdk#454, the architect): a changed group's other member the NODE does not hold
/// is answered `HeldUnknown` at once by `common::answer_held` -- as the page's `AskHeld` would be -- so the group's
/// deferred parity is released into the first wave, exactly as when a test answers it by hand. It was once left
/// unanswered "by design", a harness quietly different from production. Measured as one extra first-wave PUT against the
/// same commit with the answer withheld (`withhold_held`, the named form).
#[test]
fn the_harness_answers_a_member_the_node_does_not_hold_with_held_unknown() {
    let (foreign, foreign_leaf) = {
        let mut p = Rig::base();
        let all = p.commit(2, vec![put("k/000100", b"foreign")], |_, _| true);
        (p.e.published_root(), leaf_with(&all, b"k/000100"))
    };
    let first_wave = |withhold: bool| {
        let mut r = Rig::base();
        r.withhold_held = withhold;
        let _ = r.step(Event::HeadRead { epoch: engine::Epoch(1), seq: 2, root: foreign });
        assert_eq!(r.e.published_root(), foreign, "THE SETUP: the foreign head was not adopted");
        r.e.blocks().lose(&foreign_leaf);
        let first = r.step(Event::forced_write(ClientId(1), WriteId(3), vec![put("k/000160", b"three")]));
        assert!(asked_held(&first).contains(&foreign_leaf), "THE SETUP: the foreign leaf was not asked about");
        puts(&first).len()
    };
    let (answered, withheld) = (first_wave(false), first_wave(true));
    assert_eq!(answered, withheld + 1, "the harness did not answer the member the node lost with HeldUnknown (one released parity): {answered} first-wave PUTs answered vs {withheld} withheld");
}

/// SUPERSESSION ON A FOREIGN MOVE (the architect: "every published-root
/// move"): another device of the same identity publishes a head whose tree
/// re-codes the group this engine's straggler is in. Adopting it withdraws
/// the straggler, and the write is CARRIED -- never BACKED_UP on the foreign
/// tree's say-so, whose stragglers nothing here tracks -- until this engine's
/// next own commit backs it up.
#[test]
fn a_foreign_head_re_coding_the_group_withdraws_the_straggler_and_the_write_waits_for_the_next_own_commit() {
    // The foreign tree: the same base, then k/000100 := "foreign" (seq 2),
    // built by another engine and held by the same node.
    let foreign = {
        let mut p = Rig::base();
        let all = p.commit(2, vec![put("k/000100", b"foreign")], |_, _| true);
        assert!(states(&all, 2).contains(&State::Published));
        p.e.published_root()
    };
    let (mut r, l2) = with_straggler();
    // A newer head (seq 3) arrives with no commit in flight: adopted.
    let fx = r.step(Event::HeadRead { epoch: engine::Epoch(1), seq: 3, root: foreign });
    assert!(withdrawn(&fx).contains(&l2), "a foreign move re-coding the group did not withdraw the straggler: {:?}", withdrawn(&fx));
    assert!(!states(&fx, 2).contains(&State::ParityComplete), "write 2 BACKED_UP on a FOREIGN tree's say-so");
    assert_eq!(r.e.backing(), 0, "the withdrawn straggler's Backing is still waiting");
    // The next own commit, fully acked, takes the carried write with it -- once the node has answered for the
    // other members of the groups it changed (safety gap class 2), the foreign tree's among them.
    let all = r.commit(4, vec![put("k/000200", b"four")], |_, _| true);
    assert!(!asked_held(&all).is_empty(), "a commit over a foreign tree asked the node about none of its changed groups' other members");
    assert!(states(&all, 4).contains(&State::ParityComplete));
    assert!(states(&all, 2).contains(&State::ParityComplete), "the carried write was never BACKED_UP by the next own commit");
}

/// THE SAME VALUE UNDER MANY KEYS OF ONE LEAF is ONE block filling many
/// slots of the leaf's value group. Counted as a set it filled one slot, the
/// group never reached k, and the commit never published (measured: 50 rows
/// of one 1,400 B value -- 14 puts acked, no head, the write `Stalled`).
#[test]
fn repeated_values_in_one_group_publish() {
    let mut r = Rig::base();
    let v = vec![7u8; freenet_prolly::node::MAX_INLINE + 400];
    let ops: Vec<(Vec<u8>, Op)> = (0..50u32).map(|i| put(&format!("r/{i:06}"), &v)).collect();
    let all = r.commit(2, ops, |_, _| true);
    assert!(head(&all).is_some(), "no head: a group of one value in many slots never reached k");
    assert!(states(&all, 2).contains(&State::Published), "{:?}", states(&all, 2));
    assert!(states(&all, 2).contains(&State::ParityComplete), "{:?}", states(&all, 2));
}

// ---------------------------------------------------------------------------
// COMMIT-LIFE §P's gate tests.
// ---------------------------------------------------------------------------

fn is_parity(id: &Cid, bytes: &[u8]) -> bool {
    freenet_prolly::block_id(freenet_prolly::kind::PARITY, bytes) == *id
}

impl Rig {
    fn base_with(params: Params) -> Rig {
        let mut r = Rig { e: common::new_store_params(params), known: BTreeMap::new(), withhold_held: false };
        let ops: Vec<(Vec<u8>, Op)> = (0..600u32).map(|i| put(&format!("k/{i:06}"), &[(i % 251) as u8; 20])).collect();
        let _ = r.commit(1, ops, |_, _| true);
        r
    }
}

/// §P 1: a commit whose slowest PUTs never answer -- every PARITY block of the
/// commit (3 per changed group, so each group keeps exactly its k members) --
/// still PUBLISHES, and is SAVED, not BACKED_UP. The CONTROL, run here: with
/// race put off (wait for every PUT) the same commit does NOT sign.
#[test]
fn a_commit_whose_slowest_puts_never_answer_still_publishes_saved_not_backed_up() {
    for race in [true, false] {
        let mut r = Rig::base_with(Params { race_put: race, ..Params::default() });
        let all = r.commit(2, vec![put("k/000100", b"two")], |id, b| !is_parity(id, b));
        let held = puts(&all).iter().filter(|(id, b)| is_parity(id, b)).count();
        assert!(held >= 3, "race={race}: the commit coded {held} parity block(s): nothing to hold back");
        if race {
            assert!(states(&all, 2).contains(&State::Published), "race put: {held} slow parity PUTs held the head back");
            assert!(!states(&all, 2).contains(&State::ParityComplete), "BACKED_UP with {held} PUTs never answered");
        } else {
            assert!(head(&all).is_none(), "the CONTROL (wait for all) signed with {held} PUTs un-acked: this test cannot tell race put from it");
        }
    }
}

/// §P 3, RULE 10 as decided on #378 (the P1-hybrid): a commit's FIRST WAVE is its data and ONE parity per changed
/// group -- the root's group of one included (sdk#335) -- so the Sign, sent at k, queues behind at most one extra PUT
/// per group on the node's one queue (F61), not m. No data ack sequences a PUT, nor does the head: the head LANDING
/// sends every group's OTHER m - 1 parity, so the head's UPDATE is not queued behind them either; nothing is dropped,
/// and all of it acked is BACKED_UP.
#[test]
fn the_first_wave_is_the_data_and_one_parity_per_changed_group_and_the_rest_follow_the_sign() {
    let mut r = Rig::base();
    let first = r.step(Event::forced_write(ClientId(1), WriteId(2), vec![put("k/000100", b"two"), put("k/000400", b"four")]));
    let sent = puts(&first);
    let root = r.e.root();
    // Every changed group, as the new nodes list them: its parity ids (the root: its own group of one).
    let mut groups: Vec<Vec<Cid>> = sent
        .values()
        .filter_map(|b| Node::parse(b).ok())
        .filter(|n| !n.is_leaf())
        .flat_map(|n| {
            let ids: Vec<Cid> = n.parity().collect();
            parity::group_members(&n)
                .into_iter()
                .enumerate()
                .filter_map(|(g, (_, members))| {
                    let par = ids.get(PARITY * g..PARITY * (g + 1))?.to_vec();
                    members.iter().chain(&par).any(|c| sent.contains_key(c)).then_some(par)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    groups.push(r.e.root_parity_of(&root));
    let changed: Vec<&Vec<Cid>> = groups.iter().filter(|par| par.iter().any(|p| sent.contains_key(p))).collect();
    assert!(changed.len() >= 2, "THE SETUP: {} changed group(s) (a leaf's parent group and the root's)", changed.len());
    for par in &groups {
        let in_first = par.iter().filter(|p| sent.contains_key(*p)).count();
        assert!(in_first <= 1, "a group put {in_first} parity in its first wave, not one");
    }
    let coded: usize = sent.iter().filter(|(id, b)| is_parity(id, b)).count();
    assert_eq!(coded, changed.len(), "the first wave's parity is not exactly one per changed group");

    // No data ack sequences a PUT, the step that sends the head included: acks for everything but the root, then
    // the root.
    let mut headed: Option<u64> = None;
    let order: Vec<Cid> = sent.keys().copied().filter(|c| *c != root).chain(std::iter::once(root)).collect();
    for id in order {
        let fx = r.step(Event::PutConfirmed(id));
        if let Some((seq, _)) = head(&fx) {
            headed = Some(seq);
        }
        assert!(puts(&fx).is_empty(), "a PUT was sequenced after a data ack (the head's UPDATE would queue behind it)");
    }
    let seq = headed.expect("the head never went");
    // The head LANDS: the rest of the parity goes, and before the commit is told `Published`.
    let mut fx = r.step(Event::HeadConfirmed(seq));
    let told = fx.iter().position(|f| matches!(f, Effect::Notify { .. })).expect("Published");
    let after_head: Vec<Cid> = puts(&fx[..told]).into_keys().collect();
    // NOTHING DROPPED: the follow-ups are exactly every changed group's parity not in the first wave.
    let want: BTreeSet<Cid> = changed.iter().flat_map(|par| par.iter().copied()).filter(|p| !sent.contains_key(p)).collect();
    let got: BTreeSet<Cid> = after_head.iter().copied().collect();
    assert_eq!(got, want, "the parity that follows the Sign is not every group's other m - 1");
    // And BACKED_UP once they are acked, and the node has answered for the changed groups' other members.
    let held = asked_held(&fx);
    for p in after_head.into_iter().chain(held) {
        fx.extend(r.step(Event::PutConfirmed(p)));
    }
    assert!(states(&fx, 2).contains(&State::ParityComplete), "every parity acked, and still not BACKED_UP: {:?}", states(&fx, 2));
}

/// §P 4: a commit with NONE of its root group acked does not sign (the root
/// is a group of one, sdk#335: nothing can rebuild it), and neither does one
/// with a changed group missing k+1 -- here its
/// new leaf AND its PARITY parity (k-1 of k+PARITY left).
#[test]
fn neither_an_unacked_root_nor_a_group_below_k_signs() {
    // The root group (sdk#335: the root and its PARITY parity): everything
    // acked but all 1 + PARITY of it.
    let mut r = Rig::base();
    let fx = r.step(Event::forced_write(ClientId(1), WriteId(2), vec![put("k/000100", b"two")]));
    let root = r.e.root();
    let group: BTreeSet<Cid> = std::iter::once(root).chain(r.e.root_parity_of(&root)).collect();
    assert_eq!(group.len(), 1 + PARITY);
    let mut all = fx.clone();
    for id in puts(&fx).keys().filter(|id| !group.contains(*id)) {
        all.extend(r.step(Event::PutConfirmed(*id)));
    }
    assert!(head(&all).is_none(), "the head was signed with NONE of the root group acked");
    let more = r.step(Event::PutConfirmed(root));
    assert!(head(&more).is_some(), "with the root acked too, the head did not sign");

    // A group below k: its new leaf and all PARITY of its parity held.
    let l2 = l2();
    let mut r = Rig::base();
    let fx = r.step(Event::forced_write(ClientId(1), WriteId(2), vec![put("k/000100", b"two")]));
    let sent = puts(&fx);
    let parent = sent.values().filter_map(|b| Node::parse(b).ok()).find(|n| !n.is_leaf() && parity::group_members(n).iter().any(|(_, m)| m.contains(&l2))).expect("the leaf's parent");
    let g = parity::group_members(&parent).into_iter().position(|(_, m)| m.contains(&l2)).unwrap();
    let pids: Vec<Cid> = parent.parity().collect();
    let hold: BTreeSet<Cid> = std::iter::once(l2).chain(pids[PARITY * g..PARITY * (g + 1)].iter().copied()).collect();
    let mut all = fx.clone();
    for id in sent.keys().filter(|id| !hold.contains(*id)) {
        all.extend(r.step(Event::PutConfirmed(*id)));
    }
    assert!(head(&all).is_none(), "the head was signed with a changed group at k-1 of k+m");
}

/// NOTHING HELD BACK IS RE-DERIVED (#378 P1-hybrid, rule 7): a large commit puts every one of its held-back
/// parity as its head lands, from the bytes it held since the first send -- and is BACKED_UP when they are acked.
#[test]
fn the_held_back_parity_of_a_commit_too_large_to_carry_is_sent_from_its_own_bytes() {
    let mut r = Rig::base();
    let big = vec![put("k/000100", &[7u8; 30 * 1024])];
    let first = r.step(Event::forced_write(ClientId(1), WriteId(2), big));
    let mut all = first.clone();
    for id in puts(&first).keys() {
        all.extend(r.step(Event::PutConfirmed(*id)));
    }
    let (seq, _) = head(&all).expect("every first-wave PUT acked, and no head");
    let landed = r.step(Event::HeadConfirmed(seq));
    let told = landed.iter().position(|f| matches!(f, Effect::Notify { .. })).expect("Published");
    let follow = puts(&landed[..told]);
    let first_parity = puts(&first).iter().filter(|(id, b)| is_parity(id, b)).count();
    println!("first wave: {first_parity} parity; sent as the head landed: {}", follow.len());
    assert_eq!(follow.len(), first_parity * (PARITY - 1), "not every changed group's other m - 1 parity was sent");
    let mut fx = landed.clone();
    for id in follow.keys().copied().chain(asked_held(&landed)) {
        fx.extend(r.step(Event::PutConfirmed(id)));
    }
    assert!(states(&fx, 2).contains(&State::ParityComplete), "every parity acked, and not BACKED_UP: {:?}", states(&fx, 2));
}

/// §P 5: A SINGLE STALL PER GROUP IS STILL RACED (#378 P1-hybrid: k of k + 1). The first wave holds the group's
/// new leaf and ONE parity: with the leaf held back and its parity acked, the group is at k and the head signs.
/// THE CONTROL: with BOTH held, it does not (the rest of the parity follows only the Sign).
#[test]
fn a_single_stall_per_group_is_still_raced_by_its_first_wave_parity() {
    let l2 = l2();
    for hold_parity in [false, true] {
        let mut r = Rig::base();
        let fx = r.step(Event::forced_write(ClientId(1), WriteId(2), vec![put("k/000100", b"two")]));
        let sent = puts(&fx);
        let parent = sent.values().filter_map(|b| Node::parse(b).ok()).find(|n| !n.is_leaf() && parity::group_members(n).iter().any(|(_, m)| m.contains(&l2))).expect("the leaf's parent");
        let groups = parity::group_members(&parent);
        let pids: Vec<Cid> = parent.parity().collect();
        let g = groups.iter().position(|(_, m)| m.contains(&l2)).unwrap();
        let first: Vec<Cid> = pids[PARITY * g..PARITY * (g + 1)].iter().copied().filter(|p| sent.contains_key(p)).collect();
        assert_eq!(first.len(), 1, "the leaf's group put {} parity in its first wave, not one", first.len());
        let mut hold: BTreeSet<Cid> = BTreeSet::from([l2]);
        if hold_parity {
            hold.insert(first[0]);
        }
        let mut all = fx.clone();
        for id in sent.keys().filter(|id| !hold.contains(*id)) {
            all.extend(r.step(Event::PutConfirmed(*id)));
        }
        if hold_parity {
            assert!(head(&all).is_none(), "THE CONTROL: the head signed with the group at k - 1");
        } else {
            assert!(head(&all).is_some(), "one stalled leaf, its first-wave parity acked: the head did not sign at k of k + 1");
        }
    }
}

/// §P 6: the largest write that fit before still publishes, with its parity
/// on top (parity rides above `max_commit_blocks`: ≤ PARITY per changed group).
#[test]
fn the_largest_pre_race_put_write_still_publishes_with_its_parity() {
    let p = Params::default();
    let mut r = Rig { e: common::new_store_params(p), known: BTreeMap::new(), withhold_held: false };
    // Values by reference: one block each, so the data blocks approach the cap.
    let n = (p.max_commit_blocks as u32).saturating_sub(12);
    let ops: Vec<(Vec<u8>, Op)> = (0..n).map(|i| put(&format!("v/{i:06}"), &vec![(i % 251) as u8; 2000])).collect();
    let all = r.commit(1, ops, |_, _| true);
    let sent = puts(&all);
    let parity = sent.iter().filter(|(id, b)| is_parity(id, b)).count();
    let data = sent.len() - parity;
    println!("  {n} rows: {data} data block(s) (cap {}), {parity} parity", p.max_commit_blocks);
    assert!(data + 12 >= p.max_commit_blocks, "the fixture put {data} data blocks: not near the cap");
    assert!(parity > 0);
    assert!(states(&all, 1).contains(&State::Published), "{:?}", states(&all, 1));
    assert!(states(&all, 1).contains(&State::ParityComplete), "{:?}", states(&all, 1));
}

/// A DATA BLOCK REJECTED AFTER THE HEAD IS SENT (sdk#433, the architect's gap): with the head sent before the blocks
/// are in (`head_before_packs`), a rejection that leaves every group reachable does not end the commit, and the
/// REJECTED block is never put again. Since sdk#481 (COMMIT-LIFE C5) the engine re-puts nothing on a clock at all --
/// the page's `send` owns every retry -- so no tick puts it (or anything); the commit ends by an answer (here its
/// head lands: Published), never a loop. (Was: "never put again by settle", when settle re-derived from carried ops.)
#[test]
fn a_data_block_rejected_after_the_head_is_sent_is_never_put_again() {
    let mut r = Rig::base_with(Params { head_before_packs: true, ..Params::default() });
    let t0 = 1_000_000;
    let _ = r.step(Event::Tick(t0));
    let first = r.step(Event::forced_write(ClientId(1), WriteId(4), vec![put("k/000300", b"four")]));
    let victim = leaf_with(&first, b"k/000300");
    let (seq, _) = head(&first).expect("THE SETUP: head_before_packs did not send the head at once");
    // Nothing is answered: the commit is not ready. The new leaf is rejected.
    let fx = r.step(Event::PutRejected(victim));
    assert!(states(&fx, 4).is_empty(), "a rejection after the head was sent ended the write: {:?}", states(&fx, 4));
    // Paces come due, again and again: one tick at a time (a jump past CLOCK_RESET_TICKS is a clock reset), long
    // past every head question's round.
    let mut later = Vec::new();
    for k in 1..=600u64 {
        later.extend(r.step(Event::Tick(t0 + k)));
    }
    assert!(!puts(&later).contains_key(&victim), "a tick put the REJECTED block again");
    assert!(puts(&later).is_empty(), "the engine re-put on a clock (C5: the page's send owns retries)");
    assert!(r.e.rejected_blocks().contains(&victim));
    let landed = r.step(Event::HeadConfirmed(seq));
    assert!(!states(&later, 4).contains(&State::Lost), "silence ended the write Lost");
    assert!(states(&landed, 4).contains(&State::Published), "the commit did not publish when its head landed: {:?}", states(&landed, 4));
}

/// A BLOCK THE NODE'S CONTRACT REJECTS (sdk#433; the architect's rulings): `PutRejected` is a REAL END, never a
/// re-put. A commit in flight needing it (its head not sent) ends: its write is `Failed` -- never `Lost`, whose
/// re-send would re-derive the same block -- and the block is never put again, even on a later `PutFailed`. A
/// published commit's straggler rejected stays owed: never BACKED_UP, recorded as damaged, never put again.
#[test]
fn a_rejected_block_ends_its_commit_failed_and_is_never_put_again() {
    // In flight.
    let mut r = Rig::base();
    let first = r.step(Event::forced_write(ClientId(1), WriteId(2), vec![put("k/000100", b"two")]));
    let victim = *puts(&first).keys().next().expect("a block PUT");
    let fx = r.step(Event::PutRejected(victim));
    assert_eq!(states(&fx, 2), vec![State::Failed { why: engine::FailWhy::BlockRejected }], "the write needing a rejected block was not told Failed (and only that)");
    assert!(!puts(&fx).contains_key(&victim), "the rejected block was put again");
    assert!(!puts(&r.step(Event::PutFailed(victim))).contains_key(&victim), "a later PutFailed put the rejected block again");
    assert!(r.e.rejected_blocks().contains(&victim));
    // A published commit's straggler.
    let mut r = Rig::base();
    let all = r.commit(3, vec![put("k/000200", b"three")], |id, b| !is_parity(id, b));
    assert!(states(&all, 3).contains(&State::Published), "THE SETUP: not published with its parity held");
    let straggler = puts(&all).into_iter().find(|(id, b)| is_parity(id, b)).expect("a parity straggler").0;
    let fx = r.step(Event::PutRejected(straggler));
    assert!(fx.is_empty(), "a rejected straggler produced {:?}", fx);
    assert!(!puts(&r.step(Event::PutFailed(straggler))).contains_key(&straggler), "a rejected straggler was put again");
    assert!(r.e.rejected_blocks().contains(&straggler), "the rejected straggler is not recorded for the dashboard");
}
