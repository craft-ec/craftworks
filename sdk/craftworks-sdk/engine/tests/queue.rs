//! THE PAGE'S WRITE QUEUE, owned by the engine (R-b; COMMIT-LIFE § A write's
//! stage in the page's queue). The cells the other suites do not reach:
//! a foreign move re-judges the queue and a cascade names its cause
//! (footnote 3); a dead commit's write goes again at the front and falls,
//! named, at its bound (footnote 5); every exit of the first `Applying` write
//! tries the next (footnote 8).

use engine::{leaf_hash, ClientId, Effect, Event, Expect, Op, Params, State, Witness, WriteId};
use std::collections::BTreeMap;

mod common;
use common::Store;

fn w(client: u64, id: u64, key: &[u8], value: &[u8], read: Expect) -> Event {
    Event::Write { client: ClientId(client), write_id: WriteId(id), ops: vec![(key.to_vec(), Op::Put(value.to_vec()))], reads: vec![(key.to_vec(), read)], deferred: false }
}

fn told(fx: &[Effect], id: u64) -> Vec<State> {
    fx.iter()
        .filter_map(|f| match f {
            Effect::Notify { write_id, state, .. } if write_id.0 == id => Some(*state),
            _ => None,
        })
        .collect()
}

/// Another writer's head: a tree of `records`, its blocks on the node.
fn their_head(e: &engine::Engine<Store>, records: &[(&[u8], &[u8])]) -> freenet_prolly::Cid {
    let map: BTreeMap<Vec<u8>, Vec<u8>> = records.iter().map(|(k, v)| (k.to_vec(), v.to_vec())).collect();
    let (root, blocks) = common::tree(&map);
    for (id, bytes) in blocks.0.iter() {
        e.blocks().put(*id, bytes);
    }
    root
}

/// **A CASCADE NAMES ITS CAUSE** (footnote 3). W0 creates k (read: absent),
/// W1 reads W0's value and writes its own, W2 reads W1's. W0's commit is in
/// flight, W1 and W2 queued. Another head wins holding k = theirs: the queue
/// is re-applied there, in order -- W0 conflicts on its own read, W1
/// conflicts `after: W0`, W2 `after: W1` (the write whose value it read).
/// One cause, told once; the rest name it.
#[test]
fn a_cascade_names_its_cause() {
    let mut e = common::new_store_params(Params::default());
    let _ = stepped!(e, w(1, 0, b"k", b"v0", Expect::Absent));
    let _ = stepped!(e, w(1, 1, b"k", b"v1", Expect::Value(leaf_hash(b"v0"))));
    let _ = stepped!(e, w(1, 2, b"k", b"v2", Expect::Value(leaf_hash(b"v1"))));
    assert_eq!(e.queued_writes(), 3, "the setup did not queue three writes");
    let winner = their_head(&e, &[(b"k", b"theirs")]);
    let out = stepped!(e, Event::HeadConflict { seq: 9, root: winner });
    let causes: Vec<(u64, Option<u64>)> = out
        .iter()
        .filter_map(|f| match f {
            Effect::Conflicted { write_id, after, .. } => Some((write_id.0, after.map(|(_, w)| w.0))),
            _ => None,
        })
        .collect();
    assert_eq!(causes, vec![(0, None), (1, Some(0)), (2, Some(1))], "the cascade did not name each write's cause");
    for id in 0..3 {
        assert_eq!(told(&out, id), vec![State::Conflict], "write {id} was not told Conflict once");
    }
    assert_eq!(e.root(), winner, "a conflicted write stayed in the warm root");
    assert_eq!(e.queued_writes(), 0);
}

/// THE CONTROL: the same three writes against a winner that does NOT touch
/// k -- nothing conflicts, and all three are re-applied on it, in order.
#[test]
fn control_a_winner_that_leaves_the_key_alone_conflicts_nothing() {
    let mut e = common::new_store_params(Params::default());
    let _ = stepped!(e, w(1, 0, b"k", b"v0", Expect::Absent));
    let _ = stepped!(e, w(1, 1, b"k", b"v1", Expect::Value(leaf_hash(b"v0"))));
    let _ = stepped!(e, w(1, 2, b"k", b"v2", Expect::Value(leaf_hash(b"v1"))));
    let winner = their_head(&e, &[(b"other", b"x")]);
    let out = stepped!(e, Event::HeadConflict { seq: 9, root: winner });
    assert!(!out.iter().any(|f| matches!(f, Effect::Conflicted { .. })), "a write conflicted on a key the winner did not touch");
    assert_eq!(e.queued_writes(), 3, "the queue was not re-applied whole");
    let mut want = BTreeMap::new();
    want.insert(b"other".to_vec(), b"x".to_vec());
    want.insert(b"k".to_vec(), b"v2".to_vec());
    assert_eq!(e.root(), common::rebuild(&want), "the warm root is not the winner plus the queue, in order");
}

/// **A DEAD COMMIT'S WRITE GOES AGAIN AT THE FRONT, AND FALLS NAMED AT ITS
/// BOUND** (footnote 5): each foreign move that kills its commit AFTER ITS HEAD
/// WAS SENT spends one try (a try counts an attempt to publish: COMMIT-LIFE A8);
/// with `max_write_tries` spent, the next falls `Lost`, counted -- never
/// silently, and never before.
#[test]
fn a_dead_commit_goes_again_at_the_front_and_falls_named_at_its_bound() {
    let tries = Params::default().max_write_tries as u64;
    let mut e = common::new_store_params(Params::default());
    let mut fx = stepped!(e, w(1, 1, b"k", b"mine", Expect::Absent));
    for round in 1..=tries + 1 {
        heading(&mut e, fx);
        let winner = their_head(&e, &[(format!("other{round}").as_bytes(), b"x")]);
        let out = stepped!(e, Event::HeadConflict { seq: 10 * round, root: winner });
        fx = out.clone();
        if round <= tries {
            assert!(told(&out, 1).is_empty(), "round {round}: the write was told {:?} while it had tries left", told(&out, 1));
            assert_eq!(e.queued_writes(), 1, "round {round}: the write left the queue");
        } else {
            assert_eq!(told(&out, 1), vec![State::Lost], "past its bound the write did not fall Lost");
            assert_eq!(e.queued_writes(), 0);
        }
    }
    assert_eq!(e.lost_fell(), (0, 1), "the fall was not counted as tries spent");
}

/// **EVERY EXIT OF THE FIRST `Applying` WRITE TRIES THE NEXT** (footnote 8):
/// a cold write whose blocks never come fails its fetch, `Failed`, and the
/// write behind it -- which was waiting its turn (arrival order) -- is tried
/// in the same step. Without that it would sit `Applying` for ever.
#[test]
fn a_cold_write_that_fails_its_fetch_lets_the_next_one_go() {
    let params = Params { max_apply_rounds: 2, ..Params::default() };
    let mut e = common::new_store_params(params);
    // A tree the engine holds only the root of: every path is cold.
    let records: Vec<(Vec<u8>, Vec<u8>)> = (0..400u32).map(|i| (format!("k/{i:05}").into_bytes(), vec![7u8; 40])).collect();
    let map: BTreeMap<Vec<u8>, Vec<u8>> = records.into_iter().collect();
    let (root, all) = common::tree(&map);
    e.blocks().put(root, all.0.get(&root).expect("the root"));
    let _ = stepped!(e, Event::HeadConflict { seq: 5, root });
    let first = stepped!(e, Event::forced_write(ClientId(1), WriteId(1), vec![(b"k/00100".to_vec(), Op::Put(b"a".to_vec()))]));
    let second = stepped!(e, Event::forced_write(ClientId(1), WriteId(2), vec![(b"k/00300".to_vec(), Op::Put(b"b".to_vec()))]));
    assert!(told(&first, 1).is_empty() && told(&second, 2).is_empty(), "a cold write was answered before its path came");
    let fetch = |fx: &[Effect]| fx.iter().filter_map(|f| if let Effect::FetchBlock { id, .. } = f { Some(*id) } else { None }).collect::<Vec<_>>();
    assert!(fetch(&second).is_empty(), "the second write fetched before its turn: arrival order is apply order");
    // The first write's blocks never come. FIFO, as a paced page answers: every ask takes its turn, so the lost
    // block's group repair (sdk#405) re-asking its members never starves the write's own re-ask.
    let mut asked: std::collections::VecDeque<freenet_prolly::Cid> = fetch(&first).into_iter().collect();
    let mut exit = Vec::new();
    for _ in 0..500 {
        let Some(id) = asked.pop_front() else { break };
        let out = stepped!(e, Event::BlockMissed(id));
        asked.extend(fetch(&out));
        if told(&out, 1).iter().any(|s| matches!(s, State::Failed { .. })) {
            exit = out;
            break;
        }
    }
    assert_eq!(told(&exit, 1), vec![State::Failed { why: engine::FailWhy::FetchBudget }], "the cold write never failed its fetch, by its fetch budget");
    // Tried = accepted, or parked on its path: asking for it, or riding a GET already in flight for it (W5, one ask
    // per block -- the failed write's group repair may still hold a shared block's ask).
    let parked_now = all.0.keys().any(|id| e.readers_of(id).parked_write);
    assert!(!fetch(&exit).is_empty() || told(&exit, 2).contains(&State::Accepted) || parked_now, "the write behind the failed one was not tried in the same step");
    assert_eq!(e.queued_writes(), 1, "the failed write is still queued, or the next one left too");
}

/// **A DEAD COMMIT'S FATE IS READ FROM THE WITNESS, NEVER FROM VALUES**
/// (COMMIT-LIFE ⁵; sdk#293; the architect on docs#27). The new head's ledger
/// records how far this page's writes are in it (`through`):
/// * at or past the dead group's last arrival -> it LANDED unheard: every
///   write of it is `Published`, even when a later head changed its key
///   (landed-then-overwritten: told `Lost`, the app would roll back a write
///   that happened, and might write it again over newer data);
/// * below, or absent -> it did not land: a forced write falls `Lost`, even
///   when the tree happens to hold its values (another writer wrote the same
///   thing: told `Published`, the app would believe a commit that does not
///   carry it).
#[test]
fn a_dead_commits_fate_is_read_from_the_witness_never_from_values() {
    for (case, landed, winner_has_value) in [("landed, then overwritten", true, false), ("not there, values coincide", false, true), ("not there", false, false)] {
        let mut e = common::new_store_params(Params::default());
        let fx = stepped!(e, Event::forced_write(ClientId(1), WriteId(1), vec![(b"k".to_vec(), Op::Put(b"mine".to_vec()))]));
        heading(&mut e, fx);
        let through = e.committing_through().expect("the commit in flight carries an arrival number");
        let winner = if winner_has_value { their_head(&e, &[(b"k", b"mine"), (b"theirs", b"x")]) } else { their_head(&e, &[(b"k", b"newer"), (b"theirs", b"x")]) };
        e.set_witness(Some(if landed { Witness::Through(through) } else { Witness::NotThere }));
        let out = stepped!(e, Event::HeadConflict { seq: 9, root: winner });
        let want = if landed { State::Published } else { State::Lost };
        assert_eq!(told(&out, 1), vec![want], "{case}: told {:?}", told(&out, 1));
        assert_eq!(e.lost_fell(), (u64::from(!landed), 0), "{case}: the fall count");
        assert_eq!(e.landed_by_witness(), u64::from(landed), "{case}: the witness count");
        assert_eq!(e.root(), winner, "{case}: the dead write was applied on the winner");
    }
}

/// **ABSENT IS NOT BELOW** (core dev on docs#27): the head's `through` list is
/// bounded (LRU); with this page's entry EVICTED the ledger cannot say
/// whether the group landed. Every write of it is told `Unknown` -- named,
/// the app told to check -- never `Lost` (it may have landed) and never
/// re-applied (it may be there: a re-judge would call its own landed value a
/// `Conflict`). Checked with a winner that HOLDS the value and one that does
/// not: the answer is the same, because values decide nothing.
#[test]
fn an_evicted_witness_is_unknown_never_lost_or_conflict() {
    for holds in [true, false] {
        let mut e = common::new_store_params(Params::default());
        let fx = stepped!(e, w(1, 1, b"k", b"mine", Expect::Absent));
        heading(&mut e, fx);
        let _ = stepped!(e, Event::forced_write(ClientId(1), WriteId(2), vec![(b"j".to_vec(), Op::Put(b"forced".to_vec()))]));
        let winner = if holds { their_head(&e, &[(b"k", b"mine")]) } else { their_head(&e, &[(b"k", b"newer")]) };
        e.set_witness(Some(Witness::Unknown));
        let out = stepped!(e, Event::HeadConflict { seq: 9, root: winner });
        assert_eq!(told(&out, 1), vec![State::Unknown], "holds={holds}: the checked write of the dead commit");
        assert!(!out.iter().any(|f| matches!(f, Effect::Conflicted { write_id, .. } if write_id.0 == 1)), "holds={holds}: a write that may have landed was re-judged");
        assert_eq!(e.lost_fell(), (0, 0), "holds={holds}: a write that may have landed fell Lost");
        let mut want = BTreeMap::new();
        want.insert(b"k".to_vec(), if holds { b"mine".to_vec() } else { b"newer".to_vec() });
        want.insert(b"j".to_vec(), b"forced".to_vec());
        assert_eq!(e.root(), common::rebuild(&want), "holds={holds}: the warm root is not the winner plus the write queued behind (an Unknown write re-applied?)");
        // The forced write QUEUED behind the commit was never in it: it goes
        // on, re-applied on the winner.
        assert!(!told(&out, 2).contains(&State::Lost), "holds={holds}: the queued write behind fell");
    }
}

/// Put, confirm and publish everything in `fx` and what it causes.
fn drive(e: &mut engine::Engine<Store>, first: Vec<Effect>) -> Vec<Effect> {
    let mut all = first.clone();
    let mut queue = first;
    let mut guard = 0;
    while let Some(f) = queue.pop() {
        guard += 1;
        assert!(guard < 100_000, "the engine did not settle");
        let ev = match f {
            Effect::PutBlock { id, .. } | Effect::PutPack { id, .. } => Event::PutConfirmed(id),
            Effect::UpdateHead { seq, .. } => Event::HeadConfirmed(seq),
            _ => continue,
        };
        let out = stepped!(e, ev);
        all.extend(out.iter().cloned());
        queue.extend(out);
    }
    all
}

/// Confirm a commit's PUTs until its head is SENT (Heading) and stop there. A foreign head then meets a head that
/// LEFT -- COMMIT-LIFE's Heading x E8, the one stage whose witness is read and whose try is spent; a Racing commit's
/// head never left, so nothing of it can have landed (A8, the architect on rev 5).
fn heading(e: &mut engine::Engine<Store>, fx: Vec<Effect>) {
    let mut queue = fx;
    let mut guard = 0;
    while let Some(f) = queue.pop() {
        guard += 1;
        assert!(guard < 10_000, "the commit never sent its head");
        match f {
            Effect::PutBlock { id, .. } | Effect::PutPack { id, .. } => queue.extend(stepped!(e, Event::PutConfirmed(id))),
            Effect::UpdateHead { .. } => return,
            other => common::no_answer_owed(&other),
        }
    }
    panic!("THE SETUP: no head was sent");
}

/// Publish ONE commit (its puts, then its head) and stop: what its head's
/// confirmation emits -- the next cut, in flight -- is returned, not driven.
fn publish_one(e: &mut engine::Engine<Store>, fx: Vec<Effect>) -> Vec<Effect> {
    let mut queue = fx;
    let mut guard = 0;
    while let Some(f) = queue.pop() {
        guard += 1;
        assert!(guard < 10_000, "the commit did not publish");
        match f {
            Effect::PutBlock { id, .. } | Effect::PutPack { id, .. } => queue.extend(stepped!(e, Event::PutConfirmed(id))),
            Effect::UpdateHead { seq, .. } => return stepped!(e, Event::HeadConfirmed(seq)),
            other => common::no_answer_owed(&other),
        }
    }
    panic!("no head was sent");
}

/// **A GROUP WHOSE MIDDLE WRITE CONFLICTS AFTER A FOREIGN MOVE** (K9 §5): the
/// cut [W1 on a, W2 reading b, W3 reading W2's b] is in flight; another head
/// wins with b changed. The queue is re-applied there in order: W2 conflicts,
/// W3 cascades `after: W2`, W1 SHIPS -- and the conflicts are told before
/// the group's Published.
#[test]
fn a_group_whose_middle_write_conflicts_ships_the_rest_and_cascades_its_dependants() {
    let mut e = common::new_store_params(Params::default());
    // A first commit in flight so the three queue and are CUT together.
    let first = stepped!(e, w(1, 0, b"z", b"0", Expect::Absent));
    let _ = stepped!(e, w(1, 1, b"a", b"a1", Expect::Absent));
    let _ = stepped!(e, w(1, 2, b"b", b"b2", Expect::Absent));
    let _ = stepped!(e, w(1, 3, b"b", b"b3", Expect::Value(leaf_hash(b"b2"))));
    let out = publish_one(&mut e, first);
    assert!(told(&out, 0).contains(&State::Published), "the first commit did not publish");
    assert!(e.committing_through().is_some(), "the three writes are not in a commit");
    // The foreign head: z and b changed by another writer.
    let winner = their_head(&e, &[(b"z", b"0"), (b"b", b"theirs")]);
    let out = stepped!(e, Event::HeadConflict { seq: 9, root: winner });
    // `drive` returns what it was given, then everything it caused.
    let out = drive(&mut e, out);
    assert_eq!(told(&out, 2), vec![State::Conflict], "the middle write did not conflict");
    let w3 = out.iter().find_map(|f| match f { Effect::Conflicted { write_id, after, .. } if write_id.0 == 3 => Some(*after), _ => None });
    assert_eq!(w3.flatten().map(|(_, w)| w.0), Some(2), "the dependant did not name its cause");
    assert!(told(&out, 1).contains(&State::Published), "the unrelated write of the group did not ship");
    let conflict_at = out.iter().position(|f| matches!(f, Effect::Notify { write_id, state: State::Conflict, .. } if write_id.0 == 2));
    let published_at = out.iter().position(|f| matches!(f, Effect::Notify { write_id, state: State::Published, .. } if write_id.0 == 1));
    assert!(conflict_at < published_at, "a conflict was told after the group's Published");
}

/// **A GROUP THAT LANDED UNHEARD IS PUBLISHED, AND NEVER COMMITTED TWICE**
/// (K9 §6, ⁵): its confirmation lost, a foreign head built on it arrives
/// witnessing it (`through` at the group's last arrival). Every write of the
/// group is Published; nothing is re-applied; no second commit carries it.
#[test]
fn a_group_that_landed_unheard_is_published_and_not_committed_twice() {
    let mut e = common::new_store_params(Params::default());
    let first = stepped!(e, w(1, 0, b"z", b"0", Expect::Absent));
    for (id, k) in [(1u64, b"a"), (2, b"b"), (3, b"c")] {
        let _ = stepped!(e, w(1, id, k, b"v", Expect::Absent));
    }
    let next = publish_one(&mut e, first);
    heading(&mut e, next);
    let through = e.committing_through().expect("the group is in flight");
    let (commits_before, _) = e.commits_and_writes();
    // Their head built on ours: it holds our group and more.
    let winner = their_head(&e, &[(b"z", b"0"), (b"a", b"v"), (b"b", b"v"), (b"c", b"v"), (b"theirs", b"x")]);
    e.set_witness(Some(Witness::Through(through)));
    let out = stepped!(e, Event::HeadConflict { seq: 9, root: winner });
    for id in 1..=3 {
        assert_eq!(told(&out, id), vec![State::Published], "write {id} of the landed group");
    }
    assert!(e.committing_through().is_none(), "the landed group was committed again");
    assert_eq!(e.queued_writes(), 0);
    assert_eq!(e.commits_and_writes().0, commits_before, "a second commit was counted for a landed group");
    assert_eq!(e.root(), winner);
}

/// **ORDER WITHIN A SESSION HOLDS AT THE BYTE BOUND** (review §2 on sdk#295).
/// A session's large A is told `QueueFull`; its smaller B, which WOULD fit,
/// must be told `QueueFull` too -- taken, B would land and A, made again when
/// room frees, would land over it (the older value winning). Once the
/// session's queue is empty, A made again is taken, then B: the tree ends at
/// B's value. Another session with nothing queued is taken meanwhile (its
/// fair share: the hold is per session).
#[test]
fn a_write_told_queue_full_is_not_overtaken_by_a_smaller_later_one() {
    // The bound counts what the queue HOLDS (sdk#450): its writes' ops AND the warm-apply blocks they pin. So it is
    // set from the first write's held bytes, measured on a twin: A's ops (115) do not fit behind the first, B's (40)
    // do. The first write's value is large (400) so that, once the queue drains, A's held bytes (its ops and its
    // warm leaf) still leave room for B -- the setup asserts both.
    let held_first = {
        let mut t = common::new_store_params(Params::default());
        let _ = stepped!(t, w(1, 0, b"z", &[0u8; 400], Expect::Absent));
        t.queue_load().1
    };
    let params = Params { max_queue_bytes: held_first + 60, ..Params::default() };
    let mut e = common::new_store_params(params);
    let put = |id: u64, client: u64, k: &[u8], v: &[u8]| Event::forced_write(ClientId(client), WriteId(id), vec![(k.to_vec(), Op::Put(v.to_vec()))]);
    let first = stepped!(e, w(1, 0, b"z", &[0u8; 400], Expect::Absent));
    assert_eq!(e.queue_load().1, held_first, "THE SETUP: the twin measured another first write");
    let a = stepped!(e, put(1, 1, b"k", &[b'a'; 80]));
    assert!(matches!(told(&a, 1)[..], [State::QueueFull { .. }]), "the large write was not held back: {:?}", told(&a, 1));
    let b = stepped!(e, put(2, 1, b"k", b"bbbbb"));
    assert!(matches!(told(&b, 2)[..], [State::QueueFull { .. }]), "a smaller LATER write of the same session was taken ahead of the one told QueueFull: {:?}", told(&b, 2));
    let other = stepped!(e, put(9, 2, b"o", b"x"));
    assert!(!told(&other, 9).iter().any(|s| matches!(s, State::QueueFull { .. })), "another session with nothing queued was held by this one's hold");
    let out = drive(&mut e, first);
    let _ = drive(&mut e, out);
    assert_eq!(e.queued_writes(), 0, "the queue did not drain");
    // Made again, in order: A, then B.
    let a2 = stepped!(e, put(3, 1, b"k", &[b'a'; 80]));
    assert!(!told(&a2, 3).iter().any(|s| matches!(s, State::QueueFull { .. })), "the held write was not taken with its session's queue empty");
    assert!(e.queue_load().1 + 40 <= params.max_queue_bytes, "THE SETUP: B does not fit behind A made again ({} held, bound {})", e.queue_load().1, params.max_queue_bytes);
    let b2 = stepped!(e, put(4, 1, b"k", b"bbbbb"));
    assert!(!told(&b2, 4).iter().any(|s| matches!(s, State::QueueFull { .. })), "the hold outlived the write it held for: {:?} queued {}", told(&b2, 4), e.queued_writes());
    let mut fx = a2;
    fx.extend(b2);
    let out = drive(&mut e, fx);
    let _ = drive(&mut e, out);
    let mut want = BTreeMap::new();
    want.insert(b"z".to_vec(), vec![0u8; 400]);
    want.insert(b"o".to_vec(), b"x".to_vec());
    want.insert(b"k".to_vec(), b"bbbbb".to_vec());
    assert_eq!(e.root(), common::rebuild(&want), "the older value landed over the later one");
}
