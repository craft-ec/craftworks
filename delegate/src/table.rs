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

pub use craftworks_identity::hex;
use craftworks_identity::unhex32;
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
    /// The key log's code hash: a member's devices are checked against their key log.
    pub idlog_hash: [u8; 32],
    /// The Devices contract's code hash: a member's DEVICES are read there (owner 10-09: on their key log's current
    /// key) — their card only until they have one.
    pub devices_hash: [u8; 32],
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
    /// MEMBERS whose devices are still being read: `(did, their key log's state once got, the contract waited on)`.
    resolving: Vec<([u8; 32], Option<Vec<u8>>, [u8; 32], Option<(Vec<Vec<u8>>, bool, bool)>)>,
    /// Each writer's DID (a device of a member, or a roster writer given): whose acts are whose.
    pub writer_dids: Vec<([u8; 32], String)>,
    /// Each member's BEEKEM MEMBER from their card (`beekem`: its credential checked against their key log's data key):
    /// `(DID, member id, first leaf key)` — whom an operation's signer is, whom to add (K5).
    #[serde(default)]
    pub beekem: Vec<(String, [u8; 32], [u8; 32])>,
    /// A member's CARD whose tree is being fetched (a card with many rows keeps them in blocks): `(DID, its state,
    /// the blocks got, the block contracts still out — with each one's cid)`.
    /// What cards SAID about their member that is not a failed read (none on it, one that does not check) — reported by
    /// the keys round, never a short read.
    #[serde(default)]
    pub card_says: Vec<String>,
    /// What did not read, as the one rule takes it (`guards::list_whole`): members' devices, and feeds `(table, why)`.
    #[serde(default)]
    pub devices_unread: Vec<String>,
    #[serde(default)]
    pub feeds_unread: Vec<(String, String)>,
    /// A CARDS-ONLY read in progress (`cards_of`): members' key logs and cards, nothing else (no catalogs, feeds or
    /// writers) — what the node's keys round still needs, on top of the read it has.
    #[serde(default)]
    cards_only: bool,
    #[serde(default)]
    card_trees: Vec<([u8; 32], Vec<u8>, Vec<([u8; 32], Vec<u8>)>, Vec<([u8; 32], [u8; 32])>)>,
}


/// A space's table's full name: `x<first 12 hex of its id>-<name>` (`space.tableOf`).
pub fn table_name(space: &[u8; 32], name: &str) -> String {
    format!("x{}-{name}", &hex(space)[..12])
}
/// A table found by its NAME, never blinded: the space's own (catalog, members, log) and its public ones (`-pub-`).
fn by_name(space: &[u8; 32], table: &str) -> bool {
    ["tables", "members", "log"].iter().any(|n| table == table_name(space, n)) || is_public(space, table)
}
pub(crate) fn is_public(space: &[u8; 32], table: &str) -> bool {
    table.starts_with(&table_name(space, "pub-"))
}

/// The address of a space's SEALED bag `name` (`index.sealedAddress`): its pointer address from the space's key for it.
pub fn sealed_bag_address(space: &[u8; 32], name: &str) -> [u8; 32] {
    use sha2::Digest;
    let key = identity::space_table_key(space, &format!("bag-{name}"));
    let digest: [u8; 32] = sha2::Sha256::digest(format!("craftworks pointers space {}", hex(&key)).as_bytes()).into();
    identity::inbox_address(&digest)
}

/// Where a tail's tree BLOCK is (the one placing: this reader's and upkeep's card read): a sealed tree's under the
/// Sealed code, a public one's under the Block code.
pub fn block_id(o: &data::Open, cid: &[u8; 32], sealed_hash: &[u8; 32], block_hash: &[u8; 32]) -> Option<[u8; 32]> {
    let (sealed, params) = o.block_params(cid)?;
    Some(crate::upkeep::id_of(if sealed { sealed_hash } else { block_hash }, &params))
}

/// An item of a space's sealed bag opened (`identity::bag_open`, the page's too): under the epoch's key for the bag it
/// says it was sealed at — or, an open bag's `BAG_OPEN`, the space's own.
pub fn open_bag_item(space: &[u8; 32], epochs: &BTreeMap<u64, [u8; 32]>, name: &str, item: &[u8]) -> Option<serde_json::Value> {
    let epoch = identity::bag_item_epoch(item)?;
    let key = if epoch == identity::BAG_OPEN && identity::bag_is_open(name) {
        identity::bag_open_key(space)
    } else {
        identity::epoch_table_key(epochs.get(&(epoch as u64))?, &format!("bag-{name}"))
    };
    serde_json::from_slice(&identity::bag_open(&key, item)?).ok()
}

/// An item for one of a space's OPEN bags (its writers list, an epoch's door), sealed `BAG_OPEN` (`identity::bag_seal`,
/// the page's too): what a page writes there.
pub fn seal_open_item(space: &[u8; 32], value: &serde_json::Value, nonce: [u8; 12]) -> Vec<u8> {
    identity::bag_seal(&identity::bag_open_key(space), identity::BAG_OPEN, value.to_string().as_bytes(), nonce)
}

impl Reading {
    /// What it still WAITS for (a stalled round says it): GETs out — how many, of tails and of blocks — and whether
    /// the writers bag is read.
    pub fn waiting_for(&self) -> String {
        let blocks = self.waiting.iter().filter(|(_, _, b)| b.is_some()).count();
        let bag = match (self.bag.is_some(), self.bag_read) {
            (_, true) => "read",
            (true, false) if self.resolving.is_empty() => "asked, not answered",
            _ => "not asked yet",
        };
        format!("{} read(s) unanswered ({} tail(s), {blocks} block(s)), {} member device lookup(s) out, writers bag {bag}", self.waiting.len(), self.waiting.len() - blocks, self.resolving.len())
    }
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
            resolving: Vec::new(),
            writer_dids: Vec::new(),
            beekem: Vec::new(),
            card_says: Vec::new(),
            devices_unread: Vec::new(),
            feeds_unread: Vec::new(),
            cards_only: false,
            card_trees: Vec::new(),
        };
        (r, vec![Io::Get { id: bag, subscribe: false }])
    }

    /// A read of `tables` whose WRITERS are the DEVICES of the group's `members` (DIDs) — a page writes a space under
    /// its member key, one per device: each member's key log, then their card's `nodes`, checked against the log (as
    /// `directory.devices`). Then the writers bag, as `new`.
    pub fn of_members(c: &Codes, space: [u8; 32], epochs: BTreeMap<u64, [u8; 32]>, members: &[[u8; 32]], tables: &[&str]) -> (Reading, Vec<Io>) {
        let (mut r, _) = Reading::new(c, space, epochs, Vec::new(), tables);
        let mut io = Vec::new();
        for did in members {
            let id = crate::upkeep::id_of(&c.idlog_hash, did);
            r.resolving.push((*did, None, id, None));
            io.push(Io::Get { id, subscribe: false });
        }
        if members.is_empty() {
            io.push(Io::Get { id: r.bag.expect("set"), subscribe: false });
        }
        (r, io)
    }

    /// THE CARDS OF `dids` read on top of this read (its tables kept): each one's key log, then its card — its BeeKEM
    /// member (`beekem`). Nothing else is asked (the node's keys round: only the accounts it has not learned yet, a few
    /// a round — fx 10-18: re-reading everyone the list ever named each round asked past the node's per-round cap).
    pub fn cards_of(&mut self, c: &Codes, dids: &[[u8; 32]]) -> Vec<Io> {
        self.cards_only = true;
        let mut io = Vec::new();
        for did in dids {
            let id = crate::upkeep::id_of(&c.idlog_hash, did);
            self.resolving.push((*did, None, id, None));
            io.push(Io::Get { id, subscribe: false });
        }
        io
    }

    /// A member's key log, card or devices contract answered: their devices, once known; the bag once every member's are.
    /// Order (as `directory.devices`): the KEY LOG, then the CARD, then the DEVICES contract only where the card says
    /// the account has one (`devices`), or where nothing on the card counts — an absent contract is never searched for
    /// on every round.
    fn resolved(&mut self, c: &Codes, at: usize, state: Option<Vec<u8>>) -> Vec<Io> {
        let (did, log, _, card_creds) = self.resolving[at].clone();
        let Some(log_state) = log else {
            // Their KEY LOG: its head names their data key — their card's address.
            let Some(st) = state.filter(|st| craftworks_idlog_contract::read(&did, st).is_some()) else {
                // (A cards-only read: an account with no key log has no member to learn — said, never a short read.)
                if self.cards_only {
                    self.card_says.push(format!("{}: no key log", craftworks_account::did(&did)));
                } else if let craftworks_account::DevicesStep::Done { unread: Some(why), .. } = craftworks_account::devices_step(&did, None, None, None) {
                    self.devices_unread.push(format!("{}: {why}", craftworks_account::did(&did)));
                }
                self.resolving.remove(at);
                return self.members_done(c);
            };
            let log = craftworks_idlog_contract::read(&did, &st).expect("checked");
            let card = data::Open::new(c.tail, &log.head().data, "card").id_bytes();
            self.resolving[at] = (did, Some(st), card, None);
            return vec![Io::Get { id: card, subscribe: false }];
        };
        let log = craftworks_idlog_contract::read(&did, &log_state).expect("checked when got");
        let dev = crate::upkeep::id_of(&c.devices_hash, &craftworks_devices_contract::params(&did, &c.idlog_hash));
        let name = craftworks_account::did(&did);
        let found = match card_creds {
            None => {
                // Their CARD: its `nodes` (each credential checked against the key log), and whether it says the account
                // has a devices contract.
                let mut o = data::Open::new(c.tail, &log.head().data, "card");
                o.public = true;
                let state_was_none = state.is_none();
                let tree = self.card_trees.iter().position(|t| t.0 == did);
                let absorbed = state.as_ref().is_some_and(|st| o.absorb(st));
                if let Some(k) = tree {
                    for (cid, st) in &self.card_trees[k].2 {
                        o.absorb_block(cid, st);
                    }
                }
                let rows = match (absorbed, o.opened()) {
                    (true, Ok(data::Step::Ready(rows))) => Some(rows),
                    // ITS TREE's BLOCKS fetched (once: a block still missing after that is a card that does not read).
                    (true, Ok(data::Step::Need(cids))) if tree.is_none() => {
                        let want: Vec<([u8; 32], [u8; 32])> = cids.iter().filter_map(|cid| Some((block_id(&o, cid, &c.sealed_hash, &c.block_hash)?, *cid))).collect();
                        if !want.is_empty() {
                            let io = want.iter().map(|(id, _)| Io::Get { id: *id, subscribe: false }).collect();
                            self.card_trees.push((did, state.clone().expect("absorbed"), Vec::new(), want));
                            return io;
                        }
                        None
                    }
                    _ => None,
                };
                let marked = rows.as_ref().and_then(|r| r.get(b"devices".as_slice())).is_some_and(|v| v.as_slice() == b"1");
                match rows.as_ref().map(|r| r.get(b"beekem".as_slice())) {
                    // (`account::beekem_member`, the page's too.)
                    Some(Some(v)) => match craftworks_account::beekem_member(&did, &log.head().data, v) {
                        Ok(m) => self.beekem.push((name.clone(), m.0, m.1)),
                        Err(why) => self.card_says.push(format!("{}: {why}", name.get(12..20).unwrap_or(&name))),
                    },
                    Some(None) => self.card_says.push(format!("{}: no member on its card", name.get(12..20).unwrap_or(&name))),
                    None if self.cards_only => self.card_says.push(format!("{}: its card did not read ({})", name.get(12..20).unwrap_or(&name), if state_was_none { "not there" } else { "unopened" })),
                    None => {}
                }
                if self.cards_only {
                    self.resolving.remove(at);
                    return self.members_done(c);
                }
                let card_read = rows.is_some();
                let creds: Vec<Vec<u8>> = rows
                    .and_then(|r| r.get(b"nodes".as_slice()).cloned())
                    .and_then(|v| serde_json::from_slice::<Vec<String>>(&v).ok())
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|h| (0..h.len() / 2).map(|i| u8::from_str_radix(h.get(2 * i..2 * i + 2)?, 16).ok()).collect())
                    .collect();
                // THE ONE STEP (`account::devices_step`, the page's too): their devices contract asked when it must be.
                let step = craftworks_account::devices_step(&did, Some(&log), card_read.then_some((creds.as_slice(), marked)), None);
                if step == craftworks_account::DevicesStep::ReadDevices {
                    self.resolving[at] = (did, Some(log_state), dev, Some((creds, card_read, marked)));
                    return vec![Io::Get { id: dev, subscribe: false }];
                }
                step
            }
            Some((creds, card_read, marked)) => craftworks_account::devices_step(&did, Some(&log), card_read.then_some((creds.as_slice(), marked)), Some(state.as_deref().and_then(craftworks_devices_contract::decode).as_deref())),
        };
        let craftworks_account::DevicesStep::Done { devices: found, unread } = found else { return Vec::new() };
        self.devices_unread.extend(unread.map(|u| format!("{name}: {u}")));
        for node in found {
            if !self.writers.contains(&node) {
                self.writers.push(node);
                self.writer_dids.push((node, name.clone()));
            }
        }
        self.resolving.remove(at);
        self.members_done(c)
    }
    fn members_done(&mut self, _c: &Codes) -> Vec<Io> {
        // (A cards-only read asks nothing more: its tables were read already.)
        if self.cards_only {
            return Vec::new();
        }
        if self.resolving.is_empty() {
            vec![Io::Get { id: self.bag.expect("set"), subscribe: false }]
        } else {
            Vec::new()
        }
    }

    /// Whether this read's member holds a key at a BeeKEM GENERATION (the space is on BeeKEM, whatever its feeds said).
    pub fn holds_generation(&self) -> bool {
        self.epochs.keys().any(|e| *e >= craftworks_groups::FIRST_GENERATION)
    }

    /// Done: every tail asked read to its end.
    pub fn done(&self) -> bool {
        self.resolving.is_empty() && self.bag_read && self.waiting.is_empty() && self.jobs.iter().all(|j| j.rows.is_some())
    }

    /// Whether this read waits on contract `id`.
    pub fn wants(&self, id: &[u8; 32]) -> bool {
        self.bag == Some(*id) && !self.bag_read || self.waiting.iter().any(|(w, _, _)| w == id) || self.resolving.iter().any(|(_, _, w, _)| w == id) || self.card_trees.iter().any(|t| t.3.iter().any(|(w, _)| w == id))
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
                        let Some(id) = block_id(&o, &cid, &c.sealed_hash, &c.block_hash) else { continue };
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
        self.feeds_unread.push((j.table.clone(), format!("{}…: {why}", &hex(&j.owner)[..12])));
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
                    // (`data::catalog_place`, the page's too.)
                    let place = data::catalog_place(v);
                    if place == data::Place::None {
                        continue; // listed, never made
                    }
                    let label = if place == data::Place::Blinded && !by_name(&self.space, &t) { identity::blind_name(&identity::space_table_key(&self.space, &t), &t) } else { t.clone() };
                    io.extend(self.ask(c, j.owner, &t, label, Purpose::Feed { cap }));
                }
                io
            }
            Purpose::Feed { .. } => {
                // `departed` read from EVERY writer's feed of it (each remover writes theirs), then its writers read.
                let name = table_name(&self.space, "departed");
                let all_in = self.jobs.iter().filter(|x| x.table == name).all(|x| x.rows.is_some())
                    && !self.waiting.iter().any(|(_, w, _)| self.jobs[*w].table == name)
                    && self.jobs.iter().filter(|x| matches!(x.purpose, Purpose::Catalog { capped: None })).all(|x| x.rows.is_some());
                if j.table == name && all_in && !self.departed_asked {
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
        // WHO WROTE — the one map (`craftworks_account::writers`, the page's too): the current devices as resolved, and
        // each DEPARTED writer → its DID, capped where it stood. Its feeds read up to there, its rows its author's.
        let mut by_did: BTreeMap<String, Vec<[u8; 32]>> = BTreeMap::new();
        for (n, d) in &self.writer_dids {
            by_did.entry(d.clone()).or_default().push(*n);
        }
        let members: Vec<(String, Vec<[u8; 32]>)> = by_did.into_iter().collect();
        let rows: Vec<(String, String)> = merged.into_iter().filter_map(|(k, row)| Some((String::from_utf8(k).ok()?, String::from_utf8(row.value).ok()?))).collect();
        let mut io = Vec::new();
        for w in craftworks_account::writers(&members, &rows) {
            let Some(caps) = w.caps else { continue };
            if !self.writers.contains(&w.node) {
                self.writer_dids.push((w.node, w.did.clone()));
            }
            let cat = table_name(&self.space, "tables");
            io.extend(self.ask(c, w.node, &cat, cat.clone(), Purpose::Catalog { capped: Some(caps) }));
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
        // A block of a member's CARD: once all are in, the card read again with them.
        if let Some(k) = self.card_trees.iter().position(|t| t.3.iter().any(|(w, _)| *w == id)) {
            let t = &mut self.card_trees[k];
            let at = t.3.iter().position(|(w, _)| *w == id).expect("found");
            let (_, cid) = t.3.remove(at);
            if let Some(st) = state {
                t.2.push((cid, st));
            }
            if !t.3.is_empty() {
                return Vec::new();
            }
            let (did, card) = (t.0, t.1.clone());
            let Some(at) = self.resolving.iter().position(|(d, l, _, cr)| *d == did && l.is_some() && cr.is_none()) else { return Vec::new() };
            return self.resolved(c, at, Some(card));
        }
        if let Some(at) = self.resolving.iter().position(|(_, _, w, _)| *w == id) {
            return self.resolved(c, at, state);
        }
        if self.bag == Some(id) && !self.bag_read {
            self.bag_read = true;
            // The bag: whom it lists, whether any entry did not open here (BLIND), and the OPEN list's public tables.
            let (mut listed, mut blind, mut named) = (BTreeSet::new(), false, Vec::<([u8; 32], String)>::new());
            if let Some(st) = &state {
                let address = sealed_bag_address(&self.space, "writers");
                for p in craftworks_bag_contract::read(&address, st).unwrap_or_default() {
                    let Some(v) = open_bag_item(&self.space, &self.epochs, "writers", craftworks_bag_contract::payload(&p)) else {
                        blind = true;
                        continue;
                    };
                    let Some(w) = v["w"].as_str().and_then(unhex32) else { continue };
                    listed.insert(w);
                    named.extend(v["pub"].as_str().map(|t| (w, t.to_string())));
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
            // THE WRITERS (`account::table_writers`, the page's too): their catalogs; a public table also by the open list.
            let members = craftworks_account::table_writers(&self.writers, &listed, blind, &[], false, &[0; 32]);
            for w in &members {
                io.extend(self.ask(c, *w, &cat, cat.clone(), Purpose::Catalog { capped: None }));
            }
            let space = self.space;
            for t in self.tables.clone().into_iter().filter(|t| is_public(&space, t)) {
                let by_list: Vec<[u8; 32]> = named.iter().filter(|(_, n)| *n == t).map(|(w, _)| *w).collect();
                for w in craftworks_account::table_writers(&[], &listed, false, &by_list, true, &[0; 32]).into_iter().filter(|w| !members.contains(w)) {
                    io.extend(self.ask(c, w, &t, t.clone(), Purpose::Feed { cap: None }));
                }
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

    /// A writer's feed of a table that was asked and did NOT read (sealed with a key not held, broken).
    pub fn own_unreadable(&self, owner: &[u8; 32], name: &str) -> bool {
        let t = table_name(&self.space, name);
        self.jobs.iter().any(|j| j.owner == *owner && j.table == t && j.state.is_some() && matches!(j.rows, Some(None)))
    }

    /// The members' devices found (writer key, the DID's text).
    pub fn writer_dids(&self) -> &[([u8; 32], String)] {
        &self.writer_dids
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
            Some(cap) => (*w, rows.iter().filter(|(_, v)| craftworks_feed::within_cap(v, *cap)).map(|(k, v)| (k.clone(), v.clone())).collect()),
            None => (*w, rows.clone()),
        })
        .collect();
    let refs: Vec<([u8; 32], &BTreeMap<Vec<u8>, Vec<u8>>)> = capped.iter().map(|(w, r)| (*w, r)).collect();
    craftworks_feed::merge(&refs)
}
