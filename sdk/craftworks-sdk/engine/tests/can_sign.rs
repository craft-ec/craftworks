//! THE ENGINE DOES NOT CUT A COMMIT WHILE IT CANNOT SIGN (`Event::CanSign`,
//! stepped by the page from its one signer decision). While it cannot: writes
//! queue and apply to the warm root, so reads see them, and nothing goes to
//! the node -- no block, no pack, no parity, no head. When it can: ONE commit
//! carries the whole queue, in arrival order.

use engine::read::{ReadResult, ReqId};
use engine::{ClientId, Effect, Event, Op, Params, State, WriteId};

mod common;

fn put(k: &str, v: &str) -> (Vec<u8>, Op) {
    (k.as_bytes().to_vec(), Op::Put(v.as_bytes().to_vec()))
}

fn to_node(fx: &[Effect]) -> usize {
    fx.iter()
        .filter(|f| matches!(f, Effect::PutBlock { .. } | Effect::PutPack { .. } | Effect::UpdateHead { .. }))
        .count()
}

fn published(fx: &[Effect]) -> Vec<u64> {
    fx.iter()
        .filter_map(|f| match f {
            Effect::Notify { write_id, state: State::Published, .. } => Some(write_id.0),
            _ => None,
        })
        .collect()
}

#[test]
fn nothing_is_cut_while_it_cannot_sign_and_one_commit_carries_the_queue_when_it_can() {
    let mut e = common::new_store_params(Params::default());
    let _ = e.step(Event::HeadMissing);
    let _ = e.step(Event::CanSign(false));

    let mut held = Vec::new();
    for n in 1..=3u64 {
        let fx = e.step(Event::forced_write(ClientId(1), WriteId(n), vec![put(&format!("k/{n}"), &format!("v{n}"))]));
        e.blocks().absorb(&fx);
        held.extend(fx);
    }
    assert_eq!(to_node(&held), 0, "a commit was cut while the page cannot sign: {} effect(s) for the node", to_node(&held));

    // The writes are in the WARM root: a read of it (the page's own walk,
    // `ScanAt` at the warm root) sees all three, while the published tree is
    // still empty.
    assert_ne!(e.root(), e.published_root(), "the writes did not apply to the warm root");
    let range = freenet_prolly::range::Range {
        lo: std::ops::Bound::Unbounded,
        hi: std::ops::Bound::Unbounded,
        reverse: false,
        after: None,
        max_entries: 100,
        max_bytes: 1 << 20,
    };
    let fx = e.step(Event::ScanAt { client: ClientId(1), req_id: ReqId(9), root: e.root(), range: Box::new(range) });
    let entries = fx.iter().find_map(|f| match f {
        Effect::Reply { req_id, result: ReadResult::Page { entries, .. }, .. } if *req_id == ReqId(9) => Some(entries.clone()),
        _ => None,
    });
    let want: Vec<(Vec<u8>, Vec<u8>)> = (1..=3u64).map(|n| (format!("k/{n}").into_bytes(), format!("v{n}").into_bytes())).collect();
    assert_eq!(entries, Some(want), "the held writes are not visible in the warm root");

    // It can sign: ONE commit, carrying all three.
    let mut all = e.step(Event::CanSign(true));
    e.blocks().absorb(&all);
    assert!(to_node(&all) > 0, "CanSign(true) cut nothing");
    let mut heads = Vec::new();
    let mut queue = all.clone();
    let mut guard = 0;
    while let Some(f) = queue.pop() {
        guard += 1;
        assert!(guard < 10_000, "the commit did not settle");
        let more = match f {
            Effect::PutBlock { id, .. } | Effect::PutPack { id, .. } => e.step(Event::PutConfirmed(id)),
            Effect::UpdateHead { seq, .. } => {
                heads.push(seq);
                e.step(Event::HeadConfirmed(seq))
            }
            _ => continue,
        };
        e.blocks().absorb(&more);
        all.extend(more.clone());
        queue.extend(more);
    }
    assert_eq!(heads, vec![1], "not exactly ONE commit: heads {heads:?}");
    assert_eq!(published(&all), vec![1, 2, 3], "the queue did not publish in arrival order, in one commit");
}

/// The control: with no `CanSign` ever stepped the engine can sign, and a
/// write is cut at once (every existing path is unchanged).
#[test]
fn with_no_can_sign_stepped_a_write_is_cut_at_once() {
    let mut e = common::new_store_params(Params::default());
    let _ = e.step(Event::HeadMissing);
    let fx = e.step(Event::forced_write(ClientId(1), WriteId(1), vec![put("k/1", "v1")]));
    assert!(to_node(&fx) > 0, "the default does not cut: every existing path would change");
}

/// The SAME gate on `Flush` (the client going away): while the page cannot
/// sign, a flush cuts nothing either.
#[test]
fn a_flush_cuts_nothing_while_it_cannot_sign() {
    let mut e = common::new_store_params(Params::default());
    let _ = e.step(Event::HeadMissing);
    let _ = e.step(Event::CanSign(false));
    let mut fx = e.step(Event::forced_write(ClientId(1), WriteId(1), vec![put("k/1", "v1")]));
    fx.extend(e.step(Event::Flush));
    assert_eq!(to_node(&fx), 0, "a Flush cut a commit while the page cannot sign");
}
