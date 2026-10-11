//! A ROUND OF READS across calls (the witness and keeper rounds': each step a message out and an answer in): one
//! member's round, what it read so far (a contract's state; empty: read, not there), what it waits on, the wake-up it
//! started at — kept in the secrets under the round's name. One at a time per round; one stuck for STUCK wake-ups is
//! dropped (the next starts afresh).

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

pub fn wakeups<H: Host>(h: &H) -> u64 {
    h.get_secret(identity::UPKEEP_WAKEUPS).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes).unwrap_or(0)
}
pub fn load<H: Host>(h: &H, name: &[u8]) -> Option<Round> {
    h.get_secret(name).filter(|b| !b.is_empty()).and_then(|b| bincode::deserialize(&b).ok())
}
pub fn save<H: Host>(h: &mut H, name: &[u8], r: Option<&Round>) {
    h.set_secret(name, &r.map(|r| bincode::serialize(r).expect("a round encodes")).unwrap_or_default());
}
/// Whether a round is IN FLIGHT (and not stuck): no new one starts.
pub fn busy<H: Host>(h: &H, name: &[u8]) -> bool {
    load(h, name).is_some_and(|r| wakeups(h).saturating_sub(r.started) < STUCK)
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
pub fn take<H: Host>(h: &mut H, name: &[u8], id: &[u8; 32], state: Option<Vec<u8>>) -> Option<Round> {
    let mut r = load(h, name)?;
    if !r.waiting.contains(id) {
        return None;
    }
    r.waiting.retain(|w| w != id);
    r.got.insert(*id, state.unwrap_or_default());
    if !r.waiting.is_empty() {
        save(h, name, Some(&r));
        return None;
    }
    Some(r)
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
