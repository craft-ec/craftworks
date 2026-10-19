//! A FILE RE-KEYED with no page open (R4d) — `files.recode` as a step machine: the file read under the key its row has
//! (its root, its leaves, each generation's listed fragments), coded under the NEW key (from its content hash and the
//! space's salt now; public: the content alone), put in `piece` contracts named by the new burn hash, its index last.
//! Then (the round's part) its row changed and its OLD pieces BURNED with the secret its row kept.
//!
//! One generation at a time: its fragments GOT (each kept in a secret of its own until enough are in — a generation
//! is up to 16 × 256 KiB), decoded, coded again, PUT; their answers counted. A file whose row has no content hash (an
//! adopted one from before hashes travelled), or whose index is deeper than one level, is left to a page.

use craftworks_identity::{hex, unhex32};
use serde::{Deserialize, Serialize};
use sha2::Digest;

use craftworks_files as files;
use craftworks_identity::Host;

use crate::upkeep::{Code, Io};

/// A fragment of the generation being read, kept here until decoded: `REKEY_FRAG ‖ j`.
const FRAG: &[u8] = b"identity_upkeep/rekey/frag/";

fn sha(b: &[u8]) -> [u8; 32] {
    sha2::Sha256::digest(b).into()
}
/// The BURN SECRET a space's salt gives a file key (`files.secretOf`).
pub fn burn_secret(salt: &[u8; 32], key: &[u8; 32]) -> [u8; 32] {
    sha(&[b"craftworks burn".as_slice(), salt, key].concat())
}
/// A piece's contract state: `LIVE ‖ burn hash ‖ piece` (the `piece` contract).
fn live(burn: &[u8; 32], p: &files::Piece) -> Vec<u8> {
    [&[2u8][..], burn, &p.state].concat()
}
/// A piece's BURN: `BURNED ‖ sha-256(secret) ‖ secret`.
pub fn burned(secret: &[u8; 32]) -> Vec<u8> {
    [&[3u8][..], &sha(secret), secret].concat()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    /// Its root asked.
    Root,
    /// Inner index piece `(level, n)` asked (an index deeper than one level), walking down to its leaves.
    Inner(u8, u64),
    /// Leaf `n` asked.
    Leaf(u64),
    /// Generation g's fragments asked (`round` of `files::ASK_ROUNDS`): those still out, those held (each in a secret of
    /// its own), those that came back empty.
    Fragments { g: u64, out: Vec<(u8, [u8; 32])>, held: Vec<u8>, empty: Vec<u8>, round: usize },
    /// Generation g's new pieces put: `(contract id, j, hash)` still unanswered, and the stored ones.
    Putting { g: u64, out: Vec<([u8; 32], u8, [u8; 32])>, ok: Vec<(u8, [u8; 32])> },
    /// Its index put: ids unanswered.
    Index { out: Vec<[u8; 32]>, refused: bool },
    Done,
}

/// A RE-KEY in progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recode {
    pub id: String,
    /// Its row as read (JSON object).
    pub row: String,
    old_key: [u8; 32],
    old_root: [u8; 32],
    pub new_key: [u8; 32],
    pub burn: [u8; 32],
    pub secret: Option<[u8; 32]>,
    pub n: i64,
    pub h: [u8; 32],
    size: u64,
    depth: u8,
    /// Every index piece's hash known so far, `(level, n)` (the leaves at level 0): from the root down (`files::Root`'s
    /// walk, the page's too).
    index: std::collections::BTreeMap<(u8, u64), [u8; 32]>,
    /// Each generation's listed fragments, as its leaf says (filled leaf by leaf).
    listed: Vec<Vec<(u8, [u8; 32])>>,
    /// The new pieces stored per generation.
    pub stored: Vec<Vec<(u8, [u8; 32])>>,
    pub root: Option<[u8; 32]>,
    phase: Phase,
    /// Every contract a GET or PUT of it still waits on (its answers are its, not another round's).
    pending: Vec<[u8; 32]>,
}

/// What the re-key ended in.
#[derive(Debug, Clone, PartialEq)]
pub enum End {
    /// Re-keyed: the row's new value (JSON), and the OLD pieces' addresses (root, leaves, fragments) to burn.
    Done { row: String, old: Vec<[u8; 32]> },
    /// The same key and burn already: only the row's fields brought up to date.
    Same { row: String },
    /// Not re-keyed here (why: for the round's log; a page does it).
    Failed(String),
}

pub struct Target {
    /// Public (the content alone), or the space's salt now `(s, n)`.
    pub salt: Option<([u8; 32], i64)>,
}

impl Recode {
    /// The re-key of `row` (its `k/<id>` value, JSON) to `to`. The first GET: its root. `Err`: left to a page.
    pub fn start(piece_hash: &[u8; 32], id: &str, row: &str, to: &Target) -> Result<(Recode, Vec<Io>), String> {
        let v: serde_json::Value = serde_json::from_str(row).map_err(|_| "a row that is not JSON")?;
        let old_key = v["key"].as_str().and_then(unhex32).ok_or("a row with no key")?;
        let old_root = v["root"].as_str().and_then(unhex32).ok_or("a row with no root")?;
        let h = v["h"].as_str().and_then(unhex32).ok_or("no content hash in its row (a page hashes it first)")?;
        let (new_key, secret, n) = match to.salt {
            None => (files::content_key(&h, None), None, -1),
            Some((s, n)) => {
                let k = files::content_key(&h, Some(&s));
                (k, Some(burn_secret(&s, &k)), n)
            }
        };
        let burn = secret.map(|s| sha(&s)).unwrap_or([0; 32]);
        let r = Recode {
            id: id.to_string(),
            row: row.to_string(),
            old_key,
            old_root,
            new_key,
            burn,
            secret,
            n,
            h,
            size: 0,
            depth: 0,
            index: Default::default(),
            listed: Vec::new(),
            stored: Vec::new(),
            root: None,
            phase: Phase::Root,
            pending: Vec::new(),
        };
        let id = crate::upkeep::id_of(piece_hash, &files::hashed_address(&old_key, &old_root));
        let mut r = r;
        let io = vec![Io::Get { id, subscribe: false }];
        r.track(piece_hash, &io);
        Ok((r, io))
    }

    fn track(&mut self, piece_hash: &[u8; 32], io: &[Io]) {
        for x in io {
            match x {
                Io::Get { id, .. } => self.pending.push(*id),
                Io::Put { params, .. } => {
                    if let Ok(a) = <[u8; 32]>::try_from(params.as_slice()) {
                        self.pending.push(crate::upkeep::id_of(piece_hash, &a));
                    }
                }
                Io::Update { id, .. } => self.pending.push(*id),
            }
        }
    }
    /// Whether it waits on contract `id`.
    pub fn wants(&self, id: &[u8; 32]) -> bool {
        self.pending.contains(id)
    }
    fn answered(&mut self, id: &[u8; 32]) {
        if let Some(i) = self.pending.iter().position(|p| p == id) {
            self.pending.remove(i);
        }
    }

    /// The same key and burn hash it has: nothing to move (its row's fields brought up to date only).
    pub fn same(&self) -> bool {
        let v: serde_json::Value = serde_json::from_str(&self.row).unwrap_or_default();
        v["key"].as_str() == Some(&hex(&self.new_key)) && v["b"].as_str() == Some(&hex(&self.burn))
    }

    fn plan(&self) -> files::Plan {
        files::Plan::of(self.size)
    }

    /// The new row: its key, root, burn hash and secret, hash and salt number (`at` set by its writer).
    fn new_row(&self) -> String {
        let mut v: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&self.row).unwrap_or_default();
        v.remove("x");
        v.insert("key".into(), hex(&self.new_key).into());
        if let Some(root) = self.root {
            v.insert("root".into(), hex(&root).into());
        }
        v.insert("b".into(), hex(&self.burn).into());
        if let Some(s) = self.secret {
            v.insert("x".into(), hex(&s).into());
        }
        v.insert("h".into(), hex(&self.h).into());
        v.insert("n".into(), self.n.into());
        serde_json::Value::Object(v).to_string()
    }

    /// The OLD pieces, to burn: its root, every index piece, every listed fragment.
    fn old_pieces(&self) -> Vec<[u8; 32]> {
        let mut out = vec![files::hashed_address(&self.old_key, &self.old_root)];
        out.extend(self.index.values().map(|h| files::hashed_address(&self.old_key, h)));
        for (g, l) in self.listed.iter().enumerate() {
            out.extend(l.iter().map(|(j, _)| files::fragment_address(&self.old_key, g as u64, *j)));
        }
        out
    }

    /// DOWN THE INDEX: the next inner piece not read yet (top first), else the first generation.
    fn walk<H: Host>(&mut self, h: &mut H, piece_hash: &[u8; 32]) -> Result<Vec<Io>, String> {
        let root = files::Root { plan: self.plan(), depth: self.depth, children: Vec::new() };
        let mut inner: Vec<(u8, u64)> = root.index_pieces().into_iter().filter(|(l, _)| *l > 0).collect();
        inner.reverse();
        if let Some(&(level, n)) = inner.iter().find(|p| !self.index.contains_key(&(p.0 - 1, p.1 * files::FANOUT as u64))) {
            let hash = *self.index.get(&(level, n)).ok_or("an inner piece its parent does not list")?;
            self.phase = Phase::Inner(level, n);
            return Ok(vec![Io::Get { id: crate::upkeep::id_of(piece_hash, &files::hashed_address(&self.old_key, &hash)), subscribe: false }]);
        }
        self.ask_gen(h, piece_hash, 0)
    }

    /// Generation g's fragments asked (its leaf read first when not yet).
    fn ask_gen<H: Host>(&mut self, h: &mut H, piece_hash: &[u8; 32], g: u64) -> Result<Vec<Io>, String> {
        let plan = self.plan();
        if g >= plan.gens {
            // Every generation stored: the INDEX, last.
            let (pieces, root) = files::index_hashed(&self.new_key, &plan, &self.stored);
            self.root = Some(root);
            let mut out = Vec::new();
            let mut io = Vec::new();
            for p in pieces {
                let id = crate::upkeep::id_of(piece_hash, &p.address);
                out.push(id);
                io.push(Io::Put { code: Code::Piece, params: p.address.to_vec(), state: live(&self.burn, &p) });
            }
            self.phase = Phase::Index { out, refused: false };
            return Ok(io);
        }
        if (g as usize) >= self.listed.len() {
            let n = files::Root::leaf_of(g);
            let leaf = *self.index.get(&(0, n)).ok_or("a leaf its index does not list")?;
            self.phase = Phase::Leaf(n);
            return Ok(vec![Io::Get { id: crate::upkeep::id_of(piece_hash, &files::hashed_address(&self.old_key, &leaf)), subscribe: false }]);
        }
        for (j, _) in &self.listed[g as usize] {
            h.set_secret(&[FRAG, &[*j]].concat(), &[]);
        }
        let all: Vec<u8> = self.listed[g as usize].iter().map(|(j, _)| *j).collect();
        Ok(self.ask_fragments(piece_hash, g, &all, Vec::new(), Vec::new(), 0))
    }
    /// Round `round` of generation g's read: `ask` asked.
    fn ask_fragments(&mut self, piece_hash: &[u8; 32], g: u64, ask: &[u8], held: Vec<u8>, empty: Vec<u8>, round: usize) -> Vec<Io> {
        let out: Vec<(u8, [u8; 32])> = ask.iter().map(|j| (*j, crate::upkeep::id_of(piece_hash, &files::fragment_address(&self.old_key, g, *j)))).collect();
        let io = out.iter().map(|(_, id)| Io::Get { id: *id, subscribe: false }).collect();
        self.phase = Phase::Fragments { g, out, held, empty, round };
        io
    }
    /// The generation's DECODER over what it holds (`files::Decoder`, the page's too).
    fn decoder<H: Host>(&self, h: &H, g: u64, held: &[u8], empty: &[u8]) -> files::Decoder {
        let mut d = files::Decoder::new(&self.old_key, &self.plan(), g, self.listed[g as usize].clone());
        for j in held {
            if d.done() {
                break;
            }
            let _ = d.add(*j, &h.get_secret(&[FRAG, &[*j]].concat()).unwrap_or_default());
        }
        for j in empty {
            d.none(*j);
        }
        d
    }

    /// Generation g decoded, coded under the new key, PUT.
    fn code_gen<H: Host>(&mut self, h: &mut H, piece_hash: &[u8; 32], g: u64, d: files::Decoder) -> Result<Vec<Io>, String> {
        let plan = self.plan();
        for (j, _) in &self.listed[g as usize] {
            h.set_secret(&[FRAG, &[*j]].concat(), &[]);
        }
        let plain = d.plain().map_err(|e| format!("part {}: {e:?}", g + 1))?;
        let pieces = files::encode(&self.new_key, &plan, g, &plain, files::EXTRA);
        let mut out = Vec::new();
        let mut io = Vec::new();
        for (j, p) in pieces {
            let id = crate::upkeep::id_of(piece_hash, &p.address);
            out.push((id, j, p.hash()));
            io.push(Io::Put { code: Code::Piece, params: p.address.to_vec(), state: live(&self.burn, &p) });
        }
        self.phase = Phase::Putting { g, out, ok: Vec::new() };
        Ok(io)
    }

    /// An ANSWER to a GET. What to send next (or the end).
    pub fn got<H: Host>(&mut self, h: &mut H, piece_hash: &[u8; 32], id: [u8; 32], state: Option<Vec<u8>>) -> Result<Vec<Io>, String> {
        self.answered(&id);
        let io = self.got_(h, piece_hash, id, state)?;
        self.track(piece_hash, &io);
        Ok(io)
    }
    fn got_<H: Host>(&mut self, h: &mut H, piece_hash: &[u8; 32], id: [u8; 32], state: Option<Vec<u8>>) -> Result<Vec<Io>, String> {
        // A piece's state: `LIVE ‖ burn hash ‖ piece` (burned or missing: none).
        let piece = state.filter(|st| st.first() == Some(&2) && st.len() > 33).map(|st| st[33..].to_vec());
        match self.phase.clone() {
            Phase::Root => {
                let st = piece.ok_or("its root is not on the network")?;
                let r = files::Root::open(&self.old_key, &self.old_root, &st).map_err(|e| format!("its root: {e:?}"))?;
                self.size = r.plan.size;
                self.depth = r.depth;
                self.index = r.children.iter().enumerate().map(|(n, c)| ((r.depth, n as u64), *c)).collect();
                self.stored = Vec::new();
                self.walk(h, piece_hash)
            }
            Phase::Inner(level, n) => {
                let st = piece.ok_or("part of its index is missing")?;
                let hash = self.index[&(level, n)];
                let children = files::open_inner(&self.old_key, &hash, &st).map_err(|e| format!("its index: {e:?}"))?;
                // (Each child's place: `files::Root::parent`, the one walk.)
                let first = n * files::FANOUT as u64;
                for (i, c) in children.iter().enumerate() {
                    self.index.insert((level - 1, first + i as u64), *c);
                }
                self.walk(h, piece_hash)
            }
            Phase::Leaf(n) => {
                let st = piece.ok_or("part of its index is missing")?;
                let hash = self.index[&(0, n)];
                let gens = files::open_leaf(&self.old_key, &self.plan(), n, &hash, &st).map_err(|e| format!("its index: {e:?}"))?;
                self.listed.extend(gens);
                let g = self.stored.len() as u64;
                self.ask_gen(h, piece_hash, g)
            }
            Phase::Fragments { g, mut out, mut held, mut empty, round } => {
                let Some(at) = out.iter().position(|(_, w)| *w == id) else { return Ok(Vec::new()) };
                let (j, _) = out.remove(at);
                match piece {
                    Some(st) => {
                        h.set_secret(&[FRAG, &[j]].concat(), &st);
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
                        return self.code_gen(h, piece_hash, g, d);
                    }
                    if out.is_empty() {
                        if round + 1 < files::ASK_ROUNDS {
                            let again = d.missing();
                            return Ok(self.ask_fragments(piece_hash, g, &again, held, empty, round + 1));
                        }
                        return Err(format!("part {} could not be rebuilt ({} of {} pieces found after {} rounds)", g + 1, d.rank(), d.k(), files::ASK_ROUNDS));
                    }
                }
                self.phase = Phase::Fragments { g, out, held, empty, round };
                Ok(Vec::new())
            }
            _ => Ok(Vec::new()),
        }
    }

    /// An ANSWER to a PUT. What to send next.
    pub fn put<H: Host>(&mut self, h: &mut H, piece_hash: &[u8; 32], id: [u8; 32], ok: bool) -> Result<Vec<Io>, String> {
        self.answered(&id);
        let io = self.put_(h, piece_hash, id, ok)?;
        self.track(piece_hash, &io);
        Ok(io)
    }
    fn put_<H: Host>(&mut self, h: &mut H, piece_hash: &[u8; 32], id: [u8; 32], ok: bool) -> Result<Vec<Io>, String> {
        match self.phase.clone() {
            Phase::Putting { g, mut out, ok: mut stored } => {
                let Some(at) = out.iter().position(|(w, _, _)| *w == id) else { return Ok(Vec::new()) };
                let (_, j, hash) = out.remove(at);
                if ok {
                    stored.push((j, hash));
                }
                if !out.is_empty() {
                    self.phase = Phase::Putting { g, out, ok: stored };
                    return Ok(Vec::new());
                }
                if stored.len() < self.plan().k(g) {
                    return Err(format!("part {} could not be stored ({} of the {} it needs)", g + 1, stored.len(), self.plan().k(g)));
                }
                stored.sort();
                self.stored.push(stored);
                self.ask_gen(h, piece_hash, g + 1)
            }
            Phase::Index { mut out, refused } => {
                let Some(at) = out.iter().position(|w| *w == id) else { return Ok(Vec::new()) };
                out.remove(at);
                let refused = refused || !ok;
                if out.is_empty() {
                    if refused {
                        return Err("its index could not be stored".into());
                    }
                    self.phase = Phase::Done;
                } else {
                    self.phase = Phase::Index { out, refused };
                }
                Ok(Vec::new())
            }
            _ => Ok(Vec::new()),
        }
    }

    /// Ended: re-keyed (its new row, the old pieces to burn), or nothing to move.
    pub fn end(&self) -> Option<End> {
        (self.phase == Phase::Done).then(|| End::Done { row: self.new_row(), old: self.old_pieces() })
    }
    /// The row brought up to date only (the same key and burn).
    pub fn same_row(&self) -> End {
        End::Same { row: self.new_row() }
    }
    /// Whether the old pieces are this space's to burn: keyed under its salt (`n` ≥ 0), with a burn hash and the
    /// secret that names it.
    pub fn burnable(&self) -> Option<[u8; 32]> {
        let v: serde_json::Value = serde_json::from_str(&self.row).ok()?;
        let n = v["n"].as_i64().unwrap_or(-2);
        let b = v["b"].as_str().and_then(unhex32)?;
        let x = v["x"].as_str().and_then(unhex32)?;
        (n >= 0 && sha(&x) == b && self.old_key != self.new_key).then_some(x)
    }
}
