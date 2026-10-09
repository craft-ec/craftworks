//! ONE HEAD-JUDGEMENT FOR EVERY REGISTER ANSWER (sdk#396, safety gap (a) of the 2026-09-24 stock-take). A register
//! answer can reach the page as a recovery read (the engine's own `ReadHead` mid-commit), a read-back, a verify or
//! a hint, and every one of them is judged by the same rule before anything is adopted: a head at THIS page's
//! record's seq, under another root that this page's record BEATS in the Register's tie-break, is never adopted --
//! the register will hold mine once my UPDATE merges (the architect's attack on sdk#225, case 1).

use engine::{ClientId, Op as WriteOp, Params, WriteId};
use page::{Answer, HeadRead, Label, Ms, Op, Page, PutPath};

const KEY: [u8; 32] = [7u8; 32];

/// A page with one write, driven until its UPDATE is out and NOT answered: the page, the time, and the record the
/// signer returned. Every PUT is answered ok and the first head read finds no head.
fn at_update() -> (Page, u64, Vec<u8>) {
    let mut p = Page::new(Params::default(), PutPath::Page);
    p.write(ClientId(1), WriteId(1), vec![(b"k".to_vec(), WriteOp::Put(b"v".to_vec()))]);
    let now = 10;
    let sk = ed25519_dalek::SigningKey::from_bytes(&KEY);
    let params = wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME);
    for _ in 0..20 {
        for op in p.take_ops() {
            match op {
                Op::ReadHead { label: Label::Head } => p.answer(Answer::Head { label: Label::Head, read: None }, Ms(now)),
                Op::Put { id, .. } => p.answer(Answer::PutOk(id), Ms(now)),
                Op::AskHeld { batch, ids } => p.answer(Answer::Held { batch, present: vec![true; ids.len()] }, Ms(now)),
                Op::Sign { id, seq, root, ledger, .. } => {
                    let value = [root.as_slice(), &ledger].concat();
                    let record = contract_keys::register::head_state(&params, &sk.to_bytes(), seq, &value).expect("signs");
                    p.answer(Answer::Signer { id, answer: signer_proto::Answer::Signed(record.clone()) }, Ms(now));
                    let ops = p.take_ops();
                    assert!(ops.iter().any(|o| matches!(o, Op::Update { label: Label::Head, state } if *state == record)), "THE SETUP: the UPDATE did not go out: {ops:?}");
                    for op in ops {
                        if let Op::Put { id, .. } = op {
                            p.answer(Answer::PutOk(id), Ms(now));
                        }
                    }
                    return (p, now, record);
                }
                other => panic!("unexpected before the sign: {other:?}"),
            }
        }
    }
    panic!("the page never asked the signer");
}

/// A head at `seq` under another root whose whole value LOSES the Register's tie-break to `mine`.
fn a_losing_head(mine: &HeadRead) -> HeadRead {
    (1u8..=255)
        .map(|b| HeadRead::from_value(mine.seq, &[b; 32]).expect("a head"))
        .find(|h| h.root() != mine.root() && page::beats(mine.value(), h.value()))
        .expect("some root loses to mine")
}

/// **THE ENGINE'S MID-COMMIT RE-READ IS JUDGED LIKE EVERY OTHER REGISTER ANSWER.** This page's UPDATE is out and not
/// answered; its blocks are acked, so the engine's settle asks the head again (`ReadHead` mid-commit). The register
/// answers the SAME seq under another root that this page's record beats: my UPDATE has not merged there yet. The
/// page must not adopt it (the commit is not lost to a head about to lose) -- and once the register holds mine, the
/// write is Published on it. Red on main (gap (a)): the recovery read stepped `HeadRead` with no tie-break check,
/// and the engine took the loser as a conflict.
#[test]
fn a_mid_commit_recovery_read_never_adopts_a_head_this_pages_record_beats() {
    let (mut p, mut now, record) = at_update();
    let mine = HeadRead::from_record(&record).expect("my record reads as a head");
    // The engine's settle, until it asks the register again (no UPDATE answer, so no read-back is owed on the wire).
    let mut asked = false;
    for _ in 0..400 {
        now += 250;
        p.tick(Ms(now));
        let ops = p.take_ops();
        if ops.iter().any(|o| matches!(o, Op::ReadHead { label: Label::Head })) {
            asked = true;
            break;
        }
    }
    assert!(asked, "THE SETUP: the engine never re-read the head mid-commit");
    let theirs = a_losing_head(&mine);
    p.answer(Answer::Head { label: Label::Head, read: Some(theirs.clone()) }, Ms(now));
    assert_ne!(p.published(), (theirs.seq, theirs.root()), "adopted a head that LOSES the tie-break to this page's own record at that seq");
    // The register then holds mine (my UPDATE merged): answered, and read back, it is Published on it.
    p.answer(Answer::Updated { label: Label::Head }, Ms(now + 1));
    for _ in 0..40 {
        now += 250;
        p.tick(Ms(now));
        for op in p.take_ops() {
            match op {
                Op::ReadHead { label: Label::Head } => p.answer(Answer::Head { label: Label::Head, read: Some(mine.clone()) }, Ms(now)),
                Op::Update { label: Label::Head, .. } => p.answer(Answer::Updated { label: Label::Head }, Ms(now)),
                Op::Put { id, .. } => p.answer(Answer::PutOk(id), Ms(now)),
                Op::AskHeld { batch, ids } => p.answer(Answer::Held { batch, present: vec![true; ids.len()] }, Ms(now)),
                other => common::unanswered_op(&other),
            }
        }
        if p.published() == (mine.seq, mine.root()) {
            break;
        }
    }
    assert_eq!(p.published(), (mine.seq, mine.root()), "the write was not Published on this page's own head once the register held it");
}

mod common;
use common::{items_after_the_cut, production_of};

const LIB: &str = include_str!("../src/lib.rs");
const JUDGE: &str = include_str!("../src/judge.rs");
const OTHERS: [(&str, &str); 3] = [("fates.rs", include_str!("../src/fates.rs")), ("rto.rs", include_str!("../src/rto.rs")), ("server.rs", include_str!("../src/server.rs"))];

/// The body of `fn <name>(` in `src`, by brace matching from the signature: `None` if there is no such fn.
fn body_of<'a>(src: &'a str, name: &str) -> Option<&'a str> {
    let start = src.find(&format!("fn {name}("))?;
    let open = start + src[start..].find('{')?;
    let mut depth = 0usize;
    for (i, c) in src[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&src[start..open + i + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Lines of `src` that BUILD `Event::<variant> {` (a pattern -- `if let`, `matches!`, a match arm -- builds nothing).
fn builds(src: &str, variant: &str) -> usize {
    let lit = format!("Event::{variant} {{");
    src.lines().filter(|l| l.contains(&lit) && !l.contains("if let ") && !l.contains("matches!(") && !l.trim_start().starts_with("//")).count()
}

/// **THE STRUCTURAL CONTROL (sdk#396): no answer path judges a head any other way.** A register's head is adopted
/// (`Event::HeadRead`, `Event::HeadConflict`) ONLY in `Page::adopt`, which takes a `Heard` that only the one
/// judgement makes; the Register's tie-break (`beats`) is CALLED only in `judge.rs`; the old per-site check is gone.
/// Each assertion has its control: the reader finds the real bodies, or the counts could be zero over nothing.
#[test]
fn every_register_answer_is_judged_in_one_place_and_adopted_through_one_door() {
    let lib = production_of(LIB);
    assert!(lib.contains("fn adopt(") && lib.contains("fn step(") && lib.len() * 2 > LIB.len(), "THE CONTROL: the production cut of lib.rs lost the code it judges ({} of {} bytes)", lib.len(), LIB.len());
    // Nothing of production hides AFTER the cut (the architect): past it, only `#[cfg(test)]` modules.
    for (name, src) in [("lib.rs", LIB)].into_iter().chain(OTHERS) {
        let stray = items_after_the_cut(src);
        assert!(stray.is_empty(), "page/src/{name}: production items after the production cut (the first top-level test module), where the one-door control does not look: {stray:?}");
    }
    assert!(LIB.lines().skip(production_of(LIB).lines().count()).filter(|l| l.starts_with("mod ")).count() >= 3, "THE CONTROL: the scan after the cut found no test modules in lib.rs (it read nothing)");
    let adopt = body_of(lib, "adopt").expect("THE CONTROL: the reader found no `fn adopt` in page/src/lib.rs");
    for v in ["HeadRead", "HeadConflict"] {
        assert_eq!(builds(adopt, v), 1, "THE CONTROL: `fn adopt` does not build Event::{v} exactly once (the reader read no real body)");
        assert_eq!(builds(lib, v), 1, "Event::{v} is built outside `fn adopt`: a head can be adopted without the one judgement");
        for (name, src) in OTHERS {
            assert_eq!(builds(production_of(src), v), 0, "page/src/{name} builds Event::{v}: a head adopted outside `fn adopt`");
        }
    }
    // The `Heard` a head is adopted from: built only by the judgement.
    assert!(body_of(JUDGE, "judge").is_some_and(|b| b.contains("Heard {")), "THE CONTROL: `judge` does not build a Heard");
    assert_eq!(JUDGE.matches("Heard {").count(), 3, "a Heard is built somewhere in judge.rs other than `judge` (struct, impl, the one literal)");
    assert_eq!(lib.matches("Heard {").count(), 0, "page/src/lib.rs builds a Heard: only the judgement may");
    // The tie-break: called only in judge.rs (its definition stays in lib.rs, pinned against the Register).
    let calls = |src: &str| src.matches("beats(").count() - src.matches("fn beats(").count();
    assert!(calls(JUDGE) >= 2, "THE CONTROL: judge.rs calls the tie-break {} time(s); it is where the head's and the site's are judged", calls(JUDGE));
    assert_eq!(calls(lib), 0, "page/src/lib.rs calls the tie-break itself: a second judgement");
    for (name, src) in OTHERS {
        assert_eq!(calls(production_of(src)), 0, "page/src/{name} calls the tie-break itself: a second judgement");
    }
    assert!(!lib.contains("fn my_winning_record"), "the per-site tie-break check is back in lib.rs");
}
