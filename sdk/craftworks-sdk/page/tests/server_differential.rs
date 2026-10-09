//! `page::Server` over a scripted CLIENT API: the real signer
//! (`signer::serve`) signs, and every UPDATE goes through the Register
//! contract's own `update_state`.
//!
//! This began as THE DIFFERENTIAL against the engine delegate's Shell (main's
//! ruling B, condition 2): the same requests to both, the replies compared.
//! The Shell is deleted (the switch-over), so each case now asserts, on the
//! page alone, the concrete outcome the two AGREED on — the write states, the
//! rows, the tree. Where the page differs from the Shell by design (a restart
//! mid-commit, a same-key displacement), the page's behaviour is asserted.

mod common;
use engine::Params;
use freenet_prolly::store::Blocks;
use freenet_prolly::Cid;
use page::server::{Server, SignerFacts};
use page::fates::Fate;
use page::{Ms, Answer, Op, Page, PutPath};
use protocol::{Reply, Request, WriteState};
use std::collections::BTreeMap;

const BLOCK_CODE: &[u8] = b"differential block code";
const REGISTER_CODE: &[u8] = b"differential register code";
const SESSION: u64 = 7;
const VERSION: u16 = 4;

fn state_of(id: &Cid, body: &[u8]) -> Vec<u8> {
    let kind = [freenet_prolly::kind::TREE_NODE, freenet_prolly::kind::RAW, freenet_prolly::kind::PARITY]
        .into_iter()
        .find(|k| freenet_prolly::block_id(*k, body) == *id)
        .expect("every block hashes under one kind");
    let mut s = vec![kind];
    s.extend_from_slice(body);
    s
}

/// The node behind the page's client API: blocks, the head Register, and the
/// signer's secrets. Shared by every page on it.
struct Node {
    blocks: BTreeMap<Cid, Vec<u8>>,
    /// Blocks only a FETCH reaches (a cold node): a GET of one of these
    /// answers from here and does not populate `blocks`.
    network: Option<BTreeMap<Cid, Vec<u8>>>,
    contracts: BTreeMap<[u8; 32], Cid>,
    register: Option<Vec<u8>>,
    register_id: [u8; 32],
    register_params: Vec<u8>,
    secrets: BTreeMap<Vec<u8>, Vec<u8>>,
    record_fails: bool,
}

impl Blocks for Node {
    fn get(&self, cid: &Cid) -> Option<&[u8]> {
        self.blocks.get(cid).map(Vec::as_slice)
    }
}

struct Host<'a>(&'a mut Node);
impl signer::Host for Host<'_> {
    fn get_secret(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.0.secrets.get(key).cloned()
    }
    fn set_secret(&mut self, key: &[u8], value: &[u8]) -> bool {
        if self.0.record_fails && key == signer::RECORD {
            return false;
        }
        self.0.secrets.insert(key.to_vec(), value.to_vec());
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

impl Node {
    fn new() -> Node {
        let sk = ed25519_dalek::SigningKey::from_bytes(&[5u8; 32]);
        let params = wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME);
        let mut n = Node {
            blocks: BTreeMap::new(),
            network: None,
            contracts: BTreeMap::new(),
            register: None,
            register_id: signer::register_id(REGISTER_CODE, &params),
            register_params: params.clone(),
            secrets: BTreeMap::new(),
            record_fails: false,
        };
        let req = signer::Request::Provision {
            signing_key: sk.to_bytes().to_vec(),
            register_code: REGISTER_CODE.to_vec(),
            register_params: params,
            block_code: BLOCK_CODE.to_vec(),
        };
        assert_eq!(signer::serve(&mut Host(&mut n), &signer::encode_request(1, &req), signer::Origin::Local), signer::Answer::Provisioned);
        n
    }

    fn put(&mut self, id: Cid, body: &[u8]) {
        self.contracts.insert(contract_keys::block::contract_for(BLOCK_CODE, &id), id);
        self.blocks.insert(id, body.to_vec());
    }

    fn update(&mut self, state: &[u8]) {
        use freenet_stdlib::prelude::*;
        let params = Parameters::from(self.register_params.clone());
        self.register = Some(match &self.register {
            None => state.to_vec(),
            Some(cur) => <craftec_register_contract::Register as ContractInterface>::update_state(
                params,
                State::from(cur.clone()),
                vec![UpdateData::State(State::from(state.to_vec()))],
            )
            .expect("the register merged")
            .new_state
            .expect("a state")
            .as_ref()
            .to_vec(),
        });
    }

    fn head(&self) -> Option<(u64, Cid)> {
        let st = self.register.as_deref()?;
        contract_keys::register::head_of(st)
    }

    /// Every entry of the tree at `root`, or `None` if a block is missing.
    fn tree(&self, root: &Cid) -> Option<BTreeMap<Vec<u8>, Vec<u8>>> {
        use freenet_prolly::range::{range, read_value, Range};
        use std::ops::Bound;
        let mut out = BTreeMap::new();
        let mut after = None;
        loop {
            let r = Range { lo: Bound::Unbounded, hi: Bound::Unbounded, reverse: false, after: after.clone(), max_entries: 4096, max_bytes: usize::MAX };
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

    /// The head WHOLE, as page-io reads it off the node: seq and value.
    fn head_read(&self) -> Option<page::HeadRead> {
        page::HeadRead::from_record(self.register.as_deref()?)
    }

    fn sign(&mut self, id: u32, prev_seq: u64, prev_root: Cid, seq: u64, root: Cid, ledger: Vec<u8>) -> (u32, signer_proto::Answer) {
        let req = signer::Request::Sign {
            prev: signer::Head { seq: prev_seq, root: prev_root },
            next: signer::Next { seq, root, ledger },
            label: signer::Label::Head,
        };
        let served = signer::serve_full(&mut Host(self), &signer::encode_request(id, &req), signer::Origin::Local);
        wire::signer::read_answer(&signer::reply(&served)).expect("a signer answer reads back")
    }
}

/// Faults the page's node can play, each a switch like FullNode's.
#[derive(Default, Clone, Copy)]
struct Faults {
    /// Each block's FIRST put lands and its answer is lost; a re-send is
    /// answered. (Every answer lost for ever is a divergence by ruling: the
    /// page confirms a put only by its answer, the Shell read blocks back.)
    lose_first_put_answers: bool,
    /// The first GET of each block goes unanswered.
    lose_first_gets: bool,
    /// Each PARITY block's first put lands and its answer is lost (race put's
    /// STRAGGLER: the data acks, the head signs, the parity is re-sent).
    lose_first_parity_answers: bool,
    /// The signer fails to save its record on the first sign.
    record_not_saved_once: bool,
    /// Hold every answer (FullNode's `hold_answers`).
    hold: bool,
    /// Hold block GETs only (a cold node): what `run_holding_gets` starts.
    hold_gets: bool,
}

/// `page::Server` over the scripted node, run to a standstill like `Conn`.
struct PageRig {
    server: Server,
    /// The session this page's client speaks under: every page LOAD mints its
    /// own (a reload is a new session, and the page's device id in heads'
    /// `through` is its first session, COMMIT-LIFE ⁵).
    session: u64,
    now: u64,
    faults: Faults,
    got_once: std::collections::BTreeSet<Cid>,
    put_once: std::collections::BTreeSet<Cid>,
    held: Vec<Op>,
    /// GETs of these blocks are never answered: a node that is SILENT about
    /// them (not NotFound -- nothing at all).
    silent: std::collections::BTreeSet<Cid>,
    /// Every GET the node was asked, per block.
    gets: BTreeMap<Cid, usize>,
    /// When each GET was asked (simulated ms), in order: what a cost
    /// failure prints, so it says WHEN the re-sends went, not only how many.
    get_at: Vec<u64>,
}

impl PageRig {
    fn new() -> PageRig {
        PageRig::with(Params::default())
    }

    fn with(params: Params) -> PageRig {
        let page = Page::unstarted(params, PutPath::Page, Ms(0));
        PageRig {
            server: Server::new(page, SignerFacts { head_writable: true, head_id: [0; 32] }),
            session: SESSION,
            now: 1_000,
            faults: Faults::default(),
            got_once: Default::default(),
            put_once: Default::default(),
            held: Vec::new(),
            silent: Default::default(),
            gets: BTreeMap::new(),
            get_at: Vec::new(),
        }
    }

    fn client_as(&mut self, node: &mut Node, r: &Request) -> Vec<Reply> {
        let frame = protocol::encode_session_request(VERSION, self.session, r).expect("encodes");
        self.server.client(&frame);
        self.run(node)
    }

    /// Hand the page a request and nothing else: no `run`, so the clock does
    /// not move (a timed test drives it with [`PageRig::run_for`]).
    fn send_only(&mut self, r: &Request) {
        let frame = protocol::encode_session_request(VERSION, self.session, r).expect("encodes");
        self.server.client(&frame);
    }

    /// Run until `ms` of simulated time have passed, the page's own timers
    /// driving the clock (what a host that sleeps until `next_due` does).
    fn run_for(&mut self, node: &mut Node, ms: u64) -> Vec<Reply> {
        // Its OWN loop, never `run`: `run` moves the clock itself (to each
        // next deadline, up to 2,000 times), so a "5 minute" run through it
        // lasted 33 simulated hours and counted a day's worth of correct
        // 60 s re-sends as a storm.
        let until = self.now + ms;
        let mut replies = decode(self.server.take_replies());
        loop {
            for _ in 0..10_000 {
                let ops = self.server.take_ops();
                if ops.is_empty() {
                    break;
                }
                for op in ops {
                    if let Some(a) = self.answer(node, op) {
                        self.server.node(a, Ms(self.now));
                    }
                }
                replies.extend(decode(self.server.take_replies()));
            }
            if self.now >= until {
                break;
            }
            let next = self.server.page.next_due().map_or(until, |d| d.0.min(until));
            self.now = next.max(self.now + 1).min(until);
            self.server.tick(Ms(self.now));
            replies.extend(decode(self.server.take_replies()));
        }
        replies
    }

    /// Answer every op until none is left; when only silence is left, let the
    /// clock pass a deadline (bounded), as a real page's tick would.
    fn run(&mut self, node: &mut Node) -> Vec<Reply> {
        let mut replies = decode(self.server.take_replies());
        let mut idle = 0;
        for _ in 0..2_000 {
            let ops = self.server.take_ops();
            if ops.is_empty() {
                // Only silence left: let a deadline pass, as the page's own
                // tick would — bounded, so a page that never settles fails.
                if self.faults.hold || !self.server.page.waiting() || idle >= 40 {
                    break;
                }
                idle += 1;
                self.now = self.server.page.next_due().map_or(self.now + 1, |d| d.0.max(self.now + 1));
                self.server.tick(Ms(self.now));
                replies.extend(decode(self.server.take_replies()));
                continue;
            }
            idle = 0;
            for op in ops {
                if self.faults.hold {
                    self.held.push(op);
                    continue;
                }
                if let Some(a) = self.answer(node, op) {
                    self.server.node(a, Ms(self.now));
                }
            }
            replies.extend(decode(self.server.take_replies()));
        }
        replies
    }

    fn answer(&mut self, node: &mut Node, op: Op) -> Option<Answer> {
        Some(match op {
            Op::Put { id, bytes } => {
                node.put(id, &bytes);
                let parity = freenet_prolly::block_id(freenet_prolly::kind::PARITY, &bytes) == id;
                if (self.faults.lose_first_put_answers || (self.faults.lose_first_parity_answers && parity)) && self.put_once.insert(id) {
                    return None;
                }
                Answer::PutOk(id)
            }
            Op::Get { id } => {
                *self.gets.entry(id).or_insert(0) += 1;
                self.get_at.push(self.now);
                if self.silent.contains(&id) {
                    return None;
                }
                if self.faults.lose_first_gets && self.got_once.insert(id) {
                    return None;
                }
                if let Some(b) = node.blocks.get(&id) {
                    Answer::Got { id, bytes: b.clone() }
                } else if let Some(b) = node.network.as_ref().and_then(|n| n.get(&id)) {
                    Answer::Got { id, bytes: b.clone() }
                } else {
                    Answer::GetMissed(id)
                }
            }
            Op::Sign { id, prev_seq, prev_root, seq, root, ledger, .. } => {
                node.record_fails = std::mem::take(&mut self.faults.record_not_saved_once);
                let (id, answer) = node.sign(id, prev_seq, prev_root, seq, root, ledger);
                node.record_fails = false;
                Answer::Signer { id, answer }
            }
            Op::Update { state, .. } => {
                node.update(&state);
                Answer::Updated { label: page::Label::Head }
            }
            Op::ReadHead { .. } => Answer::Head { label: page::Label::Head, read: node.head_read() },
            Op::AskHeld { batch, ids } => Answer::Held { batch, present: ids.iter().map(|id| node.blocks.contains_key(id)).collect() },
            Op::PutApp { key } => Answer::AppPutOk(key),
            Op::Ext(_) => return None,
        })
    }
}

fn decode(frames: Vec<Vec<u8>>) -> Vec<Reply> {
    frames.iter().map(|f| protocol::decode_reply(f).expect("a reply decodes")).collect()
}


fn write(id: u64, ops: &[(&str, Option<&str>)]) -> Request {
    Request::forced_write(id, ops
            .iter()
            .map(|(k, v)| match v {
                Some(v) => protocol::Op::Put(k.as_bytes().to_vec(), v.as_bytes().to_vec()),
                None => protocol::Op::Delete(k.as_bytes().to_vec()),
            })
            .collect())
}

fn range(req_id: u64) -> Request {
    Request::Range {
        req_id,
        lo: protocol::Bound::Unbounded,
        hi: protocol::Bound::Unbounded,
        reverse: false,
        after: None,
        max_entries: 1_000,
    }
}

/// The write states a run reported for `id`, in order.
fn states(rs: &[Reply], id: u64) -> Vec<WriteState> {
    rs.iter()
        .filter_map(|r| match r {
            Reply::WriteState { write_id, state } | Reply::SessionWriteState { write_id, state, .. } if *write_id == id => {
                Some(*state)
            }
            _ => None,
        })
        .collect()
}

/// THE HAPPY PATH: identity, writes, a delete, a range, a subscribed range,
/// a get — each answered, the writes published, the rows as written (the
/// outcome the page and the Shell answered reply for reply).
#[test]
fn the_happy_path_answers_every_request() {
    let mut node = Node::new();
    let mut rig = PageRig::new();
    let first = rig.client_as(&mut node, &Request::Identity);
    assert!(matches!(first.first(), Some(Reply::Identity { head_seq: 0, .. })), "{first:?}");
    let sub = rig.client_as(&mut node, &Request::SubscribeRange { sub_id: 3, lo: protocol::Bound::Unbounded, hi: protocol::Bound::Unbounded });
    assert!(sub.iter().any(|r| matches!(r, Reply::Subscribed { sub_id: 3, .. })), "the subscribed range was not answered: {sub:?}");
    for (id, ops) in [(1, vec![("a", Some("1")), ("b", Some("2"))]), (2, vec![("c", Some("3"))]), (3, vec![("a", None)])] {
        let st = states(&rig.client_as(&mut node, &write(id, &ops)), id);
        assert!(published(&st), "write {id}: {st:?}");
    }
    let rows = page_entries(&rig.client_as(&mut node, &range(10)), 10);
    let kv = |k: &str, v: &str| (k.as_bytes().to_vec(), v.as_bytes().to_vec());
    assert_eq!(rows, Some(vec![kv("b", "2"), kv("c", "3")]), "the range did not return the rows as written");
    let got = rig.client_as(&mut node, &Request::Get { req_id: 11, key: b"b".to_vec() });
    assert!(got.iter().any(|r| matches!(r, Reply::Value { req_id: 11, value: Some(v) } if v == b"2")), "{got:?}");
    let again = rig.client_as(&mut node, &Request::Identity);
    assert!(matches!(again.first(), Some(Reply::Identity { head_seq: 3, .. })), "{again:?}");
    let (_, root) = node.head().expect("published");
    assert_eq!(tree_of(&node, &root), BTreeMap::from([kv("b", "2"), kv("c", "3")]), "the published tree");
}

/// v5 and above: `Unsupported`, naming the version it got.
#[test]
fn a_v5_frame_is_unsupported() {
    let mut node = Node::new();
    let mut rig = PageRig::new();
    let frame = protocol::encode_v5_request(SESSION, 1_000, 1, &Request::Identity).expect("a v5 envelope encodes");
    rig.server.client(&frame);
    let p = rig.run(&mut node);
    assert!(matches!(p.first(), Some(Reply::Unsupported { got: 5, .. })), "{p:?}");
}

fn tree_of(store: &impl Blocks, root: &Cid) -> BTreeMap<Vec<u8>, Vec<u8>> {
    use freenet_prolly::range::{range, read_value, Range};
    use std::ops::Bound;
    let r = Range { lo: Bound::Unbounded, hi: Bound::Unbounded, reverse: false, after: None, max_entries: 100_000, max_bytes: usize::MAX };
    let page = range(store, root, &r).expect("the tree reads");
    page.entries.iter().map(|(k, v)| (k.clone(), read_value(store, *v).expect("value").to_vec())).collect()
}

fn page_entries(rs: &[Reply], id: u64) -> Option<Vec<(Vec<u8>, Vec<u8>)>> {
    rs.iter().find_map(|r| match r {
        Reply::Page { req_id, entries, .. } if *req_id == id => Some(entries.clone()),
        _ => None,
    })
}

fn published(st: &[WriteState]) -> bool {
    st.contains(&WriteState::Published)
}

/// A PUT's answer lost, then the node heals: the page re-sends each put at its
/// deadline, never publishes on a lost answer alone, and publishes once
/// answers flow — the tree holding the write.
#[test]
fn a_lost_put_answer_publishes_once_the_node_answers() {
    let mut node = Node::new();
    let mut rig = PageRig::new();
    rig.faults.lose_first_put_answers = true;
    let t0 = 1_800_000_000u64;
    for r in [Request::Identity, Request::Tick { now: t0 }] {
        rig.client_as(&mut node, &r);
    }
    let p = rig.client_as(&mut node, &write(1, &[("a", Some("1")), ("b", Some("2"))]));
    assert!(published(&states(&p, 1)), "page: {:?}", states(&p, 1));
    let (_, root) = node.head().expect("published");
    let kv = |k: &str, v: &str| (k.as_bytes().to_vec(), v.as_bytes().to_vec());
    assert_eq!(tree_of(&node, &root), BTreeMap::from([kv("a", "1"), kv("b", "2")]));
    assert!(!rig.put_once.is_empty(), "no put answer was lost: the fault never fired");
}

/// RACE PUT's STRAGGLER (COMMIT-LIFE §P gate): every parity block's first
/// answer is lost, so the data acks and the head signs with the parity still
/// out -- SAVED, not BACKED_UP. The page re-sends each straggler on its RTO
/// (stall retry is the standard; nothing stops at the sign), and the write
/// reaches BACKED_UP.
#[test]
fn a_straggler_dropped_after_the_sign_is_re_sent_and_reaches_backed_up() {
    let mut node = Node::new();
    let mut rig = PageRig::new();
    rig.faults.lose_first_parity_answers = true;
    rig.client_as(&mut node, &Request::Identity);
    let rows: Vec<(String, String)> = (0..400).map(|i| (format!("k/{i:04}"), format!("value {i}"))).collect();
    let ops: Vec<(&str, Option<&str>)> = rows.iter().map(|(k, v)| (k.as_str(), Some(v.as_str()))).collect();
    let st = states(&rig.client_as(&mut node, &write(1, &ops)), 1);
    let pub_at = st.iter().position(|s| *s == WriteState::Published);
    let backed_at = st.iter().position(|s| *s == WriteState::ParityComplete);
    assert!(!rig.put_once.is_empty(), "no parity answer was lost: the fault never fired (no parity coded?)");
    assert!(pub_at.is_some(), "the write never published: {st:?}");
    assert!(backed_at.is_some(), "the stragglers were never re-sent to BACKED_UP: {st:?} ({} parity answer(s) lost)", rig.put_once.len());
    assert!(pub_at < backed_at, "BACKED_UP before SAVED: {st:?}");
}

/// A COLD read: the rows are only on the network, and the page's first GET of
/// each block goes unanswered and is fetched again. All 60 rows, as written.
#[test]
fn a_cold_read_with_a_lost_get_returns_every_row() {
    let mut node = Node::new();
    let mut rig = PageRig::new();
    rig.client_as(&mut node, &Request::Identity);
    let rows: Vec<(String, String)> = (0..60).map(|i| (format!("k/{i:03}"), format!("v{i}"))).collect();
    let ops: Vec<(&str, Option<&str>)> = rows.iter().map(|(k, v)| (k.as_str(), Some(v.as_str()))).collect();
    assert!(published(&states(&rig.client_as(&mut node, &write(1, &ops)), 1)));
    // The reader: nothing local.
    let mut rnode = Node::new();
    rnode.network = Some(std::mem::take(&mut node.blocks));
    rnode.register = node.register.clone();
    let mut rr = PageRig::new();
    rr.faults.lose_first_gets = true;
    rr.client_as(&mut rnode, &Request::Identity);
    let p = rr.client_as(&mut rnode, &range(20));
    let want: Vec<(Vec<u8>, Vec<u8>)> = rows.iter().map(|(k, v)| (k.as_bytes().to_vec(), v.as_bytes().to_vec())).collect();
    assert_eq!(page_entries(&p, 20), Some(want), "the page's cold read returned different rows");
    assert!(!rr.got_once.is_empty(), "no GET went unanswered: the fault never fired");
}

/// THE ROOT IS A GROUP OF ONE (sdk#335): a cold reader on another node whose
/// head's ROOT block never answers (measured live: ~60 s, and ~4 min then
/// NotFound) still reads every row -- the head lists the root's parity, and
/// the first of the root's 1 + m to arrive answers.
#[test]
fn a_cold_reader_reads_every_row_through_a_silent_root() {
    let mut node = Node::new();
    let mut rig = PageRig::new();
    rig.client_as(&mut node, &Request::Identity);
    let rows: Vec<(String, String)> = (0..200).map(|i| (format!("k/{i:04}"), format!("v{i}"))).collect();
    let ops: Vec<(&str, Option<&str>)> = rows.iter().map(|(k, v)| (k.as_str(), Some(v.as_str()))).collect();
    assert!(published(&states(&rig.client_as(&mut node, &write(1, &ops)), 1)));
    let (_, root) = node.head().expect("published");
    let listed = node.head_read().and_then(|h| h.mark()).unwrap_or_default();
    assert_eq!(listed.len(), engine::PARITY, "the signed head lists {} root parity ids", listed.len());
    let mut rnode = Node::new();
    rnode.network = Some(std::mem::take(&mut node.blocks));
    rnode.register = node.register.clone();
    let mut rr = PageRig::new();
    rr.silent.insert(root);
    rr.client_as(&mut rnode, &Request::Identity);
    let p = rr.client_as(&mut rnode, &range(20));
    let want: Vec<(Vec<u8>, Vec<u8>)> = rows.iter().map(|(k, v)| (k.as_bytes().to_vec(), v.as_bytes().to_vec())).collect();
    assert_eq!(page_entries(&p, 20), Some(want), "a cold read through a silent root did not return the rows");
    assert!(rr.gets.get(&root).copied().unwrap_or(0) >= 1, "the root was never asked: the silence never fired");
}

/// DEFERRED ON THE WIRE (sdk#350): a `DeferredCommit` frame is held --
/// `Accepted`, no block PUT, no head -- and the next data `Commit` carries it
/// in ONE commit. What a published app's open sends is its defines; a viewer
/// who never writes commits nothing.
#[test]
fn a_deferred_commit_frame_is_held_until_a_data_commit_carries_it() {
    let mut node = Node::new();
    let mut rig = PageRig::new();
    rig.client_as(&mut node, &Request::Identity);
    let deferred = Request::DeferredCommit { write_id: 1, reads: vec![(b"s/notes".to_vec(), protocol::Expect::Any)], ops: vec![protocol::Op::Put(b"s/notes".to_vec(), b"schema".to_vec())] };
    let held = states(&rig.client_as(&mut node, &deferred), 1);
    assert_eq!(held, vec![WriteState::Accepted], "a deferred define was told more than Accepted");
    assert!(node.blocks.is_empty(), "a deferred define put {} block(s)", node.blocks.len());
    assert!(node.head().is_none(), "a deferred define moved the head");
    assert_eq!(rig.server.page.unsaved_writes(), 0, "a held define counts as unsaved");
    let rs = rig.client_as(&mut node, &write(2, &[("r/1", Some("row"))]));
    assert!(published(&states(&rs, 2)), "the data write did not publish: {:?}", states(&rs, 2));
    let (seq, root) = node.head().expect("published");
    assert_eq!(seq, 1, "not ONE commit for the define and the write");
    let kv = |k: &str, v: &str| (k.as_bytes().to_vec(), v.as_bytes().to_vec());
    assert_eq!(tree_of(&node, &root), BTreeMap::from([kv("r/1", "row"), kv("s/notes", "schema")]), "the published tree");
}

/// One commit at a time, and a write while one is in flight is QUEUED
/// (R-b): `Accepted` at once, never `Busy`; it commits when the first lands.
#[test]
fn a_write_behind_a_commit_in_flight_is_queued() {
    let mut node = Node::new();
    let mut rig = PageRig::new();
    rig.client_as(&mut node, &Request::Identity);
    rig.faults.hold = true;
    let p1 = rig.client_as(&mut node, &write(1, &[("a", Some("1"))]));
    assert!(!states(&p1, 1).contains(&WriteState::Busy) && !published(&states(&p1, 1)), "the first write: {:?}", states(&p1, 1));
    let p2 = rig.client_as(&mut node, &write(2, &[("b", Some("2"))]));
    assert_eq!(states(&p2, 2), vec![WriteState::Accepted]);
    assert_eq!(rig.server.busy_told(), 0, "a write was told Busy on the page path");
    // THE PULL API: its fate while it waits is its stage, not a verdict.
    assert_eq!(rig.server.fate(SESSION, 1), Some(Fate::Committing));
    assert_eq!(rig.server.fate(SESSION, 2), Some(Fate::Queued));
    rig.faults.hold = false;
    for op in std::mem::take(&mut rig.held) {
        if let Some(a) = rig.answer(&mut node, op) {
            rig.server.node(a, Ms(rig.now));
        }
    }
    let _ = rig.run(&mut node);
    for w in [1, 2] {
        assert!(matches!(rig.server.fate(SESSION, w), Some(Fate::Published { .. })), "write {w} did not end Published");
        assert_eq!(rig.server.fate(SESSION, w), None, "a terminal fate outlived being read");
    }
}

/// THE QUEUE'S BOUND, TOLD BY NAME (R-b; COMMIT-LIFE K1): past the byte
/// bound the write is told `QueueFull` with the bytes and the limit --
/// backpressure the SDK waits on, and names only at the app's deadline --
/// never `Busy` and never a generic `Failed`; nothing of it is applied. The
/// protocol downgrades it only for a client older than v4.
#[test]
fn a_full_queue_is_told_by_name() {
    let mut node = Node::new();
    let mut rig = PageRig::with(Params { max_queue_bytes: 64, ..Params::default() });
    rig.client_as(&mut node, &Request::Identity);
    rig.faults.hold = true;
    let p = rig.client_as(&mut node, &write(1, &[("k1", Some("v"))]));
    assert_eq!(states(&p, 1), vec![WriteState::Accepted], "the session's first write was not taken");
    let p = rig.client_as(&mut node, &write(2, &[("k2", Some(&"v".repeat(64)))]));
    let told = states(&p, 2);
    assert!(matches!(told.as_slice(), [WriteState::QueueFull { limit: 64, .. }]), "not told by name on the wire: {told:?}");
    assert!(matches!(rig.server.fate(SESSION, 2), Some(Fate::QueueFull { limit: 64, .. })), "the app does not read QueueFull by name");
    assert_eq!(WriteState::QueueFull { bytes: 9, limit: 64 }.for_client(3), WriteState::Failed, "a pre-v4 client is not told the older word that means the same to it");
    assert_eq!(rig.server.queue_load().0, 1, "the refused write joined the queue");
}

/// **A write refused at the door leaves nothing held** (sdk#450): `QueueFull`
/// ends that write id (the SDK makes the write again as a new one), so the
/// server drops its final values like any other ended write. 200 refusals
/// behind one held commit: only the taken write is held. Mutant "QueueFull not
/// an end" -> 201 held -> red (probe B: 75k refusals held 96 MB).
#[test]
fn a_write_refused_queue_full_leaves_nothing_held() {
    let mut node = Node::new();
    let mut rig = PageRig::with(Params { max_queue_bytes: 64, ..Params::default() });
    rig.client_as(&mut node, &Request::Identity);
    rig.faults.hold = true;
    let p = rig.client_as(&mut node, &write(1, &[("k1", Some("v"))]));
    assert_eq!(states(&p, 1), vec![WriteState::Accepted], "THE SETUP: the session's first write was not taken");
    let mut refused = 0;
    for w in 2..202u64 {
        let p = rig.client_as(&mut node, &write(w, &[("k2", Some(&"v".repeat(64)))]));
        refused += usize::from(matches!(states(&p, w).as_slice(), [WriteState::QueueFull { .. }]));
    }
    println!("{refused} writes refused QueueFull; writes held {}", rig.server.writes_held());
    assert_eq!(refused, 200, "THE SETUP: not every write was refused");
    assert_eq!(rig.server.writes_held(), 1, "writes refused at the door are still held");
}

/// A commit that cannot finish is told `Stalled` once `max_accept_age`
/// SECONDS have passed, from the client's protocol ticks — and not before.
#[test]
fn a_held_commit_is_stalled_after_its_age() {
    let mut node = Node::new();
    let mut rig = PageRig::new();
    let t0 = 1_800_000_000u64;
    for r in [Request::Identity, Request::Tick { now: t0 }] {
        rig.client_as(&mut node, &r);
    }
    rig.faults.hold = true;
    rig.client_as(&mut node, &write(1, &[("a", Some("1"))]));
    let p = rig.client_as(&mut node, &Request::Tick { now: t0 + 30 });
    assert!(!states(&p, 1).contains(&WriteState::Stalled), "stalled early");
    let p = rig.client_as(&mut node, &Request::Tick { now: t0 + 65 });
    assert_eq!(states(&p, 1), vec![WriteState::Stalled]);
}

/// Two tabs on ONE key, the second stale: its commit meets the other tab's
/// head (the signer's NotNext), a FOREIGN MOVE. The write is FORCED (a
/// store-level write, `Expect::Any`), so its engine does not re-apply it
/// blind: it falls `Lost`, named (WRITE-PATH ⁷) -- the winner's ledger does
/// not list this page, so it did not land -- and made again it publishes on
/// the winner, the tree holding both. (With both tabs under ONE session this
/// test once read the other tab's `through` as this page's and told the lost
/// write `Published`: a device id is per page load, COMMIT-LIFE ⁵.)
#[test]
fn two_tabs_on_one_key_the_stale_one_loses_then_publishes() {
    let mut node = Node::new();
    let (mut pa, mut pb) = (PageRig::new(), PageRig::new());
    pb.session = SESSION + 1;
    pa.client_as(&mut node, &Request::Identity);
    pb.client_as(&mut node, &Request::Identity);
    assert!(published(&states(&pa.client_as(&mut node, &write(1, &[("a", Some("1"))])), 1)));
    let p = pb.client_as(&mut node, &write(2, &[("b", Some("2"))]));
    assert!(states(&p, 2).contains(&WriteState::Lost), "the forced write whose commit did not land was not told Lost: {:?}", states(&p, 2));
    assert!(!published(&states(&p, 2)), "a write that did not land was told Published: {:?}", states(&p, 2));
    let p = pb.client_as(&mut node, &write(3, &[("b", Some("2"))]));
    assert!(published(&states(&p, 3)), "page {:?}", states(&p, 3));
    let (_, root) = node.head().expect("published");
    let kv = |k: &str, v: &str| (k.as_bytes().to_vec(), v.as_bytes().to_vec());
    assert_eq!(tree_of(&node, &root), BTreeMap::from([kv("a", "1"), kv("b", "2")]), "the tree after the race");
}

/// The signer fails to save its record once: asked again after its backoff,
/// and the write publishes — with the same states as a write whose record
/// saved first time (the Shell's, on main: it signed with no record).
#[test]
fn record_not_saved_once_still_publishes() {
    let run = |fail: bool| {
        let mut node = Node::new();
        let mut rig = PageRig::new();
        rig.faults.record_not_saved_once = fail;
        rig.client_as(&mut node, &Request::Identity);
        states(&rig.client_as(&mut node, &write(1, &[("a", Some("1"))])), 1)
    };
    let (failed, clean) = (run(true), run(false));
    assert!(published(&failed), "{failed:?}");
    assert_eq!(failed, clean, "a record saved late told the write something different");
}

/// TWO ROOTS AT ONE SEQ for this key — another device of the same identity,
/// whose head won the Register's tie-break. The same identity never forks
/// (owner, sdk#225): the page ADOPTS the winner, the write built on the
/// displaced head is handed back `Lost`, sent again it publishes on the
/// winner, and nothing is unusable. (The Shell had no check at all and
/// published over it blind: the divergence this was written to show.)
#[test]
fn a_same_key_displacement_is_adopted_on_the_page() {
    let mut node = Node::new();
    let mut rig = PageRig::new();
    rig.client_as(&mut node, &Request::Identity);
    assert!(published(&states(&rig.client_as(&mut node, &write(1, &[("a", Some("1"))])), 1)));
    let (seq, mine) = node.head().expect("published");
    let key = node.secrets.get(signer::PROVISION).and_then(|b| signer::Provisioned::decode(b)).map(|p| p.key).expect("provisioned");
    // The other device's REAL tree, whose head WINS the equal-seq rule (the
    // lower BLAKE3 of the whole VALUE -- root AND ledger: this page's head
    // carries a ledger, the other's is a bare root): the fork would otherwise
    // be invisible to the signer's read, and a fake root would leave nothing
    // to build on.
    let mine_value = node.head_read().expect("read").value().to_vec();
    let _ = mine;
    let winner = (0u32..512).map(|salt| sibling_root(&mut node, salt)).find(|r| page::beats(r.as_slice(), &mine_value)).expect("some tree wins");
    let other = contract_keys::register::head_state(&node.register_params, &key, seq, &winner).expect("signs");
    node.update(&other);
    assert_eq!(node.head(), Some((seq, winner)), "the winner did not take the register");
    let second = states(&rig.client_as(&mut node, &write(2, &[("b", Some("2"))])), 2);
    assert!(second.contains(&WriteState::Lost), "a write built on the displaced head was not handed back: {second:?}");
    assert_eq!(rig.server.page.published(), (seq, winner), "the winner was not adopted");
    let again = states(&rig.client_as(&mut node, &write(2, &[("b", Some("2"))])), 2);
    assert!(published(&again), "sent again, the write did not publish on the winner: {again:?}");
    assert!(rig.server.page.unusable().is_empty(), "a same-key displacement left the page unusable: {:?}", rig.server.page.unusable());
}

/// A RESTART MID-COMMIT: the first page's UPDATE never lands and the page is
/// gone. The page's next engine is answered AlreadySigned and LANDS it — the
/// signer's record is the third memory (ENGINE-SHAPE §5). (The Shell's next
/// engine LOST that write: the divergence this was written to show.)
#[test]
fn a_restart_mid_commit_the_signer_lands_the_first_commit() {
    let mut node = Node::new();
    let mut first = PageRig::new();
    first.client_as(&mut node, &Request::Identity);
    let frame = protocol::encode_session_request(VERSION, SESSION, &write(1, &[("gone", Some("1"))])).expect("encodes");
    first.server.client(&frame);
    // Answer everything except the UPDATE: the head never lands.
    for _ in 0..50 {
        let ops = first.server.take_ops();
        if ops.is_empty() {
            break;
        }
        for op in ops {
            if matches!(op, Op::Update { .. }) {
                continue;
            }
            if let Some(a) = first.answer(&mut node, op) {
                first.server.node(a, Ms(first.now));
            }
        }
    }
    assert!(node.head().is_none(), "the head landed; this is not a restart mid-commit");
    drop(first);
    // A RESTART is a new page load: a new session (and so a new device id in
    // heads' `through` -- the same one would let the old page's `through`
    // witness the new engine's writes, whose arrival numbers start again).
    let mut next = PageRig::new();
    next.session = SESSION + 100;
    next.client_as(&mut node, &Request::Identity);
    let w2 = write(2, &[("new", Some("2"))]);
    let p = next.client_as(&mut node, &w2);
    let mut rs = p.clone();
    if !published(&states(&p, 2)) {
        rs = next.client_as(&mut node, &write(3, &[("new", Some("2"))]));
    }
    let (_, root) = node.head().expect("published");
    let tree = tree_of(&node, &root);
    assert_eq!(tree.get(&b"gone"[..]).map(Vec::as_slice), Some(&b"1"[..]), "the signer's record did not land the first commit");
    assert_eq!(tree.get(&b"new"[..]).map(Vec::as_slice), Some(&b"2"[..]), "{rs:?}");
}

/// sdk#175's twin: a READ that arrives before the page has read its head is
/// HELD, not answered from the empty tree the engine started on — it is
/// answered, with the rows, once the head is read.
#[test]
fn a_read_before_the_head_is_read_waits_for_it() {
    let mut node = Node::new();
    let mut writer = PageRig::new();
    writer.client_as(&mut node, &Request::Identity);
    assert!(published(&states(&writer.client_as(&mut node, &write(1, &[("a", Some("1")), ("b", Some("2"))])), 1)));
    // A second page: its Identity starts the engine and sends the head read,
    // and the Range goes in BEFORE any answer.
    let mut reader = PageRig::new();
    for r in [Request::Identity, range(42)] {
        let frame = protocol::encode_session_request(VERSION, SESSION, &r).expect("encodes");
        reader.server.client(&frame);
    }
    let early = decode(reader.server.take_replies());
    assert!(page_entries(&early, 42).is_none(), "a read was answered before the head was read: {early:?}");
    let p = reader.run(&mut node);
    assert_eq!(page_entries(&p, 42).map(|e| e.len()), Some(2), "the held read did not answer with the rows: {p:?}");
}

/// ANOTHER DEVICE'S TREE, for real: a fresh page from the same genesis writes
/// THE TIE-BREAK SEARCH (sdk#404): the first `salt` for which `make` builds a head that beats the page's -- the
/// page's own value varies per run (fresh record and device ids), so on some runs a winner is rare. Up to
/// `WINNING_TRIES` salts; running out FAILS naming the search, never overflows a counter.
const WINNING_TRIES: u32 = 1 << 20;

fn winning<T>(case: &str, make: impl FnMut(u32) -> Option<T>) -> T {
    winning_within(case, WINNING_TRIES, make)
}

fn winning_within<T>(case: &str, tries: u32, mut make: impl FnMut(u32) -> Option<T>) -> T {
    (0..tries).find_map(&mut make).unwrap_or_else(|| panic!("{case}: no salt in 0..{tries} built a head that beats the page's"))
}

/// THE CONTROL for the search's bound (sdk#404): one that never finds a winner fails NAMING the search.
#[test]
#[should_panic(expected = "the control: no salt in 0..300 built a head that beats the page's")]
fn a_tie_break_search_that_runs_out_fails_naming_its_case() {
    winning_within("the control", 300, |_| None::<()>);
}

/// `salt`, and its blocks go onto the node; returns its root (never signed —
/// the caller signs a head for it as the other device's signer would).
fn sibling_root(node: &mut Node, salt: u32) -> Cid {
    use engine::{ClientId, Op as WriteOp, WriteId};
    let mut p = Page::new(Params::default(), PutPath::Page);
    p.write(ClientId(9), WriteId(1), vec![(format!("sib/{salt}").into_bytes(), WriteOp::Put(vec![salt as u8; 64]))]);
    for _ in 0..50 {
        for op in p.take_ops() {
            match op {
                Op::ReadHead { .. } => p.answer(Answer::Head { label: page::Label::Head, read: None }, Ms(1)),
                Op::Put { id, bytes } => {
                    node.put(id, &bytes);
                    p.answer(Answer::PutOk(id), Ms(1));
                }
                Op::AskHeld { batch, ids } => p.answer(Answer::Held { batch, present: vec![true; ids.len()] }, Ms(1)),
                Op::Sign { root, .. } => return root,
                other => common::unanswered_op(&other),
            }
        }
    }
    panic!("the sibling page never asked to sign");
}

/// A TAB: `Db` over the page's store (READ-STATE: its reads WALK this rig's
/// engine), the rig and its node LENT to the store for each call, and what
/// crossed counted — the whole page path, native.
struct Tab {
    db: TabDb,
    /// Per write id, how many times it went on the wire (a `Commit` or a `Write`).
    sends: BTreeMap<u64, usize>,
    /// Per write id, the states it was told, in order.
    verdicts: BTreeMap<u64, Vec<protocol::WriteState>>,
    /// Tickets that ended, by id.
    ended: BTreeMap<u64, craftworks_sdk::Ended>,
}

type TabDb = craftworks_sdk::Db<craftworks_sdk::PageStore<Lent>, TabEnv>;

/// The rig while a tab's call runs: the store's host. What the server
/// answered the tab waits in `inbox` for the store; `sent` and `seen` are
/// what crossed, for the tab's counts.
#[derive(Default)]
struct Lent {
    rig: Option<(PageRig, Node)>,
    inbox: Vec<Vec<u8>>,
    sent: Vec<Vec<u8>>,
    seen: Vec<Vec<u8>>,
}

impl page::server::Host for Lent {
    fn with_server<R>(&mut self, f: impl FnOnce(&mut Server) -> R) -> R {
        let Lent { rig: Some((rig, node)), inbox, .. } = self else { panic!("a tab's call runs with the rig lent") };
        let r = f(&mut rig.server);
        answer_ops(rig, node, inbox);
        r
    }
    fn peek<R>(&self, f: impl FnOnce(&Server) -> R) -> R {
        let Lent { rig: Some((rig, _)), .. } = self else { panic!("a tab's call runs with the rig lent") };
        f(&rig.server)
    }
    fn client(&mut self, frame: &[u8]) {
        self.sent.push(frame.to_vec());
        let Lent { rig: Some((rig, node)), inbox, .. } = self else { panic!("a tab's call runs with the rig lent") };
        rig.server.client(frame);
        answer_ops(rig, node, inbox);
    }
    fn take_replies(&mut self) -> Vec<Vec<u8>> {
        let r = std::mem::take(&mut self.inbox);
        self.seen.extend(r.iter().cloned());
        r
    }
}

/// Every op the server has, answered — or HELD, under `faults.hold` — until
/// none is left, the replies kept for the tab. No clock moves here.
fn answer_ops(rig: &mut PageRig, node: &mut Node, inbox: &mut Vec<Vec<u8>>) {
    for _ in 0..2_000 {
        inbox.extend(rig.server.take_replies());
        let ops = rig.server.take_ops();
        if ops.is_empty() {
            return;
        }
        for op in ops {
            if rig.faults.hold || (rig.faults.hold_gets && matches!(op, Op::Get { .. })) {
                rig.held.push(op);
                continue;
            }
            if let Some(a) = rig.answer(node, op) {
                rig.server.node(a, Ms(rig.now));
            }
        }
    }
}

struct TabEnv(u32);
impl craftworks_sdk::Env for TabEnv {
    fn now_ms(&mut self) -> u64 {
        1_750_000_000_000
    }
    fn rand32(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        self.0
    }
}

impl Tab {
    fn open(rig: &mut PageRig, node: &mut Node) -> Tab {
        // Lent from the start: handing the store its host carries out what
        // it already holds.
        let lent = Lent { rig: Some((std::mem::replace(rig, PageRig::new()), std::mem::replace(node, Node::new()))), ..Lent::default() };
        let (store, _clock) = testkit::page_store_over(lent);
        let mut t = Tab { db: craftworks_sdk::Db::new(store, TabEnv(1), [1, 2, 3, 4]), sends: BTreeMap::new(), verdicts: BTreeMap::new(), ended: BTreeMap::new() };
        let (r, n) = t.host().rig.take().expect("lent");
        *rig = r;
        *node = n;
        t.db.store_mut().writes.client.send(&Request::Identity);
        t.pump(rig, node);
        t
    }

    fn host(&mut self) -> &mut Lent {
        self.db.store_mut().host_mut().expect("a tab has its host")
    }

    /// LEND the rig to the tab's store for `f`, then take it back, counting
    /// what crossed.
    fn lend<R>(&mut self, rig: &mut PageRig, node: &mut Node, f: impl FnOnce(&mut TabDb) -> R) -> R {
        let lent = (std::mem::replace(rig, PageRig::new()), std::mem::replace(node, Node::new()));
        self.host().rig = Some(lent);
        let out = f(&mut self.db);
        self.db.store_mut().sync();
        for (id, how) in self.db.store_mut().take_ended() {
            self.ended.insert(id, how);
        }
        let (r, n) = self.host().rig.take().expect("lent");
        *rig = r;
        *node = n;
        let session = self.db.store().writes.client.session();
        let (sent, seen) = (std::mem::take(&mut self.host().sent), std::mem::take(&mut self.host().seen));
        for f in sent {
            if let protocol::Incoming::Ok(env) = protocol::decode_request(&f) {
                if let Request::Write { write_id, .. } | Request::Commit { write_id, .. } = env.body {
                    *self.sends.entry(write_id).or_default() += 1;
                }
            }
        }
        for r in seen {
            if let Ok(Reply::SessionWriteState { session: s, write_id, state }) = protocol::decode_reply(&r) {
                if Some(s) == session {
                    self.verdicts.entry(write_id).or_default().push(state);
                }
            }
        }
        out
    }

    /// Run until nothing moves: the store's frames to the server, the ops to
    /// the node, the replies back; when only silence is left and the page is
    /// waiting, let its next deadline pass, as the page's own tick would.
    fn pump(&mut self, rig: &mut PageRig, node: &mut Node) {
        self.lend(rig, node, |db| {
            for _ in 0..400 {
                db.store_mut().sync();
                let Lent { rig: Some((rig, node)), inbox, .. } = db.store_mut().host_mut().expect("host") else { panic!("lent") };
                answer_ops(rig, node, inbox);
                if !inbox.is_empty() {
                    continue;
                }
                if rig.faults.hold || !rig.server.page.waiting() {
                    break;
                }
                rig.now = rig.server.page.next_due().map_or(rig.now + 1, |d| d.0.max(rig.now + 1));
                rig.server.tick(Ms(rig.now));
            }
        });
    }

    /// A `Db` call as a page makes it (`engine-db.js`'s `once`): a walk that
    /// stops on blocks waits on its ticket, then RESUMES at that ticket's
    /// root; bounded as the page is (MAX_HOPS).
    fn call<T>(&mut self, rig: &mut PageRig, node: &mut Node, f: impl Fn(&mut TabDb) -> Result<T, craftworks_sdk::DbError>) -> Result<T, craftworks_sdk::DbError> {
        for _ in 0..8 {
            match self.lend(rig, node, |db| {
                let r = f(db);
                db.store_mut().decide(r)
            }) {
                craftworks_sdk::Outcome::Done(v) => return Ok(v),
                craftworks_sdk::Outcome::Told(e) => return Err(e),
                craftworks_sdk::Outcome::Wait(e, t) => {
                    let mut how = self.ended.remove(&t);
                    for _ in 0..50 {
                        if how.is_some() {
                            break;
                        }
                        self.pump(rig, node);
                        how = self.ended.remove(&t);
                    }
                    match how {
                        Some(craftworks_sdk::Ended::Loaded) => self.db.store_mut().resume(t),
                        // As `engine-db.js`: the whole chain again, unpinned, at the head.
                        Some(craftworks_sdk::Ended::Superseded) => {}
                        _ => return Err(e),
                    }
                }
            }
        }
        panic!("a tab's call took more than eight hops");
    }

    /// One key as the tab reads it (a walk of its tree).
    fn get(&mut self, rig: &mut PageRig, node: &mut Node, key: &[u8]) -> Option<Vec<u8>> {
        self.call(rig, node, |db| {
            craftworks_sdk::store::Reads::get(db.store_mut(), key).map_err(|e| match e {
                craftworks_sdk::store::StoreError::NotLoaded => craftworks_sdk::DbError::NotLoaded { lo: key.to_vec(), hi: key.to_vec() },
                other => craftworks_sdk::DbError::Refused(other.to_string()),
            })
        })
        .expect("the tab reads the key")
    }
}

fn tab_schema() -> craftworks_sdk::Schema {
    serde_json::from_value(serde_json::json!({ "type": "T", "fields": [{ "name": "title", "kind": "text", "required": true }] })).expect("a schema")
}

/// M2 on the PAGE path (sdk#148's SDK half): a tab's `update` whose READ is
/// overtaken before it lands is a stale `Commit`. Reads walk the warm tree
/// (READ-STATE), so the read is stale only if it is made and then WAITS: the
/// engine is busy with another session's commit and QUEUES it (R-b), and
/// another device changes the record meanwhile ([`behind`]). The engine
/// applies NOTHING of it; the tab is told `Conflict` with `Conflicted`
/// naming the key; the write is rolled back, the conflict surfaced to the
/// app, and the write NEVER re-sent as itself (sent once, then the verdict). The other session's value stands, and it is
/// what the tab reads. Control: the same wait with nothing changed publishes
/// the update, and nothing is conflicted.
#[test]
fn a_stale_update_is_told_conflict_rolled_back_and_never_re_sent() {
    for interfere in [false, true] {
        let mut node = Node::new();
        let mut rig = PageRig::new();
        let mut tab = Tab::open(&mut rig, &mut node);
        tab.call(&mut rig, &mut node, |db| db.define("t", &tab_schema())).expect("define");
        tab.pump(&mut rig, &mut node);
        let rec = tab.call(&mut rig, &mut node, |db| db.put("t", &serde_json::json!({ "title": "first" }).as_object().unwrap().clone())).expect("put");
        tab.pump(&mut rig, &mut node);
        let loc = craftworks_sdk::id::loc_from_hex(&rec.id).expect("a record's id parses");
        let key = craftworks_sdk::db::record_key("t", loc);
        let w = tab.db.store().writes.next_write_id();
        let theirs = if interfere { vec![protocol::Op::Put(key.clone(), b"elsewhere".to_vec())] } else { Vec::new() };
        behind(&mut tab, &mut rig, &mut node, 1, theirs, |db| db.update("t", loc, serde_json::json!({ "title": "second" }).as_object().unwrap()));
        for _ in 0..5 {
            rig.now += 30_000;
            rig.server.tick(Ms(rig.now));
            tab.lend(&mut rig, &mut node, |db| db.store_mut().sync());
            tab.pump(&mut rig, &mut node);
        }
        let conflicts = tab.db.store_mut().writes.take_conflicts();
        assert_eq!(tab.sends.get(&w), Some(&1), "interfere {interfere}: the write went on the wire {:?} times (once: queued, then judged where it lands, R-b)", tab.sends.get(&w));
        if interfere {
            assert_eq!(tab.verdicts.get(&w).and_then(|v| v.last()), Some(&protocol::WriteState::Conflict), "not told Conflict: {:?}", tab.verdicts.get(&w));
            assert_eq!(conflicts.len(), 1, "a stale update was not told as ONE conflict: {conflicts:?}");
            assert_eq!((conflicts[0].write_id, &conflicts[0].key), (w, &key));
            assert_eq!(tab.lend(&mut rig, &mut node, |db| craftworks_sdk::store::Reads::row_state(db.store_mut(), &key)), craftworks_sdk::store::RowState::RolledBack);
            assert_eq!(tab.get(&mut rig, &mut node, &key), Some(b"elsewhere".to_vec()), "the tab does not read the tree's value at the conflicted key");
        } else {
            assert!(conflicts.is_empty(), "nothing moved and yet a conflict: {conflicts:?}");
            // Saved: CLEAN, or BACKED_UP once no parity is owed (sdk#294).
            let st = tab.lend(&mut rig, &mut node, |db| craftworks_sdk::store::Reads::row_state(db.store_mut(), &key));
            assert!(st.is_settled(), "the unconflicted update did not publish: {st:?}");
        }
    }
}

/// THE SCHEMA KEY (the architect's #1 on the slice): another session changes
/// the domain's SCHEMA while this tab's update waits ([`behind`]). The update
/// read the record (unchanged) and the schema (stale): it conflicts ON THE
/// SCHEMA KEY, which no rolled-back write names — and the tab's next read of
/// it is the tree's, so the next write reads the schema fresh instead of
/// conflicting again for ever.
#[test]
fn a_schema_changed_elsewhere_conflicts_on_the_schema_key() {
    let mut node = Node::new();
    let mut rig = PageRig::new();
    let mut tab = Tab::open(&mut rig, &mut node);
    tab.call(&mut rig, &mut node, |db| db.define("t", &tab_schema())).expect("define");
    tab.pump(&mut rig, &mut node);
    let rec = tab.call(&mut rig, &mut node, |db| db.put("t", &serde_json::json!({ "title": "first" }).as_object().unwrap().clone())).expect("put");
    tab.pump(&mut rig, &mut node);
    let loc = craftworks_sdk::id::loc_from_hex(&rec.id).expect("an id");
    let schema_key = schema_key_of("t");
    let wider: craftworks_sdk::Schema = serde_json::from_value(serde_json::json!({ "type": "T", "fields": [
        { "name": "title", "kind": "text", "required": true },
        { "name": "note", "kind": "text" }
    ] })).expect("a schema");
    let w = tab.db.store().writes.next_write_id();
    let theirs = vec![protocol::Op::Put(schema_key.clone(), serde_json::to_vec(&wider).unwrap())];
    behind(&mut tab, &mut rig, &mut node, 1, theirs, |db| db.update("t", loc, serde_json::json!({ "title": "second" }).as_object().unwrap()));
    tab.lend(&mut rig, &mut node, |db| db.store_mut().sync());
    tab.pump(&mut rig, &mut node);
    let conflicts = tab.db.store_mut().writes.take_conflicts();
    assert_eq!(conflicts.iter().map(|c| (c.write_id, c.key.clone())).collect::<Vec<_>>(), vec![(w, schema_key.clone())], "not a conflict on the SCHEMA key");
    assert_eq!(tab.get(&mut rig, &mut node, &schema_key), Some(serde_json::to_vec(&wider).unwrap()), "the tab does not read the schema the tree holds");
    // And the fallen write's OWN key (the record: not the key that conflicted)
    // reads the tree's value, not the unsaved one.
    let key = craftworks_sdk::db::record_key("t", loc);
    let first = tab.get(&mut rig, &mut node, &key).expect("the record");
    let d = craftworks_sdk::record::decode(&wider, &first).expect("decodes");
    assert_eq!(d.fields.get("title"), Some(&serde_json::json!("first")), "the rolled-back record shows its unsaved value");
}

/// ANOTHER DEVICE'S TREE holding exactly `entries`, built by a fresh page from
/// the genesis; its blocks go onto the node. Returns its root.
fn device_tree(node: &mut Node, entries: &[(Vec<u8>, Vec<u8>)]) -> Cid {
    use engine::{ClientId, Op as WriteOp, WriteId};
    let mut p = Page::new(Params::default(), PutPath::Page);
    p.write(ClientId(9), WriteId(1), entries.iter().map(|(k, v)| (k.clone(), WriteOp::Put(v.clone()))).collect());
    for _ in 0..50 {
        for op in p.take_ops() {
            match op {
                Op::ReadHead { .. } => p.answer(Answer::Head { label: page::Label::Head, read: None }, Ms(1)),
                Op::Put { id, bytes } => {
                    node.put(id, &bytes);
                    p.answer(Answer::PutOk(id), Ms(1));
                }
                Op::AskHeld { batch, ids } => p.answer(Answer::Held { batch, present: vec![true; ids.len()] }, Ms(1)),
                Op::Sign { root, .. } => return root,
                other => common::unanswered_op(&other),
            }
        }
    }
    panic!("the other device's page never asked to sign");
}

/// sdk#225b through the page path: a tab publishes a record; ANOTHER DEVICE of
/// the same identity lands a head on the register (signed with the same key, as
/// two devices are until #226); the tab reads it on the node's hint and adopts
/// it (#225a). What it is TOLD depends on the winner's `prev` and contents:
/// * a same-seq winner that lacks the record → `Superseded` naming the record's
///   key, and the tab's read of it is the winner's tree (absent);
/// * a winner whose prev IS the tab's tip (they built on it) → nothing;
/// * a winner two commits deep whose tree holds exactly the tab's value → nothing
///   (the architect's false-Superseded case: probed, equal, kept).
#[test]
fn a_displaced_tip_is_told_superseded_only_for_the_keys_the_winner_replaced() {
    use signer_proto::head::{value, Ledger};
    for case in ["same-seq, record lost", "built on the tip", "two deep, record kept"] {
        let mut node = Node::new();
        let mut rig = PageRig::new();
        let mut tab = Tab::open(&mut rig, &mut node);
        tab.call(&mut rig, &mut node, |db| db.define("t", &tab_schema())).expect("define");
        tab.pump(&mut rig, &mut node);
        let rec = tab.call(&mut rig, &mut node, |db| db.put("t", &serde_json::json!({ "title": "mine" }).as_object().unwrap().clone())).expect("put");
        tab.pump(&mut rig, &mut node);
        let loc = craftworks_sdk::id::loc_from_hex(&rec.id).expect("an id");
        let key = craftworks_sdk::db::record_key("t", loc);
        let (tip_seq, tip_root) = node.head().expect("the tab published");
        let tip_tree = node.tree(&tip_root).expect("the tip is whole");
        let key_of = node.secrets.get(signer::PROVISION).and_then(|b| signer::Provisioned::decode(b)).map(|p| p.key).expect("provisioned");
        let (seq, root, prev) = match case {
            "same-seq, record lost" => {
                // The other device's commit at the tip's seq, built on the
                // tip's prev, WITHOUT the record — and winning the tie-break.
                let hr = node.head_read().expect("a head");
                let base = hr.prev().expect("the tip has a prev");
                winning(&format!("{}:{}", file!(), line!()), |salt| {
                    let r = device_tree(&mut node, &[(b"other".to_vec(), salt.to_le_bytes().to_vec())]);
                    let v = value(&r, &Ledger { prev: Some(signer_proto::Head { seq: base.0, root: base.1 }), ..Ledger::default() });
                    if page::beats(&v, hr.value()) {
                        return Some((tip_seq, r, Some(base)));
                    }
                    None
                })
            }
            "built on the tip" => (tip_seq + 1, device_tree(&mut node, &[(b"other".to_vec(), b"x".to_vec())]), Some((tip_seq, tip_root))),
            _ => {
                // Two deep: a head at tip+2 whose prev is NOT the tip, but whose
                // tree holds the tab's record exactly (they built on it twice).
                let mut entries: Vec<(Vec<u8>, Vec<u8>)> = tip_tree.into_iter().collect();
                entries.push((b"other".to_vec(), b"y".to_vec()));
                (tip_seq + 2, device_tree(&mut node, &entries), Some((tip_seq + 1, [0x42; 32])))
            }
        };
        let v = value(&root, &Ledger { prev: prev.map(|(s, r)| signer_proto::Head { seq: s, root: r }), ..Ledger::default() });
        let st = contract_keys::register::head_state(&node.register_params, &key_of, seq, &v).expect("signs");
        node.update(&st);
        assert_eq!(node.head().map(|h| h.0), Some(seq), "{case}: the other device's head did not take the register");
        rig.server.head_hint();
        tab.pump(&mut rig, &mut node);
        for _ in 0..10 {
            rig.now += 1_000;
            rig.server.tick(Ms(rig.now));
            tab.pump(&mut rig, &mut node);
        }
        assert_eq!(rig.server.page.published().0, seq, "{case}: the tab did not adopt the other device's head");
        let told = tab.db.store_mut().writes.take_superseded();
        match case {
            "same-seq, record lost" => {
                assert_eq!(told.len(), 1, "{case}: {told:?}");
                assert_eq!(told[0].keys, vec![key.clone()], "{case}: the superseded rows are not the record");
                assert_eq!(tab.get(&mut rig, &mut node, &key), None, "{case}: the replaced row still shows the tab's value, not the winner's (which lacks it)");
            }
            _ => assert!(told.is_empty(), "{case}: told Superseded for a value the winner holds: {told:?}"),
        }
    }
}

/// A TIP OF TWO WRITES (main's condition on Z5): a commit carries ONE write —
/// the engine refuses a second with `Busy` — so a tip holds more than one only
/// through a NO-OP write Published at the same head (sdk#160), whose value at
/// every key is the tree's there. Here the second write puts the SAME bytes at
/// the record's key; it publishes at the same head, joining the tip. A
/// same-seq winner without the record then tells the key's LAST writer
/// Superseded -- never the earlier one, whose key a later write of its own
/// overwrote (COMMIT-LIFE K9 §2: it lost to its own app's order).
#[test]
fn a_tip_of_two_writes_to_one_key_tells_both_superseded() {
    use signer_proto::head::{value, Ledger};
    let mut node = Node::new();
    let mut rig = PageRig::new();
    let mut tab = Tab::open(&mut rig, &mut node);
    tab.call(&mut rig, &mut node, |db| db.define("t", &tab_schema())).expect("define");
    tab.pump(&mut rig, &mut node);
    let rec = tab.call(&mut rig, &mut node, |db| db.put("t", &serde_json::json!({ "title": "mine" }).as_object().unwrap().clone())).expect("put");
    let w1 = tab.db.store().writes.next_write_id() - 1;
    tab.pump(&mut rig, &mut node);
    let loc = craftworks_sdk::id::loc_from_hex(&rec.id).expect("an id");
    let key = craftworks_sdk::db::record_key("t", loc);
    let head = node.head().expect("published");
    let bytes = node.tree(&head.1).expect("whole").get(&key).cloned().expect("the record is in the tip");
    // The same bytes again: a no-op, Published at the same head.
    let w2 = tab.db.store().writes.next_write_id();
    tab.lend(&mut rig, &mut node, |db| craftworks_sdk::store::Store::apply_commit(
        db.store_mut(),
        &[(key.clone(), protocol::Expect::Value(craftworks_sdk::read_token::read_token(&bytes)))],
        &[(key.clone(), craftworks_sdk::store::Edit::Put(bytes.clone()))],
    ))
    .expect("the store took it");
    tab.pump(&mut rig, &mut node);
    assert!(tab.verdicts.get(&w2).is_some_and(|v| v.contains(&protocol::WriteState::Published)), "the second write did not publish: {:?}", tab.verdicts.get(&w2));
    assert_eq!(node.head(), Some(head), "the second write moved the head: it was not a no-op, the tip is one write");
    // Displaced at its own seq by a winner without the record.
    let hr = node.head_read().expect("a head");
    let base = hr.prev().expect("a prev");
    let key_of = node.secrets.get(signer::PROVISION).and_then(|b| signer::Provisioned::decode(b)).map(|p| p.key).expect("provisioned");
    let st = winning(&format!("{}:{}", file!(), line!()), |salt| {
        let r = device_tree(&mut node, &[(b"other".to_vec(), salt.to_le_bytes().to_vec())]);
        let v = value(&r, &Ledger { prev: Some(signer_proto::Head { seq: base.0, root: base.1 }), ..Ledger::default() });
        if page::beats(&v, hr.value()) {
            return Some(contract_keys::register::head_state(&node.register_params, &key_of, head.0, &v).expect("signs"));
        }
        None
    });
    node.update(&st);
    rig.server.head_hint();
    tab.pump(&mut rig, &mut node);
    let mut told: Vec<(u64, Vec<Vec<u8>>)> = tab.db.store_mut().writes.take_superseded().into_iter().map(|s| (s.write_id, s.keys)).collect();
    told.sort();
    let _ = w1;
    assert_eq!(told, vec![(w2, vec![key.clone()])], "the key's last writer was not the one told (or the earlier one was told too)");
}

/// sdk#225b part 2, THE MERGE (cell B): another device's commit at the tip's
/// seq, signed from the SAME base P, whose tree is exactly P plus THEIR
/// changes. Per key: changed only here (and its write's reads unchanged there)
/// → KEPT, re-applied on the winner as a Commit with reads; changed there too
/// → SUPERSEDED, told.
/// * c1 they added an unrelated key → nothing told; the merge publishes on top
///   of the winner, and the final tree holds BOTH their key and this page's
///   record;
/// * c2 they changed the record itself → the record is Superseded, and their
///   value stands.
#[test]
fn a_same_seq_race_is_merged_key_by_key() {
    use signer_proto::head::{value, Ledger};
    for case in ["one-sided mine is kept", "both changed the record", "both changed the record, behind a long delta"] {
        let mut node = Node::new();
        let mut rig = PageRig::new();
        let mut tab = Tab::open(&mut rig, &mut node);
        tab.call(&mut rig, &mut node, |db| db.define("t", &tab_schema())).expect("define");
        tab.pump(&mut rig, &mut node);
        let rec = tab.call(&mut rig, &mut node, |db| db.put("t", &serde_json::json!({ "title": "mine" }).as_object().unwrap().clone())).expect("put");
        tab.pump(&mut rig, &mut node);
        let loc = craftworks_sdk::id::loc_from_hex(&rec.id).expect("an id");
        let key = craftworks_sdk::db::record_key("t", loc);
        // The long-delta case's tip is ONE write of TWO keys: the record again
        // (which the winner changes, behind the cut) and a key only this page
        // writes. Main's point: a key wrongly kept by a cut delta sinks the
        // whole merge write, so the truly one-sided key must end PUBLISHED —
        // an outcome, not only a message.
        let extra = vec![0x02, b'x'];
        if case.ends_with("behind a long delta") {
            let (_, r) = node.head().expect("published");
            let now_rec = node.tree(&r).expect("whole").get(&key).cloned().expect("the record");
            // A write that declared NO reads (a store-level batch): the
            // stale-premise rule cannot cover it, so only the COMPLETE delta
            // decides which of its keys the winner changed — and a cut read as
            // complete would keep the record and write this page's value over
            // the winner's, the merge write carrying no read to refuse it.
            let _ = now_rec;
            tab.lend(&mut rig, &mut node, |db| craftworks_sdk::store::Store::apply_batch(
                db.store_mut(),
                &[
                    (key.clone(), craftworks_sdk::store::Edit::Put(b"mine, again".to_vec())),
                    (extra.clone(), craftworks_sdk::store::Edit::Put(b"only mine".to_vec())),
                ],
            ))
            .expect("the store took it");
            tab.pump(&mut rig, &mut node);
        }
        let (tip_seq, tip_root) = node.head().expect("published");
        let mine = node.tree(&tip_root).expect("whole").get(&key).cloned().expect("the record");
        let hr = node.head_read().expect("a head");
        let base = hr.prev().expect("the tip has a prev");
        let base_tree = node.tree(&base.1).expect("the base is whole");
        let key_of = node.secrets.get(signer::PROVISION).and_then(|b| signer::Provisioned::decode(b)).map(|p| p.key).expect("provisioned");
        let theirs_record = b"their bytes for the record".to_vec();
        let st = winning(&format!("{}:{}", file!(), line!()), |salt| {
            let mut entries: Vec<(Vec<u8>, Vec<u8>)> = base_tree.clone().into_iter().collect();
            entries.push((b"other".to_vec(), salt.to_le_bytes().to_vec()));
            if case.starts_with("both changed the record") {
                entries.push((key.clone(), theirs_record.clone()));
            }
            // 300 changes of theirs that sort BEFORE the record: longer than
            // one delta page, so the record's change comes after a CUT — a cut
            // delta is unknown, never "not changed there" (the architect's #3).
            if case.ends_with("behind a long delta") {
                for i in 0..300u16 {
                    let mut k = vec![0x00, b'f'];
                    k.extend_from_slice(&i.to_be_bytes());
                    entries.push((k, vec![1]));
                }
            }
            let r = device_tree(&mut node, &entries);
            let v = value(&r, &Ledger { prev: Some(signer_proto::Head { seq: base.0, root: base.1 }), ..Ledger::default() });
            if page::beats(&v, hr.value()) {
                return Some(contract_keys::register::head_state(&node.register_params, &key_of, tip_seq, &v).expect("signs"));
            }
            None
        });
        node.update(&st);
        rig.server.head_hint();
        tab.pump(&mut rig, &mut node);
        for _ in 0..20 {
            rig.now += 1_000;
            rig.server.tick(Ms(rig.now));
            tab.pump(&mut rig, &mut node);
        }
        let told = tab.db.store_mut().writes.take_superseded();
        let (fseq, froot) = node.head().expect("a head");
        let fin = node.tree(&froot).expect("the final tree is whole");
        match case {
            "one-sided mine is kept" => {
                assert!(told.is_empty(), "{case}: told Superseded for a key only this page changed: {told:?}");
                assert_eq!(fseq, tip_seq + 1, "{case}: no merge commit on top of the winner");
                assert_eq!(fin.get(&key), Some(&mine), "{case}: this page's record did not survive the merge");
                assert!(fin.contains_key(b"other".as_slice()), "{case}: their change did not survive the merge");
            }
            _ => {
                assert_eq!(told.iter().map(|s| s.keys.clone()).collect::<Vec<_>>(), vec![vec![key.clone()]], "{case}: {told:?}");
                assert_eq!(fin.get(&key), Some(&theirs_record), "{case}: their value at the record did not stand");
                if case.ends_with("behind a long delta") {
                    assert_eq!(fin.get(&extra).map(|v| v.as_slice()), Some(&b"only mine"[..]), "{case}: the truly one-sided key was lost with the record: the merge write fell WHOLE");
                    assert_eq!(fseq, tip_seq + 1, "{case}: no merge commit on top of the winner");
                }
            }
        }
    }
}

/// Drive the rig, answering every op EXCEPT block GETs, which are held (a cold
/// node) — from now until the caller clears `faults.hold_gets` — and handed
/// to `held`. Returns when nothing else moves.
fn run_holding_gets(tab: &mut Tab, rig: &mut PageRig, node: &mut Node, held: &mut Vec<Op>) {
    rig.faults.hold_gets = true;
    tab.pump(rig, node);
    held.append(&mut rig.held);
}

/// THE MERGE WRITE IS A COMMIT WITH READS, NEVER BLIND (the architect's #2 on
/// the table): a merge decided against the winner x can LAND on a newer head,
/// when the engine was busy and the page adopted another device's x' in
/// between. Its reads — the kept writes' own, true at x — are compared where
/// it lands: x' changed the record, so the merge conflicts and the record is
/// SUPERSEDED, told; their x' value stands. A blind merge would have written
/// this page's value over it.
#[test]
fn a_merge_that_lands_on_a_newer_head_is_judged_by_its_reads_there() {
    use signer_proto::head::{value, Ledger};
    let mut node = Node::new();
    let mut rig = PageRig::new();
    let mut tab = Tab::open(&mut rig, &mut node);
    tab.call(&mut rig, &mut node, |db| db.define("t", &tab_schema())).expect("define");
    tab.pump(&mut rig, &mut node);
    let rec = tab.call(&mut rig, &mut node, |db| db.put("t", &serde_json::json!({ "title": "mine" }).as_object().unwrap().clone())).expect("put");
    tab.pump(&mut rig, &mut node);
    let loc = craftworks_sdk::id::loc_from_hex(&rec.id).expect("an id");
    let key = craftworks_sdk::db::record_key("t", loc);
    let (tip_seq, _) = node.head().expect("published");
    let hr = node.head_read().expect("a head");
    let base = hr.prev().expect("a prev");
    let base_tree = node.tree(&base.1).expect("whole");
    let key_of = node.secrets.get(signer::PROVISION).and_then(|b| signer::Provisioned::decode(b)).map(|p| p.key).expect("provisioned");
    // x: the same-seq winner, P plus an unrelated key (so the record is one-sided mine).
    let (x_root, x_state) = winning(&format!("{}:{}", file!(), line!()), |salt| {
        let mut e: Vec<(Vec<u8>, Vec<u8>)> = base_tree.clone().into_iter().collect();
        e.push((b"other".to_vec(), salt.to_le_bytes().to_vec()));
        let r = device_tree(&mut node, &e);
        let v = value(&r, &Ledger { prev: Some(signer_proto::Head { seq: base.0, root: base.1 }), ..Ledger::default() });
        if page::beats(&v, hr.value()) {
            return Some((r, contract_keys::register::head_state(&node.register_params, &key_of, tip_seq, &v).expect("signs")));
        }
        None
    });
    node.update(&x_state);
    // The page learns of x, adopts it and starts the merge; x's blocks are cold,
    // so the merge's delta waits on GETs (held).
    let mut held = Vec::new();
    rig.server.head_hint();
    run_holding_gets(&mut tab, &mut rig, &mut node, &mut held);
    assert_eq!(rig.server.page.published().1, x_root, "the page did not adopt x");
    assert!(!held.is_empty(), "the merge's delta did not need a fetch: the window this test is about does not exist");
    // A new write of this tab, parked by the ENGINE on the same cold blocks:
    // the engine is busy. A store-level batch, so it reaches the engine at
    // once — a `Db` put would first READ its schema on x, which is cold, and
    // wait for it (READ-STATE: a read walks the tree the page stands on).
    tab.lend(&mut rig, &mut node, |db| craftworks_sdk::store::Store::apply_batch(db.store_mut(), &[(b"later".to_vec(), craftworks_sdk::store::Edit::Put(b"l".to_vec()))])).expect("a later write");
    run_holding_gets(&mut tab, &mut rig, &mut node, &mut held);
    // x': ANOTHER head of the other device, on top of x, that changes the record.
    let x_tree = node.tree(&x_root).expect("whole");
    let theirs = b"their record, on x-prime".to_vec();
    let mut e: Vec<(Vec<u8>, Vec<u8>)> = x_tree.into_iter().collect();
    e.retain(|(k, _)| *k != key);
    e.push((key.clone(), theirs.clone()));
    let x2 = device_tree(&mut node, &e);
    let v2 = value(&x2, &Ledger { prev: Some(signer_proto::Head { seq: tip_seq, root: x_root }), ..Ledger::default() });
    node.update(&contract_keys::register::head_state(&node.register_params, &key_of, tip_seq + 1, &v2).expect("signs"));
    // Release the GETs: the delta completes, the merge write meets a busy engine,
    // the later write's sign meets x' and the page adopts it; the merge re-sends ON x'.
    rig.faults.hold_gets = false;
    for op in held.drain(..) {
        if let Some(a) = rig.answer(&mut node, op) {
            rig.server.node(a, Ms(rig.now));
        }
    }
    tab.pump(&mut rig, &mut node);
    for _ in 0..20 {
        rig.now += 1_000;
        rig.server.tick(Ms(rig.now));
        tab.pump(&mut rig, &mut node);
    }
    let fin = node.tree(&node.head().expect("a head").1).expect("whole");
    assert_eq!(fin.get(&key), Some(&theirs), "the merge wrote this page's value over x-prime's: it went blind");
    let told: Vec<Vec<Vec<u8>>> = tab.db.store_mut().writes.take_superseded().into_iter().map(|s| s.keys).collect();
    assert!(told.iter().any(|k| k.contains(&key)), "the record was not told Superseded: {told:?}");
}

// ---- sdk#143/#144: a conflicted update or define is RE-RUN on the new base ----

fn rerun_schema() -> craftworks_sdk::Schema {
    serde_json::from_value(serde_json::json!({ "type": "T", "fields": [
        { "name": "title", "kind": "text", "required": true },
        { "name": "body", "kind": "text" },
        { "name": "note", "kind": "text" }
    ] }))
    .expect("a schema")
}

fn obj(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    v.as_object().expect("an object").clone()
}

fn schema_key_of(domain: &str) -> Vec<u8> {
    let mut k = vec![0u8];
    k.extend_from_slice(b"schema\0");
    k.extend_from_slice(domain.as_bytes());
    k
}

/// ANOTHER DEVICE of this identity lands `ops` on the register: a head at
/// the tip's seq + 1, built on the tip, holding the tip's tree with `ops`
/// applied. The page does not hear of it until its own commit's sign meets it
/// (a FOREIGN MOVE): then it adopts it and re-applies its queue on it, in
/// order (R-b). Writes of this page's own sessions share one queue and can
/// never be stale against each other -- only a foreign head makes a stale
/// write.
fn elsewhere(_rig: &mut PageRig, node: &mut Node, _write_id: u64, ops: Vec<protocol::Op>) {
    use signer_proto::head::{value, Ledger};
    let (tip_seq, tip_root) = node.head().expect("a head to build on");
    let mut tree = node.tree(&tip_root).expect("the tip is whole");
    for op in ops {
        match op {
            protocol::Op::Put(k, v) => {
                tree.insert(k, v);
            }
            protocol::Op::Delete(k) => {
                tree.remove(&k);
            }
        }
    }
    let entries: Vec<(Vec<u8>, Vec<u8>)> = tree.into_iter().collect();
    let root = device_tree(node, &entries);
    let key_of = node.secrets.get(signer::PROVISION).and_then(|b| signer::Provisioned::decode(b)).map(|p| p.key).expect("provisioned");
    let v = value(&root, &Ledger { prev: Some(signer_proto::Head { seq: tip_seq, root: tip_root }), ..Ledger::default() });
    let st = contract_keys::register::head_state(&node.register_params, &key_of, tip_seq + 1, &v).expect("signs");
    node.update(&st);
    assert_eq!(node.head().map(|h| h.0), Some(tip_seq + 1), "the other device's head did not take the register");
}

/// A COMMIT IN FLIGHT: another session's write of a key nobody reads, with
/// the node's answers HELD, so the engine is busy until [`release`].
fn commit_in_flight(rig: &mut PageRig, node: &mut Node, write_id: u64) {
    rig.faults.hold = true;
    let filler = protocol::encode_session_request(VERSION, SESSION + 1, &Request::forced_write(write_id, vec![protocol::Op::Put(b"\x00filler".to_vec(), write_id.to_be_bytes().to_vec())])).expect("encodes");
    rig.server.client(&filler);
    let _ = rig.run(node);
    assert!(!rig.held.is_empty(), "the filler's commit went nowhere: nothing is in flight");
}

/// The node answers what it held. What follows -- the commit landing, and
/// the queue behind it committing (R-b) -- runs on the tab's next pump, so the
/// replies it makes reach the tab: a queued write's verdicts are made in a
/// node answer's step, not in a call of the tab's.
fn release(rig: &mut PageRig, node: &mut Node) {
    rig.faults.hold = false;
    for op in std::mem::take(&mut rig.held) {
        if let Some(a) = rig.answer(node, op) {
            rig.server.node(a, Ms(rig.now));
        }
    }
}

/// A STALE WRITE, as one can still happen (R-b): the tab's write `make`
/// READS now -- while another session's commit is in flight, so it is QUEUED
/// behind it -- then ANOTHER DEVICE's `theirs` lands on the register, and
/// when the commit in flight meets it the page adopts it and re-applies the
/// queue there: the tab's write is judged where it lands. (A write made
/// AFTER theirs would read theirs: a premise is the tree's, never a copy's.)
/// `theirs` empty is the control: the same wait, nothing changed.
fn behind<T: std::fmt::Debug>(tab: &mut Tab, rig: &mut PageRig, node: &mut Node, write_id: u64, theirs: Vec<protocol::Op>, make: impl Fn(&mut TabDb) -> Result<T, craftworks_sdk::DbError>) -> T {
    commit_in_flight(rig, node, 1_000 + write_id);
    let w = tab.db.store().writes.next_write_id();
    let r = tab.call(rig, node, make).expect("the write is made");
    assert_eq!(tab.verdicts.get(&w).and_then(|v| v.last()), Some(&protocol::WriteState::Accepted), "the tab's write was not queued behind the commit in flight: {:?}", tab.verdicts.get(&w));
    if !theirs.is_empty() {
        elsewhere(rig, node, write_id, theirs);
    }
    release(rig, node);
    r
}

/// The record at `key` with `patch` over it, re-encoded as another session
/// would write it.
fn their_record(tab: &mut Tab, rig: &mut PageRig, node: &mut Node, schema: &craftworks_sdk::Schema, key: &[u8], patch: serde_json::Value) -> Vec<u8> {
    let old = tab.get(rig, node, key).expect("present");
    let mut d = craftworks_sdk::record::decode(schema, &old).expect("decodes");
    for (k, v) in obj(patch) {
        d.fields.insert(k, v);
    }
    craftworks_sdk::record::encode(schema, &d.fields, d.created, d.updated + 1, &d.author).expect("encodes")
}

/// What the HOST does on its tick, until nothing moves: send what waits
/// (the store's tick re-sends a `Busy` write), re-run what conflicted, pump.
/// Returns what the re-runs could not keep, and the raw conflicts no re-run
/// took.
fn settle(tab: &mut Tab, rig: &mut PageRig, node: &mut Node) -> (Vec<craftworks_sdk::RerunEvent>, Vec<u64>) {
    let mut events = Vec::new();
    let mut raw = Vec::new();
    for _ in 0..12 {
        tab.lend(rig, node, |db| db.store_mut().sync());
        tab.pump(rig, node);
        let step = tab.lend(rig, node, |db| {
            let step = db.rerun();
            // A re-run that stopped on blocks walks again on the next tick.
            db.store_mut().take_ticket();
            db.store_mut().unpin();
            step
        });
        events.extend(step.events);
        raw.extend(tab.db.store_mut().writes.take_conflicts().into_iter().map(|c| c.write_id).filter(|w| !step.taken.contains(w)));
        tab.pump(rig, node);
    }
    (events, raw)
}

/// The record as the node's head holds it.
fn published_record(node: &Node, schema: &craftworks_sdk::Schema, key: &[u8]) -> Option<serde_json::Map<String, serde_json::Value>> {
    let (_, root) = node.head()?;
    let tree = node.tree(&root)?;
    let b = tree.get(key)?;
    Some(craftworks_sdk::record::decode(schema, b).expect("decodes").fields)
}

fn tab_with_record(rig: &mut PageRig, node: &mut Node) -> (Tab, Vec<u8>, craftworks_sdk::id::Loc) {
    let mut tab = Tab::open(rig, node);
    tab.call(rig, node, |db| db.define("t", &rerun_schema())).expect("define");
    tab.pump(rig, node);
    let rec = tab.call(rig, node, |db| db.put("t", &obj(serde_json::json!({ "title": "t0", "body": "b0", "note": "n0" })))).expect("put");
    tab.pump(rig, node);
    let loc = craftworks_sdk::id::loc_from_hex(&rec.id).expect("an id");
    (tab, craftworks_sdk::db::record_key("t", loc), loc)
}

/// sdk#143 on the PAGE path: another session changes the record's `title`;
/// this tab's stale update changes its `body`. The update conflicts, is
/// RE-RUN on their version, and BOTH land: title t1, body b1. Nothing is told
/// (nothing was lost), the conflict is not raw news, and the stale write was
/// never sent again after its verdict — the re-run is a new write. Control:
/// the same wait with nothing changed, and the update publishes as itself.
#[test]
fn a_stale_update_to_another_field_is_re_run_and_both_changes_land() {
    for interfere in [false, true] {
        let mut node = Node::new();
        let mut rig = PageRig::new();
        let (mut tab, key, loc) = tab_with_record(&mut rig, &mut node);
        let theirs = if interfere {
            vec![protocol::Op::Put(key.clone(), their_record(&mut tab, &mut rig, &mut node, &rerun_schema(), &key, serde_json::json!({ "title": "t1" })))]
        } else {
            Vec::new()
        };
        let w = tab.db.store().writes.next_write_id();
        behind(&mut tab, &mut rig, &mut node, 1, theirs, |db| db.update("t", loc, &obj(serde_json::json!({ "body": "b1" }))));
        let (events, raw) = settle(&mut tab, &mut rig, &mut node);
        assert!(events.is_empty(), "interfere {interfere}: nothing was lost and yet {events:?}");
        assert!(raw.is_empty(), "interfere {interfere}: a raw conflict reached the app: {raw:?}");
        assert_eq!(tab.sends.get(&w), Some(&1), "the stale write itself was re-sent (it goes once: queued, then judged where it lands)");
        let rec = published_record(&node, &rerun_schema(), &key).expect("published");
        let want_title = if interfere { "t1" } else { "t0" };
        assert_eq!((rec.get("title"), rec.get("body")), (Some(&serde_json::json!(want_title)), Some(&serde_json::json!("b1"))), "interfere {interfere}");
        let resent = tab.sends.range(w + 1..).count();
        assert_eq!(resent, usize::from(interfere), "interfere {interfere}: {resent} re-run writes");
    }
}

/// THE SAME FIELD: their title t1, this tab's stale title t2. The other side
/// wins; the tab is told ONCE that `title` was dropped; nothing is written
/// again (the patch has nothing left).
#[test]
fn a_stale_update_to_the_same_field_goes_to_the_other_side_told_once() {
    let mut node = Node::new();
    let mut rig = PageRig::new();
    let (mut tab, key, loc) = tab_with_record(&mut rig, &mut node);
    let theirs = their_record(&mut tab, &mut rig, &mut node, &rerun_schema(), &key, serde_json::json!({ "title": "t1" }));
    let w = tab.db.store().writes.next_write_id();
    behind(&mut tab, &mut rig, &mut node, 1, vec![protocol::Op::Put(key.clone(), theirs)], |db| db.update("t", loc, &obj(serde_json::json!({ "title": "t2", "body": "b1" }))));
    let (events, raw) = settle(&mut tab, &mut rig, &mut node);
    assert_eq!(events, vec![craftworks_sdk::RerunEvent::Dropped { write_id: w, fields: vec!["title".into()] }]);
    assert!(raw.is_empty(), "{raw:?}");
    let rec = published_record(&node, &rerun_schema(), &key).expect("published");
    assert_eq!((rec.get("title"), rec.get("body")), (Some(&serde_json::json!("t1")), Some(&serde_json::json!("b1"))));
}

/// MAIN'S MODEL CASE, THE CHAIN: two stale updates of this tab on one record
/// (title, then body) against their change to a third field (note). The
/// chain is re-run in order, the second on the first's result: all three
/// changes survive, nothing is told.
#[test]
fn a_chain_of_stale_updates_is_re_run_in_order_and_every_change_survives() {
    let mut node = Node::new();
    let mut rig = PageRig::new();
    let (mut tab, key, loc) = tab_with_record(&mut rig, &mut node);
    let theirs = their_record(&mut tab, &mut rig, &mut node, &rerun_schema(), &key, serde_json::json!({ "note": "n1" }));
    // Both updates read while a commit is in flight, and queue; theirs lands
    // on the register before that commit does.
    commit_in_flight(&mut rig, &mut node, 1_001);
    tab.call(&mut rig, &mut node, |db| db.update("t", loc, &obj(serde_json::json!({ "title": "t1" })))).expect("the first update");
    tab.call(&mut rig, &mut node, |db| db.update("t", loc, &obj(serde_json::json!({ "body": "b1" })))).expect("the second update");
    elsewhere(&mut rig, &mut node, 1, vec![protocol::Op::Put(key.clone(), theirs)]);
    release(&mut rig, &mut node);
    let (events, raw) = settle(&mut tab, &mut rig, &mut node);
    assert!(events.is_empty(), "{events:?}");
    assert!(raw.is_empty(), "{raw:?}");
    let rec = published_record(&node, &rerun_schema(), &key).expect("published");
    assert_eq!(
        (rec.get("title"), rec.get("body"), rec.get("note")),
        (Some(&serde_json::json!("t1")), Some(&serde_json::json!("b1")), Some(&serde_json::json!("n1"))),
        "a change of the chain was lost"
    );
}

/// THE BUDGET: the record is changed elsewhere before every re-run lands.
/// With the one budget spent (the engine's `max_write_tries`, no count of
/// `Db`'s own) the chain ends as a named failure of its write —
/// never a spin — and their last value stands.
#[test]
fn a_record_changed_under_every_re_run_fails_named_after_the_budget() {
    let mut node = Node::new();
    let mut rig = PageRig::new();
    let (mut tab, key, loc) = tab_with_record(&mut rig, &mut node);
    let w = tab.db.store().writes.next_write_id();
    let mut events = Vec::new();
    let mut other_id = 1;
    let interfere = |tab: &mut Tab, rig: &mut PageRig, node: &mut Node, other_id: &mut u64| {
        let schema = rerun_schema();
        let (_, root) = node.head().expect("a head");
        let cur = node.tree(&root).expect("a tree").get(&key).cloned().expect("the record");
        let mut d = craftworks_sdk::record::decode(&schema, &cur).expect("decodes");
        d.fields.insert("note".into(), serde_json::json!(format!("n{other_id}")));
        let theirs = craftworks_sdk::record::encode(&schema, &d.fields, d.created, d.updated + 1, &d.author).expect("encodes");
        elsewhere(rig, node, *other_id, vec![protocol::Op::Put(key.clone(), theirs)]);
        *other_id += 1;
        let _ = tab;
    };
    // The update, and every re-run of it, READS while a commit is in flight
    // and queues; the record is changed again (another device) before that
    // commit lands, so the queue is re-applied on the change: Conflict.
    commit_in_flight(&mut rig, &mut node, 2_000);
    tab.call(&mut rig, &mut node, |db| db.update("t", loc, &obj(serde_json::json!({ "body": "b1" })))).expect("the update is made");
    interfere(&mut tab, &mut rig, &mut node, &mut other_id);
    release(&mut rig, &mut node);
    for round in 0..10u64 {
        tab.lend(&mut rig, &mut node, |db| db.store_mut().sync());
        tab.pump(&mut rig, &mut node);
        // Its re-run is made behind the next commit in flight...
        commit_in_flight(&mut rig, &mut node, 2_001 + round);
        let step = tab.lend(&mut rig, &mut node, |db| db.rerun());
        events.extend(step.events);
        // ...and changed again before it lands.
        interfere(&mut tab, &mut rig, &mut node, &mut other_id);
        release(&mut rig, &mut node);
    }
    let rounds = Params::default().max_write_tries as usize;
    assert_eq!(tab.sends.range(w + 1..).count(), rounds, "not exactly {rounds} re-runs went on the wire");
    assert!(
        matches!(events.as_slice(), [craftworks_sdk::RerunEvent::Failed { reason, .. }] if reason.contains(&format!("after {rounds} tries"))),
        "the chain did not end as ONE named failure: {events:?}"
    );
    let rec = published_record(&node, &rerun_schema(), &key).expect("published");
    assert_eq!(rec.get("body"), Some(&serde_json::json!("b0")), "a failed chain wrote anyway");
}

/// **ONE BUDGET ACROSS THE ENGINE'S RE-SENDS AND `Db`'s RE-RUNS** (sdk#265;
/// core dev on #295). The update's own commit dies to another device's head
/// (the engine re-applies it at the front: one try spent), which also changed
/// the record's `note`, so it conflicts there. `Db`'s re-run draws on from
/// that one: with a budget of three it has TWO re-runs left, and when the
/// record is changed under each the chain fails, named. A `Db` counting on
/// its own would re-run three times -- four tries of one write.
#[test]
fn a_write_never_gets_more_than_the_one_budget_across_re_sends_and_re_runs() {
    let budget = Params::default().max_write_tries as usize;
    assert_eq!(budget, 3, "the test's arithmetic is for a budget of three");
    let mut node = Node::new();
    let mut rig = PageRig::new();
    let (mut tab, key, loc) = tab_with_record(&mut rig, &mut node);
    let schema = rerun_schema();
    let note = |node: &Node, n: u64| {
        let (_, root) = node.head().expect("a head");
        let cur = node.tree(&root).expect("a tree").get(&key).cloned().expect("the record");
        let mut d = craftworks_sdk::record::decode(&schema, &cur).expect("decodes");
        d.fields.insert("note".into(), serde_json::json!(format!("n{n}")));
        craftworks_sdk::record::encode(&schema, &d.fields, d.created, d.updated + 1, &d.author).expect("encodes")
    };
    let w = tab.db.store().writes.next_write_id();
    // The update's own commit, in flight: its answers held.
    rig.faults.hold = true;
    tab.call(&mut rig, &mut node, |db| db.update("t", loc, &obj(serde_json::json!({ "body": "b1" })))).expect("the update is made");
    assert!(!rig.held.is_empty(), "the update's commit is not in flight");
    // A foreign head kills it and changes the record. Before it, the write is
    // driven into a COMMIT (its fetches answered, its commit's puts and sign
    // held), so the head kills a commit, never a re-apply.
    for kill in 1..=1u64 {
        let session = tab.db.store().writes.client.session().expect("the tab has a session");
        let committing = |rig: &PageRig| rig.server.queued_of(session).iter().any(|(id, f)| *id == w && matches!(f, Fate::Committing));
        for _ in 0..50 {
            if committing(&rig) {
                break;
            }
            let (commit, other): (Vec<Op>, Vec<Op>) = std::mem::take(&mut rig.held).into_iter().partition(|op| matches!(op, Op::Put { .. } | Op::Sign { .. } | Op::Update { .. }));
            rig.held = commit;
            for op in other {
                if let Some(a) = rig.answer(&mut node, op) {
                    rig.server.node(a, Ms(rig.now));
                }
            }
            tab.pump(&mut rig, &mut node);
        }
        assert!(committing(&rig), "kill {kill}: the write is not in a commit: {:?}", rig.server.queued_of(session));
        let ops = vec![protocol::Op::Put(key.clone(), note(&node, kill))];
        elsewhere(&mut rig, &mut node, kill, ops);
        for op in std::mem::take(&mut rig.held) {
            if let Some(a) = rig.answer(&mut node, op) {
                rig.server.node(a, Ms(rig.now));
            }
        }
        tab.pump(&mut rig, &mut node);
    }
    rig.faults.hold = false;
    release(&mut rig, &mut node);
    // Every re-run is made behind a commit in flight and changed again before it lands.
    let mut events = Vec::new();
    for round in 0..10u64 {
        tab.lend(&mut rig, &mut node, |db| db.store_mut().sync());
        tab.pump(&mut rig, &mut node);
        commit_in_flight(&mut rig, &mut node, 3_001 + round);
        let step = tab.lend(&mut rig, &mut node, |db| db.rerun());
        events.extend(step.events);
        let theirs = note(&node, 10 + round);
        elsewhere(&mut rig, &mut node, 10 + round, vec![protocol::Op::Put(key.clone(), theirs)]);
        release(&mut rig, &mut node);
    }
    let reruns = tab.sends.range(w + 1..).count();
    let resent_by_engine = 1usize;
    println!("  one budget {budget}: engine re-sends {resent_by_engine} + Db re-runs {reruns}");
    assert_eq!(resent_by_engine + reruns, budget, "one write got {} tries, the budget is {budget}", resent_by_engine + reruns);
    assert!(
        matches!(events.as_slice(), [craftworks_sdk::RerunEvent::Failed { reason, .. }] if reason.contains(&format!("after {budget} tries"))),
        "the chain did not end as ONE named failure: {events:?}"
    );
    let rec = published_record(&node, &rerun_schema(), &key).expect("published");
    assert_eq!(rec.get("body"), Some(&serde_json::json!("b0")), "a failed chain wrote anyway");
}

/// A record DELETED elsewhere: nothing is re-applied, and the tab is told.
#[test]
fn a_stale_update_of_a_record_deleted_elsewhere_is_told_deleted() {
    let mut node = Node::new();
    let mut rig = PageRig::new();
    let (mut tab, key, loc) = tab_with_record(&mut rig, &mut node);
    let w = tab.db.store().writes.next_write_id();
    behind(&mut tab, &mut rig, &mut node, 1, vec![protocol::Op::Delete(key.clone())], |db| db.update("t", loc, &obj(serde_json::json!({ "body": "b1" }))));
    let (events, _) = settle(&mut tab, &mut rig, &mut node);
    assert_eq!(events, vec![craftworks_sdk::RerunEvent::Deleted { write_id: w }]);
    assert!(published_record(&node, &rerun_schema(), &key).is_none(), "the deleted record came back");
}

/// sdk#144, SCHEMAS: this tab and another session each append a field to the
/// same schema from one base. The tab's define conflicts on the schema key and
/// is RE-RUN as an APPEND on theirs: [title, body, note] + x, + y lands as
/// [title, body, note, x, y] — never the stale whole schema over theirs. A
/// record then carries both.
#[test]
fn a_stale_schema_append_is_re_run_as_an_append_on_theirs() {
    let mut node = Node::new();
    let mut rig = PageRig::new();
    let (mut tab, _key, _loc) = tab_with_record(&mut rig, &mut node);
    let with = |extra: &[(&str, &str)]| -> craftworks_sdk::Schema {
        let mut s = rerun_schema();
        for (n, k) in extra {
            s.fields.push(serde_json::from_value(serde_json::json!({ "name": n, "kind": k })).expect("a field"));
        }
        s
    };
    behind(&mut tab, &mut rig, &mut node, 1, vec![protocol::Op::Put(schema_key_of("t"), serde_json::to_vec(&with(&[("x", "text")])).unwrap())], |db| db.define("t", &with(&[("y", "int")])));
    let (events, raw) = settle(&mut tab, &mut rig, &mut node);
    assert!(events.is_empty(), "{events:?}");
    assert!(raw.is_empty(), "{raw:?}");
    let (_, root) = node.head().expect("a head");
    let stored: craftworks_sdk::Schema = serde_json::from_slice(node.tree(&root).expect("a tree").get(&schema_key_of("t")).expect("the schema")).expect("a schema");
    let names: Vec<&str> = stored.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["title", "body", "note", "x", "y"]);
}

/// The same name appended on both sides: of the same kind it is already
/// there (no write); of a DIFFERENT kind the re-run is a named refusal and
/// their schema stands.
#[test]
fn a_schema_field_appended_on_both_sides_is_skipped_or_refused_by_kind() {
    for (their_kind, refused) in [("text", false), ("int", true)] {
        let mut node = Node::new();
        let mut rig = PageRig::new();
        let (mut tab, _key, _loc) = tab_with_record(&mut rig, &mut node);
        let with = |n: &str, k: &str| -> craftworks_sdk::Schema {
            let mut s = rerun_schema();
            s.fields.push(serde_json::from_value(serde_json::json!({ "name": "x", "kind": k })).expect("a field"));
            if n == "and z" {
                s.fields.push(serde_json::from_value(serde_json::json!({ "name": "z", "kind": "text" })).expect("a field"));
            }
            s
        };
        // Theirs: + x. Mine: + x (text) and z, so mine is not identical and
        // must be judged field by field.
        let w = tab.db.store().writes.next_write_id();
        behind(&mut tab, &mut rig, &mut node, 1, vec![protocol::Op::Put(schema_key_of("t"), serde_json::to_vec(&with("", their_kind)).unwrap())], |db| db.define("t", &with("and z", "text")));
        let (events, _) = settle(&mut tab, &mut rig, &mut node);
        let (_, root) = node.head().expect("a head");
        let stored: craftworks_sdk::Schema = serde_json::from_slice(node.tree(&root).expect("a tree").get(&schema_key_of("t")).expect("the schema")).expect("a schema");
        let names: Vec<&str> = stored.fields.iter().map(|f| f.name.as_str()).collect();
        if refused {
            assert!(matches!(events.as_slice(), [craftworks_sdk::RerunEvent::Failed { write_id, reason }] if *write_id == w && reason.contains("`x`")), "{events:?}");
            assert_eq!(names, ["title", "body", "note", "x"], "their schema did not stand");
        } else {
            assert!(events.is_empty(), "{events:?}");
            assert_eq!(names, ["title", "body", "note", "x", "z"]);
        }
    }
}

/// UNREACHABLE BY DESIGN (schemas only grow), GUARDED ANYWAY: the schema is
/// written back NARROWER elsewhere, without a field this tab's stale patch
/// sets. The re-run is a named refusal naming the field — never a silent drop
/// of it.
#[test]
fn a_patch_field_gone_from_the_schema_is_a_named_refusal() {
    let mut node = Node::new();
    let mut rig = PageRig::new();
    let (mut tab, key, loc) = tab_with_record(&mut rig, &mut node);
    let narrow: craftworks_sdk::Schema = serde_json::from_value(serde_json::json!({ "type": "T", "fields": [
        { "name": "title", "kind": "text", "required": true }
    ] }))
    .expect("a schema");
    let w = tab.db.store().writes.next_write_id();
    behind(&mut tab, &mut rig, &mut node, 1, vec![protocol::Op::Put(schema_key_of("t"), serde_json::to_vec(&narrow).unwrap())], |db| db.update("t", loc, &obj(serde_json::json!({ "body": "b1" }))));
    let (events, _) = settle(&mut tab, &mut rig, &mut node);
    assert!(matches!(events.as_slice(), [craftworks_sdk::RerunEvent::Failed { write_id, reason }] if *write_id == w && reason.contains("`body`") && reason.contains("no longer")), "{events:?}");
    let _ = key;
}

/// ALREADY SATISFIED (sdk#145's second layer): the other side made the SAME
/// change (their title t1, this tab's stale title t1). Nothing was lost, so
/// nothing is told and nothing is written again. The same shape as this
/// tab's own re-send after a lost answer, when the write is stored already.
#[test]
fn a_stale_update_the_other_side_already_made_is_satisfied_silently() {
    let mut node = Node::new();
    let mut rig = PageRig::new();
    let (mut tab, key, loc) = tab_with_record(&mut rig, &mut node);
    let theirs = their_record(&mut tab, &mut rig, &mut node, &rerun_schema(), &key, serde_json::json!({ "title": "t1", "note": "n1" }));
    let w = tab.db.store().writes.next_write_id();
    behind(&mut tab, &mut rig, &mut node, 1, vec![protocol::Op::Put(key.clone(), theirs)], |db| db.update("t", loc, &obj(serde_json::json!({ "title": "t1" }))));
    let (events, raw) = settle(&mut tab, &mut rig, &mut node);
    assert!(events.is_empty(), "a change that is there was told lost: {events:?}");
    assert!(raw.is_empty(), "{raw:?}");
    assert_eq!(tab.sends.range(w + 1..).count(), 0, "a satisfied change was written again");
    let rec = published_record(&node, &rerun_schema(), &key).expect("published");
    assert_eq!((rec.get("title"), rec.get("note")), (Some(&serde_json::json!("t1")), Some(&serde_json::json!("n1"))));
}

/// sdk#225b WITH GROUP COMMIT (K9 §2): a tip whose GROUP wrote one key twice
/// (W1 k=a, then W2 k=b reading a), displaced at its seq by another device's
/// commit from the same base. The merge re-applies the group IN ORDER on the
/// winner, each write re-judged there:
/// * the winner left k alone -> both land again, k holds b (the FINAL value),
///   nobody is told Superseded;
/// * the winner changed k -> k's LAST writer (W2) is told Superseded for it;
///   W1, whose k its own W2 overwrote, is never told (it lost to its own
///   app's order), and the winner's value stands.
#[test]
fn a_group_that_wrote_one_key_twice_merges_by_its_final_value() {
    use signer_proto::head::{value, Ledger};
    for they_change_k in [false, true] {
        let mut node = Node::new();
        let mut rig = PageRig::new();
        let mut tab = Tab::open(&mut rig, &mut node);
        tab.call(&mut rig, &mut node, |db| db.define("t", &tab_schema())).expect("define");
        tab.pump(&mut rig, &mut node);
        let k = b"k/twice".to_vec();
        // A commit in flight, so W1 and W2 queue behind it and are CUT together.
        rig.faults.hold = true;
        tab.lend(&mut rig, &mut node, |db| craftworks_sdk::store::Store::apply_commit(db.store_mut(), &[(b"k/first".to_vec(), protocol::Expect::Absent)], &[(b"k/first".to_vec(), craftworks_sdk::store::Edit::Put(b"f".to_vec()))])).expect("taken");
        let w1 = tab.db.store().writes.next_write_id();
        tab.lend(&mut rig, &mut node, |db| craftworks_sdk::store::Store::apply_commit(db.store_mut(), &[(k.clone(), protocol::Expect::Absent)], &[(k.clone(), craftworks_sdk::store::Edit::Put(b"a".to_vec()))])).expect("taken");
        let w2 = tab.db.store().writes.next_write_id();
        tab.lend(&mut rig, &mut node, |db| craftworks_sdk::store::Store::apply_commit(db.store_mut(), &[(k.clone(), protocol::Expect::Value(craftworks_sdk::read_token::read_token(b"a")))], &[(k.clone(), craftworks_sdk::store::Edit::Put(b"b".to_vec()))])).expect("taken");
        release(&mut rig, &mut node);
        tab.pump(&mut rig, &mut node);
        for w in [w1, w2] {
            assert!(tab.verdicts.get(&w).is_some_and(|v| v.contains(&protocol::WriteState::Published)), "write {w} did not publish: {:?}", tab.verdicts.get(&w));
        }
        let (tip_seq, tip_root) = node.head().expect("published");
        assert_eq!(node.tree(&tip_root).expect("whole").get(&k).map(Vec::as_slice), Some(&b"b"[..]), "the group's final value is not in the tip");
        // The other device's commit at the tip's seq, from the tip's base.
        let hr = node.head_read().expect("a head");
        let base = hr.prev().expect("the tip has a prev");
        let base_tree = node.tree(&base.1).expect("the base is whole");
        assert!(!base_tree.contains_key(&k) && base_tree.contains_key(b"k/first".as_slice()), "W1 and W2 were not ONE commit on top of the first: this is not a group");
        let key_of = node.secrets.get(signer::PROVISION).and_then(|b| signer::Provisioned::decode(b)).map(|p| p.key).expect("provisioned");
        let st = winning(&format!("{}:{}", file!(), line!()), |salt| {
            let mut entries: Vec<(Vec<u8>, Vec<u8>)> = base_tree.clone().into_iter().collect();
            entries.push((b"other".to_vec(), salt.to_le_bytes().to_vec()));
            if they_change_k {
                entries.push((k.clone(), b"theirs".to_vec()));
            }
            let r = device_tree(&mut node, &entries);
            let v = value(&r, &Ledger { prev: Some(signer_proto::Head { seq: base.0, root: base.1 }), ..Ledger::default() });
            if page::beats(&v, hr.value()) {
                return Some(contract_keys::register::head_state(&node.register_params, &key_of, tip_seq, &v).expect("signs"));
            }
            None
        });
        node.update(&st);
        rig.server.head_hint();
        tab.pump(&mut rig, &mut node);
        for _ in 0..20 {
            rig.now += 1_000;
            rig.server.tick(Ms(rig.now));
            tab.pump(&mut rig, &mut node);
        }
        let told: Vec<(u64, Vec<Vec<u8>>)> = tab.db.store_mut().writes.take_superseded().into_iter().map(|s| (s.write_id, s.keys)).collect();
        let (_, froot) = node.head().expect("a head");
        let fin = node.tree(&froot).expect("the final tree is whole");
        if they_change_k {
            assert_eq!(told, vec![(w2, vec![k.clone()])], "not the key's LAST writer alone told");
            assert_eq!(fin.get(&k).map(Vec::as_slice), Some(&b"theirs"[..]), "their value did not stand");
        } else {
            assert!(told.is_empty(), "told Superseded for a key the winner never touched: {told:?}");
            assert_eq!(fin.get(&k).map(Vec::as_slice), Some(&b"b"[..]), "the merge did not land the group's FINAL value");
        }
        assert!(fin.contains_key(b"other".as_slice()), "their change did not survive the merge");
    }
}

/// **A FULL `through` LIST OF OLDER ENTRIES: A FIRST COMMIT THAT DIES IS
/// RE-APPLIED, NEVER `Unknown`** (review §3 on sdk#295). A device is minted
/// per page load, so a household's list reaches `THROUGH_MAX` and stays
/// there. This page's first commit (seq 2) dies to another device's head at
/// seq 2 whose list is full -- of entries last written at seq 1, older than
/// our commit. Ours cannot have been evicted (eviction takes the lowest
/// `last`), so absent means NOT THERE: the write is re-applied on the winner
/// and publishes. Read as `Unknown` it would end there, its value never
/// landing.
#[test]
fn a_dying_first_commit_under_a_full_list_of_older_entries_is_re_applied() {
    use signer_proto::head::{value, Ledger, Through, THROUGH_MAX};
    let mut node = Node::new();
    let mut rig = PageRig::new();
    rig.client_as(&mut node, &Request::Identity);
    assert!(published(&states(&rig.client_as(&mut node, &write(1, &[("a", Some("1"))])), 1)));
    let (s1, root1) = node.head().expect("published");
    // This page's next commit, at s1 + 1: nothing of it reaches the node.
    rig.faults.hold = true;
    let mine = Request::Commit { write_id: 2, reads: vec![(b"b".to_vec(), protocol::Expect::Absent)], ops: vec![protocol::Op::Put(b"b".to_vec(), b"2".to_vec())] };
    rig.client_as(&mut node, &mine);
    assert_eq!(rig.server.page.committing_seq(), Some(s1 + 1), "the commit is not in flight at s1 + 1");
    rig.held.clear();
    rig.faults.hold = false;
    // Another device's head at the same seq, built on ours, with a FULL list
    // of entries older than our commit.
    let mut entries: Vec<(Vec<u8>, Vec<u8>)> = node.tree(&root1).expect("whole").into_iter().collect();
    entries.push((b"theirs".to_vec(), b"x".to_vec()));
    let theirs = device_tree(&mut node, &entries);
    let through: Vec<Through> = (0..THROUGH_MAX)
        .map(|i| {
            let mut d = [0u8; 16];
            d[..8].copy_from_slice(&(i as u64 + 1000).to_le_bytes());
            Through { device: d, seq: 1, last: s1 }
        })
        .collect();
    let v = value(&theirs, &Ledger { prev: Some(signer_proto::Head { seq: s1, root: root1 }), through, ..Ledger::default() });
    let key = node.secrets.get(signer::PROVISION).and_then(|b| signer::Provisioned::decode(b)).map(|p| p.key).expect("provisioned");
    let st = contract_keys::register::head_state(&node.register_params, &key, s1 + 1, &v).expect("signs");
    node.update(&st);
    assert_eq!(node.head(), Some((s1 + 1, theirs)), "their head did not take the register");
    rig.server.head_hint();
    let mut rs = rig.run(&mut node);
    for _ in 0..10 {
        rig.now += 1_000;
        rig.server.tick(Ms(rig.now));
        rs.extend(rig.run(&mut node));
    }
    let told = states(&rs, 2);
    assert!(!told.contains(&WriteState::Unknown), "a dying first commit under a full list of OLDER entries was told Unknown: {told:?}");
    assert!(published(&told), "the write was not re-applied and published on the winner: {told:?}");
    let (_, root) = node.head().expect("a head");
    let tree = tree_of(&node, &root);
    assert_eq!(tree.get(&b"b"[..]).map(Vec::as_slice), Some(&b"2"[..]), "the write's value never landed");
    assert_eq!(tree.get(&b"theirs"[..]).map(Vec::as_slice), Some(&b"x"[..]), "the winner's value was lost");
}

/// **THE MERGE GOES AT THE FRONT, BEFORE A LATER OWN WRITE** (review §1 on
/// sdk#295; K9 §2). The tip W1 (forced, k = a) publishes at s; W2 (k = b,
/// read k = a: after W1 in app order) is in flight behind it. Another device
/// wins at s from the same base, leaving k alone. The displaced W1 must be
/// re-applied BEFORE W2: then W2's read holds and the tree ends at the LATER
/// value b, nothing told superseded. Queued behind (the defect), W2 is judged
/// on the winner first -- k absent there, so it conflicts -- and W1' lands a
/// after it: the older value wins, and the page's own later write is lost.
#[test]
fn a_merge_goes_before_a_later_own_write_and_the_later_value_stands() {
    use signer_proto::head::{value, Ledger};
    let mut node = Node::new();
    let mut rig = PageRig::new();
    rig.client_as(&mut node, &Request::Identity);
    assert!(published(&states(&rig.client_as(&mut node, &write(1, &[("base", Some("0"))])), 1)));
    let (base_seq, base_root) = node.head().expect("the base");
    assert!(published(&states(&rig.client_as(&mut node, &write(2, &[("k", Some("a"))])), 2)));
    let hr = node.head_read().expect("the tip");
    let (tip_seq, _) = node.head().expect("the tip");
    // W2, in flight behind the tip: nothing of it reaches the node.
    rig.faults.hold = true;
    let w2 = Request::Commit { write_id: 3, reads: vec![(b"k".to_vec(), protocol::Expect::Value(engine::leaf_hash(b"a")))], ops: vec![protocol::Op::Put(b"k".to_vec(), b"b".to_vec())] };
    let mut rs = rig.client_as(&mut node, &w2);
    rig.held.clear();
    rig.faults.hold = false;
    // The other device's head at the tip's seq, from the same base, leaving k alone.
    let key_of = node.secrets.get(signer::PROVISION).and_then(|b| signer::Provisioned::decode(b)).map(|p| p.key).expect("provisioned");
    let base_tree = node.tree(&base_root).expect("whole");
    let st = winning(&format!("{}:{}", file!(), line!()), |salt| {
        let mut e: Vec<(Vec<u8>, Vec<u8>)> = base_tree.clone().into_iter().collect();
        e.push((b"other".to_vec(), salt.to_le_bytes().to_vec()));
        let r = device_tree(&mut node, &e);
        let v = value(&r, &Ledger { prev: Some(signer_proto::Head { seq: base_seq, root: base_root }), ..Ledger::default() });
        if page::beats(&v, hr.value()) {
            return Some(contract_keys::register::head_state(&node.register_params, &key_of, tip_seq, &v).expect("signs"));
        }
        None
    });
    node.update(&st);
    rig.server.head_hint();
    rs.extend(rig.run(&mut node));
    for _ in 0..20 {
        rig.now += 1_000;
        rig.server.tick(Ms(rig.now));
        rs.extend(rig.run(&mut node));
    }
    let told = states(&rs, 3);
    let fin = node.tree(&node.head().expect("a head").1).expect("whole");
    assert_eq!(fin.get(&b"k"[..]).map(Vec::as_slice), Some(&b"b"[..]), "the older value landed over the later one (W2 told {told:?})");
    assert!(published(&told), "the later write did not publish: {told:?}");
    assert!(fin.contains_key(&b"other"[..]), "their change did not survive");
    let superseded: Vec<&Reply> = rs.iter().filter(|r| format!("{r:?}").contains("Superseded")).collect();
    assert!(superseded.is_empty(), "a key was told superseded by the page's own later write: {superseded:?}");
    assert_eq!(rig.server.page.queue_counts().2, 0, "an impossible transition was counted (the hold was not in place)");
}

/// **A READ REPAIRS THROUGH PARITY, ON THE PAGE PATH** (Phase 4,
/// DURABILITY-STATE gap 1; the owner's test: data survives when the nodes
/// that held it go away). A page writes 6,000 rows and PUTS ITS OWN PARITY;
/// the node then LOSES blocks of one sibling group; a fresh page reads every
/// row of the lost leaves through `Page::send`. Three lost: every value right,
/// rebuilt from the group. A fourth lost: the group cannot rebuild, and the
/// read does NOT end (rule 7) -- the page keeps asking the node for five
/// minutes of NotFound, and when the block is back the read answers it.
#[test]
fn a_fresh_page_reads_rows_whose_blocks_the_node_lost_rebuilt_from_parity() {
    use freenet_prolly::node::Node as TreeNode;
    let mut node = Node::new();
    let mut rig = PageRig::new();
    rig.client_as(&mut node, &Request::Identity);
    let value = |n: u32| format!("value {n}");
    for b in 0..30u32 {
        let rows: Vec<(String, String)> = (0..200u32).map(|i| (format!("k/{:06}", b * 200 + i), value(b * 200 + i))).collect();
        let ops: Vec<(&str, Option<&str>)> = rows.iter().map(|(k, v)| (k.as_str(), Some(v.as_str()))).collect();
        assert!(published(&states(&rig.client_as(&mut node, &write(u64::from(b) + 1, &ops)), u64::from(b) + 1)), "batch {b} did not publish");
    }
    for _ in 0..100 {
        if rig.server.page.owed_groups() == 0 {
            break;
        }
        rig.now += 1_000;
        rig.server.tick(Ms(rig.now));
        rig.run(&mut node);
    }
    assert_eq!(rig.server.page.owed_groups(), 0, "the writer's parity never landed");
    // One level above the leaves, its largest group.
    let (_, root) = node.head().expect("published");
    let mut at = root;
    let (members, parity) = loop {
        let n = TreeNode::parse(node.blocks.get(&at).expect("held")).expect("a node");
        assert!(!n.is_leaf(), "one leaf: no groups");
        if n.level() == 1 {
            let ids: Vec<Cid> = n.parity().collect();
            let (g, (_, m)) = freenet_prolly::parity::group_members(&n).into_iter().enumerate().max_by_key(|(_, (_, m))| m.len()).expect("a group");
            break (m, ids[engine::PARITY * g..engine::PARITY * (g + 1)].to_vec());
        }
        at = n.child(0).0;
    };
    assert!(members.len() >= 7, "a group of {}", members.len());
    assert!(parity.iter().all(|p| node.blocks.contains_key(p)), "the writer's parity for the group is not on the node");
    let keys_of = |node: &Node, leaf: &Cid| -> Vec<Vec<u8>> {
        let n = TreeNode::parse(node.blocks.get(leaf).expect("held")).expect("a leaf");
        (0..n.len()).map(|i| n.key(i)).collect()
    };
    let three_keys: Vec<Vec<u8>> = members[..3].iter().flat_map(|l| keys_of(&node, l)).collect();
    let first_leaf_keys = keys_of(&node, &members[0]);
    let first_leaf = node.blocks.get(&members[0]).expect("held").clone();
    // THE NODE LOSES three members of the group.
    for l in &members[..3] {
        node.blocks.remove(l);
    }
    let mut reader = PageRig::new();
    reader.session = SESSION + 50;
    reader.client_as(&mut node, &Request::Identity);
    let mut req = 1_000u64;
    let mut read = |reader: &mut PageRig, node: &mut Node, key: &[u8]| -> Option<Reply> {
        req += 1;
        let id = req;
        let rs = reader.client_as(node, &Request::Get { req_id: id, key: key.to_vec() });
        rs.into_iter().find(|r| matches!(r, Reply::Value { req_id, .. } | Reply::Unavailable { req_id, .. } if *req_id == id))
    };
    let mut wrong = Vec::new();
    for key in &three_keys {
        let n: u32 = std::str::from_utf8(&key[2..]).expect("utf8").parse().expect("a number");
        match read(&mut reader, &mut node, key) {
            Some(Reply::Value { value: Some(v), .. }) if v == value(n).into_bytes() => {}
            other => wrong.push(format!("{}: {other:?}", String::from_utf8_lossy(key))),
        }
    }
    assert!(wrong.is_empty(), "{} reads wrong with 3 blocks of a group lost: {:?}", wrong.len(), &wrong[..wrong.len().min(3)]);
    assert!(reader.server.page.repair_counts().1 >= 3, "the lost leaves were not rebuilt: {:?}", reader.server.page.repair_counts());
    // REPAIR IS A WRITE (sdk#331): the reader PUT the three rebuilt leaves back, so the node is whole again.
    for (i, l) in members[..3].iter().enumerate() {
        assert!(node.blocks.contains_key(l), "rebuilt leaf {i} was not PUT back to the node");
    }
    assert_eq!(node.blocks.get(&members[0]), Some(&first_leaf), "the leaf PUT back is not the leaf that was lost");
    // PARITY + 1 lost (the three again, and more): a fresh page cannot rebuild the group any more.
    assert!(members.len() > engine::PARITY, "the group has {} members: too small to lose PARITY + 1", members.len());
    for l in &members[..engine::PARITY + 1] {
        node.blocks.remove(l);
    }
    let mut late = PageRig::new();
    late.session = SESSION + 60;
    late.client_as(&mut node, &Request::Identity);
    let key = first_leaf_keys[0].clone();
    let want: u32 = std::str::from_utf8(&key[2..]).expect("utf8").parse().expect("a number");
    let id = 9_000u64;
    let answer = |rs: &[Reply]| rs.iter().find(|r| matches!(r, Reply::Value { req_id, .. } | Reply::Unavailable { req_id, .. } if *req_id == id)).cloned();
    late.send_only(&Request::Get { req_id: id, key: key.clone() });
    let mut rs = Vec::new();
    // FIVE MINUTES of the node answering NotFound: no answer, never
    // `Unavailable`, and the block asked for again and again.
    rs.extend(late.run_for(&mut node, 300_000));
    assert_eq!(answer(&rs), None, "with the block NotFound for 5 min the read ended: {:?}", answer(&rs));
    assert!(late.server.page.waiting(), "the page stopped waiting on a read it never answered");
    let asked = late.gets.get(&members[0]).copied().unwrap_or(0);
    assert!(asked >= 5, "the lost leaf was asked for {asked} time(s) in 5 min: the page stopped asking");
    // ...and on a BACKOFF, not at the speed of the NotFounds: doubling from
    // 100 ms to the 60 s ceiling is ~15 asks in 5 min.
    assert!(asked <= 40, "the lost leaf was asked for {asked} times in 5 min: re-asked at the speed of the answers, not on a backoff; GETs at {:?} .. {:?}", &late.get_at[..late.get_at.len().min(12)], &late.get_at[late.get_at.len().saturating_sub(4)..]);
    // The block is back (a keeper's repair, a late PUT): the SAME read answers.
    node.blocks.insert(members[0], first_leaf);
    rs.extend(late.run_for(&mut node, 120_000));
    match answer(&rs) {
        Some(Reply::Value { value: Some(v), .. }) if v == value(want).into_bytes() => {}
        other => panic!("the block came back and the read answered {other:?} (asked {asked} times while it was gone)"),
    }
    println!("  {} of a group lost: 5 min NotFound, {asked} GETs, no Unavailable; block back -> the read answers", engine::PARITY + 1);
}

/// Long enough after a GET's send that the node's own GET is certainly over, whatever the RTO (sdk#447): a GET the
/// node never answered is asked again only then.
const PAST_B: u64 = page::rto::NODE_GET_BOUND_MS as u64 + page::rto::RTO_MAX_MS as u64;

/// **A BLOCK THE NODE IS SILENT ABOUT FOR FIVE MINUTES IS STILL READ** (the
/// owner's rule 7; the owner's "block … could not be had"). Silence is not
/// an answer: the page asks again once the node's own GET is over (sdk#447:
/// never beside it, the node does not dedupe), the read never ends
/// `Unavailable`, and when the node answers, the read answers.
#[test]
fn a_block_silent_for_five_minutes_then_answering_is_read() {
    let mut node = Node::new();
    let mut rig = PageRig::new();
    rig.client_as(&mut node, &Request::Identity);
    let rows: Vec<(String, String)> = (0..600u32).map(|i| (format!("k/{i:06}"), format!("value {i}"))).collect();
    let ops: Vec<(&str, Option<&str>)> = rows.iter().map(|(k, v)| (k.as_str(), Some(v.as_str()))).collect();
    assert!(published(&states(&rig.client_as(&mut node, &write(1, &ops)), 1)), "the write did not publish");
    // A fresh page; the node goes SILENT about every block (it answers
    // nothing), so the read's first GET goes unanswered.
    let mut reader = PageRig::new();
    reader.session = SESSION + 70;
    reader.client_as(&mut node, &Request::Identity);
    reader.silent = node.blocks.keys().copied().collect();
    let id = 7_000u64;
    reader.send_only(&Request::Get { req_id: id, key: b"k/000321".to_vec() });
    let mut rs = Vec::new();
    rs.extend(reader.run_for(&mut node, 300_000));
    let answer = |rs: &[Reply]| rs.iter().find(|r| matches!(r, Reply::Value { req_id, .. } | Reply::Unavailable { req_id, .. } if *req_id == id)).cloned();
    assert_eq!(answer(&rs), None, "a read over a silent node ended: {:?}", answer(&rs));
    let sent: usize = reader.gets.values().sum();
    // One node GET per bound (sdk#447): sent, and sent again once the node's own GET is over (B + an RTO < 5 min).
    assert!(sent >= 2, "{sent} GETs in 5 min of silence: the page did not ask again after the node's GET was over");
    // ...each on its OWN backoff (RTO x 2^(n-1), to the 60 s ceiling), not at
    // a shared RTO other answers keep pulling down.
    assert!(sent <= 60, "{sent} GETs in 5 min of silence: re-sent without a per-op backoff; first at {:?}, last at {:?}", &reader.get_at[..reader.get_at.len().min(12)], &reader.get_at[reader.get_at.len().saturating_sub(4)..]);
    // The node answers again: the GET still out is silent until its node GET is over, then asked again and answered.
    reader.silent.clear();
    rs.extend(reader.run_for(&mut node, PAST_B));
    match answer(&rs) {
        Some(Reply::Value { value: Some(v), .. }) if v == b"value 321" => {}
        other => panic!("the node answered again and the read answered {other:?} ({sent} GETs while silent)"),
    }
    println!("  5 min silent: {sent} GETs re-sent, no Unavailable; node answers -> the read answers");
}

/// **A HEAD IS SIGNED AS THE SUCCESSOR OF ITS OWN BASE, NEVER OF WHATEVER IS
/// PUBLISHED NOW** (data loss found by the page model, two devices, seed 10:
/// "the final tree lost p0/004"; reached there through race put's early head).
/// The page publishes `a` at seq s. Its next commit (`b`, seq s+1, built on
/// its own s) puts its blocks, but the node's ACKS are held back, so the engine
/// settles them by fact and emits the head while the page still holds it for
/// its acks. Another device of the same identity then wins seq s with `other`,
/// and the page adopts it. When the acks arrive and the held head is released,
/// it must not be signed with the WINNER as its prev: the signer would accept
/// (the prev matches the register), and `other` would be gone from every
/// later head. The commit is re-derived on the winner instead.
#[test]
fn a_commit_built_before_a_foreign_winner_is_never_signed_as_its_successor() {
    use signer_proto::head::{value, Ledger};
    let mut node = Node::new();
    let mut rig = PageRig::new();
    rig.client_as(&mut node, &Request::Identity);
    assert!(published(&states(&rig.client_as(&mut node, &write(1, &[("base", Some("0"))])), 1)));
    let (base_seq, base_root) = node.head().expect("the base");
    assert!(published(&states(&rig.client_as(&mut node, &write(2, &[("a", Some("1"))])), 2)));
    let hr = node.head_read().expect("our head at s");
    let (s, ours) = node.head().expect("our head at s");
    let mut signs: Vec<(Cid, Cid)> = Vec::new();
    let mut withheld: Vec<Answer> = Vec::new();
    // Every op the page sends is answered, the signer's included, and every
    // sign request is recorded first; a PUT's ACK is held back while
    // `withhold` (the block itself does reach the node).
    let pump = |rig: &mut PageRig, node: &mut Node, withhold: bool, signs: &mut Vec<(Cid, Cid)>, withheld: &mut Vec<Answer>| {
        for op in std::mem::take(&mut rig.held) {
            if let Op::Sign { prev_root, root, .. } = &op {
                signs.push((*prev_root, *root));
            }
            let put = matches!(op, Op::Put { .. });
            if let Some(a) = rig.answer(node, op) {
                if put && withhold {
                    withheld.push(a);
                } else {
                    rig.server.node(a, Ms(rig.now));
                }
            }
        }
        rig.now += 1_000;
        rig.server.tick(Ms(rig.now));
        rig.run(node)
    };
    // A CHECKED write (it read `b` absent): a dead commit's checked write is
    // re-applied on the winner, where a forced one would end `Lost`.
    rig.faults.hold = true;
    let w3 = Request::Commit { write_id: 3, reads: vec![(b"b".to_vec(), protocol::Expect::Absent)], ops: vec![protocol::Op::Put(b"b".to_vec(), b"2".to_vec())] };
    let mut rs = rig.client_as(&mut node, &w3);
    for _ in 0..40 {
        rs.extend(pump(&mut rig, &mut node, true, &mut signs, &mut withheld));
    }
    assert!(!withheld.is_empty(), "no put ack was held back: the case this test is about did not happen");
    assert!(signs.is_empty(), "the head was signed before its acks arrived: {signs:?}");
    // Another device wins seq s, from the same base, with `other`.
    let key_of = node.secrets.get(signer::PROVISION).and_then(|b| signer::Provisioned::decode(b)).map(|p| p.key).expect("provisioned");
    let base_tree = node.tree(&base_root).expect("whole");
    let (winner, st) = winning(&format!("{}:{}", file!(), line!()), |salt| {
        let mut e: Vec<(Vec<u8>, Vec<u8>)> = base_tree.clone().into_iter().collect();
        e.push((b"other".to_vec(), salt.to_le_bytes().to_vec()));
        let r = device_tree(&mut node, &e);
        let v = value(&r, &Ledger { prev: Some(signer_proto::Head { seq: base_seq, root: base_root }), ..Ledger::default() });
        if page::beats(&v, hr.value()) {
            return Some((r, contract_keys::register::head_state(&node.register_params, &key_of, s, &v).expect("signs")));
        }
        None
    });
    node.update(&st);
    assert_eq!(node.head(), Some((s, winner)), "the winner did not take the register");
    rig.server.head_hint();
    for _ in 0..10 {
        rs.extend(pump(&mut rig, &mut node, true, &mut signs, &mut withheld));
    }
    assert_eq!(rig.server.page.published(), (s, winner), "the page did not adopt the winner");
    // The held-back acks arrive now: the held head is released.
    for a in std::mem::take(&mut withheld) {
        rig.server.node(a, Ms(rig.now));
    }
    for _ in 0..40 {
        rs.extend(pump(&mut rig, &mut node, false, &mut signs, &mut withheld));
    }
    let lying: Vec<_> = signs.iter().filter(|(p, _)| *p == winner).filter(|(_, r)| !node.tree(r).is_some_and(|t| t.contains_key(&b"other"[..]))).collect();
    assert!(lying.is_empty(), "a root WITHOUT the winner's row was signed as the winner's successor: {} time(s)", lying.len());
    let (_, root) = node.head().expect("a head");
    let fin = tree_of(&node, &root);
    assert!(fin.contains_key(&b"other"[..]), "the winner's row is gone from the final tree");
    assert_eq!(fin.get(&b"b"[..]).map(Vec::as_slice), Some(&b"2"[..]), "the later write never landed (told {:?})", states(&rs, 3));
    let _ = ours;
}

/// **A DELTA WHOSE BLOCK IS SILENT WAITS** (rule 7). A delta answers
/// `FullReloadRequired` on a real NotFound -- the old root is most likely gone
/// -- but SILENCE is not an answer: a node that has not replied yet says
/// nothing about whether the block exists. So over five minutes of silence
/// the delta is answered nothing at all, and when the node answers, it is
/// answered the real `Delta`. (Turning silence into a miss -- the old tick --
/// answers it `FullReloadRequired` at once: the mutant this test kills.)
#[test]
fn a_delta_whose_block_is_silent_waits_and_is_answered_the_delta() {
    let mut node = Node::new();
    let mut rig = PageRig::new();
    rig.client_as(&mut node, &Request::Identity);
    let rows: Vec<(String, String)> = (0..600u32).map(|i| (format!("k/{i:06}"), format!("value {i}"))).collect();
    let ops: Vec<(&str, Option<&str>)> = rows.iter().map(|(k, v)| (k.as_str(), Some(v.as_str()))).collect();
    assert!(published(&states(&rig.client_as(&mut node, &write(1, &ops)), 1)), "the first write did not publish");
    let (_, from) = node.head().expect("published");
    assert!(published(&states(&rig.client_as(&mut node, &write(2, &[("k/000321", Some("changed"))])), 2)), "the second write did not publish");
    // A fresh page asks what changed since the first root; the node is SILENT
    // about every block.
    let mut reader = PageRig::new();
    reader.session = SESSION + 80;
    reader.client_as(&mut node, &Request::Identity);
    reader.silent = node.blocks.keys().copied().collect();
    let id = 8_000u64;
    reader.send_only(&Request::ChangesSince { req_id: id, from, lo: protocol::Bound::Unbounded, hi: protocol::Bound::Unbounded, max_entries: 100 });
    let answer = |rs: &[Reply]| rs.iter().find(|r| matches!(r, Reply::Delta { req_id, .. } | Reply::FullReloadRequired { req_id, .. } | Reply::Unavailable { req_id, .. } if *req_id == id)).cloned();
    let mut rs = reader.run_for(&mut node, 300_000);
    assert_eq!(answer(&rs), None, "a delta over a SILENT node was answered: {:?}", answer(&rs));
    let sent: usize = reader.gets.values().sum();
    assert!((1..=60).contains(&sent), "{sent} GETs in 5 min of silence");
    // The GET still out is silent until its node GET is over (sdk#447), then asked again and answered.
    reader.silent.clear();
    rs.extend(reader.run_for(&mut node, PAST_B));
    match answer(&rs) {
        Some(Reply::Delta { changes, .. }) if changes.iter().any(|(k, v)| k == b"k/000321" && v.as_deref() == Some(&b"changed"[..])) => {}
        other => panic!("the node answered again and the delta was answered {other:?} ({sent} GETs while silent)"),
    }
    println!("  delta over 5 min of silence: {sent} GETs, no answer; node answers -> the Delta");
}

// ---- SUPERSEDED READS (#330 ruling): a read pinned to a root nobody serves moves on at a newer head ----

/// A tab's point read of `key`, decided as a page decides it.
fn decided_get(tab: &mut Tab, rig: &mut PageRig, node: &mut Node, key: &[u8]) -> craftworks_sdk::Outcome<Option<Vec<u8>>> {
    tab.lend(rig, node, |db| {
        let r = craftworks_sdk::store::Reads::get(db.store_mut(), key).map_err(|e| match e {
            craftworks_sdk::store::StoreError::NotLoaded => craftworks_sdk::DbError::NotLoaded { lo: key.to_vec(), hi: key.to_vec() },
            other => craftworks_sdk::DbError::Refused(other.to_string()),
        });
        db.store_mut().decide(r)
    })
}

/// 600 rows published by one page, and a READER tab on a fresh page of the same node (session `off`), its `silent`
/// set before it opens.
fn published_rows_and_reader(off: u64, silent: impl FnOnce(&Node) -> std::collections::BTreeSet<Cid>) -> (Node, PageRig, Tab) {
    let mut node = Node::new();
    let mut w = PageRig::new();
    w.client_as(&mut node, &Request::Identity);
    let rows: Vec<(String, String)> = (0..600u32).map(|i| (format!("k/{i:06}"), format!("value {i}"))).collect();
    let ops: Vec<(&str, Option<&str>)> = rows.iter().map(|(k, v)| (k.as_str(), Some(v.as_str()))).collect();
    assert!(published(&states(&w.client_as(&mut node, &write(1, &ops)), 1)), "the write did not publish");
    let mut rig = PageRig::new();
    rig.session = SESSION + off;
    rig.silent = silent(&node);
    let tab = Tab::open(&mut rig, &mut node);
    (node, rig, tab)
}

/// Another device moves the head (a row of its own), and the page hears it.
fn move_head(rig: &mut PageRig, node: &mut Node, salt: u32) {
    elsewhere(rig, node, 0, vec![protocol::Op::Put(format!("x/{salt:06}").into_bytes(), b"theirs".to_vec())]);
    rig.server.head_hint();
}

/// **A READ PINNED TO A ROOT WHOSE BLOCK NEVER COMES MOVES ON AT A NEWER HEAD** (#330's realnet regression, measured:
/// A's view parked for 347 s on seq 74's root block, silent then NotFound, while seq 75 and 76 were readable). The
/// node is SILENT about the published root: the read waits on it — nothing ends it, no clock (rule 8). Another
/// device moves the head: the read, pinned to the old root with nothing arrived, ends SUPERSEDED, and the chain runs
/// again unpinned at the new head, where it answers — the row, and the other device's row with it.
#[test]
fn a_read_pinned_to_a_root_whose_block_never_comes_moves_on_at_a_newer_head() {
    // The ROOT GROUP silent: the root and its parity (sdk#335: the root is a group of one, and with any one of its
    // 1 + m answering the read would not wait at all).
    let root_group = |n: &Node| n.head().map(|h| h.1).into_iter().chain(n.head_read().and_then(|h| h.mark()).unwrap_or_default()).collect();
    let (mut node, mut rig, mut tab) = published_rows_and_reader(71, root_group);
    let t = match decided_get(&mut tab, &mut rig, &mut node, b"k/000321") {
        craftworks_sdk::Outcome::Wait(_, t) => t,
        other => panic!("THE SETUP: the read over a silent root did not wait: {other:?}"),
    };
    tab.pump(&mut rig, &mut node);
    assert_eq!(tab.ended.get(&t), None, "a read over a silent root ENDED with no newer head (a clock?)");
    move_head(&mut rig, &mut node, 1);
    tab.pump(&mut rig, &mut node);
    assert_eq!(tab.ended.get(&t), Some(&craftworks_sdk::Ended::Superseded), "a newer head did not move the stuck read on");
    assert_eq!(tab.get(&mut rig, &mut node, b"k/000321"), Some(b"value 321".to_vec()), "the chain again, at the new head");
    assert_eq!(tab.get(&mut rig, &mut node, b"x/000001"), Some(b"theirs".to_vec()), "the read is not at the new head");
}

/// A point read as a page makes it, on a SLOW node: every GET is held and answered only at a step for which
/// `answer_at(step)` is true; after each step's answers another device moves the head. Returns the value and how
/// many times the chain was superseded. At most `steps` steps.
fn read_under_moving_heads(tab: &mut Tab, rig: &mut PageRig, node: &mut Node, key: &[u8], steps: u32, answer_at: impl Fn(u32) -> bool) -> (Option<Option<Vec<u8>>>, u32) {
    rig.faults.hold_gets = true;
    let (mut step, mut superseded) = (0u32, 0u32);
    loop {
        let t = match decided_get(tab, rig, node, key) {
            craftworks_sdk::Outcome::Done(v) => return (Some(v), superseded),
            craftworks_sdk::Outcome::Told(e) => panic!("the read was told {e:?}"),
            craftworks_sdk::Outcome::Wait(_, t) => t,
        };
        loop {
            if step >= steps {
                return (None, superseded);
            }
            step += 1;
            if answer_at(step) {
                for op in std::mem::take(&mut rig.held) {
                    if let Some(a) = rig.answer(node, op) {
                        rig.server.node(a, Ms(rig.now));
                    }
                }
            }
            move_head(rig, node, 1_000 + step);
            tab.lend(rig, node, |db| db.store_mut().sync());
            match tab.ended.remove(&t) {
                None => continue,
                Some(craftworks_sdk::Ended::Loaded) => tab.db.store_mut().resume(t),
                Some(craftworks_sdk::Ended::Superseded) => superseded += 1,
                Some(other) => panic!("the read ended {other:?}"),
            }
            break;
        }
    }
}

/// **A READ RECEIVING BLOCKS FINISHES ON ITS TREE, HOWEVER FAST THE HEAD MOVES** (#330 ruling, progress-based). Every
/// block is there, one step late; the head moves at EVERY step. Each step brings the read a block, so it is never
/// superseded, and it answers. (The mutant "supersede on any GET in flight" restarts it at every head: it never
/// answers.)
#[test]
fn a_read_receiving_blocks_finishes_on_its_tree_under_fast_heads() {
    let (mut node, mut rig, mut tab) = published_rows_and_reader(72, |_| Default::default());
    let (v, superseded) = read_under_moving_heads(&mut tab, &mut rig, &mut node, b"k/000321", 40, |_| true);
    assert_eq!(v, Some(Some(b"value 321".to_vec())), "the read did not answer in 40 steps with the head moving at each ({superseded} superseded)");
    assert_eq!(superseded, 0, "a read that received a block at every step was superseded");
}

/// **SLOWER BLOCKS, THE SAME HEADS: THE READ STILL ANSWERS.** Blocks come every OTHER step, the head moves at every
/// one. A (re)started read with nothing arrived yet is superseded at the next head (that is the fix), and once its
/// first block has come it finishes on its tree. (Under "supersede on any GET in flight" it never answers.)
#[test]
fn a_read_with_slow_blocks_still_answers_under_fast_heads() {
    let (mut node, mut rig, mut tab) = published_rows_and_reader(73, |_| Default::default());
    let (v, superseded) = read_under_moving_heads(&mut tab, &mut rig, &mut node, b"k/000321", 60, |s| s % 2 == 0);
    assert_eq!(v, Some(Some(b"value 321".to_vec())), "the read did not answer in 60 steps ({superseded} superseded)");
    println!("  slow blocks, a head per step: answered after {superseded} supersede(s)");
}

/// sdk#415, THE BACKED-UP TIMELINE (engineer1's live run, forced): two rows saved and BACKED UP, then a third
/// written. A row's state is its OWN last write's fate (ruling A, rule 10: BACKED_UP is per commit): the first two
/// never regress while the third saves, and the third reaches BACKED UP once every PUT of its commit is acked.
/// The engine's bookkeeping settles too: no group owed, the parity scan Done.
#[test]
fn a_row_is_backed_up_by_its_own_write_and_a_later_save_never_demotes_it() {
    for case in ["no faults", "first parity answers lost", "first put answers lost", "first gets lost", "record not saved once"] {
        let faults = Faults {
            lose_first_parity_answers: case == "first parity answers lost",
            lose_first_put_answers: case == "first put answers lost",
            lose_first_gets: case == "first gets lost",
            record_not_saved_once: case == "record not saved once",
            ..Faults::default()
        };
        backed_up_timeline(case, faults);
    }
}

fn backed_up_timeline(case: &str, faults: Faults) {
    use craftworks_sdk::store::{Reads, RowState};
    let mut node = Node::new();
    let mut rig = PageRig::new();
    rig.faults = faults;
    let mut tab = Tab::open(&mut rig, &mut node);
    tab.call(&mut rig, &mut node, |db| db.define("t", &tab_schema())).expect("define");
    tab.pump(&mut rig, &mut node);
    let row = |tab: &mut Tab, rig: &mut PageRig, node: &mut Node, title: &str| {
        let rec = tab.call(rig, node, |db| db.put("t", &serde_json::json!({ "title": title }).as_object().unwrap().clone())).expect("put");
        tab.pump(rig, node);
        craftworks_sdk::db::record_key("t", craftworks_sdk::id::loc_from_hex(&rec.id).expect("an id"))
    };
    let state = |tab: &mut Tab, rig: &mut PageRig, node: &mut Node, key: &[u8]| tab.lend(rig, node, |db| Reads::row_state(db.store_mut(), key));
    let settle = |tab: &mut Tab, rig: &mut PageRig, node: &mut Node| {
        rig.run_for(node, 5_000);
        tab.pump(rig, node);
    };
    let alpha = row(&mut tab, &mut rig, &mut node, "alpha");
    let beta = row(&mut tab, &mut rig, &mut node, "beta");
    settle(&mut tab, &mut rig, &mut node);
    for (name, k) in [("alpha", &alpha), ("beta", &beta)] {
        assert_eq!(state(&mut tab, &mut rig, &mut node, k), RowState::BackedUp, "{case}: THE SETUP: {name} is not backed up before gamma");
    }
    // gamma's save RETURNS before its commit is backed up (its parity follows the head): alpha and beta are
    // looked at in that window, before anything settles -- the moment engineer1 saw all three read "saved".
    let rec = tab.call(&mut rig, &mut node, |db| db.put("t", &serde_json::json!({ "title": "gamma" }).as_object().unwrap().clone())).expect("put");
    let gamma = craftworks_sdk::db::record_key("t", craftworks_sdk::id::loc_from_hex(&rec.id).expect("an id"));
    let owed_mid = rig.server.page.owed_groups();
    // gamma's row, watched from here: its save returned, its own state change so far told and drained. From now
    // on every change of it must reach its bindings (`state_changed`), the move to backed up included.
    let _ = tab.db.store_mut().writes.take_state_changed();
    let mut gamma_seen = state(&mut tab, &mut rig, &mut node, &gamma);
    if case == "first parity answers lost" {
        assert_ne!(gamma_seen, RowState::BackedUp, "THE SETUP: gamma was backed up the moment it saved: the move to backed up is not watched");
    }
    for (name, k) in [("alpha", &alpha), ("beta", &beta)] {
        let s = state(&mut tab, &mut rig, &mut node, k);
        assert_eq!(s, RowState::BackedUp, "{case}: {name} REGRESSED to {s:?} as gamma saved ({owed_mid} group(s) owed by gamma's commit): a later save demoted a row its own commit backed up");
    }
    tab.pump(&mut rig, &mut node);
    // Watched for engineer1's 600 s, and PAST backing up: the idle page's backstop head reads (every
    // HEAD_BACKSTOP_MS) come after the save, and a row must stay backed up through them.
    let mut gamma_backed_at = None;
    for step in 0..120u64 {
        for (name, k) in [("alpha", &alpha), ("beta", &beta)] {
            let s = state(&mut tab, &mut rig, &mut node, k);
            assert_eq!(s, RowState::BackedUp, "{case}: {name} REGRESSED to {s:?} {} ms after gamma was written: a later save demoted a row its own commit backed up", step * 5_000);
        }
        let told = tab.db.store_mut().writes.take_state_changed();
        let g = state(&mut tab, &mut rig, &mut node, &gamma);
        // A row whose state CHANGED is told to its bindings (they re-read on it; builder#107): the move to backed
        // up moves no root, so without this the table shows "saved" for ever (engineer1's live run on sdk#415).
        if g != gamma_seen {
            assert!(told.contains(&gamma), "{case}: gamma moved {gamma_seen:?} -> {g:?} and its binding was not told (state_changed: {} key(s))", told.len());
            gamma_seen = g;
        }
        match (g == RowState::BackedUp, gamma_backed_at) {
            (true, None) => gamma_backed_at = Some(step),
            (false, Some(at)) => panic!("{case}: gamma REGRESSED to {g:?} at {} ms, after reading backed up at {} ms", step * 5_000, at * 5_000),
            _ => {}
        }
        settle(&mut tab, &mut rig, &mut node);
    }
    let gamma_backed = gamma_backed_at.is_some();
    let owed = rig.server.page.owed_groups();
    let scan = rig.server.page.parity_scan();
    println!("{case}: gamma backed up: {gamma_backed}; owed groups {owed}; parity scan {}", if matches!(scan, engine::ParityScan::Done { .. }) { "Done" } else { "not Done" });
    assert!(gamma_backed, "{case}: gamma never reached BACKED UP though every PUT of its commit was acked");
    assert_eq!(owed, 0, "{case}: every parity PUT acked, and groups are still owed");
    assert!(matches!(scan, engine::ParityScan::Done { .. }), "{case}: every parity PUT acked, and the parity scan is {scan:?}");
    // ANOTHER DEVICE then writes DIFFERENT bytes at gamma's key (#437): the row is no longer this page's write's --
    // it reads saved, not backed up, until that write's own fate is known here; alpha, untouched, stays backed up.
    elsewhere(&mut rig, &mut node, 99, vec![protocol::Op::Put(gamma.clone(), b"another device's bytes".to_vec())]);
    rig.server.head_hint();
    settle(&mut tab, &mut rig, &mut node);
    // Read as a binding re-reads: the other device's tree is cold here until fetched (a key the page cannot read
    // has no state yet, as before the fetch of any row).
    assert_eq!(tab.get(&mut rig, &mut node, &gamma).as_deref(), Some(b"another device's bytes".as_slice()), "{case}: THE SETUP: the other device's write is not what gamma's key holds");
    let _ = tab.get(&mut rig, &mut node, &alpha);
    let g = state(&mut tab, &mut rig, &mut node, &gamma);
    assert_ne!(g, RowState::BackedUp, "{case}: gamma's key holds another device's bytes, and still reads backed up by this page's write");
    assert_eq!(state(&mut tab, &mut rig, &mut node, &alpha), RowState::BackedUp, "{case}: a foreign write to gamma's key demoted alpha");
}

/// #437: the per-key last-write record is FIXED-SIZE -- a digest of the value, never the value (a copy would be a
/// second holder of the data, rule 3, and page memory growing with it). A 50 KB row costs it no more than a small one.
#[test]
fn the_last_write_record_keeps_a_digest_never_the_value() {
    assert!(std::mem::size_of::<page::server::LastWrite>() <= 64, "LastWrite is {} bytes", std::mem::size_of::<page::server::LastWrite>());
    let mut node = Node::new();
    let mut rig = PageRig::new();
    let mut tab = Tab::open(&mut rig, &mut node);
    tab.call(&mut rig, &mut node, |db| db.define("t", &tab_schema())).expect("define");
    tab.pump(&mut rig, &mut node);
    let big = "x".repeat(50_000);
    tab.call(&mut rig, &mut node, |db| db.put("t", &serde_json::json!({ "title": big }).as_object().unwrap().clone())).expect("put");
    tab.pump(&mut rig, &mut node);
    let bytes = rig.server.last_write_bytes();
    println!("last-write record after a 50 KB row: {bytes} B");
    assert!(bytes < 1_024, "the last-write record holds {bytes} B after a 50 KB row: it keeps the value");
}
