//! A ROUND OF READS across calls (the witness and keeper rounds': each step a message out and an answer in): one
//! member's round, what it read so far (a contract's state; empty: read, not there), what it waits on, the wake-up it
//! started at — kept in the secrets under the round's name. One at a time per round; one stuck for STUCK wake-ups is
//! dropped (the next starts afresh).
//!
//! KEPT CHEAPLY (audit 10-11: every answer re-serialized the whole round — every state it had read — into one secret,
//! quadratic in its reads): the round's own secret holds only its INDEX (the member, the ids read, those waited on, its
//! start); each state read is its own secret (`<name>/got/<id>`), written once, as it comes in. The states are read
//! back only when the round is driven on; a round ended or replaced blanks the states it no longer names.

use crate::upkeep::Io;
use craftworks_identity::{self as identity, Host};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Round {
    pub member: [u8; 32],
    /// Each contract read this round: its state (empty: read, not there).
    pub got: BTreeMap<[u8; 32], Vec<u8>>,
    /// Asked and not answered yet.
    pub waiting: Vec<[u8; 32]>,
    pub started: u64,
}
/// Wake-ups after which a round still waiting is dropped.
const STUCK: u64 = 5;

/// A round as its secret holds it: the ids read, not their states.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Index {
    member: [u8; 32],
    read: Vec<[u8; 32]>,
    waiting: Vec<[u8; 32]>,
    started: u64,
}
/// (The index's bytes are tagged: an older round's encoding never reads as one.)
const TAG: &[u8] = b"R2";

fn got_key(name: &[u8], id: &[u8; 32]) -> Vec<u8> {
    [name, b"/got/", &id[..]].concat()
}
fn index<H: Host>(h: &H, name: &[u8]) -> Option<Index> {
    h.get_secret(name).and_then(|b| b.strip_prefix(TAG).and_then(|b| bincode::deserialize(b).ok()))
}
fn put_index<H: Host>(h: &mut H, name: &[u8], i: &Index) {
    h.set_secret(name, &[TAG, &bincode::serialize(i).expect("a round encodes")[..]].concat());
}

pub fn wakeups<H: Host>(h: &H) -> u64 {
    h.get_secret(identity::UPKEEP_WAKEUPS).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes).unwrap_or(0)
}
pub fn load<H: Host>(h: &H, name: &[u8]) -> Option<Round> {
    let i = index(h, name)?;
    let got = i.read.iter().map(|id| (*id, h.get_secret(&got_key(name, id)).unwrap_or_default())).collect();
    Some(Round { member: i.member, got, waiting: i.waiting, started: i.started })
}
/// Keep round `r` (`None`: no round) — each state it read written once; those a round stored before it no longer
/// names (it ended, or was dropped for a new one) blanked.
pub fn save<H: Host>(h: &mut H, name: &[u8], r: Option<&Round>) {
    let was = index(h, name).map(|w| w.read).unwrap_or_default();
    let read: Vec<[u8; 32]> = r.map_or(vec![], |r| r.got.keys().copied().collect());
    for id in was.iter().filter(|id| !read.contains(id)) {
        h.set_secret(&got_key(name, id), &[]);
    }
    let Some(r) = r else {
        h.set_secret(name, &[]);
        return;
    };
    for (id, state) in r.got.iter().filter(|(id, _)| !was.contains(id)) {
        h.set_secret(&got_key(name, id), state);
    }
    put_index(h, name, &Index { member: r.member, read, waiting: r.waiting.clone(), started: r.started });
}
/// Whether a round is IN FLIGHT (and not stuck): no new one starts.
pub fn busy<H: Host>(h: &H, name: &[u8]) -> bool {
    index(h, name).is_some_and(|r| wakeups(h).saturating_sub(r.started) < STUCK)
}
/// The NEXT MEMBER in turn (after the last one this round served, `last`) among `all`: remembered.
pub fn next<H: Host>(h: &mut H, last: &[u8], all: &[[u8; 32]]) -> Option<[u8; 32]> {
    let was = h.get_secret(last).and_then(|b| <[u8; 32]>::try_from(b).ok());
    let after = was.and_then(|l| all.iter().position(|m| *m == l)).map_or(0, |i| i + 1);
    let m = *all.get(after % all.len().max(1))?;
    h.set_secret(last, &m);
    Some(m)
}
/// An ANSWER taken in: the round when it waits on nothing more (driven on), `None` otherwise (kept, or not its own).
/// Only the index is read and rewritten, and the one state written.
pub fn take<H: Host>(h: &mut H, name: &[u8], id: &[u8; 32], state: Option<Vec<u8>>) -> Option<Round> {
    let mut i = index(h, name)?;
    if !i.waiting.contains(id) {
        return None;
    }
    i.waiting.retain(|w| w != id);
    h.set_secret(&got_key(name, id), &state.unwrap_or_default());
    if !i.read.contains(id) {
        i.read.push(*id);
    }
    put_index(h, name, &i);
    if !i.waiting.is_empty() {
        return None;
    }
    load(h, name)
}
/// What a step said: its messages sent; the round ended (nothing to read) or kept waiting on the reads asked.
pub fn go<H: Host>(h: &mut H, name: &[u8], mut r: Round, out: Vec<Io>, want: Vec<[u8; 32]>) -> Vec<Io> {
    let ask: Vec<[u8; 32]> = want.into_iter().filter(|w| !r.got.contains_key(w) && !r.waiting.contains(w)).collect();
    if ask.is_empty() {
        save(h, name, None);
        return out;
    }
    r.waiting.extend(ask.iter().copied());
    save(h, name, Some(&r));
    out.into_iter().chain(ask.into_iter().map(|id| Io::Get { id, subscribe: false })).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testhost::Map;

    /// An answer writes ITS state and the small index — never the states read before it; the round driven on reads them
    /// all back; a round that ends blanks them.
    #[test]
    fn a_round_writes_each_state_once_and_ends_clean() {
        const R: &[u8] = b"test/round";
        let mut h = Map::default();
        let (a, b) = ([1u8; 32], [2u8; 32]);
        let asked = go(&mut h, R, Round { member: [9; 32], started: 0, ..Round::default() }, vec![], vec![a, b]);
        assert_eq!(asked.len(), 2);
        assert!(busy(&h, R));
        assert_eq!(take(&mut h, R, &a, Some(vec![7; 100_000])), None, "still waiting on b");
        assert!(h.0[R].len() < 200, "the index holds no state: {} bytes", h.0[R].len());
        assert_eq!(take(&mut h, R, &[3; 32], Some(vec![1])), None, "not asked: not taken");
        let r = take(&mut h, R, &b, None).expect("all in");
        assert_eq!((r.got[&a].len(), r.got[&b].len()), (100_000, 0), "b read, not there");
        assert!(h.0[R].len() < 200);
        assert_eq!(go(&mut h, R, r, vec![], vec![a]), vec![], "nothing new to read: the round ends");
        assert!(!busy(&h, R) && load(&h, R).is_none());
        assert!(h.0.iter().filter(|(k, _)| k.starts_with(R)).all(|(_, v)| v.is_empty()), "an ended round's states blanked");
    }
}
