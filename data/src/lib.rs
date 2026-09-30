//! DATA (ARCHITECTURE §1): an ACCOUNT's table for one app: a TAIL, whose state is the tree root plus the rows written
//! since the last flush. It is the account's, not a node's: its key is the account's DATA key, which every node of
//! the account holds, so any of them reads and writes it, and losing a node loses nothing.
//!
//! The tail's params are the Register's: the data key and a label `t/<table>`. No site is in it: the table is the
//! account's, and every site the person allows (its grant, kept by the identity delegate) reads and writes the same
//! one — another developer's front end, or a second address for the same app. The address is derived, never looked
//! up.
//!
//! This holds each open tail's WRITER (`tail::Writer`, the SDK's one client of the contract) and feeds it every
//! state the node sends (a GET or a subscription push), verified in full. Signing is the identity delegate's: the
//! page takes `message` out, the delegate signs it, the signature comes back to `commit`.

use craftec_register_contract::wire::Signed;
use freenet_prolly::apply::ApplyError;
use freenet_prolly::range::{range, read_value, PageEnd, Range, RangeError};
use freenet_prolly::store::{MemBlocks, ReadError};
use freenet_prolly::Cid;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use tail::{Op, Unsigned, Writer};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A table's tail is FLUSHED into its tree once it holds this many rows: the tail stays small (every write re-signs
/// and every reader re-reads the whole tail), the tree holds the rest.
pub const FLUSH_AT: usize = 32;

/// The tail's own row naming the ROOT's parity blocks (the root's group of one, `engine::repair::root_parity`): the
/// root is the one block no parent's group covers, so its parity ids ride beside the root, in the tail, where a
/// reader missing the root still finds them. Hidden from the rows; never written into the tree.
pub const ROOT_PARITY: &[u8] = b"\0root-parity";

/// The tail's own row saying its tree is SEALED (ARCHITECTURE: sealing covers whole tree nodes): every tree block in
/// a Sealed contract at its address ([`block_address`]), sealed whole ([`seal_block`]), with the rows IN THE CLEAR
/// inside — so keys stay ordered (range reads work) and nothing of the tree's shape shows. A tree without it is from
/// before (Block contracts, rows sealed one by one): read as it is, and built again, sealed, by the next flush.
pub const SEALED_TREE: &[u8] = b"\0sealed-tree";

/// A sealed row's key and value, and a sealed tree block, start with WHICH KEY sealed them: `[1, 0]` the table's own
/// key (derived from the account's data key), or `[2, epoch]` the account's key of that MLS epoch. A row starting
/// with neither is plaintext, written before tables were sealed: read, then sealed over ([`Open::migrate`]).
pub const SEALED: u8 = 1;
pub const SEALED_EPOCH: u8 = 2;

/// Which key sealed something. The table's own key is for the `mls` channel (a node reads it before it holds any
/// epoch) and for an account with no group on this node; every other table is sealed with its epoch's key, so a node
/// removed from the account reads nothing written after. Ordered: an epoch is newer than the table's key, a later
/// epoch newer than an earlier one — rows are only ever sealed over UPWARD.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum KeyRef {
    Table,
    Epoch(u64),
}

impl KeyRef {
    pub fn header(self) -> Vec<u8> {
        match self {
            KeyRef::Table => vec![SEALED, 0],
            KeyRef::Epoch(e) => [&[SEALED_EPOCH][..], &e.to_be_bytes()].concat(),
        }
    }

    /// Which key sealed `raw`, and how long its header is. `None`: it is not sealed (plaintext), or its header is
    /// not one this code reads.
    pub fn of(raw: &[u8]) -> Option<(KeyRef, usize)> {
        match *raw.first()? {
            SEALED if raw.get(1) == Some(&0) => Some((KeyRef::Table, 2)),
            SEALED_EPOCH => Some((KeyRef::Epoch(u64::from_be_bytes(raw.get(1..9)?.try_into().ok()?)), 9)),
            _ => None,
        }
    }

    fn sealed(raw: &[u8]) -> bool {
        matches!(raw.first(), Some(&SEALED) | Some(&SEALED_EPOCH))
    }
}

fn subkey(tk: &[u8; 32], what: &str) -> [u8; 32] {
    blake3::derive_key(what, tk)
}

fn nonce(key: &[u8; 32], parts: &[&[u8]]) -> [u8; 24] {
    let mut h = blake3::Hasher::new_keyed(key);
    for p in parts {
        h.update(&(p.len() as u32).to_le_bytes());
        h.update(p);
    }
    h.finalize().as_bytes()[..24].try_into().expect("24")
}

fn aead(key: &[u8; 32], n: &[u8; 24], aad: &[u8], data: &[u8], open: bool) -> Option<Vec<u8>> {
    use chacha20poly1305::aead::{Aead, Payload};
    use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
    let c = XChaCha20Poly1305::new(key.into());
    let p = Payload { msg: data, aad };
    if open {
        c.decrypt(XNonce::from_slice(n), p).ok()
    } else {
        c.encrypt(XNonce::from_slice(n), p).ok()
    }
}

/// A row key, sealed DETERMINISTICALLY (the nonce from the key itself): the same row under the same key always seals
/// to the same bytes, so a write or delete of a row finds the row; nothing else about the key shows.
pub fn seal_key(tk: &[u8; 32], by: KeyRef, key: &[u8]) -> Vec<u8> {
    let head = by.header();
    let k = subkey(tk, "craftworks 2026-09-28 sealed row key");
    let n = nonce(&k, &[key]);
    [&head[..], &n, &aead(&k, &n, &head, key, false).expect("seals")].concat()
}

/// A row value, sealed and BOUND to its sealed key (a value moved to another row does not open).
pub fn seal_value(tk: &[u8; 32], by: KeyRef, sealed_key: &[u8], value: &[u8]) -> Vec<u8> {
    let k = subkey(tk, "craftworks 2026-09-28 sealed row value");
    let n = nonce(&k, &[sealed_key, value]);
    [&by.header()[..], &n, &aead(&k, &n, sealed_key, value, false).expect("seals")].concat()
}

pub fn open_key(tk: &[u8; 32], raw: &[u8]) -> Option<Vec<u8>> {
    let (_, hl) = KeyRef::of(raw)?;
    let (head, rest) = raw.split_at(hl);
    let (n, ct) = rest.split_at_checked(24)?;
    aead(&subkey(tk, "craftworks 2026-09-28 sealed row key"), n.try_into().ok()?, head, ct, true)
}

pub fn open_value(tk: &[u8; 32], sealed_key: &[u8], raw: &[u8]) -> Option<Vec<u8>> {
    let (_, hl) = KeyRef::of(raw)?;
    let (n, ct) = raw[hl..].split_at_checked(24)?;
    aead(&subkey(tk, "craftworks 2026-09-28 sealed row value"), n.try_into().ok()?, sealed_key, ct, true)
}

/// A tree block SEALED WHOLE (its Block state, `kind ‖ body`), bound to its id. The nonce comes from the key and the
/// id, and an id names exactly one content, so the same block under the same key is always the same bytes.
pub fn seal_block(tk: &[u8; 32], by: KeyRef, cid: &Cid, state: &[u8]) -> Vec<u8> {
    let head = by.header();
    let k = subkey(tk, "craftworks 2026-09-28 sealed block");
    let n = nonce(&k, &[cid]);
    [&head[..], &n, &aead(&k, &n, &[&head[..], cid].concat(), state, false).expect("seals")].concat()
}

pub fn open_block(tk: &[u8; 32], cid: &Cid, raw: &[u8]) -> Option<Vec<u8>> {
    let (_, hl) = KeyRef::of(raw)?;
    let (head, rest) = raw.split_at(hl);
    let (n, ct) = rest.split_at_checked(24)?;
    aead(&subkey(tk, "craftworks 2026-09-28 sealed block"), n.try_into().ok()?, &[head, cid].concat(), ct, true)
}

/// Where a sealed block lives: a keyed hash of its id under the table's own key (the Sealed contract's params). Only
/// the account names it; nobody else can tell which blocks are one table's, or link a block to the one it came from.
pub fn block_address(tk: &[u8; 32], cid: &Cid) -> [u8; 32] {
    *blake3::keyed_hash(&subkey(tk, "craftworks 2026-09-28 block address"), cid).as_bytes()
}

pub struct Open {
    pub params: Vec<u8>,
    /// The Tail contract's code (the node needs it for the first PUT) and its hash (what names the instance).
    pub code: std::sync::Arc<Vec<u8>>,
    pub code_hash: [u8; 32],
    pub writer: Writer,
    /// Whether the network holds this tail (a GET found it, or this page put it). Before, the first write is a PUT
    /// of the whole state; after, every write is an UPDATE carrying one delta.
    pub on_network: bool,
    pub pending: Option<Unsigned>,
    /// The tree's blocks this page holds (fetched and opened, or built by a flush), by id: `Blocks` bodies, in the
    /// clear.
    pub blocks: MemBlocks,
    /// A flush's new blocks, kept until its step is signed and sent: then they are this page's too.
    staged: Option<MemBlocks>,
    /// The table's own key, from the identity delegate: the key of [`KeyRef::Table`], and the key its sealed blocks'
    /// addresses come from. Without it nothing of the table reads, and nothing is written.
    pub table_key: Option<[u8; 32]>,
    /// The account's epoch keys for this table this page holds, and the epochs the identity had none for.
    epochs: BTreeMap<u64, [u8; 32]>,
    no_key: BTreeSet<u64>,
    /// The key new rows and blocks are sealed with.
    pub writes: Option<KeyRef>,
    /// Tail rows the last read found under an OLDER key than `writes` (or plaintext): by row, the stored keys it is
    /// under and its value. Sealed over by `migrate`; a write of the same row deletes them in its own step.
    stale: BTreeMap<Vec<u8>, (Vec<Vec<u8>>, Vec<u8>)>,
    /// Sealed blocks fetched before their epoch's key was held: opened when it is.
    locked: HashMap<Cid, Vec<u8>>,
    /// The key this tail is under: its writer's.
    pub writer_key: [u8; 32],
    /// A PUBLIC tail (a person's card, a board's public copies): rows in the clear, readable by anyone who can name
    /// it; never sealed — and its tree in the clear too (Block contracts named by their ids, as trees from before
    /// sealing), so it grows past what one tail holds.
    pub public: bool,
    /// What this page's flushes put, by block id (the tests' network).
    #[cfg(test)]
    sent: HashMap<Cid, Vec<u8>>,
}

/// A flush, ready: the blocks to put FIRST (by id, the state to store: sealed whole), then the step to sign.
pub struct Flush {
    /// Whether the blocks are sealed (at their addresses in Sealed contracts) or in the clear (a public tree: Block
    /// contracts named by their ids).
    pub sealed: bool,
    pub seq: u64,
    pub hash: [u8; 32],
    pub blocks: Vec<(Cid, Vec<u8>)>,
}

/// A group of a table's asset: `k` members, then their parity (none for a tree written before parity).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetGroup {
    pub slots: Vec<Cid>,
    pub k: usize,
}

/// What a flush or a read needs before it can go on.
pub enum Step<T> {
    Ready(T),
    /// These tree blocks, fetched and handed to `absorb_block`, then ask again.
    Need(Vec<Cid>),
    /// The keys of these epochs of the account (the identity's, for this table), handed to `epoch_key`, then ask
    /// again.
    Keys(Vec<u64>),
}

/// Every row in the clear, and what reading them found.
/// A page of rows (`Open::page`): in its order, and the key to continue after (`None`: done).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub rows: Vec<(Vec<u8>, Vec<u8>)>,
    pub next: Option<Vec<u8>>,
}

struct Rows {
    rows: BTreeMap<Vec<u8>, Vec<u8>>,
    stale: BTreeMap<Vec<u8>, (Vec<Vec<u8>>, Vec<u8>)>,
    unreadable: usize,
}

impl Open {
    /// Table `table` of the account whose data key is `key`.
    pub fn new(tail_code: &[u8], key: &[u8; 32], table: &str) -> Open {
        let params = contract_keys::register_params(key, &[b"t/".as_slice(), table.as_bytes()].concat());
        let writer = Writer::new(&params).expect("our own params parse");
        Open {
            params,
            code: std::sync::Arc::new(tail_code.to_vec()),
            code_hash: contract_keys::code_hash(tail_code),
            writer,
            on_network: false,
            pending: None,
            blocks: MemBlocks::default(),
            staged: None,
            table_key: None,
            epochs: BTreeMap::new(),
            no_key: BTreeSet::new(),
            writes: None,
            stale: BTreeMap::new(),
            locked: HashMap::new(),
            writer_key: *key,
            public: false,
            #[cfg(test)]
            sent: HashMap::new(),
        }
    }

    /// The tail's signed state as this page holds it (re-published as it is: the network keeps the newest).
    pub fn state(&self) -> Vec<u8> {
        self.writer.state()
    }

    /// The instance id, as the node writes it (base58).
    pub fn id(&self) -> String {
        bs58::encode(self.id_bytes()).into_string()
    }

    /// The instance id: `contract_keys::instance` (the node's derivation, pinned to the stdlib by the SDK's tests).
    pub fn id_bytes(&self) -> [u8; 32] {
        contract_keys::instance(&self.code_hash, &self.params)
    }

    /// The table's own key: rows and blocks under it read, and — until an epoch's key is given — writes use it.
    pub fn set_table_key(&mut self, key: [u8; 32]) {
        self.table_key = Some(key);
        self.writes.get_or_insert(KeyRef::Table);
    }

    /// An epoch's key for this table (`None`: the identity has none for it). `current`: new writes are sealed with it.
    /// Blocks that were waiting for it are opened now.
    pub fn epoch_key(&mut self, epoch: u64, key: Option<[u8; 32]>, current: bool) {
        match key {
            Some(k) => {
                self.epochs.insert(epoch, k);
                if current {
                    self.writes = Some(KeyRef::Epoch(epoch));
                }
            }
            None => {
                self.no_key.insert(epoch);
            }
        }
        let waiting: Vec<Cid> = self.locked.keys().copied().collect();
        for cid in waiting {
            let raw = self.locked.remove(&cid).expect("listed");
            self.absorb_block(&cid, &raw);
        }
    }

    fn key_for(&self, by: KeyRef) -> Option<[u8; 32]> {
        match by {
            KeyRef::Table => self.table_key,
            KeyRef::Epoch(e) => self.epochs.get(&e).copied(),
        }
    }

    /// Whether the tail's tree is sealed whole (see [`SEALED_TREE`]).
    pub fn sealed_tree(&self) -> bool {
        self.writer.current().is_some_and(|t| t.body.entries.get(SEALED_TREE).is_some_and(|e| e.value.is_some()))
    }

    /// Where the tree block `cid` lives: `(sealed, params)` — a Sealed contract at its address, or (a tree from
    /// before) the Block contract its id names.
    pub fn block_params(&self, cid: &Cid) -> Option<(bool, [u8; 32])> {
        if self.sealed_tree() {
            Some((true, block_address(&self.table_key?, cid)))
        } else {
            Some((false, *cid))
        }
    }

    /// Forget the local state: a write the network refused never landed, so the next state read is the truth.
    pub fn reset(&mut self) {
        self.writer = Writer::new(&self.params).expect("our own params parse");
        self.pending = None;
    }

    /// A state from the network. Taken only if it verifies and is not behind what this writer holds.
    pub fn absorb(&mut self, state: &[u8]) -> bool {
        self.on_network = true;
        match Writer::resume(&self.params, state) {
            Some(w) if w.seq() >= self.writer.seq() => {
                self.writer = w;
                true
            }
            _ => false,
        }
    }

    /// One row written (an empty value deletes it), SEALED with the key writes use: the step to sign. `None` without
    /// that key, or if the tail would refuse it. The row's copies under older keys are deleted in the same step. (A
    /// feed's value is already a version, shaped by the `feed` package; a delete there is a version, never empty.)
    pub fn prepare_row(&mut self, key: &[u8], value: &[u8]) -> Option<(u64, [u8; 32])> {
        if self.public {
            let op = if value.is_empty() { Op::Delete { key: key.to_vec() } } else { Op::Set { key: key.to_vec(), value: value.to_vec() } };
            return self.prepare(vec![op]);
        }
        let by = self.writes?;
        let tk = self.key_for(by)?;
        let sk = seal_key(&tk, by, key);
        let mut ops = vec![if value.is_empty() {
            Op::Delete { key: sk.clone() }
        } else {
            Op::Set { value: seal_value(&tk, by, &sk, value), key: sk.clone() }
        }];
        if let Some((old, _)) = self.stale.get(key) {
            ops.extend(old.iter().filter(|o| **o != sk).map(|o| Op::Delete { key: o.clone() }));
        }
        self.prepare(ops)
    }

    /// SEAL OVER up to `n` rows under an older key (or plaintext): each written under the key writes use and its old
    /// copies deleted, in ONE step. `None` when there are none (or no key).
    pub fn migrate(&mut self, n: usize) -> Option<(u64, [u8; 32])> {
        let by = self.writes?;
        let tk = self.key_for(by)?;
        let mut ops = Vec::new();
        for (k, (old, v)) in self.stale.iter().take(n) {
            let sk = seal_key(&tk, by, k);
            ops.push(Op::Set { value: seal_value(&tk, by, &sk, v), key: sk.clone() });
            ops.extend(old.iter().filter(|o| **o != sk).map(|o| Op::Delete { key: o.clone() }));
        }
        if ops.is_empty() {
            return None;
        }
        self.prepare(ops)
    }

    /// The next step: what to sign (the Register's message fields).
    pub fn prepare(&mut self, ops: Vec<Op>) -> Option<(u64, [u8; 32])> {
        let u = self.writer.prepare(ops)?;
        let out = (u.seq, u.body.hash());
        self.pending = Some(u);
        Some(out)
    }

    /// A block the network sent, kept if it is one this table asked for: its id is computed from the bytes, never
    /// taken on trust. A sealed tree's block is opened first (with the key its header names); one whose epoch's key
    /// is not held yet is kept until it is.
    pub fn absorb_block(&mut self, want: &Cid, state: &[u8]) -> bool {
        if !self.sealed_tree() {
            return self.take_block(want, state);
        }
        let Some((by, _)) = KeyRef::of(state) else { return false };
        let Some(tk) = self.key_for(by) else {
            self.locked.insert(*want, state.to_vec());
            return true;
        };
        match open_block(&tk, want, state) {
            Some(plain) => self.take_block(want, &plain),
            None => false,
        }
    }

    fn take_block(&mut self, want: &Cid, state: &[u8]) -> bool {
        match contract_keys::block::block_of_state(state) {
            Some((id, body)) if id == *want => {
                self.blocks.insert(id, body);
                true
            }
            _ => false,
        }
    }

    /// Blocks a read needs: to fetch, or (fetched, sealed under an epoch not held) the keys to get first.
    fn need<T>(&self, ids: Vec<Cid>) -> Result<Step<T>, String> {
        let (mut fetch, mut keys) = (Vec::new(), BTreeSet::new());
        for id in ids {
            match self.locked.get(&id).and_then(|raw| KeyRef::of(raw)) {
                None => fetch.push(id),
                Some((KeyRef::Epoch(e), _)) if !self.no_key.contains(&e) => {
                    keys.insert(e);
                }
                Some((by, _)) => return Err(format!("a tree block is sealed with a key this node does not hold ({by:?})")),
            }
        }
        Ok(if keys.is_empty() { Step::Need(fetch) } else { Step::Keys(keys.into_iter().collect()) })
    }

    /// Pending rows in the tail (its own rows are not ones).
    pub fn pending_rows(&self) -> usize {
        self.writer.body().entries.keys().filter(|k| k.first() != Some(&0)).count()
    }

    /// The root's parity ids, as the tail names them.
    pub fn root_parity_ids(&self) -> Vec<Cid> {
        let body = self.writer.body();
        let Some(e) = body.entries.get(ROOT_PARITY) else { return Vec::new() };
        e.value.as_deref().unwrap_or_default().chunks_exact(32).map(|c| c.try_into().expect("32")).collect()
    }

    /// The table's ASSET (phase 4, Lifecycle: what keeping it keeps): every block its current tree reaches, in the
    /// groups they are coded in — the root's group of one, then each node's groups (a branch's children, a leaf's
    /// values), each `k` members then their parity. `Need` the nodes not held yet (the walk descends through them).
    pub fn asset(&self) -> Result<Step<Vec<AssetGroup>>, String> {
        let Some(root) = self.writer.body().root else { return Ok(Step::Ready(Vec::new())) };
        let mut groups = vec![AssetGroup { slots: [vec![root], self.root_parity_ids()].concat(), k: 1 }];
        let (mut stack, mut seen, mut need) = (vec![root], BTreeSet::new(), Vec::new());
        while let Some(cid) = stack.pop() {
            if !seen.insert(cid) {
                continue;
            }
            let Some(bytes) = self.blocks.0.get(&cid) else {
                need.push(cid);
                continue;
            };
            let node = freenet_prolly::node::Node::parse(bytes).map_err(|e| format!("tree node {} does not parse: {e:?}", hex(&cid)))?;
            let ids: Vec<Cid> = node.parity().collect();
            let pn = freenet_prolly::parity::PARITY;
            for (g, (_, members)) in freenet_prolly::parity::group_members(&node).into_iter().enumerate() {
                let par = ids.get(pn * g..pn * (g + 1)).unwrap_or(&[]);
                let k = members.len();
                groups.push(AssetGroup { slots: [members, par.to_vec()].concat(), k });
            }
            if !node.is_leaf() {
                for i in 0..node.len() {
                    stack.push(node.child(i).0);
                }
            }
        }
        if !need.is_empty() {
            return self.need(need);
        }
        Ok(Step::Ready(groups))
    }

    /// A block of the asset AS STORED — to put again (re-publishing it, or repairing it where it is missing): held
    /// here, it is its block state (sealed at its address for a sealed tree, with the key writes use: the nonce comes
    /// from the key and the id, so the bytes are the same as the first put's under that key); a PARITY block is coded
    /// again from its group's members, all held. `(sealed, state)`; `None`: neither can be made here.
    pub fn stored(&self, cid: &Cid, groups: &[AssetGroup]) -> Option<(bool, Vec<u8>)> {
        let state = match self.blocks.0.get(cid) {
            Some(body) => contract_keys::block::block_state(cid, body)?,
            None => {
                let g = groups.iter().find(|g| g.slots[g.k..].contains(cid))?;
                let members: Option<Vec<Vec<u8>>> = g.slots[..g.k]
                    .iter()
                    .map(|m| self.blocks.0.get(m).and_then(|b| contract_keys::block::block_state(m, b)))
                    .collect();
                let p = freenet_prolly::parity::encode_group(&members?).ok()?.into_iter().find(|p| freenet_prolly::block_id(freenet_prolly::kind::PARITY, p) == *cid)?;
                contract_keys::block::block_state(cid, &p)?
            }
        };
        if !self.sealed_tree() {
            return Some((false, state));
        }
        let by = self.writes?;
        Some((true, seal_block(&self.key_for(by)?, by, cid, &state)))
    }

    /// REPAIR: the group a missing tree block can be rebuilt from — the root's group of one, or the group a held
    /// node lists it in. `None`: nothing held names a group for it.
    pub fn repair_group(&self, missing: &Cid) -> Option<engine::repair::Group> {
        let root = self.writer.body().root?;
        if *missing == root {
            let ids = self.root_parity_ids();
            return (!ids.is_empty()).then(|| engine::repair::root_group(root, &ids));
        }
        engine::repair::find_group(&self.blocks, root, *missing)
    }

    /// Rebuild `missing` from what is held of its group; kept only if it hashes to `missing` (`engine::repair::rebuild`).
    pub fn rebuild(&mut self, group: &engine::repair::Group) -> Result<(), String> {
        let have: Vec<Option<Vec<u8>>> =
            group.slots.iter().enumerate().map(|(i, c)| self.blocks.0.get(c).map(|b| group.stored(i, b))).collect();
        let body = engine::repair::rebuild(group, &have)?;
        self.blocks.insert(group.missing, &body);
        Ok(())
    }

    /// Whether the tree is one from before sealing whole: the next flush builds it again, sealed.
    pub fn reseal_tree(&self) -> bool {
        !self.public && self.writer.current().is_some_and(|t| t.body.root.is_some()) && !self.sealed_tree()
    }

    /// FLUSH: write the tail's rows — opened: the tree holds them in the clear, inside sealed blocks — into the tree
    /// (the SDK's `flush_into`, the tree library's own `apply`), and prepare the step that names the new root. A tree
    /// from before is built again whole, sealed. The blocks go out FIRST: a tail must never name a root whose blocks
    /// are not there. `Need` when the old tree's blocks along the edited paths are not held yet; `Keys` when a row is
    /// sealed under an epoch whose key is not held yet.
    pub fn flush(&mut self) -> Result<Step<Flush>, String> {
        // A public tail: its tree in the clear, no key.
        let seal = if self.public {
            None
        } else {
            let by = self.writes.ok_or("this table's key is not held here")?;
            Some((by, self.key_for(by).ok_or("this table's key is not held here")?))
        };
        let reseal = self.reseal_tree();
        if self.pending_rows() == 0 && !reseal {
            return Err("nothing to flush".into());
        }
        let body = self.writer.body();
        let through = self.writer.seq();
        let mut plain = body.clone();
        plain.entries.clear();
        let mut staging = if reseal {
            let all = match self.collect()? {
                Step::Ready(r) => r.rows,
                Step::Need(x) => return Ok(Step::Need(x)),
                Step::Keys(k) => return Ok(Step::Keys(k)),
            };
            plain.root = None;
            for (k, v) in all {
                plain.entries.insert(k, tail::Entry { seq: through, value: Some(v) });
            }
            MemBlocks::default()
        } else {
            let (rows, keys) = self.tail_rows(&body);
            if !keys.is_empty() {
                return Ok(Step::Keys(keys));
            }
            for (k, seq, v) in rows {
                plain.entries.insert(k, tail::Entry { seq, value: v });
            }
            self.blocks.clone()
        };
        let (applied, op) = match tail::flush_into(&mut staging, &plain, through) {
            Ok(x) => x,
            Err(ApplyError::Read(ReadError::Need(ids))) => return self.need(ids),
            Err(e) => return Err(format!("the tree would not take the rows: {e:?}")),
        };
        for (cid, bytes) in &applied.parity {
            staging.insert(*cid, bytes);
        }
        // The root's group of one: parity no parent lists, so its ids go in the tail beside the root.
        let root_bytes = staging.0.get(&applied.root).ok_or("the new root is not among the blocks")?.clone();
        let root_parity = engine::repair::root_parity(&root_bytes).ok_or("the root's parity would not code")?;
        let mut ids = Vec::new();
        for (cid, bytes) in &root_parity {
            staging.insert(*cid, bytes);
            ids.extend_from_slice(cid);
        }
        let mut blocks = Vec::new();
        for (cid, body) in staging.0.iter().filter(|(c, _)| reseal || !self.blocks.0.contains_key(*c)) {
            let state = contract_keys::block::block_state(cid, body).ok_or_else(|| format!("block {} is of no known kind", hex(cid)))?;
            blocks.push((*cid, match seal {
                Some((by, tk)) => seal_block(&tk, by, cid, &state),
                None => state,
            }));
        }
        let mut ops = vec![op, Op::Set { key: ROOT_PARITY.to_vec(), value: ids }];
        if seal.is_some() {
            ops.push(Op::Set { key: SEALED_TREE.to_vec(), value: vec![1] });
        }
        let (seq, hash) = self.prepare(ops).ok_or("the tail would refuse the flush")?;
        self.staged = Some(staging);
        Ok(Step::Ready(Flush { sealed: seal.is_some(), seq, hash, blocks }))
    }

    /// The signature for the prepared step. Returns what to send: `Put(state)` if the network has no tail yet, else
    /// `Update(delta)`.
    pub fn commit(&mut self, sig: [u8; 64]) -> Option<Send> {
        let u = self.pending.take()?;
        let signed = Signed { terminal: false, seq: u.seq, value_hash: u.body.hash(), bitmap: 0, sigs: vec![sig] };
        let delta = self.writer.commit(u, signed)?;
        if let Some(b) = self.staged.take() {
            self.blocks = b;
        }
        Some(if self.on_network {
            Send::Update(delta)
        } else {
            self.on_network = true;
            Send::Put(self.writer.state())
        })
    }

    /// One stored row opened: `Some((row key, value))` (a delete's value `None`), `None` if its key is not held. A
    /// plaintext row is as it is.
    fn open_row(&self, k: &[u8], v: Option<&[u8]>) -> Option<(Vec<u8>, Option<Vec<u8>>)> {
        let Some((by, _)) = KeyRef::of(k) else {
            return (!KeyRef::sealed(k)).then(|| (k.to_vec(), v.map(<[u8]>::to_vec)));
        };
        let tk = self.key_for(by)?;
        let pk = open_key(&tk, k)?;
        let pv = match v {
            Some(v) => Some(open_value(&tk, k, v)?),
            None => None,
        };
        Some((pk, pv))
    }

    /// The tail's rows in the clear, in the order they were written (at one step, deletions first: a row sealed over
    /// in one step is written after its old copy is deleted), as `(row, seq, value)`; and the epochs whose keys are
    /// needed first to open the rest.
    fn tail_rows(&self, body: &tail::Body) -> (Vec<(Vec<u8>, u64, Option<Vec<u8>>)>, Vec<u64>) {
        let mut list: Vec<_> = body.entries.iter().filter(|(k, _)| k.first() != Some(&0)).collect();
        list.sort_by_key(|(_, e)| (e.seq, e.value.is_some()));
        let (mut rows, mut keys) = (Vec::new(), BTreeSet::new());
        for (k, e) in list {
            match self.open_row(k, e.value.as_deref()) {
                Some((pk, pv)) => rows.push((pk, e.seq, pv)),
                None => {
                    if let Some((KeyRef::Epoch(x), _)) = KeyRef::of(k) {
                        if !self.no_key.contains(&x) && !self.epochs.contains_key(&x) {
                            keys.insert(x);
                        }
                    }
                }
            }
        }
        (rows, keys.into_iter().collect())
    }

    /// Every row in the clear: the TREE's (at the tail's root) with the TAIL's over them (a pending delete hides a
    /// tree row). `Need` names the tree blocks to fetch first, `Keys` the epochs whose keys to get first.
    fn collect(&mut self) -> Result<Step<Rows>, String> {
        let body = self.writer.body();
        let (tail_rows, keys) = self.tail_rows(&body);
        if !keys.is_empty() {
            return Ok(Step::Keys(keys));
        }
        let mut all: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
        let mut unreadable = 0usize;
        if let Some(root) = &body.root {
            let mut tree = BTreeMap::new();
            let mut r = Range { max_entries: 4096, max_bytes: 4 << 20, ..Range::default() };
            loop {
                let page = match range(&self.blocks, root, &r) {
                    Ok(p) => p,
                    Err(RangeError::Read(ReadError::Need(ids))) => return self.need(ids),
                    Err(e) => return Err(format!("the tree does not read: {e:?}")),
                };
                if page.end == PageEnd::Blocked {
                    return self.need(page.need.clone());
                }
                for (k, v) in &page.entries {
                    match read_value(&self.blocks, *v) {
                        Ok(b) => {
                            tree.insert(k.clone(), b.to_vec());
                        }
                        Err(ReadError::Need(ids)) => return self.need(ids),
                        Err(e) => return Err(format!("a value does not read: {e:?}")),
                    }
                }
                match (&page.end, &page.next) {
                    (PageEnd::Limit, Some(n)) => r.after = Some(n.clone()),
                    _ => break,
                }
            }
            if self.sealed_tree() {
                // Sealed whole: its rows are in the clear.
                all = tree;
            } else {
                // A tree from before: rows sealed one by one (or plaintext), opened as the tail's are.
                for (k, v) in tree {
                    match self.open_row(&k, Some(&v)) {
                        Some((pk, Some(pv))) => {
                            all.insert(pk, pv);
                        }
                        _ => unreadable += 1,
                    }
                }
            }
        }
        // The tail over the tree, in the order it was written; and each row's copies under keys older than writes'.
        let mut under: BTreeMap<Vec<u8>, Vec<Vec<u8>>> = BTreeMap::new();
        for (pk, _, pv) in tail_rows {
            match pv {
                Some(v) => all.insert(pk, v),
                None => all.remove(&pk),
            };
        }
        for (k, e) in body.entries.iter().filter(|(k, _)| k.first() != Some(&0)) {
            if e.value.is_none() {
                continue;
            }
            let older = self.writes.is_some_and(|w| KeyRef::of(k).map(|(by, _)| by) < Some(w));
            match self.open_row(k, None) {
                Some((pk, _)) if older => under.entry(pk).or_default().push(k.clone()),
                Some(_) => {}
                None => unreadable += 1,
            }
        }
        let stale = under.into_iter().filter_map(|(pk, old)| all.get(&pk).map(|v| (pk, (old, v.clone())))).collect();
        Ok(Step::Ready(Rows { rows: all, stale, unreadable }))
    }

    /// A PAGE of rows in the clear (phase 3, Reads): those with keys from `lo` (inclusive) to `hi` (exclusive), at most
    /// `limit` from the tree, newest-key first when `reverse`, continuing past `after` — the TREE's walked only along
    /// the page's path (its other blocks never fetched), the TAIL's over them in the span the page covers (a pending
    /// delete hides a tree row). With it, the key to continue from (`None`: the range is done). `Need`/`Keys` as for
    /// `rows`. A tree from before whole sealing (rows sealed one by one: keys not in order) is read whole and cut.
    pub fn page(&mut self, lo: Option<Vec<u8>>, hi: Option<Vec<u8>>, reverse: bool, after: Option<Vec<u8>>, limit: usize) -> Result<Step<Page>, String> {
        use std::ops::Bound;
        let inside = |k: &[u8], lo: &Option<Vec<u8>>, hi: &Option<Vec<u8>>| lo.as_deref().is_none_or(|l| k >= l) && hi.as_deref().is_none_or(|h| k < h);
        let body = self.writer.body();
        let (tail_rows, keys) = self.tail_rows(&body);
        if !keys.is_empty() {
            return Ok(Step::Keys(keys));
        }
        let mut rows: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
        let mut next = None;
        if let Some(root) = body.root {
            if !self.sealed_tree() {
                return Ok(match self.collect()? {
                    Step::Ready(r) => {
                        let mut all: Vec<_> = r.rows.into_iter().filter(|(k, _)| inside(k, &lo, &hi) && after.as_deref().is_none_or(|a| if reverse { k.as_slice() < a } else { k.as_slice() > a })).collect();
                        if reverse {
                            all.reverse();
                        }
                        Step::Ready(Page { rows: all, next: None })
                    }
                    Step::Need(x) => Step::Need(x),
                    Step::Keys(k) => Step::Keys(k),
                });
            }
            let r = Range {
                lo: lo.clone().map_or(Bound::Unbounded, Bound::Included),
                hi: hi.clone().map_or(Bound::Unbounded, Bound::Excluded),
                reverse,
                after: after.clone(),
                max_entries: limit.max(1),
                max_bytes: 4 << 20,
                ..Range::default()
            };
            let page = match range(&self.blocks, &root, &r) {
                Ok(p) => p,
                Err(RangeError::Read(ReadError::Need(ids))) => return self.need(ids),
                Err(e) => return Err(format!("the tree does not read: {e:?}")),
            };
            if page.end == PageEnd::Blocked {
                return self.need(page.need.clone());
            }
            for (k, v) in &page.entries {
                match read_value(&self.blocks, *v) {
                    Ok(b) => {
                        rows.insert(k.clone(), b.to_vec());
                    }
                    Err(ReadError::Need(ids)) => return self.need(ids),
                    Err(e) => return Err(format!("a value does not read: {e:?}")),
                }
            }
            if page.end == PageEnd::Limit {
                next = page.next.clone();
            }
        }
        // The TAIL over the tree, in the SPAN this page covers: from where it started to where the tree page stopped
        // (the whole rest of the range when it did not stop).
        let started = |k: &[u8]| after.as_deref().is_none_or(|a| if reverse { k < a } else { k > a });
        let within = |k: &[u8]| match &next {
            None => true,
            Some(n) => if reverse { k >= n.as_slice() } else { k <= n.as_slice() },
        };
        for (pk, _, pv) in tail_rows {
            if !inside(&pk, &lo, &hi) || !started(&pk) || !within(&pk) {
                continue;
            }
            match pv {
                Some(v) => rows.insert(pk, v),
                None => rows.remove(&pk),
            };
        }
        let mut out: Vec<_> = rows.into_iter().collect();
        if reverse {
            out.reverse();
        }
        Ok(Step::Ready(Page { rows: out, next }))
    }

    /// Every row in the clear, as stored (a feed's: versions), for the merge. `Need`/`Keys` as for `rows`.
    pub fn opened(&mut self) -> Result<Step<BTreeMap<Vec<u8>, Vec<u8>>>, String> {
        Ok(match self.collect()? {
            Step::Ready(r) => {
                self.stale = r.stale;
                Step::Ready(r.rows)
            }
            Step::Need(x) => Step::Need(x),
            Step::Keys(k) => Step::Keys(k),
        })
    }

    /// The rows, for the page. `Need` names the tree blocks to fetch first, `Keys` the epochs whose keys to get
    /// first.
    pub fn rows(&mut self) -> Result<Step<Value>, String> {
        let r = match self.collect()? {
            Step::Ready(r) => r,
            Step::Need(x) => return Ok(Step::Need(x)),
            Step::Keys(k) => return Ok(Step::Keys(k)),
        };
        self.stale = r.stale;
        let body = self.writer.body();
        let rows: Vec<Value> =
            r.rows.iter().map(|(k, v)| json!({ "key": String::from_utf8_lossy(k), "value": String::from_utf8_lossy(v) })).collect();
        let writes = match self.writes {
            Some(KeyRef::Epoch(e)) => json!(format!("epoch {e}")),
            Some(KeyRef::Table) => json!("table"),
            None => Value::Null,
        };
        Ok(Step::Ready(json!({
            "id": self.id(), "seq": self.writer.seq(), "rows": rows, "root": body.root.map(|r| hex(&r)),
            "pending": self.pending_rows(), "legacy": self.stale.len(), "unreadable": r.unreadable,
            "resealTree": self.reseal_tree(), "sealedTree": self.sealed_tree(), "writes": writes,
        })))
    }
}


pub enum Send {
    Put(Vec<u8>),
    Update(Vec<u8>),
}


#[cfg(test)]
mod tests {
    use super::*;
    use craftec_register_contract::wire::Params;
    use craftec_register_contract::wire::Signed as S;
    use ed25519_dalek::{Signer, SigningKey};

    const CODE: &[u8] = b"\0asm\x01\0\0\0";

    fn sign(key: &SigningKey, o: &Open, seq: u64, hash: [u8; 32]) -> [u8; 64] {
        let p = Params::parse(&o.params).unwrap();
        key.sign(&p.signed_message(false, seq, &hash)).to_bytes()
    }

    #[test]
    fn first_write_puts_the_state_then_deltas_and_another_writer_resumes() {
        let key = SigningKey::from_bytes(&[5; 32]);
        let member = key.verifying_key().to_bytes();
        let mut o = Open::new(CODE, &member, "notes");
        let (seq, h) = o.prepare(vec![Op::Set { key: b"a".to_vec(), value: b"1".to_vec() }]).unwrap();
        let Some(Send::Put(state)) = o.commit(sign(&key, &o, seq, h)) else { panic!("first write is a put") };
        let (seq, h) = o.prepare(vec![Op::Set { key: b"b".to_vec(), value: b"2".to_vec() }]).unwrap();
        assert!(matches!(o.commit(sign(&key, &o, seq, h)), Some(Send::Update(_))));
        let Ok(Step::Ready(v)) = o.rows() else { panic!("no tree yet: nothing to fetch") };
        assert_eq!(v["rows"].as_array().unwrap().len(), 2);
        // Another page of the same member reads the first state from the network.
        let mut other = Open::new(CODE, &member, "notes");
        assert!(other.absorb(&state));
        assert_eq!(other.writer.seq(), 1);
        // An older state never moves a writer back.
        assert!(!o.absorb(&state));
        assert_eq!(o.writer.seq(), 2);
    }

    fn write(key: &SigningKey, o: &mut Open, k: &str, v: &str) {
        let op = if v.is_empty() { Op::Delete { key: k.into() } } else { Op::Set { key: k.into(), value: v.into() } };
        let (seq, h) = o.prepare(vec![op]).unwrap();
        o.commit(sign(key, o, seq, h)).unwrap();
    }

    /// A tree block as the network holds it: sealed whole, under the key `o` writes with.
    fn net(o: &Open, id: &Cid) -> Vec<u8> {
        let by = o.writes.unwrap();
        let tk = o.key_for(by).unwrap();
        seal_block(&tk, by, id, &contract_keys::block::block_state(id, o.blocks.0.get(id).expect("the writer made it")).unwrap())
    }

    fn keys(v: &Value) -> Vec<String> {
        v["rows"].as_array().unwrap().iter().map(|r| format!("{}={}", r["key"].as_str().unwrap(), r["value"].as_str().unwrap())).collect()
    }

    #[test]
    fn a_flush_moves_the_rows_into_the_tree_and_a_reader_walks_tail_then_tree() {
        let key = SigningKey::from_bytes(&[5; 32]);
        let member = key.verifying_key().to_bytes();
        let mut o = Open::new(CODE, &member, "notes");
        o.set_table_key([7; 32]);
        for (k, v) in [("a", "1"), ("b", "2"), ("c", "3")] {
            write(&key, &mut o, k, v);
        }
        let Ok(Step::Ready(f)) = o.flush() else { panic!("a flush of held rows is ready") };
        assert!(!f.blocks.is_empty(), "the new tree's blocks go out first");
        o.commit(sign(&key, &o, f.seq, f.hash)).unwrap();
        assert_eq!(o.pending_rows(), 0, "the tail is empty after the flush");
        // After the flush: a new row in the tail, one tree row deleted from the tail.
        write(&key, &mut o, "d", "4");
        write(&key, &mut o, "b", "");
        let Ok(Step::Ready(v)) = o.rows() else { panic!("the writer holds its own blocks") };
        assert_eq!(keys(&v), ["a=1", "c=3", "d=4"]);
        // Another page: the state only. It walks tail -> tree root -> blocks it must fetch.
        let mut r = Open::new(CODE, &member, "notes");
        r.set_table_key([7; 32]);
        assert!(r.absorb(&o.writer.state()));
        let mut rounds = 0;
        let v = loop {
            match r.rows().unwrap() {
                Step::Ready(v) => break v,
                Step::Keys(k) => panic!("no epochs here: {k:?}"),
                Step::Need(ids) => {
                    rounds += 1;
                    assert!(rounds < 10, "the walk ends");
                    for id in ids {
                        assert!(r.absorb_block(&id, &net(&o, &id)));
                    }
                }
            }
        };
        assert!(rounds >= 1, "control: the reader had to fetch the tree");
        assert_eq!(keys(&v), ["a=1", "c=3", "d=4"]);
        // A block that is not the one asked for is refused.
        let id = *o.blocks.0.keys().next().unwrap();
        let other = [0u8; 32];
        assert!(!r.absorb_block(&other, &net(&o, &id)));
    }

    /// KEEP (phase 4): a READER — holding only what it fetched, no parity — lists the table's asset (every group its
    /// tree reaches) and makes every block of it AS STORED: members from what it holds, parity coded again from them.
    /// Each is byte for byte what the writer put, so re-publishing and repair need nothing the reader does not have.
    #[test]
    fn a_reader_lists_the_asset_and_makes_every_block_as_stored() {
        let key = SigningKey::from_bytes(&[5; 32]);
        let member = key.verifying_key().to_bytes();
        let mut o = Open::new(CODE, &member, "chat");
        o.set_table_key([7; 32]);
        for i in 0..300 {
            write(&key, &mut o, &format!("t{i:05}"), &format!("{i}-{}", "x".repeat(900)));
            if i % 50 == 49 {
                flush(&key, &mut o);
            }
        }
        let mut r = Open::new(CODE, &member, "chat");
        r.set_table_key([7; 32]);
        assert!(r.absorb(&o.writer.state()));
        let groups = loop {
            match r.asset().unwrap() {
                Step::Ready(g) => break g,
                Step::Keys(k) => panic!("no epochs here: {k:?}"),
                Step::Need(ids) => {
                    for id in ids {
                        assert!(r.absorb_block(&id, &o.sent[&id]));
                    }
                }
            }
        };
        assert!(groups.len() > 2, "the root's group and the nodes' groups");
        assert_eq!(groups[0].k, 1, "the root is a group of one");
        let root = o.writer.body().root.unwrap();
        let (mut members, mut parity, mut made) = (0, 0, 0);
        // A leaf's values are members too: fetched as a read would.
        for g in &groups {
            for c in &g.slots[..g.k] {
                if !r.blocks.0.contains_key(c) {
                    assert!(r.absorb_block(c, &o.sent[c]), "a value block the writer put");
                }
            }
        }
        for g in &groups {
            for (i, c) in g.slots.iter().enumerate() {
                if i < g.k { members += 1 } else { parity += 1 }
                let (sealed, st) = r.stored(c, &groups).unwrap_or_else(|| panic!("block {} could not be made", hex(c)));
                assert!(sealed);
                if let Some(sent) = o.sent.get(c) {
                    assert_eq!(&st, sent, "block {} as the writer put it", hex(c));
                    made += 1;
                } else {
                    assert_eq!(*c, root, "only the latest root may be absent from what the helper recorded");
                }
            }
        }
        assert!(parity > 0 && members > parity, "members {members}, parity {parity}");
        assert!(made >= members + parity - 1);
        // Control: a parity block whose group is not all held cannot be made.
        let g = groups.iter().find(|g| g.k > 1 && g.slots.len() > g.k).unwrap();
        let mut short = Open::new(CODE, &member, "chat");
        short.set_table_key([7; 32]);
        assert!(short.stored(&g.slots[g.k], &groups).is_none());
    }

    /// READS: a reader asks the LATEST page and fetches only the blocks on its path — fewer than a whole read; pages
    /// continue back to the start and, together, are every row (the tail's pending rows and deletes included).
    #[test]
    fn a_page_reads_only_its_path_and_pages_chain_to_every_row() {
        let key = SigningKey::from_bytes(&[5; 32]);
        let member = key.verifying_key().to_bytes();
        let mut o = Open::new(CODE, &member, "chat");
        o.set_table_key([7; 32]);
        for i in 0..600 {
            write(&key, &mut o, &format!("t{i:05}"), &format!("{i}-{}", "x".repeat(900)));
            if i % 40 == 39 {
                flush(&key, &mut o);
            }
        }
        // After the flush: one new row (the newest), one tree row deleted, one tree row changed — in the tail.
        write(&key, &mut o, "t00600", "newest");
        write(&key, &mut o, "t00598", "");
        write(&key, &mut o, "t00597", "changed");
        let fetches = |o: &Open, mut f: Box<dyn FnMut(&mut Open) -> Result<Step<Page>, String>>| -> (Page, usize) {
            let mut r = Open::new(CODE, &member, "chat");
            r.set_table_key([7; 32]);
            assert!(r.absorb(&o.writer.state()));
            let mut n = 0;
            loop {
                match f(&mut r).unwrap() {
                    Step::Ready(p) => return (p, n),
                    Step::Keys(k) => panic!("no epochs: {k:?}"),
                    Step::Need(ids) => {
                        n += ids.len();
                        assert!(n < 10_000);
                        for id in ids {
                            assert!(r.absorb_block(&id, &net(o, &id)));
                        }
                    }
                }
            }
        };
        let (latest, few) = fetches(&o, Box::new(|r: &mut Open| r.page(Some(b"t".to_vec()), None, true, None, 20)));
        let names = |p: &Page| p.rows.iter().map(|(k, _)| String::from_utf8_lossy(k).into_owned()).collect::<Vec<_>>();
        assert_eq!(names(&latest)[..4], ["t00600", "t00599", "t00597", "t00596"], "newest first; the tail's new row in, its delete out");
        assert_eq!(latest.rows[2].1, b"changed", "the tail's change over the tree's");
        // CONTROL: a whole read of the same table fetches more blocks.
        let (_, all) = fetches(
            &o,
            Box::new(|r: &mut Open| {
                Ok(match r.rows()? {
                    Step::Ready(_) => Step::Ready(Page { rows: vec![], next: None }),
                    Step::Need(x) => Step::Need(x),
                    Step::Keys(k) => Step::Keys(k),
                })
            }),
        );
        assert!(few < all, "the latest page fetched {few} blocks, a whole read {all}");
        println!("READS: the latest 20 of 600 rows fetched {few} blocks; a whole read {all}");
        // Pages chain back to the start: every row, once, newest first.
        let mut r = Open::new(CODE, &member, "chat");
        r.set_table_key([7; 32]);
        assert!(r.absorb(&o.writer.state()));
        let (mut seen, mut after) = (Vec::new(), None);
        loop {
            let p = loop {
                match r.page(Some(b"t".to_vec()), None, true, after.clone(), 50).unwrap() {
                    Step::Ready(p) => break p,
                    Step::Need(ids) => ids.iter().for_each(|id| assert!(r.absorb_block(id, &net(&o, id)))),
                    Step::Keys(k) => panic!("no epochs: {k:?}"),
                }
            };
            seen.extend(names(&p));
            match p.next {
                Some(n) => after = Some(n),
                None => break,
            }
        }
        assert_eq!(seen.len(), 600, "600 made, one added, one deleted");
        assert_eq!(seen.first().unwrap(), "t00600");
        assert_eq!(seen.last().unwrap(), "t00000");
        let mut sorted = seen.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        sorted.dedup();
        assert_eq!(sorted, seen, "in order, each once");
    }

    /// ERASURE: once a table's tree is past one leaf, its root lists parity for its children, and every one of
    /// those parity blocks is among what the flush PUT (none is owed and left unsent). A single-leaf tree has no
    /// children, so no parity: the control.
    #[test]
    fn a_tree_past_one_leaf_puts_parity_for_its_blocks() {
        let key = SigningKey::from_bytes(&[5; 32]);
        let mut o = Open::new(CODE, &key.verifying_key().to_bytes(), "notes");
        o.set_table_key([7; 32]);
        let mut put: std::collections::HashSet<Cid> = Default::default();
        let mut flush = |o: &mut Open, round: usize, n: usize, size: usize| {
            for i in 0..n {
                write(&key, o, &format!("{round:02}-{i:03}"), &"x".repeat(size));
            }
            let Ok(Step::Ready(f)) = o.flush() else { panic!("ready") };
            put.extend(f.blocks.iter().map(|(c, _)| *c));
            o.commit(sign(&key, o, f.seq, f.hash)).unwrap();
        };
        // Small: one leaf, a root with no children, so nothing to code.
        flush(&mut o, 0, 3, 10);
        let root = o.writer.body().root.unwrap();
        let node = freenet_prolly::store::load(&o.blocks, &root).unwrap();
        assert_eq!((node.level(), node.parity_count()), (0, 0), "control: a one-leaf tree has no parity");
        // Past one leaf.
        for round in 1..6 {
            flush(&mut o, round, FLUSH_AT, 200);
        }
        let root = o.writer.body().root.unwrap();
        let node = freenet_prolly::store::load(&o.blocks, &root).unwrap();
        assert!(node.level() >= 1, "the tree grew a branch");
        assert!(node.parity_count() >= freenet_prolly::parity::PARITY, "the root's children are coded");
        for p in node.parity() {
            assert!(put.contains(&p), "parity block {} was put by a flush", hex(&p));
        }
    }

    /// A reader of `o`'s state that is DENIED `lost` (the network no longer has it) and must repair it from its
    /// group. Returns the rows it ends with.
    fn read_without(o: &Open, member: &[u8; 32], lost: Cid) -> (Vec<String>, bool) {
        let mut r = Open::new(CODE, member, "notes");
        r.set_table_key([7; 32]);
        assert!(r.absorb(&o.writer.state()));
        let mut repaired = false;
        for _ in 0..20 {
            match r.rows().unwrap() {
                Step::Ready(v) => return (keys(&v), repaired),
                Step::Keys(k) => panic!("no epochs here: {k:?}"),
                Step::Need(ids) => {
                    for id in ids {
                        if id == lost {
                            let g = r.repair_group(&id).expect("a group names the lost block");
                            for (i, c) in g.slots.iter().enumerate() {
                                if *c != lost && i < g.slots.len() {
                                    if o.blocks.0.contains_key(c) {
                                        r.absorb_block(c, &net(o, c));
                                    }
                                }
                            }
                            r.rebuild(&g).expect("rebuilt and verified");
                            repaired = true;
                        } else {
                            r.absorb_block(&id, &net(o, &id));
                        }
                    }
                }
            }
        }
        panic!("the read did not finish");
    }

    #[test]
    fn a_one_leaf_tree_is_repairable_its_root_through_the_parity_the_tail_names() {
        let key = SigningKey::from_bytes(&[5; 32]);
        let member = key.verifying_key().to_bytes();
        let mut o = Open::new(CODE, &member, "notes");
        o.set_table_key([7; 32]);
        for (k, v) in [("a", "1"), ("b", "2")] {
            write(&key, &mut o, k, v);
        }
        let Ok(Step::Ready(f)) = o.flush() else { panic!("ready") };
        o.commit(sign(&key, &o, f.seq, f.hash)).unwrap();
        let root = o.writer.body().root.unwrap();
        let ids = o.root_parity_ids();
        assert_eq!(ids.len(), freenet_prolly::parity::PARITY, "the tail names the root's parity");
        let put: std::collections::HashSet<Cid> = f.blocks.iter().map(|(c, _)| *c).collect();
        assert!(ids.iter().all(|i| put.contains(i)), "and the flush put every one");
        assert_eq!(o.pending_rows(), 0, "the root-parity row is not a pending row");
        // The control: with the root there, no repair.
        let (rows, repaired) = read_without(&o, &member, [9; 32]);
        assert_eq!((rows.clone(), repaired), (vec!["a=1".to_string(), "b=2".to_string()], false));
        // The root lost: rebuilt from its parity, verified, and the rows are whole.
        let (rows2, repaired) = read_without(&o, &member, root);
        assert!(repaired, "the root was rebuilt");
        assert_eq!(rows2, rows);
        // A second flush clears the old row and names the new root's; the old parity never reaches the tree.
        write(&key, &mut o, "c", "3");
        let Ok(Step::Ready(f)) = o.flush() else { panic!("ready") };
        o.commit(sign(&key, &o, f.seq, f.hash)).unwrap();
        assert_ne!(o.root_parity_ids(), ids);
        let Ok(Step::Ready(v)) = o.rows() else { panic!("held") };
        assert_eq!(keys(&v), ["a=1", "b=2", "c=3"]);
    }

    #[test]
    fn a_lost_leaf_of_a_bigger_tree_is_rebuilt_from_its_group() {
        let key = SigningKey::from_bytes(&[5; 32]);
        let member = key.verifying_key().to_bytes();
        let mut o = Open::new(CODE, &member, "notes");
        o.set_table_key([7; 32]);
        for round in 0..6 {
            for i in 0..FLUSH_AT {
                write(&key, &mut o, &format!("{round:02}-{i:03}"), &"x".repeat(200));
            }
            let Ok(Step::Ready(f)) = o.flush() else { panic!("ready") };
            o.commit(sign(&key, &o, f.seq, f.hash)).unwrap();
        }
        let root = o.writer.body().root.unwrap();
        let node = freenet_prolly::store::load(&o.blocks, &root).unwrap();
        assert!(node.level() >= 1);
        let leaf = node.child(1).0;
        let (whole, _) = read_without(&o, &member, [9; 32]);
        let (rows, repaired) = read_without(&o, &member, leaf);
        assert!(repaired, "the leaf was rebuilt");
        assert_eq!(rows, whole);
        assert_eq!(rows.len(), 6 * FLUSH_AT);
    }

    fn contains(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }

    fn put_row(key: &SigningKey, o: &mut Open, k: &str, v: &str) {
        let (seq, h) = o.prepare_row(k.as_bytes(), v.as_bytes()).unwrap();
        o.commit(sign(key, o, seq, h)).unwrap();
    }

    #[test]
    fn a_sealed_table_shows_nothing_to_the_network_and_nothing_without_its_key() {
        let key = SigningKey::from_bytes(&[5; 32]);
        let member = key.verifying_key().to_bytes();
        let mut o = Open::new(CODE, &member, "notes");
        assert!(o.prepare_row(b"a", b"x").is_none(), "no key, no write");
        o.set_table_key([7; 32]);
        put_row(&key, &mut o, "shopping", "buy oat milk");
        put_row(&key, &mut o, "trip", "book the train");
        let Ok(Step::Ready(v)) = o.rows() else { panic!() };
        assert_eq!(keys(&v), ["shopping=buy oat milk", "trip=book the train"]);
        // What the network holds: none of it in plaintext.
        let state = o.writer.state();
        for secret in ["shopping", "buy oat milk", "trip", "book the train"] {
            assert!(!contains(&state, secret.as_bytes()), "{secret} is in the stored state");
        }
        // Control: the same rows written plaintext DO show — the check can see them.
        let mut plain = Open::new(CODE, &member, "plain");
        let (seq, h) = plain.prepare(vec![Op::Set { key: b"shopping".to_vec(), value: b"buy oat milk".to_vec() }]).unwrap();
        plain.commit(sign(&key, &plain, seq, h)).unwrap();
        assert!(contains(&plain.writer.state(), b"buy oat milk"));
        // Another reader: without the key, nothing; with the wrong key, nothing; with the key, all.
        let read = |seal: Option<[u8; 32]>| {
            let mut r = Open::new(CODE, &member, "notes");
            r.absorb(&o.writer.state());
            if let Some(k) = seal {
                r.set_table_key(k);
            }
            let Ok(Step::Ready(v)) = r.rows() else { panic!() };
            (keys(&v).len(), v["unreadable"].as_u64().unwrap())
        };
        assert_eq!(read(None), (0, 2));
        assert_eq!(read(Some([8; 32])), (0, 2));
        assert_eq!(read(Some([7; 32])), (2, 0));
        // Rewriting and deleting a row find it (the sealed key is the same every time).
        put_row(&key, &mut o, "trip", "booked");
        put_row(&key, &mut o, "shopping", "");
        let Ok(Step::Ready(v)) = o.rows() else { panic!() };
        assert_eq!(keys(&v), ["trip=booked"]);
        // Through a flush and the tree: still sealed, still readable.
        let Ok(Step::Ready(f)) = o.flush() else { panic!() };
        o.commit(sign(&key, &o, f.seq, f.hash)).unwrap();
        assert!(o.blocks.0.values().any(|b| contains(b, b"booked")), "control: the tree holds the row in the clear");
        assert!(f.blocks.iter().all(|(_, b)| !contains(b, b"booked") && !contains(b, b"trip")), "no block the network gets does");
        let Ok(Step::Ready(v)) = o.rows() else { panic!() };
        assert_eq!(keys(&v), ["trip=booked"]);
    }

    #[test]
    fn rows_from_before_sealing_are_read_then_sealed_over_without_doubles() {
        let key = SigningKey::from_bytes(&[5; 32]);
        let member = key.verifying_key().to_bytes();
        let mut o = Open::new(CODE, &member, "notes");
        for (k, v) in [("a", "plain-one"), ("b", "plain-two"), ("c", "plain-three")] {
            write(&key, &mut o, k, v); // plaintext, as before sealing
        }
        o.set_table_key([7; 32]);
        let Ok(Step::Ready(v)) = o.rows() else { panic!() };
        assert_eq!((keys(&v), v["legacy"].as_u64()), (vec!["a=plain-one".into(), "b=plain-two".into(), "c=plain-three".into()], Some(3)));
        // A write to an old row seals it and drops its plaintext in the same step: no double.
        put_row(&key, &mut o, "b", "two");
        let Ok(Step::Ready(v)) = o.rows() else { panic!() };
        assert_eq!((keys(&v), v["legacy"].as_u64()), (vec!["a=plain-one".into(), "b=two".into(), "c=plain-three".into()], Some(2)));
        // The rest, two at a time.
        while let Some((seq, h)) = o.migrate(2) {
            o.commit(sign(&key, &o, seq, h)).unwrap();
            o.rows().unwrap();
        }
        let Ok(Step::Ready(v)) = o.rows() else { panic!() };
        assert_eq!((keys(&v), v["legacy"].as_u64()), (vec!["a=plain-one".into(), "b=two".into(), "c=plain-three".into()], Some(0)));
        // No plaintext VALUE is left anywhere in the stored state (the old keys remain only as deletions until the
        // next flush applies them to the tree).
        let state = o.writer.state();
        for secret in ["plain-one", "plain-two", "plain-three"] {
            assert!(!contains(&state, secret.as_bytes()), "{secret} is still stored");
        }
        assert!(o.writer.body().entries.iter().all(|(k, e)| k.first() == Some(&SEALED) || e.value.is_none()), "only sealed rows carry values");
    }

    /// Read `o`'s state as another page holding `table` and the keys of `epochs` (and none for any other): the rows,
    /// how many it could not read, and whether the tree would not read at all.
    fn read_with(o: &Open, member: &[u8; 32], table: [u8; 32], epochs: &[(u64, [u8; 32])]) -> Result<(Vec<String>, u64), String> {
        let mut r = Open::new(CODE, member, "notes");
        r.set_table_key(table);
        assert!(r.absorb(&o.writer.state()));
        for _ in 0..20 {
            match r.rows()? {
                Step::Ready(v) => return Ok((keys(&v), v["unreadable"].as_u64().unwrap())),
                Step::Need(ids) => {
                    for id in ids {
                        assert!(r.absorb_block(&id, &o.sent[&id]));
                    }
                }
                Step::Keys(want) => {
                    for e in want {
                        r.epoch_key(e, epochs.iter().find(|(x, _)| *x == e).map(|(_, k)| *k), false);
                    }
                }
            }
        }
        panic!("the read did not finish");
    }

    fn flush(key: &SigningKey, o: &mut Open) {
        let Ok(Step::Ready(f)) = o.flush() else { panic!("a flush of held rows is ready") };
        for (cid, state) in &f.blocks {
            o.sent.insert(*cid, state.clone());
        }
        o.commit(sign(key, o, f.seq, f.hash)).unwrap();
    }

    /// REMOVAL: rows and tree blocks written after the account moved to a new epoch are sealed with that epoch's key,
    /// so a node that holds only the earlier ones (a removed node) reads what was written before and nothing after.
    #[test]
    fn a_node_without_the_new_epoch_reads_what_came_before_and_nothing_after() {
        let key = SigningKey::from_bytes(&[5; 32]);
        let member = key.verifying_key().to_bytes();
        let (table, e1, e2) = ([7; 32], [11; 32], [22; 32]);
        let mut o = Open::new(CODE, &member, "notes");
        o.set_table_key(table);
        o.epoch_key(1, Some(e1), true);
        put_row(&key, &mut o, "old", "before the removal");
        flush(&key, &mut o);
        o.epoch_key(2, Some(e2), true);
        put_row(&key, &mut o, "new", "after the removal");
        // The account's nodes: everything.
        assert_eq!(read_with(&o, &member, table, &[(1, e1), (2, e2)]), Ok((vec!["new=after the removal".into(), "old=before the removal".into()], 0)));
        // The removed node: the old row, and the new one counted unreadable, not shown.
        assert_eq!(read_with(&o, &member, table, &[(1, e1)]), Ok((vec!["old=before the removal".into()], 1)));
        // After a flush under epoch 2 the whole tree is sealed with it: the removed node reads none of it.
        flush(&key, &mut o);
        assert!(read_with(&o, &member, table, &[(1, e1)]).is_err(), "the new tree does not open without epoch 2");
        assert_eq!(read_with(&o, &member, table, &[(1, e1), (2, e2)]).unwrap().0, ["new=after the removal", "old=before the removal"]);
        // Control: the table's own key alone opens nothing sealed with an epoch.
        assert!(read_with(&o, &member, table, &[]).is_err());
    }

    /// Rows are sealed over UPWARD only: under the table's key, then an epoch arrives — sealed over to it; a page that
    /// writes with the table's key (no group here) never seals an epoch's rows down to it.
    #[test]
    fn rows_are_sealed_over_to_a_newer_key_never_to_an_older_one() {
        let key = SigningKey::from_bytes(&[5; 32]);
        let member = key.verifying_key().to_bytes();
        let mut o = Open::new(CODE, &member, "notes");
        o.set_table_key([7; 32]);
        put_row(&key, &mut o, "a", "1");
        put_row(&key, &mut o, "b", "2");
        o.epoch_key(3, Some([33; 32]), true);
        let Ok(Step::Ready(v)) = o.rows() else { panic!() };
        assert_eq!((keys(&v), v["legacy"].as_u64()), (vec!["a=1".into(), "b=2".into()], Some(2)));
        while let Some((seq, h)) = o.migrate(1) {
            o.commit(sign(&key, &o, seq, h)).unwrap();
            o.rows().unwrap();
        }
        let Ok(Step::Ready(v)) = o.rows() else { panic!() };
        assert_eq!((keys(&v), v["legacy"].as_u64()), (vec!["a=1".into(), "b=2".into()], Some(0)));
        // Under the table's key only deletions remain (they reach the tree at the next flush).
        assert!(o.writer.body().entries.iter().all(|(k, e)| k.first() != Some(&SEALED) || e.value.is_none()), "no value left under the table's key");
        // A page with no epoch: it reads nothing of them and seals nothing down.
        let mut low = Open::new(CODE, &member, "notes");
        low.set_table_key([7; 32]);
        low.absorb(&o.writer.state());
        low.epoch_key(3, None, false);
        let Ok(Step::Ready(v)) = low.rows() else { panic!() };
        assert_eq!((keys(&v).len(), v["legacy"].as_u64(), v["unreadable"].as_u64()), (0, Some(0), Some(2)));
        assert!(low.migrate(10).is_none());
    }

    /// A TREE FROM BEFORE sealing whole (no marker: rows in the clear or sealed one by one, blocks not sealed) is read
    /// as it is, and the next flush builds it again whole and sealed, putting every block of the new tree.
    #[test]
    fn a_tree_from_before_is_built_again_sealed() {
        let key = SigningKey::from_bytes(&[5; 32]);
        let member = key.verifying_key().to_bytes();
        let mut o = Open::new(CODE, &member, "notes");
        o.set_table_key([7; 32]);
        for (k, v) in [("a", "1"), ("b", "2"), ("c", "3")] {
            put_row(&key, &mut o, k, v);
        }
        flush(&key, &mut o);
        // Made a tree from before: the marker gone.
        let (seq, h) = o.prepare(vec![Op::Delete { key: SEALED_TREE.to_vec() }]).unwrap();
        o.commit(sign(&key, &o, seq, h)).unwrap();
        assert!(o.reseal_tree() && !o.sealed_tree());
        let Ok(Step::Ready(v)) = o.rows() else { panic!() };
        assert_eq!((keys(&v), v["resealTree"].as_bool()), (vec!["a=1".into(), "b=2".into(), "c=3".into()], Some(true)));
        o.sent.clear();
        flush(&key, &mut o);
        assert!(o.sealed_tree() && !o.reseal_tree());
        assert_eq!(read_with(&o, &member, [7; 32], &[]), Ok((vec!["a=1".into(), "b=2".into(), "c=3".into()], 0)), "every block was put again");
    }

    /// FEEDS: two nodes each write their own feed of one table (versions shaped by the `feed` package, stored and
    /// sealed here); merged, a version replaces the one it names, a delete is a version, and it all survives a flush
    /// into the (sealed) tree.
    #[test]
    fn two_nodes_feeds_merge_causally_through_the_tree() {
        use craftworks_feed::{envelope, merge};
        let (ka, kb) = (SigningKey::from_bytes(&[5; 32]), SigningKey::from_bytes(&[6; 32]));
        let (wa, wb) = (ka.verifying_key().to_bytes(), kb.verifying_key().to_bytes());
        let open = |w: &[u8; 32]| {
            let mut o = Open::new(CODE, w, "notes");
            o.set_table_key([7; 32]);
            o.epoch_key(1, Some([11; 32]), true);
            o
        };
        let (mut a, mut b) = (open(&wa), open(&wb));
        let merged = |a: &mut Open, b: &mut Open| {
            let (Ok(Step::Ready(ra)), Ok(Step::Ready(rb))) = (a.opened(), b.opened()) else { panic!("held") };
            merge(&[(a.writer_key, &ra), (b.writer_key, &rb)])
        };
        let put = |key: &SigningKey, o: &mut Open, k: &str, v: Option<&str>, after| {
            let ver = envelope((o.writer_key, o.writer.seq() + 1), after, v.map(str::as_bytes));
            let (seq, h) = o.prepare_row(k.as_bytes(), &ver).unwrap();
            o.commit(sign(key, o, seq, h)).unwrap();
        };
        put(&ka, &mut a, "n", Some("from a"), None);
        let m = merged(&mut a, &mut b);
        let seen = m[&b"n".to_vec()].id;
        assert_eq!(seen, Some((wa, 1)));
        put(&kb, &mut b, "n", Some("b edits it"), seen);
        let m = merged(&mut a, &mut b);
        assert_eq!(String::from_utf8_lossy(&m[&b"n".to_vec()].value), "b edits it");
        // A flushes its feed into its tree: the versions go with it.
        for i in 0..3 {
            put(&ka, &mut a, &format!("x{i}"), Some("filler"), None);
        }
        let Ok(Step::Ready(f)) = a.flush() else { panic!("ready") };
        a.commit(sign(&ka, &a, f.seq, f.hash)).unwrap();
        let m = merged(&mut a, &mut b);
        assert_eq!(String::from_utf8_lossy(&m[&b"n".to_vec()].value), "b edits it", "b's version still replaces a's from a's tree");
        // A deletes what it now sees (b's): gone.
        put(&ka, &mut a, "n", None, Some((wb, 1)));
        assert!(!merged(&mut a, &mut b).contains_key(&b"n".to_vec()));
        assert_eq!(merged(&mut a, &mut b).len(), 3, "control: the other rows are there");
    }

    /// A PUBLIC tail (a person's card): its rows are in the clear in what the network holds, anyone reads them with no
    /// key, a delete removes one, and it flushes into a tree in the clear that anyone reads too.
    #[test]
    fn a_public_tail_is_readable_by_anyone_and_flushes_in_the_clear() {
        let key = SigningKey::from_bytes(&[5; 32]);
        let owner = key.verifying_key().to_bytes();
        let mut o = Open::new(CODE, &owner, "card");
        o.public = true;
        put_row(&key, &mut o, "handle", "alice");
        put_row(&key, &mut o, "kp/node", "abcdef");
        assert!(contains(&o.writer.state(), b"alice"), "in the clear, by design");
        let mut r = Open::new(CODE, &owner, "card");
        r.absorb(&o.writer.state());
        let Ok(Step::Ready(v)) = r.rows() else { panic!() };
        assert_eq!(keys(&v), ["handle=alice", "kp/node=abcdef"], "no key needed to read");
        put_row(&key, &mut o, "kp/node", "");
        let Ok(Step::Ready(v)) = o.rows() else { panic!() };
        assert_eq!(keys(&v), ["handle=alice"]);
        // FLUSHED into a tree in the clear (it grows past one tail): the blocks hold the rows readable, and a reader
        // with no key walks tail -> root -> blocks to the same rows.
        for i in 0..40 {
            put_row(&key, &mut o, &format!("p{i:02}"), "post");
        }
        flush(&key, &mut o);
        assert_eq!(o.pending_rows(), 0, "the tail is emptied");
        assert!(!o.sealed_tree(), "the tree is not marked sealed");
        assert!(o.sent.values().any(|b| contains(b, b"alice")), "its blocks are in the clear");
        let mut r = Open::new(CODE, &owner, "card");
        r.public = true;
        assert!(r.absorb(&o.writer.state()));
        let mut rounds = 0;
        let v = loop {
            match r.rows().unwrap() {
                Step::Ready(v) => break v,
                Step::Keys(k) => panic!("a public tree needs no key: {k:?}"),
                Step::Need(ids) => {
                    rounds += 1;
                    assert!(rounds < 10, "the walk ends");
                    for id in ids {
                        assert_eq!(r.block_params(&id), Some((false, id)), "a Block contract named by its id");
                        assert!(r.absorb_block(&id, &o.sent[&id]));
                    }
                }
            }
        };
        assert!(rounds >= 1, "control: the reader fetched the tree");
        assert_eq!(v["rows"].as_array().unwrap().len(), 41, "the card's row and the 40 posts");
        // Control: the same rows in a sealed tail do not show.
        let mut sealed = Open::new(CODE, &owner, "card");
        sealed.set_table_key([7; 32]);
        put_row(&key, &mut sealed, "handle", "alice");
        assert!(!contains(&sealed.writer.state(), b"alice"));
    }

    #[test]
    fn a_wrong_signature_sends_nothing() {
        let key = SigningKey::from_bytes(&[5; 32]);
        let mut o = Open::new(CODE, &key.verifying_key().to_bytes(), "notes");
        let (seq, h) = o.prepare(vec![Op::Set { key: b"a".to_vec(), value: b"1".to_vec() }]).unwrap();
        let wrong = SigningKey::from_bytes(&[6; 32]);
        assert!(o.commit(sign(&wrong, &o, seq, h)).is_none());
        assert_eq!(o.writer.seq(), 0);
        let _ = S { terminal: false, seq: 0, value_hash: [0; 32], bitmap: 0, sigs: vec![] };
    }
}
