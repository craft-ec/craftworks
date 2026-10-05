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
/// Wake-ups between rounds (one a minute: every ten minutes).
pub const EVERY: u64 = 10;
/// A round that has not moved for this many wake-ups is dropped (the next starts over from a read).
const STUCK: u64 = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
enum Step {
    Reading,
    /// A write's step sent: waiting on its answer. `rows`: still to write after it (it was a listing or a flush
    /// step) — none: it was the rows' own; `after`: what comes once they are written.
    Writing { rows: Vec<(Vec<u8>, Option<Vec<u8>>)>, confirm: [u8; 32], after: After },
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
    owner: String,
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
}
impl Held {
    pub fn of<H: Host>(h: &H) -> Option<Held> {
        Some(Held {
            tail: h.get_secret(identity::UPKEEP_TAIL)?,
            bag: contract_keys::code_hash(&h.get_secret(identity::UPKEEP_BAG)?),
            sealed: contract_keys::code_hash(&h.get_secret(identity::UPKEEP_SEALED)?),
            block: h.get_secret(identity::UPKEEP_BLOCK)?.try_into().ok()?,
            piece: contract_keys::code_hash(&h.get_secret(identity::UPKEEP_PIECE)?),
        })
    }
    fn codes(&self) -> Codes<'_> {
        Codes { tail: &self.tail, bag_hash: self.bag, sealed_hash: self.sealed, block_hash: self.block }
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
    let epochs = (0..=m.epoch).filter_map(|e| Some((e, identity::epoch_secret(h, member, m.space, e)?))).collect();
    Some((roster, epochs))
}

/// A space's round begun: its tables read.
fn begin<H: Host>(h: &mut H, c: &Held, member: [u8; 32], me: String, space: [u8; 32], rest: Vec<[u8; 32]>) -> Vec<Io> {
    let Some((_, spaces)) = identity::upkeep_mandate(h, &member) else { return Vec::new() };
    let Some(m) = spaces.iter().find(|m| m.space == space).cloned() else { return next_space(h, c, member, me, rest) };
    let Some((roster, epochs)) = space_of(h, &member, &m) else { return next_space(h, c, member, me, rest) };
    let (reading, io) = Reading::new(&c.codes(), space, epochs, roster.iter().map(|(w, _)| *w).collect(), &["files", "acts", "pub-acts"]);
    let r = Round { member, me, rest, space, owner: m.owner.clone(), roster, reading, step: Step::Reading, queue: Vec::new(), moved: wakeups(h), salt: None };
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
    let w = wakeups(h);
    if let Some(r) = load(h) {
        if w.saturating_sub(r.moved) < STUCK {
            return Vec::new();
        }
        identity::upkeep_say(h, &r.member, "a re-key round did not finish: dropped");
        keep(h, None);
    }
    let last = h.get_secret(LAST).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes).unwrap_or(0);
    if last != 0 && w.saturating_sub(last) < EVERY {
        return Vec::new();
    }
    let Some(c) = Held::of(h) else { return Vec::new() };
    for member in identity::upkeep_members(h) {
        if identity::upkeep_since_tick(h, &member) < crate::upkeep::PAGE_AWAY {
            continue;
        }
        let Some((me, spaces)) = identity::upkeep_mandate(h, &member) else { continue };
        let mut all: Vec<[u8; 32]> = spaces.iter().filter(|m| m.kind == "server").map(|m| m.space).collect();
        if all.is_empty() {
            continue;
        }
        h.set_secret(LAST, &w.to_le_bytes());
        let first = all.remove(0);
        return begin(h, &c, member, me, first, all);
    }
    Vec::new()
}

/// Whether the round waits on contract `id` (its answer is the round's, not an admission's).
pub fn wants<H: Host>(h: &H, id: &[u8; 32]) -> bool {
    load(h).is_some_and(|r| match &r.step {
        Step::Reading => r.reading.wants(id),
        Step::Writing { confirm, .. } => confirm == id,
        Step::Recoding { rec, .. } => rec.wants(id),
    })
}

/// An ANSWER the round waits for. What to send next.
pub fn replied<H: Host>(h: &mut H, reply: Reply, now_ms: u64) -> Vec<Io> {
    let Some(mut r) = load(h) else { return Vec::new() };
    let Some(c) = Held::of(h) else { return Vec::new() };
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
        (Step::Writing { rows, confirm, after }, Reply::Put { id, ok } | Reply::Updated { id, ok }) if id == confirm => {
            if !ok {
                identity::upkeep_say(h, &r.member, "a write was refused (the network holds more): the next round reads again");
                return next_space(h, &c, r.member, r.me.clone(), r.rest.clone());
            }
            // A listing or flush step answered: the rows themselves now.
            if !rows.is_empty() {
                return write_step(h, &c, r, rows, after);
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
    let g = rekey::governance(&r.reading, &r.roster, &r.owner, now_ms);
    let members: Vec<String> = r.roster.iter().map(|(_, d)| d.clone()).collect();
    let p = rekey::plan(&r.reading, &g, &members, &r.me, now_ms);
    let st = rekey::salt(&r.reading.rows("files"));
    if let (Some(removals), Some(was)) = (p.rotate, st.clone()) {
        let Some(new) = identity::upkeep_random(h) else { return next_space(h, c, r.member, r.me.clone(), r.rest.clone()) };
        identity::upkeep_say(h, &r.member, "a member removed: a new salt (the space's files re-key)");
        let rows = rekey::rotation(&was, new, removals);
        return write_step(h, c, r, rows, After::Replan);
    }
    r.salt = st.map(|s| (s.s, s.n));
    r.queue = p.due.iter().filter(|d| d.wait == 0).map(Task::of).collect();
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
    write_step(h, c, r, rows, After::Row { burn })
}

/// One step of a write (listing first when the feed is new: then the rows again once it is answered).
fn write_step<H: Host>(h: &mut H, c: &Held, mut r: Round, rows: Vec<(Vec<u8>, Option<Vec<u8>>)>, after: After) -> Vec<Io> {
    let nonce: [u8; 12] = identity::upkeep_random(h).map(|b| b[..12].try_into().expect("12")).unwrap_or([0; 12]);
    match write::rows(h, &r.member, &c.codes(), &mut r.reading, "files", &rows, nonce) {
        Ok(Next::Done(sent)) => {
            r.step = Step::Writing { rows: Vec::new(), confirm: sent.confirm, after };
            keep(h, Some(&r));
            sent.io
        }
        Ok(Next::More(sent)) => {
            // A listing (or a flush) step: once answered, the same rows again.
            r.step = Step::Writing { rows, confirm: sent.confirm, after };
            keep(h, Some(&r));
            sent.io
        }
        Err(why) => {
            identity::upkeep_say(h, &r.member, &format!("not written: {why}"));
            next_space(h, c, r.member, r.me.clone(), r.rest.clone())
        }
    }
}
