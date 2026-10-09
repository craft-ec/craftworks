//! THE PAGE PATH'S NODE: `page::Server` over a scripted client API.
//!
//! What a tab is after the switch-over: an in-page engine (`page::Page`)
//! answering the SDK's protocol (`page::Server`), whose client-API operations
//! reach a node. The node is scripted here and runs the REAL rules where a
//! model could be kinder than the truth:
//! * the head is signed by the REAL signer (`signer::serve_full`), under the
//!   secret it was provisioned with;
//! * every head UPDATE goes through the Register contract's own
//!   `update_state` (its merge, its tie-break);
//! * a block is stored under the contract id the node derives
//!   (`contract_keys::block::contract_for`).
//!
//! [`PageNode`] is the node, shared by every connection to it, as a node is
//! shared by every tab on it. [`PageConn`] is ONE tab's `page::Server`: its
//! own engine over the same blocks and the same head — never one engine two
//! tabs share.
//!
//! The knobs keep the names of the Shell fixture's (`FullNode`/`Conn`) so a
//! client test reads the same on either, and each is stated for what it does
//! HERE: `hold_answers` holds the node's ANSWERS (a put still lands when it
//! is made), `cold_over` is a second node that holds no block and answers a
//! GET from the first one's network.

use engine::Params;
use freenet_prolly::store::Blocks;
use freenet_prolly::Cid;
use page::server::{Server, SignerFacts};
use page::{Answer, Label, Ms, Op, Page, PutPath};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

/// The scripted node's Block contract code: only its HASH matters, as the
/// contract id is derived from it.
pub const BLOCK_CODE: &[u8] = b"testkit page-node block code";
/// The scripted node's Register contract code, likewise.
pub const REGISTER_CODE: &[u8] = b"testkit page-node register code";

type Map = BTreeMap<Cid, Vec<u8>>;

/// Which client-API op the node served, counted per kind.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Served {
    /// A block PUT.
    Put,
    /// A block GET.
    Get,
    /// A head UPDATE (the Register).
    Head,
    /// A head READ.
    ReadHead,
    /// A signer request.
    Sign,
}

/// The node: its blocks, the head Register, the signer's secrets.
///
/// Cloning shares it: every clone is the same node.
#[derive(Clone)]
pub struct PageNode {
    /// What this node holds locally.
    blocks: Rc<RefCell<Map>>,
    /// Where a GET is answered from when this node does not hold the block:
    /// the network behind a COLD node. `None`: every block is local.
    network: Option<Rc<RefCell<Map>>>,
    /// Contract id → block id, as the node names what it stores.
    contracts: Rc<RefCell<BTreeMap<[u8; 32], Cid>>>,
    register: Rc<RefCell<Option<Vec<u8>>>>,
    register_id: [u8; 32],
    register_params: Vec<u8>,
    secrets: Rc<RefCell<BTreeMap<Vec<u8>, Vec<u8>>>>,
    /// The signer fails to save its record on the next sign (once).
    record_fails: Rc<Cell<bool>>,
    /// GETs this node answered from the NETWORK (a cold node's misses).
    network_fetches: Rc<Cell<usize>>,
}

impl Blocks for PageNode {
    fn get(&self, cid: &Cid) -> Option<&[u8]> {
        // `Blocks` lends `&[u8]` from `&self`; the map is behind a RefCell, so
        // the bytes are leaked for the test's lifetime (a fixture, not a node).
        let b = self.blocks.borrow().get(cid).cloned()?;
        Some(Box::leak(b.into_boxed_slice()))
    }
}

/// The signer's host: the node's secrets and its synchronous contract read.
struct Host<'a>(&'a PageNode);
impl signer::Host for Host<'_> {
    fn get_secret(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.0.secrets.borrow().get(key).cloned()
    }
    fn set_secret(&mut self, key: &[u8], value: &[u8]) -> bool {
        if self.0.record_fails.get() && key == signer::RECORD {
            return false;
        }
        self.0.secrets.borrow_mut().insert(key.to_vec(), value.to_vec());
        true
    }
    fn contract_state(&self, id: &[u8; 32]) -> Option<Vec<u8>> {
        if *id == self.0.register_id {
            return self.0.register.borrow().clone();
        }
        let cid = *self.0.contracts.borrow().get(id)?;
        self.0.blocks.borrow().get(&cid).map(|b| state_of(&cid, b))
    }
}

/// A Block contract's state: `kind ‖ body`, the kind found by the hash.
fn state_of(id: &Cid, body: &[u8]) -> Vec<u8> {
    let kind = [freenet_prolly::kind::TREE_NODE, freenet_prolly::kind::RAW, freenet_prolly::kind::PARITY]
        .into_iter()
        .find(|k| freenet_prolly::block_id(*k, body) == *id)
        .expect("every block hashes under one kind");
    let mut s = vec![kind];
    s.extend_from_slice(body);
    s
}

impl Default for PageNode {
    fn default() -> Self {
        PageNode::new()
    }
}

impl PageNode {
    /// A node with its signer PROVISIONED (key, Register, Block code), as the
    /// page's first run leaves it.
    pub fn new() -> PageNode {
        let sk = ed25519_dalek::SigningKey::from_bytes(&[5u8; 32]);
        let params = wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME);
        let n = PageNode {
            blocks: Rc::default(),
            network: None,
            contracts: Rc::default(),
            register: Rc::default(),
            register_id: signer::register_id(REGISTER_CODE, &params),
            register_params: params.clone(),
            secrets: Rc::default(),
            record_fails: Rc::default(),
            network_fetches: Rc::default(),
        };
        let req = signer::Request::Provision {
            signing_key: sk.to_bytes().to_vec(),
            register_code: REGISTER_CODE.to_vec(),
            register_params: params,
            block_code: BLOCK_CODE.to_vec(),
        };
        assert_eq!(signer::serve(&mut Host(&n), &signer::encode_request(1, &req), signer::Origin::Local), signer::Answer::Provisioned);
        n
    }

    /// A COLD node over `writer`'s network: it holds no block, reads the same
    /// head Register under the same identity, and answers every GET from the
    /// network, keeping what it fetched (sdk#150's second device).
    pub fn cold_over(writer: &PageNode) -> PageNode {
        PageNode {
            blocks: Rc::default(),
            network: Some(writer.network.clone().unwrap_or_else(|| writer.blocks.clone())),
            contracts: writer.contracts.clone(),
            network_fetches: Rc::default(),
            ..writer.clone()
        }
    }

    /// A NEW tab on this node: its own `page::Server` over these blocks and
    /// this head.
    pub fn connect(&self) -> PageConn {
        self.connect_with(Params::default())
    }

    /// A new tab whose engine runs with these parameters.
    pub fn connect_with(&self, params: Params) -> PageConn {
        let now = 1_000;
        let page = Page::unstarted(params, PutPath::Page, Ms(now));
        PageConn(Rc::new(RefCell::new(ConnState {
            node: self.clone(),
            server: Server::new(page, SignerFacts { head_writable: true, head_id: self.register_id }),
            now,
            hold: false,
            held: VecDeque::new(),
            served: BTreeMap::new(),
            fetched: BTreeMap::new(),
            put_bytes: 0,
            replies: Vec::new(),
            inbox: Vec::new(),
            asked: BTreeMap::new(),
            lost_puts: BTreeMap::new(),
            lost_sent: BTreeMap::new(),
        })))
    }

    /// Store a block, under the contract id the node derives.
    pub fn put_block(&self, id: Cid, body: &[u8]) {
        self.contracts.borrow_mut().insert(contract_keys::block::contract_for(BLOCK_CODE, &id), id);
        self.blocks.borrow_mut().insert(id, body.to_vec());
    }

    /// How many blocks this node holds locally.
    pub fn blocks(&self) -> usize {
        self.blocks.borrow().len()
    }

    /// GETs this node could not answer locally and answered from the
    /// network. A tab's own GETs are counted per tab (`PageConn::served`):
    /// every tab has its own memory and asks its node, so THIS is the number
    /// that says whether the node kept what it fetched.
    pub fn network_fetches(&self) -> usize {
        self.network_fetches.get()
    }

    /// Whether this node holds `id` locally.
    pub fn holds(&self, id: &Cid) -> bool {
        self.blocks.borrow().contains_key(id)
    }

    /// The head the Register holds: its seq and ROOT.
    pub fn head(&self) -> Option<(u64, Cid)> {
        contract_keys::register::head_of(self.register.borrow().as_deref()?)
    }

    /// The head WHOLE, as page-io reads it: seq and value.
    pub fn head_read(&self) -> Option<page::HeadRead> {
        page::HeadRead::from_record(self.register.borrow().as_deref()?)
    }

    /// The signer fails to save its record on the next sign, once.
    pub fn fail_record_once(&self) {
        self.record_fails.set(true);
    }

    /// Every entry of the tree at `root`, or `None` if a block is missing.
    pub fn tree(&self, root: &Cid) -> Option<BTreeMap<Vec<u8>, Vec<u8>>> {
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

    fn update(&self, state: &[u8]) {
        use freenet_stdlib::prelude::*;
        let next = match self.register.borrow().as_ref() {
            None => state.to_vec(),
            Some(cur) => <craftec_register_contract::Register as ContractInterface>::update_state(
                Parameters::from(self.register_params.clone()),
                State::from(cur.clone()),
                vec![UpdateData::State(State::from(state.to_vec()))],
            )
            .expect("the register merged")
            .new_state
            .expect("a state")
            .as_ref()
            .to_vec(),
        };
        *self.register.borrow_mut() = Some(next);
    }

    fn sign(&self, id: u32, prev_seq: u64, prev_root: Cid, seq: u64, root: Cid, ledger: Vec<u8>) -> (u32, signer_proto::Answer) {
        let req = signer::Request::Sign {
            prev: signer::Head { seq: prev_seq, root: prev_root },
            next: signer::Next { seq, root, ledger },
            label: signer::Label::Head,
        };
        let served = signer::serve_full(&mut Host(self), &signer::encode_request(id, &req), signer::Origin::Local);
        wire::signer::read_answer(&signer::reply(&served)).expect("a signer answer reads back")
    }
}

/// ONE tab: a `page::Server` over the node, run to a standstill per call.
///
/// A HANDLE, as the Shell fixture's `Conn` was: clones are the same tab (a
/// test hands one to a store as its transport and keeps one to look).
#[derive(Clone)]
pub struct PageConn(Rc<RefCell<ConnState>>);

struct ConnState {
    node: PageNode,
    server: Server,
    /// The page's clock, in ms.
    now: u64,
    hold: bool,
    held: VecDeque<Answer>,
    served: BTreeMap<Served, usize>,
    fetched: BTreeMap<Cid, usize>,
    /// Bytes this tab PUT to the node (block bodies), counted where they land.
    put_bytes: u64,
    replies: Vec<Vec<u8>>,
    /// Replies not yet taken by a `PageStore` this tab is lent to
    /// (`page::server::Host`).
    inbox: Vec<Vec<u8>>,
    /// Requests sent and whether each has been answered, by `w<id>`/`r<id>`.
    asked: BTreeMap<String, bool>,
    /// Blocks whose next PUTs are ACKED AND NOT STORED, and how many sends each has left to lose: a node that
    /// acked a block and lost it (ws-drop's case on a real network, sdk#555's live step).
    lost_puts: BTreeMap<Cid, u32>,
    /// PUTs of each block this node acked and did NOT store (`lost_puts`).
    lost_sent: BTreeMap<Cid, u32>,
}

impl PageConn {

    /// The node this tab is on.
    pub fn node(&self) -> PageNode {
        self.0.borrow().node.clone()
    }

    /// The page's clock now.
    pub fn now_ms(&self) -> u64 {
        self.0.borrow().now_ms()
    }

    /// One client frame, run until the node has nothing left to answer.
    /// Returns every reply frame produced along the way.
    pub fn frame(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.0.borrow_mut().frame(bytes)
    }

    /// Several client frames that arrived TOGETHER — a burst made faster than
    /// a round trip — all handed to the Server before any node answer, then
    /// run to a standstill. What the Shell's `step(vec![Inbound::Client..])`
    /// was, so a write can meet a commit in flight.
    pub fn frames(&mut self, frames: &[Vec<u8>]) -> Vec<Vec<u8>> {
        self.0.borrow_mut().frames(frames)
    }

    /// A request from a client with NO session: a pre-v4 client.
    pub fn client(&mut self, r: &protocol::Request) -> Vec<Vec<u8>> {
        self.0.borrow_mut().client(r)
    }

    /// A request from a given SESSION speaking a given VERSION.
    pub fn client_as(&mut self, session: u64, version: u16, r: &protocol::Request) -> Vec<Vec<u8>> {
        self.0.borrow_mut().client_as(session, version, r)
    }

    /// The page's clock moves to `now_ms`: the page's own tick (its deadlines),
    /// then the SDK's `Tick` request, as a page sends it.
    pub fn tick_at(&mut self, now_ms: u64) -> Vec<Vec<u8>> {
        self.0.borrow_mut().tick_at(now_ms)
    }

    /// Queue the node's answers instead of delivering them (a put still
    /// lands when it is made); see [`PageConn::release_one`].
    pub fn hold_answers(&mut self) {
        self.0.borrow_mut().hold_answers()
    }

    /// Answers made from now on are delivered; what is queued stays queued.
    pub fn stop_holding(&mut self) {
        self.0.borrow_mut().stop_holding()
    }

    /// How many node answers are queued.
    pub fn held(&self) -> usize {
        self.0.borrow().held()
    }

    /// The next `times` PUTs of block `id` are acked and NOT stored (`u32::MAX`: every one), as a node that acked
    /// the block and lost it.
    pub fn lose_puts_of(&mut self, id: Cid, times: u32) {
        self.0.borrow_mut().lost_puts.insert(id, times);
    }

    /// PUTs of `id` acked and lost (`lose_puts_of`).
    pub fn lost_sends(&self, id: &Cid) -> u32 {
        self.0.borrow().lost_sent.get(id).copied().unwrap_or(0)
    }

    /// LOSE the oldest queued answer. `false` if nothing was queued.
    pub fn lose_one_held(&mut self) -> bool {
        self.0.borrow_mut().lose_one_held()
    }

    /// Deliver ONE queued answer; what it makes the page ask is answered (or
    /// queued again, while holding).
    pub fn release_one(&mut self) -> Vec<Vec<u8>> {
        self.0.borrow_mut().release_one()
    }

    /// How many of this op kind the node served.
    pub fn served(&self, what: Served) -> usize {
        self.0.borrow().served(what)
    }

    /// How many times this tab fetched THIS block.
    pub fn fetches_of(&self, id: &Cid) -> usize {
        self.0.borrow().fetches_of(id)
    }

    /// Every reply this tab's Server produced, decoded.
    pub fn replies(&self) -> Vec<protocol::Reply> {
        self.0.borrow().replies()
    }

    /// The block bytes this tab PUT to the node, summed where the node took
    /// them (not the page's own count).
    pub fn bytes_handed_to_this_node(&self) -> u64 {
        self.0.borrow().bytes_handed_to_this_node()
    }

    /// Requests sent on this tab that have had no answer yet.
    pub fn unanswered(&self) -> Vec<String> {
        self.0.borrow().unanswered()
    }

    /// This tab's Server, to look at or to drive directly.
    pub fn with_server<R>(&self, f: impl FnOnce(&mut Server) -> R) -> R {
        f(&mut self.0.borrow_mut().server)
    }
}


/// The tab as a `PageStore`'s host: every call runs the node to a standstill,
/// as a tab's pump does, and the replies wait for the store to take them.
impl page::server::Host for PageConn {
    fn with_server<R>(&mut self, f: impl FnOnce(&mut Server) -> R) -> R {
        let mut st = self.0.borrow_mut();
        let r = f(&mut st.server);
        let out = st.run();
        st.inbox.extend(out);
        r
    }
    fn peek<R>(&self, f: impl FnOnce(&Server) -> R) -> R {
        f(&self.0.borrow().server)
    }
    fn client(&mut self, frame: &[u8]) {
        let mut st = self.0.borrow_mut();
        let out = st.frame(frame);
        st.inbox.extend(out);
    }
    fn take_replies(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.0.borrow_mut().inbox)
    }
}

impl ConnState {
    /// The page's clock now.
    pub fn now_ms(&self) -> u64 {
        self.now
    }

    /// One client frame, run until the node has nothing left to answer.
    /// Returns every reply frame produced along the way.
    pub fn frame(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.frames(&[bytes.to_vec()])
    }

    /// Several client frames that arrived TOGETHER — a burst made faster than
    /// a round trip — all handed to the Server before any node answer, then
    /// run to a standstill. What the Shell's `step(vec![Inbound::Client..])`
    /// was, so a write can meet a commit in flight.
    pub fn frames(&mut self, frames: &[Vec<u8>]) -> Vec<Vec<u8>> {
        for bytes in frames {
            if let protocol::Incoming::Ok(env) = protocol::decode_request(bytes) {
                if let Some(k) = key_of(&env.body) {
                    self.asked.insert(k, false);
                }
            }
            self.server.client(bytes);
        }
        self.run()
    }

    /// A request from a client with NO session: a pre-v4 client.
    pub fn client(&mut self, r: &protocol::Request) -> Vec<Vec<u8>> {
        self.client_as(protocol::LEGACY_SESSION, protocol::SESSION_SINCE - 1, r)
    }

    /// A request from a given SESSION speaking a given VERSION.
    pub fn client_as(&mut self, session: u64, version: u16, r: &protocol::Request) -> Vec<Vec<u8>> {
        let frame = protocol::encode_session_request(version, session, r).expect("encodes");
        self.frame(&frame)
    }

    /// The page's clock moves to `now_ms`: the page's own tick (its deadlines),
    /// then the SDK's `Tick` request, as a page sends it.
    pub fn tick_at(&mut self, now_ms: u64) -> Vec<Vec<u8>> {
        self.now = self.now.max(now_ms);
        self.server.tick(Ms(self.now));
        let mut out = self.run();
        out.extend(self.client(&protocol::Request::Tick { now: protocol::tick_of(now_ms) }));
        out
    }

    /// Queue the node's answers instead of delivering them (a put still
    /// lands when it is made); see [`PageConn::release_one`].
    pub fn hold_answers(&mut self) {
        self.hold = true;
    }

    /// Answers made from now on are delivered; what is queued stays queued.
    pub fn stop_holding(&mut self) {
        self.hold = false;
    }

    /// How many node answers are queued.
    pub fn held(&self) -> usize {
        self.held.len()
    }

    /// LOSE the oldest queued answer. `false` if nothing was queued.
    pub fn lose_one_held(&mut self) -> bool {
        self.held.pop_front().is_some()
    }

    /// Deliver ONE queued answer; what it makes the page ask is answered (or
    /// queued again, while holding).
    pub fn release_one(&mut self) -> Vec<Vec<u8>> {
        let Some(a) = self.held.pop_front() else { return Vec::new() };
        // A cold node keeps what it fetched when the fetch completes: now.
        if let Answer::Got { id, bytes } = &a {
            if self.node.network.is_some() {
                self.node.put_block(*id, bytes);
            }
        }
        self.server.node(a, Ms(self.now));
        self.run()
    }

    /// How many of this op kind the node served.
    pub fn served(&self, what: Served) -> usize {
        self.served.get(&what).copied().unwrap_or(0)
    }

    /// How many times this tab fetched THIS block.
    pub fn fetches_of(&self, id: &Cid) -> usize {
        self.fetched.get(id).copied().unwrap_or(0)
    }

    /// The block bytes this tab PUT to the node, summed where the node took
    /// them (not the page's own count).
    pub fn bytes_handed_to_this_node(&self) -> u64 {
        self.put_bytes
    }

    /// Every reply this tab's Server produced, decoded.
    pub fn replies(&self) -> Vec<protocol::Reply> {
        self.replies.iter().filter_map(|b| protocol::decode_reply(b).ok()).collect()
    }

    /// Requests sent on this tab that have had no answer yet.
    pub fn unanswered(&self) -> Vec<String> {
        self.asked.iter().filter(|(_, done)| !**done).map(|(k, _)| k.clone()).collect()
    }

    /// Answer every op until none is left. When only waiting is left (a
    /// retry's deadline), let the clock pass it, bounded, as the page's own
    /// tick would; holding, silence is the answer.
    fn run(&mut self) -> Vec<Vec<u8>> {
        let mut out = self.take_replies();
        let mut idle = 0;
        for _ in 0..5_000 {
            let ops = self.server.take_ops();
            if ops.is_empty() {
                if self.hold || !self.server.page.waiting() || idle >= 40 {
                    break;
                }
                idle += 1;
                self.now = self.server.page.next_due().map_or(self.now + 1, |d| d.0.max(self.now + 1));
                self.server.tick(Ms(self.now));
                out.extend(self.take_replies());
                continue;
            }
            idle = 0;
            for op in ops {
                let a = self.answer(op);
                if self.hold {
                    self.held.extend(a);
                } else if let Some(a) = a {
                    if let Answer::Got { id, bytes } = &a {
                        if self.node.network.is_some() {
                            self.node.put_block(*id, bytes);
                        }
                    }
                    self.server.node(a, Ms(self.now));
                }
            }
            out.extend(self.take_replies());
        }
        out
    }

    fn take_replies(&mut self) -> Vec<Vec<u8>> {
        let r = self.server.take_replies();
        for b in &r {
            if let Ok(reply) = protocol::decode_reply(b) {
                if let Some(k) = answered(&reply) {
                    if let Some(a) = self.asked.get_mut(&k) {
                        *a = true;
                    }
                }
            }
        }
        self.replies.extend(r.iter().cloned());
        r
    }

    fn count(&mut self, what: Served) {
        *self.served.entry(what).or_insert(0) += 1;
    }

    fn answer(&mut self, op: Op) -> Option<Answer> {
        Some(match op {
            Op::Put { id, bytes } => {
                self.count(Served::Put);
                self.put_bytes += bytes.len() as u64;
                match self.lost_puts.get_mut(&id) {
                    Some(left) if *left > 0 => {
                        *left = left.saturating_sub(u32::from(*left != u32::MAX));
                        *self.lost_sent.entry(id).or_insert(0) += 1;
                    }
                    _ => self.node.put_block(id, &bytes),
                }
                Answer::PutOk(id)
            }
            Op::Get { id } => {
                self.count(Served::Get);
                *self.fetched.entry(id).or_insert(0) += 1;
                let local = self.node.blocks.borrow().get(&id).cloned();
                let far = || {
                    let b = self.node.network.as_ref().and_then(|n| n.borrow().get(&id).cloned());
                    if b.is_some() {
                        self.node.network_fetches.set(self.node.network_fetches.get() + 1);
                    }
                    b
                };
                match local.or_else(far) {
                    Some(bytes) => Answer::Got { id, bytes },
                    None => Answer::GetMissed(id),
                }
            }
            // This node serves the person's HEAD; a site (builder#117) is not its to answer.
            Op::Sign { label: Label::Site(_), .. } | Op::Update { label: Label::Site(_), .. } | Op::ReadHead { label: Label::Site(_) } => return None,
            Op::Sign { id, prev_seq, prev_root, seq, root, ledger, label: Label::Head } => {
                self.count(Served::Sign);
                let (id, answer) = self.node.sign(id, prev_seq, prev_root, seq, root, ledger);
                self.node.record_fails.set(false);
                Answer::Signer { id, answer }
            }
            Op::Update { label: Label::Head, state } => {
                self.count(Served::Head);
                self.node.update(&state);
                Answer::Updated { label: Label::Head }
            }
            Op::ReadHead { label: Label::Head } => {
                self.count(Served::ReadHead);
                Answer::Head { label: Label::Head, read: self.node.head_read() }
            }
            Op::AskHeld { batch, ids } => Answer::Held { batch, present: ids.iter().map(|id| self.node.holds(id)).collect() },
            // A node that takes every app PUT (a web container) it is sent.
            Op::PutApp { key } => Answer::AppPutOk(key),
            // Page-io's signer requests: the page alone never makes one.
            Op::Ext(_) => return None,
        })
    }
}

/// The key a request is tracked by in `unanswered`.
fn key_of(r: &protocol::Request) -> Option<String> {
    use protocol::Request as Q;
    match r {
        Q::Write { write_id, .. } | Q::Commit { write_id, .. } => Some(format!("w{write_id}")),
        Q::Get { req_id, .. } | Q::Range { req_id, .. } | Q::ChangesSince { req_id, .. } => Some(format!("r{req_id}")),
        _ => None,
    }
}

/// The request a reply ANSWERS, if it answers one: a write's terminal or
/// Published state, or a read's result (the Shell fixture's rule, kept).
fn answered(r: &protocol::Reply) -> Option<String> {
    use protocol::{Reply as R, WriteState as W};
    match r {
        R::WriteState { write_id, state } | R::SessionWriteState { write_id, state, .. }
            if state.terminal() || matches!(state, W::Published) =>
        {
            Some(format!("w{write_id}"))
        }
        R::Value { req_id, .. }
        | R::Page { req_id, .. }
        | R::Unavailable { req_id, .. }
        | R::Delta { req_id, .. }
        | R::FullReloadRequired { req_id, .. } => Some(format!("r{req_id}")),
        _ => None,
    }
}

