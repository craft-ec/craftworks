//! A SPACE's TABLE READ with no page open (R4a): what `storage.js` does for a page — every member's feed of a table,
//! merged — as a pure step machine (each step a GET out, an answer in, across calls; its state kept between them).
//!
//!   1. The space's WRITERS BAG (sealed with epoch 0's `bag-writers` key, AES-GCM): which writers have a catalog there
//!      — only those are asked (a GET of a contract nobody made is a whole network search).
//!   2. Each listed writer of the group's ROSTER (a DID writes a space under its `space_writer`, the same on every
//!      device): its CATALOG `x<id12>-tables` — whether it has a feed of the table, and where (blinded, or by name).
//!   3. Each feed's TAIL, then its tree's BLOCKS (Sealed contracts at the addresses its key gives; a public table's in
//!      Block contracts), each sealed with an EPOCH's key for that table (the member's kept secrets).
//!   4. DEPARTED writers (the space's `departed` table: a removed member's feeds as they stood when removed): read too,
//!      each up to its cap.
//!   5. The feeds MERGED (`feed`): the table's rows.
//!
//! A writer whose catalog or feed does not read here (absent, sealed with a key not held) is left out — the same as a
//! page: nothing is written over what is not seen, because upkeep writes only its own feed.

use std::collections::{BTreeMap, BTreeSet};

use craftworks_data as data;
use craftworks_identity as identity;
use serde::{Deserialize, Serialize};

use crate::upkeep::Io;

/// The contracts a read names: the Tail's code (its bytes: a tail's instance comes from them), and the hashes of the
/// Bag, Sealed and Block codes.
pub struct Codes<'a> {
    pub tail: &'a [u8],
    pub bag_hash: [u8; 32],
    pub sealed_hash: [u8; 32],
    pub block_hash: [u8; 32],
}

/// What a tail is read for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Purpose {
    /// A writer's catalog of the space (a departed writer's: `capped` its feeds' caps by table).
    Catalog { capped: Option<BTreeMap<String, u64>> },
    /// A writer's feed of a table, up to `cap` (a departed writer's) — its rows merged into the table's.
    Feed { cap: Option<u64> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Job {
    owner: [u8; 32],
    /// The table's NAME (its keys come from it).
    table: String,
    /// The tail's label: the name, or the name blinded.
    label: String,
    public: bool,
    purpose: Purpose,
    /// The tail's state as the network gave it (`None` once answered: not there).
    state: Option<Vec<u8>>,
    /// Its tree's blocks, as fetched: `(block id, contract state)`.
    blocks: Vec<([u8; 32], Vec<u8>)>,
    /// Read to its end: its rows as stored (`None`: absent or unreadable here).
    rows: Option<Option<BTreeMap<Vec<u8>, Vec<u8>>>>,
}

/// A READ of some of a space's tables, in progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reading {
    space: [u8; 32],
    /// The epoch secrets this member holds of the space (every table's epoch keys come from them).
    epochs: BTreeMap<u64, [u8; 32]>,
    /// The roster's writers.
    writers: Vec<[u8; 32]>,
    /// The tables wanted (full names: `x<id12>-<name>`).
    tables: Vec<String>,
    jobs: Vec<Job>,
    /// GETs out: `(contract id, job, the block it is — none: the tail)`.
    waiting: Vec<([u8; 32], usize, Option<[u8; 32]>)>,
    /// The bag: asked (and its id), read.
    bag: Option<[u8; 32]>,
    bag_read: bool,
    /// `departed` read, and its departed writers' catalogs asked.
    departed_asked: bool,
    /// Something did not read (named, for the round's log).
    pub notes: Vec<String>,
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let v: Option<Vec<u8>> = (0..32).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()).collect();
    v?.try_into().ok()
}

/// A space's table's full name: `x<first 12 hex of its id>-<name>` (`space.tableOf`).
pub fn table_name(space: &[u8; 32], name: &str) -> String {
    format!("x{}-{name}", &hex(space)[..12])
}
/// A table found by its NAME, never blinded: the space's own (catalog, members, log) and its public ones (`-pub-`).
fn by_name(space: &[u8; 32], table: &str) -> bool {
    ["tables", "members", "log"].iter().any(|n| table == table_name(space, n)) || is_public(space, table)
}
fn is_public(space: &[u8; 32], table: &str) -> bool {
    table.starts_with(&table_name(space, "pub-"))
}

/// The address of a space's SEALED bag `name` (`index.sealedAddress`): its pointer address from the space's key for it.
pub fn sealed_bag_address(space: &[u8; 32], name: &str) -> [u8; 32] {
    use sha2::Digest;
    let key = identity::space_table_key(space, &format!("bag-{name}"));
    let digest: [u8; 32] = sha2::Sha256::digest(format!("craftworks pointers space {}", hex(&key)).as_bytes()).into();
    identity::inbox_address(&digest)
}

/// An item of a space's sealed bag opened: `epoch (u32, big-endian) ‖ nonce (12) ‖ AES-GCM` under that epoch's key for
/// the bag (`index.spacePoint`).
pub fn open_bag_item(epochs: &BTreeMap<u64, [u8; 32]>, name: &str, item: &[u8]) -> Option<serde_json::Value> {
    use aes_gcm::aead::{Aead, KeyInit};
    if item.len() < 17 {
        return None;
    }
    let epoch = u32::from_be_bytes(item[..4].try_into().ok()?) as u64;
    let key = identity::epoch_table_key(epochs.get(&epoch)?, &format!("bag-{name}"));
    let cipher = aes_gcm::Aes256Gcm::new_from_slice(&key).ok()?;
    let plain = cipher.decrypt(aes_gcm::Nonce::from_slice(&item[4..16]), &item[16..]).ok()?;
    serde_json::from_slice(&plain).ok()
}

impl Reading {
    /// A read of `tables` (their short names: `files`, `acts`…) of `space`, as a member holding `epochs`, whose group's
    /// writers are `writers`. The first GET: the writers bag.
    pub fn new(c: &Codes, space: [u8; 32], epochs: BTreeMap<u64, [u8; 32]>, writers: Vec<[u8; 32]>, tables: &[&str]) -> (Reading, Vec<Io>) {
        let bag = crate::upkeep::id_of(&c.bag_hash, &sealed_bag_address(&space, "writers"));
        let r = Reading {
            space,
            epochs,
            writers,
            tables: tables.iter().map(|t| table_name(&space, t)).collect(),
            jobs: Vec::new(),
            waiting: Vec::new(),
            bag: Some(bag),
            bag_read: false,
            departed_asked: false,
            notes: Vec::new(),
        };
        (r, vec![Io::Get { id: bag, subscribe: false }])
    }

    /// Done: every tail asked read to its end.
    pub fn done(&self) -> bool {
        self.bag_read && self.waiting.is_empty() && self.jobs.iter().all(|j| j.rows.is_some())
    }

    /// Whether this read waits on contract `id`.
    pub fn wants(&self, id: &[u8; 32]) -> bool {
        self.bag == Some(*id) && !self.bag_read || self.waiting.iter().any(|(w, _, _)| w == id)
    }

    /// A tail opened as its job says: its keys, its state, its blocks.
    fn open(&self, c: &Codes, j: &Job) -> data::Open {
        let mut o = data::Open::new(c.tail, &j.owner, &j.label);
        if j.public {
            o.public = true;
        } else {
            o.set_table_key(identity::space_table_key(&self.space, &j.table));
            for (e, s) in &self.epochs {
                o.epoch_key(*e, Some(identity::epoch_table_key(s, &j.table)), false);
            }
        }
        if let Some(st) = &j.state {
            o.absorb(st);
        }
        for (cid, st) in &j.blocks {
            o.absorb_block(cid, st);
        }
        o
    }

    /// A job's tail read on as far as it goes: its rows, or the blocks to GET next.
    fn advance(&mut self, c: &Codes, i: usize) -> Vec<Io> {
        let mut o = self.open(c, &self.jobs[i]);
        for _ in 0..4 {
            match o.opened() {
                Ok(data::Step::Ready(rows)) => {
                    self.jobs[i].rows = Some(Some(rows));
                    return self.finished(c, i);
                }
                Ok(data::Step::Need(cids)) => {
                    let mut io = Vec::new();
                    for cid in cids {
                        let Some((sealed, params)) = o.block_params(&cid) else { continue };
                        let id = crate::upkeep::id_of(if sealed { &c.sealed_hash } else { &c.block_hash }, &params);
                        if !self.waiting.iter().any(|(w, _, _)| *w == id) {
                            self.waiting.push((id, i, Some(cid)));
                            io.push(Io::Get { id, subscribe: false });
                        }
                    }
                    if io.is_empty() && !self.waiting.iter().any(|(_, j, _)| *j == i) {
                        return self.fail(c, i, "its tree names blocks it cannot place");
                    }
                    return io;
                }
                // Every epoch held was given: one it still asks for is one this member never had.
                Ok(data::Step::Keys(es)) => {
                    for e in es {
                        o.epoch_key(e, None, false);
                    }
                }
                Err(why) => return self.fail(c, i, &why),
            }
        }
        self.fail(c, i, "it asks for keys this member does not hold")
    }

    fn fail(&mut self, c: &Codes, i: usize, why: &str) -> Vec<Io> {
        let j = &self.jobs[i];
        self.notes.push(format!("{} of {}…: {why}", j.table, &hex(&j.owner)[..12]));
        self.jobs[i].rows = Some(None);
        self.finished(c, i)
    }

    /// A job read to its end: a catalog names the feeds to read; `departed`, the departed writers to read.
    fn finished(&mut self, c: &Codes, i: usize) -> Vec<Io> {
        let j = self.jobs[i].clone();
        let Some(Some(rows)) = &j.rows else { return Vec::new() };
        match &j.purpose {
            Purpose::Catalog { capped } => {
                // A catalog's own rows, merged alone: each table it lists, and where it is.
                let listed = data_rows(&j.owner, rows);
                let mut io = Vec::new();
                for t in self.tables.clone() {
                    let cap = match capped {
                        Some(caps) => match caps.get(&t) {
                            Some(cap) => Some(*cap),
                            None => continue,
                        },
                        None => None,
                    };
                    let Some(v) = listed.get(t.as_bytes()) else { continue };
                    let place: serde_json::Value = serde_json::from_slice(v).unwrap_or(serde_json::Value::Null);
                    if place["n"] == 1 {
                        continue; // listed, never made
                    }
                    let label = if place["b"] == 1 && !by_name(&self.space, &t) { identity::blind_name(&identity::space_table_key(&self.space, &t), &t) } else { t.clone() };
                    io.extend(self.ask(c, j.owner, &t, label, Purpose::Feed { cap }));
                }
                io
            }
            Purpose::Feed { .. } => {
                if j.table == table_name(&self.space, "departed") && !self.departed_asked {
                    self.departed_asked = true;
                    return self.departed(c);
                }
                Vec::new()
            }
        }
    }

    /// DEPARTED writers (rows `<node hex>` → `{ heads: { table: last seq } }`): each one not of the roster, its catalog
    /// read, its feeds of the wanted tables up to their caps.
    fn departed(&mut self, c: &Codes) -> Vec<Io> {
        let name = table_name(&self.space, "departed");
        let feeds: Vec<([u8; 32], BTreeMap<Vec<u8>, Vec<u8>>)> =
            self.jobs.iter().filter(|j| j.table == name).filter_map(|j| Some((j.owner, j.rows.clone()??))).collect();
        let merged = merge(&feeds, &BTreeMap::new());
        let mut io = Vec::new();
        for (k, row) in merged {
            let Some(node) = std::str::from_utf8(&k).ok().and_then(unhex32) else { continue };
            if self.writers.contains(&node) {
                continue;
            }
            let v: serde_json::Value = serde_json::from_slice(&row.value).unwrap_or(serde_json::Value::Null);
            let caps: BTreeMap<String, u64> = v["heads"].as_object().map(|o| o.iter().filter_map(|(t, s)| Some((t.clone(), s.as_u64()?))).collect()).unwrap_or_default();
            if caps.is_empty() {
                continue;
            }
            let cat = table_name(&self.space, "tables");
            io.extend(self.ask(c, node, &cat, cat.clone(), Purpose::Catalog { capped: Some(caps) }));
        }
        io
    }

    /// A tail asked for (once).
    fn ask(&mut self, _c: &Codes, owner: [u8; 32], table: &str, label: String, purpose: Purpose) -> Vec<Io> {
        if self.jobs.iter().any(|j| j.owner == owner && j.label == label) {
            return Vec::new();
        }
        let public = is_public(&self.space, table);
        self.jobs.push(Job { owner, table: table.to_string(), label, public, purpose, state: None, blocks: Vec::new(), rows: None });
        let i = self.jobs.len() - 1;
        let id = data::Open::new(_c.tail, &owner, &self.jobs[i].label).id_bytes();
        self.waiting.push((id, i, None));
        vec![Io::Get { id, subscribe: false }]
    }

    /// An ANSWER: the bag (its writers' catalogs asked), a tail (read on), or a block (read on).
    pub fn got(&mut self, c: &Codes, id: [u8; 32], state: Option<Vec<u8>>) -> Vec<Io> {
        if self.bag == Some(id) && !self.bag_read {
            self.bag_read = true;
            let mut listed = BTreeSet::new();
            if let Some(st) = &state {
                let address = sealed_bag_address(&self.space, "writers");
                for p in craftworks_bag_contract::read(&address, st).unwrap_or_default() {
                    if let Some(w) = open_bag_item(&self.epochs, "writers", craftworks_bag_contract::payload(&p)).and_then(|v| v["w"].as_str().and_then(unhex32)) {
                        listed.insert(w);
                    }
                }
            }
            let cat = table_name(&self.space, "tables");
            let mut io = Vec::new();
            let mut wanted = self.tables.clone();
            // `departed` read alongside, for the departed writers' feeds.
            let departed = table_name(&self.space, "departed");
            if !wanted.contains(&departed) {
                wanted.push(departed);
                self.tables = wanted;
            }
            for w in self.writers.clone().into_iter().filter(|w| listed.contains(w)) {
                io.extend(self.ask(c, w, &cat, cat.clone(), Purpose::Catalog { capped: None }));
            }
            return io;
        }
        let Some(at) = self.waiting.iter().position(|(w, _, _)| *w == id) else { return Vec::new() };
        let (_, i, block) = self.waiting.remove(at);
        match (block, state) {
            (None, Some(st)) => self.jobs[i].state = Some(st),
            (None, None) => {
                self.jobs[i].rows = Some(None);
                return Vec::new();
            }
            (Some(cid), Some(st)) => self.jobs[i].blocks.push((cid, st)),
            (Some(_), None) => {
                // A block not there: the tail cannot be read whole (a page would repair it from parity).
                if !self.waiting.iter().any(|(_, j, _)| *j == i) {
                    return self.fail(c, i, "a block of its tree is missing");
                }
                return Vec::new();
            }
        }
        if self.waiting.iter().any(|(_, j, _)| *j == i) {
            return Vec::new(); // more of its blocks still coming
        }
        self.advance(c, i)
    }

    /// A table's ROWS: its feeds merged (a departed writer's up to its cap). `name`: its short name.
    pub fn rows(&self, name: &str) -> BTreeMap<Vec<u8>, craftworks_feed::Row> {
        let t = table_name(&self.space, name);
        let mut caps = BTreeMap::new();
        let feeds: Vec<([u8; 32], BTreeMap<Vec<u8>, Vec<u8>>)> = self
            .jobs
            .iter()
            .filter(|j| j.table == t)
            .filter_map(|j| {
                if let Purpose::Feed { cap: Some(cap) } = j.purpose {
                    caps.insert(j.owner, cap);
                }
                Some((j.owner, j.rows.clone()??))
            })
            .collect();
        merge(&feeds, &caps)
    }

    /// A writer's tail of a table (`name`: short — `tables` its catalog), as read: its state (`None`: not there) and its
    /// tree's blocks — what a write from upkeep goes on from. `None`: never asked.
    pub fn own(&self, owner: &[u8; 32], name: &str) -> Option<(Option<Vec<u8>>, Vec<([u8; 32], Vec<u8>)>)> {
        let t = table_name(&self.space, name);
        self.jobs.iter().find(|j| j.owner == *owner && j.table == t).map(|j| (j.state.clone(), j.blocks.clone()))
    }

    /// A writer's tail of a table WRITTEN since (its new state, its tree's blocks): what the next write goes on from.
    /// Its rows read again from what it holds (its tree's blocks are all held: it was just written here).
    pub fn took_own(&mut self, c: &Codes, owner: &[u8; 32], name: &str, label: &str, state: Vec<u8>, blocks: Vec<([u8; 32], Vec<u8>)>) {
        let t = table_name(&self.space, name);
        let i = match self.jobs.iter().position(|j| j.owner == *owner && j.table == t) {
            Some(i) => {
                self.jobs[i].state = Some(state);
                self.jobs[i].blocks = blocks;
                i
            }
            None => {
                self.jobs.push(Job {
                owner: *owner,
                table: t.clone(),
                label: label.to_string(),
                public: is_public(&self.space, &t),
                purpose: Purpose::Feed { cap: None },
                state: Some(state),
                blocks,
                rows: None,
                });
                self.jobs.len() - 1
            }
        };
        let mut o = self.open(c, &self.jobs[i]);
        self.jobs[i].rows = Some(match o.opened() {
            Ok(data::Step::Ready(rows)) => Some(rows),
            _ => None,
        });
    }

    pub fn space(&self) -> [u8; 32] {
        self.space
    }
    pub fn epochs(&self) -> &BTreeMap<u64, [u8; 32]> {
        &self.epochs
    }
}

/// A feed's rows merged alone (its catalog: each table it lists, as plain bytes).
fn data_rows(owner: &[u8; 32], rows: &BTreeMap<Vec<u8>, Vec<u8>>) -> BTreeMap<Vec<u8>, Vec<u8>> {
    craftworks_feed::merge(&[(*owner, rows)]).into_iter().map(|(k, r)| (k, r.value)).collect()
}

/// FEEDS MERGED, each up to its cap (a version's sequence is in its envelope).
fn merge(feeds: &[([u8; 32], BTreeMap<Vec<u8>, Vec<u8>>)], caps: &BTreeMap<[u8; 32], u64>) -> BTreeMap<Vec<u8>, craftworks_feed::Row> {
    let capped: Vec<([u8; 32], BTreeMap<Vec<u8>, Vec<u8>>)> = feeds
        .iter()
        .map(|(w, rows)| match caps.get(w) {
            Some(cap) => (*w, rows.iter().filter(|(_, v)| craftworks_feed::open(v).is_some_and(|ver| ver.id.1 <= *cap)).map(|(k, v)| (k.clone(), v.clone())).collect()),
            None => (*w, rows.clone()),
        })
        .collect();
    let refs: Vec<([u8; 32], &BTreeMap<Vec<u8>, Vec<u8>>)> = capped.iter().map(|(w, r)| (*w, r)).collect();
    craftworks_feed::merge(&refs)
}
