//! # The page's protocol SERVER (sdk wiring, main's ruling B)
//!
//! The web Session's `Db` → `PageStore` → `Writes` → `sdk::Client` speak the
//! page ↔ engine protocol. Until now the engine delegate's Shell answered it; here
//! the PAGE does, over [`crate::Page`] (#215): the real engine, whose node
//! work is the client-API executor and whose head the SIGNER signs. Nothing
//! above it changes, so every client-side model still judges it.
//!
//! **Ported VERBATIM from `engine-delegate/src/shell.rs` at b0ab729** (line
//! numbers below are e17711d's; b0ab729 adds only `reads: Vec::new()` and the
//! two `State::Conflict` arms, ported as they are) (main's
//! condition 1) — the protocol half of the Shell, not its node half (the
//! scheduler, read-backs and own-key head signing, which `Page` replaced):
//! * `serve` — `engine-delegate/src/serve.rs` (v5 and above answered
//!   `Unsupported`, as there);
//! * `on_protocol` — shell.rs 796–979; the one change: the event goes to
//!   `Page::event` and its effects are read back through `Page::take_client`;
//! * `reply_from` — shell.rs 1139–1279; the one change: `At` from
//!   `Page::published`;
//! * `attribute` / `step` — shell.rs 1287–1308 / 775–788; `attribute` takes
//!   the client's bytes rather than an `Inbound`;
//! * the identity / already-installed / unserved answers and the per-effect
//!   trace steps — shell.rs `handle`, 606–646 and 671–711;
//! * `as_client` / `version_of` / `session_of` / `as_*`, `reply_bytes`,
//!   `rows_in`, `state_tag` and the page constants — shell.rs 163–296.
//!
//! The differential (`tests/server_differential.rs`) holds it to the Shell:
//! the same requests, the same replies, modulo the head's signer.

use crate::{Answer, Ms, Op, Page};
use engine::{Effect, Event, KeySource, State};
use protocol::{Incoming, Reply, Request};
use std::collections::BTreeMap;

/// shell.rs 163 / 167 / 170.
const MAX_PAGE_ENTRIES: usize = protocol::MAX_PAGE_ENTRIES as usize;
const MAX_PAGE_BYTES: usize = 512 * 1024;
const MAX_STEPS: usize = 64;

/// What leaves one call: the replies for the client.
#[derive(Debug, Default)]
struct Outbound {
    replies: Vec<Vec<u8>>,
}

/// What the page knows about its signer and head, as the Session found it
/// (the Shell's `StoreFacts`, whose secret store is now the signer's).
#[derive(Debug, Clone, Copy, Default)]
pub struct SignerFacts {
    /// The signer holds a key and the Register it signs for (`Provisioned`).
    pub head_writable: bool,
    /// The head Register's instance id, or zero when there is none.
    pub head_id: [u8; 32],
}

pub struct Server {
    /// `Busy` verdicts this server has told a client (R-b, COMMIT-LIFE K1:
    /// on the page path a write is QUEUED, never `Busy`). A count for the
    /// model's check; it decides nothing. A `Cell` because verdicts are
    /// told from `reply_from(&self)`.
    busy_told: std::cell::Cell<u64>,
    /// Merge writes (sdk#225b) that published: re-applications of writes
    /// already told `Published`, told to no client.
    merge_published: u64,
    /// Every write's terminal fate until the app reads it (R-b; the pull
    /// API). Recorded in `drain`, the one place every verdict passes.
    fates: crate::fates::Fates,
    /// The (warm, published) roots [`Server::moved`] last reported.
    moved_seen: Option<(freenet_prolly::Cid, freenet_prolly::Cid)>,
    pub page: Page,
    facts: SignerFacts,
    client_version: u16,
    speaker: engine::ClientId,
    identity: bool,
    already_installed: bool,
    /// An `Install` arrived at an unprovisioned page: the Session provisions
    /// the SIGNER (`wire::signer::frame_provision`) when it sees this.
    pub installed: Option<usize>,
    pub provisioned: bool,
    unserved: Vec<u64>,
    page_clamp: BTreeMap<u64, u32>,
    tracing: bool,
    trace: Vec<protocol::Reply>,
    /// The page's client-API ops, taken by the Server as each call ends so
    /// its trace can name what LEFT (the Shell's Put and Head steps), and
    /// handed to the host by [`Server::take_ops`].
    ops: Vec<Op>,
    tracing_of: Option<protocol::TraceOf>,
    out: Vec<Vec<u8>>,
    /// sdk#225b: each write handed to the engine, by (client, write id), as the
    /// FINAL value it leaves at each key (`None`: deleted), until it ends --
    /// refused at the door (`QueueFull`) included: that write id is over, and
    /// the SDK makes the write again as a new one (sdk#450: 75k refusals held
    /// 96 MB here).
    sent: BTreeMap<WriteKey, Sent>,
    /// The writes of this page's LATEST Published commit and the head it was
    /// built on: what a same-identity displacement can take (only the tip can
    /// be displaced at its own seq; a higher seq built on it keeps it).
    tip: Option<Tip>,
    /// The published head last seen, to tell an ADOPTION (it moved with no
    /// Published of ours at the new head) from a commit of ours.
    seen_head: (u64, freenet_prolly::Cid),
    /// How many heads this page has ADOPTED — moved, and NOT by a commit of
    /// this page's (sdk#266) — the one statement of that fact. Its reader
    /// keeps its own cursor: the page's store, to supersede reads pinned to
    /// an older root (#330 ruling).
    adoptions: u64,
    /// Readers' fetches that ENDED, in order: each a ticket a walk is parked
    /// on (READ-STATE). Drained by [`Server::take_fetched`].
    fetched: Vec<(u64, Fetched)>,
    /// A displaced tip being judged key by key at the winner.
    probe: Option<Probe>,
    next_probe: u64,
    /// A same-seq race being MERGED (cell B).
    merge: Option<Merge>,
    /// EACH KEY'S LAST WRITE here (sdk#415): the Published write that last wrote it, the final value it left
    /// there, and whether its commit is BACKED_UP (`ParityComplete`). A row's state is its own last write's fate
    /// (rule 10, ruling A: BACKED_UP is per commit) -- never the tree's, which a later commit's owed parity would
    /// demote. A key no write of this page wrote has no entry: it reads Saved.
    last_write: BTreeMap<Vec<u8>, LastWrite>,
    /// The keys whose last write each Published write is, until its `ParityComplete`.
    last_keys: BTreeMap<WriteKey, Vec<Vec<u8>>>,
    /// Per session, keys whose last write just became BACKED_UP, until the client pulls them
    /// ([`Server::take_backed_up`]): a row's state changed with no root moving, so its binding must be told.
    backed_up_keys: BTreeMap<u64, Vec<Vec<u8>>>,
}

/// A key's last write here (`Server::last_write`). FIXED-SIZE (the architect on #437): the value's DIGEST, never
/// the value -- a copy of the data would be a second holder of it (rule 3), and page memory growing with every
/// key written, beside #411's bound. `None`: the write deleted the key.
pub struct LastWrite {
    write: WriteKey,
    digest: Option<[u8; 32]>,
    backed_up: bool,
}

/// What `LastWrite` keeps of a value.
fn digest_of(v: &Option<Vec<u8>>) -> Option<[u8; 32]> {
    v.as_ref().map(|b| *blake3::hash(b).as_bytes())
}

/// THE MERGE of a same-seq race (sdk#225b part 2, cell B; COMMIT-LIFE K9 §2):
/// the winner was signed from the SAME base P as this page's tip. The
/// displaced GROUP is re-applied IN ORDER onto the winner -- each write again,
/// with its own ops and its own reads, through the engine's queue -- so each
/// is re-judged where it lands (R0): one whose premise moved conflicts and
/// drops out, named, with its dependants cascading (§5); the rest land. A
/// group can partly merge. A key is SUPERSEDED when its LAST writer in the
/// tip did not land again; a write is never told superseded for a key a
/// LATER write of its own group overwrote (it lost to its own app's order).
/// The merge writes are queued like any other and ride the next cut.
///
/// A FORCED write (`Expect::Any`, sdk#235) states no premise, so the engine
/// cannot refuse it where it lands: re-applied as it is, it would write over
/// the winner blind. So the COMPLETE delta P→winner is read first (resumed
/// until done -- a cut delta is unknown, never "not theirs"), and a forced
/// write goes again WITHOUT the keys the winner changed: those are
/// superseded, told, unless the winner already holds exactly this page's
/// value there.
struct Merge {
    winner: (u64, freenet_prolly::Cid),
    writes: TipWrites,
    /// The delta read in flight (its request id) and what has come of it.
    from: freenet_prolly::Cid,
    req: u64,
    theirs: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    /// Per tip write, the keys it did NOT write again because the winner
    /// changed them (a forced write's, above).
    dropped: BTreeMap<usize, Vec<Vec<u8>>>,
    /// Each merge write's id and the tip write it re-applies (its index).
    sent: Vec<(u64, usize)>,
    /// What became of each: `true` Published again.
    landed: BTreeMap<usize, bool>,
    /// The head the merge writes published at.
    published_at: Option<(u64, freenet_prolly::Cid)>,
}

/// The engine client the merge commit is written under: session 0, never a
/// real session — so its verdicts are the Server's, never a client's.
const MERGE_CLIENT: engine::ClientId = engine::ClientId(1);

/// What a write leaves at each key, in key order (`None`: deleted).
type Finals = Vec<(Vec<u8>, Option<Vec<u8>>)>;
/// A write by (engine client, write id).
type WriteKey = (u64, u64);
/// What a write declared it READ.
type Reads = Vec<(Vec<u8>, engine::Expect)>;
/// A write as the Server keeps it: what it left at each key, and what it read.
#[derive(Clone)]
struct Sent {
    finals: Finals,
    reads: Reads,
}
/// A commit's writes.
type TipWrites = Vec<(WriteKey, Sent)>;

/// A Published commit's writes (sdk#225b).
struct Tip {
    head: (u64, freenet_prolly::Cid),
    /// The head this commit was signed from (its ledger's PREV): the base a
    /// same-seq race is merged on.
    base: Option<(u64, freenet_prolly::Cid)>,
    writes: TipWrites,
}

/// The tip's keys, read at the head that displaced it (sdk#225b, cell C —
/// and cell B until the merge commit exists): a key that holds exactly what
/// this page's write left is KEPT (they built on it, or wrote the same); any
/// other value, or a read that could not be answered, is SUPERSEDED, told.
struct Probe {
    winner: (u64, freenet_prolly::Cid),
    writes: TipWrites,
    /// What the tip left at each key: its FINAL value, the last write of the
    /// group in apply order (COMMIT-LIFE K9 §2: a group may write one key
    /// twice).
    left: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    pending: BTreeMap<u64, Vec<u8>>,
    superseded: std::collections::BTreeSet<Vec<u8>>,
}

/// The engine client a READER's fetches go under (READ-STATE, design B): the
/// page's store walks the tree itself and asks the engine only for the blocks
/// a walk stopped on. Session 0, never a real session, so no client's reply
/// is ever taken for one.
const WALK_CLIENT: engine::ClientId = engine::ClientId(2);

/// How a reader's fetch (its TICKET) ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetched {
    /// The engine's read at that root finished: the blocks it reached are here.
    Loaded,
    /// A block could not be had within the read's budget, or the read could
    /// not be held: not "absent", and never answered as empty. WHY, in the
    /// engine's words (a block id, the warm bound) — never a key or a value.
    Unavailable(String),
}

/// The page ANSWERING for its own tree (READ-STATE, design B): whatever hosts
/// a `Server` — `page-io` in a browser, the testkit's scripted node in a test
/// — lends it to the store that reads through it, and carries out what each
/// call produced (node ops go out, replies are kept for the client).
pub trait Host {
    /// Run `f` on the server, then carry out what it produced.
    fn with_server<R>(&mut self, f: impl FnOnce(&mut Server) -> R) -> R;
    /// LOOK at the server: a pull that changes nothing (R-b's `key_state`).
    fn peek<R>(&self, f: impl FnOnce(&Server) -> R) -> R;
    /// A protocol frame from the client.
    fn client(&mut self, frame: &[u8]);
    /// Protocol replies for the client, in order.
    fn take_replies(&mut self) -> Vec<Vec<u8>>;
}

/// The engine client the Server reads under for itself: session 0 is never a
/// real session (`protocol::session_is_valid`), so no client's reply is ever
/// taken for a probe's.
const PROBE_CLIENT: engine::ClientId = engine::ClientId(0);

impl Server {
    /// How many `Busy` verdicts this server has told (see the field).
    pub fn busy_told(&self) -> u64 {
        self.busy_told.get()
    }

    /// Writes whose final values this server holds (`sent`, sdk#225b): each
    /// until its write ends -- any terminal fate, `QueueFull` included.
    pub fn writes_held(&self) -> usize {
        self.sent.len()
    }

    /// Merge writes that published (see the field).
    pub fn merge_published(&self) -> u64 {
        self.merge_published
    }

    /// Writes taken forced past their reads (sdk#235).
    pub fn forced_writes(&self) -> u64 {
        self.page.forced_writes()
    }

    pub fn new(page: Page, facts: SignerFacts) -> Server {
        let mut page = page;
        // This Server merges a same-seq race (sdk#225b) and releases the hold
        // when it does not (`same_identity`).
        page.hold_on_displace();
        Server {
            busy_told: std::cell::Cell::new(0),
            merge_published: 0,
            fates: crate::fates::Fates::default(),
            moved_seen: None,
            page,
            facts,
            client_version: 0,
            speaker: as_client(protocol::LEGACY_SESSION, 1),
            identity: false,
            already_installed: false,
            installed: None,
            provisioned: false,
            unserved: Vec::new(),
            page_clamp: BTreeMap::new(),
            tracing: false,
            trace: Vec::new(),
            ops: Vec::new(),
            tracing_of: None,
            out: Vec::new(),
            sent: BTreeMap::new(),
            tip: None,
            seen_head: (0, [0; 32]),
            adoptions: 0,
            fetched: Vec::new(),
            probe: None,
            next_probe: 1,
            merge: None,
            last_write: BTreeMap::new(),
            last_keys: BTreeMap::new(),
            backed_up_keys: BTreeMap::new(),
        }
    }

    /// The Session learnt more about its signer (after provisioning it).
    pub fn set_facts(&mut self, facts: SignerFacts) {
        self.facts = facts;
    }

    /// A client's protocol frame (shell.rs `handle`, the `Inbound::Client` arm).
    pub fn client(&mut self, bytes: &[u8]) {
        let mut out = Outbound::default();
        self.attribute(bytes);
        match serve(bytes) {
            Served::Do(r, v, session) => {
                // A read before the head is recovered is PARKED by the engine
                // itself (sdk#223) and answered in order once it is; this
                // Server held it until then, and no longer needs to.
                self.client_version = self.client_version.max(v);
                self.speaker = as_client(session, v);
                // The page's id in heads' `through` (COMMIT-LIFE ⁵): its
                // first real session, set once.
                if protocol::session_is_valid(session) && session != protocol::LEGACY_SESSION {
                    let mut device = [0u8; 16];
                    device[..8].copy_from_slice(&session.to_le_bytes());
                    self.page.set_device(device);
                }
                self.on_protocol(r);
            }
            Served::Answer(reply) => {
                out.replies.push(reply_bytes(&reply));
            }
        }
        // A carried try count belongs to the write this call made, if any.
        self.page.clear_carried_tries();
        self.drain(&mut out);
        self.answer_call(&mut out);
        self.out.extend(out.replies);
    }

    /// ONE BUDGET (sdk#265): the next write this Server takes is a `Db`
    /// re-run of a write that had spent `tries`; it draws on from there.
    pub fn carry_tries(&mut self, tries: u32) {
        self.page.carry_tries(tries);
    }

    /// Every write's tries -- dead-commit re-sends and `Db` re-runs alike.
    pub fn max_write_tries(&self) -> u32 {
        self.page.max_write_tries()
    }

    /// Something the NODE (or the signer) answered, as the web layer decoded it.
    pub fn node(&mut self, a: Answer, now_ms: Ms) {
        let mut out = Outbound::default();
        self.page.answer(a, now_ms);
        self.drain(&mut out);
        self.answer_call(&mut out);
        self.out.extend(out.replies);
    }

    /// The page's clock.
    /// The node's `HeadChanged` for the head register: a hint the page acts
    /// on by READING the register (sdk#225's reload trigger).
    pub fn head_hint(&mut self) {
        let mut out = Outbound::default();
        self.page.head_hint();
        self.drain(&mut out);
        self.answer_call(&mut out);
        self.out.extend(out.replies);
    }

    /// The node's `HeadChanged` for the head register WITH its full state
    /// (sdk#378 P3): the page may take it as its own owed head's read-back,
    /// and otherwise treats it as the hint it always was.
    pub fn head_pushed(&mut self, read: crate::HeadRead) {
        let mut out = Outbound::default();
        self.page.head_pushed(read);
        self.drain(&mut out);
        self.answer_call(&mut out);
        self.out.extend(out.replies);
    }

    pub fn tick(&mut self, now_ms: Ms) {
        let mut out = Outbound::default();
        self.page.tick(now_ms);
        self.drain(&mut out);
        self.answer_call(&mut out);
        self.out.extend(out.replies);
    }

    /// Replies for the client, in order.
    pub fn take_replies(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.out)
    }

    /// Client-API operations for the web layer to frame and send.
    pub fn take_ops(&mut self) -> Vec<Op> {
        self.collect_ops();
        std::mem::take(&mut self.ops)
    }

    /// Take what the page wants sent, and name in the trace what LEAVES: the
    /// block PUTs (counted) and the head it asks the signer to sign (its
    /// seq), as the Shell's trace named its node ops. A cold write's trace
    /// that never said Put or Head would hide exactly the hop that breaks.
    fn collect_ops(&mut self) {
        let ops = self.page.take_ops();
        let puts = ops.iter().filter(|o| matches!(o, Op::Put { .. })).count();
        if puts > 0 {
            self.step(1, protocol::Step::Put, puts as u64);
        }
        for op in &ops {
            if let Op::Sign { seq, .. } = op {
                self.step(1, protocol::Step::Head, *seq);
            }
        }
        self.ops.extend(ops);
    }

    /// Every client-facing effect the page produced, turned into replies and
    /// trace steps (shell.rs `handle`, 628–660).
    fn drain(&mut self, out: &mut Outbound) {
        let mut effects = self.page.take_client();
        self.same_identity(&mut effects, out);
        self.keep_fates(&effects);
        self.reply_from(&effects, out);
        self.step(1, protocol::Step::Effects, effects.len() as u64);
        for f in &effects {
            match f {
                Effect::Notify { write_id, state, .. } => {
                    self.tracing_of = Some(protocol::TraceOf::Write(write_id.0));
                    self.step(2, protocol::Step::Reached, state_tag(*state));
                }
                Effect::Reply { req_id, result, .. } => {
                    self.tracing_of = Some(protocol::TraceOf::Read(req_id.0));
                    self.step(2, protocol::Step::Reached, rows_in(result));
                }
                Effect::FetchBlock { .. } => self.step(2, protocol::Step::Fetch, 1),
                _ => {}
            }
        }
        self.collect_ops();
    }

    /// THE SAME IDENTITY, DISPLACED (sdk#225b), before the effects become
    /// replies: the Server's own probe answers are taken out; a Published
    /// write joins the tip; and when the published head moved to a head that
    /// is NOT one of this page's commits, that ADOPTION is classified:
    ///
    /// * the winner names the tip as its `prev` → they built on it: nothing
    ///   was displaced;
    /// * anything else (a same-seq race, another prev, no prev, a refused
    ///   ledger) → each of the tip's keys is READ at the winner: what holds
    ///   exactly this page's value stands (they built on it deeper, or wrote
    ///   the same), everything else is SUPERSEDED and told to its write's
    ///   session. The per-key MERGE (re-applying one-sided keys on the
    ///   winner) replaces the second arm for a same-seq race next.
    fn same_identity(&mut self, effects: &mut Vec<Effect>, out: &mut Outbound) {
        let now = self.page.published();
        let mut published_here = false;
        let mut merge_done = false;
        // The Server's own reads and the merge write's verdicts: never a
        // client's, never replied.
        let mut own = Vec::new();
        effects.retain(|f| match f {
            Effect::Reply { client, .. } if *client == PROBE_CLIENT => {
                own.push(f.clone());
                false
            }
            // A reader's fetch ended: its ticket's end, never a reply.
            Effect::Reply { client, req_id, result } if *client == WALK_CLIENT => {
                let how = match result {
                    engine::read::ReadResult::Page { .. } => Fetched::Loaded,
                    engine::read::ReadResult::Unavailable(cid) => Fetched::Unavailable(format!("block {} could not be had", engine::short_id(cid))),
                    engine::read::ReadResult::OutOfWarmSpace => Fetched::Unavailable("the read needs more than the warm bound holds".into()),
                    _ => Fetched::Unavailable("the engine answered a walk's fetch with something other than a page".into()),
                };
                self.fetched.push((req_id.0, how));
                false
            }
            Effect::Notify { client, .. } if *client == MERGE_CLIENT => {
                own.push(f.clone());
                false
            }
            _ => true,
        });
        for f in own {
            match f {
                Effect::Reply { req_id, result, .. } => self.on_own_read(req_id.0, result),
                Effect::Notify { write_id, state, .. } => {
                    if self.on_merge_verdict(write_id.0, state, now) {
                        published_here = true;
                    }
                    merge_done |= self.merge.as_ref().is_some_and(|m| !m.sent.is_empty() && m.sent.iter().all(|(_, i)| m.landed.contains_key(i)));
                }
                _ => {}
            }
        }
        for f in effects.iter() {
            if let Effect::Notify { client, write_id, state } = f {
                let id = (client.0, write_id.0);
                match state {
                    State::Published => {
                        if let Some(sent) = self.sent.remove(&id) {
                            published_here = true;
                            // Its keys' last write is this one now (sdk#415).
                            let keys: Vec<Vec<u8>> = sent.finals.iter().map(|(k, _)| k.clone()).collect();
                            for (k, v) in &sent.finals {
                                self.last_write.insert(k.clone(), LastWrite { write: id, digest: digest_of(v), backed_up: false });
                            }
                            self.last_keys.insert(id, keys);
                            match self.tip.as_mut() {
                                Some(t) if t.head == now => t.writes.push((id, sent)),
                                _ => {
                                    let base = self.page.last_read().filter(|r| (r.seq, r.root()) == now).and_then(|r| r.prev());
                                    self.tip = Some(Tip { head: now, base, writes: vec![(id, sent)] });
                                }
                            }
                        }
                    }
                    State::Failed { .. } | State::Lost | State::Conflict | State::Unread | State::TooLarge { .. } | State::Unknown | State::QueueFull { .. } => {
                        self.sent.remove(&id);
                    }
                    _ => {}
                }
            }
        }
        if now != self.seen_head {
            // ADOPTED, or OURS? Ours is a commit of this page's that published
            // at this very head — in this call (`published_here`) or in an
            // earlier one, which is exactly what the tip records. Anything
            // else is somebody else's head, and every range the client holds
            // is behind it (sdk#266).
            // The FIRST head this page reads is not an adoption: it is
            // where the page starts, and the client holds nothing yet.
            let first = self.seen_head == (0, [0; 32]);
            let ours = published_here || self.tip.as_ref().is_some_and(|t| t.head == now);
            if !ours && !first {
                self.adoptions += 1;
            }
        }
        if now != self.seen_head && !published_here {
            if let Some(tip) = self.tip.take() {
                if now.0 >= tip.head.0 && now != tip.head {
                    let prev = self.page.last_read().filter(|r| (r.seq, r.root()) == now).and_then(|r| r.prev());
                    match (prev, tip.base) {
                        // They built on the tip: nothing displaced.
                        (Some(p), _) if p == tip.head => {}
                        // A same-seq race from the SAME base: merge.
                        (Some(p), Some(b)) if p == b && now.0 == tip.head.0 => self.start_merge(now, tip.writes, b.1),
                        // Anything else: judge each key at the winner.
                        _ => self.start_probe(now, tip.writes),
                    }
                } else {
                    self.tip = Some(tip);
                }
            }
        }
        self.seen_head = now;
        if merge_done {
            self.finish_merge(out);
        }
        // The cut is held (a same-seq displacement, `Page::hold_on_displace`)
        // only while a merge waits for its delta; with none, it goes on.
        if self.page.cut_held() && !self.merge.as_ref().is_some_and(|m| m.sent.is_empty()) {
            self.page.release_cut();
        }
        // The Server's own reads (or the merge write) may have answered at once.
        let more = self.page.take_client();
        if !more.is_empty() {
            let mut more = more;
            self.same_identity(&mut more, out);
            effects.extend(more);
        }
        self.finish_probe(out);
    }

    /// Every verdict to a client, kept as its write's fate (R-b).
    fn keep_fates(&mut self, effects: &[Effect]) {
        let seq = self.page.published().0;
        // Said in the step that emitted these notices, and read HERE only: taken whole, so none outlives its batch.
        let mut signer_why = std::mem::take(&mut self.page.signer_why);
        for f in effects {
            match f {
                Effect::Notify { client, write_id, state } => {
                    self.fates.told((session_of(*client), write_id.0), state, seq);
                    // The signer's own why beside its refusal (sdk#500), said by the page in the same step -- read at
                    // the refusal's notice, never an earlier one of the same write (its `Accepted` is in this batch too).
                    if *state == (State::Failed { why: engine::FailWhy::SignerRefused }) {
                        if let Some(why) = signer_why.remove(&(*client, *write_id)) {
                            self.fates.signer_refused((session_of(*client), write_id.0), why);
                        }
                    }
                    // BACKED_UP: the keys whose last write this is (sdk#415).
                    if *state == State::ParityComplete {
                        for k in self.last_keys.remove(&(client.0, write_id.0)).unwrap_or_default() {
                            if let Some(l) = self.last_write.get_mut(&k).filter(|l| l.write == (client.0, write_id.0)) {
                                l.backed_up = true;
                                self.backed_up_keys.entry(session_of(*client)).or_default().push(k);
                            }
                        }
                    }
                }
                Effect::Conflicted { client, write_id, key, current, after, tries } => self.fates.conflicted(
                    (session_of(*client), write_id.0),
                    key.clone(),
                    current.as_ref().map(|f| f.hash()),
                    after.map(|(c, w)| (session_of(c), w.0)),
                    *tries,
                ),
                Effect::Unread { client, write_id, key } => self.fates.unread((session_of(*client), write_id.0), key.clone()),
                _ => {}
            }
        }
    }

    // ---- THE PULL API (R-b; READ-STATE § The pull API) ----

    /// What became of `session`'s write `write_id`: its stage while it is in
    /// the queue, else its terminal fate -- READ, so it is gone after this.
    /// `None`: never made here, or its fate was read (or dropped past the
    /// bound, counted in [`Server::fates_dropped`]).
    pub fn fate(&mut self, session: u64, write_id: u64) -> Option<crate::fates::Fate> {
        if let Some(stage) = self.stage_of(session, write_id) {
            return Some(stage);
        }
        self.fates.take((session, write_id))
    }

    /// Every unread terminal fate of `session`, in the order they ended. READ.
    pub fn take_fates(&mut self, session: u64) -> Vec<(u64, crate::fates::Fate)> {
        self.fates.take_session(session)
    }

    /// The bytes the per-key last-write record holds (#437: keys and fixed-size entries, never values).
    pub fn last_write_bytes(&self) -> usize {
        self.last_write.keys().map(|k| k.len() + std::mem::size_of::<LastWrite>()).sum()
    }

    /// `session`'s keys whose last write became BACKED_UP since this was last asked (sdk#415). Drains. The write
    /// itself ended at `Published` (its fate is read and gone), so this is the one way its row's move to "backed
    /// up" -- which moves no root -- reaches the client's bindings.
    pub fn take_backed_up(&mut self, session: u64) -> Vec<Vec<u8>> {
        self.backed_up_keys.remove(&session).unwrap_or_default()
    }

    /// `session`'s writes in the queue, in order, with their stages.
    pub fn queued_of(&self, session: u64) -> Vec<(u64, crate::fates::Fate)> {
        self.page
            .queue_stages()
            .into_iter()
            .filter(|(c, _, _)| session_of(*c) == session)
            .map(|(_, w, s)| (w.0, stage_fate(s)))
            .collect()
    }

    /// This session's UNSAVED writes: the engine's one rule (a held deferred
    /// write is not one, sdk#350), counted for this session.
    pub fn unsaved_of(&self, session: u64) -> usize {
        self.page.unsaved_clients().into_iter().filter(|c| session_of(*c) == session).count()
    }

    fn stage_of(&self, session: u64, write_id: u64) -> Option<crate::fates::Fate> {
        self.page
            .queue_stages()
            .into_iter()
            .find(|(c, w, _)| session_of(*c) == session && w.0 == write_id)
            .map(|(_, _, s)| stage_fate(s))
    }

    /// `session`'s conflicts not yet taken for `Db`'s re-run (#249), each
    /// with the writes that cascade from it: `(write ids, keys)`. Drained.
    pub fn take_conflicted(&mut self, session: u64) -> Vec<(Vec<u64>, Vec<Vec<u8>>, u32)> {
        self.fates.take_conflicted(session)
    }

    /// Where `key` stands (R-b): `Saving` while the warm and published roots
    /// differ at it; else its LAST WRITE's fate (sdk#415; rule 10, ruling A):
    /// `SavedAndBackedUp` once that write's commit is BACKED_UP
    /// (`ParityComplete`) and the published tree still holds what it left,
    /// else `Saved`. Never the tree's parity state (`parity_scan`,
    /// `owed_groups`): that is the engine's bookkeeping and the assets
    /// dashboard's input, and a later commit's owed parity would demote a row
    /// its own commit backed up. `None` when a block either walk needs is not
    /// held.
    pub fn key_state(&self, key: &[u8]) -> Option<KeyState> {
        let (warm, published) = self.heads()?;
        let get = |root: &freenet_prolly::Cid| match self.page.walk(root, &engine::read::Walk::Get(key.to_vec())) {
            engine::read::Walked::Done(engine::read::ReadResult::Value(v)) => Some(v),
            _ => None,
        };
        let now = get(&published)?;
        if warm != published && get(&warm)? != now {
            return Some(KeyState::Saving);
        }
        let backed = self.last_write.get(key).is_some_and(|l| l.backed_up && l.digest == digest_of(&now));
        Some(if backed { KeyState::SavedAndBackedUp } else { KeyState::Saved })
    }

    /// The (warm, published) roots: warm for this page's own editing,
    /// published for its own tree when read as another user. `None` before the head is recovered.
    pub fn heads(&self) -> Option<(freenet_prolly::Cid, freenet_prolly::Cid)> {
        self.page.recovered().then(|| (self.page.warm_root(), self.page.published().1))
    }

    /// A WAKE-UP only: either root moved since the last call. Each binding
    /// then diffs from its own rendered root to `heads()`.
    pub fn moved(&mut self) -> bool {
        let now = self.heads();
        let moved = now.is_some() && now != self.moved_seen;
        if moved {
            self.moved_seen = now;
        }
        moved
    }

    /// Does a write still APPLYING (in no root) write a key in `[lo, hi)`?
    /// A read of that range waits for it, un-pinned (R-b: the pin trap).
    pub fn applying_touches(&self, lo: &[u8], hi: &[u8]) -> bool {
        self.page.applying_touches(lo, hi)
    }

    /// The first of `session`'s writes still `Applying`: what its client
    /// asks after (sdk#174).
    pub fn first_applying_of(&self, session: u64) -> Option<u64> {
        self.page
            .queue_stages()
            .into_iter()
            .find(|(c, _, s)| session_of(*c) == session && *s == engine::Stage::Applying)
            .map(|(_, w, _)| w.0)
    }

    /// What the last queued write on `key` is doing (R-b): `Applying` or
    /// `Queued` (not gone yet) or `Committing` (gone, unanswered). `None`:
    /// no write in the queue touches it.
    pub fn key_stage(&self, key: &[u8]) -> Option<engine::Stage> {
        self.page.stage_of_key(key)
    }

    /// The page's write queue: writes and their bytes.
    pub fn queue_load(&self) -> (usize, usize) {
        self.page.queue_load()
    }

    /// The root a walk of THIS page's tree reads (READ-STATE inv. 6): the
    /// warm root, this page's accepted writes applied. `None` until the head
    /// is recovered — before it the only tree is the empty one, and a walk
    /// of it would answer "nothing here" for a tree nobody has read.
    pub fn read_root(&self) -> Option<freenet_prolly::Cid> {
        self.page.recovered().then(|| self.page.warm_root())
    }

    /// Walk the tree at `root`, now, fetching nothing.
    pub fn walk(&self, root: &freenet_prolly::Cid, walk: &engine::read::Walk) -> engine::read::Walked {
        self.page.walk(root, walk)
    }

    /// A walk stopped on missing blocks: the engine reads `range` at `root`
    /// (or, before the head is recovered, at the head once it is), fetching
    /// what it needs, and its end is reported under `ticket` by
    /// [`Server::take_fetched`]. Every such read ENDS: answered, or given up
    /// within its rounds and GETs — never silent.
    pub fn fetch(&mut self, ticket: u64, root: Option<freenet_prolly::Cid>, range: freenet_prolly::range::Range) {
        let mut out = Outbound::default();
        let req_id = as_req_id(ticket);
        let range = Box::new(range);
        self.page.event(match root {
            Some(root) => Event::ScanAt { client: WALK_CLIENT, req_id, root, range },
            None => Event::Scan { client: WALK_CLIENT, req_id, range },
        });
        self.drain(&mut out);
        self.answer_call(&mut out);
        self.out.extend(out.replies);
    }

    /// How many heads this page has ADOPTED (not its own commits) so far.
    pub fn adoptions(&self) -> u64 {
        self.adoptions
    }

    /// A newer head was adopted: supersede the reader's fetch under `ticket`
    /// if it made no progress ([`Page::supersede_read`]). Whether it was; the
    /// store ends the ticket, `Superseded`.
    pub fn supersede_fetch(&mut self, ticket: u64) -> bool {
        self.page.supersede_read(as_req_id(ticket))
    }

    /// Readers' fetches that ended since the last call, in order. Drains.
    pub fn take_fetched(&mut self) -> Vec<(u64, Fetched)> {
        std::mem::take(&mut self.fetched)
    }

    /// One of the Server's own reads answered: a probe's `Get`, or a page of
    /// the merge's delta.
    fn on_own_read(&mut self, req: u64, result: engine::read::ReadResult) {
        if let Some(p) = self.probe.as_mut() {
            if let Some(key) = p.pending.remove(&req) {
                let holds = matches!(&result, engine::read::ReadResult::Value(v) if Some(v) == p.left.get(&key));
                if !holds {
                    p.superseded.insert(key);
                }
                return;
            }
        }
        let Some(m) = self.merge.as_mut().filter(|m| m.req == req && m.sent.is_empty()) else { return };
        match result {
            engine::read::ReadResult::Delta { changes, cursor, .. } => {
                m.theirs.extend(changes);
                match cursor {
                    // CUT: not "absent in theirs" -- resume after it.
                    Some(c) => {
                        let rid = self.next_probe;
                        self.next_probe += 1;
                        m.req = rid;
                        let from = m.from;
                        self.page.event(Event::ChangesSince {
                            client: PROBE_CLIENT,
                            req_id: as_req_id(rid),
                            from,
                            range: engine::subs::SubRange { lo: std::ops::Bound::Excluded(c), hi: std::ops::Bound::Unbounded },
                            max_entries: 256,
                        });
                    }
                    None => self.send_merge(),
                }
            }
            // The base's blocks are gone, or a block could not be had: the
            // delta cannot be known, so the keys are judged one by one at the
            // winner instead (the conservative cell).
            _ => {
                let m = self.merge.take().expect("checked");
                self.start_probe(m.winner, m.writes);
            }
        }
    }

    fn start_merge(&mut self, winner: (u64, freenet_prolly::Cid), writes: TipWrites, from: freenet_prolly::Cid) {
        let rid = self.next_probe;
        self.next_probe += 1;
        self.merge = Some(Merge { winner, writes, from, req: rid, theirs: BTreeMap::new(), dropped: BTreeMap::new(), sent: Vec::new(), landed: BTreeMap::new(), published_at: None });
        self.page.event(Event::ChangesSince {
            client: PROBE_CLIENT,
            req_id: as_req_id(rid),
            from,
            range: engine::subs::SubRange { lo: std::ops::Bound::Unbounded, hi: std::ops::Bound::Unbounded },
            max_entries: 256,
        });
    }

    /// Their COMPLETE delta is in: the group goes again, in order, each
    /// write as its own engine write -- a forced one without the keys the
    /// winner changed.
    fn send_merge(&mut self) {
        let Some(m) = self.merge.as_mut() else { return };
        let mut events = Vec::new();
        for (i, (_, w)) in m.writes.iter().enumerate() {
            let forced = |k: &Vec<u8>| w.reads.iter().any(|(rk, e)| rk == k && *e == engine::Expect::Any);
            let mut dropped = Vec::new();
            let ops: Vec<(Vec<u8>, engine::Op)> = w
                .finals
                .iter()
                .filter(|(k, _)| {
                    let keep = !(forced(k) && m.theirs.contains_key(k));
                    if !keep {
                        dropped.push(k.clone());
                    }
                    keep
                })
                .map(|(k, v)| (k.clone(), v.clone().map_or(engine::Op::Delete, engine::Op::Put)))
                .collect();
            if !dropped.is_empty() {
                m.dropped.insert(i, dropped);
            }
            if ops.is_empty() {
                // Nothing of it goes again: it "landed" as far as the merge
                // goes, and its dropped keys speak for themselves.
                m.landed.insert(i, true);
                continue;
            }
            let reads: Vec<(Vec<u8>, engine::Expect)> = w.reads.iter().filter(|(k, _)| ops.iter().any(|(ok, _)| ok == k) || !forced(k)).cloned().collect();
            let wid = self.next_probe;
            self.next_probe += 1;
            m.sent.push((wid, i));
            events.push((as_write_id(wid), ops, reads));
        }
        let nothing = events.is_empty();
        if !nothing {
            // At the FRONT, before every later write of this page (review §1
            // on sdk#295): the cut was held since the displacing head.
            self.page.merge_front(MERGE_CLIENT, events);
        }
        if nothing {
            let mut out = Outbound::default();
            self.finish_merge(&mut out);
            self.out.extend(out.replies);
        }
    }

    /// A verdict on a merge write. True when it PUBLISHED.
    fn on_merge_verdict(&mut self, write_id: u64, state: State, now: (u64, freenet_prolly::Cid)) -> bool {
        let Some(m) = self.merge.as_mut() else { return false };
        let Some(&(_, i)) = m.sent.iter().find(|(w, _)| *w == write_id) else { return false };
        match state {
            State::Published => {
                m.landed.insert(i, true);
                m.published_at = Some(now);
                self.merge_published += 1;
                true
            }
            // It could not land where it was judged (or its fate cannot be
            // known): its keys are superseded, told -- never blind. Never
            // `QueueFull`: a merge write goes in at the front, past the bound
            // (its bytes were counted when first taken).
            State::Lost | State::Conflict | State::Unread | State::Failed { .. } | State::TooLarge { .. } | State::Unknown => {
                m.landed.insert(i, false);
                false
            }
            _ => false,
        }
    }

    /// The merge is over: the writes that landed again are this page's tip;
    /// each write's session is told its keys the winner replaced (the keys
    /// whose last writer did not land again).
    fn finish_merge(&mut self, out: &mut Outbound) {
        let Some(m) = self.merge.take() else { return };
        let last = last_writers(&m.writes);
        let left = left_of(&m.writes);
        let superseded: std::collections::BTreeSet<Vec<u8>> = last
            .iter()
            .filter(|(k, i)| {
                let fell = !m.landed.get(*i).copied().unwrap_or(false);
                let dropped = m.dropped.get(*i).is_some_and(|d| d.contains(*k)) && m.theirs.get(*k) != left.get(*k);
                fell || dropped
            })
            .map(|(k, _)| k.clone())
            .collect();
        if let Some(head) = m.published_at {
            let writes: TipWrites = m.writes.iter().enumerate().filter(|(i, _)| m.landed.get(i).copied().unwrap_or(false)).map(|(_, w)| w.clone()).collect();
            self.tip = Some(Tip { head, base: Some(m.winner), writes });
        }
        tell_superseded(out, m.winner, &m.writes, &superseded);
    }

    fn start_probe(&mut self, winner: (u64, freenet_prolly::Cid), writes: TipWrites) {
        let left = left_of(&writes);
        let keys: Vec<Vec<u8>> = left.keys().cloned().collect();
        let mut pending = BTreeMap::new();
        let mut reqs = Vec::new();
        for key in keys {
            let rid = self.next_probe;
            self.next_probe += 1;
            pending.insert(rid, key.clone());
            reqs.push((rid, key));
        }
        self.probe = Some(Probe { winner, writes, left, pending, superseded: Default::default() });
        for (rid, key) in reqs {
            self.page.event(Event::Get { client: PROBE_CLIENT, req_id: as_req_id(rid), key });
        }
    }

    /// Every probe answered: tell each write's session which of its keys the
    /// winner replaced.
    fn finish_probe(&mut self, out: &mut Outbound) {
        if !self.probe.as_ref().is_some_and(|p| p.pending.is_empty()) {
            return;
        }
        let p = self.probe.take().expect("checked");
        tell_superseded(out, p.winner, &p.writes, &p.superseded);
    }

    /// The answers that are about the CALL, not an effect (shell.rs 663–700).
    fn answer_call(&mut self, out: &mut Outbound) {
        if self.identity {
            self.identity = false;
            let (head_seq, head_root) = self.page.published();
            out.replies.push(reply_bytes(&protocol::Reply::Identity {
                engine: concat!("craftworks-engine/", env!("CARGO_PKG_VERSION")).into(),
                key_source: "Provisioned(Test)".into(),
                head_seq,
                head_root,
                head_writable: self.facts.head_writable,
                head_id: self.facts.head_id,
            }));
        }
        if self.already_installed {
            self.already_installed = false;
            out.replies.push(reply_bytes(&protocol::Reply::AlreadyInstalled));
        }
        for req_id in std::mem::take(&mut self.unserved) {
            out.replies.push(reply_bytes(&protocol::Reply::Unavailable { req_id, blocked_on: [0u8; 32] }));
        }
        for r in std::mem::take(&mut self.trace) {
            out.replies.push(reply_bytes(&r));
        }
    }

    fn step(&mut self, depth: u8, what: protocol::Step, n: u64) {
        if !self.tracing || self.trace.len() >= MAX_STEPS {
            return;
        }
        let Some(of) = self.tracing_of else {
            // A step with nothing to attribute it to is a line in a log. The
            // whole point of a call TREE is that every step belongs to an
            // operation, so one that does not is dropped rather than emitted
            // under a made-up id.
            return;
        };
        self.trace
            .push(protocol::Reply::Step { of, depth, what, n });
    }

    fn on_protocol(&mut self, r: protocol::Request) -> Vec<Effect> {
        use protocol::Request as P;
        // ONE conversion, used by every arm that takes a range. Three arms
        // take one now; three copies of this would be three places for the
        // bounds to be read differently.
        fn bound(b: protocol::Bound) -> std::ops::Bound<Vec<u8>> {
            use std::ops::Bound as B;
            match b {
                protocol::Bound::Unbounded => B::Unbounded,
                protocol::Bound::Included(k) => B::Included(k),
                protocol::Bound::Excluded(k) => B::Excluded(k),
            }
        }
        // A VIEW WRITES NOTHING, at the door (read-only has one owner, the
        // page): its write is told `Failed` -- terminal, nothing applied --
        // by name, and the engine never queues it.
        if self.page.read_only() {
            if let P::Write { write_id, .. } | P::Commit { write_id, .. } | P::DeferredCommit { write_id, .. } = &r {
                let id = *write_id;
                self.page.say(instrument::Site::of("page::said::read-only-write"), format!("read-only: write {id} refused at the door (a view writes nothing)"));
                self.page.client_fx.push(Effect::Notify { client: self.speaker, write_id: as_write_id(id), state: State::Failed { why: engine::FailWhy::ReadOnly } });
                return Vec::new();
            }
        }
        let ev = match r {
            // v5's write (craftworks-sdk#183). Cannot arrive: decode refuses it
            // below v5, and `serve` answers every v5 frame `Unsupported` until
            // the engine implements the order rule (build step 2). NOT a panic
            // if it ever does — a panic in a delegate closes the engine.
            P::WriteFrom { .. } => {
                debug_assert!(false, "a WriteFrom reached the shell: serve must answer v5 Unsupported until step 2");
                return Vec::new();
            }
            P::Identity => {
                // Identity is also how a session BEGINS. There is no separate
                // `start` in the protocol and there should not be: answering
                // "where is your head" requires reading it, so the engine is
                // started here and the head read goes out with this call.
                //
                // The answer carries the head as known RIGHT NOW — 0 on a
                // brand-new engine that has never published, the real seq on
                // a rehydrated one, since a rehydrated engine carries its
                // published seq in its context. A client that wants the
                // freshest asks again after the read lands.
                self.identity = true;
                Event::Start {
                    key: KeySource::Provisioned(engine::Provisioned::Test),
                    epochs: vec![as_epoch(1)],
                }
            }
            P::Get { req_id, key } => Event::Get {
                client: self.speaker,
                req_id: as_req_id(req_id),
                key,
            },
            P::Write { write_id, ops } => write_event(self.speaker, write_id, Vec::new(), ops, false),
            // M2 (sdk#148): a write that says what it READ. The reads go to the
            // engine, which checks them where the ops land.
            P::Commit { write_id, reads, ops } => write_event(self.speaker, write_id, reads, ops, false),
            // sdk#350: the same write, committed only IN COMPANY.
            P::DeferredCommit { write_id, reads, ops } => write_event(self.speaker, write_id, reads, ops, true),
            // LIVE (sdk#174, #257): `Session::tick` → `Writes::ask_after` sends
            // it for the client's oldest `Applying` write, at most once a
            // second. `on_ask` re-asks a parked write's missing blocks and
            // resets its idle count, which is what keeps
            // `release_silent_parked_write` from failing a write whose GET is
            // merely slow. Keyed by the asking session like every request.
            P::AskWrite { write_id } => Event::AskWrite {
                client: self.speaker,
                write_id: as_write_id(write_id),
            },
            P::Tick { now } => Event::Tick(now),
            P::Flush => Event::Flush,
            P::Range {
                req_id,
                lo,
                hi,
                reverse,
                after,
                max_entries,
            } => {
                // CLAMPED here, and the clamp is reported back rather than
                // applied silently: a caller that asked for a thousand rows
                // and got a hundred needs to know the page it holds is not
                // the page it asked for, or it will read the short answer as
                // the end of the range.
                let clamped = (max_entries as usize).clamp(1, MAX_PAGE_ENTRIES);
                self.page_clamp.insert(req_id, clamped as u32);
                Event::Scan {
                    client: self.speaker,
                    req_id: as_req_id(req_id),
                    range: Box::new(freenet_prolly::range::Range {
                        lo: bound(lo),
                        hi: bound(hi),
                        reverse,
                        after,
                        max_entries: clamped,
                        max_bytes: MAX_PAGE_BYTES,
                    }),
                }
            }
            P::Preload { roots } => Event::Preload {
                client: self.speaker,
                // Fixed-width ids off the wire. A root is 32 bytes; anything
                // else is not one, and the engine's budget bounds how many of
                // them are walked.
                roots: roots.into_iter().collect(),
            },
            P::SubscribeRange { sub_id, lo, hi } => Event::SubscribeRange {
                client: self.speaker,
                sub_id,
                range: engine::subs::SubRange {
                    lo: bound(lo),
                    hi: bound(hi),
                },
            },
            // The ENGINE's copy only. The node has no unsubscribe, so a
            // delegate goes on being woken for contracts its engine has
            // forgotten — which is ordinary, not an error, and is why nothing
            // here treats an unknown wake-up as one. Writing a release path
            // that silently does nothing at the node would be worse than
            // having none.
            P::Unsubscribe { sub_id } => Event::Unsubscribe {
                client: self.speaker,
                sub_id,
            },
            P::ChangesSince {
                req_id,
                from,
                lo,
                hi,
                max_entries,
            } => Event::ChangesSince {
                client: self.speaker,
                req_id: as_req_id(req_id),
                from,
                range: engine::subs::SubRange {
                    lo: bound(lo),
                    hi: bound(hi),
                },
                max_entries: max_entries as usize,
            },
            // Still v1 vocabulary this shell does not serve. Answered as such
            // rather than silently ignored: a client that asked and heard
            // nothing cannot tell "not implemented" from "lost".
            P::Trace { on } => {
                self.tracing = on;
                return Vec::new();
            }
            P::Subscribe { .. } => {
                self.unserved.push(0);
                return Vec::new();
            }
            P::Install {
                block_code,
                register_code,
                signing_key,
                ..
            } => {
                // FIRST WRITER WINS, and the guard is HERE rather than in any
                // page.
                //
                // `Install` overwrites the signing key, and the Register
                // instance is derived from a keyset — so a second install
                // mints a second key, moves the head's contract id, and
                // orphans everything written under the first. A polite client
                // does not prevent that: two tabs opened together on a fresh
                // node both find it unprovisioned and both install, a
                // re-issue after a stall installs again, an older page knows
                // nothing of the rule, and any web page at all can send one
                // message. The delegate is the only place that sees them all.
                //
                // So an install over a provisioned delegate changes NOTHING
                // and says so. It is the ordinary outcome of a race, not an
                // error. Replacing a key or the contract code is the hand-over
                // design in sdk#14, never a blind overwrite.
                if self.facts.head_writable {
                    self.already_installed = true;
                    return Vec::new();
                }
                // Handled by the entry point, which is the only place with a
                // secret store. The shell records that it was asked, so a
                // caller can tell "installed" from "never arrived".
                self.installed = Some(block_code.len() + register_code.len());
                let _ = &signing_key;
                self.provisioned = true;
                return Vec::new();
            }
        };
        // PORT: the engine is the page's; its effects are read back through
        // `Page::take_client` by the caller, as `handle` read `effects`.
        // sdk#225b: what each write leaves at each key, in case its commit is
        // displaced by another device's.
        if let Event::Write { client, write_id, ops, reads, .. } = &ev {
            let mut finals: BTreeMap<Vec<u8>, Option<Vec<u8>>> = BTreeMap::new();
            for (k, o) in ops {
                finals.insert(k.clone(), match o {
                    engine::Op::Put(v) => Some(v.clone()),
                    engine::Op::Delete => None,
                });
            }
            self.sent.insert((client.0, write_id.0), Sent { finals: finals.into_iter().collect(), reads: reads.clone() });
        }
        self.page.event(ev);
        Vec::new()
    }

    fn reply_from(&self, effects: &[Effect], out: &mut Outbound) {
        use protocol::WriteState as W;
        for f in effects {
            let r = match f {
                Effect::Notify {
                    client, write_id, state,
                } => {
                    let version = version_of(*client);
                    let state = match state {
                        State::Accepted => W::Accepted,
                        State::Stalled => W::Stalled,
                        State::Published => W::Published,
                        State::ParityComplete => W::ParityComplete,
                        State::Busy => {
                            self.busy_told.set(self.busy_told.get() + 1);
                            W::Busy
                        }
                        // sdk#500: the cause named; `for_client` tells a pre-v4 client `Failed`.
                        State::Failed { why } => W::FailedWhy { why: *why },
                        State::Lost => W::Lost,
                        // ⁵: may have landed; the app is told to check.
                        State::Unknown => W::Unknown,
                        // M2: nothing applied, a read no longer held. `for_client`
                        // tells a pre-v4 client `Failed`, which is true to it.
                        State::Conflict => W::Conflict,
                        // sdk#235: refused at the door, a key written unread.
                        // `for_client` tells a pre-v4 client `Failed`.
                        State::Unread => W::Unread,
                        // R-b: the page's queue at its bound, by name.
                        State::QueueFull { bytes, limit } => W::QueueFull {
                            bytes: protocol::saturating_u32(*bytes),
                            limit: protocol::saturating_u32(*limit),
                        },
                        State::TooLarge { bound, limit, got } => W::too_large(
                            match bound {
                                engine::WriteBound::CommitBlocks => {
                                    protocol::WriteBound::CommitBlocks
                                }
                                engine::WriteBound::WriteBytes => protocol::WriteBound::WriteBytes,
                            },
                            *limit,
                            *got,
                        ),
                    }
                    // THE ONE PLACE a write state leaves the delegate, so the
                    // one place a client is told what IT can read — in the
                    // version of the write's own client, never of whichever
                    // tab happened to speak in this call.
                    .for_client(version);
                    // Named only for a writer that can read it AND has a session
                    // to name: a v4 frame that sent the legacy session is as
                    // indistinguishable as any pre-v4 page, and gains nothing.
                    if version >= protocol::SESSION_SINCE && session_of(*client) != protocol::LEGACY_SESSION {
                        protocol::Reply::SessionWriteState {
                            session: session_of(*client),
                            write_id: write_id.0,
                            state,
                        }
                    } else {
                        protocol::Reply::WriteState { write_id: write_id.0, state }
                    }
                }
                // WHICH read no longer held, and what the tree holds there now
                // (M2) — to a v4 writer with a session (the same bundle: see
                // `protocol::Request::Commit`). An older one has its `Failed`.
                // `after` (a cascade's cause, R-b) is pulled, not sent:
                // `Server::conflicted()`.
                Effect::Conflicted { client, write_id, key, current, .. } => {
                    let version = version_of(*client);
                    if version < protocol::SESSION_SINCE || session_of(*client) == protocol::LEGACY_SESSION {
                        continue;
                    }
                    protocol::Reply::Conflicted {
                        session: session_of(*client),
                        write_id: write_id.0,
                        key: key.clone(),
                        current: current.as_ref().map(|f| f.hash()),
                    }
                }
                // WHICH key a write changed unread (sdk#235) — to the same
                // bundle only, as `Conflicted`; an older client has `Failed`.
                Effect::Unread { client, write_id, key } => {
                    let version = version_of(*client);
                    if version < protocol::SESSION_SINCE || session_of(*client) == protocol::LEGACY_SESSION {
                        continue;
                    }
                    protocol::Reply::Unread { session: session_of(*client), write_id: write_id.0, key: key.clone() }
                }
                Effect::Reply { req_id, result, .. } => {
                    // WHERE THIS ENGINE STANDS, as it answers.
                    //
                    // Taken once, here, so every answer in this call reports the
                    // same head — two answers from one call describing two trees
                    // would be the very confusion `At` exists to remove.
                    let (seq, root) = self.page.published();
                    let at = protocol::At { seq, root };
                    match result {
                        engine::read::ReadResult::Value(v) => protocol::Reply::Value {
                            req_id: req_id.0,
                            value: v.clone(),
                        },
                        engine::read::ReadResult::Page {
                            entries, cursor, ..
                        } => protocol::Reply::Page {
                            req_id: req_id.0,
                            entries: entries.clone(),
                            cursor: cursor.clone(),
                            // What was USED, not what was asked for.
                            max_entries: self
                                .page_clamp
                                .get(&req_id.0)
                                .copied()
                                .unwrap_or(entries.len() as u32),
                            at,
                        },
                        engine::read::ReadResult::Unavailable(cid) => {
                            protocol::Reply::Unavailable {
                                req_id: req_id.0,
                                blocked_on: *cid,
                            }
                        }
                        engine::read::ReadResult::OutOfWarmSpace => protocol::Reply::Unavailable {
                            req_id: req_id.0,
                            blocked_on: [0u8; 32],
                        },
                        engine::read::ReadResult::Delta {
                            changes,
                            cursor,
                            new_root,
                        } => protocol::Reply::Delta {
                            req_id: req_id.0,
                            changes: changes.clone(),
                            cursor: cursor.clone(),
                            new_root: *new_root,
                            at,
                        },
                        engine::read::ReadResult::FullReloadRequired { new_root } => {
                            protocol::Reply::FullReloadRequired {
                                req_id: req_id.0,
                                new_root: *new_root,
                                at,
                            }
                        }
                    }
                }
                // A subscribed range moved. PUSHED — no client asked for this
                // message, which is the whole point of it.
                Effect::Changed {
                    sub_id,
                    new_root,
                    seq,
                    why,
                    ..
                } => protocol::Reply::Changed {
                    sub_id: *sub_id,
                    new_root: *new_root,
                    seq: *seq,
                    why: match why {
                        engine::subs::Why::Diffed => protocol::Why::Diffed,
                        engine::subs::Why::BlockMissing => protocol::Why::BlockMissing,
                        engine::subs::Why::Budgeted => protocol::Why::Budgeted,
                        engine::subs::Why::Stale => protocol::Why::Stale,
                    },
                },
                // A subscribe was taken or refused. Answered either way: a
                // client that believes it is subscribed and is not waits for
                // ever, and nothing it can see would tell it so.
                Effect::Subscribed {
                    sub_id, accepted, ..
                } => protocol::Reply::Subscribed {
                    sub_id: *sub_id,
                    accepted: match accepted {
                        engine::subs::Accepted::Yes => protocol::Accepted::Yes,
                        engine::subs::Accepted::Full => protocol::Accepted::Full,
                        engine::subs::Accepted::TooWide => protocol::Accepted::TooWide,
                    },
                },
                _ => continue,
            };
            out.replies.push(reply_bytes(&r));
        }
    }

    fn attribute(&mut self, bytes: &[u8]) {
        if !self.tracing {
            return;
        }
        let Served::Do(r, _, _) = serve(bytes) else {
            return;
        };
        let (of, began) = match &r {
            // A write is traced whatever its shape: since sdk#235 every write
            // the SDK sends is a `Commit` (a forced one reads `Any`), so a
            // trace that started only on `Write` went silent.
            protocol::Request::Write { write_id, ops } | protocol::Request::Commit { write_id, ops, .. } | protocol::Request::DeferredCommit { write_id, ops, .. } => {
                (protocol::TraceOf::Write(*write_id), ops.len() as u64)
            }
            protocol::Request::Get { req_id, .. }
            | protocol::Request::Range { req_id, .. }
            | protocol::Request::ChangesSince { req_id, .. } => {
                (protocol::TraceOf::Read(*req_id), 0)
            }
            _ => return,
        };
        self.tracing_of = Some(of);
        self.step(0, protocol::Step::Began, began);
    }
}

/// Every write shape on the wire as the ONE engine write event: a
/// `DeferredCommit` (sdk#350) is only the wire's form of the flag.
fn write_event(speaker: engine::ClientId, write_id: u64, reads: Vec<(Vec<u8>, protocol::Expect)>, ops: Vec<protocol::Op>, deferred: bool) -> Event {
    Event::Write {
        client: speaker,
        write_id: as_write_id(write_id),
        ops: ops
            .into_iter()
            .map(|o| match o {
                protocol::Op::Put(k, v) => (k, engine::Op::Put(v)),
                protocol::Op::Delete(k) => (k, engine::Op::Delete),
            })
            .collect(),
        reads: reads
            .into_iter()
            .map(|(k, e)| {
                (
                    k,
                    match e {
                        protocol::Expect::Absent => engine::Expect::Absent,
                        protocol::Expect::Present => engine::Expect::Present,
                        protocol::Expect::Value(h) => engine::Expect::Value(h),
                        protocol::Expect::Any => engine::Expect::Any,
                    },
                )
            })
            .collect(),
        deferred,
    }
}

/// `engine-delegate/src/serve.rs`, verbatim.
enum Served {
    Do(Request, u16, u64),
    Answer(Reply),
}

/// What a tip left at each key: its FINAL value, the last write of the group
/// in apply order (COMMIT-LIFE K9 §2).
fn left_of(writes: &TipWrites) -> BTreeMap<Vec<u8>, Option<Vec<u8>>> {
    let mut left: BTreeMap<Vec<u8>, Option<Vec<u8>>> = BTreeMap::new();
    for (_, w) in writes {
        for (k, v) in &w.finals {
            left.insert(k.clone(), v.clone());
        }
    }
    left
}

/// Which tip write (its index) wrote each key LAST.
fn last_writers(writes: &TipWrites) -> BTreeMap<Vec<u8>, usize> {
    let mut last = BTreeMap::new();
    for (i, (_, w)) in writes.iter().enumerate() {
        for (k, _) in &w.finals {
            last.insert(k.clone(), i);
        }
    }
    last
}

/// Each write's session is told the keys of ITS that the winner replaced.
/// Never for a key a LATER write of the same group overwrote: that write
/// lost to its own app's order, as it would have in sequence (K9 §2).
fn tell_superseded(out: &mut Outbound, winner: (u64, freenet_prolly::Cid), writes: &TipWrites, superseded: &std::collections::BTreeSet<Vec<u8>>) {
    let last = last_writers(writes);
    for (i, ((client, write_id), w)) in writes.iter().enumerate() {
        let keys: Vec<Vec<u8>> = w.finals.iter().map(|(k, _)| k.clone()).filter(|k| superseded.contains(k) && last.get(k) == Some(&i)).collect();
        let c = engine::ClientId(*client);
        if keys.is_empty() || version_of(c) < protocol::SESSION_SINCE || session_of(c) == protocol::LEGACY_SESSION {
            continue;
        }
        out.replies.push(reply_bytes(&protocol::Reply::Superseded { session: session_of(c), write_id: *write_id, seq: winner.0, root: winner.1, keys }));
    }
}

fn served() -> Vec<u16> {
    protocol::KNOWN.iter().copied().filter(|v| *v <= protocol::CURRENT).collect()
}

fn serve(bytes: &[u8]) -> Served {
    match protocol::decode_request(bytes) {
        Incoming::Ok(env) if env.version > protocol::CURRENT => Served::Answer(Reply::Unsupported {
            got: env.version,
            known: served(),
        }),
        Incoming::Ok(env) => Served::Do(env.body, env.version, env.session),
        Incoming::Unsupported(got) => Served::Answer(Reply::Unsupported { got, known: served() }),
        Incoming::Dropped(reason) => Served::Answer(Reply::Dropped { reason }),
    }
}

/// shell.rs 180–296, verbatim.
fn state_tag(s: State) -> u64 {
    match s {
        State::Accepted => 0,
        State::Stalled => 1,
        State::Published => 2,
        State::ParityComplete => 3,
        State::Busy => 4,
        State::Failed { .. } => 5,
        State::Lost => 6,
        State::TooLarge { .. } => 7,
        // Never reached on this path: the delegate's writes carry no reads
        // (`reads: Vec::new()` below), so nothing can conflict. Tagged as a
        // failure, which is what it would mean to this client.
        State::Conflict => 5,
        // Refused at the door (sdk#235): a failure to this client, like
        // `Conflict`.
        State::Unread => 5,
        State::QueueFull { .. } => 5,
        State::Unknown => 1,
    }
}

pub(crate) fn reply_bytes(r: &protocol::Reply) -> Vec<u8> {
    protocol::encode_reply(r).unwrap_or_else(|reason| {
        protocol::encode_reply(&protocol::Reply::Dropped { reason }).expect(
            "a Dropped reply is a handful of bytes; `a_dropped_reply_always_encodes` pins it",
        )
    })
}

fn rows_in(r: &engine::read::ReadResult) -> u64 {
    use engine::read::ReadResult as R;
    match r {
        R::Value(v) => v.is_some() as u64,
        R::Page { entries, .. } => entries.len() as u64,
        R::Delta { changes, .. } => changes.len() as u64,
        R::FullReloadRequired { .. } | R::Unavailable(_) | R::OutOfWarmSpace => 0,
    }
}

fn as_client(session: u64, version: u16) -> engine::ClientId {
    let s = session & ((1u64 << protocol::SESSION_BITS) - 1);
    engine::ClientId((s << 16) | version as u64)
}

fn version_of(c: engine::ClientId) -> u16 {
    (c.0 & 0xFFFF) as u16
}

fn session_of(c: engine::ClientId) -> u64 {
    c.0 >> 16
}

fn as_write_id(n: u64) -> engine::WriteId {
    engine::WriteId(n)
}

fn as_req_id(n: u64) -> engine::read::ReqId {
    engine::read::ReqId(n)
}

fn as_epoch(n: u32) -> engine::Epoch {
    engine::Epoch(n)
}


/// Where a key stands (R-b; READ-STATE § The pull API).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyState {
    /// This page's write to it is not published yet.
    Saving,
    /// Published.
    Saved,
    /// Published, and the commit of the key's last write is BACKED_UP (`ParityComplete`), the tree still holding
    /// its value (sdk#415).
    SavedAndBackedUp,
}

impl KeyState {
    pub fn code(self) -> &'static str {
        match self {
            KeyState::Saving => "SAVING",
            KeyState::Saved => "SAVED",
            KeyState::SavedAndBackedUp => "SAVED_AND_BACKED_UP",
        }
    }
}

fn stage_fate(s: engine::Stage) -> crate::fates::Fate {
    match s {
        engine::Stage::Applying => crate::fates::Fate::Applying,
        engine::Stage::Queued => crate::fates::Fate::Queued,
        engine::Stage::Committing => crate::fates::Fate::Committing,
    }
}
