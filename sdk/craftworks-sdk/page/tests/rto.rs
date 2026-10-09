//! EVERY node call retries on the RTO estimator (main's B2 ruling): per call,
//! adaptive, moving with the network. No fixed deadline anywhere.
//!
//! * a fast node teaches the page a short RTO, and a lost answer is asked
//!   again after about that RTO — not after a fixed 2 s, not after 60 s;
//! * KARN: an answer to a re-sent call is never a sample;
//! * §5.5: a timeout doubles the RTO until a clean answer.

mod common;
use engine::{ClientId, Op as WriteOp, Params, WriteId};
use page::rto::{RTO_FLOOR_MS, RTO_INITIAL_MS};
use page::{Answer, Ms, Op, Page, PutPath};

/// A page past its first head read, answering every op after `delay` ms,
/// for `writes` writes of distinct keys; returns the page and the clock.
fn warmed(writes: u64, delay: u64) -> (Page, u64) {
    let mut p = Page::new(Params::default(), PutPath::Page);
    let mut now = 1_000u64;
    let mut due: Vec<(u64, Answer)> = Vec::new();
    let mut head: Option<(u64, [u8; 32])> = None;
    let mut state: Option<Vec<u8>> = None;
    for w in 0..=writes {
        if w > 0 {
            p.write(ClientId(1), WriteId(w), vec![(format!("k{w}").into_bytes(), WriteOp::Put(b"v".to_vec()))]);
        }
        for _ in 0..400 {
            for op in p.take_ops() {
                let a = match op {
                    Op::Put { id, .. } => Answer::PutOk(id),
                    Op::Get { id } => Answer::GetMissed(id),
                    Op::ReadHead { .. } => Answer::Head { label: page::Label::Head, read: head.map(Into::into) },
                    // A signer that signs whatever it is asked: the clock is
                    // the subject here, not the signing rule.
                    Op::Sign { id, seq, root, .. } => {
                        let record = [seq.to_le_bytes().as_slice(), &root].concat();
                        head = Some((seq, root));
                        state = Some(record.clone());
                        Answer::Signer { id, answer: signer_proto::Answer::Signed(record) }
                    }
                    Op::Update { .. } => Answer::Updated { label: page::Label::Head },
                    Op::AskHeld { batch, ids } => Answer::Held { batch, present: vec![true; ids.len()] },
                    Op::PutApp { key } => Answer::AppPutOk(key),
                    Op::Ext(_) => continue,
                };
                due.push((now + delay, a));
            }
            now += 1;
            let (ready, later): (Vec<_>, Vec<_>) = due.into_iter().partition(|(t, _)| *t <= now);
            due = later;
            for (_, a) in ready {
                p.answer(a, Ms(now));
            }
            p.tick(Ms(now));
            if due.is_empty() && !p.waiting() {
                break;
            }
        }
    }
    let _ = state;
    (p, now)
}

#[test]
fn a_fast_node_teaches_a_short_rto() {
    let (p, _) = warmed(10, 5);
    let (rto, srtt, _) = p.clock();
    assert!(srtt.is_some_and(|s| s < 20.0), "SRTT did not learn a 5 ms node: {srtt:?}");
    assert!((rto as f64) < 200.0, "RTO {rto} ms on a 5 ms node");
    assert!((rto as f64) >= RTO_FLOOR_MS, "RTO {rto} ms under its floor");
}

/// THE STALL CASE: on a node the page has learnt to be fast, one PUT's answer
/// is lost. The PUT is asked again after about one RTO — well under the old
/// fixed 2 s, and nowhere near F52's 60 s.
#[test]
fn a_lost_answer_is_asked_again_after_about_one_rto() {
    let (mut p, mut now) = warmed(10, 5);
    let (rto, _, _) = p.clock();
    p.write(ClientId(1), WriteId(100), vec![(b"stall".to_vec(), WriteOp::Put(b"v".to_vec()))]);
    let sent_at = now;
    let first: Vec<_> = p.take_ops().into_iter().filter(|o| matches!(o, Op::Put { .. })).collect();
    assert!(!first.is_empty(), "the write put nothing");
    // Nothing is answered. When is each PUT asked again?
    let mut resent_at = None;
    for _ in 0..5_000 {
        now += 1;
        p.tick(Ms(now));
        if p.take_ops().iter().any(|o| matches!(o, Op::Put { .. })) {
            resent_at = Some(now);
            break;
        }
    }
    let after = resent_at.expect("never asked again") - sent_at;
    assert!(after >= rto, "asked again after {after} ms, before its RTO ({rto} ms)");
    assert!(after <= rto + 5, "asked again after {after} ms, not at its RTO ({rto} ms)");
    assert!(after < 2_000, "asked again after {after} ms: a fixed 2 s deadline");
}

/// KARN'S RULE: the answer to a RE-SENT call is never a sample. A slow answer
/// to the second copy leaves SRTT where the clean answers put it.
#[test]
fn a_re_sent_calls_answer_is_not_a_sample() {
    let (mut p, mut now) = warmed(10, 5);
    let (_, srtt_before, _) = p.clock();
    p.write(ClientId(1), WriteId(100), vec![(b"karn".to_vec(), WriteOp::Put(b"v".to_vec()))]);
    let puts: Vec<[u8; 32]> = p.take_ops().into_iter().filter_map(|o| if let Op::Put { id, .. } = o { Some(id) } else { None }).collect();
    // Let every PUT time out once and be re-sent...
    let mut resent = false;
    for _ in 0..5_000 {
        now += 1;
        p.tick(Ms(now));
        if p.take_ops().iter().any(|o| matches!(o, Op::Put { .. })) {
            resent = true;
            break;
        }
    }
    assert!(resent);
    // ...then answer them very late: 3 s after the re-send.
    now += 3_000;
    for id in puts {
        p.answer(Answer::PutOk(id), Ms(now));
    }
    let (_, srtt_after, _) = p.clock();
    assert_eq!(srtt_after, srtt_before, "a re-sent call's answer was taken as a sample");
}

/// §5.5: with no answers at all, each timeout doubles the RTO.
#[test]
fn with_no_answers_the_rto_backs_off() {
    let mut p = Page::new(Params::default(), PutPath::Page);
    let (start, _, _) = p.clock();
    assert_eq!(start as f64, RTO_INITIAL_MS);
    let mut now = 1_000u64;
    let _ = p.take_ops(); // the first head read, never answered
    let mut seen = Vec::new();
    for _ in 0..20_000 {
        now += 10;
        p.tick(Ms(now));
        if !p.take_ops().is_empty() {
            seen.push(p.clock().0);
            if seen.len() == 3 {
                break;
            }
        }
    }
    assert_eq!(seen, vec![2_000, 4_000, 8_000], "the RTO did not double per timeout");
}

/// sdk#175 at the executor: the engine's own head read that is never
/// answered is asked again on its RTO for as long as it takes — it never
/// becomes `HeadMissing`, which would open an EMPTY tree — and nothing ENDS
/// it (rule 8): the page says "not answering for N s" and keeps asking.
#[test]
fn an_unanswered_head_read_is_never_an_empty_tree() {
    let mut p = Page::new(Params::default(), PutPath::Page);
    let mut now = 1_000u64;
    let mut asked = 0;
    for _ in 0..40_000 {
        now += 1;
        p.tick(Ms(now));
        asked += p.take_ops().iter().filter(|o| **o == Op::ReadHead { label: page::Label::Head }).count();
    }
    assert!(asked >= 3, "the head read was not asked again ({asked})");
    // A write now: had the engine taken "no head", it would commit onto the
    // EMPTY tree and ask to sign from genesis. Every PUT is ANSWERED, so that
    // sign is reachable (a check with its puts unanswered could never see one:
    // main's M2 survived it) — and it must not be asked. Only the head read
    // stays silent.
    p.write(ClientId(1), WriteId(1), vec![(b"k".to_vec(), WriteOp::Put(b"v".to_vec()))]);
    for _ in 0..100 {
        now += 10;
        p.tick(Ms(now));
        for op in p.take_ops() {
            match op {
                Op::Put { id, .. } => p.answer(Answer::PutOk(id), Ms(now)),
                Op::AskHeld { batch, ids } => p.answer(Answer::Held { batch, present: vec![true; ids.len()] }, Ms(now)),
                Op::Sign { .. } => panic!("a write was signed onto a tree nobody read"),
                other => common::unanswered_op(&other),
            }
        }
    }
    // RULE 8: silence ENDS nothing. The page says what it waits on and for
    // how long, and keeps asking; it is not made unusable by a clock.
    assert!(p.unusable().is_empty(), "silence ended the page: {:?}", p.unusable());
    let (what, ms) = p.not_answering().expect("40 s of silence and the page does not say what it waits on");
    assert_eq!(what, "the head read", "the page names the wrong wait");
    assert!(ms >= 40_000, "the head read has been silent since the first tick, but the page says {ms} ms");
}
