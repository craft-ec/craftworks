//! THE WITNESS ROUND with no page open (TOKEN §7a): every wake-up, for each member whose home site handed over a WITNESS
//! MANDATE (`identity::WitnessMandate`), this delegate reads the beacon and the witness queue (this standing epoch's and
//! the last), and for each entry runs THE WITNESS RULE (`craftworks_rules::witness::entry`, the page's own) over what it
//! read — asking the network for each contract the rule wants, one round trip at a time. An entry the member's stake is
//! drawn for, and the rule takes, is signed through the SAME guard as a page's (`identity::upkeep_witness_sign`: never
//! another entry at a place) and its signature dropped into the entry's witness bag (a PUT of a bag that exists: the
//! node merges it). Each step is a message out and an answer in, across calls: the round lives in the secrets.

use crate::upkeep::{Code, Io, Reply};
use craftworks_identity::{self as identity, Host, WitnessMandate};
use craftworks_rules::{tokens, witness};
use craftworks_token_contract as t;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The round in flight (one at a time, one member at a time).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Round {
    member: [u8; 32],
    /// Each contract read this round: its state (empty: read, not there).
    got: BTreeMap<[u8; 32], Vec<u8>>,
    /// Asked and not answered yet.
    waiting: Vec<[u8; 32]>,
    /// The wake-up it started at (a round answered nothing for STUCK wake-ups is dropped).
    started: u64,
}

pub const ROUND: &[u8] = b"identity_witness/round";
/// The member the last round was for (rounds take members in turn).
const LAST: &[u8] = b"identity_witness/last";
/// Queue items this member settled (witnessed, or not drawn for): never asked again (the newest kept).
const DONE: &[u8] = b"identity_witness/done/";
const DONE_MAX: usize = 4096;
/// Wake-ups after which a round still waiting is dropped (the next starts afresh).
const STUCK: u64 = 5;

fn wakeups<H: Host>(h: &H) -> u64 {
    h.get_secret(identity::UPKEEP_WAKEUPS).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes).unwrap_or(0)
}
fn round<H: Host>(h: &H) -> Option<Round> {
    h.get_secret(ROUND).filter(|b| !b.is_empty()).and_then(|b| bincode::deserialize(&b).ok())
}
fn keep<H: Host>(h: &mut H, r: Option<&Round>) {
    h.set_secret(ROUND, &r.map(|r| bincode::serialize(r).expect("a round encodes")).unwrap_or_default());
}
fn done<H: Host>(h: &H, member: &[u8; 32]) -> Vec<Vec<u8>> {
    h.get_secret(&[DONE, &member[..]].concat()).and_then(|b| bincode::deserialize(&b).ok()).unwrap_or_default()
}
fn set_done<H: Host>(h: &mut H, member: &[u8; 32], mut all: Vec<Vec<u8>>) {
    if all.len() > DONE_MAX {
        all.drain(..all.len() - DONE_MAX);
    }
    h.set_secret(&[DONE, &member[..]].concat(), &bincode::serialize(&all).expect("done encodes"));
}

/// A WAKE-UP: when no round is in flight (or the one in flight is stuck), a round started for the next member, after
/// the last served, with a mandate and their page away (an open page witnesses for itself).
pub fn woke<H: Host>(h: &mut H) -> Vec<Io> {
    if let Some(r) = round(h) {
        if wakeups(h).saturating_sub(r.started) < STUCK {
            return Vec::new();
        }
    }
    let all: Vec<[u8; 32]> = identity::upkeep_members(h).into_iter().filter(|m| identity::upkeep_witness_mandate(h, m).is_some() && identity::upkeep_since_tick(h, m) >= crate::upkeep::PAGE_AWAY).collect();
    let last = h.get_secret(LAST).and_then(|b| <[u8; 32]>::try_from(b).ok());
    let after = last.and_then(|l| all.iter().position(|m| *m == l)).map_or(0, |i| i + 1);
    let Some(member) = all.get(after % all.len().max(1)).copied() else {
        keep(h, None);
        return Vec::new();
    };
    h.set_secret(LAST, &member);
    let r = Round { member, started: wakeups(h), ..Round::default() };
    drive(h, r)
}

/// An ANSWER from the network: kept if this round asked for it, then the round driven on.
pub fn replied<H: Host>(h: &mut H, reply: &Reply) -> Vec<Io> {
    let Reply::Got { id, state } = reply else { return Vec::new() };
    let Some(mut r) = round(h) else { return Vec::new() };
    if !r.waiting.contains(id) {
        return Vec::new();
    }
    r.waiting.retain(|w| w != id);
    r.got.insert(*id, state.clone().unwrap_or_default());
    if !r.waiting.is_empty() {
        keep(h, Some(&r));
        return Vec::new();
    }
    drive(h, r)
}

/// The round as far as what it read takes it: the next reads asked, or the signatures dropped and the round ended.
fn drive<H: Host>(h: &mut H, mut r: Round) -> Vec<Io> {
    let Some(m) = identity::upkeep_witness_mandate(h, &r.member) else {
        keep(h, None);
        return Vec::new();
    };
    let (out, want) = step(h, &r, &m);
    if want.is_empty() {
        keep(h, None);
        return out;
    }
    let ask: Vec<[u8; 32]> = want.into_iter().filter(|w| !r.got.contains_key(w) && !r.waiting.contains(w)).collect();
    r.waiting.extend(ask.iter().copied());
    keep(h, Some(&r));
    out.into_iter().chain(ask.into_iter().map(|id| Io::Get { id, subscribe: false })).collect()
}

/// One pass over what is read: (what to send now, what to read first). Signatures already made are sent even while
/// more is read for the entries after them.
fn step<H: Host>(h: &mut H, r: &Round, m: &WitnessMandate) -> (Vec<Io>, Vec<[u8; 32]>) {
    let got = |id: &[u8; 32]| r.got.get(id).cloned();
    let ctx = tokens::Ctx { code_hash: m.token, beacon: m.beacon, idlog: m.idlog };
    let lot = tokens::Lot { pool: m.pool, bag: m.bag, beacon_code: m.beacon_code };
    // THE CLOCK.
    let beacon = match tokens::there(&got, &m.beacon) {
        Err(w) => return (vec![], vec![w]),
        Ok(None) => return (vec![], vec![]),
        Ok(Some(b)) => b,
    };
    let Some(height) = craftworks_beacon_contract::wire::decode(&beacon).map(|s| s.height()) else { return (vec![], vec![]) };
    let epoch = height / craftworks_reputation::EPOCH;
    // THE QUEUE: this standing epoch's and the last (chained bags, k = 0 up to the first with room).
    let mut items: Vec<Vec<u8>> = Vec::new();
    for e in [epoch.saturating_sub(1), epoch] {
        for k in 0..64u32 {
            let address = craftworks_wallet::queue::address(e, k);
            let id = t::id(&lot.bag, &address);
            let st = match tokens::there(&got, &id) {
                Err(w) => return (vec![], vec![w]),
                Ok(st) => st,
            };
            let these: Vec<Vec<u8>> = st.and_then(|b| craftworks_bag_contract::read(&address, &b)).unwrap_or_default().iter().map(|x| craftworks_bag_contract::payload(x).to_vec()).collect();
            let full = these.len() >= craftworks_bag_contract::chained::ROOM;
            items.extend(these);
            if !full {
                break;
            }
        }
    }
    // EACH ENTRY by the rule.
    let mut settled = done(h, &r.member);
    let mut out = Vec::new();
    for it in items {
        if settled.contains(&it) {
            continue;
        }
        let Some(q) = craftworks_wallet::queue::Queued::decode(&it) else { continue };
        match witness::entry(&ctx, &lot, &got, &q, &r.member, height) {
            Err(why) => identity::upkeep_say(h, &r.member, &format!("witness: {why}")),
            Ok(tokens::Step::Want(w)) => {
                set_done(h, &r.member, settled);
                return (out, w);
            }
            Ok(tokens::Step::Sign(_)) => {}
            Ok(tokens::Step::Done(witness::Verdict::NotSeated)) => settled.push(it),
            Ok(tokens::Step::Done(witness::Verdict::Refused(why))) => identity::upkeep_say(h, &r.member, &format!("witness: refused seq {}: {why}", q.seq)),
            Ok(tokens::Step::Done(witness::Verdict::Sign { chain, seq, entry })) => {
                match identity::upkeep_witness_sign(h, &r.member, &chain, seq, &entry) {
                    Ok(sig) => {
                        let item = t::wallet::bytes::witness_sig(&t::witness::WitnessSig { chain, seq, entry, sig });
                        let address = t::wallet::witness_bag(&chain, seq);
                        let state = craftworks_bag_contract::encode(&address, &[craftworks_bag_contract::grind(&address, &item)]);
                        out.push(Io::Put { code: Code::Bag, params: address.to_vec(), state });
                    }
                    Err(w) => identity::upkeep_say(h, &r.member, &format!("witness: not signed ({w:?})")),
                }
                settled.push(it);
            }
        }
    }
    set_done(h, &r.member, settled);
    (out, vec![])
}
