//! An app's PUT (a published web container) goes through the PAGE's sender:
//! the same RTO deadline and re-send as every op, until the node ANSWERS —
//! acknowledged, or refused in its words — or a person CANCELS it (rules 7,
//! 8). No time ends it: a slow node only means it waits longer, and the page
//! says how long ("not answering for N s").

use engine::Params;
use page::rto::RTO_MAX_MS;
use page::{AppPut, Answer, Ms, Op, Page, PutPath};

const RTO_MAX: u64 = RTO_MAX_MS as u64;
const KEY: &str = "8YwvenTZJgTESoiutjWRR2zYZVg1HkNBYoQ347ST66Du";

/// The app PUTs of `key` the page sent, in order, and when.
fn app_puts(ops: &[Op], key: &str) -> usize {
    ops.iter().filter(|o| matches!(o, Op::PutApp { key: k } if k == key)).count()
}

/// Drive the page's clock from `from` to `to` in 100 ms steps, answering
/// nothing; the app PUTs it sent on the way, with their times.
fn silent(p: &mut Page, from: u64, to: u64) -> Vec<u64> {
    let mut sent = Vec::new();
    let mut now = from;
    while now <= to {
        p.tick(Ms(now));
        for _ in 0..app_puts(&p.take_ops(), KEY) {
            sent.push(now);
        }
        now += 100;
    }
    sent
}

#[test]
fn a_put_nobody_answers_for_five_minutes_is_re_sent_throughout_and_put_when_answered() {
    let mut p = Page::new(Params::default(), PutPath::Page);
    p.tick(Ms(0));
    p.put_app(KEY.into(), Ms(0));
    assert_eq!(app_puts(&p.take_ops(), KEY), 1, "the PUT did not go out at once");
    let resent = silent(&mut p, 100, 300_000);
    let waited = p.not_answering();
    println!("silent 5 min: re-sent {} times, at {:?} ms; not answering {:?}", resent.len(), resent, waited);
    assert_eq!(p.app_put(KEY), Some(&AppPut::Pending), "five minutes of silence ENDED the PUT");
    assert!(resent.len() >= 5, "a silent node's PUT was re-sent {} time(s) in five minutes", resent.len());
    assert!(resent.last().is_some_and(|t| *t >= 240_000), "it stopped being sent: last at {:?}", resent.last());
    assert!(waited.as_ref().is_some_and(|(_, ms)| *ms >= 299_000), "the page did not say how long it waited: {waited:?}");
    // The node answers at last.
    p.answer(Answer::AppPutOk(KEY.into()), Ms(300_050));
    assert_eq!(p.app_put(KEY), Some(&AppPut::Put));
    // What still waits is not the PUT (this bare page's own head read is).
    assert!(p.not_answering().is_none_or(|(what, _)| what != "the app's publication"), "still 'not answering' for the PUT after its answer: {:?}", p.not_answering());
}

#[test]
fn a_person_cancels_a_pending_put_and_nothing_more_is_sent() {
    let mut p = Page::new(Params::default(), PutPath::Page);
    p.tick(Ms(0));
    p.put_app(KEY.into(), Ms(0));
    let _ = p.take_ops();
    assert!(!silent(&mut p, 100, 10_000).is_empty(), "not re-sent before the cancel");
    p.cancel_app_put(KEY);
    assert_eq!(p.app_put(KEY), Some(&AppPut::Cancelled));
    assert!(silent(&mut p, 10_100, 200_000).is_empty(), "a cancelled PUT was still sent");
    // A late answer does not flip it.
    p.answer(Answer::AppPutOk(KEY.into()), Ms(200_100));
    assert_eq!(p.app_put(KEY), Some(&AppPut::Cancelled));
}

#[test]
fn a_put_answered_only_on_its_re_send_succeeds() {
    let mut p = Page::new(Params::default(), PutPath::Page);
    p.tick(Ms(0));
    p.put_app(KEY.into(), Ms(0));
    assert_eq!(app_puts(&p.take_ops(), KEY), 1);
    // The first one is lost: nothing answers it. The page must send it again.
    let resent = silent(&mut p, 100, 30_000);
    assert!(!resent.is_empty(), "the lost PUT was never sent again: nothing could answer it");
    p.answer(Answer::AppPutOk(KEY.into()), Ms(resent[0] + 50));
    assert_eq!(p.app_put(KEY), Some(&AppPut::Put));
    assert!(silent(&mut p, 30_100, 30_100 + 3 * RTO_MAX).is_empty(), "an acknowledged PUT was sent again");
}

#[test]
fn a_refusal_ends_it_in_the_nodes_words_and_late_or_foreign_answers_change_nothing() {
    let mut p = Page::new(Params::default(), PutPath::Page);
    p.tick(Ms(0));
    p.put_app(KEY.into(), Ms(0));
    p.put_app("other".into(), Ms(0));
    let _ = p.take_ops();
    // Somebody else's answer settles nothing of ours.
    p.answer(Answer::AppPutOk("never-put".into()), Ms(10));
    assert_eq!((p.app_put(KEY), p.app_put("never-put")), (Some(&AppPut::Pending), None));
    p.answer(Answer::AppPutRefused { key: KEY.into(), said: "invalid put".into() }, Ms(20));
    assert_eq!(p.app_put(KEY), Some(&AppPut::Refused("invalid put".into())));
    assert_eq!(p.app_put("other"), Some(&AppPut::Pending), "a refusal settled the wrong PUT");
    // A late answer to the ended one does not flip it.
    p.answer(Answer::AppPutOk(KEY.into()), Ms(30));
    assert_eq!(p.app_put(KEY), Some(&AppPut::Refused("invalid put".into())));
    // Asking again after it ENDED starts it over; asking while pending does not
    // send a second copy.
    p.put_app(KEY.into(), Ms(0));
    p.put_app("other".into(), Ms(0));
    let ops = p.take_ops();
    assert_eq!((app_puts(&ops, KEY), app_puts(&ops, "other")), (1, 0));
    assert_eq!(p.app_put(KEY), Some(&AppPut::Pending));
}
