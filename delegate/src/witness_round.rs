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
/// Queue items this member settled (witnessed, or not drawn for): never asked again (the newest kept).
const DONE: &[u8] = b"identity_witness/done/";
const DONE_MAX: usize = 4096;

fn done<H: Host>(h: &H, member: &[u8; 32]) -> Vec<Vec<u8>> {
    h.get_secret(&[DONE, &member[..]].concat()).and_then(|b| bincode::deserialize(&b).ok()).unwrap_or_default()
}
fn set_done<H: Host>(h: &mut H, member: &[u8; 32], mut all: Vec<Vec<u8>>) {
    if all.len() > DONE_MAX {
        all.drain(..all.len() - DONE_MAX);
    }
    h.set_secret(&[DONE, &member[..]].concat(), &bincode::serialize(&all).expect("done encodes"));
}

/// A WAKE-UP: when no round is in flight, a round started for the next member in turn with a mandate and their page
/// away (an open page witnesses for itself).
pub fn woke<H: Host>(h: &mut H) -> Vec<Io> {
    if reads::busy(h, ROUND) {
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
    let (out, want) = step(h, &r, &m);
    reads::go(h, ROUND, r, out, want)
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
            let full = craftworks_bag_contract::chained::full(these.iter().map(Vec::len));
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
            // Not yet decidable: asked again on a later wake-up (never settled).
            Ok(tokens::Step::Done(witness::Verdict::Later(_))) => {}
            Ok(tokens::Step::Done(witness::Verdict::Refused(why))) => identity::upkeep_say(h, &r.member, &format!("witness: refused seq {}: {why}", q.seq)),
            Ok(tokens::Step::Done(witness::Verdict::Sign { at, entry })) => {
                match identity::upkeep_witness_sign(h, &r.member, &at.chain, at.seq, &entry) {
                    Ok(sig) => {
                        // Into the place's signatures container (its chain's writer made it): an UPDATE, the node
                        // merging it (`witness::merge`: the member's key registered in the lot's claims epoch).
                        let state = at.state(&[(r.member, t::witness::WitnessSig { chain: at.chain, seq: at.seq, entry, sig })]);
                        out.push(Io::Update { id: at.id(), delta: state });
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
