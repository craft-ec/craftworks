//! THE NODE'S FOLLOWER (FOUNDATION "Members and keys") — the ONE place a space's keys follow its member list. The
//! account's node runs it on every wake-up (and whenever a page asks it to run now), page open or not: no page gathers,
//! decides or commits for membership. A round, for one member on this node, space by space:
//!
//!   1. READ the space's acts (`table::Reading`, every member's feeds) — the member list (`gov`'s replay): who is listed,
//!      the codes in force, the open door, a level, the bans, who may let people in.
//!   2. READ its bags: each code's, the open door's, a level's (people asking to join), the member bag (`key <space>`:
//!      a listed device's "my key"), and the writers list (the welcomes made: each key answered once).
//!   3. DECIDE: a join by `rules::admission` (its own key, once, one admitter by turn); then `rules::members::follow`
//!      over the list, the group's entries and the keys posted — one commit's plan: who out, which keys in.
//!   4. CHECK each key in: their key log (the key is theirs) and card (the inbox; a level group's level).
//!   5. WRITE an `admitted` act for each join (`Gov::stamp`, `write::rows`): the list first, then the keys follow it.
//!   6. ONE COMMIT (`Mls::follow`) into its epoch's log, the next epoch's log made, the one welcome sealed to each new
//!      entry's inbox, each answered in the writers list.
//!
//! A read that came back SHORT (a feed silent, an act named and not held) takes nobody out: only a whole read of the
//! list may say someone left it. Each step is a message out and an answer in; the round lives in the delegate's secrets.
//! Pure: the whole round runs in tests against a scripted network.

use craftworks_data as data;
use craftworks_identity::{self as identity, Admitted, Host, Mandate};
use craftworks_rules::admission::{decide, Decision, Entry, Request, View, Welcomed};
use craftworks_rules::members;
use serde::{Deserialize, Serialize};

use crate::rekey_round::Held;
use crate::table::Reading;
use crate::write::{self, Next};

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

/// A key to put in: a listed device's own ("my key": `code` "key"), or a join's (`code`: the code, "open", "level").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Ask {
    space: [u8; 32],
    did: String,
    did_bytes: [u8; 32],
    code: String,
    /// The key package (hex): the one its entry is made with.
    kp: String,
}

/// THE COMMIT made (`Mls::follow`), carried until its log, the next epoch's log and the welcomes are sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Add {
    from_epoch: u64,
    from_secret: [u8; 32],
    commit: Vec<u8>,
    welcome: Option<Vec<u8>>,
    info: Vec<u8>,
    epoch: u64,
    secret: [u8; 32],
    state: Vec<u8>,
    removed: usize,
    /// Each new entry: its ask and the inbox key its welcome is sealed to.
    to: Vec<(Ask, [u8; 32])>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Step {
    /// The space's acts being read.
    Acts,
    /// Its bags: `(id, code)` not answered yet.
    Bags { waiting: Vec<([u8; 32], String)> },
    KeyLog { ask: Ask },
    /// Their card, at the address their key log's data key gives…
    Card { ask: Ask, data: [u8; 32] },
    /// …and its tree BLOCKS: `want` (contract id, block) not answered yet, `got` those that were.
    CardBlocks { ask: Ask, data: [u8; 32], card: Vec<u8>, want: Vec<([u8; 32], [u8; 32])>, got: Vec<([u8; 32], Vec<u8>)>, depth: u8 },
    /// `admitted` acts being written into `name`: `rows` still to write after this step, `waiting` its answers.
    Writing { name: String, rows: Vec<(Vec<u8>, Option<Vec<u8>>)>, waiting: Vec<[u8; 32]>, refused: bool, blocks: bool },
    Log { add: Add },
    Commit { add: Add },
    Next { add: Add },
    /// The welcomes sent: `waiting` the inboxes not answered yet.
    Welcome { add: Add, waiting: Vec<[u8; 32]>, tries: u8 },
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
    /// The writers list: the welcomes any member made here `(did, kp tag, at)`.
    heard: Vec<(String, [u8; 16], u64)>,
    /// The bags' items `(code, text)`, decided once every bag is read.
    items: Vec<(String, String)>,
    /// The plan: entries out; keys in still to check; keys checked (with their inbox).
    remove: Vec<u32>,
    check: Vec<Ask>,
    ready: Vec<(Ask, [u8; 32])>,
    /// A level group's level (its join policy), for a level join's card.
    level: Option<String>,
    /// The wake-up count when the round last moved: one stuck for `STUCK` wake-ups is dropped (or, sending welcomes,
    /// sent again).
    moved: u64,
}

pub const ROUND: &[u8] = b"identity_upkeep/round";
/// A page that handed the mandate over this recently is running (the re-key round stays out of its way; the follower
/// does not: it is the only one).
pub const PAGE_AWAY: u64 = 2;
pub const STUCK: u64 = 3;
/// How long the first in turn has before the next one acts (`rules::admission`, `rules::members`).
const TAKEOVER_MS: u64 = 120_000;
/// The member whose round ran last (the next round starts after it: each person in turn).
const LAST_SERVED: &[u8] = b"identity_upkeep/last-served";
/// The round's read of a space's writers list (beside its bags).
const WRITERS: &str = "writers";
/// The member bag: a listed device's "my key".
const KEY: &str = "key";

fn wakeups<H: Host>(h: &H) -> u64 {
    h.get_secret(identity::UPKEEP_WAKEUPS).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes).unwrap_or(0)
}
fn round<H: Host>(h: &H) -> Option<Round> {
    h.get_secret(ROUND).filter(|b| !b.is_empty()).and_then(|b| bincode::deserialize(&b).ok())
}
fn keep<H: Host>(h: &mut H, r: Option<&Round>) {
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

/// A key's step taken here (`identity::Decided`: the `member` lifecycle's move, by the node).
fn decided<H: Host>(h: &mut H, member: &[u8; 32], ask: &Ask, step: &str, why: &str, now_ms: u64) {
    let q = craftworks_rules::admission::tag(&ask.kp);
    identity::upkeep_decide(h, member, identity::Decided { space: ask.space, did: ask.did.clone(), q, at: now_ms, step: step.into(), why: why.into() });
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

/// The group's ENTRIES `(index, did)` (each credential's DID).
fn leaves(g: &craftworks_mls::Account) -> Vec<members::Leaf> {
    g.members().into_iter().filter_map(|(index, _, cred)| Some(members::Leaf { index, did: craftworks_account::did(&identity::read_credential(&cred)?.0) })).collect()
}

/// A space's round begun: its acts read, by its members' devices (the group's entries and its owner).
fn begin<H: Host>(h: &mut H, c: &Held, member: [u8; 32], me: String, mut rest: Vec<[u8; 32]>) -> Vec<Io> {
    while !rest.is_empty() {
        let space = rest.remove(0);
        let Some(m) = identity::upkeep_mandate(h, &member).and_then(|(_, ms)| ms.into_iter().find(|m| m.space == space)) else { continue };
        let Some((_, epochs)) = crate::rekey_round::space_of(h, &member, &m) else {
            identity::upkeep_say(h, &member, &format!("{}: its group does not load here: not followed", m.name));
            continue;
        };
        let Ok(g) = craftworks_mls::Account::load(craftworks_mls::Rule::Space(space), &m.state) else { continue };
        let mut dids: Vec<[u8; 32]> = leaves(&g).iter().filter_map(|l| craftworks_account::did_bytes(&l.did)).collect();
        if let Some(o) = craftworks_account::did_bytes(&m.owner) {
            if !dids.contains(&o) {
                dids.push(o);
            }
        }
        let (reading, io) = Reading::of_members(&c.codes(), space, epochs, &dids, &["acts", "pub-acts"]);
        let r = Round { member, me, space, rest, reading, step: Step::Acts, heard: Vec::new(), items: Vec::new(), remove: Vec::new(), check: Vec::new(), ready: Vec::new(), level: None, moved: wakeups(h) };
        keep(h, Some(&r));
        return io;
    }
    keep(h, None);
    Vec::new()
}
fn next_space<H: Host>(h: &mut H, c: &Held, r: Round) -> Vec<Io> {
    begin(h, c, r.member, r.me, r.rest)
}

/// The space's list as its acts say (`gov`'s replay, the page's too).
fn governance<H: Host>(h: &H, r: &Round, now_ms: u64) -> Option<craftworks_gov::Gov> {
    let m = mandate(h, r)?;
    Some(crate::rekey::governance(&r.reading, &r.reading.writer_dids, &m.owner, now_ms))
}

/// A WAKE-UP: a stuck round's welcomes sent again (or the round dropped); else, none running, the next member's round
/// begun (each person in turn). The GETs to send.
pub fn woke<H: Host>(h: &mut H, now_ms: u64) -> Vec<Io> {
    let _ = now_ms;
    let w = wakeups(h);
    if let Some(mut r) = round(h) {
        if w.saturating_sub(r.moved) < STUCK {
            return Vec::new();
        }
        // Stuck. Welcomes not taken are sent again (the entries are in the group already: they must hear of it).
        if let Step::Welcome { add, waiting, tries } = r.step.clone() {
            if tries < 5 {
                let to: Vec<(Ask, [u8; 32])> = add.to.iter().filter(|(a, _)| waiting.contains(&inbox_id(h, a))).cloned().collect();
                let io: Vec<Io> = to.iter().flat_map(|(a, inbox)| welcome_io(h, &r.member, &r.me, a, *inbox, &add)).collect();
                r.step = Step::Welcome { add, waiting, tries: tries + 1 };
                r.moved = w;
                keep(h, Some(&r));
                return io;
            }
        }
        let at = match &r.step {
            Step::Acts => "the space's acts never answered whole".to_string(),
            Step::Bags { waiting } => format!("{} bag(s) never answered", waiting.len()),
            Step::KeyLog { .. } => "a key log never answered".into(),
            Step::Card { .. } | Step::CardBlocks { .. } => "a card never answered".into(),
            Step::Writing { .. } => "the admitted acts were never answered".into(),
            Step::Log { .. } => "the epoch's log never answered".into(),
            Step::Commit { .. } => "the commit was never answered".into(),
            Step::Next { .. } => "the next epoch's log never answered".into(),
            Step::Welcome { .. } => "the welcomes were not taken in 5 tries".into(),
        };
        identity::upkeep_say(h, &r.member, &format!("{}: the round stalled — {at}: dropped", hex(&r.space[..6])));
        keep(h, None);
    }
    start(h)
}

/// The next member's round (each person in turn), over their spaces whose list is the acts (a server's).
fn start<H: Host>(h: &mut H) -> Vec<Io> {
    let Some(c) = Held::of(h) else { return Vec::new() };
    let w = wakeups(h);
    let members = identity::upkeep_members(h);
    let last = h.get_secret(LAST_SERVED).and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok());
    let from = last.and_then(|l| members.iter().position(|m| *m == l)).map_or(0, |i| i + 1);
    for member in members.iter().cycle().skip(from).take(members.len()).copied() {
        let Some((me, spaces)) = identity::upkeep_mandate(h, &member) else { continue };
        let all: Vec<[u8; 32]> = spaces.iter().filter(|m| m.kind == "server").map(|m| m.space).collect();
        if all.is_empty() {
            continue;
        }
        h.set_secret(LAST_SERVED, &member);
        identity::upkeep_set_round_at(h, &member, w);
        let io = begin(h, &c, member, me, all);
        if !io.is_empty() {
            return io;
        }
    }
    Vec::new()
}

/// The contract a step waits on (the reading's and the bags' matched there).
fn waits_for(r: &Round, c: &Held, channel: Option<&str>) -> Option<[u8; 32]> {
    match &r.step {
        Step::Acts | Step::Bags { .. } | Step::CardBlocks { .. } | Step::Writing { .. } | Step::Welcome { .. } => None,
        Step::KeyLog { ask } => Some(id_of(&c.idlog, &ask.did_bytes)),
        Step::Card { data, .. } => Some(data::Open::new(&c.tail, data, "card").id_bytes()),
        Step::Log { add } | Step::Commit { add } => Some(epoch_log(&c.tail, channel?, &add.from_secret).id_bytes()),
        Step::Next { add } => Some(epoch_log(&c.tail, channel?, &add.secret).id_bytes()),
    }
}

/// An ANSWER from the network: the round moved on. What to send next.
pub fn replied<H: Host>(h: &mut H, reply: Reply, now_ms: u64) -> Vec<Io> {
    // An answer the RE-KEY round waits on is its own (both rounds run side by side).
    let id = match &reply {
        Reply::Got { id, .. } | Reply::Put { id, .. } | Reply::Updated { id, .. } => *id,
    };
    if crate::rekey_round::wants(h, &id) {
        return crate::rekey_round::replied(h, reply, now_ms);
    }
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
    r.moved = wakeups(h);
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
            bags(h, &c, r, now_ms)
        }
        (Step::Bags { mut waiting }, Reply::Got { id, state }) => {
            let Some(i) = waiting.iter().position(|(w, _)| *w == id) else { return Vec::new() };
            let (_, code) = waiting.remove(i);
            if let Some(st) = state {
                if code == WRITERS {
                    let address = crate::table::sealed_bag_address(&r.space, "writers");
                    for p in craftworks_bag_contract::read(&address, &st).unwrap_or_default() {
                        let Some(v) = crate::table::open_bag_item(&r.space, &Default::default(), "writers", craftworks_bag_contract::payload(&p)) else { continue };
                        let (Some(did), Some(kp)) = (v["welcomed"].as_str(), v["kp"].as_str().and_then(unhex).and_then(|t| <[u8; 16]>::try_from(t).ok())) else { continue };
                        r.heard.push((did.to_string(), kp, v["at"].as_u64().unwrap_or(0)));
                    }
                } else {
                    for p in craftworks_bag_contract::read(&identity::invite_address(&bag_name(&code, &r.space)), &st).unwrap_or_default() {
                        r.items.push((code.clone(), String::from_utf8_lossy(craftworks_bag_contract::payload(&p)).into_owned()));
                    }
                }
            } else if code == WRITERS {
                // Without the writers list a key could be answered twice: this space waits for the next round.
                identity::upkeep_say(h, &member, &format!("{}: its writers list did not answer: not followed this round", m.name));
                return next_space(h, &c, r);
            }
            if !waiting.is_empty() {
                r.step = Step::Bags { waiting };
                keep(h, Some(&r));
                return Vec::new();
            }
            plan(h, &c, r, &m, now_ms)
        }
        (Step::KeyLog { ask }, Reply::Got { state, .. }) => {
            let size = state.as_ref().map(Vec::len);
            let Some(log) = state.and_then(|st| craftworks_idlog_contract::read(&ask.did_bytes, &st)) else {
                let why = size.map_or("the network answered none".to_string(), |n| format!("its state ({n} bytes) did not read"));
                decided(h, &member, &ask, "fail", &format!("no key log — {why}"), now_ms);
                return next_check(h, &c, r, now_ms);
            };
            // THE KEY IS THEIRS: its credential names this DID under the key its log holds now (a bag item is an
            // unsigned drop: a key made by anyone else would let that one in under their name).
            let theirs = unhex(&ask.kp).and_then(|kp| craftworks_mls::key_package_credential(&kp).ok()).is_some_and(|cred| craftworks_account::space_member(&ask.did_bytes, &log, &cred));
            if !theirs {
                decided(h, &member, &ask, "skip", "the key is not theirs (another account's, or one their log moved past)", now_ms);
                return next_check(h, &c, r, now_ms);
            }
            let data = log.head().data;
            let card = data::Open::new(&c.tail, &data, "card");
            r.step = Step::Card { ask, data };
            keep(h, Some(&r));
            vec![Io::Get { id: card.id_bytes(), subscribe: false }]
        }
        (Step::Card { ask, data }, Reply::Got { state, .. }) => {
            let Some(card) = state else {
                decided(h, &member, &ask, "fail", "they have no card (the network answered none)", now_ms);
                return next_check(h, &c, r, now_ms);
            };
            card_read(h, &c, r, ask, data, card, Vec::new(), 0, now_ms)
        }
        (Step::CardBlocks { ask, data, card, mut want, mut got, depth }, Reply::Got { id, state }) => {
            let Some(i) = want.iter().position(|(w, _)| *w == id) else { return Vec::new() };
            let (_, cid) = want.remove(i);
            got.push((cid, state.unwrap_or_default()));
            if !want.is_empty() {
                r.step = Step::CardBlocks { ask, data, card, want, got, depth };
                keep(h, Some(&r));
                return Vec::new();
            }
            card_read(h, &c, r, ask, data, card, got, depth + 1, now_ms)
        }
        (Step::Writing { name, rows, mut waiting, refused, blocks }, Reply::Put { id, ok } | Reply::Updated { id, ok }) if waiting.contains(&id) => {
            waiting.retain(|w| *w != id);
            let refused = refused || !ok;
            if !waiting.is_empty() {
                r.step = Step::Writing { name, rows, waiting, refused, blocks };
                keep(h, Some(&r));
                return Vec::new();
            }
            if refused {
                // The list was not written: no key of theirs goes in (the keys follow the list, never ahead of it).
                identity::upkeep_say(h, &member, &format!("{}: the admitted act(s) were refused (the network holds more): the next round reads again", m.name));
                return next_space(h, &c, r);
            }
            if !rows.is_empty() {
                return write_step(h, &c, r, &name, rows, blocks, now_ms);
            }
            commit(h, &c, r, &m, now_ms)
        }
        (Step::Log { add }, Reply::Got { state, .. }) => {
            let mut log = epoch_log(&c.tail, &m.channel, &add.from_secret);
            if let Some(st) = &state {
                log.absorb(st);
            }
            // THE LOG's ONE ROW RULE (`rules::ordering`, the pages' too): taken if any writer's row holds the position.
            use craftworks_rules::ordering;
            let taken = matches!(log.opened(), Ok(data::Step::Ready(rows)) if ordering::entry_at(rows.iter().map(|(k, v)| (k.as_slice(), v.as_slice())), "c/", add.from_epoch).is_some());
            if taken {
                // Another member moved the group from this epoch first: this node's group is behind — a page brings it
                // forward (`moved` by the page's mandate); nothing is sent.
                for (a, _) in &add.to {
                    decided(h, &member, a, "fail", &format!("the group moved from epoch {} meanwhile", add.from_epoch), now_ms);
                }
                identity::upkeep_say(h, &member, &format!("{}: the group moved from epoch {} meanwhile: this node's group is behind", m.name, add.from_epoch));
                return next_space(h, &c, r);
            }
            let at = ordering::row_key("c/", add.from_epoch, &hex(&member[..6]));
            // Its PROVENANCE as a page's rows carry it (`keys.provOf`): who moved the group, why, when.
            let why = format!("follow (node): {} in, {} out", add.to.len(), add.removed);
            let entry = serde_json::json!({ "commit": hex(&add.commit), "info": hex(&add.info), "by": hex(&member[..8]), "why": why, "at": now_ms, "v": "node" }).to_string();
            let key = identity::epoch_log_key(&add.from_secret);
            let Some(send) = signed(&mut log, &key, at.as_bytes(), entry.as_bytes()) else {
                identity::upkeep_say(h, &member, &format!("{}: the epoch's log would not take the commit", m.name));
                return next_space(h, &c, r);
            };
            let io = send_io(&log, send, Code::Tail);
            r.step = Step::Commit { add };
            keep(h, Some(&r));
            vec![io]
        }
        (Step::Commit { add }, Reply::Updated { ok, .. } | Reply::Put { ok, .. }) => {
            if !ok {
                identity::upkeep_say(h, &member, &format!("{}: the commit was refused: the group moved meanwhile", m.name));
                return next_space(h, &c, r);
            }
            // The group moved: held here now (its secret kept, the mandate's group this one).
            moved(h, &member, &r.space, &add);
            let mut next = epoch_log(&c.tail, &m.channel, &add.secret);
            let open = serde_json::json!({ "info": hex(&add.info), "prev": hex(&add.from_secret) }).to_string();
            let key = identity::epoch_log_key(&add.secret);
            let Some(send) = signed(&mut next, &key, b"open", open.as_bytes()) else {
                identity::upkeep_say(h, &member, &format!("{}: the next epoch's log would not take its first row", m.name));
                return next_space(h, &c, r);
            };
            let io = send_io(&next, send, Code::Tail);
            r.step = Step::Next { add };
            keep(h, Some(&r));
            vec![io]
        }
        (Step::Next { add }, Reply::Put { .. } | Reply::Updated { .. }) => {
            // Made or not (a node that fetched it first: the same row), the welcomes go: they are in the group.
            if add.to.is_empty() {
                identity::upkeep_say(h, &member, &format!("{}: {} entr(ies) out, epoch {}", m.name, add.removed, add.epoch));
                return next_space(h, &c, r);
            }
            let me = r.me.clone();
            let io: Vec<Io> = add.to.iter().flat_map(|(a, inbox)| welcome_io(h, &member, &me, a, *inbox, &add)).collect();
            let waiting = add.to.iter().map(|(a, _)| inbox_id(h, a)).collect();
            r.step = Step::Welcome { add, waiting, tries: 0 };
            keep(h, Some(&r));
            io
        }
        (Step::Welcome { add, mut waiting, tries }, Reply::Put { id, ok }) if waiting.contains(&id) => {
            if ok {
                waiting.retain(|w| *w != id);
            }
            if !waiting.is_empty() {
                // (Not taken: sent again at a later wake-up.)
                r.step = Step::Welcome { add, waiting, tries };
                keep(h, Some(&r));
                return Vec::new();
            }
            let mut all = identity::upkeep_admitted(h, &member);
            let mut io = Vec::new();
            for (a, _) in &add.to {
                all.push(Admitted { space: a.space, did: a.did.clone(), code: a.code.clone(), at: now_ms, epoch: add.epoch, kp: craftworks_rules::admission::tag(&a.kp) });
                decided(h, &member, a, "welcomed", &format!("epoch {}", add.epoch), now_ms);
                // THE GROUP'S SHARED LIST of welcomes made: every member's node reads it — this key answered.
                io.extend(answered_io(h, &member, a, now_ms));
            }
            identity::upkeep_set_admitted(h, &member, &all);
            identity::upkeep_say(h, &member, &format!("{}: {} in, {} out, epoch {}", m.name, add.to.len(), add.removed, add.epoch));
            io.extend(next_space(h, &c, r));
            io
        }
        // An answer this step does not wait for (a late one): nothing.
        _ => Vec::new(),
    }
}

/// THE BAGS: each code in force, the open door, a level's, the member bag, the writers list.
fn bags<H: Host>(h: &mut H, c: &Held, mut r: Round, now_ms: u64) -> Vec<Io> {
    let Some(g) = governance(h, &r, now_ms) else { return next_space(h, c, r) };
    let mut codes: Vec<String> = g.live_invites(now_ms as f64).iter().map(|i| i.code.clone()).collect();
    let join = g.policy_at("", "join", f64::INFINITY).map(str::to_string);
    if join.as_deref() == Some("anyone") {
        codes.push("open".into());
    }
    r.level = join.as_deref().and_then(craftworks_gov::level_of).map(str::to_string);
    if r.level.is_some() {
        codes.push("level".into());
    }
    codes.push(KEY.into());
    let mut waiting: Vec<([u8; 32], String)> = codes.into_iter().map(|code| (id_of(&c.bag, &identity::invite_address(&bag_name(&code, &r.space))), code)).collect();
    waiting.push((id_of(&c.bag, &crate::table::sealed_bag_address(&r.space, "writers")), WRITERS.into()));
    let io = waiting.iter().map(|(id, _)| Io::Get { id: *id, subscribe: true }).collect();
    r.step = Step::Bags { waiting };
    keep(h, Some(&r));
    io
}

/// THE PLAN: joins decided (`rules::admission`), then the keys follow the list (`rules::members::follow`).
fn plan<H: Host>(h: &mut H, c: &Held, mut r: Round, m: &Mandate, now_ms: u64) -> Vec<Io> {
    let Some(gov) = governance(h, &r, now_ms) else { return next_space(h, c, r) };
    let Ok(group) = craftworks_mls::Account::load(craftworks_mls::Rule::Space(r.space), &m.state) else { return next_space(h, c, r) };
    let entries = leaves(&group);
    let holds = |d: &str| entries.iter().any(|l| l.did == d);
    let listed: Vec<String> = gov.roster.iter().filter(|d| !gov.bans.contains(*d)).cloned().collect();
    let admitters: Vec<String> = listed.iter().filter(|d| gov.may(d, gov.role(d, true).as_deref(), "invite")).cloned().collect();
    let mut welcomed: Vec<Welcomed> = r.heard.iter().map(|(d, kp, at)| Welcomed { did: d.clone(), kp: *kp, at: *at }).collect();
    welcomed.extend(identity::upkeep_admitted(h, &r.member).into_iter().filter(|a| a.space == r.space).map(|a| Welcomed { did: a.did, kp: a.kp, at: a.at }));
    let spent = identity::upkeep_spent(h, &r.member);
    let mut keys: Vec<members::Key> = Vec::new();
    let mut joins: Vec<(String, String)> = Vec::new(); // (did, code)
    for (code, text) in std::mem::take(&mut r.items) {
        let Ok(q) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        let (Some(did), Some(kp)) = (q["did"].as_str(), q["kp"].as_str()) else { continue };
        let at = q["at"].as_u64().unwrap_or(0);
        if did == r.me || gov.bans.contains(did) {
            continue;
        }
        if code == KEY {
            // "MY KEY": a listed device's own (one not listed is a join's to ask).
            if listed.iter().any(|x| x == did) {
                keys.push(members::Key { did: did.into(), kp: kp.into(), at });
            }
            continue;
        }
        if listed.iter().any(|x| x == did) || joins.iter().any(|(d, _)| d == did) {
            continue;
        }
        // A JOIN: THE ONE RULE (`rules::admission`): its own key, once, one admitter by turn.
        let request = Request { did, kp: Some(kp), at, entry: Entry::Join };
        let view = View { me: &r.me, admitters: &admitters, spent: &spent, welcomed: &welcomed, held: holds(did), epoch: m.epoch, now: now_ms, takeover_ms: TAKEOVER_MS };
        match decide(&request, &view) {
            Decision::Admit { kp, .. } => {
                keys.push(members::Key { did: did.into(), kp, at });
                joins.push((did.into(), code));
            }
            Decision::Skip(why) => {
                identity::upkeep_decide(h, &r.member, identity::Decided { space: r.space, did: did.into(), q: craftworks_rules::admission::tag(kp), at: now_ms, step: "skip".into(), why });
            }
        }
    }
    // A SHORT READ takes nobody out: every entry the group holds counts as listed until the whole list is read.
    let have: std::collections::BTreeSet<String> = r.reading.rows("acts").keys().chain(r.reading.rows("pub-acts").keys()).map(|k| String::from_utf8_lossy(k).into_owned()).collect();
    let whole = r.reading.notes.is_empty() && gov.missing(&have).is_empty();
    let mut list: Vec<String> = listed.clone();
    list.extend(joins.iter().map(|(d, _)| d.clone()));
    if !whole {
        list.extend(entries.iter().map(|l| l.did.clone()));
    }
    let followers: Vec<String> = listed.iter().filter(|d| holds(d)).cloned().collect();
    let p = members::follow(&members::View {
        me: &r.me,
        listed: &list,
        leaves: &entries,
        keys: &keys,
        welcomed: &welcomed,
        spent: &spent,
        followers: &followers,
        epoch: m.epoch,
        now: now_ms,
        takeover_ms: TAKEOVER_MS,
    });
    // WHAT IT FOUND, SAID — always (a round that does nothing is otherwise silent).
    identity::upkeep_say(
        h,
        &r.member,
        &format!(
            "{}: {} listed, {} entr(ies), {} key(s) posted, {} join(s) let in; plan {} in, {} out{}{}",
            m.name,
            listed.len(),
            entries.len(),
            keys.len() - joins.len(),
            joins.len(),
            p.add.len(),
            p.remove.len(),
            if whole { String::new() } else { format!(" (the list read SHORT: nobody out — {})", r.reading.notes.join("; ")) },
            if p.skipped.is_empty() { String::new() } else { format!("; skipped: {}", p.skipped.iter().map(|(d, w)| format!("{}… {w}", &d[d.len().saturating_sub(6)..])).collect::<Vec<_>>().join(", ")) }
        ),
    );
    if p.is_empty() {
        return next_space(h, c, r);
    }
    r.remove = p.remove;
    r.check = p
        .add
        .into_iter()
        .filter_map(|(did, kp, _)| {
            let code = joins.iter().find(|(d, _)| *d == did).map(|(_, c)| c.clone()).unwrap_or_else(|| KEY.into());
            Some(Ask { space: r.space, did_bytes: craftworks_account::did_bytes(&did)?, did, code, kp })
        })
        .collect();
    r.ready = Vec::new();
    next_check(h, c, r, now_ms)
}

/// The next key to check (their key log first); all checked: the joins' acts written, then the commit.
fn next_check<H: Host>(h: &mut H, c: &Held, mut r: Round, now_ms: u64) -> Vec<Io> {
    if r.check.is_empty() {
        return admit(h, c, r, now_ms);
    }
    let ask = r.check.remove(0);
    let id = id_of(&c.idlog, &ask.did_bytes);
    r.step = Step::KeyLog { ask };
    keep(h, Some(&r));
    vec![Io::Get { id, subscribe: false }]
}

/// THE LIST FIRST: an `admitted` act for each join checked (`Gov::stamp`), into the acts a reader of the space reads.
fn admit<H: Host>(h: &mut H, c: &Held, r: Round, now_ms: u64) -> Vec<Io> {
    let Some(m) = mandate(h, &r) else { return next_space(h, c, r) };
    let joins: Vec<&Ask> = r.ready.iter().map(|(a, _)| a).filter(|a| a.code != KEY).collect();
    if joins.is_empty() {
        return commit(h, c, r, &m, now_ms);
    }
    let Some(gov) = governance(h, &r, now_ms) else { return next_space(h, c, r) };
    let mut rows = Vec::new();
    for a in joins {
        let Some(id) = identity::upkeep_random(h) else { return next_space(h, c, r) };
        let act = gov.stamp(serde_json::json!({ "act": "admitted", "did": a.did, "code": a.code }), now_ms as f64);
        rows.push((hex(&id[..8]).into_bytes(), Some(act.to_string().into_bytes())));
    }
    // A public space's acts are public (readers outside must know them); a private one's sealed.
    let name = if gov.is_public() { "pub-acts" } else { "acts" };
    write_step(h, c, r, name, rows, false, now_ms)
}

/// One step of a write (listing first when the feed is new, a flush when due: then the rows again once answered).
fn write_step<H: Host>(h: &mut H, c: &Held, mut r: Round, name: &str, rows: Vec<(Vec<u8>, Option<Vec<u8>>)>, blocks_put: bool, _now_ms: u64) -> Vec<Io> {
    let nonce: [u8; 12] = identity::upkeep_random(h).map(|b| b[..12].try_into().expect("12")).unwrap_or([0; 12]);
    match write::rows(h, &r.member, &c.codes(), &mut r.reading, name, &rows, nonce, blocks_put, creates()) {
        Ok(Next::Done(sent)) => {
            r.step = Step::Writing { name: name.into(), rows: Vec::new(), waiting: sent.confirm, refused: false, blocks: false };
            keep(h, Some(&r));
            sent.io
        }
        Ok(Next::More(sent)) => {
            r.step = Step::Writing { name: name.into(), rows, waiting: sent.confirm, refused: false, blocks: sent.blocks };
            keep(h, Some(&r));
            sent.io
        }
        Err(why) => {
            identity::upkeep_say(h, &r.member, &format!("the admitted act(s) not written: {why}"));
            next_space(h, c, r)
        }
    }
}

/// ONE COMMIT (`Mls::follow`): the plan's entries out and every checked key in; its epoch's log read first.
fn commit<H: Host>(h: &mut H, c: &Held, mut r: Round, m: &Mandate, now_ms: u64) -> Vec<Io> {
    let _ = now_ms;
    if r.ready.is_empty() && r.remove.is_empty() {
        return next_space(h, c, r);
    }
    let made = (|| -> Result<Add, String> {
        let from_secret = identity::epoch_secret(h, &r.member, m.space, m.epoch).ok_or(format!("no secret of epoch {} here", m.epoch))?;
        crate::arm_random(identity::upkeep_random(h).ok_or("no randomness yet")?);
        let mut g = craftworks_mls::Account::load(craftworks_mls::Rule::Space(m.space), &m.state)?;
        if g.epoch() != m.epoch {
            return Err(format!("the mandate's group is at epoch {}, not {}", g.epoch(), m.epoch));
        }
        let kps: Vec<Vec<u8>> = r.ready.iter().map(|(a, _)| unhex(&a.kp).ok_or("a key is not hex")).collect::<Result<_, _>>()?;
        let (commit, welcome) = g.follow(&r.remove, &kps)?;
        Ok(Add { from_epoch: m.epoch, from_secret, commit, welcome, info: g.group_info()?, epoch: g.epoch(), secret: g.epoch_secret()?, state: g.save()?, removed: r.remove.len(), to: r.ready.clone() })
    })();
    match made {
        Ok(add) => {
            let log = epoch_log(&c.tail, &m.channel, &add.from_secret);
            r.step = Step::Log { add };
            keep(h, Some(&r));
            vec![Io::Get { id: log.id_bytes(), subscribe: false }]
        }
        Err(why) => {
            identity::upkeep_say(h, &r.member, &format!("{}: no commit — {why}", m.name));
            next_space(h, c, r)
        }
    }
}

/// THEIR CARD read as far as it goes (its blocks fetched, at most 4 times deeper): the key checked, or the blocks asked
/// for, or why not.
#[allow(clippy::too_many_arguments)]
fn card_read<H: Host>(h: &mut H, c: &Held, mut r: Round, ask: Ask, data: [u8; 32], card: Vec<u8>, got: Vec<([u8; 32], Vec<u8>)>, depth: u8, now_ms: u64) -> Vec<Io> {
    let read = match read_card(&c.tail, &c.block, &data, &card, &got) {
        Ok(CardNext::Need(want)) if depth < 4 => {
            let io = want.iter().map(|(id, _)| Io::Get { id: *id, subscribe: false }).collect();
            r.step = Step::CardBlocks { ask, data, card, want, got, depth };
            keep(h, Some(&r));
            return io;
        }
        Ok(CardNext::Need(want)) => Err(format!("their card still names {} block(s) after {depth} reads", want.len())),
        Ok(CardNext::Read(read)) => Ok(read),
        Err(why) => Err(why),
    };
    // A LEVEL GROUP's join: only while their card's contributions reach its level (the one rule, gov's).
    let read = read.and_then(|(inbox, contrib)| {
        if ask.code == "level" {
            let need = r.level.clone().ok_or("not a level group")?;
            let (have, _) = craftworks_gov::level_from(contrib.as_deref().unwrap_or("[]"), now_ms as f64);
            if !craftworks_gov::level_reaches(have, &need) {
                return Err(format!("their card shows {have}, not {need}"));
            }
        }
        Ok(inbox)
    });
    match read {
        Ok(inbox) => r.ready.push((ask, inbox)),
        Err(why) => decided(h, &r.member, &ask, "fail", &why, now_ms),
    }
    next_check(h, c, r, now_ms)
}

/// The GROUP moved to `add`'s epoch: its secret kept for the member, the mandate's group this one; a page loads it.
fn moved<H: Host>(h: &mut H, member: &[u8; 32], space: &[u8; 32], add: &Add) {
    identity::keep_epoch(h, member, Some(*space), add.epoch, &add.secret);
    let Some((me, spaces)) = identity::upkeep_mandate(h, member) else { return };
    let spaces: Vec<Mandate> = spaces
        .into_iter()
        .map(|mut m| {
            if m.space == *space {
                m.epoch = add.epoch;
                m.state = add.state.clone();
            }
            m
        })
        .collect();
    identity::upkeep_set_mandate(h, member, &me, &spaces);
    identity::upkeep_set_moved(h, member, space, true);
}

/// The inbox a welcome to `a` is dropped in (its contract id).
fn inbox_id<H: Host>(h: &H, a: &Ask) -> [u8; 32] {
    let bag = Held::of(h).map(|c| c.bag).unwrap_or_default();
    id_of(&bag, &identity::inbox_address(&a.did_bytes))
}

/// THE WELCOME, sealed to the new entry's inbox key, dropped in their inbox (a PUT the hosts merge).
fn welcome_io<H: Host>(h: &mut H, member: &[u8; 32], me: &str, a: &Ask, inbox: [u8; 32], add: &Add) -> Vec<Io> {
    let Some(welcome) = &add.welcome else { return Vec::new() };
    let Some((_, spaces)) = identity::upkeep_mandate(h, member) else { return Vec::new() };
    let Some(m) = spaces.iter().find(|m| m.space == a.space) else { return Vec::new() };
    let Some(eph) = identity::upkeep_random(h) else { return Vec::new() };
    // THE ONE WELCOME (`identity::welcome_message`, the page's too): with the key it was made for, when, and the
    // space's history below the epoch it opens at (the secrets this member holds).
    let history = identity::epochs_below(h, member, m.space, add.from_epoch + 1);
    let item = identity::welcome_message(&identity::Welcome {
        space: m.space,
        space_kind: &m.kind,
        from: me,
        owner: &m.owner,
        nonce: m.nonce.as_deref(),
        name: &m.name,
        welcome,
        kp: craftworks_rules::admission::tag(&a.kp),
        made: identity::upkeep_now(h).unwrap_or(0) * 1000,
        code: (a.code != KEY).then_some(a.code.as_str()),
        circle: None,
        group: None,
        repair: None,
        history: &history,
    })
    .to_string();
    let sealed = identity::seal_to(&inbox, item.as_bytes(), eph);
    let address = identity::inbox_address(&a.did_bytes);
    let state = craftworks_bag_contract::encode(&address, &[craftworks_bag_contract::grind(&address, &sealed)]);
    vec![Io::Put { code: Code::Bag, params: address.to_vec(), state }]
}

/// `{ w, welcomed, kp, at }` into the space's writers list (`table::seal_writers_item`): this key answered.
fn answered_io<H: Host>(h: &mut H, member: &[u8; 32], a: &Ask, now_ms: u64) -> Vec<Io> {
    let Some(r) = identity::upkeep_random(h) else { return Vec::new() };
    let nonce: [u8; 12] = r[..12].try_into().expect("12 of 32");
    let value = serde_json::json!({ "w": hex(member), "welcomed": a.did, "kp": hex(&craftworks_rules::admission::tag(&a.kp)), "at": now_ms, "by": "node" });
    let address = crate::table::sealed_bag_address(&a.space, "writers");
    let item = crate::table::seal_writers_item(&a.space, &value, nonce);
    let state = craftworks_bag_contract::encode(&address, &[craftworks_bag_contract::grind(&address, &item)]);
    vec![Io::Put { code: Code::Bag, params: address.to_vec(), state }]
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

/// A bag's NAME: a code's own; the open door `open <space>`; a level group's `level <space>`; the member bag `key <space>`.
pub fn bag_name(code: &str, space: &[u8; 32]) -> String {
    match code {
        "open" | "level" | "key" => format!("{code} {}", hex(space)),
        _ => code.to_string(),
    }
}

/// A card's inbox key (hex at `inbox`) and running contributions (JSON at `contrib`). Its key packages are not read:
/// a welcome is made with the request's own key.
/// Not read: WHICH way (a report must carry what settles its cause) — or the tree BLOCKS to fetch first (`Need`).
type CardRead = ([u8; 32], Option<String>);
enum CardNext {
    Read(CardRead),
    Need(Vec<([u8; 32], [u8; 32])>),
}
fn read_card(tail: &[u8], block_hash: &[u8; 32], data: &[u8; 32], state: &[u8], blocks: &[([u8; 32], Vec<u8>)]) -> Result<CardNext, String> {
    // A PUBLIC tail under their data key: the state verifies against the params that key gives, or is not taken.
    let mut o = data::Open::new(tail, data, "card");
    o.public = true;
    if !o.absorb(state) {
        return Err(format!("their card's state ({} bytes) does not verify under their data key", state.len()));
    }
    for (cid, st) in blocks {
        o.absorb_block(cid, st);
    }
    let rows = match o.opened() {
        Ok(data::Step::Ready(rows)) => rows,
        Ok(data::Step::Need(cids)) => {
            let want: Vec<([u8; 32], [u8; 32])> = cids.iter().filter_map(|cid| Some((crate::table::block_id(&o, cid, &[0; 32], block_hash)?, *cid))).collect();
            if want.is_empty() {
                return Err(format!("their card names {} tree block(s) it cannot place", cids.len()));
            }
            return Ok(CardNext::Need(want));
        }
        Ok(data::Step::Keys(es)) => return Err(format!("their card asks for the keys of {} epoch(s)", es.len())),
        Err(why) => return Err(format!("their card does not open: {why}")),
    };
    let names = || rows.keys().map(|k| String::from_utf8_lossy(k).into_owned()).collect::<Vec<_>>().join(", ");
    let text = rows.get(b"inbox".as_slice()).ok_or_else(|| format!("their card has no inbox row ({} row(s): {})", rows.len(), names()))?;
    let inbox: [u8; 32] = std::str::from_utf8(text).ok().and_then(unhex).and_then(|b| b.try_into().ok()).ok_or_else(|| format!("their card's inbox row is not a key ({} bytes)", text.len()))?;
    let contrib = rows.get(b"contrib".as_slice()).and_then(|v| String::from_utf8(v.clone()).ok());
    Ok(CardNext::Read((inbox, contrib)))
}

