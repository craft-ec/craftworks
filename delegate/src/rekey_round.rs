//! THE RE-KEY ROUND with no page open (R4e): `file-keys.js`'s pass, run by upkeep for a member whose page is away —
//! each of their spaces in turn:
//!
//!   1. READ its `files`, `acts` and `pub-acts` (`table`), every member's feed merged.
//!   2. PLAN (`rekey`): a new salt if a removal is in and the group moved; the rows due, this member's turn for each.
//!   3. A new salt WRITTEN (`write`), then each due row whose turn it is now RE-KEYED (`recode`), its row written (and
//!      any page's progress for it cleared) in one step, its OLD pieces BURNED when no row still names that key.
//!
//! Its state lives in a secret between calls (as an admission round's); answers reach it by which contract it waits
//! on, so an admission round and a re-key round run side by side. Once every `EVERY` wake-ups at most (each round reads
//! every space: GETs on the network).

use serde::{Deserialize, Serialize};

use craftworks_identity::{self as identity, Host};

use crate::recode::{self, End, Recode, Target};
use crate::rekey;
use crate::table::{self, Codes, Reading};
use crate::upkeep::{Io, Reply};
use crate::write::{self, Next};

pub const ROUND: &[u8] = b"identity_upkeep/rekey/round";
const LAST: &[u8] = b"identity_upkeep/rekey/last";
/// The member served last (members on one node take turns).
const LAST_MEMBER: &[u8] = b"identity_upkeep/rekey/member";
/// Every contract a re-key round asked of or put lately (a burn, a bag item, a block, a fragment asked and not needed):
/// their answers are the round's — never an admission round's. The newest `ISSUED_MAX`.
const ISSUED: &[u8] = b"identity_upkeep/rekey/issued";
const ISSUED_MAX: usize = 4096;
/// When this member first saw a row due (`<space hex>/<id>` → ms): its takeover counts from there at the least.
const SEEN: &[u8] = b"identity_upkeep/rekey/seen";
/// The wake-up the next round is due at, sooner than `EVERY` when a row's turn comes sooner.
const NEXT_AT: &[u8] = b"identity_upkeep/rekey/next";
/// Wake-ups between rounds (one a minute: every ten minutes).
pub const EVERY: u64 = 10;
/// A round that has not moved for this many wake-ups is dropped (the next starts over from a read).
const STUCK: u64 = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
enum Step {
    Reading,
    /// A write's step sent: waiting on its answers (`waiting`: all must be taken). `rows`: still to write after it (it
    /// was a listing, a flush's blocks or its step) — none: it was the rows' own; `blocks`: those were a flush's blocks
    /// (its step next); `after`: what comes once the rows are written.
    Writing { rows: Vec<(Vec<u8>, Option<Vec<u8>>)>, waiting: Vec<[u8; 32]>, refused: bool, blocks: bool, after: After },
    Recoding { rec: Recode, due: Task },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum After {
    /// A new salt written: plan again.
    Replan,
    /// A row written: its old pieces burned (`old`, with `secret`) when no row names `old_key` any more; then the next.
    Row { burn: Option<([u8; 32], Vec<[u8; 32]>, String)> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Round {
    member: [u8; 32],
    me: String,
    /// The spaces still to visit after this one.
    rest: Vec<[u8; 32]>,
    space: [u8; 32],
    /// The space's name (for what upkeep says).
    name: String,
    owner: String,
    /// The group's members (DIDs): who takes turns. Their DEVICES write the space (`reading.writer_dids`).
    roster: Vec<([u8; 32], String)>,
    reading: Reading,
    step: Step,
    /// Rows still due this pass (turn now).
    queue: Vec<Task>,
    moved: u64,
    /// A salt's secret for the rows due (the space's now).
    salt: Option<([u8; 32], i64)>,
}

fn load<H: Host>(h: &H) -> Option<Round> {
    h.get_secret(ROUND).filter(|b| !b.is_empty()).and_then(|b| bincode::deserialize(&b).ok())
}
fn keep<H: Host>(h: &mut H, r: Option<&Round>) {
    h.set_secret(ROUND, &r.map(|r| bincode::serialize(r).expect("a round encodes")).unwrap_or_default());
}
fn issued<H: Host>(h: &H) -> Vec<[u8; 32]> {
    h.get_secret(ISSUED).and_then(|b| bincode::deserialize(&b).ok()).unwrap_or_default()
}
/// What a round sends: each contract it names noted as the round's (its answer routed here).
fn issue<H: Host>(h: &mut H, c: &Held, io: Vec<Io>) -> Vec<Io> {
    let mut all = issued(h);
    for x in &io {
        let id = match x {
            Io::Get { id, .. } | Io::Update { id, .. } => *id,
            Io::Put { code, params, .. } => {
                let hash = match code {
                    crate::upkeep::Code::Tail => contract_keys::code_hash(&c.tail),
                    crate::upkeep::Code::Bag => c.bag,
                    crate::upkeep::Code::Sealed => c.sealed,
                    crate::upkeep::Code::Piece => c.piece,
                };
                crate::upkeep::id_of(&hash, params)
            }
        };
        all.push(id);
    }
    let over = all.len().saturating_sub(ISSUED_MAX);
    all.drain(..over);
    h.set_secret(ISSUED, &bincode::serialize(&all).expect("ids encode"));
    io
}
fn seen<H: Host>(h: &H) -> std::collections::BTreeMap<String, u64> {
    h.get_secret(SEEN).and_then(|b| bincode::deserialize(&b).ok()).unwrap_or_default()
}

fn wakeups<H: Host>(h: &H) -> u64 {
    h.get_secret(identity::UPKEEP_WAKEUPS).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes).unwrap_or(0)
}

/// The contracts, as upkeep holds them.
pub struct Held {
    tail: Vec<u8>,
    bag: [u8; 32],
    sealed: [u8; 32],
    block: [u8; 32],
    piece: [u8; 32],
    idlog: [u8; 32],
}
impl Held {
    pub fn of<H: Host>(h: &H) -> Option<Held> {
        Some(Held {
            tail: h.get_secret(identity::UPKEEP_TAIL)?,
            bag: contract_keys::code_hash(&h.get_secret(identity::UPKEEP_BAG)?),
            sealed: contract_keys::code_hash(&h.get_secret(identity::UPKEEP_SEALED)?),
            block: h.get_secret(identity::UPKEEP_BLOCK)?.try_into().ok()?,
            piece: contract_keys::code_hash(&h.get_secret(identity::UPKEEP_PIECE)?),
            idlog: h.get_secret(identity::UPKEEP_IDLOG)?.try_into().ok()?,
        })
    }
    fn codes(&self) -> Codes<'_> {
        Codes { tail: &self.tail, bag_hash: self.bag, sealed_hash: self.sealed, block_hash: self.block, idlog_hash: self.idlog }
    }
}

/// A space's ROSTER (writer key → DID) from the mandate's group, and the epoch secrets the member holds.
fn space_of<H: Host>(h: &H, member: &[u8; 32], m: &identity::Mandate) -> Option<(Vec<([u8; 32], String)>, std::collections::BTreeMap<u64, [u8; 32]>)> {
    let g = craftworks_mls::Account::load(craftworks_mls::Rule::Space(m.space), &m.state).ok()?;
    let roster = g
        .members()
        .into_iter()
        .filter_map(|(_, node, cred)| {
            let (did, ..) = identity::read_credential(&cred)?;
            Some((node.try_into().ok()?, craftworks_account::did(&did)))
        })
        .collect();
    // EVERY epoch held (the page keeps each one as the group moves, before any mandate names it): a row sealed with
    // an epoch newer than the mandate's group — a new salt after a removal — still reads.
    let top = identity::latest_epoch(h, member, m.space).unwrap_or(m.epoch).max(m.epoch);
    let epochs = (0..=top).filter_map(|e| Some((e, identity::epoch_secret(h, member, m.space, e)?))).collect();
    Some((roster, epochs))
}

/// A space's round begun: its tables read.
fn begin<H: Host>(h: &mut H, c: &Held, member: [u8; 32], me: String, space: [u8; 32], rest: Vec<[u8; 32]>) -> Vec<Io> {
    let Some((_, spaces)) = identity::upkeep_mandate(h, &member) else { return Vec::new() };
    let Some(m) = spaces.iter().find(|m| m.space == space).cloned() else { return next_space(h, c, member, me, rest) };
    let Some((roster, epochs)) = space_of(h, &member, &m) else {
        identity::upkeep_say(h, &member, &format!("{}: its group does not load here: not re-keyed", m.name));
        return next_space(h, c, member, me, rest);
    };
    // Its WRITERS: each member's devices (a page writes a space under its member key, one per device).
    let dids: Vec<[u8; 32]> = roster.iter().filter_map(|(_, d)| craftworks_account::did_bytes(d)).collect();
    let (reading, io) = Reading::of_members(&c.codes(), space, epochs, &dids, &["files", "acts", "pub-acts"]);
    let r = Round { member, me, rest, space, name: m.name.clone(), owner: m.owner.clone(), roster, reading, step: Step::Reading, queue: Vec::new(), moved: wakeups(h), salt: None };
    keep(h, Some(&r));
    io
}
fn next_space<H: Host>(h: &mut H, c: &Held, member: [u8; 32], me: String, mut rest: Vec<[u8; 32]>) -> Vec<Io> {
    if rest.is_empty() {
        keep(h, None);
        return Vec::new();
    }
    let space = rest.remove(0);
    begin(h, c, member, me, space, rest)
}

/// Why the last wake-up began no round (a page shows it: `identity::upkeep_status`).
pub const WHY: &[u8] = b"identity_upkeep/rekey/why";
fn why<H: Host>(h: &mut H, w: u64, what: &str) {
    h.set_secret(WHY, format!("wake-up {w}: {what}").as_bytes());
}

/// A WAKE-UP: a round begun (none running, the time come, a member whose page is away), or a stuck one dropped.
pub fn woke<H: Host>(h: &mut H) -> Vec<Io> {
    let w = wakeups(h);
    let io = woke_(h);
    if !io.is_empty() {
        why(h, w, "a round begun");
    }
    io
}
fn woke_<H: Host>(h: &mut H) -> Vec<Io> {
    let w = wakeups(h);
    if let Some(r) = load(h) {
        if w.saturating_sub(r.moved) < STUCK {
            why(h, w, "a round is running");
            return Vec::new();
        }
        identity::upkeep_say(h, &r.member, "a re-key round did not finish: dropped");
        keep(h, None);
    }
    let last = h.get_secret(LAST).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes).unwrap_or(0);
    let next_at = h.get_secret(NEXT_AT).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes).unwrap_or(u64::MAX);
    if last != 0 && w.saturating_sub(last) < EVERY && w < next_at {
        why(h, w, &format!("the last round was at wake-up {last}; the next at {}", (last + EVERY).min(next_at)));
        return Vec::new();
    }
    h.set_secret(NEXT_AT, &u64::MAX.to_le_bytes());
    let Some(c) = Held::of(h) else {
        why(h, w, "the contracts it needs were not handed over yet (a page hands them)");
        return Vec::new();
    };
    // Members take turns: the one after the member served last first.
    let mut members = identity::upkeep_members(h);
    if let Some(i) = h.get_secret(LAST_MEMBER).and_then(|l| members.iter().position(|m| m.as_slice() == l.as_slice())) {
        members.rotate_left(i + 1);
    }
    let mut seen_why = Vec::new();
    for member in members {
        if identity::upkeep_since_tick(h, &member) < crate::upkeep::PAGE_AWAY {
            seen_why.push("a page is open");
            continue;
        }
        let Some((me, spaces)) = identity::upkeep_mandate(h, &member) else {
            seen_why.push("no mandate");
            continue;
        };
        let mut all: Vec<[u8; 32]> = spaces.iter().filter(|m| m.kind == "server").map(|m| m.space).collect();
        if all.is_empty() {
            seen_why.push("no spaces");
            continue;
        }
        h.set_secret(LAST, &w.to_le_bytes());
        h.set_secret(LAST_MEMBER, &member);
        let first = all.remove(0);
        let io = begin(h, &c, member, me, first, all);
        return issue(h, &c, io);
    }
    why(h, w, &if seen_why.is_empty() { "no member here".to_string() } else { seen_why.join(", ") });
    Vec::new()
}

/// Whether contract `id`'s answer is the round's: one it waits on, or one it asked of or put lately (never an
/// admission round's).
pub fn wants<H: Host>(h: &H, id: &[u8; 32]) -> bool {
    issued(h).contains(id)
        || load(h).is_some_and(|r| match &r.step {
            Step::Reading => r.reading.wants(id),
            Step::Writing { waiting, .. } => waiting.contains(id),
            Step::Recoding { rec, .. } => rec.wants(id),
        })
}

/// An ANSWER the round waits for (or one of its own it no longer needs: taken, nothing more). What to send next.
pub fn replied<H: Host>(h: &mut H, reply: Reply, now_ms: u64) -> Vec<Io> {
    let Some(c) = Held::of(h) else { return Vec::new() };
    let io = replied_(h, &c, reply, now_ms);
    issue(h, &c, io)
}
fn replied_<H: Host>(h: &mut H, c: &Held, reply: Reply, now_ms: u64) -> Vec<Io> {
    let Some(mut r) = load(h) else { return Vec::new() };
    r.moved = wakeups(h);
    match (r.step.clone(), reply) {
        (Step::Reading, Reply::Got { id, state }) => {
            let io = r.reading.got(&c.codes(), id, state);
            if !r.reading.done() {
                keep(h, Some(&r));
                return io;
            }
            plan(h, &c, r, now_ms)
        }
        (Step::Writing { rows, mut waiting, refused, blocks, after }, Reply::Put { id, ok } | Reply::Updated { id, ok }) if waiting.contains(&id) => {
            waiting.retain(|w| *w != id);
            let refused = refused || !ok;
            if !waiting.is_empty() {
                r.step = Step::Writing { rows, waiting, refused, blocks, after };
                keep(h, Some(&r));
                return Vec::new();
            }
            if refused {
                identity::upkeep_say(h, &r.member, "a write was refused (the network holds more): the next round reads again");
                return next_space(h, c, r.member, r.me.clone(), r.rest.clone());
            }
            // A listing, a flush's blocks or its step answered: the rows themselves now (a flush's step first).
            if !rows.is_empty() {
                return write_step(h, c, r, rows, after, blocks);
            }
            match after {
                After::Replan => plan(h, &c, r, now_ms),
                After::Row { burn } => {
                    let mut io = Vec::new();
                    if let Some((old_key, old, secret)) = burn {
                        let still = rekey::key_rows(&r.reading.rows("files")).iter().any(|k| k.v.get("key").and_then(|v| v.as_str()) == Some(&table::hex(&old_key)));
                        if !still {
                            if let Some(x) = unhex32(&secret) {
                                io.extend(old.iter().map(|a| Io::Put { code: crate::upkeep::Code::Piece, params: a.to_vec(), state: recode::burned(&x) }));
                                identity::upkeep_say(h, &r.member, &format!("{} old piece(s) burned", old.len()));
                            }
                        }
                    }
                    io.extend(next_due(h, &c, r, now_ms));
                    io
                }
            }
        }
        (Step::Recoding { mut rec, due }, reply) => {
            let res = match reply {
                Reply::Got { id, state } => rec.got(h, &c.piece, id, state),
                Reply::Put { id, ok } => rec.put(h, &c.piece, id, ok),
                Reply::Updated { .. } => Ok(Vec::new()),
            };
            match res {
                Err(why) => {
                    identity::upkeep_say(h, &r.member, &format!("{}…: not re-keyed — {why}", due.short()));
                    next_due(h, &c, r, now_ms)
                }
                Ok(io) => match rec.end() {
                    Some(End::Done { row, old }) => {
                        let burn = rec.burnable().map(|x| (old_key_of(&due), old, table::hex(&x)));
                        identity::upkeep_say(h, &r.member, &format!("{}…: re-keyed with no page open", due.short()));
                        write_row(h, &c, r, &due, &row, burn, now_ms)
                    }
                    _ => {
                        r.step = Step::Recoding { rec, due };
                        keep(h, Some(&r));
                        io
                    }
                },
            }
        }
        _ => {
            keep(h, Some(&r));
            Vec::new()
        }
    }
}

/// A row due, as the round keeps it (its value as JSON text: the round's state is bincode).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Task {
    id: String,
    row: String,
    public: bool,
}
impl Task {
    fn of(d: &rekey::Due) -> Task {
        Task { id: d.row.id.clone(), row: serde_json::Value::Object(d.row.v.clone()).to_string(), public: d.public }
    }
    fn short(&self) -> &str {
        &self.id[..self.id.len().min(8)]
    }
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let v: Option<Vec<u8>> = (0..32).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()).collect();
    v?.try_into().ok()
}
fn old_key_of(d: &Task) -> [u8; 32] {
    serde_json::from_str::<serde_json::Value>(&d.row).ok().and_then(|v| v["key"].as_str().and_then(unhex32)).unwrap_or_default()
}

/// THE PLAN: a new salt first; else the rows due whose turn it is now, one by one.
fn plan<H: Host>(h: &mut H, c: &Held, mut r: Round, now_ms: u64) -> Vec<Io> {
    // Whose acts are whose: each device read → its member's DID.
    let g = rekey::governance(&r.reading, &r.reading.writer_dids, &r.owner, now_ms);
    let members: Vec<String> = r.roster.iter().map(|(_, d)| d.clone()).collect();
    let p = rekey::plan(&r.reading, &g, &members, &r.me, now_ms);
    let st = rekey::salt(&r.reading.rows("files"));
    if let (Some(removals), Some(was)) = (p.rotate, st.clone()) {
        let Some(new) = identity::upkeep_random(h) else { return next_space(h, c, r.member, r.me.clone(), r.rest.clone()) };
        identity::upkeep_say(h, &r.member, "a member removed: a new salt (the space's files re-key)");
        let rows = rekey::rotation(&r.reading.rows("files"), &was, new, removals);
        return write_step(h, c, r, rows, After::Replan, false);
    }
    r.salt = st.map(|s| (s.s, s.n));
    // Each row's turn from when it last moved — and from when this member first saw it due.
    let mut first = seen(h);
    let mut queue = Vec::new();
    let mut soonest: Option<u64> = None;
    for d in &p.due {
        let k = format!("{}/{}", table::hex(&r.space), d.row.id);
        let at = *first.entry(k).or_insert(now_ms);
        match d.wait_from(at, now_ms) {
            0 => queue.push(Task::of(d)),
            wait => soonest = Some(soonest.map_or(wait, |s| s.min(wait))),
        }
    }
    h.set_secret(SEEN, &bincode::serialize(&first).expect("seen encodes"));
    // What it found, SAID — always (a round that does nothing is otherwise silent): what it read, what is due, and a row
    // not its turn yet brings the next round forward to that turn.
    {
        let name = r.name.clone();
        let files = r.reading.rows("files");
        let notes = if r.reading.notes.is_empty() { String::new() } else { format!(" ({})", r.reading.notes.join("; ")) };
        identity::upkeep_say(
            h,
            &r.member,
            &format!(
                "{name}: {} row(s) of files read{notes}; {} file(s) due, {} at my turn now{}",
                files.len(),
                p.due.len(),
                queue.len(),
                soonest.map(|s| format!(", the next in {} s", s / 1000)).unwrap_or_default()
            ),
        );
        if let Some(s) = soonest {
            let at = wakeups(h) + s.div_ceil(60_000) + 1;
            let was = h.get_secret(NEXT_AT).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes).unwrap_or(u64::MAX);
            h.set_secret(NEXT_AT, &at.min(was).to_le_bytes());
        }
    }
    r.queue = queue;
    next_due(h, c, r, now_ms)
}

/// The next due row re-keyed (or the next space).
fn next_due<H: Host>(h: &mut H, c: &Held, mut r: Round, now_ms: u64) -> Vec<Io> {
    while !r.queue.is_empty() {
        let due = r.queue.remove(0);
        let target = Target { salt: if due.public { None } else { r.salt } };
        if !due.public && r.salt.is_none() {
            continue;
        }
        match Recode::start(&c.piece, &due.id, &due.row, &target) {
            Ok((rec, io)) => {
                if rec.same() {
                    let End::Same { row } = rec.same_row() else { continue };
                    return write_row(h, c, r, &due, &row, None, now_ms);
                }
                r.step = Step::Recoding { rec, due };
                keep(h, Some(&r));
                return io;
            }
            Err(why) => identity::upkeep_say(h, &r.member, &format!("{}…: left to a page — {why}", due.short())),
        }
    }
    next_space(h, c, r.member, r.me.clone(), r.rest.clone())
}

/// A re-keyed row written (its page progress cleared in the same step), then its old pieces burned.
fn write_row<H: Host>(h: &mut H, c: &Held, r: Round, due: &Task, row: &str, burn: Option<([u8; 32], Vec<[u8; 32]>, String)>, now_ms: u64) -> Vec<Io> {
    let mut v: serde_json::Map<String, serde_json::Value> = serde_json::from_str(row).unwrap_or_default();
    v.insert("at".into(), now_ms.into());
    let rows = vec![
        (format!("k/{}", due.id).into_bytes(), Some(serde_json::Value::Object(v).to_string().into_bytes())),
        (format!("p/{}", due.id).into_bytes(), None),
    ];
    write_step(h, c, r, rows, After::Row { burn }, false)
}

/// One step of a write (listing first when the feed is new, a flush when due: then the rows again once answered).
/// `blocks_put`: a flush's blocks were confirmed (its step now).
fn write_step<H: Host>(h: &mut H, c: &Held, mut r: Round, rows: Vec<(Vec<u8>, Option<Vec<u8>>)>, after: After, blocks_put: bool) -> Vec<Io> {
    let nonce: [u8; 12] = identity::upkeep_random(h).map(|b| b[..12].try_into().expect("12")).unwrap_or([0; 12]);
    match write::rows(h, &r.member, &c.codes(), &mut r.reading, "files", &rows, nonce, blocks_put) {
        Ok(Next::Done(sent)) => {
            r.step = Step::Writing { rows: Vec::new(), waiting: sent.confirm, refused: false, blocks: false, after };
            keep(h, Some(&r));
            sent.io
        }
        Ok(Next::More(sent)) => {
            // A listing, a flush's blocks or its step: once answered, the same rows again.
            r.step = Step::Writing { rows, waiting: sent.confirm, refused: false, blocks: sent.blocks, after };
            keep(h, Some(&r));
            sent.io
        }
        Err(why) => {
            identity::upkeep_say(h, &r.member, &format!("not written: {why}"));
            next_space(h, c, r.member, r.me.clone(), r.rest.clone())
        }
    }
}
