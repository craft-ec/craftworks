//! THE WITNESS ROUND with no page open (TOKEN §7a): every wake-up, for each member whose home site handed over a WITNESS
//! MANDATE (`identity::WitnessMandate`), this delegate reads the beacon and the witness queue (this standing epoch's and
//! the last), and for each entry runs THE WITNESS RULE (`craftworks_rules::witness::entry`, the page's own) over what it
//! read — asking the network for each contract the rule wants, one round trip at a time. An entry the member's stake is
//! drawn for, and the rule takes, is signed through the SAME guard as a page's (`identity::upkeep_witness_sign`: never
//! another entry at a place) and its signature sent to its place's signatures container (`witness::Params`: an UPDATE
//! of the container its chain's writer made — the node merges it, taking it only from a key of the lot's claims
//! registry). Each step is a message out and an answer in, across calls: the round lives in the secrets.

use crate::reads::{self, Round};
use crate::upkeep::{Io, Reply};
use craftworks_identity::{self as identity, Host, WitnessMandate};
use craftworks_rules::{tokens, witness};
use craftworks_token_contract as t;

pub const ROUND: &[u8] = b"identity_witness/round";
/// The member the last round was for (rounds take members in turn).
const LAST: &[u8] = b"identity_witness/last";
/// Queue items this member settled (witnessed, or not drawn for): never asked again (the newest kept) — each by its
/// HASH (32 bytes, not the item), oldest first; looked up through a set, written only when one is added.
const DONE: &[u8] = b"identity_witness/settled/";
const DONE_MAX: usize = 4096;

fn done<H: Host>(h: &H, member: &[u8; 32]) -> Vec<[u8; 32]> {
    h.get_secret(&[DONE, &member[..]].concat()).and_then(|b| bincode::deserialize(&b).ok()).unwrap_or_default()
}
fn set_done<H: Host>(h: &mut H, member: &[u8; 32], mut all: Vec<[u8; 32]>) {
    if all.len() > DONE_MAX {
        all.drain(..all.len() - DONE_MAX);
    }
    h.set_secret(&[DONE, &member[..]].concat(), &bincode::serialize(&all).expect("done encodes"));
}
/// A queue item's key in the settled list.
fn item_hash(it: &[u8]) -> [u8; 32] {
    *blake3::hash(it).as_bytes()
}

/// A WAKE-UP: when no round is in flight, a round started for the next member in turn with a mandate and their page
/// away (an open page witnesses for itself).
pub fn woke<H: Host>(h: &mut H) -> Vec<Io> {
    if reads::busy(h, ROUND) {
        // A round PAUSED (its statement's audit seals spread over wake-ups; nothing to read): driven on.
        if reads::paused(h, ROUND) {
            if let Some(r) = reads::load(h, ROUND) {
                return drive(h, r);
            }
        }
        return Vec::new();
    }
    let all: Vec<[u8; 32]> = identity::upkeep_members(h).into_iter().filter(|m| identity::upkeep_witness_mandate(h, m).is_some() && identity::upkeep_since_tick(h, m) >= crate::upkeep::PAGE_AWAY).collect();
    let Some(member) = reads::next(h, LAST, &all) else {
        reads::save(h, ROUND, None);
        return Vec::new();
    };
    let r = Round { member, started: reads::wakeups(h), ..Round::default() };
    drive(h, r)
}

/// An ANSWER from the network: kept if this round asked for it, then the round driven on.
pub fn replied<H: Host>(h: &mut H, reply: &Reply) -> Vec<Io> {
    let Reply::Got { id, state } = reply else { return Vec::new() };
    match reads::take(h, ROUND, id, state.clone()) {
        Some(r) => drive(h, r),
        None => Vec::new(),
    }
}

/// The round as far as what it read takes it: the next reads asked, or the signatures dropped and the round ended.
fn drive<H: Host>(h: &mut H, r: Round) -> Vec<Io> {
    let Some(m) = identity::upkeep_witness_mandate(h, &r.member) else {
        reads::save(h, ROUND, None);
        return Vec::new();
    };
    let (out, want, paused) = step(h, &r, &m);
    if paused {
        // PAUSED (seals left for the next wake-up): the round kept, as of now (never dropped as stuck while it seals).
        let r = Round { started: reads::wakeups(h), ..r };
        reads::save(h, ROUND, Some(&r));
        return out;
    }
    reads::go(h, ROUND, r, out, want)
}

/// A BAG's payloads as read (`Err(id)`: read its contract first; not there: none).
pub(crate) fn bag_items(got: tokens::Got, bag: &[u8; 32], address: &[u8; 32]) -> Result<Vec<Vec<u8>>, [u8; 32]> {
    let st = tokens::there(got, &t::id(bag, address))?;
    Ok(st.and_then(|b| craftworks_bag_contract::read(address, &b)).unwrap_or_default().iter().map(|x| craftworks_bag_contract::payload(x).to_vec()).collect())
}
/// A CHAINED list's payloads of epoch `e` (bags k = 0 up to the first with room: `address(e, k)`), as the page's
/// `index.bag.chain` reads it.
pub(crate) fn chain_items(got: tokens::Got, bag: &[u8; 32], address: &dyn Fn(u64, u32) -> [u8; 32], e: u64) -> Result<Vec<Vec<u8>>, [u8; 32]> {
    let mut items = Vec::new();
    for k in 0..64u32 {
        let these = bag_items(got, bag, &address(e, k))?;
        let full = craftworks_bag_contract::chained::full(these.iter().map(Vec::len));
        items.extend(these);
        if !full {
            break;
        }
    }
    Ok(items)
}

/// One pass over what is read: (what to send now, what to read first, PAUSED: seals left for a later call).
/// Signatures already made are sent even while more is read for the entries after them.
fn step<H: Host>(h: &mut H, r: &Round, m: &WitnessMandate) -> (Vec<Io>, Vec<[u8; 32]>, bool) {
    let got = |id: &[u8; 32]| r.got.get(id).cloned();
    let ctx = tokens::Ctx { code_hash: m.token, beacon: m.beacon, idlog: m.idlog };
    let lot = tokens::Lot { pool: m.pool, bag: m.bag, beacon_code: m.beacon_code };
    // THE CLOCK.
    let beacon = match tokens::there(&got, &m.beacon) {
        Err(w) => return (vec![], vec![w], false),
        Ok(None) => return (vec![], vec![], false),
        Ok(Some(b)) => b,
    };
    let Some(height) = craftworks_beacon_contract::wire::decode(&beacon).map(|s| s.height()) else { return (vec![], vec![], false) };
    let epoch = height / craftworks_reputation::EPOCH;
    // THE QUEUE: this standing epoch's and the last (chained bags, k = 0 up to the first with room).
    let mut items: Vec<Vec<u8>> = Vec::new();
    for e in [epoch.saturating_sub(1), epoch] {
        match chain_items(&got, &lot.bag, &craftworks_wallet::queue::address, e) {
            Err(w) => return (vec![], vec![w], false),
            Ok(these) => items.extend(these),
        }
    }
    // EACH ENTRY by the rule.
    let mut settled = done(h, &r.member);
    let mut seen: std::collections::HashSet<[u8; 32]> = settled.iter().copied().collect();
    let before = settled.len();
    let mut out = Vec::new();
    for it in items {
        let k = item_hash(&it);
        if seen.contains(&k) {
            continue;
        }
        let Some(q) = craftworks_wallet::queue::Queued::decode(&it) else { continue };
        match witness::entry(&ctx, &lot, &got, &q, &r.member, height) {
            Err(why) => identity::upkeep_say(h, &r.member, &format!("witness: {why}")),
            Ok(tokens::Step::Want(w)) => {
                if settled.len() != before {
                    set_done(h, &r.member, settled);
                }
                return (out, w, false);
            }
            Ok(tokens::Step::Sign(_)) => {}
            Ok(tokens::Step::Done(witness::Verdict::NotSeated)) => {
                seen.insert(k);
                settled.push(k);
            }
            // Not yet decidable: asked again on a later wake-up (never settled).
            Ok(tokens::Step::Done(witness::Verdict::Later(_))) => {}
            Ok(tokens::Step::Done(witness::Verdict::Refused(why))) => identity::upkeep_say(h, &r.member, &format!("witness: refused seq {}: {why}", q.seq)),
            Ok(tokens::Step::Done(witness::Verdict::Sign { at, entry })) => {
                sign_into(h, &r.member, &at, entry, &mut out);
                seen.insert(k);
                settled.push(k);
            }
        }
    }
    // THE EPOCH STATEMENTS due (`witness_statement`): built by the page's rule, signed where the root is posted.
    let next = crate::witness_statement::step(h, r, m, height, &mut settled, &mut out);
    if settled.len() != before {
        set_done(h, &r.member, settled);
    }
    match next {
        crate::witness_statement::Next::Read(w) => (out, w, false),
        crate::witness_statement::Next::Pause => (out, vec![], true),
        crate::witness_statement::Next::Done => (out, vec![], false),
    }
}

/// SIGN `entry` at place `at` through the guard (`identity::upkeep_witness_sign`: never another entry at a place) and
/// send the signature into the place's signatures container: an UPDATE of the container its chain's writer (or a
/// statement's poster) made — the node merging it (`witness::merge`: the member's key registered in the lot's claims
/// epoch). The page's encoding (`witness_put`): `Params::state` of the one signature.
pub(crate) fn sign_into<H: Host>(h: &mut H, member: &[u8; 32], at: &t::witness::Params, entry: [u8; 32], out: &mut Vec<Io>) -> bool {
    match identity::upkeep_witness_sign(h, member, &at.chain, at.seq, &entry) {
        Ok(sig) => {
            let state = at.state(&[(*member, t::witness::WitnessSig { chain: at.chain, seq: at.seq, entry, sig })]);
            out.push(Io::Update { id: at.id(), delta: state });
            true
        }
        Err(w) => {
            identity::upkeep_say(h, member, &format!("witness: not signed ({w:?})"));
            false
        }
    }
}
