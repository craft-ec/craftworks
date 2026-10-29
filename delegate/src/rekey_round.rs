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

use craftworks_identity::unhex32;
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
/// A re-key HELD for a clean key tree (`KEYS_WAIT`): begun THE MOMENT the keys round marks the tree clean (`clean_now`,
/// then `run_now` on the same answer) — waited on the condition, never on a wake-up (10-29, fz: held at wake-up 2, the
/// tree clean from wake-up 1, the next round due at 12; then one wake-up — a minute — per step).
const WAITS_CLEAN: &[u8] = b"identity_upkeep/rekey/waits-clean";
const RUN_NOW: &[u8] = b"identity_upkeep/rekey/run-now";
pub(crate) fn clean_now<H: Host>(h: &mut H) {
    if h.get_secret(WAITS_CLEAN).is_some_and(|b| b == [1]) {
        let w = identity::upkeep_wakeups(h);
        h.set_secret(NEXT_AT, &w.to_le_bytes());
        h.set_secret(WAITS_CLEAN, &[]);
        h.set_secret(RUN_NOW, &[1]);
    }
}
/// The re-key `clean_now` brought forward, begun now (on the answer that marked the tree clean).
pub fn run_now<H: Host>(h: &mut H) -> Vec<Io> {
    if !h.get_secret(RUN_NOW).is_some_and(|b| b == [1]) {
        return Vec::new();
    }
    h.set_secret(RUN_NOW, &[]);
    woke(h)
}

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
    roster: Vec<String>,
    reading: Reading,
    step: Step,
    /// Rows still due this pass (turn now).
    queue: Vec<Task>,
    /// When it last moved (wake-up count; the node's time, ms): one `upkeep::stuck` is dropped (the next starts over
    /// from a read).
    moved: u64,
    moved_ms: u64,
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


/// The contracts, as upkeep holds them.
pub struct Held {
    pub(crate) tail: Vec<u8>,
    pub(crate) bag: [u8; 32],
    sealed: [u8; 32],
    pub(crate) block: [u8; 32],
    pub(crate) piece: [u8; 32],
    pub(crate) idlog: [u8; 32],
    devices: [u8; 32],
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
            devices: h.get_secret(identity::UPKEEP_DEVICES)?.try_into().ok()?,
        })
    }
    pub(crate) fn codes(&self) -> Codes<'_> {
        Codes { tail: &self.tail, bag_hash: self.bag, sealed_hash: self.sealed, block_hash: self.block, idlog_hash: self.idlog, devices_hash: self.devices }
    }
}

/// A space's MEMBERS (DIDs): its LIST as the follower last read it whole (`upkeep::roster_kept` — K8.1, the list is who
/// is in a space) and its owner; and the epoch secrets the member holds. (Never the MLS group: a space made since K8.3
/// has none — 10-27, every such space read UNLOADABLE here and its files were never re-keyed with the pages closed.)
pub(crate) fn space_of<H: Host>(h: &H, member: &[u8; 32], m: &identity::Mandate) -> (Vec<String>, std::collections::BTreeMap<u64, [u8; 32]>) {
    let mut roster = crate::upkeep::roster_kept(h, member, &m.space);
    if !m.owner.is_empty() && !roster.contains(&m.owner) {
        roster.push(m.owner.clone());
    }
    // EVERY epoch held (the page keeps each one as the group moves, before any mandate names it): a row sealed with
    // an epoch newer than the mandate's group — a new salt after a removal — still reads.
    // (`identity::epochs_below`, the one walk of the kept epochs: counting up to the latest number walked millions with
    // BeeKEM's generation numbers, 2^24 and up — inventory #13, 10-19.)
    let epochs = identity::epochs_below(h, member, m.space, u64::MAX).into_iter().collect();
    (roster, epochs)
}

/// A space's round begun: its tables read.
fn begin<H: Host>(h: &mut H, c: &Held, member: [u8; 32], me: String, space: [u8; 32], rest: Vec<[u8; 32]>) -> Vec<Io> {
    let Some((_, spaces)) = identity::upkeep_mandate(h, &member) else { return Vec::new() };
    let Some(m) = spaces.iter().find(|m| m.space == space).cloned() else { return next_space(h, c, member, me, rest) };
    let (roster, epochs) = space_of(h, &member, &m);
    // Its WRITERS: each member's devices (a page writes a space under its member key, one per device).
    let dids: Vec<[u8; 32]> = roster.iter().filter_map(|d| craftworks_account::did_bytes(d)).collect();
    let (reading, io) = crate::upkeep::members_read(h, &member, &c.codes(), space, epochs, &dids, &["files", "acts", "pub-acts"]);
    let r = Round { member, me, rest, space, name: m.name.clone(), owner: m.owner.clone(), roster, reading, step: Step::Reading, queue: Vec::new(), moved: identity::upkeep_wakeups(h), moved_ms: identity::upkeep_now_ms(h), salt: None };
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


/// A WAKE-UP: a round begun (none running, the time come, a member whose page is away), or a stuck one dropped.
pub fn woke<H: Host>(h: &mut H) -> Vec<Io> {
    let w = identity::upkeep_wakeups(h);
    if let Some(r) = load(h) {
        if !crate::upkeep::stuck(h, r.moved, r.moved_ms) {
            return Vec::new();
        }
        crate::upkeep::followed(h, &r.member, &r.space, craftworks_rules::steps::STALLED, &format!("{}: its files' re-key round did not finish: dropped", r.name));
        keep(h, None);
    }
    let last = h.get_secret(LAST).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes).unwrap_or(0);
    let next_at = h.get_secret(NEXT_AT).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes).unwrap_or(u64::MAX);
    if last != 0 && w.saturating_sub(last) < EVERY && w < next_at {
        return Vec::new();
    }
    h.set_secret(NEXT_AT, &u64::MAX.to_le_bytes());
    let Some(c) = Held::of(h) else {
        for m in identity::upkeep_members(h) {
            crate::upkeep::followed(h, &m, &craftworks_rules::steps::ACCOUNT, craftworks_rules::steps::REKEY_NO_CODES, "the contracts a re-key needs were not handed over yet (a page hands them)");
        }
        return Vec::new();
    };
    // Members take turns: the one after the member served last first.
    let mut members = identity::upkeep_members(h);
    if let Some(i) = h.get_secret(LAST_MEMBER).and_then(|l| members.iter().position(|m| m.as_slice() == l.as_slice())) {
        members.rotate_left(i + 1);
    }
    // (A member passed over: why, recorded — quiet; the same reason once.)
    let idle = |h: &mut H, m: &[u8; 32], why: &str| crate::upkeep::followed(h, m, &craftworks_rules::steps::ACCOUNT, craftworks_rules::steps::REKEY_IDLE, why);
    for member in members {
        if identity::upkeep_since_tick(h, &member) < crate::upkeep::PAGE_AWAY {
            idle(h, &member, "a page is open: it re-keys");
            continue;
        }
        let Some((me, spaces)) = identity::upkeep_mandate(h, &member) else {
            idle(h, &member, "no mandate handed over");
            continue;
        };
        let mut all: Vec<[u8; 32]> = spaces.iter().filter(|m| m.kind == "server").map(|m| m.space).collect();
        if all.is_empty() {
            idle(h, &member, "no spaces to re-key");
            continue;
        }
        h.set_secret(LAST, &w.to_le_bytes());
        h.set_secret(LAST_MEMBER, &member);
        let first = all.remove(0);
        let io = begin(h, &c, member, me, first, all);
        return issue(h, &c, io);
    }
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
    // R2 (FOUNDATION "The node's rounds"): an answer moves the round — and counts as it moving (R3) — only when its
    // CURRENT step asked for it; one it issued earlier, late or duplicated, moves nothing and refreshes nothing.
    let own = match (&r.step, &reply) {
        (Step::Reading, Reply::Got { id, .. }) => r.reading.wants(id),
        (Step::Writing { waiting, .. }, Reply::Put { id, .. } | Reply::Updated { id, .. }) => waiting.contains(id),
        (Step::Recoding { rec, .. }, Reply::Got { id, .. } | Reply::Put { id, .. } | Reply::Updated { id, .. }) => rec.wants(id),
        (Step::Reading, Reply::Put { .. } | Reply::Updated { .. }) | (Step::Writing { .. }, Reply::Got { .. }) => false,
    };
    if !own {
        return Vec::new();
    }
    r.moved = identity::upkeep_wakeups(h);
    r.moved_ms = identity::upkeep_now_ms(h);
    match (r.step.clone(), reply) {
        (Step::Reading, Reply::Got { id, state }) => {
            let io = crate::upkeep::read_got(h, &mut r.reading, &r.space, &c.codes(), id, state);
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
                crate::upkeep::followed(h, &r.member, &r.space, craftworks_rules::steps::FILES_WRITE_REFUSED, &format!("{}: a write was refused (the network holds more): the next round reads again", r.name));
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
                                crate::upkeep::followed(h, &r.member, &r.space, craftworks_rules::steps::FILES_BURNED, &format!("{}: {} old piece(s) burned", r.name, old.len()));
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
                    crate::upkeep::followed(h, &r.member, &r.space, craftworks_rules::steps::FILES_NOT_REKEYED, &format!("{}: {}…: not re-keyed — {why}", r.name, due.short()));
                    next_due(h, &c, r, now_ms)
                }
                Ok(io) => match rec.end() {
                    Some(End::Done { row, old }) => {
                        let burn = rec.burnable().map(|x| (old_key_of(&due), old, table::hex(&x)));
                        let root_id = rec.root.map(|root| table::hex(&crate::upkeep::id_of(&c.piece, &craftworks_files::hashed_address(&rec.new_key, &root)))).unwrap_or_default();
                        crate::upkeep::followed(h, &r.member, &r.space, craftworks_rules::steps::FILES_REKEYED, &format!("{}: {}…: re-keyed with no page open (key {}…, root contract {}…)", r.name, due.short(), &table::hex(&rec.new_key)[..8], &root_id[..root_id.len().min(12)]));
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
        // (Not its own: answered above, R2. Named so the table stays whole.)
        (Step::Reading, Reply::Put { .. } | Reply::Updated { .. }) | (Step::Writing { .. }, Reply::Got { .. } | Reply::Put { .. } | Reply::Updated { .. }) => Vec::new(),
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

fn old_key_of(d: &Task) -> [u8; 32] {
    serde_json::from_str::<serde_json::Value>(&d.row).ok().and_then(|v| v["key"].as_str().and_then(unhex32)).unwrap_or_default()
}

/// THE PLAN: a new salt first; else the rows due whose turn it is now, one by one.
fn plan<H: Host>(h: &mut H, c: &Held, mut r: Round, now_ms: u64) -> Vec<Io> {
    // Whose acts are whose: each device read → its member's DID.
    let g = rekey::governance(&r.reading, &r.reading.writer_dids, &r.owner, now_ms);
    let p = rekey::plan(&r.reading, &g, &r.roster, &r.me, now_ms);
    let st = rekey::salt(&r.reading.rows("files"));
    if let (Some(removals), Some(was)) = (p.rotate, st.clone()) {
        // (THE KEY TREE CLEAN FIRST — the page's gate too: the new salt sealed with the generation written with only once
        // the tree found clean at THAT mark placed no account off the list now; else the keys round goes first — 10-28
        // review: the salt was sealed with a generation the one removed held.)
        let at = crate::account_round::clean_at(h, &r.member, &r.space);
        let latest = identity::latest_epoch(h, &r.member, r.space);
        let clean = at.as_ref().is_some_and(|(n, placed)| latest == Some(*n) && placed.iter().all(|d| r.roster.contains(d)));
        if !clean {
            // (Which way not clean — said: no clean merge marked, the mark on another generation, or one off the list.)
            let why = match &at {
                None => "no merge with nothing waiting marked yet".to_string(),
                Some((n, _)) if latest != Some(*n) => format!("marked at {n}, writing {latest:?}"),
                Some((_, placed)) => format!("{} placed off the list", placed.iter().filter(|d| !r.roster.contains(d)).count()),
            };
            crate::upkeep::followed(h, &r.member, &r.space, craftworks_rules::steps::KEYS_WAIT, &format!("{}: a member removed — the new salt waits for the key tree to be clean ({why})", r.name));
            h.set_secret(WAITS_CLEAN, &[1]);
            return next_space(h, c, r.member, r.me.clone(), r.rest.clone());
        }
        let Some(new) = identity::upkeep_random(h) else { return next_space(h, c, r.member, r.me.clone(), r.rest.clone()) };
        crate::upkeep::followed(h, &r.member, &r.space, craftworks_rules::steps::FILES_NEW_SALT, &format!("{}: a member removed: a new salt (the space's files re-key)", r.name));
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
        let notes = if r.reading.feeds_unread.is_empty() { String::new() } else { format!(" ({:?})", r.reading.feeds_unread) };
        crate::upkeep::followed(
            h,
            &r.member,
            &r.space,
            craftworks_rules::steps::FILES_READ,
            &format!(
                "{name}: {} row(s) of files read{notes}; {} file(s) due, {} at my turn now{}",
                files.len(),
                p.due.len(),
                queue.len(),
                soonest.map(|s| format!(", the next in {} s", s / 1000)).unwrap_or_default()
            ),
        );
        if let Some(s) = soonest {
            let at = identity::upkeep_wakeups(h) + s.div_ceil(60_000) + 1;
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
                if !crate::upkeep::creates() {
                    crate::upkeep::followed(h, &r.member, &r.space, craftworks_rules::steps::FILES_LEFT, &format!("{}: {}…: its new pieces would stay on this node", r.name, due.short()));
                    continue;
                }
                r.step = Step::Recoding { rec, due };
                keep(h, Some(&r));
                return io;
            }
            Err(why) => crate::upkeep::followed(h, &r.member, &r.space, craftworks_rules::steps::FILES_LEFT, &format!("{}: {}…: {why}", r.name, due.short())),
        }
    }
    next_space(h, c, r.member, r.me.clone(), r.rest.clone())
}

/// A re-keyed row written (its page progress cleared in the same step), then its old pieces burned.
fn write_row<H: Host>(h: &mut H, c: &Held, r: Round, due: &Task, row: &str, burn: Option<([u8; 32], Vec<[u8; 32]>, String)>, now_ms: u64) -> Vec<Io> {
    // (`rekey::key_row_write`, the page's too.)
    let (k, v) = craftworks_rules::rekey::key_row_write(&due.id, &serde_json::from_str(row).unwrap_or_default(), now_ms);
    let rows = vec![
        (k.into_bytes(), Some(v.into_bytes())),
        (format!("p/{}", due.id).into_bytes(), None),
    ];
    write_step(h, c, r, rows, After::Row { burn }, false)
}

/// One step of a write (listing first when the feed is new, a flush when due: then the rows again once answered).
/// `blocks_put`: a flush's blocks were confirmed (its step now).
fn write_step<H: Host>(h: &mut H, c: &Held, mut r: Round, rows: Vec<(Vec<u8>, Option<Vec<u8>>)>, after: After, blocks_put: bool) -> Vec<Io> {
    let nonce: [u8; 12] = identity::upkeep_random(h).map(|b| b[..12].try_into().expect("12")).unwrap_or([0; 12]);
    match write::rows(h, &r.member, &c.codes(), &mut r.reading, "files", &rows, nonce, blocks_put, crate::upkeep::creates()) {
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
            crate::upkeep::followed(h, &r.member, &r.space, craftworks_rules::steps::FILES_NOT_WRITTEN, &format!("{}: {why}", r.name));
            next_space(h, c, r.member, r.me.clone(), r.rest.clone())
        }
    }
}
