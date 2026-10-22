//! K5 — A BEEKEM SPACE KEPT TO ITS LIST BY THE NODE, every page closed (docs/proposals/members-and-keys-beekem.md, K5).
//! The follower round read the space's acts AND its `pub-keyops` (every member's feeds); here, for a space with a tree:
//!
//!   1. THE TREE, rebuilt from the operations (nothing kept: every leaf secret of the account derives — K6).
//!   2. OUR CHECK from the replay (`gov`) and the members' cards (`Reading::beekem`): the page's `members` check.
//!   3. THE GENERATIONS reached kept as the space's epochs (`keep_epoch`): the node reads the space's new rows.
//!   4. RECONCILE, from a WHOLE read only (I6): add each listed account its card names and the tree does not place (by
//!      the latest act letting it in), remove each placed member whose account the list no longer names (by the act
//!      putting it out); then ONE key change. No turns: several nodes doing it at once merge.
//!   5. WRITTEN into this member's own `pub-keyops` (`write::rows`).

use std::collections::BTreeMap;

use craftworks_groups::{Account, Check, Group, ListCheck};
use craftworks_identity::{self as identity, Host, Mandate};
use sha2::Digest;

use super::{followed, governance, hex, keep, next_space, whole, Io, Round, Step};
use crate::rekey_round::Held;
use crate::file_io::{FileRead, FileWrite, Read, Wrote};
use crate::write::{self, Next};
use craftworks_identity::unhex32;

/// The most cards a round reads that it has not learned yet (the rest the next round): a round stays under the
/// node's per-round cap on network operations.
pub(super) const CARDS_A_ROUND: usize = 8;
/// What this node LEARNED of a space's members (`member ‖ space` → `[(DID, member id, leaf, ms read)]`): read again by
/// `members::card_due` (the page's too). (`KNOWN_BEFORE`: kept before with no time — taken as read long ago, so read
/// again, and used until then.)
const KNOWN: &[u8] = b"identity_upkeep/beekem-known2/";
const KNOWN_BEFORE: &[u8] = b"identity_upkeep/beekem-known/";

/// When this node last ASKED for an account's card (`member ‖ space` → `[(DID, ms)]`): one with no card (an account
/// gone, a key log never made) is asked again by `members::card_due` — never every round.
const ASKED: &[u8] = b"identity_upkeep/beekem-asked/";
pub(super) fn asked<H: Host>(h: &H, member: &[u8; 32], space: &[u8; 32]) -> Vec<(String, u64)> {
    h.get_secret(&[ASKED, &member[..], &space[..]].concat()).and_then(|b| bincode::deserialize(&b).ok()).unwrap_or_default()
}
pub(super) fn asking<H: Host>(h: &mut H, member: &[u8; 32], space: &[u8; 32], dids: &[String], now_ms: u64) {
    let mut all: Vec<(String, u64)> = asked(h, member, space).into_iter().filter(|(d, t)| !dids.contains(d) && !craftworks_rules::members::card_due(Some((*t, false)), now_ms)).collect();
    all.extend(dids.iter().map(|d| (d.clone(), now_ms)));
    h.set_secret(&[ASKED, &member[..], &space[..]].concat(), &bincode::serialize(&all).expect("a list encodes"));
}

/// A member learned: its DID, member id, leaf, and when its card was read.
pub(crate) type Known = (String, [u8; 32], [u8; 32], u64);
pub(crate) fn known<H: Host>(h: &H, member: &[u8; 32], space: &[u8; 32]) -> Vec<Known> {
    if let Some(k) = h.get_secret(&[KNOWN, &member[..], &space[..]].concat()).and_then(|b| bincode::deserialize(&b).ok()) {
        return k;
    }
    let before: Vec<(String, [u8; 32], [u8; 32])> = h.get_secret(&[KNOWN_BEFORE, &member[..], &space[..]].concat()).and_then(|b| bincode::deserialize(&b).ok()).unwrap_or_default();
    before.into_iter().map(|(d, id, leaf)| (d, id, leaf, 0)).collect()
}
/// Whether an account's card is read this round (`members::card_due`, the page's too): by when it was last read, or
/// last asked for and none was there.
pub(super) fn card_due(known: &[Known], asked: &[(String, u64)], did: &str, now_ms: u64) -> bool {
    let last = known.iter().find(|k| k.0 == did).map(|k| (k.3, true)).or_else(|| asked.iter().find(|(d, _)| d == did).map(|(_, t)| (*t, false)));
    craftworks_rules::members::card_due(last, now_ms)
}
fn learn<H: Host>(h: &mut H, member: &[u8; 32], space: &[u8; 32], read: &[(String, [u8; 32], [u8; 32])], now_ms: u64) -> Vec<(String, [u8; 32], [u8; 32])> {
    let mut all = known(h, member, space);
    for (d, id, leaf) in read {
        let m = (d.clone(), *id, *leaf, now_ms);
        if let Some(at) = all.iter().position(|k| k.0 == *d) {
            all[at] = m;
        } else {
            all.push(m);
        }
    }
    h.set_secret(&[KNOWN, &member[..], &space[..]].concat(), &bincode::serialize(&all).expect("a list encodes"));
    all.into_iter().map(|(d, id, leaf, _)| (d, id, leaf)).collect()
}

/// No act names the owner's own placement (its tree's first operation).


/// THE ROUND's BEEKEM STEP for this space (the follower's, after its read).
pub(super) fn reconcile<H: Host>(h: &mut H, c: &Held, r: Round, m: &Mandate, now_ms: u64) -> Vec<Io> {
    let Some(gov) = governance(h, &r, now_ms) else { return next_space(h, c, r) };
    let Some(seed) = identity::upkeep_space_member(h, &r.member) else {
        followed(h, &r.member, &r.space, craftworks_rules::steps::KEYS_NO_MEMBER, &format!("{}: the account's data key is not held here: its keys are a page's", m.name));
        return next_space(h, c, r);
    };
    let mut g = Group::empty(&r.space, Account::of(&seed));
    // Every member LEARNED, this read's added (a card read now replaces what was kept).
    let members = learn(h, &r.member, &r.space, &r.reading.beekem, now_ms);
    // OUR CHECK (`groups::ListCheck`, the page's too).
    let list = ListCheck {
        by_member: members.iter().map(|(d, id, _)| (*id, d.clone())).collect(),
        listed: gov.roster.iter().cloned().collect(),
        owner: m.owner.clone(),
        acts: gov
            .counted
            .iter()
            .filter_map(|a| Some((sha2::Sha256::digest(a.get("id")?.as_str()?.as_bytes()).into(), craftworks_gov::entry_move(a)?)))
            .collect(),
        ..ListCheck::default()
    };
    // 1–2. THE OPERATIONS merged through our check, in any order (one ahead of its predecessors offered again).
    // (`members::keyop_row`, the page's too: one in its row, or a large one stored as a FILE — read once by
    // `file_io::FileRead`, then kept here by its key.)
    use craftworks_rules::members::{keyop_row, KeyopRow};
    let mut ops: Vec<Vec<u8>> = Vec::new();
    let mut to_read: Vec<(String, String)> = Vec::new();
    for (k, row) in r.reading.rows("pub-keyops") {
        let (Ok(k), Ok(v)) = (std::str::from_utf8(&k), std::str::from_utf8(&row.value)) else { continue };
        match keyop_row(v) {
            Some(KeyopRow::Op(o)) => ops.push(o),
            Some(KeyopRow::File(f)) => match h.get_secret(&file_op_key(k)).filter(|o| !o.is_empty()) {
                Some(o) => ops.push(o),
                None if !r.files_unread.iter().any(|x| x == k) => to_read.push((k.to_string(), f.to_string())),
                None => {}
            },
            None => {}
        }
    }
    if !to_read.is_empty() {
        return read_file(h, c, r, m, to_read);
    }
    // NOTHING FROM A SHORT READ (R4, the page's too): judged against a list read short, the adds of every account it
    // missed are refused (10-20, the owner's page). The tree stays as last merged from a whole read.
    if let Err(why) = whole(&r, &gov) {
        followed(h, &r.member, &r.space, craftworks_rules::steps::KEYS_SHORT, &format!("{}: {why} — the keys are not changed this round", m.name));
        return next_space(h, c, r);
    }
    // (`Group::merge_all`: the one loop, the page's too.)
    let merged = g.merge_all(&ops, &list);
    let (refused, waiting) = (merged.refused.len(), merged.waiting);
    let why: Vec<String> = merged.refused.iter().take(2).map(|w| w.chars().take(80).collect()).collect();
    // 3. THE GENERATIONS: the space's epochs on this node.
    for (n, k) in g.generations() {
        identity::keep_epoch(h, &r.member, Some(r.space), n, &k);
    }
    let placed = g.members();
    // 4. RECONCILE (the list read whole: checked before the merge).
    if !g.placed() {
        followed(h, &r.member, &r.space, craftworks_rules::steps::KEYS_NOT_PLACED, &format!("{}: this account is not in the tree yet ({} waiting, {refused} refused: {}; {} card(s) name a member, {} operation row(s){})", m.name, waiting, why.join(" / "), members.len(), r.reading.rows("pub-keyops").len(), if r.reading.card_says.is_empty() { String::new() } else { format!("; cards: {}", r.reading.card_says.join(", ")) }));
        return next_space(h, c, r);
    }
    // THE ONE PLAN (`Group::reconcile`, the page's too): the listed placed, the put-out taken out by their act.
    // (Each account's LATEST act letting it in / putting it out: what an add or a removal names.)
    let acts = |inn: bool| -> BTreeMap<String, [u8; 32]> {
        gov.counted
            .iter()
            .filter_map(|a| match craftworks_gov::entry_move(a)? {
                (i, did) if i == inn => Some((did, <[u8; 32]>::from(sha2::Sha256::digest(a.get("id")?.as_str()?.as_bytes())))),
                _ => None,
            })
            .collect()
    };
    let (ins, outs) = (acts(true), acts(false));
    // (`Group::due`: reconcile's own conditions, the page's too.)
    if !g.due(&members, &|d| list.listed(d), &outs) {
        followed(h, &r.member, &r.space, craftworks_rules::steps::KEYS_IN_STEP, &format!("{}: the tree places the list ({} placed, {} waiting, {refused} refused)", m.name, placed.len(), waiting));
        return next_space(h, c, r);
    }
    // (A delegate has no randomness of its own: a key change draws from upkeep's pool.)
    let Some(rand) = identity::upkeep_random(h) else {
        followed(h, &r.member, &r.space, craftworks_rules::steps::KEYS_WAIT, &format!("{}: the tree is not the list — no randomness yet (a page stirs the pool)", m.name));
        return next_space(h, c, r);
    };
    let done = g.reconcile(&members, &|d| list.listed(d), &ins, &outs, rand);
    let ops = done.ops;
    // AN OPERATION TOO LARGE FOR A ROW is stored as a file — new contracts, which reach the network only from a page
    // (`upkeep::creates`, as a re-key's new pieces): then the change is a page's, and nothing of it is kept here (a
    // generation nobody else could ever read).
    if !crate::upkeep::creates() && ops.iter().any(|op| !inline(op)) {
        followed(h, &r.member, &r.space, craftworks_rules::steps::KEYS_WAIT, &format!("{}: the key change has an operation too large for a row: a page stores it (a contract made here stays on this node)", m.name));
        return next_space(h, c, r);
    }
    for (n, k) in g.generations() {
        identity::keep_epoch(h, &r.member, Some(r.space), n, &k);
    }
    // Each operation in its row; a large one stored as a FILE first, its row naming it (`members::keyop_*`, the page's
    // too).
    let mut rows: Vec<(Vec<u8>, Option<Vec<u8>>)> = Vec::new();
    let mut large: Vec<String> = Vec::new();
    for op in &ops {
        let key = craftworks_rules::members::keyop_key(op);
        if inline(op) {
            rows.push((key.into_bytes(), Some(hex(op).into_bytes())));
        } else {
            h.set_secret(&out_op_key(&key), op);
            large.push(key);
        }
    }
    let names = |v: &[String]| v.iter().map(|d| d.get(12..20).unwrap_or(d).to_string()).collect::<Vec<_>>().join(", ");
    followed(
        h,
        &r.member,
        &r.space,
        craftworks_rules::steps::KEYS_RECONCILED,
        &format!("{}: {} added ({}), {} removed ({}), one key change{}{}", m.name, done.added.len(), names(&done.added), done.removed.len(), names(&done.removed), if large.is_empty() { String::new() } else { format!(" — {} stored as files", large.len()) }, if done.not.is_empty() { format!(" — {}", done.not.join("; ")) } else { String::new() }),
    );
    if !large.is_empty() {
        return write_file(h, c, r, m, large, rows);
    }
    write(h, c, r, m, "pub-keyops", rows, false)
}

/// Whether an operation goes in its row (`members::keyop_inline`, the page's too) — a test may send every one to a
/// file (a large operation needs a large group).
fn inline(op: &[u8]) -> bool {
    #[cfg(test)]
    if ALL_AS_FILES.with(|f| f.get()) {
        return false;
    }
    craftworks_rules::members::keyop_inline(op)
}
#[cfg(test)]
thread_local! {
    pub(crate) static ALL_AS_FILES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A key operation read from its file, kept here by its key (its bytes never change: content-addressed).
fn file_op_key(key: &str) -> Vec<u8> {
    [b"identity_upkeep/keyop-file/".as_slice(), key.as_bytes()].concat()
}
/// A large operation this round made, held until its file is stored.
fn out_op_key(key: &str) -> Vec<u8> {
    [b"identity_upkeep/keyop-out/".as_slice(), key.as_bytes()].concat()
}
/// A part of the file being read.
fn part_key(g: u64) -> Vec<u8> {
    [b"identity_upkeep/keyop-read/part/".as_slice(), &g.to_be_bytes()].concat()
}
/// Where `file_io` keeps a key operation's read and write between calls.
const READ_SLOT: &[u8] = b"keyop-read/";
const WRITE_SLOT: &[u8] = b"keyop-write/";

/// The first of `files` READ (`file_io::FileRead`, from the reference its row names).
fn read_file<H: Host>(h: &mut H, c: &Held, mut r: Round, m: &Mandate, mut files: Vec<(String, String)>) -> Vec<Io> {
    while !files.is_empty() {
        let (key, f) = files.remove(0);
        let refd: serde_json::Value = serde_json::from_str(&f).unwrap_or_default();
        let (Some(fk), Some(root)) = (refd["key"].as_str().and_then(unhex32), refd["root"].as_str().and_then(unhex32)) else {
            r.files_unread.push(key);
            continue;
        };
        let (read, io) = FileRead::start(READ_SLOT, &c.piece, fk, root);
        r.step = Step::KeyFileRead { key, read, parts: 0, rest: files };
        keep(h, Some(&r));
        return io;
    }
    // Every file read (or not this round): the operations merged.
    let now = identity::upkeep_now_ms(h);
    reconcile(h, c, r, m, now)
}

/// An answer to the file read: its parts kept; whole, checked against its key (`members::keyop_file_ok`, the
/// page's too) and kept by it; then the next file.
pub(super) fn file_got<H: Host>(h: &mut H, c: &Held, mut r: Round, m: &Mandate, id: [u8; 32], state: Option<Vec<u8>>, now_ms: u64) -> Vec<Io> {
    let Step::KeyFileRead { key, mut read, mut parts, rest } = r.step.clone() else { return Vec::new() };
    if !read.wants(&id) {
        return Vec::new();
    }
    let failed = |h: &mut H, mut r: Round, why: String| {
        followed(h, &r.member, &r.space, craftworks_rules::steps::KEYS_OPS_UNREAD, &format!("{}: a key operation's file did not read — {why}", m.name));
        r.files_unread.push(key.clone());
        read_file(h, c, r, m, rest.clone())
    };
    let io = match read.got(h, &c.piece, id, state) {
        Err(why) => return failed(h, r, why),
        Ok(Read::Io(io)) => io,
        Ok(Read::Part(g, plain)) => {
            h.set_secret(&part_key(g), &plain);
            parts = g + 1;
            match read.next(h, &c.piece) {
                Err(why) => return failed(h, r, why),
                Ok(io) => io,
            }
        }
    };
    if read.done() {
        let op: Vec<u8> = (0..parts).flat_map(|g| h.get_secret(&part_key(g)).unwrap_or_default()).collect();
        for g in 0..parts {
            h.set_secret(&part_key(g), &[]);
        }
        if !craftworks_rules::members::keyop_file_ok(&key, &op) {
            return failed(h, r, "its bytes are not the operation its row names".into());
        }
        h.set_secret(&file_op_key(&key), &op);
        let _ = now_ms;
        return read_file(h, c, r, m, rest);
    }
    r.step = Step::KeyFileRead { key, read, parts, rest };
    keep(h, Some(&r));
    io
}

/// The first of the `large` operations STORED as a public file of the space (`file_io::FileWrite`, as a page's put:
/// keyed by its content alone).
fn write_file<H: Host>(h: &mut H, c: &Held, mut r: Round, m: &Mandate, mut large: Vec<String>, rows: Vec<(Vec<u8>, Option<Vec<u8>>)>) -> Vec<Io> {
    let Some(key) = (!large.is_empty()).then(|| large.remove(0)) else {
        return write(h, c, r, m, "pub-keyops", rows, false);
    };
    let op = h.get_secret(&out_op_key(&key)).unwrap_or_default();
    let fk = craftworks_files::content_key(blake3::hash(&op).as_bytes(), None);
    let mut w = FileWrite::new(WRITE_SLOT, fk, [0; 32], op.len() as u64);
    let io = w.part(h, &c.piece, 0, part_of(&op, 0));
    r.step = Step::KeyFileWrite { key, write: w, left: large, rows };
    keep(h, Some(&r));
    io
}
/// Part `g` of a file's bytes (its plan's).
fn part_of(b: &[u8], g: u64) -> &[u8] {
    let plan = craftworks_files::Plan::of(b.len() as u64);
    let per = plan.chunk * craftworks_files::GEN;
    let from = (g as usize * per).min(b.len());
    &b[from..(from + per).min(b.len())]
}

/// An answer to the file write: the next part, the index; stored, its row (`{ file: <reference> }`) and its listing
/// (`rules::rekey::file_ref` / `key_row_write`, the page's too) — then the next file.
pub(super) fn file_put<H: Host>(h: &mut H, c: &Held, mut r: Round, m: &Mandate, id: [u8; 32], ok: bool, now_ms: u64) -> Vec<Io> {
    let Step::KeyFileWrite { key, write: mut w, left, mut rows } = r.step.clone() else { return Vec::new() };
    if !w.wants(&id) {
        return Vec::new();
    }
    let op = h.get_secret(&out_op_key(&key)).unwrap_or_default();
    let step = match w.put(h, &c.piece, id, ok) {
        Ok(s) => s,
        Err(why) => {
            followed(h, &r.member, &r.space, craftworks_rules::steps::KEYS_WRITE, &format!("{}: a key operation's file was not stored — {why}", m.name));
            return next_space(h, c, r);
        }
    };
    let io = match step {
        Wrote::Io(io) => io,
        Wrote::Part(g) if (g + 1) < craftworks_files::Plan::of(w.size()).gens => w.part(h, &c.piece, g + 1, part_of(&op, g + 1)),
        Wrote::Part(_) => match w.index(&c.piece) {
            Ok(io) => io,
            Err(why) => {
                followed(h, &r.member, &r.space, craftworks_rules::steps::KEYS_WRITE, &format!("{}: a key operation's file was not stored — {why}", m.name));
                return next_space(h, c, r);
            }
        },
        Wrote::Done(root) => {
            let id = hex(root);
            let space = hex(r.space);
            let refd = craftworks_rules::rekey::file_ref(&id, Some(&space), &w.key(), &root, &[0; 32], w.size(), "file", "application/octet-stream");
            rows.push((key.clone().into_bytes(), Some(serde_json::json!({ "file": refd }).to_string().into_bytes())));
            let fields = serde_json::json!({ "key": hex(w.key()), "root": id, "b": hex([0u8; 32]), "h": hex(blake3::hash(&op).as_bytes()), "pub": true, "app": "members", "n": -1 });
            let (k, v) = craftworks_rules::rekey::key_row_write(&id, fields.as_object().expect("an object"), now_ms);
            r.listing.push((k.into_bytes(), Some(v.into_bytes())));
            h.set_secret(&out_op_key(&key), &[]);
            return write_file(h, c, r, m, left, rows);
        }
    };
    r.step = Step::KeyFileWrite { key, write: w, left, rows };
    keep(h, Some(&r));
    io
}

/// The space's `files` table read, to list the files written in it.
pub(super) fn list_files<H: Host>(h: &mut H, c: &Held, mut r: Round, m: &Mandate) -> Vec<Io> {
    let Some((epochs, dids)) = crate::upkeep::readers(h, &r.member, m) else { return next_space(h, c, r) };
    let (reading, io) = crate::table::Reading::of_members(&c.codes(), r.space, epochs, &dids, &["files"]);
    r.reading = reading;
    r.step = Step::ListFiles;
    keep(h, Some(&r));
    io
}

/// ONE STEP of writing `rows` into this member's feed of `table` (listing first when the feed is new, a flush when
/// due — then the rows again once answered).
pub(super) fn write<H: Host>(h: &mut H, c: &Held, mut r: Round, m: &Mandate, table: &str, rows: Vec<(Vec<u8>, Option<Vec<u8>>)>, blocks_put: bool) -> Vec<Io> {
    let nonce: [u8; 12] = identity::upkeep_random(h).map(|b| b[..12].try_into().expect("12")).unwrap_or([0; 12]);
    match write::rows(h, &r.member, &c.codes(), &mut r.reading, table, &rows, nonce, blocks_put, crate::upkeep::creates()) {
        Ok(Next::Done(sent)) => {
            r.step = Step::Keys { table: table.into(), rows: Vec::new(), waiting: sent.confirm, blocks: false };
            keep(h, Some(&r));
            sent.io
        }
        Ok(Next::More(sent)) => {
            r.step = Step::Keys { table: table.into(), rows, waiting: sent.confirm, blocks: sent.blocks };
            keep(h, Some(&r));
            sent.io
        }
        Err(why) => {
            followed(h, &r.member, &r.space, craftworks_rules::steps::KEYS_WRITE, &format!("{}: the key operations were not written — {why}", m.name));
            next_space(h, c, r)
        }
    }
}
