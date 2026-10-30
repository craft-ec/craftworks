//! THE NODE'S FOLLOWER (FOUNDATION "Members and keys", the design of 10-17; K5) — a space is kept to its LIST here, by
//! the account's node, on every wake-up (and whenever a page asks it to run now), page open or not: its BeeKEM tree
//! (`keys_round`) adds whom the list names and takes out whom it no longer does. A round, for one member on this node,
//! space by space (every kind: a server, a conversation, a level group, an audience, a circle):
//!
//!   1. READ the space's acts and key operations (`table::Reading`, every member's feeds) — the list (`gov`'s replay),
//!      whole or not. Nothing is decided from a short read (I6).
//!   2. RECONCILE the tree to the list (`keys_round::reconcile`), its operations written into this member's own feed.
//!
//! (No MLS here — K8.6: a space with no tree is said `not-migrated` and not followed.)
//!
//! Each step is a message out and an answer in; the round lives in the delegate's secrets. Pure: the whole round runs
//! in tests against a scripted network.

use craftworks_identity::hex;
use craftworks_identity::{self as identity, Host, Mandate};
use serde::{Deserialize, Serialize};

use crate::rekey_round::Held;

/// K5: a BeeKEM space kept to its list by the node (its own file; a child, so it reads the round as upkeep does).
#[path = "keys_round.rs"]
mod keys_round;
#[cfg(test)]
pub(crate) use keys_round::ALL_AS_FILES;
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Step {
    /// The space's acts being read.
    Acts,
    /// K5: rows being written into this member's own feed of `table` — a BeeKEM space's operations (`pub-keyops`), or
    /// the files written for them (`files`) — `rows` still to send; `waiting`: the contracts whose answers come first;
    /// `blocks`: those were a flush's blocks.
    Keys { table: String, rows: Vec<(Vec<u8>, Option<Vec<u8>>)>, waiting: Vec<[u8; 32]>, blocks: bool },
    /// K5: a key operation stored as a FILE being read (`file_io::FileRead`, part by part); `rest`: the files still to
    /// read `(key, reference)`.
    KeyFileRead { key: String, read: crate::file_io::FileRead, parts: u64, rest: Vec<(String, String)> },
    /// K5: a large key operation being stored as a FILE (`file_io::FileWrite`, the page's put); `left`: the large
    /// operations still to store; `rows`: the `pub-keyops` rows so far.
    KeyFileWrite { key: String, write: crate::file_io::FileWrite, left: Vec<String>, rows: Vec<(Vec<u8>, Option<Vec<u8>>)> },
    /// The space's `files` table being read, to list the files written (`Round::listing`) in it.
    ListFiles,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Round {
    member: [u8; 32],
    me: String,
    space: [u8; 32],
    /// The spaces still to visit after this one.
    rest: Vec<[u8; 32]>,
    reading: Reading,
    step: Step,
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
    /// Key-operation files that did not read this round (not asked again until the next).
    #[serde(default)]
    files_unread: Vec<String>,
    /// The files this round wrote, to list in the space's `files` table (`k/<id>` rows) once its operations are written.
    #[serde(default)]
    listing: Vec<(Vec<u8>, Option<Vec<u8>>)>,
}

pub const ROUND: &[u8] = b"identity_upkeep/round";
pub const STUCK: u64 = 3;
/// The member whose round ran last (the next round starts after it: each person in turn).
const LAST_SERVED: &[u8] = b"identity_upkeep/last-served";
/// A FOLLOWER PASS over every member, driven by CHANGE (10-30): a subscribed feed's notification marks the node CHANGED
/// and begins a pass at once (or another as the one running ends); a wake-up begins one only when something changed, or
/// as the deadline — `rekey_round::EVERY` wake-ups with none (a notification lost: a node restarted). The members this
/// pass served (`PASS_DONE`), the wake-up of the last pass (`PASS_AT`), and the mark.
const PASS_DONE: &[u8] = b"identity_upkeep/pass-done";
const PASS_AT: &[u8] = b"identity_upkeep/pass-at";
const CHANGED: &[u8] = b"identity_upkeep/changed";
/// A SUBSCRIBED contract changed (`ContractNotification`): the node is marked changed, and a pass begun now when none
/// is running.
pub fn notified<H: Host>(h: &mut H) -> Vec<Io> {
    h.set_secret(CHANGED, &[1]);
    if running(h) {
        return Vec::new();
    }
    new_pass(h)
}
#[cfg(test)]
pub(crate) fn tests_drop_round<H: Host>(h: &mut H) {
    keep(h, None);
}
fn new_pass<H: Host>(h: &mut H) -> Vec<Io> {
    h.set_secret(CHANGED, &[]);
    h.set_secret(PASS_DONE, &[]);
    h.set_secret(PASS_AT, &identity::upkeep_wakeups(h).to_le_bytes());
    start(h)
}

fn round<H: Host>(h: &H) -> Option<Round> {
    h.get_secret(ROUND).filter(|b| !b.is_empty()).and_then(|b| bincode::deserialize(&b).ok())
}
pub(crate) fn keep<H: Host>(h: &mut H, r: Option<&Round>) {
    h.set_secret(ROUND, &r.map(|r| bincode::serialize(r).expect("a round encodes")).unwrap_or_default());
}
fn mandate<H: Host>(h: &H, r: &Round) -> Option<Mandate> {
    identity::upkeep_mandate(h, &r.member)?.1.into_iter().find(|m| m.space == r.space)
}

/// THE ROUND's step over a space (`identity::Decided`: the `group` lifecycle's `follow-<step>` move, by the node — its
/// plan, its commit, an entry skipped, a stall, each with why). Never free text beside it.
pub(crate) fn followed<H: Host>(h: &mut H, member: &[u8; 32], space: &[u8; 32], step: &str, why: &str) {
    identity::upkeep_trace(h, &[format!("decided {step} [{}]: {why}", space[..6].iter().map(|b| format!("{b:02x}")).collect::<String>())]);
    let at = identity::upkeep_now_ms(h);
    identity::upkeep_decide(h, member, identity::Decided { seq: 0, space: *space, did: String::new(), q: [0; 16], at, step: step.into(), why: why.into() });
}

/// Whether a contract this delegate CREATES reaches the network. Yes on freenet 0.2.146 (measured 10-29, k90 on fx/fy/fz:
/// the node on fz re-keyed a file with every page closed — new pieces — and a page on fx read its 200 000 bytes whole).
/// Not on ≤ 0.2.141 (measured 10-05: stored on its own node only, `BROADCAST_NO_TARGETS`; freenet #5542) — there,
/// anything that makes one (a file's new pieces, a new feed or catalog, a flush's tree blocks) is left to a page.
pub const NEW_CONTRACTS_REACH_THE_NETWORK: bool = true;
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

/// A space's READERS: the epochs held here and whose devices are read (`account::reading_dids`, the page's too: the
/// list and the owner).
pub(crate) fn readers<H: Host>(h: &H, member: &[u8; 32], m: &Mandate) -> (std::collections::BTreeMap<u64, [u8; 32]>, Vec<[u8; 32]>) {
    let epochs = identity::epochs_below(h, member, m.space, u64::MAX).into_iter().collect();
    // THE LIST (K8.1), as last read whole here (never an MLS group: K8.6).
    // (Only accounts whose card names their member — `keys_round::known`: one never carded, or gone, is never read
    // round after round; fx 10-18, the per-round cap.)
    let known: Vec<String> = keys_round::known(h, member, &m.space).into_iter().map(|k| k.0).collect();
    let mut listed: Vec<String> = roster_kept(h, member, &m.space).into_iter().filter(|d| known.contains(d)).collect();
    // …and the accounts the writers bag NAMES that the list does not (K8.3, the page's rule — `storage.currentWriters`): a
    // self-joiner's `join` act is in ITS OWN feed, read before the list names it; its devices confirm it (only the devices
    // its key log and card name are read). (10-29, fz: B and C joined by a code; the node's list stayed the owner alone —
    // their acts never read — and it never placed them, took nobody out, re-keyed nothing.)
    for d in bag_named(h, member, &m.space) {
        if !listed.contains(&d) {
            listed.push(d);
        }
    }
    let dids = craftworks_account::reading_dids(&listed, Some(&m.owner)).iter().filter_map(|d| craftworks_account::did_bytes(d)).collect();
    (epochs, dids)
}

/// A space's round begun: its acts read, by its members' devices (the list's and its owner's).
fn begin<H: Host>(h: &mut H, c: &Held, member: [u8; 32], me: String, mut rest: Vec<[u8; 32]>) -> Vec<Io> {
    while !rest.is_empty() {
        let space = rest.remove(0);
        let Some(m) = identity::upkeep_mandate(h, &member).and_then(|(_, ms)| ms.into_iter().find(|m| m.space == space)) else { continue };
        let (epochs, dids) = readers(h, &member, &m);
        let (reading, io) = members_read(h, &member, &c.codes(), space, epochs, &dids, &["acts", "pub-acts", "pub-keyops"]);
        let r = Round { member, me, space, rest, reading, step: Step::Acts, moved_ms: identity::upkeep_now_ms(h), moved: identity::upkeep_wakeups(h), by_list: false, files_unread: Vec::new(), listing: Vec::new() };
        keep(h, Some(&r));
        return io;
    }
    keep(h, None);
    Vec::new()
}
pub(crate) fn next_space<H: Host>(h: &mut H, c: &Held, r: Round) -> Vec<Io> {
    let io = begin(h, c, r.member, r.me, r.rest);
    // (This member's spaces done: the NEXT member's round now, on this answer — every member a pass, never one a
    // wake-up; 10-30, fz: a member's kick waited ~20 min behind the node's other members, one a minute.) The pass over
    // and the node CHANGED meanwhile: another pass at once.
    if io.is_empty() && round(h).is_none() {
        let more = start(h);
        if more.is_empty() && h.get_secret(CHANGED).is_some_and(|b| b == [1]) {
            return new_pass(h);
        }
        return more;
    }
    io
}

/// The space's list as its acts say (`gov`'s replay, the page's too). Read WHOLE, it is KEPT (`roster_kept`): whose
/// devices the next round reads (K8.1 — the list, not the MLS group, is who is in a space).
pub(crate) fn governance<H: Host>(h: &mut H, r: &Round, now_ms: u64) -> Option<craftworks_gov::Gov> {
    let m = mandate(h, r)?;
    let gov = crate::rekey::governance(&r.reading, &r.reading.writer_dids, &m.owner, now_ms);
    // (The bag's named accounts the list does not hold yet: the next round reads them — see `readers`.)
    if !r.reading.bag_dids.is_empty() {
        let named: Vec<&String> = r.reading.bag_dids.iter().filter(|d| !gov.roster.contains(*d)).collect();
        let _ = h.set_secret(&[BAG_NAMED, &r.member[..], &r.space[..]].concat(), &serde_json::to_vec(&named).unwrap_or_default());
    }
    if whole(r, &gov).is_ok() {
        let list: Vec<&String> = gov.roster.iter().collect();
        let _ = h.set_secret(&roster_key(&r.member, &r.space), &serde_json::to_vec(&list).unwrap_or_default());
    }
    Some(gov)
}
/// A space's LIST as this node last read it whole (none yet: empty — its owner is read, and the list widens round by
/// round as its acts name more).
/// ABSENT, DECIDED (`Reading::absent`), per space: the listed tails the node answered NOT THERE, each with what listed
/// it — kept across rounds, so a contract nobody made is not asked again (a 60 s not-found that parks this delegate,
/// every request to it queued behind) until what lists it changes. The newest `ABSENT_KEPT`.
const ABSENT: &[u8] = b"identity_upkeep/absent/";
const ABSENT_KEPT: usize = 256;
pub(crate) fn absent_of<H: Host>(h: &H, space: &[u8; 32]) -> Vec<([u8; 32], [u8; 32])> {
    h.get_secret(&[ABSENT, &space[..]].concat()).and_then(|b| bincode::deserialize(&b).ok()).unwrap_or_default()
}
fn keep_absent<H: Host>(h: &mut H, space: &[u8; 32], absent: &[([u8; 32], [u8; 32])]) {
    let from = absent.len().saturating_sub(ABSENT_KEPT);
    let _ = h.set_secret(&[ABSENT, &space[..]].concat(), &bincode::serialize(&absent[from..]).unwrap_or_default());
}
/// A space's read of its members' tables (`Reading::of_members`), what it found absent before remembered.
pub(crate) fn members_read<H: Host>(h: &H, member: &[u8; 32], c: &crate::table::Codes, space: [u8; 32], epochs: std::collections::BTreeMap<u64, [u8; 32]>, dids: &[[u8; 32]], tables: &[&str]) -> (Reading, Vec<Io>) {
    let (mut reading, io) = Reading::of_members(c, space, epochs, dids, tables);
    reading.absent = absent_of(h, &space);
    // (The generation written with: the identity's writing mark — never the newest one held.)
    reading.writing = identity::latest_epoch(h, member, space);
    (reading, io)
}
/// An answer to a read (`Reading::got`); what it found absent kept.
pub(crate) fn read_got<H: Host>(h: &mut H, reading: &mut Reading, space: &[u8; 32], c: &crate::table::Codes, id: [u8; 32], state: Option<Vec<u8>>) -> Vec<Io> {
    let n = reading.absent.len();
    let io = reading.got(c, id, state);
    if reading.absent.len() != n {
        keep_absent(h, space, &reading.absent);
    }
    io
}
/// The accounts a space's writers bag NAMED at the last read that its list did not (`Reading::bag_dids`): read next round.
const BAG_NAMED: &[u8] = b"identity_upkeep/bag-named/";
pub(crate) fn bag_named<H: Host>(h: &H, member: &[u8; 32], space: &[u8; 32]) -> Vec<String> {
    h.get_secret(&[BAG_NAMED, &member[..], &space[..]].concat()).and_then(|v| serde_json::from_slice(&v).ok()).unwrap_or_default()
}
const ROSTER: &[u8] = b"identity_upkeep/roster/";
pub(crate) fn roster_key(member: &[u8; 32], space: &[u8; 32]) -> Vec<u8> {
    [ROSTER, &member[..], &space[..]].concat()
}
pub(crate) fn roster_kept<H: Host>(h: &H, member: &[u8; 32], space: &[u8; 32]) -> Vec<String> {
    h.get_secret(&roster_key(member, space)).and_then(|v| serde_json::from_slice(&v).ok()).unwrap_or_default()
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
            Step::Keys { .. } => "a key operation's write never answered".into(),
            Step::KeyFileRead { .. } => "a key operation's file never answered".into(),
            Step::KeyFileWrite { .. } => "a key operation's file was never stored".into(),
            Step::ListFiles => "the space's files never answered".into(),
        };
        followed(h, &r.member, &r.space, craftworks_rules::steps::STALLED, &format!("{}: the round stalled — {at}: dropped", hex(&r.space[..6])));
        keep(h, None);
    }
    // (Something changed, or the deadline: a pass. Else nothing — no sweep on the clock.)
    let w = identity::upkeep_wakeups(h);
    let last = h.get_secret(PASS_AT).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes);
    let changed = h.get_secret(CHANGED).is_some_and(|b| b == [1]);
    if changed || last.is_none_or(|l| w.saturating_sub(l) >= crate::rekey_round::EVERY) {
        return new_pass(h);
    }
    Vec::new()
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
/// A round RUNNING (not `stuck`): left to go on.
fn running<H: Host>(h: &H) -> bool {
    round(h).is_some_and(|r| !stuck(h, r.moved, r.moved_ms))
}
/// A ROUND STUCK — every round of the node's (this one, the re-key's, the reads'), one rule: not moved for `STUCK`
/// wake-ups, or for `STUCK_MS` of the node's time — by TIME too, never only by wake-ups (which may not come: I2).
/// `moved`: the wake-up count when it last moved; `moved_ms`: the node's time then.
pub fn stuck<H: Host>(h: &H, moved: u64, moved_ms: u64) -> bool {
    let now = identity::upkeep_now_ms(h);
    identity::upkeep_wakeups(h).saturating_sub(moved) >= STUCK || now.saturating_sub(moved_ms) >= STUCK_MS
}
/// A round not moved for this long (the node's time) is stalled.
const STUCK_MS: u64 = 3 * 60 * 1000;

/// The next member's round (each person in turn), over every space of theirs — each member ONCE a pass (one served in
/// this pass already is passed over: `next_space` goes on to the next as each finishes).
fn start<H: Host>(h: &mut H) -> Vec<Io> {
    let Some(c) = Held::of(h) else { return Vec::new() };
    let w = identity::upkeep_wakeups(h);
    let members = identity::upkeep_members(h);
    let mut done: Vec<[u8; 32]> = h.get_secret(PASS_DONE).and_then(|b| bincode::deserialize(&b).ok()).unwrap_or_default();
    let last = h.get_secret(LAST_SERVED).and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok());
    let from = last.and_then(|l| members.iter().position(|m| *m == l)).map_or(0, |i| i + 1);
    for member in members.iter().cycle().skip(from).take(members.len()).copied() {
        if done.contains(&member) {
            continue;
        }
        done.push(member);
        h.set_secret(PASS_DONE, &bincode::serialize(&done).expect("ids encode"));
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
    let Some(mut r) = round(h) else { return Vec::new() };
    let Some(c) = Held::of(h) else { return Vec::new() };
    let Some(m) = mandate(h, &r) else {
        keep(h, None);
        return Vec::new();
    };
    r.moved = identity::upkeep_wakeups(h);
    r.moved_ms = now_ms;
    let member = r.member;
    match (r.step.clone(), reply) {
        (Step::Acts, Reply::Got { id, state }) => {
            if !r.reading.wants(&id) {
                return Vec::new();
            }
            let io = read_got(h, &mut r.reading, &r.space, &c.codes(), id, state);
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
                // WHOSE CARDS (`gov::check_dids`, the page's too): the listed, the owner, everyone an act moved — each read
                // again by `members::card_due` (the page's too).
                let mut want: Vec<String> = craftworks_gov::check_dids(&gov.roster.iter().cloned().collect::<Vec<_>>(), &m.owner, &gov.counted).into_iter().filter(|d| !r.reading.beekem.iter().any(|(k, ..)| k == d) && keys_round::card_due(&known, &asked, d, now_ms)).collect();
                // (Never read first, then the longest ago: a re-read never holds back a new member.)
                want.sort_by_key(|d| known.iter().find(|k| k.0 == *d).map(|k| k.3).or_else(|| asked.iter().find(|(x, _)| x == d).map(|(_, t)| *t)).map_or(0, |t| t + 1));
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
                followed(h, &member, &r.space, craftworks_rules::steps::KEYS_OPS_UNREAD, &format!("{}: its key operations read empty though it holds a BeeKEM generation ({notes}): not followed this round", m.name));
                return next_space(h, &c, r);
            }
            // NO TREE AT ALL: an MLS-era space, not migrated — not followed (no MLS catch-up: K8.6); the page hides it.
            followed(h, &member, &r.space, craftworks_rules::steps::NOT_MIGRATED, &format!("{}: no BeeKEM tree (not migrated): not followed", m.name));
            next_space(h, &c, r)
        }
        (Step::Keys { table, rows, mut waiting, blocks }, Reply::Put { id, ok } | Reply::Updated { id, ok }) if waiting.contains(&id) => {
            waiting.retain(|w| *w != id);
            if !ok {
                followed(h, &member, &r.space, craftworks_rules::steps::KEYS_REFUSED, &format!("{}: a key operation's write was refused: the next round reads again", m.name));
                return next_space(h, &c, r);
            }
            if !waiting.is_empty() {
                r.step = Step::Keys { table, rows, waiting, blocks };
                keep(h, Some(&r));
                return Vec::new();
            }
            if !rows.is_empty() {
                return keys_round::write(h, &c, r, &m, &table, rows, blocks);
            }
            // Its operations written: the files written for them listed in the space's `files` table (as a page's put).
            if table == "pub-keyops" && !r.listing.is_empty() {
                return keys_round::list_files(h, &c, r, &m);
            }
            next_space(h, &c, r)
        }
        (Step::KeyFileRead { .. }, Reply::Got { id, state }) => keys_round::file_got(h, &c, r, &m, id, state, now_ms),
        (Step::KeyFileWrite { .. }, Reply::Put { id, ok } | Reply::Updated { id, ok }) => keys_round::file_put(h, &c, r, &m, id, ok, now_ms),
        (Step::ListFiles, Reply::Got { id, state }) => {
            if !r.reading.wants(&id) {
                return Vec::new();
            }
            let io = read_got(h, &mut r.reading, &r.space, &c.codes(), id, state);
            if !r.reading.done() {
                keep(h, Some(&r));
                return io;
            }
            let rows = std::mem::take(&mut r.listing);
            keys_round::write(h, &c, r, &m, "files", rows, false)
        }
        // R2 (FOUNDATION "The node's rounds"): an answer this step did not ask for — late, duplicated, the wrong kind —
        // moves nothing and is kept nowhere. Every (state, answer) cell is named: the table is the compiler's to check.
        (Step::Acts | Step::KeyFileRead { .. } | Step::ListFiles, Reply::Put { .. } | Reply::Updated { .. })
        | (Step::Keys { .. } | Step::KeyFileWrite { .. }, Reply::Got { .. })
        | (Step::Keys { .. }, Reply::Put { .. } | Reply::Updated { .. }) => Vec::new(),
    }
}

