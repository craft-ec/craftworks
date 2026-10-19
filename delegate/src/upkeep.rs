//! THE NODE'S FOLLOWER (FOUNDATION "Members and keys", the design of 10-17) — whoever a space's list no longer names is
//! taken out of its keys HERE, by the account's node, on every wake-up (and whenever a page asks it to run now), page
//! open or not. Nobody is let IN here: a listed device enters by itself (`mls::enter`). A round, for one member on this
//! node, space by space (every kind: a server, a conversation, a level group, an audience, a circle):
//!
//!   1. READ the space's acts (`table::Reading`, every member's feeds) — the list (`gov`'s replay), whole or not.
//!   2. CATCH UP: each position's entries — its epoch log's and its DOOR's (a device that entered by itself) — in the
//!      one order (`rules::ordering`); the first MLS applies AND the list allows taken (`mls::entry_ok`, a self-entry's
//!      proof by `Gov::admits`): the rule every page reads by. Nothing is decided from a short read (I6).
//!   3. FOLLOW OUT (`rules::members::follow`): every entry of an account the list no longer names — one member at a time
//!      (`admission::turn`, taken over when away).
//!   4. ONE COMMIT into its epoch's log; the next epoch's log opened (its way back); its GROUP INFO put in that epoch's
//!      door and announced in the writers list — what a listed device with no key enters from.
//!
//! Each step is a message out and an answer in; the round lives in the delegate's secrets. Pure: the whole round runs
//! in tests against a scripted network.

use craftworks_data as data;
use craftworks_identity::{self as identity, Host, Mandate};
use craftworks_rules::members;
use serde::{Deserialize, Serialize};

use crate::rekey_round::Held;

/// K5: a BeeKEM space kept to its list by the node (its own file; a child, so it reads the round as upkeep does).
#[path = "keys_round.rs"]
mod keys_round;
use crate::table::Reading;

/// What upkeep asks of the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Io {
    /// GET (and follow: a request bag stays current on this node) a contract's state.
    Get { id: [u8; 32], subscribe: bool },
    /// PUT a contract (`code`: which of upkeep's) with a state.
    Put { code: Code, params: Vec<u8>, state: Vec<u8> },
    /// UPDATE a contract with a delta.
    Update { id: [u8; 32], delta: Vec<u8> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Code {
    Bag,
    Tail,
    /// A tree block of a sealed table (a flush from upkeep).
    Sealed,
    /// A file's piece (a re-key from upkeep): its `piece` contract.
    Piece,
}

/// What the network answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Got { id: [u8; 32], state: Option<Vec<u8>> },
    Put { id: [u8; 32], ok: bool },
    Updated { id: [u8; 32], ok: bool },
}

/// THE COMMIT made (`Mls::follow`: entries out), carried until its log and the next epoch's are written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Moved {
    from_epoch: u64,
    from_secret: [u8; 32],
    commit: Vec<u8>,
    info: Vec<u8>,
    epoch: u64,
    secret: [u8; 32],
    state: Vec<u8>,
    removed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Step {
    /// The space's acts being read.
    Acts,
    /// Position `epoch`'s entries being read: its epoch log's and its door's (`None`: not answered yet).
    CatchUp { epoch: u64, secret: [u8; 32], log: Option<Option<Vec<u8>>>, door: Option<Option<Vec<u8>>> },
    Log { m: Moved },
    Commit { m: Moved },
    Next { m: Moved },
    /// K5: a BeeKEM space's operations being written into this member's own `pub-keyops` (`rows` still to send;
    /// `waiting`: the contracts whose answers come first; `blocks`: those were a flush's blocks).
    Keys { rows: Vec<(Vec<u8>, Option<Vec<u8>>)>, waiting: Vec<[u8; 32]>, blocks: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Round {
    member: [u8; 32],
    me: String,
    space: [u8; 32],
    /// The spaces still to visit after this one.
    rest: Vec<[u8; 32]>,
    reading: Reading,
    step: Step,
    /// The group as this round holds it (caught up so far).
    state: Vec<u8>,
    /// When the round last moved (the node's time, ms): one not moved for `STUCK_MS` is dropped — by TIME, never only by
    /// wake-ups (which may not come: I2).
    #[serde(default)]
    moved_ms: u64,
    /// The wake-up count when the round last moved.
    moved: u64,
    /// K5: this read is by EVERY account the list ever named (a BeeKEM space: whom to add or take out are read by their
    /// cards — a newcomer is in no group yet).
    #[serde(default)]
    by_list: bool,
}

pub const ROUND: &[u8] = b"identity_upkeep/round";
/// A page that handed the mandate over this recently is running (the re-key round stays out of its way; the follower
/// does not: it is the only one).
pub const PAGE_AWAY: u64 = 2;
pub const STUCK: u64 = 3;
/// The member whose round ran last (the next round starts after it: each person in turn).
const LAST_SERVED: &[u8] = b"identity_upkeep/last-served";
/// Hex characters of a group info per door item (a bag item holds 16 KiB) — the page's `keys` PART.
const PART: usize = 12000;
/// When this node first saw entries due out at an epoch (`member ‖ space` → `(epoch, ms)`): where a takeover counts
/// from when no act names a time (a stray never put out by one).
const DUE: &[u8] = b"identity_upkeep/due/";

/// The epoch whose info this node last published (`member ‖ space` → epoch): once an epoch.
const ANNOUNCED: &[u8] = b"identity_upkeep/announced/";

/// THE GROUP INFO, once an epoch (FOUNDATION "Members and keys" 4: what a listed device with no key enters from): the
/// owner's node — its signer proven by the space's id — and the nodes of two members in turn (`members::publishes_info`:
/// the owner away, a keyless member still enters), whenever no commit of its own published it.
fn group_info<H: Host>(h: &mut H, c: &Held, r: &Round, m: &Mandate, followers: &[String], g: &mut craftworks_mls::Account) -> Vec<Io> {
    let key = [ANNOUNCED, &r.member[..], &r.space[..]].concat();
    let epoch = g.epoch();
    if !members::publishes_info(&r.me, &m.owner, followers, epoch) || h.get_secret(&key).and_then(|b| <[u8; 8]>::try_from(b.as_slice()).ok()).map(u64::from_le_bytes) == Some(epoch) {
        return Vec::new();
    }
    let (Ok(info), Ok(secret)) = (g.group_info(), g.epoch_secret()) else { return Vec::new() };
    let io = door_io(h, c, &r.member, &r.space, epoch, &secret, &info);
    // NOTHING WRITTEN (no randomness: a page stirs the pool): said, and the epoch NOT marked — asked again next round
    // (before, it was marked first, and an epoch whose info found the pool empty was never published).
    if io.is_empty() {
        followed(h, &r.member, &r.space, "info-held", &format!("{}: epoch {epoch}: no randomness yet (a page stirs the pool): its group info waits", m.name));
        return io;
    }
    h.set_secret(&key, &epoch.to_le_bytes());
    let whose = if r.me == m.owner { "the owner's" } else { "a member's, in turn" };
    followed(h, &r.member, &r.space, "info", &format!("{}: epoch {epoch}: group info published ({whose})", m.name));
    io
}

/// FROM WHEN a removal is due here: the act's time, or when this node first saw it due at this epoch (kept).
fn due_since<H: Host>(h: &mut H, member: &[u8; 32], space: &[u8; 32], epoch: u64, out_since: u64, now_ms: u64) -> u64 {
    let key = [DUE, &member[..], &space[..]].concat();
    let seen = h.get_secret(&key).and_then(|b| bincode::deserialize::<(u64, u64)>(&b).ok()).filter(|(e, _)| *e == epoch).map(|(_, t)| t);
    let seen = seen.unwrap_or_else(|| {
        h.set_secret(&key, &bincode::serialize(&(epoch, now_ms)).expect("two numbers"));
        now_ms
    });
    out_since.max(seen)
}

fn round<H: Host>(h: &H) -> Option<Round> {
    h.get_secret(ROUND).filter(|b| !b.is_empty()).and_then(|b| bincode::deserialize(&b).ok())
}
pub(crate) fn keep<H: Host>(h: &mut H, r: Option<&Round>) {
    h.set_secret(ROUND, &r.map(|r| bincode::serialize(r).expect("a round encodes")).unwrap_or_default());
}
fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len() / 2).map(|i| u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()).collect()
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn mandate<H: Host>(h: &H, r: &Round) -> Option<Mandate> {
    identity::upkeep_mandate(h, &r.member)?.1.into_iter().find(|m| m.space == r.space)
}

/// THE ROUND's step over a space (`identity::Decided`: the `group` lifecycle's `follow-<step>` move, by the node — its
/// plan, its commit, an entry skipped, a stall, each with why). Never free text beside it.
pub(crate) fn followed<H: Host>(h: &mut H, member: &[u8; 32], space: &[u8; 32], step: &str, why: &str) {
    let at = identity::upkeep_now(h).unwrap_or(0) * 1000;
    identity::upkeep_decide(h, member, identity::Decided { seq: 0, space: *space, did: String::new(), q: [0; 16], at, step: step.into(), why: why.into() });
}

/// Whether a contract this delegate CREATES reaches the network. Not on freenet ≤ 0.2.141 (measured 10-05 on a test
/// node: a delegate's `PutContractRequest` is stored on its own node, then broadcast only to peers already interested in
/// it — a new contract has none (`BROADCAST_NO_TARGETS`, giving up), and a GET from anywhere, its own node's pages too,
/// answers NotFound; freenet #5542). So upkeep writes only to contracts already there: anything that makes one — a
/// file's new pieces, a new feed or catalog, a flush's tree blocks — is left to a page.
pub const NEW_CONTRACTS_REACH_THE_NETWORK: bool = false;
#[cfg(test)]
thread_local! {
    /// A test's network that does take them (the paths that create, exercised).
    pub static CREATES: std::cell::Cell<bool> = const { std::cell::Cell::new(NEW_CONTRACTS_REACH_THE_NETWORK) };
}
pub fn creates() -> bool {
    #[cfg(test)]
    return CREATES.with(|c| c.get());
    #[cfg(not(test))]
    NEW_CONTRACTS_REACH_THE_NETWORK
}

/// The id of the contract `code` names with `params`.
pub fn id_of(code_hash: &[u8; 32], params: &[u8]) -> [u8; 32] {
    contract_keys::instance(code_hash, params)
}

/// The group's ENTRIES — each one's DID, and whether the proof in its leaf is one `gov` admits now (none given: not
/// judged — the round begins before the acts are read).
fn leaves(g: &craftworks_mls::Account, gov: Option<&craftworks_gov::Gov>, now_ms: u64) -> Vec<members::Leaf> {
    g.entries()
        .into_iter()
        .map(|(index, did, proof)| {
            // (Never one the acts put out — removed, banned, left, lapsed — whatever its leaf says: the list decides.)
            let admitted = gov.is_some_and(|gov| !gov.gone.contains(&did) && !gov.bans.contains(&did) && serde_json::from_slice::<serde_json::Value>(&proof).is_ok_and(|p| gov.admits(&did, &p, now_ms as f64).is_ok()));
            members::Leaf { index, did, admitted }
        })
        .collect()
}

/// A space's round begun: its acts read, by its members' devices (the group's entries and its owner).
fn begin<H: Host>(h: &mut H, c: &Held, member: [u8; 32], me: String, mut rest: Vec<[u8; 32]>) -> Vec<Io> {
    while !rest.is_empty() {
        let space = rest.remove(0);
        let Some(m) = identity::upkeep_mandate(h, &member).and_then(|(_, ms)| ms.into_iter().find(|m| m.space == space)) else { continue };
        let Some((_, epochs)) = crate::rekey_round::space_of(h, &member, &m) else {
            followed(h, &member, &space, "unloadable", &format!("{}: its group does not load here: not followed", m.name));
            continue;
        };
        let Ok(g) = craftworks_mls::Account::load(craftworks_mls::Rule::Space(space), &m.state) else { continue };
        // WHOSE DEVICES are read (`account::reading_dids`, the page's too): the group's members and the owner.
        let group: Vec<String> = leaves(&g, None, 0).into_iter().map(|l| l.did).collect();
        let dids: Vec<[u8; 32]> = craftworks_account::reading_dids(&group, Some(&m.owner)).iter().filter_map(|d| craftworks_account::did_bytes(d)).collect();
        let (reading, io) = Reading::of_members(&c.codes(), space, epochs, &dids, &["acts", "pub-acts", "pub-keyops"]);
        let r = Round { member, me, space, rest, reading, step: Step::Acts, state: m.state.clone(), moved_ms: identity::upkeep_now(h).unwrap_or(0) * 1000, moved: identity::upkeep_wakeups(h), by_list: false };
        keep(h, Some(&r));
        return io;
    }
    keep(h, None);
    Vec::new()
}
pub(crate) fn next_space<H: Host>(h: &mut H, c: &Held, r: Round) -> Vec<Io> {
    begin(h, c, r.member, r.me, r.rest)
}

/// The space's list as its acts say (`gov`'s replay, the page's too).
pub(crate) fn governance<H: Host>(h: &H, r: &Round, now_ms: u64) -> Option<craftworks_gov::Gov> {
    let m = mandate(h, r)?;
    Some(crate::rekey::governance(&r.reading, &r.reading.writer_dids, &m.owner, now_ms))
}

/// The list read WHOLE (the guard): no feed silent, no act named and not held.
pub(crate) fn whole(r: &Round, gov: &craftworks_gov::Gov) -> Result<(), String> {
    let have: std::collections::BTreeSet<String> = r.reading.rows("acts").keys().chain(r.reading.rows("pub-acts").keys()).map(|k| String::from_utf8_lossy(k).into_owned()).collect();
    craftworks_rules::guards::list_whole(&r.reading.devices_unread, &r.reading.feeds_unread, gov.missing(&have).len())
}

/// A WAKE-UP: a stuck round dropped (said why); else, none running, the next member's round begun (each in turn).
pub fn woke<H: Host>(h: &mut H, now_ms: u64) -> Vec<Io> {
    let _ = now_ms;
    if let Some(r) = round(h) {
        if running(h) {
            return Vec::new();
        }
        let at = match &r.step {
            Step::Acts => format!("the space's acts never answered whole ({})", r.reading.waiting_for()),
            Step::CatchUp { epoch, .. } => format!("epoch {epoch}'s entries never answered"),
            Step::Log { .. } => "the epoch's log never answered".into(),
            Step::Commit { .. } => "the commit was never answered".into(),
            Step::Next { .. } => "the next epoch's log never answered".into(),
            Step::Keys { .. } => "a key operation's write never answered".into(),
        };
        followed(h, &r.member, &r.space, "stalled", &format!("{}: the round stalled — {at}: dropped", hex(&r.space[..6])));
        keep(h, None);
    }
    start(h)
}

/// A PAGE ASKED (`Request::FollowNow`): a round begun now when none is running (a running one sees the change itself).
pub fn now<H: Host>(h: &mut H, member: [u8; 32], now_ms: u64) -> Vec<Io> {
    let _ = now_ms;
    // ITS OWN round running: left to go on. ANOTHER member's: set aside — a page asking is this person waiting on
    // their own space now (a household's node, fx 10-17: one slow member's rounds kept another's removal out for
    // minutes); the other's round begins again at a later wake-up.
    if running(h) && round(h).is_some_and(|r| r.member == member) {
        return Vec::new();
    }
    keep(h, None);
    // THIS member's round (the one whose page asked) — never the next in turn.
    let Some(c) = Held::of(h) else { return Vec::new() };
    let Some((me, spaces)) = identity::upkeep_mandate(h, &member) else { return Vec::new() };
    let all: Vec<[u8; 32]> = spaces.iter().map(|m| m.space).collect();
    let w = identity::upkeep_wakeups(h);
    identity::upkeep_set_round_at(h, &member, w);
    identity::upkeep_set_next(h, &member, 0);
    begin(h, &c, member, me, all)
}
/// A round RUNNING (moved within `STUCK` wake-ups and `STUCK_MS`): left to go on.
fn running<H: Host>(h: &H) -> bool {
    let now = identity::upkeep_now(h).unwrap_or(0) * 1000;
    round(h).is_some_and(|r| identity::upkeep_wakeups(h).saturating_sub(r.moved) < STUCK && now.saturating_sub(r.moved_ms) < STUCK_MS)
}
/// A round not moved for this long (the node's time) is stalled.
const STUCK_MS: u64 = 3 * 60 * 1000;

/// The next member's round (each person in turn), over every space of theirs.
fn start<H: Host>(h: &mut H) -> Vec<Io> {
    let Some(c) = Held::of(h) else { return Vec::new() };
    let w = identity::upkeep_wakeups(h);
    let members = identity::upkeep_members(h);
    let last = h.get_secret(LAST_SERVED).and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok());
    let from = last.and_then(|l| members.iter().position(|m| *m == l)).map_or(0, |i| i + 1);
    for member in members.iter().cycle().skip(from).take(members.len()).copied() {
        let Some((me, spaces)) = identity::upkeep_mandate(h, &member) else { continue };
        let all: Vec<[u8; 32]> = spaces.iter().map(|m| m.space).collect();
        if all.is_empty() {
            continue;
        }
        h.set_secret(LAST_SERVED, &member);
        identity::upkeep_set_round_at(h, &member, w);
        identity::upkeep_set_next(h, &member, 0);
        let io = begin(h, &c, member, me, all);
        if !io.is_empty() {
            return io;
        }
    }
    Vec::new()
}

/// A position's DOOR (its bag, by the space's id and the epoch).
fn door_id(c: &Held, space: &[u8; 32], epoch: u64) -> [u8; 32] {
    id_of(&c.bag, &crate::table::sealed_bag_address(space, &format!("door-{epoch}")))
}

/// The contract a step waits on (`CatchUp` matches its two itself).
fn waits_for(r: &Round, c: &Held, channel: Option<&str>) -> Option<[u8; 32]> {
    match &r.step {
        Step::Acts | Step::CatchUp { .. } | Step::Keys { .. } => None,
        Step::Log { m } | Step::Commit { m } => Some(epoch_log(&c.tail, channel?, &m.from_secret).id_bytes()),
        Step::Next { m } => Some(epoch_log(&c.tail, channel?, &m.secret).id_bytes()),
    }
}

/// CATCH UP from the group this round holds: position `epoch`'s log and door asked.
fn catch_up<H: Host>(h: &mut H, c: &Held, mut r: Round, m: &Mandate) -> Vec<Io> {
    let Ok(g) = craftworks_mls::Account::load(craftworks_mls::Rule::Space(r.space), &r.state) else { return next_space(h, c, r) };
    let (epoch, Ok(secret)) = (g.epoch(), g.epoch_secret()) else { return next_space(h, c, r) };
    let log = epoch_log(&c.tail, &m.channel, &secret);
    let io = vec![Io::Get { id: log.id_bytes(), subscribe: false }, Io::Get { id: door_id(c, &r.space, epoch), subscribe: false }];
    r.step = Step::CatchUp { epoch, secret, log: None, door: None };
    keep(h, Some(&r));
    io
}

/// An ANSWER from the network: to EVERY round that asked for it (the re-key round and this one run side by side and
/// read the same space — its writers bag, its members' devices: fx 10-17, an answer given to the re-key round alone left
/// this one waiting with nothing asked, until it was dropped as stalled). What to send next.
pub fn replied<H: Host>(h: &mut H, reply: Reply, now_ms: u64) -> Vec<Io> {
    let id = match &reply {
        Reply::Got { id, .. } | Reply::Put { id, .. } | Reply::Updated { id, .. } => *id,
    };
    let mut io = if crate::rekey_round::wants(h, &id) { crate::rekey_round::replied(h, reply.clone(), now_ms) } else { Vec::new() };
    io.extend(follow_replied(h, reply, now_ms));
    io
}

fn follow_replied<H: Host>(h: &mut H, reply: Reply, now_ms: u64) -> Vec<Io> {
    let id = match &reply {
        Reply::Got { id, .. } | Reply::Put { id, .. } | Reply::Updated { id, .. } => *id,
    };
    let Some(mut r) = round(h) else { return Vec::new() };
    let Some(c) = Held::of(h) else { return Vec::new() };
    let Some(m) = mandate(h, &r) else {
        keep(h, None);
        return Vec::new();
    };
    // AN ANSWER IS ITS STEP'S ONLY IF IT IS FOR THE CONTRACT THAT STEP ASKED (10-14: a stray answer taken as a key log).
    if waits_for(&r, &c, Some(&m.channel)).is_some_and(|w| w != id) {
        return Vec::new();
    }
    r.moved = identity::upkeep_wakeups(h);
    r.moved_ms = now_ms;
    let member = r.member;
    match (r.step.clone(), reply) {
        (Step::Acts, Reply::Got { id, state }) => {
            if !r.reading.wants(&id) {
                return Vec::new();
            }
            let io = r.reading.got(&c.codes(), id, state);
            if !r.reading.done() {
                keep(h, Some(&r));
                return io;
            }
            // A SPACE WITH A BEEKEM TREE (its operations in `pub-keyops`): kept to the list here (K5), not by MLS.
            if !r.reading.rows("pub-keyops").is_empty() {
                if r.by_list {
                    return keys_round::reconcile(h, &c, r, &m, now_ms);
                }
                // THE CARDS STILL NEEDED: the accounts listed now whose member this node has not learned (kept per space:
                // `keys_round::known`) — at most CARDS_A_ROUND, the rest next round; this read's tables kept. (Not
                // everyone the list ever named, every round: fx 10-18, past the node's per-round cap, and on the live
                // network the delegate out of time — the page's requests unanswered.)
                let Some(gov) = governance(h, &r, now_ms) else { return next_space(h, &c, r) };
                let known = keys_round::known(h, &r.member, &r.space);
                let asked = keys_round::asked(h, &r.member, &r.space);
                let recently = |d: &str| asked.iter().any(|(k, t)| k == d && now_ms.saturating_sub(*t) < keys_round::ASK_AGAIN_MS);
                let mut want: Vec<String> = gov
                    .roster
                    .iter()
                    .chain(std::iter::once(&m.owner))
                    .filter(|d| !known.iter().any(|(k, ..)| k == *d) && !r.reading.beekem.iter().any(|(k, ..)| k == *d) && !recently(d))
                    .cloned()
                    .collect();
                want.sort();
                want.dedup();
                want.truncate(keys_round::CARDS_A_ROUND);
                keys_round::asking(h, &r.member, &r.space, &want, now_ms);
                let want: Vec<[u8; 32]> = want.iter().filter_map(|d| craftworks_account::did_bytes(d)).collect();
                r.by_list = true;
                if want.is_empty() {
                    return keys_round::reconcile(h, &c, r, &m, now_ms);
                }
                let io = r.reading.cards_of(&c.codes(), &want);
                keep(h, Some(&r));
                return io;
            }
            // A BEEKEM SPACE whose key operations read EMPTY (its feeds not found, or not answered): its operations did not
            // read — never taken for an MLS space (10-18, live: a node's round went to the MLS catch-up of a BeeKEM space
            // at epoch 149, whose log nobody writes, and stalled 33 times). Said with what the read noted; next round.
            if r.reading.holds_generation() {
                let notes = if r.reading.feeds_unread.is_empty() { "no feed noted".to_string() } else { r.reading.feeds_unread.iter().map(|(t, w)| format!("{t} of {w}")).collect::<Vec<_>>().join("; ") };
                followed(h, &member, &r.space, "short", &format!("{}: its key operations read empty though it holds a BeeKEM generation ({notes}): not followed this round", m.name));
                return next_space(h, &c, r);
            }
            catch_up(h, &c, r, &m)
        }
        (Step::Keys { rows, mut waiting, blocks }, Reply::Put { id, ok } | Reply::Updated { id, ok }) if waiting.contains(&id) => {
            waiting.retain(|w| *w != id);
            if !ok {
                followed(h, &member, &r.space, "keys-refused", &format!("{}: a key operation's write was refused: the next round reads again", m.name));
                return next_space(h, &c, r);
            }
            if !waiting.is_empty() {
                r.step = Step::Keys { rows, waiting, blocks };
                keep(h, Some(&r));
                return Vec::new();
            }
            if rows.is_empty() {
                return next_space(h, &c, r);
            }
            keys_round::write(h, &c, r, &m, rows, blocks)
        }
        (Step::CatchUp { epoch, secret, mut log, mut door }, Reply::Got { id, state }) => {
            if id == epoch_log(&c.tail, &m.channel, &secret).id_bytes() && log.is_none() {
                log = Some(state);
            } else if id == door_id(&c, &r.space, epoch) && door.is_none() {
                door = Some(state);
            } else {
                return Vec::new();
            }
            let (Some(log), Some(door)) = (log.clone(), door.clone()) else {
                r.step = Step::CatchUp { epoch, secret, log, door };
                keep(h, Some(&r));
                return Vec::new();
            };
            entries(h, &c, r, &m, epoch, &secret, log, door, now_ms)
        }
        (Step::Log { m: mv }, Reply::Got { state, .. }) => {
            let mut log = epoch_log(&c.tail, &m.channel, &mv.from_secret);
            if let Some(st) = &state {
                log.absorb(st);
            }
            // THE LOG's ONE ROW RULE (`rules::ordering`, the pages' too): taken if any writer's row holds the position.
            use craftworks_rules::ordering;
            let taken = matches!(log.opened(), Ok(data::Step::Ready(rows)) if ordering::entry_at(rows.iter().map(|(k, v)| (k.as_slice(), v.as_slice())), "c/", mv.from_epoch).is_some());
            if taken {
                followed(h, &member, &r.space, "behind", &format!("{}: the group moved from epoch {} meanwhile: caught up next round", m.name, mv.from_epoch));
                return next_space(h, &c, r);
            }
            let at = ordering::row_key("c/", mv.from_epoch, &hex(&member[..6]));
            // Its PROVENANCE as a page's rows carry it (`keys.provOf`): who moved the group, why, when.
            let why = format!("follow (node): {} out", mv.removed);
            let entry = serde_json::json!({ "commit": hex(&mv.commit), "info": hex(&mv.info), "by": hex(&member[..8]), "why": why, "at": now_ms, "v": "node" }).to_string();
            let key = identity::epoch_log_key(&mv.from_secret);
            let Some(send) = signed(&mut log, &key, at.as_bytes(), entry.as_bytes()) else {
                followed(h, &member, &r.space, "log-refused", &format!("{}: the epoch's log would not take the commit", m.name));
                return next_space(h, &c, r);
            };
            let io = send_io(&log, send, Code::Tail);
            r.step = Step::Commit { m: mv };
            keep(h, Some(&r));
            vec![io]
        }
        (Step::Commit { m: mv }, Reply::Updated { ok, .. } | Reply::Put { ok, .. }) => {
            if !ok {
                followed(h, &member, &r.space, "commit-refused", &format!("{}: the commit was refused: the group moved meanwhile", m.name));
                return next_space(h, &c, r);
            }
            // The group moved: held here now (its secret kept, the mandate's group this one).
            held(h, &member, &r.space, mv.epoch, &mv.secret, &mv.state);
            let mut next = epoch_log(&c.tail, &m.channel, &mv.secret);
            let open = serde_json::json!({ "info": hex(&mv.info), "prev": hex(&mv.from_secret) }).to_string();
            let key = identity::epoch_log_key(&mv.secret);
            let Some(send) = signed(&mut next, &key, b"open", open.as_bytes()) else {
                followed(h, &member, &r.space, "next-refused", &format!("{}: the next epoch's log would not take its first row", m.name));
                return next_space(h, &c, r);
            };
            let io = send_io(&next, send, Code::Tail);
            r.step = Step::Next { m: mv };
            keep(h, Some(&r));
            vec![io]
        }
        (Step::Next { m: mv }, Reply::Put { .. } | Reply::Updated { .. }) => {
            followed(h, &member, &r.space, "commit", &format!("{}: {} entr(ies) out, epoch {}", m.name, mv.removed, mv.epoch));
            // ITS GROUP INFO in the new epoch's door, announced: what a listed device with no key enters from.
            h.set_secret(&[ANNOUNCED, &member[..], &r.space[..]].concat(), &mv.epoch.to_le_bytes());
            let mut io = door_io(h, &c, &member, &r.space, mv.epoch, &mv.secret, &mv.info);
            io.extend(next_space(h, &c, r));
            io
        }
        // An answer this step does not wait for (a late one): nothing.
        _ => Vec::new(),
    }
}

/// POSITION `epoch`'s ENTRIES — the epoch log's and the door's, in the one order (`rules::ordering::order`): the first
/// MLS applies AND the list allows (`mls::entry_ok`) taken, then the next position; none: caught up — follow out.
#[allow(clippy::too_many_arguments)]
fn entries<H: Host>(h: &mut H, c: &Held, mut r: Round, m: &Mandate, epoch: u64, secret: &[u8; 32], log: Option<Vec<u8>>, door: Option<Vec<u8>>, now_ms: u64) -> Vec<Io> {
    use craftworks_rules::ordering;
    let Some(gov) = governance(h, &r, now_ms) else { return next_space(h, c, r) };
    // NOTHING DECIDED FROM A SHORT READ (I6): no entry judged, nobody taken out.
    if let Err(why) = whole(&r, &gov) {
        followed(h, &r.member, &r.space, "short", &format!("{}: {why} — nothing decided this round", m.name));
        return next_space(h, c, r);
    }
    let mut o = epoch_log(&c.tail, &m.channel, secret);
    if let Some(st) = &log {
        o.absorb(st);
    }
    let mut all: Vec<Vec<u8>> = match o.opened() {
        Ok(data::Step::Ready(rows)) => ordering::entries_at(rows.iter().map(|(k, v)| (k.as_slice(), v.as_slice())), "c/", epoch).into_iter().map(<[u8]>::to_vec).collect(),
        _ => Vec::new(),
    };
    let name = format!("door-{epoch}");
    let address = crate::table::sealed_bag_address(&r.space, &name);
    for p in door.as_deref().and_then(|st| craftworks_bag_contract::read(&address, st)).unwrap_or_default() {
        let Some(v) = crate::table::open_bag_item(&r.space, &Default::default(), &name, craftworks_bag_contract::payload(&p)) else { continue };
        if v["k"] == "enter" && v["e"].as_u64() == Some(epoch) {
            if let Some(e) = v["entry"].as_str() {
                all.push(e.as_bytes().to_vec());
            }
        }
    }
    let Ok(mut g) = craftworks_mls::Account::load(craftworks_mls::Rule::Space(r.space), &r.state) else { return next_space(h, c, r) };
    let list = GovList(&gov, now_ms);
    for entry in ordering::order(all.iter().map(Vec::as_slice)) {
        let Some(commit) = serde_json::from_slice::<serde_json::Value>(entry).ok().and_then(|v| v["commit"].as_str().and_then(unhex)) else { continue };
        match g.apply_checked(&commit, &list) {
            Ok(craftworks_mls::Applied::Applied(_)) => {
                let (Ok(s), Ok(state)) = (g.epoch_secret(), g.save()) else { return next_space(h, c, r) };
                held(h, &r.member, &r.space, g.epoch(), &s, &state);
                r.state = state;
                return catch_up(h, c, r, m);
            }
            Ok(craftworks_mls::Applied::Skipped(why)) => followed(h, &r.member, &r.space, "entry-skipped", &format!("{}: epoch {epoch}: {why}", m.name)),
            Err(_) => {}
        }
    }
    follow_out(h, c, r, m, &gov, g, now_ms)
}

/// THE LIST an entry is judged by (`mls::List`): EVER listed (`Gov::ever_listed`), a self-entry's proof by the list's
/// rule now (`Gov::admits`: uses, expiry, standing — one entering is listed by its own `join` act soon after), who may be
/// taken out (`may_be_taken_out`) — the page's the same (`keys`).
struct GovList<'a>(&'a craftworks_gov::Gov, u64);
impl craftworks_mls::List for GovList<'_> {
    fn listed(&self, did: &str) -> bool {
        self.0.ever_listed.contains(did) && !self.0.bans.contains(did)
    }
    fn admits(&self, did: &str, proof: &[u8]) -> bool {
        serde_json::from_slice::<serde_json::Value>(proof).is_ok_and(|p| self.0.admits(did, &p, self.1 as f64).is_ok())
    }
    fn may_take_out(&self, did: &str) -> bool {
        self.0.may_be_taken_out(did)
    }
}

/// FOLLOW OUT (`rules::members::follow`): every entry of an account not listed — ONE commit, by ONE member at a time.
fn follow_out<H: Host>(h: &mut H, c: &Held, mut r: Round, m: &Mandate, gov: &craftworks_gov::Gov, mut g: craftworks_mls::Account, now_ms: u64) -> Vec<Io> {
    let entries = leaves(&g, Some(gov), now_ms);
    let listed: Vec<String> = gov.roster.iter().cloned().collect();
    let followers: Vec<String> = listed.iter().filter(|d| entries.iter().any(|l| l.did == **d)).cloned().collect();
    let (from_epoch, Ok(from_secret)) = (g.epoch(), g.epoch_secret()) else { return next_space(h, c, r) };
    let p = members::follow(&members::View {
        me: &r.me,
        listed: &listed,
        whole: true,
        leaves: &entries,
        followers: &followers,
        epoch: from_epoch,
        out_since: due_since(h, &r.member, &r.space, from_epoch, gov.out_since() as u64, now_ms),
        now: now_ms,
        takeover_ms: craftworks_rules::guards::TAKEOVER_MS,
    });
    if p.is_empty() {
        if let Some(why) = &p.not_my_turn {
            followed(h, &r.member, &r.space, "plan", &format!("{}: {} out by the list — {why}", m.name, entries.iter().filter(|l| !gov.roster.contains(&l.did) && !l.admitted).count()));
        }
        // NOT ITS TURN YET: when it is (the page asks the node then: its wake-ups may not come).
        if let Some(w) = p.wait {
            identity::upkeep_set_next(h, &r.member, now_ms + w);
        }
        let mut io = group_info(h, c, &r, m, &followers, &mut g);
        io.extend(next_space(h, c, r));
        return io;
    }
    let whom: Vec<String> = entries.iter().filter(|l| p.remove.contains(&l.index)).map(|l| format!("{}…", l.did.get(12..20).unwrap_or(&l.did))).collect();
    followed(h, &r.member, &r.space, "plan", &format!("{}: {} out ({})", m.name, p.remove.len(), whom.join(", ")));
    // (A delegate has no randomness of its own: upkeep's pool ARMS a stream before MLS draws — `crate::arm_random`.)
    let Some(seed) = identity::upkeep_random(h) else {
        followed(h, &r.member, &r.space, "no-commit", &format!("{}: no randomness yet (a page stirs the pool): the removal waits", m.name));
        return next_space(h, c, r);
    };
    crate::arm_random(seed);
    let commit = match g.remove(&p.remove) {
        Ok(c) => c,
        Err(why) => {
            followed(h, &r.member, &r.space, "no-commit", &format!("{}: the removal of {} would not build — {why}", m.name, whom.join(", ")));
            return next_space(h, c, r);
        }
    };
    let (Ok(info), Ok(secret), Ok(state)) = (g.group_info(), g.epoch_secret(), g.save()) else { return next_space(h, c, r) };
    let mv = Moved { from_epoch, from_secret, commit, info, epoch: g.epoch(), secret, state, removed: p.remove.len() };
    let log = epoch_log(&c.tail, &m.channel, &mv.from_secret);
    r.step = Step::Log { m: mv };
    keep(h, Some(&r));
    vec![Io::Get { id: log.id_bytes(), subscribe: false }]
}

/// THE GROUP HELD AT `epoch`: its secret kept for the member, the mandate's group this one; a page loads it.
fn held<H: Host>(h: &mut H, member: &[u8; 32], space: &[u8; 32], epoch: u64, secret: &[u8; 32], state: &[u8]) {
    identity::keep_epoch(h, member, Some(*space), epoch, secret);
    let Some((me, spaces)) = identity::upkeep_mandate(h, member) else { return };
    let spaces: Vec<Mandate> = spaces
        .into_iter()
        .map(|mut m| {
            if m.space == *space {
                m.epoch = epoch;
                m.state = state.to_vec();
            }
            m
        })
        .collect();
    identity::upkeep_set_mandate(h, member, &me, &spaces);
    identity::upkeep_set_moved(h, member, space, true);
}

/// EPOCH `epoch`'s GROUP INFO into its door, in parts (`{ k: "info", e, d, i, n, x }`, the page's `keys.publishInfo`),
/// and announced in the writers list (`{ w, epoch, branch, ih }`): sealed with the space's open key.
fn door_io<H: Host>(h: &mut H, c: &Held, member: &[u8; 32], space: &[u8; 32], epoch: u64, secret: &[u8; 32], info: &[u8]) -> Vec<Io> {
    use sha2::Digest;
    let ih = hex(&sha2::Sha256::digest(info));
    let x = hex(info);
    let n = x.len().div_ceil(PART);
    let name = format!("door-{epoch}");
    let address = crate::table::sealed_bag_address(space, &name);
    let mut items = Vec::new();
    for i in 0..n {
        let Some(r) = identity::upkeep_random(h) else { return Vec::new() };
        let part = &x[i * PART..((i + 1) * PART).min(x.len())];
        let item = crate::table::seal_open_item(space, &serde_json::json!({ "k": "info", "e": epoch, "d": ih, "i": i, "n": n, "x": part }), r[..12].try_into().expect("12 of 32"));
        items.push(craftworks_bag_contract::grind(&address, &item));
    }
    let mut io = vec![Io::Put { code: Code::Bag, params: address.to_vec(), state: craftworks_bag_contract::encode(&address, &items) }];
    let _ = c;
    if let Some(r) = identity::upkeep_random(h) {
        let branch = hex(&identity::epoch_log_key(secret).verifying_key().to_bytes());
        let writers = crate::table::sealed_bag_address(space, "writers");
        let at = identity::upkeep_now(h).unwrap_or(0) * 1000;
        let item = crate::table::seal_open_item(space, &serde_json::json!({ "w": hex(member), "epoch": epoch, "branch": branch, "ih": ih, "at": at, "by": "node" }), r[..12].try_into().expect("12 of 32"));
        io.push(Io::Put { code: Code::Bag, params: writers.to_vec(), state: craftworks_bag_contract::encode(&writers, &[craftworks_bag_contract::grind(&writers, &item)]) });
    }
    io
}

/// An epoch's LOG: a tail under the key its secret gives, named by the space's channel, sealed with the table key its
/// secret gives for that channel (as `keys` opens it on a page).
fn epoch_log(tail: &[u8], channel: &str, secret: &[u8; 32]) -> data::Open {
    let mut o = data::Open::new(tail, &identity::epoch_log_key(secret).verifying_key().to_bytes(), channel);
    o.set_table_key(identity::epoch_table_key(secret, channel));
    o
}

/// One row written and signed with `key`: what to send.
fn signed(o: &mut data::Open, key: &ed25519_dalek::SigningKey, k: &[u8], v: &[u8]) -> Option<data::Send> {
    use ed25519_dalek::Signer;
    let (seq, hash) = o.prepare_row(k, v)?;
    let p = craftec_register_contract::wire::Params::parse(&o.params)?;
    o.commit(key.sign(&p.signed_message(false, seq, &hash)).to_bytes())
}

fn send_io(o: &data::Open, send: data::Send, code: Code) -> Io {
    match send {
        data::Send::Put(state) => Io::Put { code, params: o.params.clone(), state },
        data::Send::Update(delta) => Io::Update { id: o.id_bytes(), delta },
    }
}

