//! `Db`: collections of typed records over a [`Store`].
//!
//! Keys follow the architecture's keyspace (§5):
//! `0x01 ‖ domain ‖ 0x00 ‖ rkey(16)` for records, `0x00 "schema" 0x00 domain` for
//! a domain's schema — so the schema travels with the data it types.

use crate::definition::DefKey;
use crate::id::{self, Env, IdGen, Loc, RKey};
use core_types::name::SystemDomain;
use crate::record;
use crate::schema::Schema;
use crate::store::{sorted_edits, Edit, IdWidth, Reads, Store, StoreError};
use protocol::Expect;
use freenet_prolly::node::{MAX_KEY, MAX_VALUE};
use serde::Serialize;
use serde_json::{Map, Value};

/// The longest domain as the TREE stores it: `<app>.<name>`, an app id
/// (`app::MAX_APP`) and a name (`app::MAX_NAME`) with a `.` between.
///
/// Not a key budget in itself: a record key is a tag, this, a separator and
/// up to two 16-byte ids, far inside `MAX_KEY` (512) — asserted in
/// `tests/app_namespace.rs`. It was 32 and checked on the PREFIXED name, so
/// an app's id ate into every domain it named (sdk#276).
pub const MAX_DOMAIN: usize = crate::app::MAX_APP + 1 + crate::app::MAX_NAME;
const T_SYSTEM: u8 = 0x00;
const T_RECORD: u8 = 0x01;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Record {
    pub id: String,
    pub created: u64,
    pub updated: u64,
    pub fields: Map<String, Value>,
    /// What THIS record's own write is doing.
    ///
    /// Metadata on the row, never a field: it describes the record rather
    /// than being part of it, and a schema that had to declare it would make
    /// every app's data shape depend on how it happened to be stored.
    ///
    /// Over the in-memory store this is always `Clean`, which is the truth —
    /// a write there is applied the moment it is made, so there is never one
    /// in flight to report.
    #[serde(rename = "state", serialize_with = "ser_state")]
    pub state: crate::store::RowState,
}

fn ser_state<S: serde::Serializer>(
    s: &crate::store::RowState,
    ser: S,
) -> std::result::Result<S::Ok, S::Error> {
    // The stable CODE, never the Rust variant name: a row branches on this,
    // and a rename would change behaviour silently.
    ser.serialize_str(s.code())
}

/// What [`Db::create_at`] did: made the record, or found one already there.
///
/// An outcome and not an error, because both answers are ordinary — a second
/// handoff of the same row is EXPECTED to find its copy — and both carry the
/// record. What `Exists` never means is "overwritten": the held record comes
/// back exactly as it is stored, and deciding whether to update it is the
/// caller's, which is the only one that knows what the copy was copied from.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "outcome", content = "record", rename_all = "lowercase")]
pub enum CreateAt {
    Created(Record),
    Exists(Record),
}

/// How far ahead of this device's clock a slot's time may be.
///
/// A slot's first 8 bytes ARE the record's `created`, and every newest-first
/// list sorts by them — so a slot dated in the future would sit at the top of
/// every such list until that time passed, for ever if it is far enough out.
/// The allowance is for two devices' clocks disagreeing, not for a date the
/// caller chose: a policy value, five minutes, not a measured one.
pub const SLOT_SKEW_MS: u64 = 5 * 60 * 1000;

#[derive(Default, Clone, Copy)]
pub struct Scan {
    pub reverse: bool,
    /// 0 = no limit.
    pub limit: usize,
    /// Resume strictly after this record (in scan direction).
    pub after: Option<RKey>,
}

pub struct Db<S: Store + Reads, E: Env> {
    store: S,
    env: E,
    ids: IdGen,
    /// Signing identity. All zero until identities land (phase 3).
    author: [u8; 32],
    /// What each of this Db's writes MEANT, by the store's write id, while it
    /// is pending: the patch an update made, the fields a define appended. On
    /// a Conflict the operation is RE-RUN on the new base (sdk#143/#144).
    ops: std::collections::BTreeMap<u64, Op>,
    /// Conflicted chains being re-run, oldest first.
    reruns: std::collections::VecDeque<Rerun>,
    /// SCHEMA WRITES ARE DEFERRED (sdk#350): set once, at open, on a published
    /// app's own tree (an asked session). A define then commits only in the
    /// company of a data write, so opening an app commits nothing. A builder's
    /// Db never sets it: a publisher's schema edit commits as it always did.
    defer_schema: bool,
}

/// What an update or a define meant, so a Conflict can re-run it.
#[derive(Debug, Clone)]
enum Op {
    Patch { domain: String, loc: Loc, base: Map<String, Value>, patch: Map<String, Value> },
    Append { domain: String, appended: Vec<crate::schema::Field> },
}

/// A conflicted chain: its operations in the order they were made, and the
/// latest moment it may wait for a reload.
#[derive(Debug)]
struct Rerun {
    ops: std::collections::VecDeque<(u64, Op)>,
    /// The tries the chain had spent in the ENGINE (dead-commit re-sends and
    /// earlier re-runs alike): ONE budget, the engine's `max_write_tries`
    /// (sdk#265). `Db` keeps no count of its own.
    tries: u32,
}

/// What a re-run could NOT keep (sdk#143/#144), for the app to tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RerunEvent {
    /// These fields of the write were changed elsewhere too: the other write
    /// stands, this one's value at them was dropped.
    Dropped { write_id: u64, fields: Vec<String> },
    /// The record was deleted elsewhere: nothing of the write was re-applied.
    Deleted { write_id: u64 },
    /// Nothing of the write could be re-applied: named why.
    Failed { write_id: u64, reason: String },
}

/// One call of [`Db::rerun`]: what it needs loaded before it can go on, and
/// what it could not keep.
#[derive(Debug, Default)]
pub struct RerunStep {
    pub load: Vec<(Vec<u8>, Vec<u8>)>,
    pub events: Vec<RerunEvent>,
    /// The conflicted writes this call took up to re-run. Their raw Conflict
    /// is not the app's news: the re-run's outcome is.
    pub taken: Vec<u64>,
}

/// Why a `Db` call did not answer.
///
/// **`NotLoaded` is a variant and not a message.** It is the one error an app
/// RECOVERS from rather than reports: the range has not been read yet, it is
/// not known to be empty, and the fix is a reload. Every other refusal means
/// the caller asked for something it may not have — a domain that is not
/// defined, a key over the limit, a record that will not decode — and no
/// reload changes that.
///
/// It was a `String`. A browser app would have had to match on the text of a
/// message to tell "read me again" from "you are wrong", which breaks the
/// first time anybody rewords it, and silently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbError {
    /// The store has not loaded the range this read needed. **Not empty.**
    ///
    /// Carries the SPAN the read actually needed, because the caller's whole
    /// recovery is to load it — and an error that says "something was not
    /// loaded" without saying what leaves the caller to guess a range, which
    /// is how a recovery becomes either a spin or a full download.
    ///
    /// The span is the read's OWN span and nothing wider: `[key, key+1)` for
    /// a get, `[lo, hi)` for a scan. Widening it to the whole domain would
    /// make one `get` fetch every record in it, which is the page-freezing
    /// version of being helpful; the loaded-interval bookkeeping means a
    /// later, wider read asks for whatever is still missing.
    NotLoaded { lo: Vec<u8>, hi: Vec<u8> },
    /// The store could not reach a block this read needed. The key may well
    /// exist — this is not an absence either.
    Unavailable,
    /// A key or a record is over the store's limit. The caller asked for
    /// something that will never fit; no reload helps.
    TooLarge(String),
    /// A domain that is not defined, or a name that is not a domain.
    NotDefined(String),
    /// Anything else the caller could have got right, in words.
    Refused(String),
    /// The STORE refused this write before applying it anywhere — the queue
    /// full, no session, too large to send (craftworks-sdk#180; R-b).
    /// NOTHING WAS WRITTEN. Each reason has its own code, because each asks something
    /// different of the caller — wait, reload, split.
    WriteRefused(crate::store::Refused),
    /// NOT DECIDED YET whether this session may write here: the node's
    /// signer has not answered whose node it is (still asking, or silent --
    /// rule 8: silence is not an answer). Nothing was written; the SAME call
    /// made once the signer answers takes the answered branch. A caller
    /// WAITS -- on the session's own wake, with no deadline -- never gives
    /// up on it (`engine-db.js`, the one wait path it shares with
    /// `QUEUE_FULL`).
    NotDecided(String),
}

impl DbError {
    /// A STABLE code, from a fixed list, for a caller to branch on.
    ///
    /// This is what crosses the JavaScript boundary. An app must be able to
    /// tell "read me again" from "you are wrong" — and the only alternative
    /// to a code is matching on the text of a message, which breaks the first
    /// time anybody rewords it, and breaks silently. Nothing downstream, in
    /// `wrap.js` or in an app, ever branches on the message.
    ///
    /// The list is fixed and exhaustive: adding a variant has to add a code
    /// here, and the compiler says so.
    pub fn code(&self) -> &'static str {
        match self {
            DbError::NotLoaded { .. } => "NOT_LOADED",
            DbError::Unavailable => "UNAVAILABLE",
            DbError::TooLarge(_) => "TOO_LARGE",
            DbError::NotDefined(_) => "NOT_DEFINED",
            DbError::Refused(_) => "REFUSED",
            DbError::NotDecided(_) => "NOT_DECIDED",
            DbError::WriteRefused(r) => match r {
                crate::store::Refused::QueueFull { .. } => "QUEUE_FULL",
                crate::store::Refused::TooLargeToSend { .. } => "TOO_LARGE_TO_SEND",
                crate::store::Refused::TooLarge { .. } => "TOO_LARGE",
                crate::store::Refused::NoSession => "NO_SESSION",
                crate::store::Refused::Unread => "UNREAD",
            },
        }
    }

    /// Whether the SAME write, made again once the queue has drained, may
    /// succeed (`QUEUE_FULL`, R-b). Not a reload -- the recovery is to wait
    /// for writes to publish and retry.
    pub fn is_retryable(&self) -> bool {
        matches!(self, DbError::WriteRefused(crate::store::Refused::QueueFull { .. }))
    }

    /// The bound a `QUEUE_FULL` refusal met: the bytes the page's queue holds.
    pub fn cap(&self) -> Option<usize> {
        match self {
            DbError::WriteRefused(crate::store::Refused::QueueFull { limit, .. }) => Some(*limit),
            _ => None,
        }
    }

    /// Whether a RELOAD is the recovery. The only two that a reload can fix
    /// are the two that are about reaching data rather than about the ask.
    pub fn is_transient(&self) -> bool {
        matches!(self, DbError::NotLoaded { .. } | DbError::Unavailable)
    }
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbError::NotLoaded { .. } => f.write_str(
                "this range has not been loaded yet — reload it; it is not known to be empty",
            ),
            DbError::Unavailable => f.write_str(
                "the engine could not reach a block this read needed; the key may well exist",
            ),
            DbError::TooLarge(m) | DbError::NotDefined(m) | DbError::Refused(m) => f.write_str(m),
            DbError::NotDecided(m) => write!(f, "not decided yet: {m}"),
            DbError::WriteRefused(r) => write!(f, "not written: {r}"),
        }
    }
}

impl std::error::Error for DbError {}

/// So every `?` over a `String` error in this module keeps working, and only
/// the two store-read funnels have to say which variant they mean.
impl From<String> for DbError {
    fn from(m: String) -> Self {
        DbError::Refused(m)
    }
}

impl From<&str> for DbError {
    fn from(m: &str) -> Self {
        DbError::Refused(m.to_string())
    }
}

impl DbError {
    /// A store's refusal, told which span the read was for.
    ///
    /// Kept exhaustive so a new `StoreError` has to be classified here rather
    /// than falling into `Refused` and quietly becoming un-recoverable.
    fn from_store(e: StoreError, lo: &[u8], hi: &[u8]) -> Self {
        match e {
            StoreError::NotLoaded => DbError::NotLoaded {
                lo: lo.to_vec(),
                hi: hi.to_vec(),
            },
            StoreError::Unavailable => DbError::Unavailable,
            StoreError::NoAnswer => DbError::Refused(e.to_string()),
        }
    }

    /// The span a `NotLoaded` needs loaded, if that is what this is.
    pub fn needs(&self) -> Option<(&[u8], &[u8])> {
        match self {
            DbError::NotLoaded { lo, hi } => Some((lo, hi)),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, DbError>;

fn check_domain(d: &str) -> Result<()> {
    if core_types::name::domain_ok(d, MAX_DOMAIN) {
        Ok(())
    } else {
        Err(DbError::NotDefined(format!(
            "domain `{d}` must be 1–{MAX_DOMAIN} of a-z 0-9 _ - ."
        )))
    }
}

/// The smallest key strictly greater than `k`.
fn succ(k: &[u8]) -> Vec<u8> {
    let mut out = k.to_vec();
    out.push(0);
    out
}

/// `T_RECORD | domain | 0`: the head every one of `domain`'s record keys starts with ([`record_key`]'s own). The ONE
/// form of it: whatever recognises a domain's records by key (ws-lose `--domain`) takes it from here.
pub fn record_prefix(domain: &str) -> Vec<u8> {
    let mut k = vec![T_RECORD];
    k.extend_from_slice(domain.as_bytes());
    k.push(0);
    k
}
/// `T_RECORD | domain | 0 | [parent] | rkey`.
///
/// The parent, when there is one, sits BEFORE the rkey, which is the whole
/// point: it makes every child of one parent a contiguous band, so reading
/// them is a bounded prefix scan instead of a scan of the domain filtered by
/// a field (craftworks-sdk#122).
pub fn record_key(domain: &str, at: impl Into<Loc>) -> Vec<u8> {
    let loc = at.into();
    let mut k = record_prefix(domain);
    if let Some(parent) = &loc.parent {
        k.extend_from_slice(parent);
    }
    k.extend_from_slice(&loc.rkey);
    k
}

/// Every key under `parent` in `domain`, as a half-open range.
fn parent_span(domain: &str, parent: &RKey) -> (Vec<u8>, Vec<u8>) {
    let mut lo = record_prefix(domain);
    lo.extend_from_slice(parent);
    // The band is exactly the 16-byte rkeys following this prefix, so the end
    // is the prefix with a 16-byte all-ones tail exceeded -- simplest correct
    // form is to bump the prefix itself.
    let mut hi = lo.clone();
    for b in hi.iter_mut().rev() {
        if *b == 0xff { *b = 0; } else { *b += 1; break; }
    }
    (lo, hi)
}
/// An ORDINARY write's domain: never under the reserved prefix (ARCHITECTURE §19). The definition doors below reach
/// the one commit path (`Db::write`) without this; every other write goes through it, whichever door it came in by.
fn ordinary(domain: &str) -> Result<()> {
    if core_types::name::reserved(domain) {
        return Err(crate::app::reserved_refusal(domain));
    }
    Ok(())
}

fn schema_key(domain: &str) -> Vec<u8> {
    let mut k = vec![T_SYSTEM];
    k.extend_from_slice(b"schema\0");
    k.extend_from_slice(domain.as_bytes());
    k
}
/// Split a stored key back into the parent (if the domain has one) and rkey.
fn loc_of_key(schema: &Schema, p: &[u8], k: &[u8]) -> Result<Loc> {
    let tail = &k[p.len()..];
    let want = if schema.parent.is_some() { 32 } else { 16 };
    if tail.len() != want {
        return Err(DbError::from("corrupt record key".to_string()));
    }
    Ok(if schema.parent.is_some() {
        Loc::under(tail[..16].try_into().unwrap(), tail[16..].try_into().unwrap())
    } else {
        Loc::bare(tail.try_into().unwrap())
    })
}

/// The parent a record declares, read from the field the schema names.
fn parent_of(schema: &Schema, fields: &Map<String, Value>) -> Result<Option<RKey>> {
    let Some(pf) = &schema.parent else { return Ok(None) };
    let v = fields.get(pf).and_then(|v| v.as_str()).ok_or_else(|| {
        DbError::Refused(format!("`{pf}` names this record's parent and is required"))
    })?;
    id::from_hex(v).map(Some).ok_or_else(|| {
        DbError::Refused(format!(
            "`{pf}` must be a record id (32 hex characters); got `{v}`"
        ))
    })
}

/// The smallest key greater than every key starting with `p` (p ends in 0x00).
fn upper(p: &[u8]) -> Vec<u8> {
    let mut h = p.to_vec();
    *h.last_mut().unwrap() += 1;
    h
}

impl<S: Store + Reads, E: Env> Db<S, E> {
    /// Every key a domain's records live under, as `[lo, hi)`.
    ///
    /// The ONE place that turns a name into a range. A caller that built this
    /// itself would be encoding the key layout — which is the thing this
    /// boundary exists to hide, and which could then never change without
    /// breaking every caller that had hard-coded it.
    ///
    /// It takes no `&self` because it is a property of the layout and not of
    /// any particular database, and it does not check that the domain is
    /// DEFINED: that is a question about this database's contents, and the
    /// caller that needs it asks separately.
    /// Every key one PARENT's children live under, in a domain that declares
    /// a parent, as `[lo, hi)` — the band `Db::children` reads.
    pub fn parent_range(domain: &str, parent: &RKey) -> (Vec<u8>, Vec<u8>) {
        parent_span(domain, parent)
    }

    /// What a session WATCHES for one binding: the whole domain, or one
    /// parent's band of it (craftworks-sdk#137). A string, so it can cross to
    /// JavaScript and back as an opaque name; its layout is known only here
    /// and in [`Db::watch_range`]. `#` cannot occur in a domain name.
    /// The domain a definition is held in, as the tree stores it (`<app>.craftworks.<which>`, or unprefixed with no
    /// app): for a session to WATCH it (§19, `Session::bind_definition`), the one statement of the name, never spelled
    /// by a caller.
    pub fn definition_domain(app: Option<&str>, which: SystemDomain) -> String {
        system_name(app, which)
    }

    pub fn watch_key(domain: &str, parent: Option<&RKey>) -> String {
        match parent {
            Some(p) => format!("{domain}#{}", id::to_hex(p)),
            None => domain.to_string(),
        }
    }

    /// The range a [`Db::watch_key`] names, or `None` for a string that is
    /// not one.
    pub fn watch_range(key: &str) -> Option<(Vec<u8>, Vec<u8>)> {
        match key.split_once('#') {
            Some((domain, parent)) => {
                check_domain(domain).ok()?;
                Some(Self::parent_range(domain, &id::from_hex(parent)?))
            }
            None => {
                check_domain(key).ok()?;
                Some(Self::domain_range(key))
            }
        }
    }

    pub fn domain_range(domain: &str) -> (Vec<u8>, Vec<u8>) {
        let lo = record_prefix(domain);
        let hi = upper(&lo);
        (lo, hi)
    }

    /// The domain a RECORD key is in, or `None` for any other key (a schema,
    /// an index entry).
    pub fn domain_of_key(key: &[u8]) -> Option<String> {
        let rest = key.strip_prefix(&[T_RECORD])?;
        let z = rest.iter().position(|b| *b == 0)?;
        let domain = std::str::from_utf8(&rest[..z]).ok()?;
        check_domain(domain).ok()?;
        Some(domain.to_string())
    }

    /// The domains these RECORD keys are in, as the APP names them — sorted,
    /// once each. What a Session hands JavaScript when its own writes change
    /// state, so the bindings it holds (keyed by the app-relative name) re-run.
    ///
    /// Returned app-relative, never as stored: a key of another app is not
    /// this app's to name and is dropped. craftworks-sdk#267 prefixed every
    /// stored domain with its app, and this path went on reporting the STORED
    /// name (`<app>.notes`) — which no binding is keyed by — so a tab's own
    /// write never re-ran its bindings and a plain table said "saving" for
    /// good: builder#107, back (measured in the builder's two-tab control).
    pub fn own_domains_of_keys(app: Option<&str>, keys: &[Vec<u8>]) -> Vec<crate::app::AppName> {
        let mut out: Vec<crate::app::AppName> = keys
            .iter()
            .filter_map(|k| Self::domain_of_key(k))
            .filter_map(|stored| crate::app::own(app, &crate::app::StoredName::of_tree(stored)))
            .collect();
        out.sort();
        out.dedup();
        out
    }

    pub fn new(store: S, env: E, device: [u8; 4]) -> Self {
        Db {
            store,
            env,
            ids: IdGen::new(device),
            author: [0; 32],
            ops: std::collections::BTreeMap::new(),
            reruns: std::collections::VecDeque::new(),
            defer_schema: false,
        }
    }

    /// Defer this Db's schema writes (sdk#350): see `defer_schema`. Set once,
    /// at open, by the session that knows it is a published app's own tree.
    pub fn set_defer_schema(&mut self, on: bool) {
        self.defer_schema = on;
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    /// The store, mutably. Reads take `&mut self`, so a test or a tool that
    /// wants to look underneath needs this rather than the shared borrow.
    pub fn store_mut(&mut self) -> &mut S {
        &mut self.store
    }

    /// Write several keys as ONE change to the store.
    ///
    /// A record and the index entries that point at it are one fact. Behind
    /// `Store` may be a tree where every write costs a new root, so writing them
    /// separately would publish roots for states the app never had. Today a
    /// record write is one key; the shape is here so that adding index entries
    /// is adding to this list rather than adding a second write.
    ///
    /// Sorting and last-write-wins happen here, in the SDK: a batch is a
    /// sequence the caller wrote in order and the store takes a set, so it is
    /// the caller that knows which of two writes to a key is the later one.
    ///
    /// **This is where the SDK screens what it sends a store.** Every key and
    /// value the app can reach the store through passes here, including keys the
    /// SDK derives rather than takes (index terms are built from a record's
    /// contents, so an app can produce a long one without ever writing it). A
    /// store is allowed to treat a limit breach as a bug and stop — see
    /// [`Store`] — so the app has to be told before, in a way it can handle.
    /// Remember what the write just made meant, if the store numbers writes.
    fn remember(&mut self, op: Op) {
        if let Some(id) = self.store.last_write_id() {
            self.ops.insert(id, op);
        }
    }

    /// RE-RUN WHAT CONFLICTED (sdk#143/#144, the owner's cell rule at field
    /// level): a write refused because a key it READ had moved is made again
    /// on the new base — its PATCH, not its bytes — with fresh reads. Fields
    /// the other side changed too go to the other side (told once); the rest
    /// land. A chain (the conflicted write and the later ones on its record
    /// that fell with it) is re-run in the order it was made, each on the
    /// record as the previous re-run left it, under ONE budget.
    ///
    /// The host calls this on its tick: it answers what must be LOADED before
    /// it can go on (the copy forgot the conflicted keys) and what it could not
    /// keep. A wait for a load ENDS on the node's answer, never on time (rule
    /// 8); the chain's re-runs are bounded by the ONE budget of tries every
    /// write has, the engine's (sdk#265): a re-run is a try, drawn on from
    /// what the chain had already spent.
    ///
    /// Stated residual (the architect's #5): "unchanged" is per field, current
    /// against the value this write read. Another writer changing a field and
    /// changing it BACK across two of its writes reads as unchanged, and this
    /// patch then wins over their final value.
    pub fn rerun(&mut self) -> RerunStep {
        let mut step = RerunStep::default();
        for chain in self.store.take_conflict_chains() {
            let ops: std::collections::VecDeque<(u64, Op)> =
                chain.write_ids.iter().filter_map(|id| self.ops.remove(id).map(|op| (*id, op))).collect();
            if !ops.is_empty() {
                step.taken.extend(ops.iter().map(|(id, _)| *id));
                self.reruns.push_back(Rerun { ops, tries: chain.tries });
            }
        }
        // What is no longer pending and did not conflict ended: forget it.
        let store = &self.store;
        self.ops.retain(|id, _| store.is_pending_write(*id));
        let mut waiting = std::collections::VecDeque::new();
        while let Some(mut r) = self.reruns.pop_front() {
            match self.rerun_chain(&mut r, &mut step) {
                Ok(()) => {}
                // Waiting for a load: it ENDS on the node's answer (a record
                // that cannot be had fails by name, above) — never on time
                // (rule 8).
                Err(()) => waiting.push_back(r),
            }
        }
        self.reruns = waiting;
        step
    }

    /// Re-run a chain's operations in order. `Err` means "waiting for a load"
    /// (named in `step.load`); the chain keeps what is left.
    fn rerun_chain(&mut self, r: &mut Rerun, step: &mut RerunStep) -> std::result::Result<(), ()> {
        let budget = self.store.max_write_tries();
        if r.tries >= budget {
            for (id, _) in r.ops.drain(..) {
                step.events.push(RerunEvent::Failed {
                    write_id: id,
                    reason: format!("the record was changed elsewhere again after {budget} tries"),
                });
            }
            return Ok(());
        }
        // This re-run is one more try of the same write: its new write goes to
        // the engine with it spent, and counts on from there.
        let tries = r.tries + 1;
        while let Some((id, op)) = r.ops.front().cloned() {
            match op {
                Op::Patch { domain, loc, base, patch, .. } => {
                    let key = record_key(&domain, loc);
                    let schema = match self.schema(&domain) {
                        Ok(Some(s)) => s,
                        Ok(None) => {
                            r.ops.pop_front();
                            step.events.push(RerunEvent::Failed { write_id: id, reason: format!("domain `{domain}` has no schema") });
                            continue;
                        }
                        Err(DbError::NotLoaded { lo, hi }) => {
                            step.load.push((lo, hi));
                            return Err(());
                        }
                        Err(e) => {
                            r.ops.pop_front();
                            step.events.push(RerunEvent::Failed { write_id: id, reason: e.to_string() });
                            continue;
                        }
                    };
                    let cur = match self.get_key(&key) {
                        Ok(Some(b)) => b,
                        Ok(None) => {
                            r.ops.pop_front();
                            step.events.push(RerunEvent::Deleted { write_id: id });
                            continue;
                        }
                        Err(DbError::NotLoaded { lo, hi }) => {
                            step.load.push((lo, hi));
                            return Err(());
                        }
                        Err(e) => {
                            r.ops.pop_front();
                            step.events.push(RerunEvent::Failed { write_id: id, reason: e.to_string() });
                            continue;
                        }
                    };
                    r.ops.pop_front();
                    let cur = match record::decode(&schema, &cur) {
                        Ok(d) => d.fields,
                        Err(e) => {
                            step.events.push(RerunEvent::Failed { write_id: id, reason: e });
                            continue;
                        }
                    };
                    // USER fields only: `updated`/`created`/`author` are not
                    // fields, so another write never counts as changing them.
                    let theirs = |f: &String| cur.get(f) != base.get(f);
                    // Schemas only GROW, so a patch field the schema no longer
                    // has is unreachable: a named refusal, not a silent drop.
                    if let Some(gone) = patch.keys().find(|f| !schema.fields.iter().any(|g| &g.name == *f)) {
                        step.events.push(RerunEvent::Failed { write_id: id, reason: format!("field `{gone}` is no longer in `{domain}`'s schema") });
                        continue;
                    }
                    // ALREADY SATISFIED (sdk#145's second layer): a field that
                    // already holds this patch's value is neither lost nor
                    // written again. That is the case of this write's own
                    // re-send after a lost answer, when it is stored already:
                    // it conflicts (its read no longer holds), and it must not
                    // be told "dropped" for a change that is there.
                    let holds = |f: &String, v: &Value| match cur.get(f) {
                        Some(c) => c == v,
                        None => v.is_null(),
                    };
                    let dropped: Vec<String> = patch.iter().filter(|(f, v)| !holds(f, v) && theirs(f)).map(|(f, _)| f.clone()).collect();
                    let mine: Map<String, Value> =
                        patch.iter().filter(|(f, v)| !holds(f, v) && !theirs(f)).map(|(f, v)| (f.clone(), v.clone())).collect();
                    if !dropped.is_empty() {
                        step.events.push(RerunEvent::Dropped { write_id: id, fields: dropped });
                    }
                    if mine.is_empty() {
                        continue;
                    }
                    self.store.carry_tries(tries);
                    let made = self.update(&domain, loc, &mine);
                    self.store.carry_tries(0);
                    if let Err(e) = made {
                        step.events.push(RerunEvent::Failed { write_id: id, reason: e.to_string() });
                    }
                }
                Op::Append { domain, appended, .. } => {
                    let stored = match self.schema(&domain) {
                        Ok(s) => s,
                        Err(DbError::NotLoaded { lo, hi }) => {
                            step.load.push((lo, hi));
                            return Err(());
                        }
                        Err(e) => {
                            r.ops.pop_front();
                            step.events.push(RerunEvent::Failed { write_id: id, reason: e.to_string() });
                            continue;
                        }
                    };
                    r.ops.pop_front();
                    let Some(mut next) = stored else {
                        step.events.push(RerunEvent::Failed { write_id: id, reason: format!("`{domain}`'s schema is gone") });
                        continue;
                    };
                    let mut add = Vec::new();
                    let mut refused = None;
                    for f in &appended {
                        match next.fields.iter().find(|g| g.name == f.name) {
                            Some(g) if g.kind == f.kind => {}
                            Some(g) => refused = Some(format!("field `{}` was added elsewhere as {:?}, here as {:?}", f.name, g.kind, f.kind)),
                            None => add.push(f.clone()),
                        }
                    }
                    if let Some(reason) = refused {
                        step.events.push(RerunEvent::Failed { write_id: id, reason });
                        continue;
                    }
                    if add.is_empty() {
                        continue;
                    }
                    next.fields.extend(add);
                    self.store.carry_tries(tries);
                    let made = self.define(&domain, &next);
                    self.store.carry_tries(0);
                    if let Err(e) = made {
                        step.events.push(RerunEvent::Failed { write_id: id, reason: e.to_string() });
                    }
                }
            }
        }
        Ok(())
    }

    /// What a key holds NOW, as a read a write declares: its value's leaf hash
    /// (the engine's token) or `Absent`. Not loaded is an error, exactly as
    /// for `get`: "absent" is a claim, and a wrong one would let a write land
    /// over a value it never saw.
    fn read_of(&mut self, key: &[u8]) -> Result<(Vec<u8>, Expect)> {
        Ok((
            key.to_vec(),
            match self.get_key(key)? {
                Some(v) => Expect::Value(crate::read_token::read_token(&v)),
                None => Expect::Absent,
            },
        ))
    }

    /// Every write says what it READ (M2, sdk#148): `reads` holds a read for
    /// every key in `edits` (so `writes ⊆ keys(reads)` by construction), and
    /// the engine checks them where the edits land.
    fn write(&mut self, reads: Vec<(Vec<u8>, Expect)>, edits: Vec<(Vec<u8>, Edit)>) -> Result<()> {
        self.write_as(reads, edits, false)
    }

    /// [`Db::write`], DEFERRED or not (sdk#350): one body, so a deferred
    /// write is refused by exactly the rules an ordinary one is.
    fn write_as(&mut self, reads: Vec<(Vec<u8>, Expect)>, edits: Vec<(Vec<u8>, Edit)>, deferred: bool) -> Result<()> {
        debug_assert!(edits.iter().all(|(k, _)| reads.iter().any(|(r, _)| r == k)), "a write without a read of its key");
        // AN APP'S WRITE IS NEVER FORCED (sdk#235 ruling 1). `Expect::Any` is
        // the transitional form for store-level batches that cannot read
        // first; every `Db` op reads what it writes, so an `Any` here is a bug
        // in this file, refused by name rather than sent to be counted.
        if let Some((k, _)) = reads.iter().find(|(_, e)| *e == Expect::Any) {
            return Err(DbError::Refused(format!(
                "a Db write may not force key {}: an app's write always says what it read",
                crate::hex(k)
            )));
        }
        for (key, edit) in &edits {
            if key.len() > MAX_KEY {
                return Err(DbError::TooLarge(format!(
                    "key is {} bytes and the limit is {MAX_KEY}",
                    key.len()
                )));
            }
            if let Edit::Put(v) = edit {
                if v.len() > MAX_VALUE {
                    return Err(DbError::TooLarge(format!(
                        "record is {} bytes and the limit is {MAX_VALUE}; \
                         store content this large as a file or a blob and keep a \
                         reference to it in the record",
                        v.len()
                    )));
                }
            }
        }
        // THE REFUSAL IS THE ANSWER (sdk#180). A write the store refused was
        // not made; answering `Created` for it told the caller its data was
        // written when it had been dropped.
        let edits = sorted_edits(edits);
        let applied = if deferred { self.store.apply_deferred_commit(&reads, &edits) } else { self.store.apply_commit(&reads, &edits) };
        applied.map_err(DbError::WriteRefused)
    }

    pub fn env_mut(&mut self) -> &mut E {
        &mut self.env
    }

    /// Declare (or extend) a domain's schema. Extending may only append optional fields.
    pub fn define(&mut self, domain: &str, schema: &Schema) -> Result<()> {
        ordinary(domain)?;
        check_domain(domain)?;
        schema.check()?;
        let base = self.schema(domain)?;
        if let Some(old) = &base {
            old.allows(schema)?;
        }
        let bytes = serde_json::to_vec(schema).map_err(|e| e.to_string())?;
        // READ: the schema as it stands (or its absence). Two tabs that define
        // the same schema from one Absent base both succeed: a write already
        // there is not a conflict (the engine's rule, sdk#148).
        let read = self.read_of(&schema_key(domain))?;
        let deferred = self.defer_schema;
        self.write_as(vec![read], vec![(schema_key(domain), Edit::Put(bytes))], deferred)?;
        let appended: Vec<crate::schema::Field> = match &base {
            Some(b) => schema.fields.iter().filter(|f| !b.fields.iter().any(|g| g.name == f.name)).cloned().collect(),
            None => schema.fields.clone(),
        };
        self.remember(Op::Append { domain: domain.to_string(), appended });
        Ok(())
    }

    /// The domain's schema, or `None` if it has none.
    ///
    /// Fallible because the READ is: a store whose data is remote cannot
    /// answer from `&self`, and saying "no schema" when the truth is "could
    /// not look" would make an app define a domain that already exists.
    pub fn schema(&mut self, domain: &str) -> Result<Option<Schema>> {
        match self.get_key(&schema_key(domain))? {
            Some(b) => Ok(serde_json::from_slice(&b).ok()),
            None => Ok(None),
        }
    }

    fn need_schema(&mut self, domain: &str) -> Result<Schema> {
        check_domain(domain)?;
        self.schema(domain)?.ok_or_else(|| {
            DbError::NotDefined(format!("domain `{domain}` has no schema; define it first"))
        })
    }

    pub fn domains(&mut self) -> Result<Vec<String>> {
        let lo = schema_key("");
        let mut hi = lo.clone();
        *hi.last_mut().unwrap() = 1;
        Ok(self
            .scan_keys(&lo, &hi, false, usize::MAX)?
            .into_iter()
            .filter_map(|(k, _)| String::from_utf8(k[lo.len()..].to_vec()).ok())
            .collect())
    }

    pub fn put(&mut self, domain: &str, fields: &Map<String, Value>) -> Result<Record> {
        ordinary(domain)?;
        let schema = self.need_schema(domain)?;
        let rkey = self.ids.next(&mut self.env);
        // The rkey stays SDK-minted. The caller chooses only the PARENT, which
        // is what keeps an app from encoding the key layout -- it names an
        // intent, and the SDK still owns what a key is.
        let loc = Loc { parent: parent_of(&schema, fields)?, rkey };
        let now = id::created_ms(&rkey);
        let bytes = record::encode(&schema, fields, now, now, &self.author)?;
        let key = record_key(domain, loc);
        // READ: a fresh key is ABSENT (a create, never an overwrite), and the
        // schema it was encoded under is the one that stands (C1: a record is
        // positional, so a schema that moved under it would scramble it).
        let reads = vec![(key.clone(), Expect::Absent), self.read_of(&schema_key(domain))?];
        self.write(reads, vec![(key.clone(), Edit::Put(bytes.clone()))])?;
        self.read(&schema, &loc, &key, &bytes)
    }

    /// Create a record at a key THE CALLER chose, or find the one already there.
    ///
    /// `put` mints the rkey, so writing "the same record" twice writes two.
    /// A copy whose key is a FUNCTION of its source — [`id::slot_from`] —
    /// lands on the same key however many times, from however many tabs, it
    /// is made, and nothing has to remember that it was (craftworks-sdk#149).
    ///
    /// **A create, never an overwrite.** If a record is already stored at the
    /// slot, it is returned as [`CreateAt::Exists`] and the tree is not
    /// touched.
    ///
    /// **Not loaded is not absent.** The slot is READ before anything is
    /// decided, and a read that could not be answered is an error, exactly as
    /// it is for `get`. A fresh session holds nothing; answering "absent"
    /// there would write Preview's copy over a published record — including
    /// one edited since it was published. The recovery is the usual one: load
    /// and ask again, which repeats the attempt and not the effect, because
    /// nothing is written before the read answers.
    ///
    /// The slot's first 8 bytes are the record's `created`, as a minted id's
    /// are — so a copy keeps its source's creation time and sorts among the
    /// minted records by it. That makes `created` the caller's to choose, so a
    /// time more than [`SLOT_SKEW_MS`] ahead of this clock is refused. The
    /// record's `updated` is NOW: it was written now, whatever it copies.
    ///
    /// The parent, in a domain that has one, comes from the fields as it does
    /// for `put`; the slot is the rkey under it.
    ///
    /// Stated residual, until the write can carry `expected: Absent`
    /// (craftworks-sdk#148 step 1): two sessions that both read the slot
    /// absent both write it, and the second write wins. There is still at
    /// most ONE record at the slot; which content it holds is the later's.
    pub fn create_at(
        &mut self,
        domain: &str,
        slot: RKey,
        fields: &Map<String, Value>,
    ) -> Result<CreateAt> {
        ordinary(domain)?;
        let schema = self.need_schema(domain)?;
        let loc = Loc { parent: parent_of(&schema, fields)?, rkey: slot };
        let key = record_key(domain, loc);
        if let Some(held) = self.get_key(&key)? {
            return self.read(&schema, &loc, &key, &held).map(CreateAt::Exists);
        }
        let now = self.env.now_ms();
        let created = id::created_ms(&slot);
        if created > now.saturating_add(SLOT_SKEW_MS) {
            return Err(DbError::Refused(format!(
                "this slot is dated {}ms ahead of this device's clock; a record's \
                 creation time may not be more than {SLOT_SKEW_MS}ms in the future",
                created - now
            )));
        }
        let bytes = record::encode(&schema, fields, created, now.max(created), &self.author)?;
        // READ: the slot ABSENT, so two sessions that both found it absent
        // make ONE record — the other is told Conflict (closing the residual
        // above, sdk#148) — and the schema it was encoded under.
        let reads = vec![(key.clone(), Expect::Absent), self.read_of(&schema_key(domain))?];
        self.write(reads, vec![(key.clone(), Edit::Put(bytes.clone()))])?;
        self.read(&schema, &loc, &key, &bytes).map(CreateAt::Created)
    }

    /// Merge `patch` over the record; a `null` value removes that field.
    pub fn update(
        &mut self,
        domain: &str,
        at: impl Into<Loc>,
        patch: &Map<String, Value>,
    ) -> Result<Record> {
        ordinary(domain)?;
        let schema = self.need_schema(domain)?;
        let loc = self.locate(&schema, domain, at)?;
        let key = record_key(domain, loc);
        let old = self.get_key(&key)?.ok_or("no such record")?;
        let mut d = record::decode(&schema, &old)?;
        // The parent decides the KEY, so a patch that moved it would leave the
        // record filed under its old parent while claiming the new one. A
        // re-parent is a delete and a write, and it is not this call.
        if let Some(pf) = &schema.parent {
            if let Some(v) = patch.get(pf) {
                if v != d.fields.get(pf).unwrap_or(&Value::Null) {
                    return Err(DbError::Refused(format!(
                        "`{pf}` is this domain's parent and decides the record's \
                         key; changing it here would leave the record filed under \
                         the old one"
                    )));
                }
            }
        }
        for (k, v) in patch {
            if v.is_null() {
                d.fields.remove(k);
            } else {
                d.fields.insert(k.clone(), v.clone());
            }
        }
        // `updated` never runs backwards, even if the clock does.
        let updated = self.env.now_ms().max(d.updated);
        let bytes = record::encode(&schema, &d.fields, d.created, updated, &d.author)?;
        // READ: the record exactly as it was patched — a patch over bytes
        // that moved since would replace a change this session never saw —
        // and the schema it was decoded and re-encoded under.
        let reads = vec![(key.clone(), Expect::Value(crate::read_token::read_token(&old))), self.read_of(&schema_key(domain))?];
        let base = record::decode(&schema, &old)?.fields;
        self.write(reads, vec![(key.clone(), Edit::Put(bytes.clone()))])?;
        self.remember(Op::Patch { domain: domain.to_string(), loc, base, patch: patch.clone() });
        self.read(&schema, &loc, &key, &bytes)
    }

    /// A record, or `None`.
    ///
    /// **An id this domain cannot address answers `None` rather than
    /// refusing.** The ids reaching a read come from OUTSIDE the program — a
    /// value in browser storage written by an older build, a pasted link, a
    /// record deleted on another device — and for every one of those the
    /// honest answer is *absent*, which this already has a representation for.
    /// Making "malformed" a different CONTROL-FLOW outcome from "missing"
    /// forces every caller that touches an outside id into a `try`, and the
    /// one that forgets is the one that crashes (craftworks-sdk#118).
    ///
    /// This matters more since #122, not less: a component id stored before
    /// that change is 32 hex where the domain now wants 64, so the stale-id
    /// case is no longer hypothetical.
    ///
    /// A WRITE still refuses, and that asymmetry is deliberate: answering
    /// "nothing to do" to a delete or an update aimed at an id nobody can
    /// resolve would hide the caller's mistake instead of reporting it.
    pub fn get(&mut self, domain: &str, at: impl Into<Loc>) -> Result<Option<Record>> {
        let schema = self.need_schema(domain)?;
        let at: Loc = at.into();
        let Ok(loc) = self.locate(&schema, domain, at) else {
            // Well-formed (it parsed) and the wrong WIDTH for this domain: the
            // one case the answer "not found" would otherwise leave silent.
            self.store.wrong_width(IdWidth::of(at.parent.is_some()), IdWidth::of(schema.parent.is_some()));
            return Ok(None);
        };
        let key = record_key(domain, loc);
        match self.get_key(&key)? {
            Some(b) => self.read(&schema, &loc, &key, &b).map(Some),
            None => Ok(None),
        }
    }

    pub fn delete(&mut self, domain: &str, at: impl Into<Loc>) -> Result<bool> {
        ordinary(domain)?;
        check_domain(domain)?;
        let schema = self.need_schema(domain)?;
        let loc = self.locate(&schema, domain, at)?;
        let key = record_key(domain, loc);
        // READ: what is deleted is what was read — its value, or its absence
        // (and a record CREATED since an absent read is a conflict, not a
        // silent "nothing to do").
        let read = self.read_of(&key)?;
        let existed = !matches!(read.1, Expect::Absent);
        self.write(vec![read], vec![(key, Edit::Delete)])?;
        Ok(existed)
    }

    pub fn scan(&mut self, domain: &str, opts: Scan) -> Result<Vec<Record>> {
        let schema = self.need_schema(domain)?;
        let p = record_prefix(domain);
        let (mut lo, mut hi) = (p.clone(), upper(&p));
        if let Some(a) = opts.after {
            let k = record_key(domain, Loc::bare(a));
            if opts.reverse {
                hi = k;
            } else {
                lo = [k, vec![0]].concat(); // the next possible key
            }
        }
        let limit = if opts.limit == 0 {
            usize::MAX
        } else {
            opts.limit
        };
        self.scan_keys(&lo, &hi, opts.reverse, limit)?
            .iter()
            .map(|(k, v)| {
                let loc = loc_of_key(&schema, &p, k)?;
                self.read(&schema, &loc, k, v)
            })
            .collect()
    }

    /// ONE PAGE OF THE WHOLE TREE, every key of every app (sdk#479, REPAIR): up to `limit` rows after `after`, read
    /// through the normal read -- which races each block's group and puts back what the node lost. Returns how many
    /// rows it read and where the next page starts (`None`: the tree is read). Rows are counted, never decoded: a
    /// repair pass needs every block walked, not the records.
    pub fn scan_all(&mut self, after: Option<Vec<u8>>, limit: usize) -> Result<(usize, Option<Vec<u8>>)> {
        let lo = after.map(|k| [k, vec![0]].concat()).unwrap_or_default();
        // Past every key: a key is at most MAX_KEY bytes, so MAX_KEY + 1 of 0xff sorts after all of them.
        let hi = vec![0xff; MAX_KEY + 1];
        let limit = limit.max(1);
        let rows = self.scan_keys(&lo, &hi, false, limit)?;
        let next = (rows.len() == limit).then(|| rows.last().map(|(k, _)| k.clone())).flatten();
        Ok((rows.len(), next))
    }

    /// THE CHILDREN OF ONE PARENT, as a bounded read.
    ///
    /// The app names the parent; it never builds a range. What the range IS
    /// stays the SDK's business, which is the same rule `Session::preload`
    /// keeps — a caller that built one would be encoding the key layout.
    ///
    /// The cost is the size of THIS parent's band. It does not grow with the
    /// number of records under other parents, which is the whole difference
    /// from the scan-and-filter this replaces (craftworks-sdk#122).
    pub fn children(&mut self, domain: &str, parent: &RKey, opts: Scan) -> Result<Vec<Record>> {
        let schema = self.need_schema(domain)?;
        let pf = schema.parent.clone().ok_or_else(|| {
            DbError::Refused(format!(
                "domain `{domain}` does not declare a parent, so it has no \
                 children to read: records in it are keyed by their own id alone"
            ))
        })?;
        let _ = pf;
        let p = record_prefix(domain);
        let (mut lo, mut hi) = parent_span(domain, parent);
        if let Some(a) = opts.after {
            let k = record_key(domain, Loc::under(*parent, a));
            if opts.reverse { hi = k; } else { lo = [k, vec![0]].concat(); }
        }
        let limit = if opts.limit == 0 { usize::MAX } else { opts.limit };
        self.scan_keys(&lo, &hi, opts.reverse, limit)?
            .iter()
            .map(|(k, v)| {
                let loc = loc_of_key(&schema, &p, k)?;
                self.read(&schema, &loc, k, v)
            })
            .collect()
    }

    /// Resolve what a caller handed us into a full location, or refuse.
    ///
    /// A bare rkey in a parent-keyed domain cannot address anything: the key
    /// needs the parent. A WRITE refuses here. A read does not reach this
    /// refusal — `get` turns it into `None`, because the bare ids that reach a
    /// read come from outside (a value stored before #122, a pasted link) and
    /// for those "absent" is the honest answer (craftworks-sdk#118).
    fn locate(&self, schema: &Schema, domain: &str, at: impl Into<Loc>) -> Result<Loc> {
        let loc = at.into();
        match (&schema.parent, &loc.parent) {
            (Some(pf), None) => Err(DbError::Refused(format!(
                "domain `{domain}` keys its records under `{pf}`, so it wants a \
                 64-hex id (parent then record); this one is 32 hex"
            ))),
            (None, Some(_)) => Err(DbError::Refused(format!(
                "domain `{domain}` does not key its records under a parent, so it \
                 wants a 32-hex id; this one is 64 hex"
            ))),
            _ => Ok(loc),
        }
    }

    /// How many records the domain holds — from the store's `count`, which a
    /// store with the tree answers in O(height) node reads (craftworks-sdk#123).
    /// It used to enumerate every key and value in the domain to return a
    /// number every node already carries.
    pub fn count(&mut self, domain: &str) -> Result<usize> {
        check_domain(domain)?;
        let p = record_prefix(domain);
        let hi = upper(&p);
        let n = self
            .store
            .count(&p, &hi)
            .map_err(|e| DbError::from_store(e, &p, &hi))?;
        Ok(n as usize)
    }

    /// A store read, with the store's own refusal turned into `Db`'s error.
    ///
    /// Every `&self` read in `Db` goes through these two, so there is one
    /// place where a store that cannot answer synchronously becomes something
    /// an app can catch rather than something that kills the instance.
    fn get_key(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        // The span for a single key is that key alone. `succ` is the
        // smallest key strictly after it, so `[key, succ)` contains exactly
        // one key and the loaded-interval bookkeeping stays exact.
        self.store
            .get(key)
            .map_err(|e| DbError::from_store(e, key, &succ(key)))
    }

    fn scan_keys(
        &mut self,
        lo: &[u8],
        hi: &[u8],
        reverse: bool,
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.store
            .scan(lo, hi, reverse, limit)
            .map_err(|e| DbError::from_store(e, lo, hi))
    }

    /// Build a record, asking the store what this row's own write is doing.
    ///
    /// The KEY is passed rather than recomputed from the domain, so a caller
    /// cannot ask about a different row than the one it decoded.
    fn read(&self, schema: &Schema, loc: &Loc, key: &[u8], bytes: &[u8]) -> Result<Record> {
        let d = record::decode(schema, bytes)?;
        Ok(Record {
            id: id::loc_to_hex(loc),
            created: d.created,
            updated: d.updated,
            fields: d.fields,
            state: self.store.row_state(key),
        })
    }
}

/// A reserved domain's name as this db holds it: behind the app's id (the tree's form), or -- with none, the in-tab
/// db -- as an app writes it. The one place a [`SystemDomain`] becomes a stored name.
fn system_name(app: Option<&str>, d: SystemDomain) -> String {
    match app {
        Some(a) => format!("{a}.{}", d.code()),
        None => d.code().to_string(),
    }
}

/// A definition record's schema: its key and its body (the record's JSON, as text).
fn definition_schema() -> Schema {
    let field = |name: &str, kind, required| crate::schema::Field { name: name.into(), kind, required };
    use crate::schema::Kind::{Bytes, Text};
    // `bytes` (app-as-data P5): a FILE's contents, appended -- a tree whose draft P2 wrote goes on decoding, and its
    // schema is extended by the next door that writes (`system_schema`).
    Schema { type_name: "Definition".into(), fields: vec![field("key", Text, true), field("body", Text, true), field("bytes", Bytes, false)], parent: None }
}

/// One record of an app's definition: its key, its body (JSON), and -- a file's (`f/<path>`) -- its bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct DefRecord {
    pub key: DefKey,
    pub body: Value,
    pub bytes: Option<Vec<u8>>,
}

/// The builder's per-domain "live" markers' schema: the one `handoff.js` defines (`{ type: "Published", fields: [] }`).
fn published_schema() -> Schema {
    Schema { type_name: "Published".into(), fields: vec![], parent: None }
}

/// A definition record's slot: a function of its key alone, so the draft's and the app's records of one key share it
/// and a publish copies record for record.
fn definition_slot(key: &DefKey) -> RKey {
    id::slot_from(0, "definition", &key.to_string())
}

/// A published marker's slot: `handoff.js`'s `markerSlot` (`slotFrom(0, "domain", d)`), so the markers the builder
/// wrote before this door are the ones it reads.
fn published_slot(domain: &str) -> RKey {
    id::slot_from(0, "domain", domain)
}

/// THE DEFINITION DOORS (ARCHITECTURE §19; app-as-data P2): the ONLY writes of an app's reserved domains. Each takes a
/// [`SystemDomain`] (never a name a caller spelled) and reaches the one commit path, [`Db::write`], directly -- the
/// ordinary writes refuse the reserved prefix (`ordinary`). `app` is the tree's app id, `None` for the in-tab db.
impl<S: Store + Reads, E: Env> Db<S, E> {
    /// `domain`'s schema as THIS write needs it: read (so it must stand), and PUT in the same write when absent -- or
    /// when the one standing is an earlier version of it (fields only appended: `Schema::allows`). Any other schema
    /// under a reserved name is refused: nothing but these doors writes one.
    fn system_schema(&mut self, domain: &str, schema: &Schema, reads: &mut Vec<(Vec<u8>, Expect)>, edits: &mut Vec<(Vec<u8>, Edit)>) -> Result<()> {
        reads.push(self.read_of(&schema_key(domain))?);
        match self.schema(domain)? {
            Some(s) if &s == schema => Ok(()),
            Some(s) if s.allows(schema).is_err() => Err(DbError::Refused(format!("`{domain}` holds another schema than its definition door writes"))),
            Some(_) | None => {
                edits.push((schema_key(domain), Edit::Put(serde_json::to_vec(schema).map_err(|e| e.to_string())?)));
                Ok(())
            }
        }
    }

    /// Every record of a domain, by its key AFTER the domain's record prefix (the part a draft and an app share).
    fn records_of(&mut self, domain: &str) -> Result<std::collections::BTreeMap<Vec<u8>, Vec<u8>>> {
        let p = record_prefix(domain);
        Ok(self.scan_keys(&p, &upper(&p), false, usize::MAX)?.into_iter().map(|(k, v)| (k[p.len()..].to_vec(), v)).collect())
    }

    /// EDIT THE DRAFT: `key`'s record in `craftworks.draft` holds `body`, created or replaced. The same body again is
    /// nothing to write. A FILE is written by [`Db::draft_file`], never here (one form per kind).
    pub fn draft_put(&mut self, app: Option<&str>, key: &DefKey, body: &Value) -> Result<()> {
        if let DefKey::File(path) = key {
            return Err(DbError::Refused(format!("`f/{path}` is a file: its bytes are written by the file door")));
        }
        self.draft_record(app, key, body.to_string(), None)
    }

    /// A FILE OF THE APP'S CODE (app-as-data P5): `f/<path>`'s record in the draft holds `bytes` (and `meta`, JSON:
    /// may be `{}`), created or replaced -- the same door, the same one commit path, as every definition record. A
    /// record is bounded (`MAX_VALUE`): a file whose record would be larger is refused naming its path and both sizes.
    pub fn draft_file(&mut self, app: Option<&str>, path: &str, bytes: &[u8], meta: &Value) -> Result<()> {
        let key = DefKey::parse(&format!("f/{path}"))?;
        self.draft_record(app, &key, meta.to_string(), Some(bytes))
    }

    /// The draft's `key` record: `body`, and a file's `bytes`. Unchanged is nothing to write.
    fn draft_record(&mut self, app: Option<&str>, key: &DefKey, body: String, file: Option<&[u8]>) -> Result<()> {
        let domain = system_name(app, SystemDomain::Draft);
        let schema = definition_schema();
        let (mut reads, mut edits) = (Vec::new(), Vec::new());
        self.system_schema(&domain, &schema, &mut reads, &mut edits)?;
        let k = record_key(&domain, Loc::bare(definition_slot(key)));
        let old = self.get_key(&k)?.map(|o| record::decode(&schema, &o)).transpose()?;
        let file_hex = file.map(core_types::hex::encode);
        if old.as_ref().is_some_and(|o| {
            o.fields.get("body").and_then(Value::as_str) == Some(body.as_str()) && o.fields.get("bytes").and_then(Value::as_str) == file_hex.as_deref()
        }) {
            return Ok(());
        }
        let now = self.env.now_ms();
        let created = old.as_ref().map_or(now, |o| o.created);
        let mut fields = Map::new();
        fields.insert("key".into(), Value::from(key.to_string()));
        fields.insert("body".into(), Value::from(body));
        if let Some(h) = file_hex {
            fields.insert("bytes".into(), Value::from(h));
        }
        let bytes = record::encode(&schema, &fields, created, now.max(created), &self.author)?;
        if bytes.len() > MAX_VALUE {
            return Err(DbError::TooLarge(format!(
                "`{key}` is {} bytes as a record ({} of file), over the {MAX_VALUE} a record may be",
                bytes.len(),
                file.map_or(0, <[u8]>::len)
            )));
        }
        reads.push(self.read_of(&k)?);
        edits.push((k, Edit::Put(bytes)));
        self.write(reads, edits)
    }

    /// Remove `key`'s record from the draft; `false` when there was none (nothing written).
    pub fn draft_delete(&mut self, app: Option<&str>, key: &DefKey) -> Result<bool> {
        let domain = system_name(app, SystemDomain::Draft);
        let k = record_key(&domain, Loc::bare(definition_slot(key)));
        let read = self.read_of(&k)?;
        if read.1 == Expect::Absent {
            return Ok(false);
        }
        self.write(vec![read], vec![(k, Edit::Delete)])?;
        Ok(true)
    }

    /// PUBLISH: make `craftworks.app` EQUAL `craftworks.draft` in ONE write -- every changed record put, every app
    /// record with no draft counterpart deleted -- so the head moves once and no reader sees a mixed definition. The
    /// write reads every draft record it copies, so a publish racing a draft edit is refused, never half-applied.
    /// Answers how many records changed; 0 (nothing written) when they are equal.
    pub fn publish_definition(&mut self, app: Option<&str>) -> Result<usize> {
        let (draft_name, app_name) = (system_name(app, SystemDomain::Draft), system_name(app, SystemDomain::App));
        let draft = self.records_of(&draft_name)?;
        let published = self.records_of(&app_name)?;
        self.entry_loads(&draft)?;
        let (mut reads, mut edits) = (Vec::new(), Vec::new());
        for (tail, bytes) in &draft {
            if published.get(tail) != Some(bytes) {
                let k = [record_prefix(&app_name), tail.clone()].concat();
                reads.push(self.read_of(&k)?);
                edits.push((k, Edit::Put(bytes.clone())));
            }
        }
        for tail in published.keys().filter(|t| !draft.contains_key(*t)) {
            let k = [record_prefix(&app_name), tail.clone()].concat();
            reads.push(self.read_of(&k)?);
            edits.push((k, Edit::Delete));
        }
        let changed = edits.len();
        if changed == 0 {
            return Ok(0);
        }
        for tail in draft.keys() {
            reads.push(self.read_of(&[record_prefix(&draft_name), tail.clone()].concat())?);
        }
        self.system_schema(&app_name, &definition_schema(), &mut reads, &mut edits)?;
        self.write(reads, edits)?;
        Ok(changed)
    }

    /// THE ENTRY LOADS (app-as-data P5, the SDK the one owner): a draft whose `meta` names an `entry` must hold that
    /// file -- `"entry": "f/<path>"` naming an `f/` record of the draft -- or its publish is refused by name. A draft
    /// with no `meta.entry` is a definition-only app (the loader mounts the runtime), and stays valid.
    fn entry_loads(&self, draft: &std::collections::BTreeMap<Vec<u8>, Vec<u8>>) -> Result<()> {
        let schema = definition_schema();
        let mut keys = std::collections::BTreeSet::new();
        let mut entry = None;
        for b in draft.values() {
            let d = record::decode(&schema, b)?;
            let key = d.fields.get("key").and_then(Value::as_str).unwrap_or("").to_string();
            if key == DefKey::Meta.to_string() {
                let body: Value = serde_json::from_str(d.fields.get("body").and_then(Value::as_str).unwrap_or("null")).unwrap_or(Value::Null);
                entry = body.get("entry").cloned();
            }
            keys.insert(key);
        }
        let Some(entry) = entry else { return Ok(()) };
        let named = entry.as_str().map(DefKey::parse);
        match named {
            Some(Ok(k @ DefKey::File(_))) if keys.contains(&k.to_string()) => Ok(()),
            _ => Err(DbError::Refused(format!("meta.entry is {entry}: it must name an `f/<path>` file of the draft, and the draft holds none such -- a published app's entry must load"))),
        }
    }

    /// The definition `which` holds (`Draft` or `App`): each record's key and body, in slot order. `app` names whose:
    /// a READ, so it may be any app of this tree (the builder's project list reads each project's `meta`, §19 P3);
    /// the writing doors take only the caller's own. An app id is checked here, the one place both surfaces ask.
    pub fn definition(&mut self, app: Option<&str>, which: SystemDomain) -> Result<Vec<DefRecord>> {
        if let Some(a) = app {
            crate::app::check(a)?;
        }
        if which == SystemDomain::Published {
            return Err(DbError::Refused(format!("`{}` holds markers, not a definition", which.code())));
        }
        let schema = definition_schema();
        self.records_of(&system_name(app, which))?
            .values()
            .map(|b| {
                let d = record::decode(&schema, b)?;
                let key = DefKey::parse(d.fields.get("key").and_then(Value::as_str).unwrap_or(""))?;
                let body = serde_json::from_str(d.fields.get("body").and_then(Value::as_str).unwrap_or("null")).map_err(|e| DbError::Refused(e.to_string()))?;
                let bytes = d.fields.get("bytes").and_then(Value::as_str).map(|h| core_types::hex::decode(h).ok_or_else(|| DbError::Refused(format!("`{key}`'s bytes are not hex")))).transpose()?;
                Ok(DefRecord { key, body, bytes })
            })
            .collect()
    }

    /// THE APPS OF THIS TREE THAT HOLD A DRAFT, by id, sorted: a READ (§19 P3b: the builder's project list is these,
    /// each named by its own `meta`). From the tree's domain names, so it never reads a record; the unnamed (in-tab)
    /// app's own draft is not an app of the tree and is not listed.
    pub fn definition_apps(&mut self) -> Result<Vec<String>> {
        let tail = format!(".{}", SystemDomain::Draft.code());
        let mut apps: Vec<String> = self
            .domains()?
            .into_iter()
            .filter_map(|d| d.strip_suffix(tail.as_str()).map(str::to_string))
            .filter(|a| core_types::name::app_ok(a))
            .collect();
        apps.sort();
        apps.dedup();
        Ok(apps)
    }

    /// MARK `domain` LIVE (the builder's marker, until app-as-data P5 derives liveness from `craftworks.app`): its
    /// record in `craftworks.published`, created at its slot, or the one already there -- `create_at`'s answer.
    pub fn mark_published(&mut self, app: Option<&str>, domain: &str) -> Result<CreateAt> {
        crate::app::check_name(domain)?;
        let name = system_name(app, SystemDomain::Published);
        let schema = published_schema();
        let loc = Loc::bare(published_slot(domain));
        let k = record_key(&name, loc);
        if let Some(held) = self.get_key(&k)? {
            return self.read(&schema, &loc, &k, &held).map(CreateAt::Exists);
        }
        let (mut reads, mut edits) = (Vec::new(), Vec::new());
        self.system_schema(&name, &schema, &mut reads, &mut edits)?;
        let now = self.env.now_ms();
        let bytes = record::encode(&schema, &Map::new(), 0, now, &self.author)?;
        reads.push((k.clone(), Expect::Absent));
        edits.push((k.clone(), Edit::Put(bytes.clone())));
        self.write(reads, edits)?;
        self.read(&schema, &loc, &k, &bytes).map(CreateAt::Created)
    }

    /// `domain`'s marker ([`Db::mark_published`]) as a RECORD with its write state -- `None` when unmarked. What a
    /// caller that must know the marker is SAVED reads (builder#88: a publish is not done over a marker still in
    /// flight): [`Db::is_published`] says only that one EXISTS, pending included. Reads only.
    pub fn published_state(&mut self, app: Option<&str>, domain: &str) -> Result<Option<Record>> {
        crate::app::check_name(domain)?;
        let name = system_name(app, SystemDomain::Published);
        let loc = Loc::bare(published_slot(domain));
        let k = record_key(&name, loc);
        match self.get_key(&k)? {
            Some(held) => self.read(&published_schema(), &loc, &k, &held).map(Some),
            None => Ok(None),
        }
    }

    /// Is `domain` marked live ([`Db::mark_published`])? Its marker EXISTS -- pending included (for "saved", read
    /// [`Db::published_state`]). Reads only.
    pub fn is_published(&mut self, app: Option<&str>, domain: &str) -> Result<bool> {
        Ok(self.published_state(app, domain)?.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree_store::TreeStore;

    /// A DRAFT P2 WROTE is extended, never refused (app-as-data P5): its schema stored before `bytes` existed (key,
    /// body) is an earlier version of the door's, so the next door write appends the field in the same write -- and
    /// the old record decodes under it. A schema that is NOT an earlier version is still refused.
    #[test]
    fn a_draft_under_the_p2_schema_is_extended_by_the_next_door_write() {
        let mut d = Db::new(TreeStore::new(), crate::id::SystemEnv, *b"dev1");
        let domain = system_name(Some("notes"), SystemDomain::Draft);
        let mut p2 = definition_schema();
        p2.fields.truncate(2);
        d.store.apply_batch(&[(schema_key(&domain), Edit::Put(serde_json::to_vec(&p2).unwrap()))]).unwrap();
        d.draft_put(Some("notes"), &DefKey::Meta, &serde_json::json!({ "name": "old" })).expect("a door write over the P2 schema");
        assert_eq!(d.schema(&domain).unwrap(), Some(definition_schema()), "the schema was not extended");
        d.draft_file(Some("notes"), "app.js", b"x", &serde_json::json!({})).expect("a file over the extended schema");
        assert_eq!(d.definition(Some("notes"), SystemDomain::Draft).unwrap().len(), 2);
        let other = Schema { type_name: "Other".into(), fields: vec![], parent: None };
        d.store.apply_batch(&[(schema_key(&domain), Edit::Put(serde_json::to_vec(&other).unwrap()))]).unwrap();
        assert!(d.draft_put(Some("notes"), &DefKey::Meta, &serde_json::json!({})).is_err(), "a foreign schema was overwritten");
    }

    /// **An app's write is never forced** (sdk#235 ruling 1): a `Db` write
    /// carrying `Expect::Any` is REFUSED by name — a real refusal, not a
    /// debug assertion — and the store is never reached.
    #[test]
    fn a_db_write_carrying_any_is_refused_and_the_store_is_untouched() {
        let mut d = Db::new(TreeStore::new(), crate::id::SystemEnv, *b"dev1");
        let key = b"\x01notes\x00k".to_vec();
        let e = d
            .write(vec![(key.clone(), Expect::Any)], vec![(key.clone(), Edit::Put(b"v".to_vec()))])
            .expect_err("a Db write carrying Any was accepted");
        assert!(matches!(e, DbError::Refused(_)), "not a refusal: {e:?}");
        assert!(e.to_string().contains("may not force key"), "{e}");
        assert_eq!(d.get_key(&key).unwrap(), None, "the refused write reached the store");
        // THE CONTROL: the same write with a real read is made.
        d.write(vec![(key.clone(), Expect::Absent)], vec![(key.clone(), Edit::Put(b"v".to_vec()))]).expect("a write with its read is made");
        assert_eq!(d.get_key(&key).unwrap(), Some(b"v".to_vec()));
    }

    /// The key screen guards DERIVED keys. Nothing an app can write today
    /// produces one — a record key is a 1-byte tag, a domain of at most 32, a
    /// separator and a 16-byte id — but index terms will be built from a
    /// record's contents (the first 64 B of a term plus a hash of the rest), so
    /// an app will be able to produce a long one without ever writing it. The
    /// screen is tested where it lives, at the funnel every write goes through.
    #[test]
    fn a_key_past_the_trees_limit_is_refused_before_the_store_is_touched() {
        struct Panics;
        impl Store for Panics {
            fn apply_batch(&mut self, _: &[(Vec<u8>, Edit)]) -> std::result::Result<(), crate::store::Refused> {
                panic!("the store must not be reached")
            }
        }
        impl crate::store::Reads for Panics {
            fn get(&mut self, _: &[u8]) -> crate::store::Read<Option<Vec<u8>>> {
                Ok(None)
            }
            fn scan(
                &mut self,
                _: &[u8],
                _: &[u8],
                _: bool,
                _: usize,
            ) -> crate::store::Read<Vec<(Vec<u8>, Vec<u8>)>> {
                Ok(Vec::new())
            }
            fn root(&mut self) -> crate::store::Read<[u8; 32]> {
                Ok([0u8; 32])
            }
            fn changes_since(
                &mut self,
                _: [u8; 32],
                _: &[u8],
                _: &[u8],
                _: u32,
            ) -> crate::store::Read<crate::store::Delta> {
                Ok(crate::store::Delta::FullReloadRequired {
                    new_root: [0u8; 32],
                })
            }
        }

        let mut d = Db::new(Panics, crate::id::SystemEnv, *b"dev1");
        let long = vec![b'k'; MAX_KEY + 1];
        let e = d.write(vec![(long.clone(), protocol::Expect::Absent)], vec![(long, Edit::Put(b"v".to_vec()))]).unwrap_err();
        assert_eq!(
            e.code(),
            "TOO_LARGE",
            "a key over the limit must carry the code an app branches on, not only a message"
        );
        assert!(e.to_string().contains(&MAX_KEY.to_string()), "{e}");
        // The longest legal key is accepted — so the refusal is the length and
        // not the screen refusing everything.
        let mut d = Db::new(TreeStore::new(), crate::id::SystemEnv, *b"dev1");
        d.write(vec![(vec![b'k'; MAX_KEY], protocol::Expect::Absent)], vec![(vec![b'k'; MAX_KEY], Edit::Put(b"v".to_vec()))])
            .unwrap();
    }
}
