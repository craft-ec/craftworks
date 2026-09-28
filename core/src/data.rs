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
use freenet_stdlib::prelude::{ContractContainer, ContractKey};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use tail::{Op, Unsigned, Writer};

/// A table's tail is FLUSHED into its tree once it holds this many rows: the tail stays small (every write re-signs
/// and every reader re-reads the whole tail), the tree holds the rest.
pub const FLUSH_AT: usize = 32;

/// The tail's own row naming the ROOT's parity blocks (the root's group of one, `engine::repair::root_parity`): the
/// root is the one block no parent's group covers, so its parity ids ride beside the root, in the tail, where a
/// reader missing the root still finds them. Hidden from the rows; never written into the tree.
pub const ROOT_PARITY: &[u8] = b"\0root-parity";


pub struct Open {
    pub params: Vec<u8>,
    pub contract: ContractContainer,
    pub writer: Writer,
    /// Whether the network holds this tail (a GET found it, or this page put it). Before, the first write is a PUT
    /// of the whole state; after, every write is an UPDATE carrying one delta.
    pub on_network: bool,
    pub pending: Option<Unsigned>,
    /// The tree's blocks this page holds (fetched, or built by a flush), by id: `Blocks` bodies.
    pub blocks: MemBlocks,
    /// A flush's new blocks, kept until its step is signed and sent: then they are this page's too.
    staged: Option<MemBlocks>,
}

/// A flush, ready: the blocks to put FIRST (as Block contract states), then the step to sign.
pub struct Flush {
    pub seq: u64,
    pub hash: [u8; 32],
    pub blocks: Vec<(Cid, Vec<u8>)>,
}

/// What a flush or a read needs before it can go on.
pub enum Step<T> {
    Ready(T),
    /// These tree blocks, fetched and handed to `absorb_block`, then ask again.
    Need(Vec<Cid>),
}

impl Open {
    /// Table `table` of the account whose data key is `key`.
    pub fn new(tail_code: &[u8], key: &[u8; 32], table: &str) -> Open {
        let params = wire::register_params(key, &[b"t/".as_slice(), table.as_bytes()].concat());
        let (_, contract, _) = wire::puts::contract(tail_code, &params, &[]);
        let writer = Writer::new(&params).expect("our own params parse");
        Open { params, contract, writer, on_network: false, pending: None, blocks: MemBlocks::default(), staged: None }
    }

    pub fn id(&self) -> String {
        self.contract.key().id().encode()
    }

    pub fn id_bytes(&self) -> [u8; 32] {
        self.contract.key().id().as_bytes().try_into().expect("32 bytes")
    }

    pub fn key(&self) -> ContractKey {
        self.contract.key()
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

    /// The next step: what to sign (the Register's message fields).
    pub fn prepare(&mut self, ops: Vec<Op>) -> Option<(u64, [u8; 32])> {
        let u = self.writer.prepare(ops)?;
        let out = (u.seq, u.body.hash());
        self.pending = Some(u);
        Some(out)
    }

    /// A block the network sent (a Block contract's state, `kind ‖ body`), kept if it is one this table asked for:
    /// its id is computed from the bytes, never taken on trust.
    pub fn absorb_block(&mut self, want: &Cid, state: &[u8]) -> bool {
        match wire::block::block_of_state(state) {
            Some((id, body)) if id == *want => {
                self.blocks.insert(id, body);
                true
            }
            _ => false,
        }
    }

    /// Pending rows in the tail (its own root-parity row is not one).
    pub fn pending_rows(&self) -> usize {
        self.writer.body().entries.keys().filter(|k| k.as_slice() != ROOT_PARITY).count()
    }

    /// The root's parity ids, as the tail names them.
    pub fn root_parity_ids(&self) -> Vec<Cid> {
        let body = self.writer.body();
        let Some(e) = body.entries.get(ROOT_PARITY) else { return Vec::new() };
        e.value.as_deref().unwrap_or_default().chunks_exact(32).map(|c| c.try_into().expect("32")).collect()
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

    /// FLUSH: write the tail's rows into the tree (the SDK's `flush_into`, the tree library's own `apply`) and
    /// prepare the step that names the new root. The blocks go out FIRST: a tail must never name a root whose blocks
    /// are not there. `Need` when the old tree's blocks along the edited paths are not held yet.
    pub fn flush(&mut self) -> Result<Step<Flush>, String> {
        let mut body = self.writer.body();
        if self.pending_rows() == 0 {
            return Err("nothing to flush".into());
        }
        // The tail's own root-parity row stays out of the tree; the flush clears it with the rest, and the same step
        // names the new root's.
        body.entries.remove(ROOT_PARITY);
        let mut staging = self.blocks.clone();
        let (applied, op) = match tail::flush_into(&mut staging, &body, self.writer.seq()) {
            Ok(x) => x,
            Err(ApplyError::Read(ReadError::Need(ids))) => return Ok(Step::Need(ids)),
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
        for (cid, body) in staging.0.iter().filter(|(c, _)| !self.blocks.0.contains_key(*c)) {
            let state = wire::block::block_state(cid, body).ok_or_else(|| format!("block {} is of no known kind", crate::hex(cid)))?;
            blocks.push((*cid, state));
        }
        let (seq, hash) =
            self.prepare(vec![op, Op::Set { key: ROOT_PARITY.to_vec(), value: ids }]).ok_or("the tail would refuse the flush")?;
        self.staged = Some(staging);
        Ok(Step::Ready(Flush { seq, hash, blocks }))
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

    /// The rows, for the page: the TREE's (at the tail's root) with the TAIL's pending rows over them (a pending
    /// delete hides a tree row). `Need` names the tree blocks to fetch first.
    pub fn rows(&self) -> Result<Step<Value>, String> {
        let body = self.writer.body();
        let mut all: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
        if let Some(root) = &body.root {
            let mut r = Range { max_entries: 4096, max_bytes: 4 << 20, ..Range::default() };
            loop {
                let page = match range(&self.blocks, root, &r) {
                    Ok(p) => p,
                    Err(RangeError::Read(ReadError::Need(ids))) => return Ok(Step::Need(ids)),
                    Err(e) => return Err(format!("the tree does not read: {e:?}")),
                };
                if page.end == PageEnd::Blocked {
                    return Ok(Step::Need(page.need.clone()));
                }
                for (k, v) in &page.entries {
                    match read_value(&self.blocks, *v) {
                        Ok(b) => {
                            all.insert(k.clone(), b.to_vec());
                        }
                        Err(ReadError::Need(ids)) => return Ok(Step::Need(ids)),
                        Err(e) => return Err(format!("a value does not read: {e:?}")),
                    }
                }
                match (&page.end, &page.next) {
                    (PageEnd::Limit, Some(n)) => r.after = Some(n.clone()),
                    _ => break,
                }
            }
        }
        for (k, e) in body.entries.iter().filter(|(k, _)| k.as_slice() != ROOT_PARITY) {
            match &e.value {
                Some(v) => all.insert(k.clone(), v.clone()),
                None => all.remove(k),
            };
        }
        let rows: Vec<Value> =
            all.iter().map(|(k, v)| json!({ "key": String::from_utf8_lossy(k), "value": String::from_utf8_lossy(v) })).collect();
        Ok(Step::Ready(json!({ "id": self.id(), "seq": self.writer.seq(), "rows": rows, "root": body.root.map(|r| crate::hex(&r)), "pending": body.entries.len() })))
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

    fn keys(v: &Value) -> Vec<String> {
        v["rows"].as_array().unwrap().iter().map(|r| format!("{}={}", r["key"].as_str().unwrap(), r["value"].as_str().unwrap())).collect()
    }

    #[test]
    fn a_flush_moves_the_rows_into_the_tree_and_a_reader_walks_tail_then_tree() {
        let key = SigningKey::from_bytes(&[5; 32]);
        let member = key.verifying_key().to_bytes();
        let mut o = Open::new(CODE, &member, "notes");
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
        assert!(r.absorb(&o.writer.state()));
        let mut rounds = 0;
        let v = loop {
            match r.rows().unwrap() {
                Step::Ready(v) => break v,
                Step::Need(ids) => {
                    rounds += 1;
                    assert!(rounds < 10, "the walk ends");
                    for id in ids {
                        let state = wire::block::block_state(&id, o.blocks.0.get(&id).expect("the writer made it")).unwrap();
                        assert!(r.absorb_block(&id, &state));
                    }
                }
            }
        };
        assert!(rounds >= 1, "control: the reader had to fetch the tree");
        assert_eq!(keys(&v), ["a=1", "c=3", "d=4"]);
        // A block that is not the one asked for is refused.
        let (id, body) = o.blocks.0.iter().next().unwrap();
        let other = [0u8; 32];
        assert!(!r.absorb_block(&other, &wire::block::block_state(id, body).unwrap()));
    }

    /// ERASURE: once a table's tree is past one leaf, its root lists parity for its children, and every one of
    /// those parity blocks is among what the flush PUT (none is owed and left unsent). A single-leaf tree has no
    /// children, so no parity: the control.
    #[test]
    fn a_tree_past_one_leaf_puts_parity_for_its_blocks() {
        let key = SigningKey::from_bytes(&[5; 32]);
        let mut o = Open::new(CODE, &key.verifying_key().to_bytes(), "notes");
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
            assert!(put.contains(&p), "parity block {} was put by a flush", crate::hex(&p));
        }
    }

    /// A reader of `o`'s state that is DENIED `lost` (the network no longer has it) and must repair it from its
    /// group. Returns the rows it ends with.
    fn read_without(o: &Open, member: &[u8; 32], lost: Cid) -> (Vec<String>, bool) {
        let mut r = Open::new(CODE, member, "notes");
        assert!(r.absorb(&o.writer.state()));
        let mut repaired = false;
        for _ in 0..20 {
            match r.rows().unwrap() {
                Step::Ready(v) => return (keys(&v), repaired),
                Step::Need(ids) => {
                    for id in ids {
                        if id == lost {
                            let g = r.repair_group(&id).expect("a group names the lost block");
                            for (i, c) in g.slots.iter().enumerate() {
                                if *c != lost && i < g.slots.len() {
                                    if let Some(b) = o.blocks.0.get(c) {
                                        r.absorb_block(c, &wire::block::block_state(c, b).unwrap());
                                    }
                                }
                            }
                            r.rebuild(&g).expect("rebuilt and verified");
                            repaired = true;
                        } else {
                            r.absorb_block(&id, &wire::block::block_state(&id, o.blocks.0.get(&id).unwrap()).unwrap());
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
