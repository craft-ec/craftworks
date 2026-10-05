//! A FILE RE-KEYED with no page open (R4d) — `files.recode` as a step machine: the file read under the key its row has
//! (its root, its leaves, each generation's listed fragments), coded under the NEW key (from its content hash and the
//! space's salt now; public: the content alone), put in `piece` contracts named by the new burn hash, its index last.
//! Then (the round's part) its row changed and its OLD pieces BURNED with the secret its row kept.
//!
//! One generation at a time: its fragments GOT (each kept in a secret of its own until enough are in — a generation
//! is up to 16 × 256 KiB), decoded, coded again, PUT; their answers counted. A file whose row has no content hash (an
//! adopted one from before hashes travelled), or whose index is deeper than one level, is left to a page.

use serde::{Deserialize, Serialize};
use sha2::Digest;

use craftworks_files as files;
use craftworks_identity::Host;

use crate::upkeep::{Code, Io};

/// A fragment of the generation being read, kept here until decoded: `REKEY_FRAG ‖ j`.
const FRAG: &[u8] = b"identity_upkeep/rekey/frag/";

fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let v: Option<Vec<u8>> = (0..32).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()).collect();
    v?.try_into().ok()
}
fn hex(b: &[u8]) -> String {
    crate::table::hex(b)
}
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
    /// Leaf `n` asked.
    Leaf(u64),
    /// Generation g's fragments asked: those still out.
    Fragments { g: u64, out: Vec<(u8, [u8; 32])>, held: Vec<u8> },
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
    leaves: Vec<[u8; 32]>,
    /// Each generation's listed fragments, as its leaf says (filled leaf by leaf).
    listed: Vec<Vec<(u8, [u8; 32])>>,
    /// The new pieces stored per generation.
    pub stored: Vec<Vec<(u8, [u8; 32])>>,
    pub root: Option<[u8; 32]>,
    phase: Phase,
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
            leaves: Vec::new(),
            listed: Vec::new(),
            stored: Vec::new(),
            root: None,
            phase: Phase::Root,
        };
        let id = crate::upkeep::id_of(piece_hash, &files::hashed_address(&old_key, &old_root));
        Ok((r, vec![Io::Get { id, subscribe: false }]))
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

    /// The OLD pieces, to burn: its root, its leaves, every listed fragment.
    fn old_pieces(&self) -> Vec<[u8; 32]> {
        let mut out = vec![files::hashed_address(&self.old_key, &self.old_root)];
        out.extend(self.leaves.iter().map(|h| files::hashed_address(&self.old_key, h)));
        for (g, l) in self.listed.iter().enumerate() {
            out.extend(l.iter().map(|(j, _)| files::fragment_address(&self.old_key, g as u64, *j)));
        }
        out
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
            let n = g / files::GENS_PER_LEAF as u64;
            let leaf = *self.leaves.get(n as usize).ok_or("a leaf its root does not list")?;
            self.phase = Phase::Leaf(n);
            return Ok(vec![Io::Get { id: crate::upkeep::id_of(piece_hash, &files::hashed_address(&self.old_key, &leaf)), subscribe: false }]);
        }
        let listed = self.listed[g as usize].clone();
        for (j, _) in &listed {
            h.set_secret(&[FRAG, &[*j]].concat(), &[]);
        }
        let out: Vec<(u8, [u8; 32])> = listed.iter().map(|(j, _)| (*j, crate::upkeep::id_of(piece_hash, &files::fragment_address(&self.old_key, g, *j)))).collect();
        let io = out.iter().map(|(_, id)| Io::Get { id: *id, subscribe: false }).collect();
        self.phase = Phase::Fragments { g, out, held: Vec::new() };
        Ok(io)
    }

    /// Generation g decoded from the fragments held, coded under the new key, PUT.
    fn code_gen<H: Host>(&mut self, h: &mut H, piece_hash: &[u8; 32], g: u64, held: &[u8]) -> Result<Vec<Io>, String> {
        let plan = self.plan();
        let mut d = files::Decoder::new(&self.old_key, &plan, g, self.listed[g as usize].clone());
        for j in held {
            let st = h.get_secret(&[FRAG, &[*j]].concat()).unwrap_or_default();
            let _ = d.add(*j, &st);
            if d.done() {
                break;
            }
        }
        for (j, _) in &self.listed[g as usize] {
            h.set_secret(&[FRAG, &[*j]].concat(), &[]);
        }
        if !d.done() {
            return Err(format!("part {} could not be rebuilt ({} of {} pieces)", g + 1, d.rank(), plan.k(g)));
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
        // A piece's state: `LIVE ‖ burn hash ‖ piece` (burned or missing: none).
        let piece = state.filter(|st| st.first() == Some(&2) && st.len() > 33).map(|st| st[33..].to_vec());
        match self.phase.clone() {
            Phase::Root => {
                let st = piece.ok_or("its root is not on the network")?;
                let r = files::Root::open(&self.old_key, &self.old_root, &st).map_err(|e| format!("its root: {e:?}"))?;
                if r.depth != 0 {
                    return Err("an index deeper than one level (a page re-keys it)".into());
                }
                self.size = r.plan.size;
                self.leaves = r.children.clone();
                self.stored = Vec::new();
                self.ask_gen(h, piece_hash, 0)
            }
            Phase::Leaf(n) => {
                let st = piece.ok_or("part of its index is missing")?;
                let hash = self.leaves[n as usize];
                let gens = files::open_leaf(&self.old_key, &self.plan(), n, &hash, &st).map_err(|e| format!("its index: {e:?}"))?;
                self.listed.extend(gens);
                let g = self.stored.len() as u64;
                self.ask_gen(h, piece_hash, g)
            }
            Phase::Fragments { g, mut out, mut held } => {
                let Some(at) = out.iter().position(|(_, w)| *w == id) else { return Ok(Vec::new()) };
                let (j, _) = out.remove(at);
                if let Some(st) = piece {
                    h.set_secret(&[FRAG, &[j]].concat(), &st);
                    held.push(j);
                }
                let k = self.plan().k(g);
                // Enough (k valid ones may decode; any extra is spare) — or every one answered.
                if held.len() >= k + 2 || out.is_empty() {
                    return self.code_gen(h, piece_hash, g, &held);
                }
                self.phase = Phase::Fragments { g, out, held };
                Ok(Vec::new())
            }
            _ => Ok(Vec::new()),
        }
    }

    /// An ANSWER to a PUT. What to send next.
    pub fn put<H: Host>(&mut self, h: &mut H, piece_hash: &[u8; 32], id: [u8; 32], ok: bool) -> Result<Vec<Io>, String> {
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
