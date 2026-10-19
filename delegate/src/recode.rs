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

use crate::file_io::{FileRead, FileWrite, Read, Wrote};
use crate::upkeep::Io;

/// Where a re-key's read and write keep what they hold between calls (`file_io`).
const SLOT: &[u8] = b"rekey/";

fn sha(b: &[u8]) -> [u8; 32] {
    sha2::Sha256::digest(b).into()
}
/// The BURN SECRET a space's salt gives a file key (`files.secretOf`).
pub fn burn_secret(salt: &[u8; 32], key: &[u8; 32]) -> [u8; 32] {
    sha(&[b"craftworks burn".as_slice(), salt, key].concat())
}
/// A piece's BURN: `BURNED ‖ sha-256(secret) ‖ secret`.
pub fn burned(secret: &[u8; 32]) -> Vec<u8> {
    [&[3u8][..], &sha(secret), secret].concat()
}

/// A RE-KEY in progress: the file READ under its old key piped, part by part, into a WRITE under its new one
/// (`file_io`, the one reader and writer).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recode {
    pub id: String,
    /// Its row as read (JSON object).
    pub row: String,
    old_key: [u8; 32],
    pub new_key: [u8; 32],
    pub burn: [u8; 32],
    pub secret: Option<[u8; 32]>,
    pub n: i64,
    pub h: [u8; 32],
    read: FileRead,
    write: Option<FileWrite>,
    pub root: Option<[u8; 32]>,
}

/// What the re-key ended in.
#[derive(Debug, Clone, PartialEq)]
pub enum End {
    /// Re-keyed: the row's new value (JSON), and the OLD pieces' addresses (root, index, fragments) to burn.
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
        let (read, io) = FileRead::start(SLOT, piece_hash, old_key, old_root);
        Ok((Recode { id: id.to_string(), row: row.to_string(), old_key, new_key, burn, secret, n, h, read, write: None, root: None }, io))
    }

    /// Whether it waits on contract `id`.
    pub fn wants(&self, id: &[u8; 32]) -> bool {
        self.read.wants(id) || self.write.as_ref().is_some_and(|w| w.wants(id))
    }

    /// The same key and burn hash it has: nothing to move (its row's fields brought up to date only).
    pub fn same(&self) -> bool {
        let v: serde_json::Value = serde_json::from_str(&self.row).unwrap_or_default();
        v["key"].as_str() == Some(&hex(self.new_key)) && v["b"].as_str() == Some(&hex(self.burn))
    }

    /// The new row: its key, root, burn hash and secret, hash and salt number (`at` set by its writer).
    fn new_row(&self) -> String {
        let mut v: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&self.row).unwrap_or_default();
        v.remove("x");
        v.insert("key".into(), hex(self.new_key).into());
        if let Some(root) = self.root {
            v.insert("root".into(), hex(root).into());
        }
        v.insert("b".into(), hex(self.burn).into());
        if let Some(s) = self.secret {
            v.insert("x".into(), hex(s).into());
        }
        v.insert("h".into(), hex(self.h).into());
        v.insert("n".into(), self.n.into());
        serde_json::Value::Object(v).to_string()
    }

    /// An ANSWER to a GET (the read's): a part decoded goes to the write.
    pub fn got<H: Host>(&mut self, h: &mut H, piece_hash: &[u8; 32], id: [u8; 32], state: Option<Vec<u8>>) -> Result<Vec<Io>, String> {
        match self.read.got(h, piece_hash, id, state)? {
            Read::Io(io) => Ok(io),
            Read::Part(g, plain) => {
                let size = self.read.size();
                let w = self.write.get_or_insert_with(|| FileWrite::new(SLOT, self.new_key, self.burn, size));
                Ok(w.part(h, piece_hash, g, &plain))
            }
        }
    }

    /// An ANSWER to a PUT (the write's): a part stored asks the read for the next; every part in, the index.
    pub fn put<H: Host>(&mut self, h: &mut H, piece_hash: &[u8; 32], id: [u8; 32], ok: bool) -> Result<Vec<Io>, String> {
        let Some(w) = self.write.as_mut() else { return Ok(Vec::new()) };
        match w.put(h, piece_hash, id, ok)? {
            Wrote::Io(io) => Ok(io),
            Wrote::Part(_) => {
                let io = self.read.next(h, piece_hash)?;
                if self.read.done() {
                    return self.write.as_mut().expect("writing").index(piece_hash);
                }
                Ok(io)
            }
            Wrote::Done(root) => {
                self.root = Some(root);
                Ok(Vec::new())
            }
        }
    }

    /// Ended: re-keyed (its new row, the old pieces to burn).
    pub fn end(&self) -> Option<End> {
        self.root.map(|_| End::Done { row: self.new_row(), old: self.read.pieces() })
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
