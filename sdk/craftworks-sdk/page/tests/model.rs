//! THE MODEL TEST for the engine library in the page (sdk#213).
//!
//! Two pages on ONE key (one signer) write through a SCRIPTED CLIENT API. On
//! the far side run the real rules, not models of them:
//! * the SIGNER: `signer::serve` over an in-memory host whose synchronous
//!   read sees exactly what the scripted node holds;
//! * the head REGISTER itself: every UPDATE goes through the contract's own
//!   `update_state` (craftec-register-contract, natively) — the lower
//!   BLAKE3(value) at an equal seq, F56's sticky fork evidence, and a losing
//!   UPDATE still answered as a success.
//!
//! Faults, each on its OWN random stream (a new fault never shifts an old
//! one's draws): a PUT lost before it lands, a PUT's answer lost after it
//! landed, a transient PUT refusal (F51's queue), a GET answer lost, a sign
//! request lost, the signer failing to save its record (`RecordNotSaved`),
//! an UPDATE lost, an UPDATE answered and NOT applied, a head read lost, and
//! every answer's delay.
//!
//! The app is an outbox: a write the engine reports `Lost` (a rebase) or
//! `Busy` (one commit at a time) is submitted again, as WritePath does.
//!
//! INVARIANTS, checked on every step:
//! 1. `Published` only while the node's register holds the page's (seq, root).
//! 2. Every UPDATE carries bytes the signer returned.
//! 3. A published root is RECOVERABLE on the node: a reader that repairs
//!    (#300) reads every block it reaches, because race put signs a head
//!    when every changed group has k of its k+m (COMMIT-LIFE §P: SAVED).
//!    And (3b) at a write's `ParityComplete` (BACKED_UP), the root it was
//!    published at is WHOLE on the node: every block held, no repair needed.
//! 4. Once the faults stop, every write is published, and the final tree
//!    holds every key both pages wrote.

use engine::{ClientId, Op as WriteOp, Params, State, WriteId};
use freenet_prolly::store::Blocks;
use freenet_prolly::Cid;
use page::{Answer, Ms, Op, Page, PutPath};
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use testkit::model;

const BLOCK_CODE: &[u8] = b"model block code";
const REGISTER_CODE: &[u8] = b"model register code";

/// A tiny xorshift, one per stream.
#[derive(Clone)]
struct Rng(u64);
impl Rng {
    fn new(seed: u64, stream: u64) -> Rng {
        Rng((seed ^ stream.wrapping_mul(0x9E37_79B9_7F4A_7C15)).max(1))
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn chance(&mut self, per_mille: u64) -> bool {
        self.next() % 1000 < per_mille
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Fault rates, per mille. Zero after the write phase.
#[derive(Clone, Copy)]
struct Faults {
    put_lost: u64,
    put_answer_lost: u64,
    put_refused: u64,
    get_lost: u64,
    sign_lost: u64,
    update_lost: u64,
    update_not_applied: u64,
    head_lost: u64,
    record_not_saved: u64,
    /// The node's `HeadChanged` push to a page is dropped (a full
    /// notification channel, an evicted subscription).
    hint_lost: u64,
}

const FAULTS: Faults = Faults {
    put_lost: 60,
    put_answer_lost: 60,
    put_refused: 60,
    get_lost: 60,
    sign_lost: 80,
    update_lost: 80,
    update_not_applied: 80,
    head_lost: 80,
    record_not_saved: 60,
    hint_lost: 300,
};
const CALM: Faults = Faults {
    put_lost: 0,
    put_answer_lost: 0,
    put_refused: 0,
    get_lost: 0,
    sign_lost: 0,
    update_lost: 0,
    update_not_applied: 0,
    head_lost: 0,
    record_not_saved: 0,
    hint_lost: 0,
};

/// The block's kind, recovered from its id (the network holds `kind ‖ body`).
fn state_of(id: &Cid, body: &[u8]) -> Vec<u8> {
    let kind = [freenet_prolly::kind::TREE_NODE, freenet_prolly::kind::RAW, freenet_prolly::kind::PARITY]
        .into_iter()
        .find(|k| freenet_prolly::block_id(*k, body) == *id)
        .expect("every block hashes under one kind");
    let mut s = vec![kind];
    s.extend_from_slice(body);
    s
}

/// The scripted node: what it holds, and the signer's secrets.
struct Node {
    blocks: BTreeMap<Cid, Vec<u8>>,
    /// Block contract id → block id, for the signer's sync read.
    contracts: BTreeMap<[u8; 32], Cid>,
    register: Option<Vec<u8>>,
    register_id: [u8; 32],
    register_params: Vec<u8>,
    /// Each DEVICE's signer secrets (the key and its record): one device is
    /// two tabs on one signer; two devices are two signers with one key on one
    /// register — the user's own devices (sdk#225).
    secrets: Vec<BTreeMap<Vec<u8>, Vec<u8>>>,
    /// Which device's signer the current request is served by.
    dev: usize,
    /// The signer's record write fails while this is set (a fault).
    record_fails: bool,
    /// Every head the register has ever held: what "read back" can mean.
    held_heads: std::collections::BTreeSet<(u64, Cid)>,
    /// The VALUE the register held for each of those heads (root ‖ ledger).
    held_values: BTreeMap<(u64, Cid), Vec<u8>>,
}

impl Blocks for Node {
    fn get(&self, cid: &Cid) -> Option<&[u8]> {
        self.blocks.get(cid).map(Vec::as_slice)
    }
}

struct Host<'a>(&'a mut Node);
impl signer::Host for Host<'_> {
    fn get_secret(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.0.secrets[self.0.dev].get(key).cloned()
    }
    fn set_secret(&mut self, key: &[u8], value: &[u8]) -> bool {
        if self.0.record_fails && key == signer::RECORD {
            return false;
        }
        let d = self.0.dev;
        self.0.secrets[d].insert(key.to_vec(), value.to_vec());
        true
    }
    fn contract_state(&self, id: &[u8; 32]) -> Option<Vec<u8>> {
        if *id == self.0.register_id {
            return self.0.register.clone();
        }
        let cid = self.0.contracts.get(id)?;
        self.0.blocks.get(cid).map(|b| state_of(cid, b))
    }
}


fn seq_of(node: &Node) -> u64 {
    node.head().map_or(0, |h| h.0)
}

fn head_of(state: &[u8]) -> (u64, Cid) {
    let (seq, v) = signer_proto::head::record_of(state).expect("a register record");
    (seq, v[..32].try_into().expect("32"))
}

impl Node {
    fn new() -> (Node, Vec<u8>) {
        Node::with_devices(1)
    }

    fn with_devices(devices: usize) -> (Node, Vec<u8>) {
        let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let vk = sk.verifying_key().to_bytes();
        let params = wire::register_params(&vk, wire::HEAD_NAME);
        let mut n = Node {
            blocks: BTreeMap::new(),
            contracts: BTreeMap::new(),
            register: None,
            register_id: signer::register_id(REGISTER_CODE, &params),
            register_params: params.clone(),
            secrets: vec![BTreeMap::new(); devices],
            dev: 0,
            record_fails: false,
            held_heads: Default::default(),
            held_values: BTreeMap::new(),
        };
        let req = signer::Request::Provision {
            signing_key: sk.to_bytes().to_vec(),
            register_code: REGISTER_CODE.to_vec(),
            register_params: params.clone(),
            block_code: BLOCK_CODE.to_vec(),
        };
        for d in 0..devices {
            n.dev = d;
            assert_eq!(signer::serve(&mut Host(&mut n), &signer::encode_request(1, &req), signer::Origin::Local), signer::Answer::Provisioned);
        }
        n.dev = 0;
        (n, params)
    }

    fn put(&mut self, id: Cid, body: &[u8]) {
        self.contracts.insert(contract_keys::block::contract_for(BLOCK_CODE, &id), id);
        self.blocks.insert(id, body.to_vec());
    }

    /// An UPDATE, through the Register contract's OWN `update_state`: the
    /// state the node keeps is exactly the contract's answer (F56: a loser is
    /// still told success; that is the caller's `Updated`).
    fn update(&mut self, state: &[u8]) {
        use freenet_stdlib::prelude::*;
        let params = Parameters::from(self.register_params.clone());
        let next = match &self.register {
            None => {
                let v = <craftec_register_contract::Register as ContractInterface>::validate_state(
                    params,
                    State::from(state.to_vec()),
                    RelatedContracts::default(),
                );
                assert!(matches!(v, Ok(ValidateResult::Valid)), "the signer's record is not a valid register state");
                state.to_vec()
            }
            Some(cur) => {
                let m = <craftec_register_contract::Register as ContractInterface>::update_state(
                    params,
                    State::from(cur.clone()),
                    vec![UpdateData::State(State::from(state.to_vec()))],
                )
                .expect("the register merged");
                m.new_state.expect("the register answered a state").as_ref().to_vec()
            }
        };
        self.register = Some(next);
        if let Some(h) = self.head() {
            self.held_heads.insert(h);
            if let Some(r) = self.head_read() {
                self.held_values.insert(h, r.value().to_vec());
            }
        }
    }

    fn head(&self) -> Option<(u64, Cid)> {
        self.register.as_deref().map(head_of)
    }

    /// The head WHOLE, as page-io reads it off the node: seq and value.
    fn head_read(&self) -> Option<page::HeadRead> {
        page::HeadRead::from_record(self.register.as_deref()?)
    }

    /// The REAL signer's answer, exactly as it encodes it and the page's
    /// `wire::signer::read_answer` decodes it.
    fn sign(&mut self, id: u32, prev_seq: u64, prev_root: Cid, seq: u64, root: Cid, ledger: Vec<u8>) -> (u32, signer_proto::Answer) {
        let req = signer::Request::Sign {
            prev: signer::Head { seq: prev_seq, root: prev_root },
            next: signer::Next { seq, root, ledger },
            label: signer::Label::Head,
        };
        // Through the BYTES both ways: the request under the page's id, the answer under the id the signer echoes.
        let served = signer::serve_full(&mut Host(self), &signer::encode_request(id, &req), signer::Origin::Local);
        wire::signer::read_answer(&signer::reply(&served)).expect("a signer answer reads back")
    }

    /// Every entry of the tree at `root`, or `None` if any block is missing.
    fn tree(&self, root: &Cid) -> Option<BTreeMap<Vec<u8>, Vec<u8>>> {
        use freenet_prolly::range::{range, read_value, Range};
        use std::ops::Bound;
        let mut out = BTreeMap::new();
        let mut after = None;
        loop {
            let r = Range {
                lo: Bound::Unbounded,
                hi: Bound::Unbounded,
                reverse: false,
                after: after.clone(),
                max_entries: 4096,
                max_bytes: usize::MAX,
            };
            let page = range(self, root, &r).ok()?;
            if !page.need.is_empty() {
                return None;
            }
            for (k, v) in &page.entries {
                out.insert(k.clone(), read_value(self, *v).ok()?.to_vec());
            }
            if page.finished() {
                return Some(out);
            }
            after = page.next.clone();
        }
    }
}

impl Node {
    /// Every node of `b` that `a` does not have (a paged diff, unioned), or why it could not be completed.
    fn nodes_not_in(&self, a: &Cid, b: &Cid) -> Result<std::collections::BTreeSet<Cid>, String> {
        use freenet_prolly::range::Range;
        use std::ops::Bound;
        let r = Range { lo: Bound::Unbounded, hi: Bound::Unbounded, reverse: false, after: None, max_entries: usize::MAX, max_bytes: usize::MAX };
        let mut out = std::collections::BTreeSet::new();
        let mut resume = None;
        loop {
            let page = freenet_prolly::diff::diff(self, a, b, &r, resume.as_ref()).map_err(|e| format!("diff: {e:?}"))?;
            if let Some(n) = page.need.first() {
                return Err(format!("block {:?} of the commit's path is not on the node", &n[..4]));
            }
            out.extend(page.new_blocks.iter().copied());
            match page.next {
                Some(n) => resume = Some(n),
                None => return Ok(out),
            }
        }
    }

    /// INVARIANT 3b as the architect ruled it (rule 10, BACKED_UP is a WRITE state): every group the commit that
    /// moved `prev` to `root` CHANGED is whole on the node -- each member and each parity block -- and so is the
    /// root's group of one (the root and the parity its head's mark lists). A group of a new node whose parity
    /// ids are all the replaced nodes' own is unchanged (parity ids are a function of the members). Whether the
    /// whole TREE is whole is the assets dashboard's question (KEEPER §4), not a write's.
    fn changed_groups_whole(&self, prev: (u64, Cid), root: &Cid, mark: Option<Vec<Cid>>) -> Result<(), Unwhole> {
        let missing = |b: &Cid| !self.blocks.contains_key(b);
        let unplaced = |why: String| Unwhole { hole: Hole::Unplaced, why };
        if missing(root) {
            return Err(Unwhole { hole: Hole::Root, why: "the root is not on the node".into() });
        }
        for p in mark.unwrap_or_default() {
            if missing(&p) {
                return Err(Unwhole { hole: Hole::Root, why: format!("root parity {:?} is not on the node", &p[..4]) });
            }
        }
        let new_nodes = if prev.0 == 0 { self.all_nodes(root).map_err(unplaced)? } else { self.nodes_not_in(&prev.1, root).map_err(unplaced)? };
        let old_parity = if prev.0 == 0 { Default::default() } else { self.parity_of(&self.nodes_not_in(root, &prev.1).map_err(unplaced)?) };
        for n in &new_nodes {
            let b = self.blocks.get(n).ok_or_else(|| unplaced(format!("new node {:?} is not on the node", &n[..4])))?;
            let node = freenet_prolly::node::Node::parse(b).map_err(|e| unplaced(format!("a node that does not parse: {e:?}")))?;
            let ids: Vec<Cid> = node.parity().collect();
            for (g, (_, members)) in freenet_prolly::parity::group_members(&node).into_iter().enumerate() {
                let par = ids.get(engine::PARITY * g..engine::PARITY * (g + 1)).unwrap_or(&[]);
                if !par.is_empty() && par.iter().all(|p| old_parity.contains(p)) {
                    continue;
                }
                if let Some(m) = members.iter().chain(par).find(|m| missing(m)) {
                    let why = format!("block {:?} of a group the commit changed (node {:?}, group {g}) is not on the node", &m[..4], &n[..4]);
                    let hole = if par.is_empty() { Hole::Unplaced } else { Hole::Group(par.iter().copied().collect()) };
                    return Err(Unwhole { hole, why });
                }
            }
        }
        Ok(())
    }

    /// The parity ids the given nodes list (those held).
    fn parity_of(&self, nodes: &std::collections::BTreeSet<Cid>) -> std::collections::BTreeSet<Cid> {
        nodes.iter().filter_map(|n| self.blocks.get(n)).filter_map(|b| freenet_prolly::node::Node::parse(b).ok()).flat_map(|n| n.parity().collect::<Vec<_>>()).collect()
    }

    /// Did the commit `prev` → `root` RE-CODE the group whose parity ids are `par`: a node it replaced listed that
    /// parity, and none of its new nodes lists it any more?
    fn recoded(&self, prev: (u64, Cid), root: &Cid, par: &std::collections::BTreeSet<Cid>) -> bool {
        if prev.0 == 0 {
            return false;
        }
        let (Ok(old), Ok(new)) = (self.nodes_not_in(root, &prev.1), self.nodes_not_in(&prev.1, root)) else { return false };
        par.is_subset(&self.parity_of(&old)) && par.is_disjoint(&self.parity_of(&new))
    }

    /// Every node of the tree at `root` (the first commit changed all of them).
    fn all_nodes(&self, root: &Cid) -> Result<std::collections::BTreeSet<Cid>, String> {
        let mut out = std::collections::BTreeSet::new();
        let mut at = vec![*root];
        while let Some(id) = at.pop() {
            let b = self.blocks.get(&id).ok_or_else(|| format!("node {:?} is not on the node", &id[..4]))?;
            let node = freenet_prolly::node::Node::parse(b).map_err(|e| format!("{e:?}"))?;
            out.insert(id);
            if node.level() > 0 {
                at.extend((0..node.len()).map(|i| node.child(i).0));
            }
        }
        Ok(out)
    }
}

/// The node's blocks, plus blocks REBUILT from their sibling groups (#300):
/// what a reader that repairs can read. Race put signs a head when every
/// changed group is RECOVERABLE (k of k+m), not when every block is held
/// (COMMIT-LIFE §P), so a published tree may be readable only this way until
/// its stragglers land.
struct Repairing<'a> {
    node: &'a Node,
    rebuilt: BTreeMap<Cid, Vec<u8>>,
}

impl Blocks for Repairing<'_> {
    fn get(&self, cid: &Cid) -> Option<&[u8]> {
        self.node.blocks.get(cid).or_else(|| self.rebuilt.get(cid)).map(Vec::as_slice)
    }
}

impl Node {
    /// Every entry of the tree at `root` as a REPAIRING reader reads it, or
    /// why not: the first block no group could rebuild.
    fn tree_repairing(&self, root: &Cid) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, String> {
        use freenet_prolly::range::{range, Range};
        use freenet_prolly::node::Value;
        use std::ops::Bound;
        let mut r = Repairing { node: self, rebuilt: BTreeMap::new() };
        for _ in 0..10_000 {
            let mut missing: Vec<Cid> = Vec::new();
            let mut out = BTreeMap::new();
            let mut after = None;
            loop {
                let q = Range { lo: Bound::Unbounded, hi: Bound::Unbounded, reverse: false, after: after.clone(), max_entries: 4096, max_bytes: usize::MAX };
                let page = range(&r, root, &q).map_err(|e| format!("range: {e:?}"))?;
                if !page.need.is_empty() {
                    missing.extend(page.need.iter().copied());
                    break;
                }
                for (k, v) in &page.entries {
                    match v {
                        Value::Ref { cid, .. } if r.get(cid).is_none() => missing.push(*cid),
                        _ => {
                            if let Ok(b) = freenet_prolly::range::read_value(&r, *v) {
                                out.insert(k.clone(), b.to_vec());
                            }
                        }
                    }
                }
                if page.finished() {
                    break;
                }
                after = page.next.clone();
            }
            if missing.is_empty() {
                return Ok(out);
            }
            for id in missing {
                let g = engine::repair::find_group(&r, *root, id).ok_or_else(|| format!("block {:?} is in no held group", &id[..4]))?;
                let have: Vec<Option<Vec<u8>>> = g.slots.iter().enumerate().map(|(i, s)| r.get(s).filter(|b| g.fits(i, b)).map(|b| g.stored(i, b))).collect();
                let held = have.iter().filter(|h| h.is_some()).count();
                let body = engine::repair::rebuild(&g, &have).map_err(|e| format!("block {:?} not rebuildable: {held} of k={} held: {e}", &id[..4], g.k))?;
                r.rebuilt.insert(id, body);
            }
        }
        Err("the repairing walk did not finish".into())
    }
}

/// One op in flight: it takes effect when DELIVERED.
struct Flight {
    at: u64,
    page: usize,
    op: Op,
}

/// A page and its app: an outbox of (key, value) writes not yet published.
struct App {
    page: Page,
    client: ClientId,
    next_id: u64,
    /// write id → (key, value), in flight in the engine.
    inflight: BTreeMap<u64, (Vec<u8>, Vec<u8>)>,
    /// When each write was submitted (page ms): `Stalled` is judged by it.
    submitted: BTreeMap<u64, u64>,
    /// Writes to (re-)submit.
    todo: Vec<(Vec<u8>, Vec<u8>)>,
    /// Every key this app wrote and its LAST value.
    wrote: BTreeMap<Vec<u8>, Vec<u8>>,
    published: usize,
    /// Writes that CHANGED NOTHING (their value already in the tree): told Published in the same step as Accepted,
    /// or ParityComplete in the same step as Published -- a committing write can do neither (its blocks are acked,
    /// and its parity follows the head, in later steps). They changed no group, so 3b judges nothing of theirs.
    nothing_changed: std::collections::BTreeSet<u64>,
}

impl App {
    fn submit(&mut self, now: u64) {
        if let Some((k, v)) = self.todo.first().cloned() {
            self.todo.remove(0);
            self.next_id += 1;
            self.inflight.insert(self.next_id, (k.clone(), v.clone()));
            self.submitted.insert(self.next_id, now);
            self.page.write(self.client, WriteId(self.next_id), vec![(k, WriteOp::Put(v))]);
        }
    }
}

/// (page, write, the head it was told Published at, its key and value).
type PublishedAt = (usize, u64, (u64, Cid), Option<(Vec<u8>, Vec<u8>)>);

#[derive(Default, Debug)]
struct Seen {
    /// Every op every page sent, in order, with when and which page: the run's op sequence, hashed. A recorder that
    /// became an input would change it.
    ops_digest: u64,
    /// Ops each page sent.
    ops_sent: [usize; 2],
    published: usize,
    lost: usize,
    busy: usize,
    updates: usize,
    record_not_saved: usize,
    landings: u32,
    most_landing_updates: u32,
    /// (page, write, the head it was told Published at).
    published_at: Vec<PublishedAt>,
    /// Published writes whose head a same-seq WINNER later displaced, and
    /// whose value the final tree does not hold: the per-key merge (#225b)
    /// is what keeps them. Counted and named here, never silent.
    displaced: usize,
    /// Seqs the register held under two roots: the devices really raced.
    races: usize,
    /// Sends of the forced straggler dropped (`Cfg::hold_page0_first_value`).
    held_drops: usize,
    /// Writes of page 1 told Published while the forced straggler was off the node, at a root not whole.
    over_the_hole: usize,
}

/// What a run plays.
#[derive(Clone, Copy)]
struct Cfg {
    faults: Faults,
    /// 1: the two pages are two TABS on one signer. 2: two DEVICES of one
    /// identity — two signers, one key, one register (sdk#225): they race at
    /// every seq, and the register's tie-break decides.
    devices: usize,
    /// Every `HeadChanged` push is dropped, calm or not: an idle page learns
    /// only by the backstop read.
    no_hints: bool,
    /// THE FORCED STRAGGLER (safety gap class 2): the VALUE block page 0 puts for its FIRST write (`p0/000`) is
    /// lost on every send until the calm, so that commit publishes at k (race put) with its own block off the
    /// node. Page 1 starts only once page 0 has published every write, and OVERWRITES page 0's other keys: a value
    /// replaced in the straggler's group is a ONE-OFF change, its parity updated from the old parity (prolly's
    /// `update_group`) without reading the straggler -- so page 1 commits a changed group holding a foreign block
    /// that is not on the node.
    hold_page0_first_value: bool,
    /// Each page RECORDS its ops (sdk#386's instrument work), and the run checks the recording accounts for every op.
    record: bool,
}

const NORMAL: Cfg = Cfg { faults: FAULTS, devices: 1, no_hints: false, hold_page0_first_value: false, record: false };

/// Does `later` descend from `h` through the signer's records (next → prev)?
fn descends(edges: &BTreeMap<(u64, Cid), (u64, Cid)>, later: (u64, Cid), h: (u64, Cid)) -> bool {
    let mut cur = later;
    loop {
        if cur == h {
            return true;
        }
        if cur.0 <= h.0 {
            return false;
        }
        match edges.get(&cur) {
            Some(prev) => cur = *prev,
            None => return false,
        }
    }
}

fn run(seed: u64, writes_per_page: usize, path: PutPath) -> Result<Seen, String> {
    run_with(seed, writes_per_page, path, NORMAL)
}

fn run_with(seed: u64, writes_per_page: usize, path: PutPath, cfg: Cfg) -> Result<Seen, String> {
    let mut edges: BTreeMap<(u64, Cid), (u64, Cid)> = BTreeMap::new();
    let (mut node, _) = Node::with_devices(cfg.devices);
    let dev_of = |page: usize| if cfg.devices == 1 { 0 } else { page };
    let mut s_delay = Rng::new(seed, 1);
    let mut s_put_lost = Rng::new(seed, 2);
    let mut s_put_ans = Rng::new(seed, 3);
    let mut s_put_ref = Rng::new(seed, 4);
    let mut s_get = Rng::new(seed, 5);
    let mut s_sign = Rng::new(seed, 6);
    let mut s_upd = Rng::new(seed, 7);
    let mut s_upd_na = Rng::new(seed, 8);
    let mut s_head = Rng::new(seed, 9);
    let mut s_app = Rng::new(seed, 10);
    let mut s_rec = Rng::new(seed, 11);
    let mut s_hint = Rng::new(seed, 12);
    // THE CLOCK'S ORIGIN, varied per seed (sdk#397): a browser page's clock is Date.now(), EPOCH ms, and a model
    // whose pages always started at 0 could not see a request dated against another origin -- the defect that
    // pinned the RTO at its ceiling live. The model's own `now` counts from 0; every page sees `origin + now`.
    let origin: u64 = if seed.is_multiple_of(2) { 0 } else { 1_790_253_181_367 };

    let mut apps: Vec<App> = (0..2)
        .map(|i| App {
            page: {
                // Recording from before the first op (the architect's check 2 counts every one).
                let mut p = Page::unstarted(model_params(), path, Ms(origin));
                if cfg.record {
                    p.record_into(1 << 20);
                }
                p.start();
                p
            },
            client: ClientId(i as u64 + 1),
            next_id: 0,
            inflight: BTreeMap::new(),
            submitted: BTreeMap::new(),
            todo: (0..writes_per_page)
                .map(|n| {
                    // 3 KB values: a tree with blocks BELOW its root, so a
                    // root can be signed while a child is missing — the
                    // signer's own check guards the root block only.
                    let mut v = format!("v{seed}-{i}-{n}:").into_bytes();
                    v.resize(3_000, b'a' + (n % 26) as u8);
                    (format!("p{i}/{n:03}").into_bytes(), v)
                })
                .collect(),
            wrote: BTreeMap::new(),
            published: 0,
            nothing_changed: Default::default(),
        })
        .collect();
    if cfg.hold_page0_first_value {
        // Page 1 overwrites page 0's keys but the first (the straggler's); its values, written after, are the
        // final ones.
        for (n, w) in apps[1].todo.iter_mut().enumerate() {
            w.0 = format!("p0/{:03}", n + 1).into_bytes();
        }
    }
    for a in &mut apps {
        let todo = a.todo.clone();
        for (k, v) in todo {
            a.wrote.insert(k, v);
        }
    }
    if cfg.hold_page0_first_value {
        let over: Vec<Vec<u8>> = apps[1].wrote.keys().cloned().collect();
        for k in over {
            apps[0].wrote.remove(&k);
        }
    }

    let mut flights: Vec<Flight> = Vec::new();
    let mut seen = Seen::default();
    let mut held: Option<Cid> = None;
    let mut page1_read = false;
    let mut digest = std::collections::hash_map::DefaultHasher::new();
    let mut now = 0u64;
    let calm_at = 400_000u64;
    let end = calm_at + 600_000;
    while now < end {
        let faults = if now < calm_at { cfg.faults } else { CALM };
        // The apps submit.
        let page0_done = apps[0].todo.is_empty() && apps[0].inflight.is_empty();
        if cfg.hold_page0_first_value && page0_done && !page1_read {
            // Page 1 READS the straggler's key first: its value is NotFound on the node, and the read path
            // rebuilds it from its group (race put published it at k) into page 1's memory -- so page 1's writes
            // apply over it without the node ever holding it.
            page1_read = true;
            let client = apps[1].client;
            apps[1].page.event(engine::Event::Get { client, req_id: engine::read::ReqId(1), key: b"p0/000".to_vec() });
        }
        for (i, a) in apps.iter_mut().enumerate() {
            if cfg.hold_page0_first_value && i == 1 && !page0_done {
                continue;
            }
            if a.inflight.is_empty() && !a.todo.is_empty() && s_app.chance(300) {
                a.submit(now);
            }
        }
        // Ops leave the pages.
        for (i, a) in apps.iter_mut().enumerate() {
            for op in a.page.take_ops() {
                (now, i, format!("{op:?}")).hash(&mut digest);
                seen.ops_sent[i] += 1;
                // INVARIANT 2.
                if let Op::Update { label: page::Label::Head, state } = &op {
                    if !a.page.signer_records().contains(state) {
                        return Err(format!("page {i} UPDATEd bytes the signer never returned"));
                    }
                }
                flights.push(Flight { at: now + 1 + s_delay.below(300), page: i, op });
            }
        }
        // Deliver what is due, in order.
        flights.sort_by_key(|f| f.at);
        let due: Vec<Flight> = {
            let n = flights.iter().take_while(|f| f.at <= now).count();
            flights.drain(..n).collect()
        };
        for f in due {
            let answer = match f.op {
                Op::Put { id, bytes } if held.is_none() && f.page == 0 && cfg.hold_page0_first_value && freenet_prolly::block_id(freenet_prolly::kind::RAW, &bytes) == id => {
                    held = Some(id);
                    seen.held_drops += 1;
                    None
                }
                Op::Put { id, .. } if held == Some(id) && now < calm_at => {
                    seen.held_drops += 1;
                    None
                }
                Op::Put { id, bytes } => {
                    if s_put_lost.chance(faults.put_lost) {
                        // On the wrapper path an answer is no evidence: a
                        // put that never landed is still answered ok, so an
                        // executor that trusted it would publish a hole.
                        (path == PutPath::Wrapper).then_some(Answer::PutOk(id))
                    } else if s_put_ref.chance(faults.put_refused) {
                        Some(Answer::PutRefused { id, transient: true })
                    } else {
                        node.put(id, &bytes);
                        // The wrapper path: the answer never reaches the page
                        // (sdk#214) — here it does, and must confirm nothing.
                        if s_put_ans.chance(faults.put_answer_lost) {
                            None
                        } else {
                            Some(Answer::PutOk(id))
                        }
                    }
                }
                Op::Get { id } => {
                    if s_get.chance(faults.get_lost) {
                        None
                    } else {
                        Some(match node.blocks.get(&id) {
                            Some(b) => Answer::Got { id, bytes: b.clone() },
                            None => Answer::GetMissed(id),
                        })
                    }
                }
                Op::Sign { id, prev_seq, prev_root, seq, root, ledger, .. } => {
                    if s_sign.chance(faults.sign_lost) {
                        None
                    } else {
                        node.record_fails = s_rec.chance(faults.record_not_saved);
                        node.dev = dev_of(f.page);
                        let (id, a) = node.sign(id, prev_seq, prev_root, seq, root, ledger);
                        node.record_fails = false;
                        if let Some(rec) = node.secrets[node.dev].get(signer::RECORD) {
                            let rec: signer::Record = bincode::deserialize(rec).expect("the signer's record");
                            edges.insert((rec.next.seq, rec.next.root), (rec.prev.seq, rec.prev.root));
                        }
                        if matches!(a, signer_proto::Answer::Refused(signer_proto::Why::RecordNotSaved)) {
                            seen.record_not_saved += 1;
                        }
                        Some(Answer::Signer { id, answer: a })
                    }
                }
                Op::Update { state, .. } => {
                    seen.updates += 1;
                    if s_upd.chance(faults.update_lost) {
                        None
                    } else {
                        if !s_upd_na.chance(faults.update_not_applied) {
                            let before = node.head();
                            node.update(&state);
                            // THE SUBSCRIPTION: a head that moved is pushed to
                            // every OTHER page as `HeadChanged` — a hint, and
                            // a lossy one.
                            if node.head() != before {
                                for (j, other) in apps.iter_mut().enumerate() {
                                    if j != f.page && !cfg.no_hints && !s_hint.chance(faults.hint_lost) {
                                        other.page.head_hint();
                                    }
                                }
                            }
                        }
                        Some(Answer::Updated { label: page::Label::Head })
                    }
                }
                Op::AskHeld { batch, ids } => {
                    if s_head.chance(faults.head_lost) {
                        None
                    } else {
                        Some(Answer::Held { batch, present: ids.iter().map(|id| node.blocks.contains_key(id)).collect() })
                    }
                }
                // The engine never makes an app PUT; this model sends none.
                Op::PutApp { key } => Some(Answer::AppPutOk(key)),
                Op::Ext(_) => None,
                Op::ReadHead { .. } => {
                    if s_head.chance(faults.head_lost) {
                        None
                    } else {
                        Some(Answer::Head { label: page::Label::Head, read: node.head_read() })
                    }
                }
            };
            let Some(answer) = answer else { continue };
            apps[f.page].page.answer(answer, Ms(origin + now));
            check(&mut apps, f.page, &node, &mut seen, now, held, &edges)?;
        }
        now += 5;
        for i in 0..apps.len() {
            apps[i].page.tick(Ms(origin + now));
            check(&mut apps, i, &node, &mut seen, now, held, &edges)?;
        }
        let converged = apps.iter().all(|a| Some(a.page.published()) == node.head());
        // The forced straggler was held off the node for the whole write phase: its page re-sends it on a
        // backoff long by then, so the run also waits for it to land.
        let whole = !cfg.hold_page0_first_value || node.head().is_some_and(|h| node.tree(&h.1).is_some());
        if now > calm_at && apps.iter().all(|a| a.todo.is_empty() && a.inflight.is_empty()) && flights.is_empty() && converged && whole {
            break;
        }
    }
    for a in &apps {
        let (l, m) = a.page.landings();
        seen.landings += l;
        seen.most_landing_updates = seen.most_landing_updates.max(m);
    }
    // RACES: seqs the register held under two roots (non-vacuity for the
    // two-device runs).
    let mut per_seq: BTreeMap<u64, usize> = BTreeMap::new();
    for (sq, _) in &node.held_heads {
        *per_seq.entry(*sq).or_default() += 1;
    }
    seen.races = per_seq.values().filter(|n| **n > 1).count();
    // CONVERGED: every page ends on the register's head — an IDLE one too,
    // by the `HeadChanged` hint or, every hint lost, the backstop read.
    for (i, a) in apps.iter().enumerate() {
        if Some(a.page.published()) != node.head() {
            return Err(format!("page {i} ends on {:?}, the register on {:?}: it never learned", a.page.published().0, node.head().map(|h| h.0)));
        }
    }
    // INVARIANT 1, in full: a Published write's head H was read from the
    // register (checked as it happened), AND the final register head
    // descends from H through the signers' records (BOTH devices'), OR the
    // final chain passes through ANOTHER root at H's own seq — a sibling
    // device's head the register kept, by the equal-seq tie-break or by a
    // higher seq built on it (F56: the higher seq wins). The same identity
    // never forks (sdk#225): that is the register's rule, never a fork. Until
    // the per-key merge (#225b) those writes' values are gone, and they are
    // COUNTED (`displaced`), not silent.
    if let Some(fin) = node.head() {
        let tree = node.tree(&fin.1).unwrap_or_default();
        let mut chain: BTreeMap<u64, Cid> = BTreeMap::new();
        let mut c = fin;
        chain.insert(c.0, c.1);
        while let Some(p) = edges.get(&c) {
            c = *p;
            chain.insert(c.0, c.1);
        }
        for (i, wid, h, kv) in &seen.published_at {
            if descends(&edges, fin, *h) {
                continue;
            }
            let sibling = chain.get(&h.0).is_some_and(|x| *x != h.1 && node.held_heads.contains(&(h.0, *x)));
            if !sibling {
                return Err(format!(
                    "page {i}: write {wid} was Published at {:?}, which the final head {:?} does not descend from through any sibling at that seq",
                    h.0, fin.0
                ));
            }
            if kv.as_ref().is_some_and(|(k, v)| tree.get(k) != Some(v)) {
                seen.displaced += 1;
            }
        }
    }
    // INVARIANT 4.
    for (i, a) in apps.iter().enumerate() {
        if !a.todo.is_empty() || !a.inflight.is_empty() {
            return Err(format!(
                "page {i}: {} writes never published ({} still in the engine); unusable: {:?}",
                a.todo.len() + a.inflight.len(),
                a.inflight.len(),
                a.page.unusable()
            ));
        }
    }
    // Nothing the pages could not do: a refusal hidden in `unusable` would
    // otherwise pass as long as the retries got round it.
    for (i, a) in apps.iter().enumerate() {
        if !a.page.unusable().is_empty() {
            return Err(format!("page {i} recorded: {:?}", a.page.unusable()));
        }
    }
    let (_, root) = node.head().ok_or("no head at the end")?;
    let tree = node.tree(&root).ok_or("the final root is not whole on the node")?;
    for a in &apps {
        for (k, v) in &a.wrote {
            if tree.get(k) != Some(v) {
                // Only a write Published at a head a winner displaced may be
                // missing (counted above, #225b's to keep).
                let displaced = seen.published_at.iter().any(|(_, _, h, kv)| {
                    kv.as_ref().is_some_and(|(pk, pv)| pk == k && pv == v) && !descends(&edges, (seq_of(&node), root), *h)
                });
                if !displaced {
                    return Err(format!("the final tree lost {:?}", String::from_utf8_lossy(k)));
                }
            }
        }
    }
    // Ops the LAST tick sent (the loop ends after a tick, before the next take): sent all the same, and recorded, so
    // counted and hashed like every other. A run that converged on a tick that also re-sent something left them
    // uncounted, and the recording then held more Request edges than "ops sent" (sdk#447's re-derived timing).
    for (i, a) in apps.iter_mut().enumerate() {
        for op in a.page.take_ops() {
            (now, i, format!("{op:?}")).hash(&mut digest);
            seen.ops_sent[i] += 1;
        }
    }
    seen.ops_digest = digest.finish();
    if cfg.record {
        for (i, a) in apps.iter().enumerate() {
            recording_accounts_for_every_op(&a.page, seen.ops_sent[i]).map_err(|e| format!("page {i}'s recording: {e}"))?;
        }
    }
    Ok(seen)
}

/// Where a commit's changed groups are not whole.
enum Hole {
    /// The root, or the parity its head's mark lists (the root's group of one).
    Root,
    /// A group, named by its parity ids.
    Group(std::collections::BTreeSet<Cid>),
    /// Somewhere the check could not place in a group (a node of the commit's path missing).
    Unplaced,
}

struct Unwhole {
    hole: Hole,
    why: String,
}

/// Which later commit may CARRY a write whose own changed groups are not whole.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Carrier {
    /// A later commit of the page that DESCENDS from the write's head through a step that RE-CODED the hole's
    /// group (for the root's group: any step -- every commit replaces the root), with every group IT changed
    /// whole: what `supersede`'s carry is. The re-coding step may be another page's head adopted in between: that
    /// is the root move that withdrew the stragglers, and the next own commit is the carrier.
    Recoded,
    /// ANY later whole commit of the page: too loose (the architect), kept only as the control's other side.
    Any,
}

/// INVARIANT 3b for one write told ParityComplete: its commit (`own`) has every group it changed whole on the
/// node, or a later commit of the same page (`later`) CARRIED it (`how`).
fn judge_backed_up(node: &Node, edges: &BTreeMap<(u64, Cid), (u64, Cid)>, own: (u64, Cid), later: &[(u64, Cid)], how: Carrier) -> Result<(), String> {
    let mark = |h: (u64, Cid)| node.held_values.get(&h).and_then(|v| page::HeadRead::from_value(h.0, v)).and_then(|r| r.mark());
    let Some(prev) = edges.get(&own) else { return Ok(()) };
    let Err(u) = node.changed_groups_whole(*prev, &own.1, mark(own)) else { return Ok(()) };
    // Did some step of the chain own -> ... -> h (the signers' records) re-code the hole's group?
    let recoded_on_the_way = |h: (u64, Cid)| -> bool {
        let mut cur = h;
        let mut recoded = false;
        while cur != own {
            let Some(p) = edges.get(&cur) else { return false };
            if cur.0 <= own.0 {
                return false; // not a descendant of the write's head
            }
            recoded |= match &u.hole {
                Hole::Root => true,
                Hole::Group(par) => node.recoded(*p, &cur.1, par),
                Hole::Unplaced => false,
            };
            cur = *p;
        }
        recoded
    };
    let carried = later.iter().filter(|h| h.0 > own.0).any(|h| {
        let Some(hp) = edges.get(h) else { return false };
        let replaced = match how {
            Carrier::Any => true,
            Carrier::Recoded => recoded_on_the_way(*h),
        };
        replaced && node.changed_groups_whole(*hp, &h.1, mark(*h)).is_ok()
    });
    if carried {
        Ok(())
    } else {
        Err(format!("a group its commit changed is not WHOLE on the node, and no later commit of this page re-coded it whole: {}", u.why))
    }
}

/// THE RECORDING ACCOUNTS FOR EVERY OP (the architect's check 2): one Request edge per op the page sent, and every
/// Request is answered (a Response), closed (an Exit: timed out or withdrawn), or still on the wire at the end --
/// exactly as many as the page holds on the wire. Nothing dropped, and no answer for a request never made.
fn recording_accounts_for_every_op(p: &Page, sent: usize) -> Result<(), String> {
    use instrument::{Dir, Event, Record};
    let r = p.recording().ok_or("no recording attached")?;
    if r.dropped() > 0 {
        return Err(format!("{} events dropped: the ring is too small for the run", r.dropped()));
    }
    let (mut requests, mut responses, mut exits) = (Vec::new(), std::collections::BTreeSet::new(), std::collections::BTreeSet::new());
    for e in r.events() {
        match e {
            Event::Edge { dir: Dir::Request, id, .. } => requests.push(id.ordinal()),
            Event::Edge { dir: Dir::Response, id, .. } => {
                responses.insert(id.ordinal());
            }
            Event::Exit { op, .. } => {
                // The op decoded back to its label (instrument v2 carries the kind): only a SEND's end counts here.
                if let Some(l) = instrument::Label::of_op(op).filter(|l| l.kind() == instrument::Kind::Request) {
                    exits.insert(l.ordinal());
                }
            }
            _ => {}
        }
    }
    if requests.len() != sent {
        return Err(format!("{} ops sent, {} Request edges", sent, requests.len()));
    }
    if let Some(f) = responses.iter().find(|o| !requests.contains(o)) {
        return Err(format!("a Response for req#{f}, never requested"));
    }
    let open = requests.iter().filter(|o| !responses.contains(o) && !exits.contains(o)).count();
    if open != p.ops_on_wire() {
        return Err(format!("{open} Requests neither answered nor closed, but {} ops on the wire", p.ops_on_wire()));
    }
    Ok(())
}

/// THE PROBE IS NEVER AN INPUT (the architect's check 4): the same seeds, recording OFF and ON, send the same ops at
/// the same times -- the op sequences hash equal -- and the ON run's recording accounts for every op. Both clock
/// origins (odd and even seeds). Mutant "a branch reads the recorder" -> red.
#[test]
fn a_recording_page_sends_exactly_what_a_silent_one_does_and_records_every_op() {
    let (min, _) = seed_range(4, FULL_SEEDS);
    for (seed, (off, on)) in model::run(1..min.max(2) + 1, |seed| {
        let off = run_with(seed, WRITES, PutPath::Page, NORMAL).unwrap_or_else(|e| panic!("seed {seed}, off: {e}"));
        let on = run_with(seed, WRITES, PutPath::Page, Cfg { record: true, ..NORMAL }).unwrap_or_else(|e| panic!("seed {seed}, on: {e}"));
        (off, on)
    }) {
        println!("seed {seed}: ops sent {:?}, digest off {:x} on {:x}", on.ops_sent, off.ops_digest, on.ops_digest);
        assert_eq!(off.ops_sent, on.ops_sent, "seed {seed}: recording changed how many ops the pages sent");
        assert_eq!(off.ops_digest, on.ops_digest, "seed {seed}: recording changed what the pages sent, or when");
    }
}

/// The model's page Params: the defaults, and `CRAFTWORKS_MODEL_BLOCK_BUDGET` (bytes) as the page's block-store
/// budget when set -- a tiny one (1) makes eviction run at the end of every step, in every schedule (sdk#411).
fn model_params() -> Params {
    let mut p = Params::default();
    if let Some(b) = std::env::var("CRAFTWORKS_MODEL_BLOCK_BUDGET").ok().and_then(|v| v.trim().parse().ok()) {
        p.max_page_block_bytes = b;
    }
    p
}

fn check(apps: &mut [App], i: usize, node: &Node, seen: &mut Seen, now: u64, held: Option<Cid>, edges: &BTreeMap<(u64, Cid), (u64, Cid)>) -> Result<(), String> {
    // THE PIN RULE HOLDS (sdk#411): no re-put ever found its block's bytes evicted, and no parked write's pins
    // outlive the write that owns them.
    let missing = apps[i].page.reput_missing();
    if missing > 0 {
        return Err(format!("page {i}: {missing} re-put(s) found their block's bytes EVICTED (a pinned block was dropped)"));
    }
    if !apps[i].page.parked_write_is_live() {
        return Err(format!("page {i}: a parked write outlived its owner (its pins would outlive it)"));
    }
    // THE REGISTER IS NEVER 2+ BEHIND THE SIGNER'S RECORD (1b on the sign
    // side, the architect's attack): past one, the record for the seq between
    // is overwritten and no page could land it.
    for secrets in &node.secrets {
        if let Some(rec) = secrets.get(signer::RECORD) {
            let rec: signer::Record = bincode::deserialize(rec).expect("the signer's record");
            let reg = node.head().map_or(0, |h| h.0);
            if rec.next.seq > reg + 1 {
                return Err(format!("the signer's record (seq {}) is {} ahead of the register (seq {reg})", rec.next.seq, rec.next.seq - reg));
            }
        }
    }
    let a = &mut apps[i];
    let notices = a.page.take_notices();
    // A write that CHANGES NOTHING (its value already in the tree) commits nothing: it is told Published at the
    // engine's head -- someone else's commit, judged by its own writes -- in the same step as Accepted or as
    // ParityComplete (`App::nothing_changed`).
    for (_, w, st) in &notices {
        let with = |s: State| notices.iter().any(|(_, w2, st2)| w2 == w && *st2 == s);
        if *st == State::Published && (with(State::Accepted) || with(State::ParityComplete)) {
            a.nothing_changed.insert(w.0);
        }
    }
    for (_, wid, state) in notices {
        match state {
            State::Published => {
                // INVARIANT 1: Published at a head the register was READ to
                // hold — now or earlier (a write that changes nothing is
                // Published at the engine's published head, and another page
                // may have moved the register on since; sdk#160) — and the
                // write's own value is in that head's tree.
                let (seq, root) = a.page.published();
                if !node.held_heads.contains(&(seq, root)) {
                    return Err(format!(
                        "page {i}: write {} Published at ({seq}, {root:?}), a head the register never held (it holds {:?})",
                        wid.0,
                        node.head()
                    ));
                }
                if let Some((k, v)) = a.inflight.get(&wid.0) {
                    // As a REPAIRING reader reads it (§P: SAVED is k of
                    // k+m per group; its stragglers may still be in flight).
                    match node.tree_repairing(&root) {
                        Ok(t) if t.get(k) == Some(v) => {}
                        Ok(_) => return Err(format!("page {i}: write {} Published at ({seq}, ..) whose tree does not hold its value, even repaired", wid.0)),
                        Err(why) => return Err(format!("page {i}: write {} Published at ({seq}, ..) whose tree is not readable, even repaired: {why}", wid.0)),
                    }
                }
                // INVARIANT 3: recoverable, not necessarily whole (§P).
                if let Err(why) = node.tree_repairing(&root) {
                    return Err(format!("page {i}: Published at a root that is not RECOVERABLE on the node: {why}"));
                }
                if i == 1 && held.is_some_and(|h| !node.blocks.contains_key(&h) && node.tree(&root).is_none()) {
                    seen.over_the_hole += 1;
                }
                seen.published_at.push((i, wid.0, (seq, root), a.inflight.get(&wid.0).cloned()));
                if a.inflight.remove(&wid.0).is_some() {
                    a.published += 1;
                    seen.published += 1;
                }
            }
            State::Lost | State::Busy | State::Failed { .. } | State::TooLarge { .. } => {
                if state == State::Lost {
                    // ONLY A CONFIRMED WINNER IS ADOPTED (the architect's
                    // attack on sdk#225, case 1): the head this page now
                    // stands on never loses the tie-break to a record this
                    // page's signer made at the same seq.
                    let (ps, pr) = a.page.published();
                    let theirs = node.held_values.get(&(ps, pr)).cloned().unwrap_or_else(|| pr.to_vec());
                    for rec in a.page.signer_records() {
                        let r = page::HeadRead::from_record(rec).expect("a signer record");
                        if r.seq == ps && r.root() != pr && page::beats(r.value(), &theirs) {
                            return Err(format!("page {i}: adopted ({ps}, ..), which LOSES the tie-break to its own record at that seq"));
                        }
                    }
                    seen.lost += 1;
                } else if state == State::Busy {
                    seen.busy += 1;
                }
                if let Some(w) = a.inflight.remove(&wid.0) {
                    a.todo.push(w);
                }
            }
            // NO WRITE IS STALLED BEFORE ITS BUDGET: the engine says Stalled
            // after `max_accept_age` (64) SECONDS unconfirmed. A page clock
            // passed through in ms (#215's defect) told writes Stalled after
            // 64 ms — the model now sees that, not only the unit test.
            // The engine's clock is WHOLE seconds (`engine_seconds` floors),
            // so 64 of its seconds can be just over 63 real ones: seed 10's
            // fork run is told Stalled at 63 815 ms, inside the budget.
            State::Stalled => {
                let since = a.submitted.get(&wid.0).copied().unwrap_or(0);
                let age = now.saturating_sub(since);
                if age <= 63_000 {
                    return Err(format!("page {i}: write {} told Stalled {age} ms after it was submitted (budget 64 engine seconds, over 63 s)", wid.0));
                }
            }
            // INVARIANT 3b (the architect's ruling: BACKED_UP is a WRITE state, rule 10): every group the commit
            // that published the write CHANGED is whole on the node, with no repair. The whole TREE is the assets
            // dashboard's question (KEEPER §4): a straggler of an older write in a group this commit did not touch
            // keeps THAT write un-BACKED_UP, not this one.
            //
            // A CARRIED write (a later root move re-coded its groups: `supersede` withdrew the old version's
            // stragglers and moved the write onto a newer own commit's Backing) is judged by its CARRIER: the later
            // commit of this page that RE-CODED the hole's group, with every group IT changed whole
            // (`judge_backed_up`). Never by what the current head references, nor by any unrelated later commit
            // that finished (the architect: both trust the carry instead of checking it).
            State::ParityComplete if !a.nothing_changed.contains(&wid.0) => {
                if let Some((_, _, own, _)) = seen.published_at.iter().rev().find(|(p, w, _, _)| *p == i && *w == wid.0) {
                    let later: Vec<(u64, Cid)> = seen.published_at.iter().filter(|(p, _, _, _)| *p == i).map(|(_, _, h, _)| *h).collect();
                    if let Err(why) = judge_backed_up(node, edges, *own, &later, Carrier::Recoded) {
                        return Err(format!("page {i}: write {} is ParityComplete, but {why}", wid.0));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// THE ONE KNOB for how many random fault schedules the model runs: `CRAFTWORKS_MODEL_SEEDS`, read by
/// `testkit::model` for every model test (sdk#536; the batch gate runs the full count, `gate.sh --pr` a share). The
/// page model's full count IS the knob's unit. Every loop below derives its seeds from it ([`seed_range`]) and runs
/// them on every core (`model::run` / `model::until`); `CRAFTWORKS_MODEL_SEED=<n>` runs one seed alone.
const FULL_SEEDS: u64 = model::REFERENCE;
const WRITES: usize = 12;

fn seeds() -> u64 {
    model::knob()
}

/// One loop's seeds, as `num/den` of the knob: AT LEAST that many (`min`), then on while the loop's coverage floor
/// is not reached, NEVER past the same share of the full count (`cap`). So a small count is still a real check --
/// every floor is still reached, or the loop runs to what it always ran and the floor assert says so -- and the full
/// count runs exactly what it always ran.
fn seed_range(num: u64, den: u64) -> (u64, u64) {
    let min = model::share(FULL_SEEDS * num / den);
    (min, (FULL_SEEDS * num / den).max(min))
}

/// A floor (the sweep reached what it is about) is asserted over a whole sweep, never over one seed run alone.
fn whole_sweep() -> bool {
    model::alone().is_none()
}

#[test]
fn two_pages_on_one_key_publish_every_write_through_faults_and_the_invariants_hold() {
    let mut total = Seen::default();
    let (min, cap) = seed_range(1, 1);
    let all = model::until(
        1,
        min,
        cap,
        |seed| run(seed, WRITES, PutPath::Page).unwrap_or_else(|e| panic!("seed {seed}: {e}")),
        |all| all.iter().any(|(_, s)| s.lost > 0) && all.iter().any(|(_, s)| s.record_not_saved > 0),
    );
    let seed = all.len() as u64;
    for (_, s) in all {
        total.published += s.published;
        total.lost += s.lost;
        total.busy += s.busy;
        total.updates += s.updates;
        total.record_not_saved += s.record_not_saved;
        total.landings += s.landings;
        total.most_landing_updates = total.most_landing_updates.max(s.most_landing_updates);
    }
    println!("{seed} seeds (CRAFTWORKS_MODEL_SEEDS={}) × 2 pages × {WRITES} writes: {total:?}", seeds());
    // The model is not vacuous: the race and the faults were reached.
    assert_eq!(total.published, seed as usize * 2 * WRITES, "not every write was published once");
    if whole_sweep() {
        assert!(total.lost > 0, "no rebase was ever reached: the two pages never raced");
        assert!(total.updates > total.published / 2, "too few UPDATEs for the writes published");
        assert!(total.record_not_saved > 0, "the signer never failed to save its record: RecordNotSaved unexercised");
    }
    // The LAND cell is NOT floored here: a random run reached it only by schedule luck (1 landing in 80 seeds at
    // budget 1 on main, lost entirely to a correct back-off change). It is reached BY CONSTRUCTION, at every seed
    // and budget, in `a_landing_whose_update_is_lost_twice_still_lands` (the architect, 2026-09-26).
}

/// LANDING UNDER LOSS, BY CONSTRUCTION AT EVERY SEED AND BUDGET (the architect, 2026-09-26). A random run reached
/// a landing only by schedule luck (1 in 80 seeds at budget 1 on main), and a correct back-off change (OP-LIFE's Q4)
/// took that luck away. So every seed plays the stale-page landing (`a_gone_with_its_record_unlanded`: A's record
/// signed, its UPDATE never landed, A gone) with B's landing UPDATE lost `seed % 4` times, on the MODEL's params
/// (so `CRAFTWORKS_MODEL_BLOCK_BUDGET` applies): B must land A's record, needing exactly those re-sends, and publish
/// its own write on top.
#[test]
fn a_landing_whose_update_is_lost_twice_still_lands() {
    // `seed % 4` UPDATEs lost: every loss count 0..=3 needs four seeds, whatever share the knob gives.
    let min = seed_range(1, 2).0.max(4);
    let mut most = 0;
    for (seed, (landings, needed)) in model::run(1..min + 1, |seed| a_stale_page_lands_losing((seed % 4) as u32)) {
        let lose = (seed % 4) as u32;
        assert_eq!(landings, 1, "seed {seed}: B did not land A's record exactly once (losing {lose})");
        assert!(needed > lose, "seed {seed}: the landing needed {needed} UPDATEs, but {lose} were lost");
        most = most.max(needed);
    }
    println!("forced landing: {min} seeds, each landed; most UPDATEs one landing needed: {most}");
    assert!(!whole_sweep() || most >= 4, "no seed lost the landing's UPDATE three times: the harshest case was not reached");
}

/// TWO LIVE PAGES UNDER HEAVY UPDATE LOSS (UPDATEs lost three times as often, at random): every write still
/// publishes, with every invariant the run checks. Its landing count is PRINTED, not floored: a landing here was
/// schedule luck (the LAND cell is owned by `a_landing_whose_update_is_lost_twice_still_lands`, by construction),
/// but a drop to zero stays visible in the output.
#[test]
fn two_live_pages_under_heavy_update_loss_publish_every_write() {
    let harsh = Cfg { faults: Faults { update_lost: 300, ..FAULTS }, ..NORMAL };
    let (min, _) = seed_range(1, 2);
    let (mut landings, mut most) = (0, 0);
    for (seed, s) in model::run(1..min + 1, |seed| run_with(seed, WRITES, PutPath::Page, harsh).unwrap_or_else(|e| panic!("seed {seed}: {e}"))) {
        assert_eq!(s.published, 2 * WRITES, "seed {seed}: not every write published");
        landings += s.landings;
        most = most.max(s.most_landing_updates);
    }
    println!("harsh, two live pages: {min} seeds, every write published; {landings} landings (information, not a floor), most UPDATEs one needed: {most}");
}

/// The stale-page landing on the model's params, B's landing UPDATE lost `lose` times: (B's landings, the most
/// UPDATEs one needed).
fn a_stale_page_lands_losing(lose: u32) -> (u32, u32) {
    let (mut node, _) = Node::new();
    let mut a = Page::new(model_params(), PutPath::Page);
    let mut b = Page::new(model_params(), PutPath::Page);
    let mut now = 1_000u64;
    serve(&mut a, &mut node, &mut now, &mut 0);
    serve(&mut b, &mut node, &mut now, &mut 0);
    a_gone_with_its_record_unlanded(a, &mut node, &mut now);
    b.write(ClientId(2), WriteId(1), vec![(b"b1".to_vec(), WriteOp::Put(b"z".to_vec()))]);
    let mut left = lose;
    serve(&mut b, &mut node, &mut now, &mut left);
    assert_eq!(left, 0, "B did not re-send its landing's UPDATE: {} of {lose} losses unused", left);
    assert_eq!(node.head().map(|h| h.0), Some(2), "A's record was never landed");
    b.write(ClientId(2), WriteId(2), vec![(b"b1".to_vec(), WriteOp::Put(b"z".to_vec()))]);
    serve(&mut b, &mut node, &mut now, &mut 0);
    let (_, root) = node.head().expect("a head");
    let tree = node.tree(&root).expect("whole");
    for (k, v) in [(&b"a1"[..], &b"x"[..]), (b"a2", b"y"), (b"b1", b"z")] {
        assert_eq!(tree.get(k).map(Vec::as_slice), Some(v), "{:?} missing from the final tree", String::from_utf8_lossy(k));
    }
    assert!(b.unusable().is_empty(), "{:?}", b.unusable());
    b.landings()
}

/// TWO DEVICES OF ONE IDENTITY (sdk#225): two signers, one key, one
/// register, racing at every seq through the same faults, `HeadChanged`
/// pushes lost 30% of the time. The same identity never forks: no page is
/// ever unusable, every adopt is the tie-break's CONFIRMED winner, every
/// write publishes, and both pages end on the register's head. Non-vacuous:
/// the register held some seq under two roots. A Published write a winner
/// displaced is counted — the per-key merge (#225b) is what keeps it.
#[test]
fn two_devices_on_one_key_race_and_no_page_is_ever_unusable() {
    let cfg = Cfg { devices: 2, ..NORMAL };
    let (mut races, mut displaced, mut lost) = (0, 0, 0);
    let (min, cap) = seed_range(1, 2);
    let all = model::until(1, min, cap, |seed| run_with(seed, WRITES, PutPath::Page, cfg).unwrap_or_else(|e| panic!("seed {seed}: {e}")), |all| {
        all.iter().any(|(_, s)| s.races > 0)
    });
    let seeds_run = all.len();
    for (seed, s) in all {
        assert_eq!(s.published, 2 * WRITES, "seed {seed}: not every write was published once");
        races += s.races;
        displaced += s.displaced;
        lost += s.lost;
    }
    println!("two devices: {seeds_run} seeds, {races} raced seqs, {lost} Lost and re-sent, {displaced} Published writes displaced by a winner (#225b keeps them)");
    assert!(!whole_sweep() || races > 0, "the two devices never raced at a seq: the test is vacuous");
}

/// SAFETY GAP CLASS 2, FORCED (the architect's ruling; engineer3 found it by the model under #390's timing): a
/// straggler of ONE page's commit -- its own new value block, lost on every send until the calm, so the commit
/// publishes at k -- while the OTHER page adopts that head and commits on top. The other page's own blocks all ack,
/// and BACKED_UP must still wait for the foreign block: invariant 3b (at `ParityComplete` the root is WHOLE on the
/// node). No random fault: the hold is the whole schedule. Non-vacuous: the block was dropped, and page 1 published
/// at a root that was not whole while it was.
#[test]
fn a_foreign_straggler_in_flight_holds_back_the_other_pages_backed_up() {
    let cfg = Cfg { faults: CALM, hold_page0_first_value: true, ..NORMAL };
    let writes = WRITES;
    let (mut drops, mut over) = (0, 0);
    let (min, _) = seed_range(4, FULL_SEEDS);
    for (seed, s) in model::run(1..min + 1, |seed| run_with(seed, writes, PutPath::Page, cfg).unwrap_or_else(|e| panic!("seed {seed}: {e}"))) {
        assert_eq!(s.published, 2 * writes, "seed {seed}: not every write was published once");
        drops += s.held_drops;
        over += s.over_the_hole;
    }
    println!("forced straggler: {min} seeds, {drops} sends dropped, {over} page-1 writes Published over the hole");
    assert!(drops > 0, "the straggler was never held: the test is vacuous");
    assert!(over > 0, "page 1 never published over the hole: the interleaving was not forced");
}

/// AN IDLE PAGE LEARNS WITH EVERY HINT LOST: no `HeadChanged` ever reaches a
/// page, so only the backstop read (HEAD_BACKSTOP_MS) tells a device that
/// stopped writing that the other one moved the register. Both still end on
/// the register's head (the run's convergence check).
#[test]
fn with_every_hint_lost_an_idle_device_still_learns_by_the_backstop() {
    let cfg = Cfg { devices: 2, no_hints: true, ..NORMAL };
    let (min, _) = seed_range(6, FULL_SEEDS);
    model::run(1..min + 1, |seed| {
        run_with(seed, WRITES, PutPath::Page, cfg).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
    });
}

/// THE CONTROL for invariant 3: the whole-tree check FAILS on a root with a
/// block missing. No executor mutant reached it — the signer's own root check
/// and the `after` ordering stop a root going out before its blocks, so
/// invariant 1 fires first — so this shows the check can fire at all.
#[test]
fn control_the_whole_tree_check_fails_on_a_missing_block() {
    let (mut node, _) = Node::new();
    let mut p = Page::new(Params::default(), PutPath::Page);
    p.write(ClientId(1), WriteId(1), vec![(b"k".to_vec(), WriteOp::Put(vec![7u8; 5_000]))]);
    let mut puts = Vec::new();
    for _ in 0..20 {
        for op in p.take_ops() {
            match op {
                Op::Put { id, bytes } => {
                    node.put(id, &bytes);
                    puts.push(id);
                    p.answer(Answer::PutOk(id), Ms(0));
                }
                Op::ReadHead { .. } => p.answer(Answer::Head { label: page::Label::Head, read: node.head_read() }, Ms(0)),
                Op::Sign { id, prev_seq, prev_root, seq, root, ledger, .. } => {
                    let (id, a) = node.sign(id, prev_seq, prev_root, seq, root, ledger);
                    p.answer(Answer::Signer { id, answer: a }, Ms(0));
                }
                Op::Update { state, .. } => {
                    node.update(&state);
                    p.answer(Answer::Updated { label: page::Label::Head }, Ms(0));
                }
                Op::Get { id } => p.answer(Answer::GetMissed(id), Ms(0)),
                Op::AskHeld { batch, ids } => p.answer(Answer::Held { batch, present: ids.iter().map(|id| node.blocks.contains_key(id)).collect() }, Ms(0)),
                Op::PutApp { key } => p.answer(Answer::AppPutOk(key), Ms(0)),
                Op::Ext(_) => {}
            }
        }
    }
    let (_, root) = node.head().expect("the write published");
    assert!(node.tree(&root).is_some(), "the published tree is whole");
    let gone = *puts.iter().find(|b| **b != root).expect("a block under the root");
    node.blocks.remove(&gone);
    assert!(node.tree(&root).is_none(), "the whole-tree check passed over a missing block");
    // And the re-stated 3b (the commit's changed groups): the first commit changed every group.
    assert!(node.changed_groups_whole((0, [0u8; 32]), &root, None).is_err(), "the changed-groups check passed over a missing block");
}

/// One page's write, driven to a standstill against the node with every op answered; the signer's record edge
/// (next -> prev) recorded as the model's `edges`. The head it ends on.
fn drive_one(p: &mut Page, node: &mut Node, edges: &mut BTreeMap<(u64, Cid), (u64, Cid)>, w: u64, key: &str, value: Vec<u8>) -> (u64, Cid) {
    p.write(ClientId(1), WriteId(w), vec![(key.as_bytes().to_vec(), WriteOp::Put(value))]);
    for _ in 0..40 {
        for op in p.take_ops() {
            match op {
                Op::Put { id, bytes } => {
                    node.put(id, &bytes);
                    p.answer(Answer::PutOk(id), Ms(0));
                }
                Op::ReadHead { .. } => p.answer(Answer::Head { label: page::Label::Head, read: node.head_read() }, Ms(0)),
                Op::Sign { id, prev_seq, prev_root, seq, root, ledger, .. } => {
                    let (id, a) = node.sign(id, prev_seq, prev_root, seq, root, ledger);
                    if let Some(rec) = node.secrets[0].get(signer::RECORD) {
                        let rec: signer::Record = bincode::deserialize(rec).expect("the signer's record");
                        edges.insert((rec.next.seq, rec.next.root), (rec.prev.seq, rec.prev.root));
                    }
                    p.answer(Answer::Signer { id, answer: a }, Ms(0));
                }
                Op::Update { state, .. } => {
                    node.update(&state);
                    p.answer(Answer::Updated { label: page::Label::Head }, Ms(0));
                }
                Op::Get { id } => match node.blocks.get(&id) {
                    Some(b) => p.answer(Answer::Got { id, bytes: b.clone() }, Ms(0)),
                    None => p.answer(Answer::GetMissed(id), Ms(0)),
                },
                Op::AskHeld { batch, ids } => p.answer(Answer::Held { batch, present: ids.iter().map(|id| node.blocks.contains_key(id)).collect() }, Ms(0)),
                Op::PutApp { key } => p.answer(Answer::AppPutOk(key), Ms(0)),
                Op::Ext(_) => {}
            }
        }
    }
    node.head().expect("the write published")
}

/// THE CONTROL for 3b's CARRIER (the architect's narrowing): a write whose changed group is left NOT whole (its
/// value block gone from the node) is not carried by a later commit that finished in ANOTHER group -- only by
/// one that RE-CODED the hole's group. Here write 3 adds a much larger value (another size class: another value
/// group of the same leaf) and is whole: the loose carrier (`Any`) excuses write 2, the narrow one (`Recoded`)
/// does not. Write 4 then overwrites write 2's key: its group is re-coded without the lost block, and THAT
/// carries write 2.
#[test]
fn a_later_commit_in_another_group_does_not_carry_a_broken_write() {
    let (mut node, _) = Node::new();
    let mut edges = BTreeMap::new();
    let mut p = Page::new(Params::default(), PutPath::Page);
    let _ = drive_one(&mut p, &mut node, &mut edges, 1, "a", vec![1u8; 3_000]);
    let before: std::collections::BTreeSet<Cid> = node.blocks.keys().copied().collect();
    let two = drive_one(&mut p, &mut node, &mut edges, 2, "b", vec![2u8; 3_000]);
    let value_b = *node
        .blocks
        .iter()
        .find(|(id, b)| !before.contains(*id) && freenet_prolly::block_id(freenet_prolly::kind::RAW, b) == **id)
        .expect("write 2's value block")
        .0;
    node.blocks.remove(&value_b);
    assert!(judge_backed_up(&node, &edges, two, &[], Carrier::Recoded).is_err(), "THE SETUP: write 2's changed groups are whole with its value gone");
    let three = drive_one(&mut p, &mut node, &mut edges, 3, "c", vec![3u8; 60_000]);
    assert!(node.changed_groups_whole(edges[&three], &three.1, None).is_ok(), "THE SETUP: write 3's own changed groups are not whole");
    assert!(judge_backed_up(&node, &edges, two, &[three], Carrier::Any).is_ok(), "THE SETUP: the loose carrier did not excuse write 2 -- the case does not show the gap");
    let why = judge_backed_up(&node, &edges, two, &[three], Carrier::Recoded).expect_err("a later commit in ANOTHER group carried a write whose group is not whole");
    println!("narrow carrier, write 3 in another group: {why}");
    let four = drive_one(&mut p, &mut node, &mut edges, 4, "b", vec![4u8; 3_000]);
    assert!(judge_backed_up(&node, &edges, two, &[three, four], Carrier::Recoded).is_ok(), "a commit that RE-CODED the hole's group without the lost block did not carry write 2");
}

/// THE WRAPPER PATH: a PUT's answer confirms nothing, only the signer's
/// read-local `Held` does. Same faults, same invariants.
#[test]
fn on_the_wrapper_path_a_put_is_confirmed_by_held_and_the_invariants_hold() {
    let mut published = 0;
    let (min, _) = seed_range(1, 2);
    let all = model::run(1..min + 1, |seed| run(seed, WRITES, PutPath::Wrapper).unwrap_or_else(|e| panic!("seed {seed}: {e}")));
    for (_, s) in &all {
        published += s.published;
    }
    assert_eq!(published, all.len() * 2 * WRITES);
}

/// Serve every op of `p` until it idles, LOSING the next `drop_updates` UPDATEs (each unanswered, not applied).
fn serve(p: &mut Page, node: &mut Node, now: &mut u64, drop_updates: &mut u32) {
    for _ in 0..400 {
        let ops = p.take_ops();
        if ops.is_empty() {
            if !p.waiting() {
                return;
            }
            *now = p.next_due().map_or(*now + 1, |d| d.0.max(*now + 1));
            p.tick(Ms(*now));
            continue;
        }
        for op in ops {
            let ans = match op {
                Op::Put { id, bytes } => {
                    node.put(id, &bytes);
                    Some(Answer::PutOk(id))
                }
                Op::Get { id } => Some(match node.blocks.get(&id) {
                    Some(b) => Answer::Got { id, bytes: b.clone() },
                    None => Answer::GetMissed(id),
                }),
                Op::Sign { id, prev_seq, prev_root, seq, root, ledger, .. } => {
                    let (id, answer) = node.sign(id, prev_seq, prev_root, seq, root, ledger);
                    Some(Answer::Signer { id, answer })
                }
                Op::Update { state, .. } => {
                    if *drop_updates > 0 {
                        *drop_updates -= 1;
                        None
                    } else {
                        node.update(&state);
                        Some(Answer::Updated { label: page::Label::Head })
                    }
                }
                Op::ReadHead { .. } => Some(Answer::Head { label: page::Label::Head, read: node.head_read() }),
                Op::AskHeld { batch, ids } => Some(Answer::Held { batch, present: ids.iter().map(|id| node.blocks.contains_key(id)).collect() }),
                Op::PutApp { key } => Some(Answer::AppPutOk(key)),
                Op::Ext(_) => None,
            };
            if let Some(ans) = ans {
                p.answer(ans, Ms(*now));
            }
        }
    }
}

/// Page A publishes seq 1, then SIGNS its second commit (the signer's record is seq 2), its UPDATE never lands,
/// and A is GONE: the register one behind the record, for a stale page to land (the LAND cell).
fn a_gone_with_its_record_unlanded(mut a: Page, node: &mut Node, now: &mut u64) {
    let (node, now) = (node, now);
    // A publishes seq 1.
    a.write(ClientId(1), WriteId(1), vec![(b"a1".to_vec(), WriteOp::Put(b"x".to_vec()))]);
    serve(&mut a, node, now, &mut 0);
    assert_eq!(node.head().map(|h| h.0), Some(1));
    // A's second commit is SIGNED (record seq 2), its UPDATE never lands, and A is gone.
    a.write(ClientId(1), WriteId(2), vec![(b"a2".to_vec(), WriteOp::Put(b"y".to_vec()))]);
    for _ in 0..30 {
        let ops = a.take_ops();
        for op in ops {
            let ans = match op {
                Op::Put { id, bytes } => {
                    node.put(id, &bytes);
                    Some(Answer::PutOk(id))
                }
                Op::Sign { id, prev_seq, prev_root, seq, root, ledger, .. } => {
                        let (id, answer) = node.sign(id, prev_seq, prev_root, seq, root, ledger);
                        Some(Answer::Signer { id, answer })
                    }
                // Reads are answered as a node answers them: under a small block budget (the model's
                // CRAFTWORKS_MODEL_BLOCK_BUDGET) A evicts, and its second commit must read its blocks back.
                Op::Get { id } => Some(match node.blocks.get(&id) {
                    Some(b) => Answer::Got { id, bytes: b.clone() },
                    None => Answer::GetMissed(id),
                }),
                Op::AskHeld { batch, ids } => Some(Answer::Held { batch, present: ids.iter().map(|id| node.blocks.contains_key(id)).collect() }),
                // The UPDATE is the one thing never answered: A is gone before it lands.
                _ => None,
            };
            if let Some(ans) = ans {
                a.answer(ans, Ms(*now));
            }
        }
    }
    let rec: signer::Record = bincode::deserialize(node.secrets[0].get(signer::RECORD).expect("a record")).expect("decodes");
    assert_eq!((rec.next.seq, node.head().map(|h| h.0)), (2, Some(1)), "the record is not one ahead of the register");
    drop(a);
}

/// THE LAND CELL, where it is needed: page A signs its commit, its UPDATE
/// never lands, and A is GONE. Page B is stale (it never saw A's first head),
/// so the signer answers it NotNext naming A's unlanded record. The register is
/// one behind, so B LANDS A's record itself (asking from the register's head
/// with next = register + 1: `AlreadySigned`), adopts it once the register
/// shows it, and B's own write — told Lost by that rebase — publishes on top.
#[test]
fn a_stale_page_lands_a_gone_pages_record_then_publishes() {
    let (mut node, _) = Node::new();
    let mut a = Page::new(Params::default(), PutPath::Page);
    let mut b = Page::new(Params::default(), PutPath::Page);
    let mut now = 1_000u64;
    // Both start on the empty tree.
    serve(&mut a, &mut node, &mut now, &mut 0);
    serve(&mut b, &mut node, &mut now, &mut 0);
    a_gone_with_its_record_unlanded(a, &mut node, &mut now);
    // B, stale at seq 0, writes: NotNext names record 2; B lands it.
    b.write(ClientId(2), WriteId(1), vec![(b"b1".to_vec(), WriteOp::Put(b"z".to_vec()))]);
    serve(&mut b, &mut node, &mut now, &mut 0);
    let lost = b.take_notices().iter().any(|(_, w, s)| w.0 == 1 && *s == State::Lost);
    assert!(lost, "B's write was not told Lost by the rebase onto the landed record");
    assert!(b.landings().0 > 0, "B never landed the record");
    assert_eq!(node.head().map(|h| h.0), Some(2), "A's record was never landed");
    b.write(ClientId(2), WriteId(2), vec![(b"b1".to_vec(), WriteOp::Put(b"z".to_vec()))]);
    serve(&mut b, &mut node, &mut now, &mut 0);
    let (_, root) = node.head().expect("a head");
    let tree = node.tree(&root).expect("whole");
    for (k, v) in [(&b"a1"[..], &b"x"[..]), (b"a2", b"y"), (b"b1", b"z")] {
        assert_eq!(tree.get(k).map(Vec::as_slice), Some(v), "{:?} missing from the final tree", String::from_utf8_lossy(k));
    }
    assert!(b.unusable().is_empty(), "{:?}", b.unusable());
}

/// A LANDING WHOSE UPDATE IS LOST TWICE, BY CONSTRUCTION (sdk#425; main's condition 2): the LAND cell above, with
/// B's first TWO landing UPDATEs lost. B re-sends, the third lands A's record, and B's own write publishes on top.
/// No seed: the random run's twice-lost floor was schedule luck (sdk#424).
#[test]
fn a_stale_pages_landing_whose_update_is_lost_twice_still_lands() {
    let (mut node, _) = Node::new();
    let mut a = Page::new(Params::default(), PutPath::Page);
    let mut b = Page::new(Params::default(), PutPath::Page);
    let mut now = 1_000u64;
    serve(&mut a, &mut node, &mut now, &mut 0);
    serve(&mut b, &mut node, &mut now, &mut 0);
    a_gone_with_its_record_unlanded(a, &mut node, &mut now);
    b.write(ClientId(2), WriteId(1), vec![(b"b1".to_vec(), WriteOp::Put(b"z".to_vec()))]);
    let mut lose = 2u32;
    serve(&mut b, &mut node, &mut now, &mut lose);
    assert_eq!(lose, 0, "B did not re-send its landing's UPDATE after losing it: {} of the 2 losses were used", 2 - lose);
    let (landings, most) = b.landings();
    println!("B: {landings} landing(s), most UPDATEs one landing needed: {most}");
    assert_eq!(landings, 1, "B did not land A's record exactly once");
    assert!(most >= 3, "the landing's UPDATE was not lost twice and re-sent (most: {most})");
    assert_eq!(node.head().map(|h| h.0), Some(2), "A's record was never landed");
    b.write(ClientId(2), WriteId(2), vec![(b"b1".to_vec(), WriteOp::Put(b"z".to_vec()))]);
    serve(&mut b, &mut node, &mut now, &mut 0);
    let (_, root) = node.head().expect("a head");
    let tree = node.tree(&root).expect("whole");
    for (k, v) in [(&b"a1"[..], &b"x"[..]), (b"a2", b"y"), (b"b1", b"z")] {
        assert_eq!(tree.get(k).map(Vec::as_slice), Some(v), "{:?} missing from the final tree", String::from_utf8_lossy(k));
    }
    assert!(b.unusable().is_empty(), "{:?}", b.unusable());
}

/// **HF, a foreign member's Held re-put, on real engine state at a 1-byte budget** (sdk#411): page A publishes a
/// value group of 30; page B changes one value, so its commit asks the node about the group's OTHER members (A's,
/// ConfirmHeld) -- B fetched them to re-code the group, so it HOLDS their bytes. Answered ABSENT HELD_ABSENTS times,
/// B puts each AGAIN with those bytes (its self-heal of another page's straggler): only HF holds them while the ask is
/// out. Mutant "HF unpinned" -> evicted -> never put again -> red.
#[test]
fn a_foreign_member_absent_is_put_again_from_page_memory_at_a_one_byte_budget() {
    let (mut node, _) = Node::new();
    let mut a = Page::new(Params::default(), PutPath::Page);
    let mut b = Page::new(Params { max_page_block_bytes: 1, ..Params::default() }, PutPath::Page);
    let mut now = 1_000u64;
    /// Serve `p` as the node does; `absent`: every Held is answered absent. Returns the PUTs (id, bytes) it sent.
    fn serve(p: &mut Page, node: &mut Node, now: &mut u64, absent: bool, rounds: usize) -> Vec<(Cid, Vec<u8>)> {
        let mut puts = Vec::new();
        for _ in 0..rounds {
            let ops = p.take_ops();
            if ops.is_empty() {
                if !p.waiting() {
                    break;
                }
                *now = p.next_due().map_or(*now + 1, |d| d.0.max(*now + 1));
                p.tick(Ms(*now));
                continue;
            }
            for op in ops {
                let ans = match op {
                    Op::Put { id, bytes } => {
                        puts.push((id, bytes.clone()));
                        node.put(id, &bytes);
                        Some(Answer::PutOk(id))
                    }
                    Op::Get { id } => Some(match node.blocks.get(&id) {
                        Some(b) => Answer::Got { id, bytes: b.clone() },
                        None => Answer::GetMissed(id),
                    }),
                    Op::Sign { id, prev_seq, prev_root, seq, root, ledger, .. } => {
                        let (id, answer) = node.sign(id, prev_seq, prev_root, seq, root, ledger);
                        Some(Answer::Signer { id, answer })
                    }
                    Op::Update { state, .. } => {
                        node.update(&state);
                        Some(Answer::Updated { label: page::Label::Head })
                    }
                    Op::ReadHead { .. } => Some(Answer::Head { label: page::Label::Head, read: node.head_read() }),
                    Op::AskHeld { batch, ids } => Some(Answer::Held { batch, present: ids.iter().map(|id| !absent && node.blocks.contains_key(id)).collect() }),
                    Op::PutApp { key } => Some(Answer::AppPutOk(key)),
                    Op::Ext(_) => None,
                };
                if let Some(ans) = ans {
                    p.answer(ans, Ms(*now));
                }
            }
        }
        puts
    }
    serve(&mut a, &mut node, &mut now, false, 400);
    serve(&mut b, &mut node, &mut now, false, 400);
    let rows: Vec<(Vec<u8>, WriteOp)> = (0..30).map(|i| (format!("v/{i:02}").into_bytes(), WriteOp::Put(vec![i as u8; 2_000]))).collect();
    a.write(ClientId(1), WriteId(1), rows);
    serve(&mut a, &mut node, &mut now, false, 2_000);
    assert_eq!(node.head().map(|h| h.0), Some(1), "THE SETUP: A did not publish");
    let a_values: BTreeMap<Cid, Vec<u8>> = node.blocks.iter().filter(|(_, v)| v.len() == 2_000).map(|(k, v)| (*k, v.clone())).collect();
    b.head_hint();
    serve(&mut b, &mut node, &mut now, false, 400);
    b.write(ClientId(2), WriteId(1), vec![(b"v/05".to_vec(), WriteOp::Put(vec![99u8; 2_000]))]);
    let puts = serve(&mut b, &mut node, &mut now, true, 5_000);
    let again: Vec<&(Cid, Vec<u8>)> = puts.iter().filter(|(id, _)| a_values.contains_key(id)).collect();
    println!("B put {} of A's {} value blocks again from its memory (1-byte budget); B's store peak pinned {} B", again.len(), a_values.len(), b.blocks().stats().peak_pinned_bytes);
    assert!(!again.is_empty(), "no foreign member absent on the node was put again from page memory: HF did not hold its bytes");
    assert!(again.iter().all(|(id, bytes)| a_values.get(id) == Some(bytes)), "a foreign member was put again with other bytes");
    assert_eq!(b.reput_missing(), 0);
}

/// THE FOREIGN MEMBERS' HELD ASK, ON REAL ENGINE STATE (sdk#416's class 2; sdk#455): page A publishes a value group
/// of 30; page B, which never PUT those values, changes ONE of them. B's commit asks the node about the group's OTHER
/// members (ConfirmHeld) in ONE batched op. Answered ABSENT, each is asked again on the backoff -- the engine still
/// waits on it (rule 7) -- and PUT from here only after HELD_ABSENTS absents in a row, and only with the bytes B holds
/// (it fetched them to re-code the group).
#[test]
fn a_foreign_members_absent_are_asked_again_and_put_only_after_held_absents() {
    let (mut node, _) = Node::new();
    let mut a = Page::new(Params::default(), PutPath::Page);
    let mut b = Page::new(Params::default(), PutPath::Page);
    let mut now = 1_000u64;
    /// What `p` sent, in order: a Held ask (its ids) or a PUT (its id, and whether the page held its bytes).
    enum Sent {
        Held(Vec<Cid>),
        Put(Cid),
    }
    // Serve `p` as the node does; `absent`: every Held is answered absent.
    fn serve(p: &mut Page, node: &mut Node, now: &mut u64, absent: bool, rounds: usize) -> Vec<Sent> {
        let mut sent = Vec::new();
        for _ in 0..rounds {
            let ops = p.take_ops();
            if ops.is_empty() {
                if !p.waiting() {
                    break;
                }
                *now = p.next_due().map_or(*now + 1, |d| d.0.max(*now + 1));
                p.tick(Ms(*now));
                continue;
            }
            for op in ops {
                let ans = match op {
                    Op::Put { id, bytes } => {
                        sent.push(Sent::Put(id));
                        node.put(id, &bytes);
                        Some(Answer::PutOk(id))
                    }
                    Op::Get { id } => Some(match node.blocks.get(&id) {
                        Some(b) => Answer::Got { id, bytes: b.clone() },
                        None => Answer::GetMissed(id),
                    }),
                    Op::Sign { id, prev_seq, prev_root, seq, root, ledger, .. } => {
                        let (id, answer) = node.sign(id, prev_seq, prev_root, seq, root, ledger);
                        Some(Answer::Signer { id, answer })
                    }
                    Op::Update { state, .. } => {
                        node.update(&state);
                        Some(Answer::Updated { label: page::Label::Head })
                    }
                    Op::ReadHead { .. } => Some(Answer::Head { label: page::Label::Head, read: node.head_read() }),
                    Op::AskHeld { batch, ids } => {
                        sent.push(Sent::Held(ids.clone()));
                        Some(Answer::Held { batch, present: ids.iter().map(|id| !absent && node.blocks.contains_key(id)).collect() })
                    }
                    Op::PutApp { key } => Some(Answer::AppPutOk(key)),
                    Op::Ext(_) => None,
                };
                if let Some(ans) = ans {
                    p.answer(ans, Ms(*now));
                }
            }
        }
        sent
    }
    serve(&mut a, &mut node, &mut now, false, 400);
    serve(&mut b, &mut node, &mut now, false, 400);
    let rows: Vec<(Vec<u8>, WriteOp)> = (0..30).map(|i| (format!("v/{i:02}").into_bytes(), WriteOp::Put(vec![i as u8; 2_000]))).collect();
    a.write(ClientId(1), WriteId(1), rows);
    serve(&mut a, &mut node, &mut now, false, 400);
    assert_eq!(node.head().map(|h| h.0), Some(1), "THE SETUP: A did not publish");
    let a_blocks: std::collections::BTreeSet<Cid> = node.blocks.keys().copied().collect();
    b.head_hint();
    serve(&mut b, &mut node, &mut now, false, 400);
    b.write(ClientId(2), WriteId(1), vec![(b"v/05".to_vec(), WriteOp::Put(vec![99u8; 2_000]))]);
    let sent = serve(&mut b, &mut node, &mut now, true, 3_000);
    let helds: Vec<&Vec<Cid>> = sent.iter().filter_map(|s| if let Sent::Held(ids) = s { Some(ids) } else { None }).collect();
    let foreign: std::collections::BTreeSet<Cid> = helds.iter().flat_map(|ids| ids.iter()).copied().filter(|id| a_blocks.contains(id)).collect();
    let first = helds.first().map_or(0, |ids| ids.len());
    println!("B: {} Held op(s), the first of {} ids; {} foreign members asked", helds.len(), first, foreign.len());
    assert!(first >= 20, "THE SETUP: B's commit did not ask about the group's other members in ONE op ({first} ids)");
    for id in &foreign {
        // How many times it was asked before its first PUT from here (all of them if never PUT).
        let mut asks = 0;
        let mut put_after = None;
        for s in &sent {
            match s {
                Sent::Held(ids) if ids.contains(id) => asks += 1,
                Sent::Put(p) if p == id && put_after.is_none() => put_after = Some(asks),
                _ => {}
            }
        }
        assert!(asks >= 2, "a foreign member answered absent was not asked again (rule 7)");
        if let Some(n) = put_after {
            assert!(n >= page::HELD_ABSENTS as usize, "a foreign member was PUT after only {n} absent(s), not HELD_ABSENTS");
        }
    }
}
