use super::*;
use craftec_register_contract::testing::{keyset, rng, World};
use craftec_register_contract::wire::Authority;
use ed25519_dalek::Signer;
use freenet_prolly::store::MemBlocks;
use std::collections::BTreeMap;

/// What the signer does: sign the message with the first `k` keys.
fn sign(w: &World, u: &Unsigned) -> Signed {
    let (bitmap, sigs) = match &w.auth {
        Authority::One(_) => (0u16, vec![w.signers[0].sign(&u.message).to_bytes()]),
        Authority::Quorum { .. } => (
            (0..w.k).fold(0u16, |m, i| m | (1 << i)),
            (0..w.k)
                .map(|i| w.signers[i].sign(&u.message).to_bytes())
                .collect(),
        ),
    };
    Signed {
        terminal: false,
        seq: u.seq,
        value_hash: u.body.hash(),
        bitmap,
        sigs,
    }
}

/// A host running the contract's own merge on what the writer sends.
struct Host {
    w: Params,
    held: Option<Tail>,
}

impl Host {
    fn take(&mut self, cand: &[u8]) {
        self.held = craftec_tail_contract::absorb(self.held.take(), cand, &self.w);
    }
    fn body(&self) -> Body {
        self.held
            .as_ref()
            .map(|t| t.body.clone())
            .unwrap_or_default()
    }
}

fn write(w: &World, wr: &mut Writer, ops: Vec<Op>) -> Vec<u8> {
    let u = wr.prepare(ops).expect("a legal step");
    let s = sign(w, &u);
    wr.commit(u, s).expect("the writer's own signature")
}

fn set(k: &[u8], v: Vec<u8>) -> Op {
    Op::Set {
        key: k.to_vec(),
        value: v,
    }
}

#[test]
fn a_row_is_read_from_the_tail_then_from_the_tree_after_a_flush() {
    let w = keyset(1, 1, true);
    let mut wr = Writer::new(&w.params_bytes).unwrap();
    let mut host = Host {
        w: w.params.clone(),
        held: None,
    };
    let mut blocks = MemBlocks::default();
    host.take(&write(&w, &mut wr, vec![set(b"a", b"1".to_vec())]));
    assert_eq!(get(&blocks, &host.body(), b"a"), Ok(Some(b"1".to_vec())));
    // Flush: tree blocks first, then the delta naming the root.
    let (_, op) = flush_into(&mut blocks, &wr.body(), wr.seq()).unwrap();
    host.take(&write(&w, &mut wr, vec![op]));
    let b = host.body();
    assert!(b.root.is_some());
    assert!(b.entries.is_empty(), "the row left the tail");
    assert_eq!(
        get(&blocks, &b, b"a"),
        Ok(Some(b"1".to_vec())),
        "and is read from the tree"
    );
}

#[test]
fn a_delete_in_the_tail_hides_the_tree_row_and_reaches_the_tree_on_flush() {
    let w = keyset(1, 1, true);
    let mut wr = Writer::new(&w.params_bytes).unwrap();
    let mut blocks = MemBlocks::default();
    write(&w, &mut wr, vec![set(b"a", b"1".to_vec())]);
    let (_, op) = flush_into(&mut blocks, &wr.body(), wr.seq()).unwrap();
    write(&w, &mut wr, vec![op]);
    write(&w, &mut wr, vec![Op::Delete { key: b"a".to_vec() }]);
    assert_eq!(
        get(&blocks, &wr.body(), b"a"),
        Ok(None),
        "the tombstone wins over the tree"
    );
    let (_, op) = flush_into(&mut blocks, &wr.body(), wr.seq()).unwrap();
    write(&w, &mut wr, vec![op]);
    assert_eq!(
        get(&blocks, &wr.body(), b"a"),
        Ok(None),
        "and the tree no longer has it"
    );
    // Deleting a key the tree never had flushes too.
    write(
        &w,
        &mut wr,
        vec![Op::Delete {
            key: b"zz".to_vec(),
        }],
    );
    let (_, op) = flush_into(&mut blocks, &wr.body(), wr.seq()).unwrap();
    assert!(wr.prepare(vec![op]).is_some());
}

#[test]
fn a_read_names_the_tree_blocks_it_still_needs() {
    let w = keyset(1, 1, true);
    let mut wr = Writer::new(&w.params_bytes).unwrap();
    let mut blocks = MemBlocks::default();
    write(&w, &mut wr, vec![set(b"a", b"1".to_vec())]);
    let (applied, op) = flush_into(&mut blocks, &wr.body(), wr.seq()).unwrap();
    write(&w, &mut wr, vec![op]);
    // A reader that holds no tree blocks is told which to fetch, never a wrong answer.
    let empty = MemBlocks::default();
    assert_eq!(
        get(&empty, &wr.body(), b"a"),
        Err(ReadError::Need(vec![applied.root]))
    );
}

#[test]
fn the_writer_refuses_a_signature_that_does_not_cover_its_step() {
    let w = keyset(1, 1, true);
    let mut wr = Writer::new(&w.params_bytes).unwrap();
    let u = wr.prepare(vec![set(b"a", b"1".to_vec())]).unwrap();
    let other = wr.prepare(vec![set(b"a", b"2".to_vec())]).unwrap();
    assert_eq!(wr.commit(u, sign(&w, &other)), None);
    assert_eq!(wr.seq(), 0, "nothing moved");
}

#[test]
fn a_writer_resumes_from_the_state_a_host_holds() {
    let w = keyset(2, 3, false);
    let mut wr = Writer::new(&w.params_bytes).unwrap();
    let mut host = Host {
        w: w.params.clone(),
        held: None,
    };
    host.take(&write(&w, &mut wr, vec![set(b"a", b"1".to_vec())]));
    host.take(&write(&w, &mut wr, vec![set(b"b", b"2".to_vec())]));
    let state = host.held.as_ref().unwrap().encode(&w.auth);
    let mut again = Writer::resume(&w.params_bytes, &state).unwrap();
    assert_eq!(again.seq(), 2);
    assert_eq!(again.body(), wr.body());
    host.take(&write(&w, &mut again, vec![set(b"c", b"3".to_vec())]));
    assert_eq!(host.held.as_ref().unwrap().seq(), 3);
}

/// The whole cycle, at random: writes, deletes, values inline and in their own blocks, and flushes. After every
/// step a host that only ever saw the deltas answers every key exactly as a plain map would.
#[test]
fn a_host_fed_only_deltas_answers_like_a_map_through_many_flushes() {
    for seed in 1..=8u64 {
        let w = keyset(1, 1, true);
        let mut wr = Writer::new(&w.params_bytes).unwrap();
        let mut host = Host {
            w: w.params.clone(),
            held: None,
        };
        let mut blocks = MemBlocks::default();
        let mut model: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
        let mut r = rng(seed);
        let keys: Vec<Vec<u8>> = (0..40u16)
            .map(|i| format!("k{i:03}").into_bytes())
            .collect();
        let mut flushes = 0;
        for _ in 0..300 {
            let pick = (r() % 10) as u8;
            if pick == 0 && !wr.body().entries.is_empty() {
                let (_, op) = flush_into(&mut blocks, &wr.body(), wr.seq()).unwrap();
                host.take(&write(&w, &mut wr, vec![op]));
                flushes += 1;
            } else {
                let mut ops = Vec::new();
                for _ in 0..(1 + r() % 3) {
                    let k = keys[(r() % keys.len() as u64) as usize].clone();
                    if r().is_multiple_of(4) {
                        model.remove(&k);
                        ops.push(Op::Delete { key: k });
                    } else {
                        // Some values past the tree's inline limit, so they live in blocks of their own.
                        let len = if r().is_multiple_of(5) {
                            1500
                        } else {
                            1 + (r() % 40) as usize
                        };
                        let v: Vec<u8> = (0..len)
                            .map(|i| (r() as u8).wrapping_add(i as u8))
                            .collect();
                        model.insert(k.clone(), v.clone());
                        ops.push(set(&k, v));
                    }
                }
                host.take(&write(&w, &mut wr, ops));
            }
            let body = host.body();
            assert_eq!(
                body,
                wr.body(),
                "the host followed every delta (seed {seed})"
            );
            for k in &keys {
                assert_eq!(
                    get(&blocks, &body, k).unwrap(),
                    model.get(k).cloned(),
                    "key {:?} (seed {seed})",
                    String::from_utf8_lossy(k)
                );
            }
        }
        assert!(
            flushes > 10,
            "the run exercised flushes (seed {seed}: {flushes})"
        );
    }
}
