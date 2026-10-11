//! THE KEEPER ROUND with no page open (TOKEN §7a): every wake-up, for the next member in turn whose home site handed
//! over a KEEPER MANDATE (`identity::KeeperMandate`: where its standing record is, the epoch it opened, what it hosts)
//! and whose page is away, this delegate does what the page's keeper does — with the page's own rules
//! (`craftworks_rules::keeper`): a standing epoch the page did not open, opened on the same holdings; each height since
//! the last that the beacon samples for this node, answered. The SEAL is the slow part (seconds a piece, by design):
//! ONE piece is sealed a wake-up, the answer put once all its picks are — a keeper is KEPT with KEEP_MIN answers an
//! epoch, so a minute a pick is in time. Its key signs only standing messages (`identity::upkeep_standing_sign`); the
//! record is written as an UPDATE whose delta is the merged record (the contract merges it: no code needed here).

use crate::reads::{self, Round};
use crate::upkeep::{Io, Reply};
use craftworks_identity::{self as identity, Host, KeeperMandate};
use craftworks_rules::keeper::{self, Keep};
use craftworks_rules::tokens::{there, Step};
use serde::{Deserialize, Serialize};

pub const ROUND: &[u8] = b"identity_keeper/round";
const LAST: &[u8] = b"identity_keeper/last";
/// Where a member's progress is: the last height handled, and the answer being sealed.
const AT: &[u8] = b"identity_keeper/at/";
/// Heights looked back at (the page's: a challenge missed while away is answered while its output is readable).
const BACK: u64 = 1024;

/// An ANSWER being made: its height, the beacon's output there, the pieces asked, the sealed hashes so far.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    pub height: u64,
    pub out: [u8; 32],
    pub ids: Vec<[u8; 32]>,
    pub hashes: Vec<([u8; 32], [u8; 32])>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct At {
    pub upto: u64,
    pub pending: Option<Pending>,
}
pub fn at<H: Host>(h: &H, m: &[u8; 32]) -> At {
    h.get_secret(&[AT, &m[..]].concat()).and_then(|b| bincode::deserialize(&b).ok()).unwrap_or_default()
}
fn set_at<H: Host>(h: &mut H, m: &[u8; 32], a: &At) {
    h.set_secret(&[AT, &m[..]].concat(), &bincode::serialize(a).expect("progress encodes"));
}

/// A WAKE-UP: when no round is in flight, one for the next member in turn with a mandate and their page away.
pub fn woke<H: Host>(h: &mut H) -> Vec<Io> {
    if reads::busy(h, ROUND) {
        return Vec::new();
    }
    let all: Vec<[u8; 32]> = identity::upkeep_members(h).into_iter().filter(|m| identity::upkeep_keeper_mandate(h, m).is_some() && identity::upkeep_since_tick(h, m) >= crate::upkeep::PAGE_AWAY).collect();
    let Some(member) = reads::next(h, LAST, &all) else {
        reads::save(h, ROUND, None);
        return Vec::new();
    };
    drive(h, Round { member, started: reads::wakeups(h), ..Round::default() })
}

/// An ANSWER from the network: kept if this round asked for it, then the round driven on.
pub fn replied<H: Host>(h: &mut H, reply: &Reply) -> Vec<Io> {
    let Reply::Got { id, state } = reply else { return Vec::new() };
    match reads::take(h, ROUND, id, state.clone()) {
        Some(r) => drive(h, r),
        None => Vec::new(),
    }
}

fn drive<H: Host>(h: &mut H, r: Round) -> Vec<Io> {
    let Some(m) = identity::upkeep_keeper_mandate(h, &r.member) else {
        reads::save(h, ROUND, None);
        return Vec::new();
    };
    let (out, want) = match step(h, &r, m) {
        Ok(x) => x,
        Err(why) => {
            identity::upkeep_say(h, &r.member, &format!("keeper: {why}"));
            (Vec::new(), Vec::new())
        }
    };
    reads::go(h, ROUND, r, out, want)
}

/// A keeper step to its end with the member's key signing what it asks (standing messages only).
fn signing<H: Host, T>(h: &mut H, member: &[u8; 32], f: &dyn Fn(&[(Vec<u8>, [u8; 64])]) -> Result<Step<T>, String>) -> Result<Step<T>, String> {
    let mut sigs = Vec::new();
    for _ in 0..8 {
        match f(&sigs)? {
            Step::Sign(m) => {
                let s = identity::upkeep_standing_sign(h, member, &m).map_err(|w| format!("not signed ({w:?})"))?;
                sigs.push((m, s));
            }
            other => return Ok(other),
        }
    }
    Err("too many signatures".into())
}

/// One pass over what is read: (what to send now, what to read first).
fn step<H: Host>(h: &mut H, r: &Round, mut m: KeeperMandate) -> Result<(Vec<Io>, Vec<[u8; 32]>), String> {
    let got = |id: &[u8; 32]| r.got.get(id).cloned();
    let k = Keep { standing: m.standing, beacon_code: m.beacon_code, beacon: m.beacon };
    let public = r.member;
    let record = keeper::record_id(&k, &public);
    let update = |state: Vec<u8>| Io::Update { id: record, delta: state };
    let mut out = Vec::new();
    // THE CLOCK.
    let height = match there(&got, &m.beacon) {
        Err(w) => return Ok((out, vec![w])),
        Ok(b) => match b.and_then(|b| craftworks_beacon_contract::wire::decode(&b)) {
            Some(b) => b.height(),
            None => return Ok((out, vec![])),
        },
    };
    let epoch = craftworks_standing::epoch(height);
    // AN EPOCH THE PAGE DID NOT OPEN: opened on the holdings it handed over (what this node still hosts).
    if m.epoch < epoch {
        let hosted = m.hosted.clone();
        match signing(h, &public, &|s| keeper::open_epoch(&k, &got, &public, epoch, &hosted, s))? {
            Step::Want(w) => return Ok((out, w)),
            Step::Sign(_) => unreachable!("signed"),
            Step::Done(put) => {
                out.extend(put.map(|p| update(p.state)));
                m.epoch = epoch;
                identity::upkeep_set_keeper_mandate(h, &public, &m);
                identity::upkeep_say(h, &public, &format!("keeper: epoch {epoch} opened with no page open ({} piece(s))", m.hosted.len()));
            }
        }
    }
    let mut a = at(h, &public);
    // THE NEXT SAMPLED HEIGHT since the last handled (this epoch's).
    if a.pending.is_none() {
        for x in a.upto.saturating_add(1).max(height.saturating_sub(BACK))..=height {
            if craftworks_standing::epoch(x) != epoch {
                a.upto = x;
                continue;
            }
            let o = match craftworks_rules::witness::beacon_out(&m.beacon_code, &got, &m.beacon, x) {
                Err(w) => return Ok((out, vec![w])),
                Ok(Err(_)) => {
                    a.upto = x;
                    continue;
                }
                Ok(Ok(o)) => o,
            };
            match keeper::asked(&k, &got, &public, x, &o, &m.hosted)? {
                Step::Want(w) => return Ok((out, w)),
                Step::Done(Some(ids)) => {
                    a.pending = Some(Pending { height: x, out: o, ids, hashes: Vec::new() });
                    break;
                }
                _ => a.upto = x,
            }
        }
    }
    // THE ANSWER being made: one piece sealed a wake-up; the answer put once every pick is.
    if let Some(p) = a.pending.as_mut() {
        if let Some(id) = p.ids.iter().find(|id| !p.hashes.iter().any(|x| x.0 == **id)).copied() {
            let piece = match there(&got, &id) {
                Err(w) => {
                    set_at(h, &public, &a);
                    return Ok((out, vec![w]));
                }
                Ok(b) => b.unwrap_or_default(),
            };
            p.hashes.push((id, craftworks_standing::sealed_hash(&public, &id, &piece, &p.out, &craftworks_standing::keeper_peer(&public))));
            if p.ids.iter().any(|id| !p.hashes.iter().any(|x| x.0 == *id)) {
                set_at(h, &public, &a);
                return Ok((out, vec![]));
            }
        }
        let p = a.pending.clone().expect("pending");
        let hosted = m.hosted.clone();
        match signing(h, &public, &|s| keeper::answer(&k, &got, &public, p.height, &p.out, &hosted, &p.hashes, s))? {
            Step::Want(w) => {
                set_at(h, &public, &a);
                return Ok((out, w));
            }
            Step::Sign(_) => unreachable!("signed"),
            Step::Done(put) => {
                if let Some(put) = put {
                    out.push(update(put.state));
                    identity::upkeep_say(h, &public, &format!("keeper: height {} answered with no page open ({} piece(s))", p.height, p.ids.len()));
                }
                a.upto = p.height;
                a.pending = None;
            }
        }
    }
    set_at(h, &public, &a);
    Ok((out, vec![]))
}
