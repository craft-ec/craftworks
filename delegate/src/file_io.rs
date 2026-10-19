//! A FILE READ and a FILE WRITTEN by the node with no page open — the one reader and the one writer every node round
//! uses (a re-key: `recode`, a read piped into a write; a space's large key operations: `keys_round`). Steps of the
//! `files` crate's rules, the page's too: the index walked by `files::Root`, each part read in rounds by
//! `files::Decoder`, stored ratelessly by `files::store_next`, its index put after every part and its root last.
//!
//! What a step holds between calls is in the node's secrets under its SLOT (`identity_upkeep/<slot>…`): a part's
//! fragments while it is read (up to 16 × 256 KiB), a part's bytes while it is stored.

use craftworks_files as files;
use craftworks_identity::Host;
use serde::{Deserialize, Serialize};

use crate::upkeep::{Code, Io};

/// A piece's contract state: `LIVE ‖ burn hash ‖ piece` (the `piece` contract).
pub fn live(burn: &[u8; 32], p: &files::Piece) -> Vec<u8> {
    [&[2u8][..], burn, &p.state].concat()
}
/// A piece read back (burned or missing: none).
fn piece_of(state: Option<Vec<u8>>) -> Option<Vec<u8>> {
    state.filter(|st| st.first() == Some(&2) && st.len() > 33).map(|st| st[33..].to_vec())
}
fn secret_key(slot: &[u8], what: &[u8]) -> Vec<u8> {
    [b"identity_upkeep/".as_slice(), slot, what].concat()
}
fn get_io(piece_hash: &[u8; 32], address: &[u8; 32]) -> Io {
    Io::Get { id: crate::upkeep::id_of(piece_hash, address), subscribe: false }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum ReadPhase {
    Root,
    /// Inner index piece `(level, n)` asked, walking down to the leaves.
    Inner(u8, u64),
    /// Leaf `n` asked.
    Leaf(u64),
    /// Part g's fragments asked (`round` of `files::ASK_ROUNDS`): still out, held (each in a secret), back empty.
    Fragments { g: u64, out: Vec<(u8, [u8; 32])>, held: Vec<u8>, empty: Vec<u8>, round: usize },
    /// Part g handed out: the next asked when the caller says (`next`).
    Handed(u64),
    Done,
}

/// What a read step gave.
#[derive(Debug)]
pub enum Read {
    Io(Vec<Io>),
    /// Part `g` decoded: its bytes.
    Part(u64, Vec<u8>),
}

/// A FILE READ, part by part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRead {
    slot: Vec<u8>,
    key: [u8; 32],
    root_hash: [u8; 32],
    size: u64,
    depth: u8,
    /// Every index piece's hash known so far, `(level, n)` (the leaves at level 0), from the root down.
    index: std::collections::BTreeMap<(u8, u64), [u8; 32]>,
    /// Each part's listed fragments, as its leaf says (filled leaf by leaf).
    listed: Vec<files::Listed>,
    phase: ReadPhase,
    /// Every contract a GET of it still waits on.
    pending: Vec<[u8; 32]>,
}

impl FileRead {
    /// The read of the file keyed `key` whose root is `root_hash`: its root asked.
    pub fn start(slot: &[u8], piece_hash: &[u8; 32], key: [u8; 32], root_hash: [u8; 32]) -> (FileRead, Vec<Io>) {
        let io = vec![get_io(piece_hash, &files::hashed_address(&key, &root_hash))];
        let mut r = FileRead { slot: slot.to_vec(), key, root_hash, size: 0, depth: 0, index: Default::default(), listed: Vec::new(), phase: ReadPhase::Root, pending: Vec::new() };
        r.track(&io);
        (r, io)
    }
    fn track(&mut self, io: &[Io]) {
        for x in io {
            if let Io::Get { id, .. } = x {
                self.pending.push(*id);
            }
        }
    }
    pub fn wants(&self, id: &[u8; 32]) -> bool {
        self.pending.contains(id)
    }
    pub fn size(&self) -> u64 {
        self.size
    }
    pub fn done(&self) -> bool {
        self.phase == ReadPhase::Done
    }
    fn plan(&self) -> files::Plan {
        files::Plan::of(self.size)
    }
    fn frag(&self, j: u8) -> Vec<u8> {
        secret_key(&self.slot, &[b"frag/".as_slice(), &[j]].concat())
    }

    /// Its PIECES (what a burn names): its root, every index piece, every listed fragment.
    pub fn pieces(&self) -> Vec<[u8; 32]> {
        let mut out = vec![files::hashed_address(&self.key, &self.root_hash)];
        out.extend(self.index.values().map(|h| files::hashed_address(&self.key, h)));
        for (g, l) in self.listed.iter().enumerate() {
            out.extend(l.iter().map(|(j, _)| files::fragment_address(&self.key, g as u64, *j)));
        }
        out
    }

    /// An ANSWER to a GET.
    pub fn got<H: Host>(&mut self, h: &mut H, piece_hash: &[u8; 32], id: [u8; 32], state: Option<Vec<u8>>) -> Result<Read, String> {
        if let Some(i) = self.pending.iter().position(|p| *p == id) {
            self.pending.remove(i);
        }
        let out = self.got_(h, piece_hash, id, piece_of(state))?;
        if let Read::Io(io) = &out {
            let io = io.clone();
            self.track(&io);
        }
        Ok(out)
    }
    fn got_<H: Host>(&mut self, h: &mut H, piece_hash: &[u8; 32], id: [u8; 32], piece: Option<Vec<u8>>) -> Result<Read, String> {
        match self.phase.clone() {
            ReadPhase::Root => {
                let st = piece.ok_or("its root is not on the network")?;
                let r = files::Root::open(&self.key, &self.root_hash, &st).map_err(|e| format!("its root: {e:?}"))?;
                self.size = r.plan.size;
                self.depth = r.depth;
                self.index = r.children.iter().enumerate().map(|(n, c)| ((r.depth, n as u64), *c)).collect();
                self.walk(h, piece_hash).map(Read::Io)
            }
            ReadPhase::Inner(level, n) => {
                let st = piece.ok_or("part of its index is missing")?;
                let hash = self.index[&(level, n)];
                let children = files::open_inner(&self.key, &hash, &st).map_err(|e| format!("its index: {e:?}"))?;
                // (Each child's place: `files::Root::parent`, the one walk.)
                let first = n * files::FANOUT as u64;
                for (i, c) in children.iter().enumerate() {
                    self.index.insert((level - 1, first + i as u64), *c);
                }
                self.walk(h, piece_hash).map(Read::Io)
            }
            ReadPhase::Leaf(n) => {
                let st = piece.ok_or("part of its index is missing")?;
                let hash = self.index[&(0, n)];
                let gens = files::open_leaf(&self.key, &self.plan(), n, &hash, &st).map_err(|e| format!("its index: {e:?}"))?;
                self.listed.extend(gens);
                let g = n * files::GENS_PER_LEAF as u64;
                self.ask(h, piece_hash, g).map(Read::Io)
            }
            ReadPhase::Fragments { g, mut out, mut held, mut empty, round } => {
                let Some(at) = out.iter().position(|(_, w)| *w == id) else { return Ok(Read::Io(Vec::new())) };
                let (j, _) = out.remove(at);
                match piece {
                    Some(st) => {
                        h.set_secret(&self.frag(j), &st);
                        held.push(j);
                        empty.retain(|x| *x != j);
                    }
                    None if !empty.contains(&j) => empty.push(j),
                    None => {}
                }
                // DECODED on the first k valid, independent ones; or, every one back, the missing asked again (the
                // rounds: `files::ASK_ROUNDS`, the page's too) — then lost.
                if held.len() >= self.plan().k(g) || out.is_empty() {
                    let d = self.decoder(h, g, &held, &empty);
                    if d.done() {
                        for (j, _) in &self.listed[g as usize] {
                            h.set_secret(&self.frag(*j), &[]);
                        }
                        let plain = d.plain().map_err(|e| format!("part {}: {e:?}", g + 1))?;
                        self.phase = ReadPhase::Handed(g);
                        return Ok(Read::Part(g, plain));
                    }
                    if out.is_empty() {
                        if round + 1 < files::ASK_ROUNDS {
                            let again = d.missing();
                            return Ok(Read::Io(self.ask_fragments(piece_hash, g, &again, held, empty, round + 1)));
                        }
                        return Err(format!("part {} could not be rebuilt ({} of {} pieces found after {} rounds)", g + 1, d.rank(), d.k(), files::ASK_ROUNDS));
                    }
                }
                self.phase = ReadPhase::Fragments { g, out, held, empty, round };
                Ok(Read::Io(Vec::new()))
            }
            ReadPhase::Handed(_) | ReadPhase::Done => Ok(Read::Io(Vec::new())),
        }
    }

    /// Part `g` taken by the caller: the next asked (none, and `done`, when every part was read).
    pub fn next<H: Host>(&mut self, h: &mut H, piece_hash: &[u8; 32]) -> Result<Vec<Io>, String> {
        let ReadPhase::Handed(g) = self.phase else { return Ok(Vec::new()) };
        if g + 1 >= self.plan().gens {
            self.phase = ReadPhase::Done;
            return Ok(Vec::new());
        }
        let io = self.ask(h, piece_hash, g + 1)?;
        self.track(&io);
        Ok(io)
    }

    /// DOWN THE INDEX: the next inner piece not read yet (top first), else the first part.
    fn walk<H: Host>(&mut self, h: &mut H, piece_hash: &[u8; 32]) -> Result<Vec<Io>, String> {
        let root = files::Root { plan: self.plan(), depth: self.depth, children: Vec::new() };
        let mut inner: Vec<(u8, u64)> = root.index_pieces().into_iter().filter(|(l, _)| *l > 0).collect();
        inner.reverse();
        if let Some(&(level, n)) = inner.iter().find(|p| !self.index.contains_key(&(p.0 - 1, p.1 * files::FANOUT as u64))) {
            let hash = *self.index.get(&(level, n)).ok_or("an inner piece its parent does not list")?;
            self.phase = ReadPhase::Inner(level, n);
            return Ok(vec![get_io(piece_hash, &files::hashed_address(&self.key, &hash))]);
        }
        self.ask(h, piece_hash, 0)
    }

    /// Part g's fragments asked (its leaf read first when not yet).
    fn ask<H: Host>(&mut self, h: &mut H, piece_hash: &[u8; 32], g: u64) -> Result<Vec<Io>, String> {
        if (g as usize) >= self.listed.len() {
            let n = files::Root::leaf_of(g);
            let leaf = *self.index.get(&(0, n)).ok_or("a leaf its index does not list")?;
            self.phase = ReadPhase::Leaf(n);
            return Ok(vec![get_io(piece_hash, &files::hashed_address(&self.key, &leaf))]);
        }
        for (j, _) in &self.listed[g as usize] {
            h.set_secret(&self.frag(*j), &[]);
        }
        let all: Vec<u8> = self.listed[g as usize].iter().map(|(j, _)| *j).collect();
        Ok(self.ask_fragments(piece_hash, g, &all, Vec::new(), Vec::new(), 0))
    }
    fn ask_fragments(&mut self, piece_hash: &[u8; 32], g: u64, ask: &[u8], held: Vec<u8>, empty: Vec<u8>, round: usize) -> Vec<Io> {
        let out: Vec<(u8, [u8; 32])> = ask.iter().map(|j| (*j, crate::upkeep::id_of(piece_hash, &files::fragment_address(&self.key, g, *j)))).collect();
        let io = out.iter().map(|(_, id)| Io::Get { id: *id, subscribe: false }).collect();
        self.phase = ReadPhase::Fragments { g, out, held, empty, round };
        io
    }
    /// The part's DECODER over what it holds (`files::Decoder`, the page's too).
    fn decoder<H: Host>(&self, h: &H, g: u64, held: &[u8], empty: &[u8]) -> files::Decoder {
        let mut d = files::Decoder::new(&self.key, &self.plan(), g, self.listed[g as usize].clone());
        for j in held {
            if d.done() {
                break;
            }
            let _ = d.add(*j, &h.get_secret(&self.frag(*j)).unwrap_or_default());
        }
        for j in empty {
            d.none(*j);
        }
        d
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum WritePhase {
    /// Waiting for a part.
    Ready,
    /// Part g's pieces put: `(contract id, j, hash)` unanswered, the stored ones, the next `j` not tried.
    Putting { g: u64, out: Vec<([u8; 32], u8, [u8; 32])>, ok: Vec<(u8, [u8; 32])>, next: usize },
    /// Its index put: ids unanswered; its ROOT `(address, state)` held back until the rest is stored.
    Index { out: Vec<[u8; 32]>, refused: bool, root: Option<([u8; 32], Vec<u8>)> },
    Done,
}

/// What a write step gave.
#[derive(Debug)]
pub enum Wrote {
    Io(Vec<Io>),
    /// Part `g` stored: the next one wanted (or, every part in, `index`).
    Part(u64),
    /// Stored whole: its root's hash (the reference's).
    Done([u8; 32]),
}

/// A FILE WRITTEN, part by part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileWrite {
    slot: Vec<u8>,
    key: [u8; 32],
    burn: [u8; 32],
    size: u64,
    stored: Vec<files::Listed>,
    root: Option<[u8; 32]>,
    phase: WritePhase,
    pending: Vec<[u8; 32]>,
}

impl FileWrite {
    /// A file of `size` bytes stored under `key`, its pieces named by `burn` (a public file's: zero).
    pub fn new(slot: &[u8], key: [u8; 32], burn: [u8; 32], size: u64) -> FileWrite {
        FileWrite { slot: slot.to_vec(), key, burn, size, stored: Vec::new(), root: None, phase: WritePhase::Ready, pending: Vec::new() }
    }
    fn plan(&self) -> files::Plan {
        files::Plan::of(self.size)
    }
    fn plain_key(&self) -> Vec<u8> {
        secret_key(&self.slot, b"plain")
    }
    fn put_io(&mut self, piece_hash: &[u8; 32], p: &files::Piece) -> ([u8; 32], Io) {
        let id = crate::upkeep::id_of(piece_hash, &p.address);
        self.pending.push(id);
        (id, Io::Put { code: Code::Piece, params: p.address.to_vec(), state: live(&self.burn, p) })
    }
    pub fn wants(&self, id: &[u8; 32]) -> bool {
        self.pending.contains(id)
    }
    /// Its pieces stored per part (what an index lists).
    pub fn stored(&self) -> &[files::Listed] {
        &self.stored
    }
    pub fn key(&self) -> [u8; 32] {
        self.key
    }
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Part `g` (its bytes): coded and PUT, the bytes kept until it is stored (a fragment not stored is replaced by
    /// a new one minted from them).
    pub fn part<H: Host>(&mut self, h: &mut H, piece_hash: &[u8; 32], g: u64, plain: &[u8]) -> Vec<Io> {
        let pieces = files::encode(&self.key, &self.plan(), g, plain, files::EXTRA);
        h.set_secret(&self.plain_key(), plain);
        let mut out = Vec::new();
        let mut io = Vec::new();
        for (j, p) in &pieces {
            let (id, x) = self.put_io(piece_hash, p);
            out.push((id, *j, p.hash()));
            io.push(x);
        }
        let next = pieces.iter().map(|(j, _)| *j as usize + 1).max().unwrap_or(0);
        self.phase = WritePhase::Putting { g, out, ok: Vec::new(), next };
        io
    }

    /// Every part stored: the INDEX, the root after the rest.
    pub fn index(&mut self, piece_hash: &[u8; 32]) -> Result<Vec<Io>, String> {
        let (mut pieces, root) = files::index_hashed(&self.key, &self.plan(), &self.stored);
        self.root = Some(root);
        // (`files::index_hashed`: the root is last.)
        let last = pieces.pop().ok_or("an index with no root")?;
        let mut out = Vec::new();
        let mut io = Vec::new();
        for p in &pieces {
            let (id, x) = self.put_io(piece_hash, p);
            out.push(id);
            io.push(x);
        }
        let held = (last.address, live(&self.burn, &last));
        if io.is_empty() {
            let (id, x) = self.put_io(piece_hash, &last);
            self.phase = WritePhase::Index { out: vec![id], refused: false, root: None };
            return Ok(vec![x]);
        }
        self.phase = WritePhase::Index { out, refused: false, root: Some(held) };
        Ok(io)
    }

    /// An ANSWER to a PUT.
    pub fn put<H: Host>(&mut self, h: &mut H, piece_hash: &[u8; 32], id: [u8; 32], ok: bool) -> Result<Wrote, String> {
        if let Some(i) = self.pending.iter().position(|p| *p == id) {
            self.pending.remove(i);
        }
        match self.phase.clone() {
            WritePhase::Putting { g, mut out, ok: mut stored, mut next } => {
                let Some(at) = out.iter().position(|(w, _, _)| *w == id) else { return Ok(Wrote::Io(Vec::new())) };
                let (_, j, hash) = out.remove(at);
                if ok {
                    stored.push((j, hash));
                }
                // RATELESS (`files::store_next`, the page's too): one not stored replaced by a new one.
                let plan = self.plan();
                match files::store_next(&plan, g, stored.len(), next, out.len()) {
                    files::StoreNext::Wait => {
                        self.phase = WritePhase::Putting { g, out, ok: stored, next };
                        Ok(Wrote::Io(Vec::new()))
                    }
                    files::StoreNext::Mint(j) => {
                        let plain = h.get_secret(&self.plain_key()).ok_or("the part being stored is not held")?;
                        let p = files::mint(&self.key, &plan, g, &plain, j);
                        let (id, x) = self.put_io(piece_hash, &p);
                        out.push((id, j, p.hash()));
                        next = j as usize + 1;
                        self.phase = WritePhase::Putting { g, out, ok: stored, next };
                        Ok(Wrote::Io(vec![x]))
                    }
                    files::StoreNext::Failed => Err(format!("part {} could not be stored ({} of the {} it needs)", g + 1, stored.len(), plan.k(g))),
                    files::StoreNext::Done => {
                        h.set_secret(&self.plain_key(), &[]);
                        stored.sort();
                        self.stored.push(stored);
                        self.phase = WritePhase::Ready;
                        Ok(Wrote::Part(g))
                    }
                }
            }
            WritePhase::Index { mut out, refused, root } => {
                let Some(at) = out.iter().position(|w| *w == id) else { return Ok(Wrote::Io(Vec::new())) };
                out.remove(at);
                let refused = refused || !ok;
                if !out.is_empty() {
                    self.phase = WritePhase::Index { out, refused, root };
                    return Ok(Wrote::Io(Vec::new()));
                }
                if refused {
                    return Err("its index could not be stored".into());
                }
                // The rest stored: its ROOT now.
                if let Some((address, state)) = root {
                    let id = crate::upkeep::id_of(piece_hash, &address);
                    self.pending.push(id);
                    self.phase = WritePhase::Index { out: vec![id], refused: false, root: None };
                    return Ok(Wrote::Io(vec![Io::Put { code: Code::Piece, params: address.to_vec(), state }]));
                }
                self.phase = WritePhase::Done;
                Ok(Wrote::Done(self.root.expect("its index made")))
            }
            WritePhase::Ready | WritePhase::Done => Ok(Wrote::Io(Vec::new())),
        }
    }
}
