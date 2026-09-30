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
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

struct Codes {
    tail: Vec<u8>,
    bag_hash: [u8; 32],
    idlog_hash: [u8; 32],
}
fn codes<H: Host>(h: &H) -> Option<Codes> {
    let bag = h.get_secret(identity::UPKEEP_BAG)?;
    let tail = h.get_secret(identity::UPKEEP_TAIL)?;
    let idlog_hash = h.get_secret(identity::UPKEEP_IDLOG)?.try_into().ok()?;
    Some(Codes { bag_hash: contract_keys::code_hash(&bag), tail, idlog_hash })
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
        identity::upkeep_say(h, &r.member, "a round did not finish: dropped");
        keep(h, None);
    }
    let Some(c) = codes(h) else { return Vec::new() };
    // Each person's turn in order: the first whose page is away and who has a request bag to read.
    for member in identity::upkeep_members(h) {
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
            for code in bags {
                let name = if code == "open" { format!("open {}", hex(&m.space)) } else { code.clone() };
                waiting.push((id_of(&c.bag_hash, &identity::invite_address(&name)), m.space, code));
            }
        }
        if waiting.is_empty() {
            continue;
        }
        let io = waiting.iter().map(|(id, _, _)| Io::Get { id: *id, subscribe: true }).collect();
        keep(h, Some(&Round { member, step: Step::Requests { waiting }, asks: Vec::new(), moved: w }));
        return io;
    }
    Vec::new()
}

/// An ANSWER from the network: the round moved on. What to send next.
pub fn replied<H: Host>(h: &mut H, reply: Reply, now_ms: u64) -> Vec<Io> {
    let Some(mut r) = round(h) else { return Vec::new() };
    let Some(c) = codes(h) else { return Vec::new() };
    let member = r.member;
    let Some((me, spaces)) = identity::upkeep_mandate(h, &member) else {
        keep(h, None);
        return Vec::new();
    };
    r.moved = wakeups(h);
    let space_of = |s: &[u8; 32]| spaces.iter().find(|m| m.space == *s).cloned();
    match (r.step.clone(), reply) {
        (Step::Requests { mut waiting }, Reply::Got { id, state }) => {
            let Some(i) = waiting.iter().position(|(w, _, _)| *w == id) else { return Vec::new() };
            let (_, space, code) = waiting.remove(i);
            if let (Some(st), Some(m)) = (state, space_of(&space)) {
                let name = if code == "open" { format!("open {}", hex(&space)) } else { code.clone() };
                for p in craftworks_bag_contract::read(&identity::invite_address(&name), &st).unwrap_or_default() {
                    let Ok(q) = serde_json::from_slice::<serde_json::Value>(craftworks_bag_contract::payload(&p)) else { continue };
                    let (Some("join"), Some(did)) = (q["kind"].as_str(), q["did"].as_str()) else { continue };
                    let Some(did_bytes) = craftworks_account::did_bytes(did) else { continue };
                    let known = r.asks.iter().any(|a| a.space == space && a.did == did);
                    let admitted = identity::upkeep_admitted(h, &member).iter().any(|a| a.space == space && a.did == did);
                    if did == me || known || admitted || m.members.iter().any(|x| x == did) || m.bans.iter().any(|x| x == did) {
                        continue;
                    }
                    r.asks.push(Ask { space, did: did.to_string(), did_bytes, code: code.clone() });
                }
            }
            if !waiting.is_empty() {
                r.step = Step::Requests { waiting };
                keep(h, Some(&r));
                return Vec::new();
            }
            next_ask(h, r, &c)
        }
        (Step::KeyLog { ask }, Reply::Got { state, .. }) => {
            let log = state.and_then(|st| craftworks_idlog_contract::read(&ask.did_bytes, &st));
            let Some(log) = log else {
                identity::upkeep_say(h, &member, &format!("{}: no key log", ask.did));
                return next_ask(h, r, &c);
            };
            let data = log.head().data;
            let card = data::Open::new(&c.tail, &data, "card");
            r.step = Step::Card { ask, data };
            keep(h, Some(&r));
            vec![Io::Get { id: card.id_bytes(), subscribe: false }]
        }
        (Step::Card { ask, data }, Reply::Got { state, .. }) => {
            let Some(m) = space_of(&ask.space) else { return next_ask(h, r, &c) };
            match add(h, &member, &c, &m, &data, state) {
                Ok(add) => {
                    // The commit goes into the log of the epoch it moves FROM: read it first (taken: stop).
                    let log = epoch_log(&c, &m.channel, &add.from_secret);
                    r.step = Step::Log { ask, add };
                    keep(h, Some(&r));
                    vec![Io::Get { id: log.id_bytes(), subscribe: false }]
                }
                Err(why) => {
                    identity::upkeep_say(h, &member, &format!("{} not admitted: {why}", ask.did));
                    next_ask(h, r, &c)
                }
            }
        }
        (Step::Log { ask, add }, Reply::Got { state, .. }) => {
            let Some(m) = space_of(&ask.space) else { return next_ask(h, r, &c) };
            let mut log = epoch_log(&c, &m.channel, &add.from_secret);
            if let Some(st) = &state {
                log.absorb(st);
            }
            let at = format!("c/{:012}", add.from_epoch);
            let taken = matches!(log.opened(), Ok(data::Step::Ready(rows)) if rows.contains_key(at.as_bytes()));
            if taken {
                // Another member moved the group from this epoch first: the mandate is behind. Stop until a page
                // hands a newer one over.
                identity::upkeep_say(h, &member, &format!("{}: the group moved from epoch {} meanwhile", m.name, add.from_epoch));
                keep(h, None);
                return Vec::new();
            }
            let entry = serde_json::json!({ "commit": hex(&add.commit), "info": hex(&add.info) }).to_string();
            let key = identity::epoch_log_key(&add.from_secret);
            let Some(send) = signed(&mut log, &key, at.as_bytes(), entry.as_bytes()) else {
                identity::upkeep_say(h, &member, "the epoch's log would not take the commit");
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
                identity::upkeep_say(h, &member, "the commit was refused: the group moved meanwhile");
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
                identity::upkeep_say(h, &member, "the next epoch's log would not take its first row");
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
            identity::upkeep_say(h, &member, &format!("{} admitted (epoch {})", ask.did, add.epoch));
            next_ask(h, r, &c)
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
    let item = serde_json::json!({
        "kind": "welcome", "space": hex(&m.space), "spaceKind": m.kind, "from": me, "owner": m.owner, "nonce": m.nonce,
        "name": m.name, "welcome": hex(&add.welcome), "code": ask.code,
    })
    .to_string();
    let sealed = identity::seal_to(&add.inbox, item.as_bytes(), eph);
    let address = identity::inbox_address(&ask.did_bytes);
    let state = craftworks_bag_contract::encode(&address, &[craftworks_bag_contract::grind(&address, &sealed)]);
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
                m.members.push(ask.did.clone());
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
fn add<H: Host>(h: &mut H, member: &[u8; 32], c: &Codes, m: &Mandate, data: &[u8; 32], card: Option<Vec<u8>>) -> Result<Add, String> {
    let from_secret = identity::epoch_secret(h, member, m.space, m.epoch).ok_or(format!("no secret of epoch {} here", m.epoch))?;
    let card = card.ok_or("they have no card")?;
    let (packages, inbox) = read_card(c, data, &card).ok_or("their card has no key package or no inbox")?;
    // A key package works ONCE: never one the account used (the page's record) or upkeep used since (not yet recorded).
    let used: Vec<[u8; 16]> = identity::upkeep_spent(h, member).into_iter().chain(identity::upkeep_admitted(h, member).into_iter().map(|a| a.kp)).collect();
    let unused: Vec<&(Vec<u8>, [u8; 16])> = packages.iter().filter(|(_, t)| !used.contains(t)).collect();
    if unused.is_empty() {
        return Err("their card has no key package left unused (it renews when they are next online)".into());
    }
    let pick = identity::upkeep_random(h).ok_or("no randomness yet (a page stirs it)")?;
    let (kp, tag) = unused[pick[0] as usize % unused.len()];
    crate::arm_random(identity::upkeep_random(h).ok_or("no randomness yet")?);
    let mut g = craftworks_mls::Account::load(craftworks_mls::Rule::Space(m.space), &m.state)?;
    if g.epoch() != m.epoch {
        return Err(format!("the mandate's group is at epoch {}, not {}", g.epoch(), m.epoch));
    }
    let (commit, welcome) = g.add(kp)?;
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
        kp: *tag,
    })
}

/// A card's key packages (hex list at `kp`) and inbox key (hex at `inbox`).
fn read_card(c: &Codes, data: &[u8; 32], state: &[u8]) -> Option<(Vec<(Vec<u8>, [u8; 16])>, [u8; 32])> {
    // A PUBLIC tail under their data key: the state verifies against the params that key gives, or is not taken.
    let mut o = data::Open::new(&c.tail, data, "card");
    o.public = true;
    if !o.absorb(state) {
        return None;
    }
    let data::Step::Ready(rows) = o.opened().ok()? else { return None };
    let unhex = |s: &str| -> Option<Vec<u8>> { (0..s.len() / 2).map(|i| u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()).collect() };
    let inbox: [u8; 32] = unhex(std::str::from_utf8(rows.get(b"inbox".as_slice())?).ok()?)?.try_into().ok()?;
    let list: Vec<String> = serde_json::from_slice(rows.get(b"kp".as_slice())?).ok()?;
    let packages: Vec<(Vec<u8>, [u8; 16])> = list.iter().filter_map(|x| Some((unhex(x)?, identity::kp_tag(x)))).collect();
    (!packages.is_empty()).then_some((packages, inbox))
}
