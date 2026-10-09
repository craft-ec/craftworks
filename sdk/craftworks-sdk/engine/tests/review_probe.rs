use engine::*;

mod common;
use common::Store;
fn put(k: String, v: Vec<u8>) -> (Vec<u8>, Op) {
    (k.into_bytes(), Op::Put(v))
}

#[test]
fn a_failed_pack_put_is_re_emitted() {
    let mut e = Engine::new(
        Params {
            // This test is ABOUT the pack path, which Phase 3 leaves off the
            // write path and Phase 4 (#39) turns back on. The format and its
            // handling stay tested either way.
            pack_on_write: true,
            ..Params::default()
        },
        Store::default(),
    );
    // A NEW tree, and the engine is told so: a write before the head is
    // recovered waits for it (sdk#223). These probes are about the PACK path.
    let _ = e.step(Event::HeadMissing);
    let fx = stepped!(
        e,
        Event::forced_write(ClientId(1), WriteId(1), vec![put("a".into(), b"x".to_vec())])
    );
    let pack = fx
        .iter()
        .find_map(|f| {
            if let Effect::PutPack { id, .. } = f {
                Some(*id)
            } else {
                None
            }
        })
        .expect("a pack");
    let retry = stepped!(e, Event::PutFailed(pack));
    println!("PROBE1 effects after PutFailed(pack): {}", retry.len());
    assert!(
        !retry.is_empty(),
        "a failed pack put re-emits nothing: the commit stalls for ever"
    );
}

#[test]
fn parity_complete_fires_once_per_write() {
    let mut e = Engine::new(
        Params {
            // This test is ABOUT the pack path, which Phase 3 leaves off the
            // write path and Phase 4 (#39) turns back on. The format and its
            // handling stay tested either way.
            pack_on_write: true,
            ..Params::default()
        },
        Store::default(),
    );
    // A NEW tree, and the engine is told so: a write before the head is
    // recovered waits for it (sdk#223). These probes are about the PACK path.
    let _ = e.step(Event::HeadMissing);
    let ops: Vec<_> = (0..4000)
        .map(|i| put(format!("key-{i:06}"), vec![7u8; 40]))
        .collect();
    let mut q = stepped!(
        e,
        Event::forced_write(ClientId(1), WriteId(1), ops)
    );
    let mut pc = 0;
    let mut guard = 0;
    let mut t = 1u64;
    while guard < 10_000 {
        guard += 1;
        let Some(f) = q.pop() else {
            t += 1000;
            let more = stepped!(e, Event::Tick(t));
            if more.is_empty() && t > 50_000 {
                break;
            }
            q.extend(more);
            continue;
        };
        match f {
            Effect::PutPack { id, .. }
            | Effect::PutBlock { id, .. }
 => q.extend(stepped!(e, Event::PutConfirmed(id))),
            Effect::UpdateHead { seq, .. } => q.extend(stepped!(e, Event::HeadConfirmed(seq))),
            Effect::Notify {
                state: State::ParityComplete,
                ..
            } => pc += 1,
            other => common::no_answer_owed(&other),
        }
    }
    println!("PROBE2 ParityComplete notifications for ONE write: {pc}");
    assert_eq!(pc, 1);
}
