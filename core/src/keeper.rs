//! THE KEEPER LOOP in the page's core (TOKEN §7a: keepers are paid on proven keeping) — a node's STANDING RECORD kept in
//! its standing contract (one per peer): each standing epoch its BINDING (its member key runs the peer) and its
//! HOLDINGS (what it hosts, committed); at each height sampled for it, its ANSWER to the beacon's challenge (the asked
//! pieces' sealed hashes — sealed in the page's worker). Every record is signed by the node's member key through its
//! identity delegate (`TokenSign`, the identity's STANDING_DOMAIN): a step answers what to SIGN, or the PUT. Pure, over
//! `got` (a contract's state as read; `Some(empty)`: read and not there).

use crate::tokens::{there, Got, Step};
use craftworks_standing::{codec, record, Answer, Bind, Holdings};
use craftworks_standing_contract::Params;
use craftworks_token_contract as t;

/// Where a node's record is: the standing contract's code hash, the beacon (its code hash and id).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keep {
    pub standing: [u8; 32],
    pub beacon_code: [u8; 32],
    pub beacon: [u8; 32],
}

/// The record's PUT: its params and state.
#[derive(Debug, Clone, PartialEq)]
pub struct Put {
    pub params: Vec<u8>,
    pub state: Vec<u8>,
}

/// A node's holdings cover the whole ring (LOCATION is a soft rule: the arc applies when peers place it).
const WHOLE: (u64, u64) = (0, u64::MAX);

fn params(k: &Keep, peer: &str) -> Params {
    Params { beacon_code: k.beacon_code, beacon: k.beacon, peer: peer.into() }
}

/// The node's record as read (empty when not there).
fn record_of(k: &Keep, got: Got, peer: &str) -> Result<Result<record::State, [u8; 32]>, String> {
    let id = t::id(&k.standing, &params(k, peer).encode());
    Ok(match there(got, &id) {
        Err(w) => Err(w),
        Ok(None) => Ok(record::State::default()),
        Ok(Some(b)) => Ok(codec::decode(&b).ok_or("a standing record that does not read")?),
    })
}

fn signed(sigs: &[(Vec<u8>, [u8; 64])], m: &[u8]) -> Option<[u8; 64]> {
    sigs.iter().find(|(x, _)| x == m).map(|x| x.1)
}

fn put(k: &Keep, peer: &str, held: &record::State, e: record::Epoch) -> Put {
    let merged = record::merge(held, &record::State { epochs: vec![e] });
    Put { params: params(k, peer).encode(), state: codec::encode(&merged) }
}

/// OPEN standing epoch `epoch` in the node's record: its binding and its holdings (`hosted`: what its node hosts, id and
/// size), both signed by `public` (the member key; `sigs`: the delegate's answers so far). `None`: already open.
pub fn open_epoch(k: &Keep, got: Got, public: &[u8; 32], peer: &str, epoch: u64, hosted: &[([u8; 32], u64)], sigs: &[(Vec<u8>, [u8; 64])]) -> Result<Step<Option<Put>>, String> {
    let held = match record_of(k, got, peer)? {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(s) => s,
    };
    if held.epochs.iter().any(|e| e.epoch == epoch && e.holdings.iter().any(|h| h.key == *public)) {
        return Ok(Step::Done(None));
    }
    let mut b = Bind::draft(*public, peer, epoch);
    let Some(sb) = signed(sigs, &b.signed()) else { return Ok(Step::Sign(b.signed())) };
    b.sig = sb;
    let (mut h, _) = Holdings::draft(*public, peer, epoch, hosted, WHOLE);
    let Some(sh) = signed(sigs, &h.signed()) else { return Ok(Step::Sign(h.signed())) };
    h.sig = sh;
    Ok(Step::Done(Some(put(k, peer, &held, record::Epoch { epoch, binds: vec![b], holdings: vec![h], ..Default::default() }))))
}

/// The holdings `public` committed for `epoch` in the record, and their ids (from `hosted`, recomputed: they must
/// commit to the same root). `None`: not open, or `hosted` moved since.
fn holdings(held: &record::State, public: &[u8; 32], peer: &str, epoch: u64, hosted: &[([u8; 32], u64)]) -> Option<(Holdings, Vec<[u8; 32]>)> {
    let h = held.epochs.iter().find(|e| e.epoch == epoch)?.holdings.iter().find(|h| h.key == *public)?.clone();
    let (again, ids) = Holdings::draft(*public, peer, epoch, hosted, WHOLE);
    (again.root == h.root).then_some((h, ids))
}

/// The PIECES height `height` (its beacon output `out`) asks this node for: `None` when not sampled for it, its epoch
/// not open, or answered already.
pub fn asked(k: &Keep, got: Got, public: &[u8; 32], peer: &str, height: u64, out: &[u8; 32], hosted: &[([u8; 32], u64)]) -> Result<Step<Option<Vec<[u8; 32]>>>, String> {
    if !craftworks_standing::sampled(out, peer) {
        return Ok(Step::Done(None));
    }
    let held = match record_of(k, got, peer)? {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(s) => s,
    };
    let epoch = craftworks_standing::epoch(height);
    if held.epochs.iter().any(|e| e.epoch == epoch && e.answers.iter().any(|a| a.height == height && a.key == *public)) {
        return Ok(Step::Done(None));
    }
    Ok(Step::Done(holdings(&held, public, peer, epoch, hosted).map(|(h, ids)| Answer::asked(&h, &ids, out))))
}

/// The ANSWER at `height` from the asked pieces' sealed hashes (`hashes`: id → hash), signed: the record to put
/// (`None`: nothing to answer — see [`asked`]).
#[allow(clippy::too_many_arguments)]
pub fn answer(k: &Keep, got: Got, public: &[u8; 32], peer: &str, height: u64, out: &[u8; 32], hosted: &[([u8; 32], u64)], hashes: &[([u8; 32], [u8; 32])], sigs: &[(Vec<u8>, [u8; 64])]) -> Result<Step<Option<Put>>, String> {
    let held = match record_of(k, got, peer)? {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(s) => s,
    };
    let epoch = craftworks_standing::epoch(height);
    let Some((h, ids)) = holdings(&held, public, peer, epoch, hosted) else { return Ok(Step::Done(None)) };
    let Some(mut a) = Answer::draft_hashed(&h, &ids, height, out, &|id| hashes.iter().find(|x| x.0 == *id).map(|x| x.1)) else { return Ok(Step::Done(None)) };
    let Some(s) = signed(sigs, &a.message()) else { return Ok(Step::Sign(a.message())) };
    a.sig = s;
    Ok(Step::Done(Some(put(k, peer, &held, record::Epoch { epoch, answers: vec![a], ..Default::default() }))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};
    use std::collections::HashMap;

    /// The delegate's loop over a step: sign what it asks with `key` (the member key).
    fn run<T>(key: &SigningKey, step: &mut dyn FnMut(&[(Vec<u8>, [u8; 64])]) -> Result<Step<T>, String>) -> T {
        let mut sigs = Vec::new();
        for _ in 0..8 {
            match step(&sigs).unwrap() {
                Step::Done(x) => return x,
                Step::Sign(m) => {
                    assert!(m.starts_with(craftworks_identity::STANDING_DOMAIN), "under the standing domain");
                    sigs.push((m.clone(), key.sign(&m).to_bytes()));
                }
                Step::Want(_) => panic!("a want"),
            }
        }
        panic!("no end")
    }

    #[test]
    fn a_node_opens_its_epoch_and_answers_a_sampled_height_in_a_record_that_holds() {
        let k = Keep { standing: [31; 32], beacon_code: [32; 32], beacon: [33; 32] };
        let key = SigningKey::from_bytes(&[44; 32]);
        let public = key.verifying_key().to_bytes();
        let peer = "v6MkPeerTestFx01";
        let piece = |id: &[u8; 32]| [id.to_vec(), vec![7u8; 100]].concat();
        let hosted: Vec<([u8; 32], u64)> = (0..40u32).map(|i| (*blake3::hash(&i.to_le_bytes()).as_bytes(), 132)).collect();
        let rid = t::id(&k.standing, &params(&k, peer).encode());
        let mut got: HashMap<[u8; 32], Vec<u8>> = HashMap::new();
        got.insert(rid, vec![]);
        // OPEN epoch 3: binding and holdings, both signed under the standing domain.
        let g = got.clone();
        let p = run(&key, &mut |s| open_epoch(&k, &|x| g.get(x).cloned(), &public, peer, 3, &hosted, s)).expect("opened");
        got.insert(rid, p.state.clone());
        let g = got.clone();
        assert_eq!(run(&key, &mut |s| open_epoch(&k, &|x| g.get(x).cloned(), &public, peer, 3, &hosted, s)), None, "opened once");
        // A SAMPLED height of epoch 3: the asked pieces, their sealed hashes, the answer.
        let out = |h: u64| *blake3::hash(&h.to_le_bytes()).as_bytes();
        let h = (3 * craftworks_standing::EPOCH..4 * craftworks_standing::EPOCH).find(|h| craftworks_standing::sampled(&out(*h), peer)).expect("a sampled height");
        let ask = |got: &HashMap<[u8; 32], Vec<u8>>, h: u64| match asked(&k, &|x| got.get(x).cloned(), &public, peer, h, &out(h), &hosted).unwrap() {
            Step::Done(x) => x,
            _ => panic!("a step"),
        };
        let unsampled = (h + 1..).find(|x| !craftworks_standing::sampled(&out(*x), peer)).unwrap();
        assert_eq!(ask(&got, unsampled), None, "not sampled: nothing asked");
        let pieces = ask(&got, h).expect("asked");
        assert_eq!(pieces.len(), craftworks_standing::PICKS.min(40));
        let hashes: Vec<([u8; 32], [u8; 32])> = pieces.iter().map(|id| (*id, craftworks_standing::sealed_hash(&public, id, &piece(id), &out(h), peer))).collect();
        let g = got.clone();
        let p = run(&key, &mut |s| answer(&k, &|x| g.get(x).cloned(), &public, peer, h, &out(h), &hosted, &hashes, s)).expect("answered");
        got.insert(rid, p.state.clone());
        assert_eq!(ask(&got, h), None, "answered once");
        // The RECORD holds, as the standing contract checks it; and each answer checks against a piece's real seal.
        let rec = codec::decode(&p.state).unwrap();
        assert_eq!(record::holds(peer, &rec, &|x| Some(out(x))), Ok(()));
        let e = rec.epochs.iter().find(|e| e.epoch == 3).unwrap();
        let (hd, a) = (&e.holdings[0], &e.answers[0]);
        assert_eq!(craftworks_standing::check(hd, a, &out(h), None, Some((0, &piece(&a.picks[0].id)))), Ok(()));
        // A wrong hash (a piece not held): its answer does not check.
        let mut bad = hashes.clone();
        bad[0].1 = [0; 32];
        let mut g2 = got.clone();
        g2.insert(rid, codec::encode(&record::State { epochs: rec.epochs.iter().map(|e| record::Epoch { answers: vec![], ..e.clone() }).collect() }));
        let p2 = run(&key, &mut |s| answer(&k, &|x| g2.get(x).cloned(), &public, peer, h, &out(h), &hosted, &bad, s)).unwrap();
        let rec2 = codec::decode(&p2.state).unwrap();
        let a2 = &rec2.epochs.iter().find(|e| e.epoch == 3).unwrap().answers[0];
        let j = a2.picks.iter().position(|p| p.id == bad[0].0).unwrap();
        assert!(craftworks_standing::check(hd, a2, &out(h), None, Some((j, &piece(&bad[0].0)))).is_err());
    }
}
