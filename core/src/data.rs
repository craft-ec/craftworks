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

    /// Pending rows in the tail.
    pub fn pending_rows(&self) -> usize {
        self.writer.body().entries.len()
    }

    /// FLUSH: write the tail's rows into the tree (the SDK's `flush_into`, the tree library's own `apply`) and
    /// prepare the step that names the new root. The blocks go out FIRST: a tail must never name a root whose blocks
    /// are not there. `Need` when the old tree's blocks along the edited paths are not held yet.
    pub fn flush(&mut self) -> Result<Step<Flush>, String> {
        let body = self.writer.body();
        if body.entries.is_empty() {
            return Err("nothing to flush".into());
        }
        let mut staging = self.blocks.clone();
        let (applied, op) = match tail::flush_into(&mut staging, &body, self.writer.seq()) {
            Ok(x) => x,
            Err(ApplyError::Read(ReadError::Need(ids))) => return Ok(Step::Need(ids)),
            Err(e) => return Err(format!("the tree would not take the rows: {e:?}")),
        };
        for (cid, bytes) in &applied.parity {
            staging.insert(*cid, bytes);
        }
        let mut blocks = Vec::new();
        for (cid, body) in staging.0.iter().filter(|(c, _)| !self.blocks.0.contains_key(*c)) {
            let state = wire::block::block_state(cid, body).ok_or_else(|| format!("block {} is of no known kind", crate::hex(cid)))?;
            blocks.push((*cid, state));
        }
        let (seq, hash) = self.prepare(vec![op]).ok_or("the tail would refuse the flush")?;
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
        for (k, e) in &body.entries {
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
