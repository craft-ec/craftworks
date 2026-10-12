//! THE KEEPER ROUND with no page open (TOKEN §7a): every wake-up, for the next member in turn whose home site handed
//! over a KEEPER MANDATE (`identity::KeeperMandate`: where its standing record is, the epoch it opened, what it hosts)
//! and whose page is away, this delegate does what the page's keeper does — with the page's own rules
//! (`craftworks_rules::keeper`): a standing epoch the page did not open, opened on the same holdings; each height since
//! the last that the beacon samples for this node, answered. The SEAL is the slow part (seconds a piece, by design):
//! ONE piece is sealed a wake-up, the answer put once all its picks are — a keeper is KEPT with KEEP_MIN answers an
//! epoch, so a minute a pick is in time. Its key signs only standing messages (`identity::upkeep_standing_sign`); the
//! record is written as an UPDATE whose delta is the merged record (the contract merges it: no code needed here).

use crate::reads::{self, Round};
use crate::upkeep::{Code, Io, Reply};
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
    /// The standing epoch it last LISTED in, and the CLAIMS epoch it last CLAIMED for and POINTED to its claim for
    /// (audit 10-11: the page's, while it is closed; C2b 10-12: the claim an entry of its own chain, its pointer posted
    /// once its witnesses accepted it).
    pub listed: Option<u64>,
    pub claimed: Option<u64>,
    pub pointed: Option<u64>,
    /// The wake-up a listing was last sent in (a pass run again in it does not send it twice).
    pub sent: Option<u64>,
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
    // (A pass that asks for reads runs again once they are in: the claim is marked done when built, the listing when
    // sent this wake-up — neither is sent twice.)
    let mut late: Vec<Io> = Vec::new();
    // LISTED in this epoch's keepers registry (chained bags: the first with room), its account's listing.
    if a.listed != Some(epoch) {
        let item = craftworks_reputation::keepers::Keeper { did: m.did, cred: m.credential.clone(), key_seq: m.key_seq }.encode();
        for k in 0..64u32 {
            let address = craftworks_reputation::keepers::address(epoch, k);
            let st = match there(&got, &craftworks_token_contract::id(&m.bag, &address)) {
                Err(w) => {
                    set_at(h, &public, &a);
                    return Ok((out, vec![w]));
                }
                Ok(st) => st,
            };
            let items: Vec<Vec<u8>> = st.and_then(|b| craftworks_bag_contract::read(&address, &b)).unwrap_or_default().iter().map(|x| craftworks_bag_contract::payload(x).to_vec()).collect();
            if items.contains(&item) {
                a.listed = Some(epoch);
                break;
            }
            if craftworks_bag_contract::chained::full(items.iter().map(Vec::len)) {
                continue;
            }
            // (Read back on the next wake-up: listed once it is there.)
            if a.sent == Some(r.started) {
                break;
            }
            a.sent = Some(r.started);
            late.push(Io::Put { code: Code::Bag, params: address.to_vec(), state: craftworks_bag_contract::encode(&address, &[craftworks_bag_contract::grind(&address, &item)]) });
            break;
        }
    }
    // CLAIMED (the account's seat node only, and only in the claim window — shortly before the next claims epoch's
    // freeze: a node asleep then is not seated): its $CRAFT stake for that epoch, its key the witness key — the page's
    // own rule (`keeper::claim`), signed by the member key under the token's domains; the claim queued for its witnesses.
    let ctx = craftworks_rules::tokens::Ctx { code_hash: m.token, beacon: m.beacon, idlog: m.idlog };
    let ce = keeper::claim_epoch(height);
    if m.seat && ce.is_some() && a.claimed != ce {
        let mut sigs: Vec<(Vec<u8>, [u8; 64])> = Vec::new();
        for _ in 0..8 {
            let dev = craftworks_rules::tokens::Device { public, credential: m.credential.clone(), key_seq: m.key_seq, sigs: sigs.clone() };
            match keeper::claim(&ctx, &got, &m.did, height, &dev)? {
                Step::Want(w) => {
                    set_at(h, &public, &a);
                    return Ok((out, w));
                }
                Step::Sign(msg) => {
                    let sig = identity::upkeep_token_sign(h, &public, &msg).map_err(|w| format!("not signed ({w:?})"))?;
                    sigs.push((msg, sig));
                }
                Step::Done(None) => {
                    a.claimed = ce;
                    break;
                }
                Step::Done(Some(c)) => {
                    late.push(Io::Update { id: craftworks_token_contract::id(&m.token, &c.chain.params), delta: c.chain.state.clone() });
                    // QUEUED for its witnesses (the witness queue of this standing epoch: its first bag with room).
                    let st = craftworks_token_contract::State::decode(&c.chain.state).ok_or("a claim's chain that does not read")?;
                    let token = keeper::craft(&ctx).1;
                    let seq = st.entries.last().map_or(0, |e| e.seq);
                    let item = craftworks_wallet::queue::Queued { token: craftworks_token_contract::Tok { definition: token, item: [0; 32] }, owner: m.did, seq }.encode();
                    for k in 0..64u32 {
                        let address = craftworks_wallet::queue::address(epoch, k);
                        let items: Vec<Vec<u8>> = match there(&got, &craftworks_token_contract::id(&m.bag, &address)) {
                            Err(w) => {
                                set_at(h, &public, &a);
                                return Ok((out, vec![w]));
                            }
                            Ok(st) => st.and_then(|b| craftworks_bag_contract::read(&address, &b)).unwrap_or_default().iter().map(|x| craftworks_bag_contract::payload(x).to_vec()).collect(),
                        };
                        if items.contains(&item) || !craftworks_bag_contract::chained::full(items.iter().map(Vec::len)) {
                            late.push(Io::Update { id: craftworks_token_contract::id(&m.bag, &address), delta: craftworks_bag_contract::encode(&address, &[craftworks_bag_contract::grind(&address, &item)]) });
                            break;
                        }
                    }
                    identity::upkeep_say(h, &public, &format!("keeper: {} claimed for claims epoch {} with no page open", c.amount, c.epoch));
                    a.claimed = ce;
                    break;
                }
            }
        }
    }
    // POINTED (C2b): once its claim for the coming claims epoch is ACCEPTED, the pointer to it — its signatures with it —
    // into the epoch's pointer bags (`keeper::point`: the first with room), so readers find it (a staker already in the
    // epoch before is found without one). Only into a bag that exists (a delegate's new contracts do not leave its node:
    // the page makes the coming epochs' first bags).
    let next = height / craftworks_token_contract::supply::EPOCH + 1;
    if m.seat && a.pointed != Some(next) && height < craftworks_token_contract::pointers::frozen_at(next) {
        let lot = craftworks_rules::tokens::Lot { pool: keeper::craft(&ctx).1, bag: m.bag, beacon_code: m.beacon_code, idlog: m.idlog };
        match keeper::point(&ctx, &lot, &got, &m.did, next, height)? {
            Step::Want(w) => {
                out.append(&mut late);
                set_at(h, &public, &a);
                return Ok((out, w));
            }
            Step::Done(Some(p)) => {
                late.push(Io::Update { id: p.id, delta: p.state });
                identity::upkeep_say(h, &public, &format!("keeper: its claim for claims epoch {next} pointed to with no page open"));
                a.pointed = Some(next);
            }
            // (Not claimed, not accepted yet, or pointed to already: asked again on a later wake-up.)
            _ => {}
        }
    }
    // (What these built goes out now, whatever the answers below still ask for — and their marks are kept now.)
    out.append(&mut late);
    set_at(h, &public, &a);
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
