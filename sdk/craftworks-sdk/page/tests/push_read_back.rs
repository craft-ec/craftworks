//! A SAVE'S READ-BACK, AND THE NODE'S PUSH (sdk#378 P2 + P3, the architect's rulings). Every node op is one place
//! on the node's ONE queue (F61), so a save sends no register read it does not need:
//! - P2: while this page's own read-back is owed (its UPDATE is out), a `HeadChanged` hint starts no second read:
//!   the read-back reads the register and judges whatever it holds.
//! - P3: a pushed FULL state showing EXACTLY this page's owed head is the read-back (judged by the one read-back
//!   rule); anything else is the hint it always was, and a dropped push leaves the read-back GET to its deadline.

use engine::{ClientId, Op as WriteOp, Params, WriteId};
use page::{Answer, HeadRead, Label, Ms, Op, Page, PutPath};

/// A page with one write, driven until its UPDATE is out (not yet answered): the page, the time, and the record
/// the signer returned (the UPDATE's exact bytes). Every PUT is answered ok and the first head read finds no head.
fn at_update() -> (Page, u64, Vec<u8>) {
    let mut p = Page::new(Params::default(), PutPath::Page);
    p.write(ClientId(1), WriteId(1), vec![(b"k".to_vec(), WriteOp::Put(b"v".to_vec()))]);
    let now = 10;
    let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
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
                    return (p, now, record);
                }
                other => panic!("unexpected before the sign: {other:?}"),
            }
        }
    }
    panic!("the page never asked the signer");
}

fn head_reads(ops: &[Op]) -> usize {
    ops.iter().filter(|o| matches!(o, Op::ReadHead { label: Label::Head })).count()
}

/// **P2: no second read while the read-back is owed.** The UPDATE is out; the node's hint (our own UPDATE's push,
/// or anyone's) starts no register read of its own. Mutant "hint reads always" -> red.
#[test]
fn a_hint_starts_no_read_while_the_read_back_is_owed() {
    let (mut p, _, _) = at_update();
    p.head_hint();
    assert_eq!(head_reads(&p.take_ops()), 0, "a hint read the register while this page's own read-back was owed");
}

/// **P3: the pushed full state showing exactly this page's head IS the read-back.** The commit is Published on it,
/// with no register GET, and the UPDATE's later answer asks nothing more. Mutants "a push never confirms" and "the
/// UPDATE's answer still asks the read-back" -> red.
#[test]
fn a_push_of_exactly_this_pages_head_confirms_it_with_no_read() {
    let (mut p, now, record) = at_update();
    let (seq0, _) = p.published();
    let read = HeadRead::from_record(&record).expect("the record reads as a head");
    let want = (read.seq, read.root());
    p.head_pushed(read);
    assert_eq!(p.published(), want, "the pushed head that is exactly this page's was not taken as its read-back");
    assert!(want.0 > seq0, "THE SETUP: the owed head is not newer than the published one");
    assert_eq!(head_reads(&p.take_ops()), 0, "a register read was sent after the push confirmed the head");
    p.answer(Answer::Updated { label: Label::Head }, Ms(now + 1));
    assert_eq!(head_reads(&p.take_ops()), 0, "the UPDATE's answer asked a read-back the push had already done");
}

/// **A push of ANOTHER head confirms nothing** (condition 2: only exactly the owed head, by the read-back rule):
/// the same seq under another root leaves the commit owed, and the read-back GET still follows the UPDATE's
/// answer. Mutant "a push at the owed seq confirms, whatever its root" -> red.
#[test]
fn a_push_of_another_head_confirms_nothing_and_the_read_back_still_reads() {
    let (mut p, now, record) = at_update();
    let before = p.published();
    let mine = HeadRead::from_record(&record).expect("reads");
    let other = HeadRead::from_value(mine.seq, &[7u8; 32]).expect("a head");
    p.head_pushed(other);
    assert_eq!(p.published(), before, "a pushed head that is not this page's was taken as its read-back");
    let _ = p.take_ops();
    p.answer(Answer::Updated { label: Label::Head }, Ms(now + 1));
    assert_eq!(head_reads(&p.take_ops()), 1, "the read-back was not asked after a push that confirmed nothing");
}

/// **Condition 3, THE CONTROL: no push at all** -- the UPDATE's answer asks the read-back GET, as it always did.
#[test]
fn control_a_dropped_push_leaves_the_read_back_get() {
    let (mut p, now, _) = at_update();
    p.answer(Answer::Updated { label: Label::Head }, Ms(now + 1));
    assert_eq!(head_reads(&p.take_ops()), 1, "the read-back was not asked");
}

/// The page's write published by its read-back GET (the GET's answer shows exactly its head): the page at rest,
/// standing on its own head, and that head as read.
fn published_by_the_read_back() -> (Page, u64, HeadRead) {
    let (mut p, now, record) = at_update();
    let mine = HeadRead::from_record(&record).expect("reads");
    p.answer(Answer::Updated { label: Label::Head }, Ms(now + 1));
    assert_eq!(head_reads(&p.take_ops()), 1, "THE SETUP: the read-back GET did not go out");
    p.answer(Answer::Head { label: Label::Head, read: Some(mine.clone()) }, Ms(now + 2));
    assert_eq!(p.published(), (mine.seq, mine.root()), "THE SETUP: the write was not published by its read-back");
    let _ = p.take_ops();
    (p, now + 2, mine)
}

/// **Done x E1 (the architect, #378): a full-state push of the head this page already stands on reads nothing.**
/// The read-back answered first, then the node's push of that same head arrives: it is news to nobody, so no
/// register GET (one op on the node's one queue, F61). Mutant "a known head is a hint like any push" -> 1 read -> red.
#[test]
fn a_push_of_the_head_already_known_reads_nothing() {
    let (mut p, _, mine) = published_by_the_read_back();
    p.head_pushed(mine);
    assert_eq!(head_reads(&p.take_ops()), 0, "a push of the head this page already stands on sent a register read");
}

/// **THE CONTROLS: anything else is still the hint it was, one read each.** A DELTA push (no state: `head_hint`);
/// a NEWER head; and the SAME (seq, root) with another value (another ledger: the Register's tie-break is over
/// values, so it is not the head this page knows). Mutant "known by (seq, root) alone" -> the last is 0 reads -> red.
#[test]
fn control_a_delta_a_newer_head_or_another_value_is_still_read() {
    let (mut p, _, _) = published_by_the_read_back();
    p.head_hint();
    assert_eq!(head_reads(&p.take_ops()), 1, "a delta push (no state) did not read the register");
    let (mut p, _, mine) = published_by_the_read_back();
    p.head_pushed(HeadRead::from_value(mine.seq + 1, &[9u8; 32]).expect("a head"));
    assert_eq!(head_reads(&p.take_ops()), 1, "a push of a NEWER head did not read the register");
    let (mut p, _, mine) = published_by_the_read_back();
    let other_value = [mine.root().as_slice(), b"another ledger"].concat();
    let same_root = HeadRead::from_value(mine.seq, &other_value).expect("a head");
    assert_eq!((same_root.seq, same_root.root()), (mine.seq, mine.root()), "THE SETUP: not the same (seq, root)");
    assert_ne!(same_root.value(), mine.value(), "THE SETUP: not another value");
    p.head_pushed(same_root);
    assert_eq!(head_reads(&p.take_ops()), 1, "a push of the same (seq, root) under ANOTHER value was taken as known");
}
