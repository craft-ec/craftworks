//! What the executor does with each kind of signer answer, one cell at a time
//! (engineer2's review of sdk#215). The model (`tests/model.rs`) covers the
//! rest; these are the cells it cannot reach reliably: a MISROUTED answer, a
//! fork, a register the signer's node does not hold, and the pace of a
//! wrapper-path `Held` that keeps answering absent.

mod common;
use engine::{ClientId, Op as WriteOp, Params, State, WriteId};
use page::{Ms, Answer, Op, Page, PutPath, BACKOFF_MS, HELD_ABSENTS};
use signer_proto::{Answer as A, Head, Why};

/// A page with one write, driven until it asks the signer; returns it, the
/// time, and the id the sign was asked under. Every PUT is answered ok; the
/// first head read finds no head.
fn at_sign(path: PutPath) -> (Page, u64, u32) {
    at_sign_as(path, ClientId(1))
}

/// [`at_sign`], its write made by `client`.
fn at_sign_as(path: PutPath, client: ClientId) -> (Page, u64, u32) {
    let mut p = Page::new(Params::default(), path);
    p.write(client, WriteId(1), vec![(b"k".to_vec(), WriteOp::Put(b"v".to_vec()))]);
    let now = 10;
    for _ in 0..20 {
        for op in p.take_ops() {
            match op {
                Op::ReadHead { label: page::Label::Head } => p.answer(Answer::Head { label: page::Label::Head, read: None }, Ms(now)),
                Op::Put { id, .. } => p.answer(Answer::PutOk(id), Ms(now)),
                Op::AskHeld { batch, ids } => p.answer(Answer::Held { batch, present: vec![true; ids.len()] }, Ms(now)),
                Op::Sign { id, .. } => return (p, now, id),
                other => panic!("unexpected before the sign: {other:?}"),
            }
        }
    }
    panic!("the page never asked the signer");
}

fn signs(ops: &[Op]) -> usize {
    ops.iter().filter(|o| matches!(o, Op::Sign { .. })).count()
}

/// An answer under ANOTHER id (SG02) is not the sign's, even one shaped like a
/// sign's answer -- a late `NotNext` to a superseded ask, an UNATTRIBUTED
/// `Put` -- and neither is another verb's answer under the sign's own id: the
/// sign is still re-asked at its deadline.
#[test]
fn a_misrouted_answer_does_not_stop_the_sign_being_asked_again() {
    let stale = A::NotNext { current: Head { seq: 7, root: [7; 32] } };
    for (other_id, misrouted) in [
        (true, stale.clone()),
        (true, A::Refused(Why::NotSuccessor)),
        (false, A::Held { present: vec![true] }),
        (false, A::Provisioned),
        (false, A::Refused(Why::BlockCount { max: 128, got: 200 })),
    ] {
        let (mut p, now, id) = at_sign(PutPath::Page);
        let under = if other_id { id.wrapping_add(1000) } else { id };
        p.answer(Answer::Signer { id: under, answer: misrouted.clone() }, Ms(now + 1));
        p.tick(Ms(now + page::rto::RTO_INITIAL_MS as u64));
        assert_eq!(signs(&p.take_ops()), 1, "after a misrouted {misrouted:?} (id {under}) the sign was never asked again");
        assert!(p.unusable().is_empty(), "a misrouted answer was taken as the sign's: {:?}", p.unusable());
    }
}

/// THE CONTROL: a real sign answer DOES end the wait — a `NotNext` is acted
/// on, so no re-ask of that request follows at the deadline.
#[test]
fn control_a_sign_shaped_answer_ends_the_wait() {
    let (mut p, now, id) = at_sign(PutPath::Page);
    p.answer(Answer::Signer { id, answer: A::Refused(Why::NotSuccessor) }, Ms(now + 1));
    p.tick(Ms(now + page::rto::RTO_INITIAL_MS as u64));
    assert_eq!(signs(&p.take_ops()), 0, "a permanent refusal was re-asked");
    assert_eq!(p.unusable().len(), 1);
}

/// THE SAME IDENTITY NEVER FORKS (owner, sdk#225): an older signer's
/// `Refused(Forked{mine, read})` is the Register's tie-break having kept
/// ANOTHER device's head at this seq. It is recovered exactly as `NotNext{read}`:
/// the register is READ (1b), what it holds is adopted, the commit built on the
/// displaced head is `Lost` for the app to send again — and nothing is unusable.
#[test]
fn a_fork_answer_is_recovered_as_not_next_and_nothing_is_unusable() {
    let (mut p, now, id) = at_sign(PutPath::Page);
    let theirs = Head { seq: 1, root: [2; 32] };
    let fork = Why::Forked { mine: Head { seq: 1, root: [1; 32] }, read: theirs };
    p.answer(Answer::Signer { id, answer: A::Refused(fork) }, Ms(now + 1));
    let ops = p.take_ops();
    assert!(ops.contains(&Op::ReadHead { label: page::Label::Head }), "the register was not read after a Forked answer: {ops:?}");
    p.answer(Answer::Head { label: page::Label::Head, read: Some((theirs.seq, theirs.root).into()) }, Ms(now + 2));
    assert_eq!(p.published(), (theirs.seq, theirs.root), "the register's head was not adopted");
    let lost = p.take_notices().into_iter().any(|(_, w, s)| w == WriteId(1) && s == State::Lost);
    assert!(lost, "the write built on the displaced head was not handed back as Lost");
    assert!(p.unusable().is_empty(), "a same-key fork left the page unusable: {:?}", p.unusable());
}

/// `HeadUnknown`: the node does not hold the register, so the page READS it
/// (which makes the node hold it) and asks again after a backoff.
#[test]
fn head_unknown_reads_the_register_then_asks_again_after_a_backoff() {
    let (mut p, now, id) = at_sign(PutPath::Page);
    p.answer(Answer::Signer { id, answer: A::Refused(Why::HeadUnknown) }, Ms(now));
    let ops = p.take_ops();
    assert!(ops.contains(&Op::ReadHead { label: page::Label::Head }), "the register was not read to make it held: {ops:?}");
    p.tick(Ms(now + 1));
    assert_eq!(signs(&p.take_ops()), 0, "asked again with no backoff");
    p.tick(Ms(now + 2 * BACKOFF_MS));
    assert_eq!(signs(&p.take_ops()), 1, "never asked again after its backoff");
    assert!(p.unusable().is_empty());
}

/// `RecordNotSaved` and `RootNotHeld` are retried the same way.
#[test]
fn record_not_saved_and_root_not_held_are_asked_again_after_a_backoff() {
    for why in [Why::RecordNotSaved, Why::RootNotHeld] {
        let (mut p, now, id) = at_sign(PutPath::Page);
        p.answer(Answer::Signer { id, answer: A::Refused(why.clone()) }, Ms(now));
        p.tick(Ms(now + 2 * BACKOFF_MS));
        assert_eq!(signs(&p.take_ops()), 1, "{why:?} was not asked again");
    }
}

/// The wrapper path: `Held` absent is asked AGAIN on a backoff, and the block
/// is PUT again only after `HELD_ABSENTS` in a row — never one PUT per absent.
#[test]
fn a_held_absent_is_asked_again_and_re_put_only_after_several() {
    let mut p = Page::new(Params::default(), PutPath::Wrapper);
    p.write(ClientId(1), WriteId(1), vec![(b"k".to_vec(), WriteOp::Put(b"v".to_vec()))]);
    let mut now = 10;
    let (mut puts, mut asks) = (0usize, 0usize);
    let mut blocks = std::collections::BTreeSet::new();
    for _ in 0..200 {
        for op in p.take_ops() {
            match op {
                Op::ReadHead { label: page::Label::Head } => p.answer(Answer::Head { label: page::Label::Head, read: None }, Ms(now)),
                Op::Put { id, .. } => {
                    puts += 1;
                    blocks.insert(id);
                    p.answer(Answer::PutOk(id), Ms(now));
                }
                // Asks counted per BLOCK asked (sdk#455: many ride one op).
                Op::AskHeld { batch, ids } => {
                    asks += ids.len();
                    p.answer(Answer::Held { batch, present: vec![false; ids.len()] }, Ms(now));
                }
                other => common::unanswered_op(&other),
            }
        }
        now += 50;
        p.tick(Ms(now));
        if asks >= 3 * HELD_ABSENTS as usize {
            break;
        }
    }
    assert!(!blocks.is_empty(), "nothing was put");
    assert!(asks >= 3 * HELD_ABSENTS as usize, "Held was not asked again after an absent ({asks} asks)");
    // Each block once, then once more per HELD_ABSENTS absents — never once
    // per absent.
    let allowed = blocks.len() + asks / HELD_ABSENTS as usize;
    assert!(puts <= allowed, "{puts} PUTs for {} blocks and {asks} Held asks (at most {allowed})", blocks.len());
}

/// The page's clock is MILLISECONDS; the engine's is SECONDS. The engine
/// tells a write `Stalled` after `max_accept_age` (64) SECONDS unconfirmed —
/// so a page ticking 20 times a second over one ordinary second of unanswered
/// puts must not be told Stalled (#215 passed the milliseconds through, and
/// every write that waited 64 ms was reported stalled).
#[test]
fn the_engine_hears_the_page_clock_in_seconds() {
    let mut p = Page::new(Params::default(), PutPath::Page);
    let mut now = 1_000_000;
    p.tick(Ms(now));
    for op in p.take_ops() {
        if op == (Op::ReadHead { label: page::Label::Head }) {
            p.answer(Answer::Head { label: page::Label::Head, read: None }, Ms(now));
        }
    }
    p.write(ClientId(1), WriteId(1), vec![(b"k".to_vec(), WriteOp::Put(b"v".to_vec()))]);
    assert!(p.take_ops().iter().any(|o| matches!(o, Op::Put { .. })), "the write put nothing");
    for _ in 0..20 {
        now += 50;
        p.tick(Ms(now));
        let _ = p.take_ops();
    }
    let stalled = p.take_notices().iter().filter(|(_, _, s)| *s == State::Stalled).count();
    assert_eq!(stalled, 0, "a write was told Stalled after one second: the engine read milliseconds as seconds");
}

/// S1b: an OLD signer's `Forked` while the page already stands on the
/// register's head at that seq. The old rule refuses EVERY ask until the
/// register passes that seq, so the sign is not re-asked (a loop otherwise),
/// it is named once, and it is asked again once a head read shows the
/// register past it.
#[test]
fn an_old_signers_fork_on_the_head_the_page_stands_on_is_not_re_asked_until_the_register_moves() {
    let (mut p, now, id) = at_sign(PutPath::Page);
    // A REAL tree to stand on (the empty one), so the next write applies
    // without a fetch.
    let theirs = Head { seq: 1, root: p.published().1 };
    let fork = || A::Refused(Why::Forked { mine: Head { seq: 1, root: [1; 32] }, read: theirs });
    // Adopt theirs first (S1), then write again: the commit is built on
    // theirs, and the old signer still says Forked.
    p.answer(Answer::Signer { id, answer: fork() }, Ms(now + 1));
    let _ = p.take_ops();
    p.answer(Answer::Head { label: page::Label::Head, read: Some((theirs.seq, theirs.root).into()) }, Ms(now + 2));
    let _ = p.take_notices();
    p.write(ClientId(1), WriteId(2), vec![(b"k2".to_vec(), WriteOp::Put(b"v2".to_vec()))]);
    let mut id2 = None;
    for _ in 0..20 {
        for op in p.take_ops() {
            match op {
                Op::Put { id, .. } => p.answer(Answer::PutOk(id), Ms(now + 3)),
                Op::AskHeld { batch, ids } => p.answer(Answer::Held { batch, present: vec![true; ids.len()] }, Ms(now + 3)),
                Op::Sign { id, prev_seq, prev_root, .. } => {
                    assert_eq!((prev_seq, prev_root), (theirs.seq, theirs.root), "not built on the adopted head");
                    id2 = Some(id);
                }
                other => common::unanswered_op(&other),
            }
        }
        if id2.is_some() {
            break;
        }
    }
    let id2 = id2.expect("the second write asked the signer");
    p.answer(Answer::Signer { id: id2, answer: fork() }, Ms(now + 4));
    let named = p.unusable().iter().filter(|u| u.contains("SIGNER UPGRADE NEEDED")).count();
    assert_eq!(named, 1, "the old signer's refusal was not named once: {:?}", p.unusable());
    for k in 1..=20 {
        p.tick(Ms(now + 4 + k * page::rto::RTO_INITIAL_MS as u64));
        assert_eq!(signs(&p.take_ops()), 0, "re-asked an old signer that refuses every ask: a loop");
    }
    // The register moves past seq 1: the page is not left waiting. The newer
    // head is adopted, and the commit built on seq 1 is dead and handed back
    // `Lost` for the app to send again.
    let later = now + 30 * page::rto::RTO_INITIAL_MS as u64;
    p.answer(Answer::Head { label: page::Label::Head, read: Some((2, [9; 32]).into()) }, Ms(later));
    p.tick(Ms(later + 1));
    assert_eq!(p.published().0, 2, "the register's newer head was not adopted");
    let lost = p.take_notices().into_iter().any(|(_, w, s)| w == WriteId(2) && s == State::Lost);
    assert!(lost, "the commit on the old seq was left waiting instead of handed back");
}

/// A `HeadChanged` hint is READ even while a commit is owed (the architect's
/// #5 on sdk#225): parity and heads are owed right after every commit, which
/// is exactly when another device's head is likeliest to land.
#[test]
fn a_hint_is_read_while_a_commit_is_owed() {
    let (mut p, _, _) = at_sign(PutPath::Page);
    let _ = p.take_ops();
    p.head_hint();
    assert!(p.take_ops().contains(&Op::ReadHead { label: page::Label::Head }), "a hint was dropped because a commit is owed");
}

/// **PUBLISH-LIFE ⁵ (sdk#485 E8): a FINAL refusal ENDS the commit, at once.** The signer's answer is a real end (rule
/// 8): the engine hears `HeadRefused { seq }` and every write of the commit is told `Failed` in the SAME step -- no
/// tick, so no stall bound is involved -- and the signer's why is said ONCE, naming each failed write. Mutant "→ Idle
/// without the event" (the owed head silently dropped) -> the write is never told Failed -> red.
#[test]
fn a_final_sign_refusal_fails_the_commits_writes_at_once_and_names_the_why() {
    for refusal in [Why::NotProvisioned, Why::CannotSign, Why::FromApp] {
        let (mut p, now, id) = at_sign(PutPath::Page);
        let _ = p.take_notices();
        p.answer(Answer::Signer { id, answer: A::Refused(refusal.clone()) }, Ms(now + 1));
        let told: Vec<State> = p.take_notices().into_iter().filter(|(_, w, _)| *w == WriteId(1)).map(|(_, _, s)| s).collect();
        assert_eq!(told, vec![State::Failed { why: engine::FailWhy::SignerRefused }], "{refusal:?}: the write was not told Failed, as SIGNER_REFUSED, in the refusal's own step");
        let said = p.unusable();
        assert_eq!(said.len(), 1, "{refusal:?}: the refusal was not said exactly once: {said:?}");
        assert!(said[0].contains(&format!("{refusal:?}")) && said[0].contains("writes 1"), "{refusal:?}: the said line does not name the why and the failed write: {said:?}");
        // Nothing more is asked for the refused commit.
        p.tick(Ms(now + 10 * page::rto::RTO_INITIAL_MS as u64));
        assert_eq!(signs(&p.take_ops()), 0, "{refusal:?}: a refused commit was signed again");
    }
}

/// **sdk#500: a write the signer's final refusal failed names its cause, and the signer's own why, on the fate the
/// app PULLS** -- not only in a said line beside it. Through the `Server`, which keeps fates: the fate is
/// `Failed { SignerRefused }` with the signer's why, and `SIGNER_REFUSED` is the code the app is told. Mutant "the
/// page records no why" (`signer_why` never filled) -> `signer: None` -> red.
#[test]
fn a_signer_refusal_names_its_cause_and_the_signers_why_on_the_pulled_fate() {
    const SESSION: u64 = 7;
    // A client id is its session over its protocol version (`page::server`'s `session_of`).
    let (p, now, id) = at_sign_as(PutPath::Page, ClientId((SESSION << 16) | u64::from(protocol::CURRENT)));
    let mut s = page::server::Server::new(p, page::server::SignerFacts::default());
    s.node(Answer::Signer { id, answer: A::Refused(Why::CannotSign) }, Ms(now + 1));
    let fate = s.fate(SESSION, 1).expect("the refused write has no fate");
    let page::fates::Fate::Failed { why, signer } = fate else { panic!("the refused write's fate is not Failed: {fate:?}") };
    assert_eq!(why, engine::FailWhy::SignerRefused);
    assert_eq!(why.code(), "SIGNER_REFUSED");
    let signer = signer.expect("the signer's own why did not ride beside the refusal");
    assert!(signer.contains("CannotSign"), "the why beside the refusal is not the signer's: {signer}");
}
