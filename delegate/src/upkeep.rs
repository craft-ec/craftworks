//! UPKEEP with no page open: ADMITTING askers into the spaces this account may invite into, by the MANDATE a page
//! handed over (`identity::Mandate`). The node wakes the delegate every minute; each wake-up starts a round when no page
//! has handed the mandate over lately (a page that ticks does this itself). A round, one asker at a time:
//!
//!   1. GET each space's request bags (an open space's `open <id>`, each code in force) — whoever asked and is not in.
//!   2. GET the asker's key log (its data key: the card's address) and card (a key package, the inbox key).
//!   3. MLS: add them to the space's group (its state from the mandate) — a commit, a welcome, the next epoch.
//!   4. The commit into its epoch's log (GET it, UPDATE: taken already → the group moved elsewhere: stop), the next
//!      epoch's log made (PUT its `open` row), the welcome sealed to their inbox key and dropped in their inbox (PUT).
//!   5. Recorded: the mandate's group moved (a page loads it), the admission (a page writes the `admitted` act).
//!
//! Each step is a message out and an answer in, across calls: the round's state lives in the delegate's secrets. Pure:
//! the node's messages are the entry's (`lib.rs`), so the whole round runs in tests against a scripted network.

use craftworks_data as data;
use craftworks_rules::admission::{decide, Decision, Entry, Request, View, Welcomed};
use craftworks_identity::{self as identity, Admitted, Host, Mandate};
use serde::{Deserialize, Serialize};

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

/// Someone asking into a space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Ask {
    space: [u8; 32],
    did: String,
    did_bytes: [u8; 32],
    code: String,
    /// The key package the request carries (hex): the one its welcome is made with (`rules::admission`).
    kp: String,
}

/// An addition made (MLS), carried until its commit, next log and welcome are sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Add {
    from_epoch: u64,
    from_secret: [u8; 32],
    commit: Vec<u8>,
    welcome: Vec<u8>,
    info: Vec<u8>,
    epoch: u64,
    secret: [u8; 32],
    state: Vec<u8>,
    inbox: [u8; 32],
    /// The key package used (`identity::kp_tag`).
    kp: [u8; 16],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Step {
    /// Waiting for request bags: `(id, space, code)` not answered yet.
    Requests { waiting: Vec<([u8; 32], [u8; 32], String)> },
    KeyLog { ask: Ask },
    /// Waiting for their card, at the address their key log's data key gives.
    Card { ask: Ask, data: [u8; 32] },
    /// …and for its tree BLOCKS (its rows past its head): `want` (contract id, block) not answered yet, `got` those
    /// that were; `depth` how many times it asked for more.
    CardBlocks { ask: Ask, data: [u8; 32], card: Vec<u8>, want: Vec<([u8; 32], [u8; 32])>, got: Vec<([u8; 32], Vec<u8>)>, depth: u8 },
    Log { ask: Ask, add: Add },
    Commit { ask: Ask, add: Add },
    Next { ask: Ask, add: Add },
    Welcome { ask: Ask, add: Add, tries: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Round {
    /// Whose round (each person on the node has their own mandate).
    member: [u8; 32],
    step: Step,
    /// Askers still to admit after this one.
    asks: Vec<Ask>,
    /// The group's SHARED LIST of welcomes made, as each space's writers list said this round: `(space, did, kp tag, at)`.
    heard: Vec<([u8; 32], String, [u8; 16], u64)>,
    /// Requests read, decided once every bag of the round is read (over all that was heard): `(space, bag, item JSON)`.
    pending: Vec<([u8; 32], String, String)>,
    /// The wake-up count when the round last moved: a round stuck for `STUCK` wake-ups is dropped (or, sending a
    /// welcome, sent again).
    moved: u64,
}

pub const ROUND: &[u8] = b"identity_upkeep/round";
/// A page that handed the mandate over this recently is running: upkeep stays out of its way.
pub const PAGE_AWAY: u64 = 2;
pub const STUCK: u64 = 3;

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
/// The contract a step waits on (the Requests step: its bags, matched there).
fn waits_for(step: &Step, c: &Codes, space_of: &dyn Fn(&[u8; 32]) -> Option<Mandate>) -> Option<[u8; 32]> {
    let channel = |a: &Ask| space_of(&a.space).map(|m| m.channel);
    match step {
        Step::Requests { .. } => None,
        Step::KeyLog { ask } => Some(id_of(&c.idlog_hash, &ask.did_bytes)),
        Step::Card { data, .. } => Some(data::Open::new(&c.tail, data, "card").id_bytes()),
        Step::CardBlocks { .. } => None,
        Step::Log { ask, add } | Step::Commit { ask, add } => Some(epoch_log(c, &channel(ask)?, &add.from_secret).id_bytes()),
        Step::Next { ask, add } => Some(epoch_log(c, &channel(ask)?, &add.secret).id_bytes()),
        Step::Welcome { ask, .. } => Some(id_of(&c.bag_hash, &identity::inbox_address(&ask.did_bytes))),
    }
}

/// A request's step taken here (`identity::Decided`: the `member` lifecycle's move, by the node).
fn decided<H: Host>(h: &mut H, member: &[u8; 32], ask: &Ask, step: &str, why: &str, now_ms: u64) {
    let q = craftworks_rules::admission::tag(&ask.kp);
    identity::upkeep_decide(h, member, identity::Decided { space: ask.space, did: ask.did.clone(), q, at: now_ms, step: step.into(), why: why.into() });
}

/// The member whose round ran last (the next round starts after it: each person in turn).
const LAST_SERVED: &[u8] = b"identity_upkeep/last-served";

/// The round's read of a space's writers list (beside its request bags).
const WRITERS: &str = "writers";
/// How long the first in turn has before the next one lets the asker in (`rules::admission`).
const TAKEOVER_MS: u64 = 120_000;
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

struct Codes {
    tail: Vec<u8>,
    bag_hash: [u8; 32],
    idlog_hash: [u8; 32],
    /// A public tail's tree blocks (a card's rows past what its head carries): `table::block_id`.
    block_hash: [u8; 32],
}
fn codes<H: Host>(h: &H) -> Option<Codes> {
    let bag = h.get_secret(identity::UPKEEP_BAG)?;
    let tail = h.get_secret(identity::UPKEEP_TAIL)?;
    let idlog_hash = h.get_secret(identity::UPKEEP_IDLOG)?.try_into().ok()?;
    let block_hash = h.get_secret(identity::UPKEEP_BLOCK)?.try_into().ok()?;
    Some(Codes { bag_hash: contract_keys::code_hash(&bag), tail, idlog_hash, block_hash })
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

/// A WAKE-UP: a round started (none running, no page lately, a mandate and the codes held), or a stuck one dropped or
/// retried. The GETs to send.
pub fn woke<H: Host>(h: &mut H, now_ms: u64) -> Vec<Io> {
    let w = wakeups(h);
    if let Some(mut r) = round(h) {
        if w.saturating_sub(r.moved) < STUCK {
            return Vec::new();
        }
        // Stuck. A welcome not answered is sent again (the asker is in the group already: they must hear of it);
        // anything else is dropped, the next round starts over from the bags.
        if let Step::Welcome { ask, add, tries } = &r.step {
            if *tries < 5 {
                let (ask, add, tries) = (ask.clone(), add.clone(), tries + 1);
                let io = welcome_io(h, &r.member, &ask, &add);
                r.step = Step::Welcome { ask, add, tries };
                r.moved = w;
                keep(h, Some(&r));
                return io;
            }
        }
        // THE BAGS, SOME SILENT (10-15, live: "1 bag(s) never answered (writers)" dropped a whole round, every space's
        // requests with it): the round goes on with what answered. A bag that never answered adds no request; a space
        // whose WRITERS list never answered is held back whole — without it a request could be answered twice (each
        // request once: `rules::admission`). The next round reads them all again.
        if let Step::Requests { waiting } = &r.step {
            let held: Vec<[u8; 32]> = waiting.iter().filter(|(_, _, code)| code == WRITERS).map(|(_, space, _)| *space).collect();
            let silent = waiting.len();
            if let Some(&(id, space, ref code)) = waiting.first() {
                let last = (id, space, code.clone());
                r.pending.retain(|(s, ..)| !held.contains(s));
                r.step = Step::Requests { waiting: vec![last] };
                keep(h, Some(&r));
                let member = r.member;
                let spaces: Vec<String> = held.iter().map(|s| hex(&s[..6])).collect();
                identity::upkeep_say(h, &member, &format!("{silent} bag(s) never answered: went on without them{}", if spaces.is_empty() { String::new() } else { format!(", space(s) {} held back (their writers list silent)", spaces.join(", ")) }));
                return replied(h, Reply::Got { id, state: None }, now_ms);
            }
        }
        // DROPPED, SAID WHERE (a report carries what settles its cause): an asker's step, as that request's (the
        // `member` machine's, by the node); the bags' — which never answered — said.
        let at = match &r.step {
            Step::Requests { waiting } => {
                let kinds: Vec<&str> = waiting.iter().map(|(_, _, code)| if ["writers", "stuck", "open", "level"].contains(&code.as_str()) { code.as_str() } else { "a code's" }).collect();
                format!("{} bag(s) never answered ({})", waiting.len(), kinds.join(", "))
            }
            Step::KeyLog { .. } => "their key log never answered".into(),
            Step::Card { .. } => "their card never answered".into(),
            Step::CardBlocks { want, .. } => format!("{} of their card's blocks never answered", want.len()),
            Step::Log { .. } => "the epoch's log never answered".into(),
            Step::Commit { .. } => "the commit was never answered".into(),
            Step::Next { .. } => "the next epoch's log never answered".into(),
            Step::Welcome { .. } => "the welcome was not taken in 5 tries".into(),
        };
        match &r.step {
            Step::KeyLog { ask } | Step::Card { ask, .. } | Step::CardBlocks { ask, .. } | Step::Log { ask, .. } | Step::Commit { ask, .. } | Step::Next { ask, .. } | Step::Welcome { ask, .. } => {
                let ask = ask.clone();
                decided(h, &r.member, &ask, "fail", &format!("the round stalled: {at}"), now_ms);
            }
            Step::Requests { .. } => identity::upkeep_say(h, &r.member, &format!("a round did not finish: dropped — {at}")),
        }
        keep(h, None);
    }
    let Some(c) = codes(h) else { return Vec::new() };
    // EACH PERSON IN TURN (10-14, measured on fx: with a dozen accounts on one node, each wake-up went to the first of
    // them away — one person was never upkept): from the one after the member last served, the first whose page is
    // away and who has a request bag to read.
    let members = identity::upkeep_members(h);
    let last = h.get_secret(LAST_SERVED).and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok());
    let from = last.and_then(|l| members.iter().position(|m| *m == l)).map_or(0, |i| i + 1);
    for member in members.iter().cycle().skip(from).take(members.len()).copied() {
        if identity::upkeep_since_tick(h, &member) < PAGE_AWAY {
            continue;
        }
        let Some((_, spaces)) = identity::upkeep_mandate(h, &member) else { continue };
        let mut waiting = Vec::new();
        for m in &spaces {
            let mut bags: Vec<String> = m.codes.iter().filter(|(_, exp, _)| *exp == 0 || now_ms < *exp).map(|(code, _, _)| code.clone()).collect();
            if m.open {
                bags.push("open".into());
            }
            // A LEVEL GROUP: who asks at `level <space>` (their card's level checked when it is read).
            if m.level.as_deref().is_some_and(|l| craftworks_gov::LEVELS.iter().any(|(id, _)| *id == l)) {
                bags.push("level".into());
            }
            // THE MEMBER BAG: members by the acts asking in again (told, behind, off the branch), `stuck <space>`.
            bags.push("stuck".into());
            for code in bags {
                let name = bag_name(&code, &m.space);
                waiting.push((id_of(&c.bag_hash, &identity::invite_address(&name)), m.space, code));
            }
            // …and the space's WRITERS list: the welcomes any member made (`rules::admission` answers each request once).
            waiting.push((id_of(&c.bag_hash, &crate::table::sealed_bag_address(&m.space, "writers")), m.space, WRITERS.into()));
        }
        if waiting.is_empty() {
            continue;
        }
        let io = waiting.iter().map(|(id, _, _)| Io::Get { id: *id, subscribe: true }).collect();
        keep(h, Some(&Round { member, step: Step::Requests { waiting }, asks: Vec::new(), heard: Vec::new(), pending: Vec::new(), moved: w }));
        h.set_secret(LAST_SERVED, &member);
        identity::upkeep_set_round_at(h, &member, w);
        return io;
    }
    Vec::new()
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
    let Some(c) = codes(h) else { return Vec::new() };
    let member = r.member;
    let Some((me, spaces)) = identity::upkeep_mandate(h, &member) else {
        keep(h, None);
        return Vec::new();
    };
    let space_of = |s: &[u8; 32]| spaces.iter().find(|m| m.space == *s).cloned();
    // AN ANSWER IS ITS STEP'S ONLY IF IT IS FOR THE CONTRACT THAT STEP ASKED (10-14, measured on fx: the inbox read each
    // wake-up answered while a round waited for a key log — its 6 bytes taken as the asker's log, "no key log"; a
    // card's step took another answer, "their card has no inbox": nobody was ever let in with no page open).
    if waits_for(&r.step, &c, &space_of).is_some_and(|w| w != id) {
        return Vec::new();
    }
    r.moved = wakeups(h);
    match (r.step.clone(), reply) {
        (Step::Requests { mut waiting }, Reply::Got { id, state }) => {
            let Some(i) = waiting.iter().position(|(w, _, _)| *w == id) else { return Vec::new() };
            let (_, space, code) = waiting.remove(i);
            if let Some(st) = state {
                if code == WRITERS {
                    let address = crate::table::sealed_bag_address(&space, "writers");
                    for p in craftworks_bag_contract::read(&address, &st).unwrap_or_default() {
                        let Some(v) = crate::table::open_bag_item(&space, &Default::default(), "writers", craftworks_bag_contract::payload(&p)) else { continue };
                        let (Some(did), Some(kp)) = (v["welcomed"].as_str(), v["kp"].as_str().and_then(unhex).and_then(|t| <[u8; 16]>::try_from(t).ok())) else { continue };
                        r.heard.push((space, did.to_string(), kp, v["at"].as_u64().unwrap_or(0)));
                    }
                } else {
                    let name = bag_name(&code, &space);
                    for p in craftworks_bag_contract::read(&identity::invite_address(&name), &st).unwrap_or_default() {
                        r.pending.push((space, code.clone(), String::from_utf8_lossy(craftworks_bag_contract::payload(&p)).into_owned()));
                    }
                }
            }
            if !waiting.is_empty() {
                r.step = Step::Requests { waiting };
                keep(h, Some(&r));
                return Vec::new();
            }
            // EVERY BAG READ: each request decided over all that was heard.
            for (space, code, text) in std::mem::take(&mut r.pending) {
                let Some(m) = space_of(&space) else { continue };
                let Ok(q) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
                let Some(did) = q["did"].as_str() else { continue };
                let Some(did_bytes) = craftworks_account::did_bytes(did) else { continue };
                let known = r.asks.iter().any(|a| a.space == space && a.did == did);
                if did == me || known || m.bans.iter().any(|x| x == did) {
                    continue;
                }
                // THE ONE RULE (`rules::admission`, the page runs it too): the request's own key, once per request,
                // one admitter by turn. The space's member bag (`stuck`): AGAIN, from a member by the acts (in its
                // members); a code's, the open door's, a level's: a JOIN.
                let entry = match (code == "stuck", q["kind"].as_str()) {
                    (true, Some("stuck")) if m.members.iter().any(|x| x == did) => Entry::Again { from: q["epoch"].as_u64(), seen: q["seen"].as_u64() },
                    (false, Some("join")) => Entry::Join,
                    _ => continue,
                };
                let mine = identity::upkeep_admitted(h, &member).into_iter().filter(|a| a.space == space);
                let mut welcomed: Vec<Welcomed> = r.heard.iter().filter(|(s, ..)| *s == space).map(|(_, d, kp, at)| Welcomed { did: d.clone(), kp: *kp, at: *at }).collect();
                welcomed.extend(mine.map(|a| Welcomed { did: a.did, kp: a.kp, at: a.at }));
                let spent = identity::upkeep_spent(h, &member);
                let request = Request { did, kp: q["kp"].as_str(), at: q["at"].as_u64().unwrap_or(0), entry };
                let view = View {
                    me: &me,
                    admitters: &m.admitters,
                    spent: &spent,
                    welcomed: &welcomed,
                    held: held_by(&m, &did_bytes),
                    epoch: m.epoch,
                    now: now_ms,
                    takeover_ms: TAKEOVER_MS,
                };
                let kp = match decide(&request, &view) {
                    Decision::Admit { kp, .. } => kp,
                    Decision::Skip(why) => {
                        let q = q["kp"].as_str().map(craftworks_rules::admission::tag).unwrap_or_default();
                        identity::upkeep_decide(h, &member, identity::Decided { space, did: did.to_string(), q, at: now_ms, step: "skip".into(), why });
                        continue;
                    }
                };
                r.asks.push(Ask { space, did: did.to_string(), did_bytes, code: code.clone(), kp });
            }
            next_ask(h, r, &c)
        }
        (Step::KeyLog { ask }, Reply::Got { state, .. }) => {
            // Said WHICH (10-07: "no key log" on fx/fy while a page read it): none answered, or a state that did not read.
            let size = state.as_ref().map(Vec::len);
            let log = state.and_then(|st| craftworks_idlog_contract::read(&ask.did_bytes, &st));
            let Some(log) = log else {
                let why = size.map_or("the network answered none".to_string(), |n| format!("its state ({n} bytes) did not read"));
                decided(h, &member, &ask, "fail", &format!("no key log — {why}"), now_ms);
                return next_ask(h, r, &c);
            };
            // THE REQUEST'S KEY IS THEIRS: its credential names this DID under the key its log holds now — a request is
            // an unsigned drop in a bag, and a key made by anyone else would let that one in under their name.
            let theirs = unhex(&ask.kp).and_then(|kp| craftworks_mls::key_package_credential(&kp).ok()).is_some_and(|cred| craftworks_account::space_member(&ask.did_bytes, &log, &cred));
            if !theirs {
                decided(h, &member, &ask, "skip", "the request's key is not theirs (another account's, or a key their log moved past)", now_ms);
                return next_ask(h, r, &c);
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
                return next_ask(h, r, &c);
            };
            card_read(h, r, &c, &member, ask, data, card, Vec::new(), 0, now_ms, &space_of)
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
            card_read(h, r, &c, &member, ask, data, card, got, depth + 1, now_ms, &space_of)
        }
        (Step::Log { ask, add }, Reply::Got { state, .. }) => {
            let Some(m) = space_of(&ask.space) else { return next_ask(h, r, &c) };
            let mut log = epoch_log(&c, &m.channel, &add.from_secret);
            if let Some(st) = &state {
                log.absorb(st);
            }
            // THE LOG's ONE ROW RULE (`rules::ordering`, the pages' too): taken if any writer's row holds the position;
            // this member's own row under its own tag.
            use craftworks_rules::ordering;
            let taken = matches!(log.opened(), Ok(data::Step::Ready(rows)) if ordering::entry_at(rows.iter().map(|(k, v)| (k.as_slice(), v.as_slice())), "c/", add.from_epoch).is_some());
            let at = ordering::row_key("c/", add.from_epoch, &hex(&member[..6]));
            if taken {
                // Another member moved the group from this epoch first: the mandate is behind. Stop until a page
                // hands a newer one over.
                decided(h, &member, &ask, "fail", &format!("the group moved from epoch {} meanwhile (its mandate behind)", add.from_epoch), now_ms);
                keep(h, None);
                return Vec::new();
            }
            // Its PROVENANCE as a page's rows carry it (`keys.provOf`): this member's key, why, when — whoever applies it
            // says who moved the group (the `group` machine's `applied` step).
            let entry = serde_json::json!({ "commit": hex(&add.commit), "info": hex(&add.info), "by": hex(&member[..8]), "why": "admit (node)", "at": now_ms, "v": "node" }).to_string();
            let key = identity::epoch_log_key(&add.from_secret);
            let Some(send) = signed(&mut log, &key, at.as_bytes(), entry.as_bytes()) else {
                decided(h, &member, &ask, "fail", "the epoch's log would not take the commit", now_ms);
                keep(h, None);
                return Vec::new();
            };
            let io = send_io(&log, send, Code::Tail);
            r.step = Step::Commit { ask, add };
            keep(h, Some(&r));
            vec![io]
        }
        (Step::Commit { ask, add }, Reply::Updated { ok, .. } | Reply::Put { ok, .. }) => {
            if !ok {
                decided(h, &member, &ask, "fail", "the commit was refused: the group moved meanwhile", now_ms);
                keep(h, None);
                return Vec::new();
            }
            // The group moved: held here now (this epoch's secret kept, the mandate's group this one) — whatever
            // happens next, what upkeep holds is the group as it is.
            let Some(m) = space_of(&ask.space) else { return next_ask(h, r, &c) };
            moved(h, &member, &me, &spaces, &ask, &add);
            // The next epoch's log, made: its `open` row (the group info after, and the epoch before's secret, so
            // whoever holds this epoch opens every earlier one).
            let mut next = epoch_log(&c, &m.channel, &add.secret);
            let open = serde_json::json!({ "info": hex(&add.info), "prev": hex(&add.from_secret) }).to_string();
            let key = identity::epoch_log_key(&add.secret);
            let Some(send) = signed(&mut next, &key, b"open", open.as_bytes()) else {
                decided(h, &member, &ask, "fail", "the next epoch's log would not take its first row", now_ms);
                keep(h, None);
                return Vec::new();
            };
            let io = send_io(&next, send, Code::Tail);
            r.step = Step::Next { ask, add };
            keep(h, Some(&r));
            vec![io]
        }
        (Step::Next { ask, add }, Reply::Put { .. } | Reply::Updated { .. }) => {
            // Made or not (a node that fetched it first: the same row), the welcome goes: they are in the group.
            let io = welcome_io(h, &member, &ask, &add);
            r.step = Step::Welcome { ask, add, tries: 0 };
            keep(h, Some(&r));
            io
        }
        (Step::Welcome { ask, add, tries }, Reply::Put { ok, .. }) => {
            if !ok {
                // Sent again at a later wake-up (`woke`).
                r.step = Step::Welcome { ask, add, tries };
                keep(h, Some(&r));
                return Vec::new();
            }
            let mut all = identity::upkeep_admitted(h, &member);
            all.push(Admitted { space: ask.space, did: ask.did.clone(), code: ask.code.clone(), at: now_ms, epoch: add.epoch, kp: add.kp });
            identity::upkeep_set_admitted(h, &member, &all);
            decided(h, &member, &ask, "welcomed", &format!("epoch {}", add.epoch), now_ms);
            // THE GROUP'S SHARED LIST of welcomes made (its writers list, as a page writes it): every member's admit and
            // every node's reads it — this request answered, its key spent — with no page of this account open.
            let mut io = answered_io(h, &member, &ask, &add, now_ms);
            io.extend(next_ask(h, r, &c));
            io
        }
        // An answer this step does not wait for (a late one): nothing.
        _ => Vec::new(),
    }
}

/// The next asker (their key log first), or the round is over.
fn next_ask<H: Host>(h: &mut H, mut r: Round, c: &Codes) -> Vec<Io> {
    if r.asks.is_empty() {
        keep(h, None);
        return Vec::new();
    }
    let ask = r.asks.remove(0);
    let id = id_of(&c.idlog_hash, &ask.did_bytes);
    r.step = Step::KeyLog { ask };
    keep(h, Some(&r));
    vec![Io::Get { id, subscribe: false }]
}

/// An epoch's LOG: a tail under the key its secret gives, named by the space's channel, sealed with the table key its
/// secret gives for that channel (as `keys` opens it on a page).
fn epoch_log(c: &Codes, channel: &str, secret: &[u8; 32]) -> data::Open {
    let mut o = data::Open::new(&c.tail, &identity::epoch_log_key(secret).verifying_key().to_bytes(), channel);
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

/// The WELCOME, sealed to the asker's inbox key, dropped in their inbox (a PUT the hosts merge).
fn welcome_io<H: Host>(h: &mut H, member: &[u8; 32], ask: &Ask, add: &Add) -> Vec<Io> {
    let Some((me, spaces)) = identity::upkeep_mandate(h, member) else { return Vec::new() };
    let Some(m) = spaces.iter().find(|m| m.space == ask.space) else { return Vec::new() };
    let Some(eph) = identity::upkeep_random(h) else { return Vec::new() };
    // THE ONE WELCOME (`identity::welcome_message`, the page's too): with the key it was made for, when, and the
    // space's history below the epoch it opens at (the secrets this member holds).
    let history = identity::epochs_below(h, member, m.space, add.from_epoch + 1);
    let item = identity::welcome_message(&identity::Welcome {
        space: m.space,
        space_kind: &m.kind,
        from: &me,
        owner: &m.owner,
        nonce: m.nonce.as_deref(),
        name: &m.name,
        welcome: &add.welcome,
        kp: add.kp,
        made: identity::upkeep_now(h).unwrap_or(0) * 1000,
        code: (ask.code != "stuck").then_some(ask.code.as_str()),
        circle: None,
        group: None,
        repair: None,
        history: &history,
    })
    .to_string();
    let sealed = identity::seal_to(&add.inbox, item.as_bytes(), eph);
    let address = identity::inbox_address(&ask.did_bytes);
    let state = craftworks_bag_contract::encode(&address, &[craftworks_bag_contract::grind(&address, &sealed)]);
    vec![Io::Put { code: Code::Bag, params: address.to_vec(), state }]
}

/// `{ w, welcomed, kp, at }` into the space's writers list (`table::seal_writers_item`), as `conversation.welcome` writes it.
fn answered_io<H: Host>(h: &mut H, member: &[u8; 32], ask: &Ask, add: &Add, now_ms: u64) -> Vec<Io> {
    let Some(r) = identity::upkeep_random(h) else { return Vec::new() };
    let nonce: [u8; 12] = r[..12].try_into().expect("12 of 32");
    let value = serde_json::json!({ "w": hex(member), "welcomed": ask.did, "kp": hex(&add.kp), "at": now_ms, "by": "node" });
    let address = crate::table::sealed_bag_address(&ask.space, "writers");
    let item = crate::table::seal_writers_item(&ask.space, &value, nonce);
    let state = craftworks_bag_contract::encode(&address, &[craftworks_bag_contract::grind(&address, &item)]);
    vec![Io::Put { code: Code::Bag, params: address.to_vec(), state }]
}

/// The GROUP moved to `add`'s epoch: its secret kept for the member, the mandate's group and members this one, the
/// code's use counted; a page loads it (`moved`).
fn moved<H: Host>(h: &mut H, member: &[u8; 32], me: &str, spaces: &[Mandate], ask: &Ask, add: &Add) {
    identity::keep_epoch(h, member, Some(ask.space), add.epoch, &add.secret);
    let spaces: Vec<Mandate> = spaces
        .iter()
        .cloned()
        .map(|mut m| {
            if m.space == ask.space {
                m.epoch = add.epoch;
                m.state = add.state.clone();
                if !m.members.contains(&ask.did) {
                    m.members.push(ask.did.clone());
                }
                if let Some(i) = m.codes.iter().position(|(code, _, _)| *code == ask.code) {
                    match m.codes[i].2 {
                        1 => {
                            m.codes.remove(i);
                        }
                        0 => {}
                        n => m.codes[i].2 = n - 1,
                    }
                }
            }
            m
        })
        .collect();
    identity::upkeep_set_mandate(h, member, me, &spaces);
    identity::upkeep_set_moved(h, member, &ask.space, true);
}

/// The ADDITION: the asker's card read (a key package, the inbox key), the group loaded from the mandate, the asker
/// added. Randomness is upkeep's pool (armed here, ratcheted); the clock upkeep's.
#[allow(clippy::too_many_arguments)]
fn add<H: Host>(h: &mut H, member: &[u8; 32], m: &Mandate, (inbox, contrib): CardRead, ask: &Ask, now_ms: u64) -> Result<Add, String> {
    let from_secret = identity::epoch_secret(h, member, m.space, m.epoch).ok_or(format!("no secret of epoch {} here", m.epoch))?;
    // A LEVEL GROUP's asker: let in only while their card's contributions reach its level (the one rule, gov's).
    if ask.code == "level" {
        let need = m.level.as_deref().ok_or("not a level group")?;
        let (have, _) = craftworks_gov::level_from(contrib.as_deref().unwrap_or("[]"), now_ms as f64);
        if !craftworks_gov::level_reaches(have, need) {
            return Err(format!("their card shows {have}, not {need}"));
        }
    }
    // THE REQUEST'S OWN KEY (`rules::admission::decide` chose it): never one picked from their card.
    let tag = craftworks_rules::admission::tag(&ask.kp);
    let kp = unhex(&ask.kp).ok_or("the request's key is not hex")?;
    crate::arm_random(identity::upkeep_random(h).ok_or("no randomness yet")?);
    let mut g = craftworks_mls::Account::load(craftworks_mls::Rule::Space(m.space), &m.state)?;
    if g.epoch() != m.epoch {
        return Err(format!("the mandate's group is at epoch {}, not {}", g.epoch(), m.epoch));
    }
    // Their entries the group holds still (a member let in again): out in the same commit.
    let stale: Vec<u32> = g.members().into_iter().filter(|(_, _, cred)| cred.get(4..36) == Some(ask.did_bytes.as_slice())).map(|(i, _, _)| i).collect();
    let (commit, welcome) = if stale.is_empty() { g.add(&kp)? } else { g.readd(&stale, &kp)? };
    Ok(Add {
        from_epoch: m.epoch,
        from_secret,
        commit,
        welcome,
        info: g.group_info()?,
        epoch: g.epoch(),
        secret: g.epoch_secret()?,
        state: g.save()?,
        inbox,
        kp: tag,
    })
}

/// Whether the mandate's group holds an entry of theirs (its members' credentials name their DID).
fn held_by(m: &Mandate, did: &[u8; 32]) -> bool {
    craftworks_mls::Account::load(craftworks_mls::Rule::Space(m.space), &m.state)
        .map(|g| g.members().into_iter().any(|(_, _, cred)| cred.get(4..36) == Some(did.as_slice())))
        .unwrap_or(false)
}

/// A request bag's NAME: a code's own; the open door `open <space>`; a level group's `level <space>`.
pub fn bag_name(code: &str, space: &[u8; 32]) -> String {
    match code {
        "open" | "level" | "stuck" => format!("{code} {}", hex(space)),
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
fn read_card(c: &Codes, data: &[u8; 32], state: &[u8], blocks: &[([u8; 32], Vec<u8>)]) -> Result<CardNext, String> {
    // A PUBLIC tail under their data key: the state verifies against the params that key gives, or is not taken.
    let mut o = data::Open::new(&c.tail, data, "card");
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
            let want: Vec<([u8; 32], [u8; 32])> = cids.iter().filter_map(|cid| Some((crate::table::block_id(&o, cid, &[0; 32], &c.block_hash)?, *cid))).collect();
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

/// THEIR CARD read as far as it goes (its blocks fetched, at most 4 times deeper): the addition made from it, or the
/// blocks asked for, or why not — the request's step.
#[allow(clippy::too_many_arguments)]
fn card_read<H: Host>(h: &mut H, mut r: Round, c: &Codes, member: &[u8; 32], ask: Ask, data: [u8; 32], card: Vec<u8>, got: Vec<([u8; 32], Vec<u8>)>, depth: u8, now_ms: u64, space_of: &dyn Fn(&[u8; 32]) -> Option<Mandate>) -> Vec<Io> {
    let Some(m) = space_of(&ask.space) else { return next_ask(h, r, c) };
    let read = match read_card(c, &data, &card, &got) {
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
    match read.and_then(|read| add(h, member, &m, read, &ask, now_ms)) {
        Ok(add) => {
            // The commit goes into the log of the epoch it moves FROM: read it first (taken: stop).
            let log = epoch_log(c, &m.channel, &add.from_secret);
            r.step = Step::Log { ask, add };
            keep(h, Some(&r));
            vec![Io::Get { id: log.id_bytes(), subscribe: false }]
        }
        Err(why) => {
            decided(h, member, &ask, "fail", &why, now_ms);
            next_ask(h, r, c)
        }
    }
}
