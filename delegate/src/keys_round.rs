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

use craftworks_groups::{Account, Check, Group, Merged};
use craftworks_identity::{self as identity, Host, Mandate};
use sha2::Digest;

use super::{followed, governance, hex, keep, next_space, whole, Io, Round, Step};
use crate::rekey_round::Held;
use crate::write::{self, Next};

/// The largest operation a row carries (hex): a larger one is a page's to write as a file (the row names it).
const ROW: usize = 60000;
/// No act names the owner's own placement (its tree's first operation).
const NO_ACT: [u8; 32] = [0; 32];

/// OUR CHECK, the list's — the page's `members.check()`, the same rule.
struct List {
    by_member: BTreeMap<[u8; 32], String>,
    listed: Vec<String>,
    owner: String,
    /// sha256(act id) → (in?, the account).
    acts: BTreeMap<[u8; 32], (bool, String)>,
}
impl Check for List {
    fn account_of(&self, member: &[u8; 32]) -> Option<String> {
        self.by_member.get(member).cloned()
    }
    fn listed(&self, did: &str) -> bool {
        self.listed.iter().any(|d| d == did) || did == self.owner
    }
    fn allows(&self, act: &[u8; 32], add: bool, target: &[u8; 32]) -> bool {
        let Some(t) = self.by_member.get(target) else { return false };
        if *act == NO_ACT {
            return add && *t == self.owner;
        }
        self.acts.get(act).is_some_and(|(i, d)| *i == add && d == t)
    }
}

fn unhex(s: &[u8]) -> Option<Vec<u8>> {
    let s = std::str::from_utf8(s).ok()?;
    (0..s.len() / 2).map(|i| u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()).collect()
}

/// THE ROUND's BEEKEM STEP for this space (the follower's, after its read).
pub(super) fn reconcile<H: Host>(h: &mut H, c: &Held, r: Round, m: &Mandate, now_ms: u64) -> Vec<Io> {
    let Some(gov) = governance(h, &r, now_ms) else { return next_space(h, c, r) };
    let Some(seed) = identity::upkeep_space_member(h, &r.member) else {
        followed(h, &r.member, &r.space, "keys-no-member", &format!("{}: the account's data key is not held here: its keys are a page's", m.name));
        return next_space(h, c, r);
    };
    let mut g = Group::empty(&r.space, Account::of(&seed));
    let list = List {
        by_member: r.reading.beekem.iter().map(|(d, id, _)| (*id, d.clone())).collect(),
        listed: gov.roster.iter().cloned().collect(),
        owner: m.owner.clone(),
        acts: gov
            .counted
            .iter()
            .filter_map(|a| Some((sha2::Sha256::digest(a.get("id")?.as_str()?.as_bytes()).into(), craftworks_gov::entry_move(a)?)))
            .collect(),
    };
    // 1–2. THE OPERATIONS merged through our check, in any order (one ahead of its predecessors offered again).
    let mut pending: Vec<Vec<u8>> = r.reading.rows("pub-keyops").values().filter_map(|row| unhex(&row.value)).collect();
    let mut refused = 0;
    let mut why: Vec<String> = Vec::new();
    for _ in 0..64 {
        let before = pending.len();
        pending.retain(|op| match g.merge(op, &list) {
            Merged::Applied => false,
            Merged::Waiting => true,
            Merged::Refused(w) => {
                refused += 1;
                if why.len() < 2 {
                    why.push(w.chars().take(80).collect());
                }
                false
            }
        });
        if pending.is_empty() || pending.len() == before {
            break;
        }
    }
    // 3. THE GENERATIONS: the space's epochs on this node.
    for (n, k) in g.generations() {
        identity::keep_epoch(h, &r.member, Some(r.space), n, &k);
    }
    let placed = g.members();
    // 4. RECONCILE — from a whole read only.
    if let Err(why) = whole(&r, &gov) {
        followed(h, &r.member, &r.space, "keys-short", &format!("{}: {why} — the keys are not changed this round", m.name));
        return next_space(h, c, r);
    }
    if !g.placed() {
        followed(h, &r.member, &r.space, "keys-not-placed", &format!("{}: this account is not in the tree yet ({} waiting, {refused} refused: {}; {} card(s) name a member, {} operation row(s){})", m.name, pending.len(), why.join(" / "), r.reading.beekem.len(), r.reading.rows("pub-keyops").len(), if r.reading.card_says.is_empty() { String::new() } else { format!("; cards: {}", r.reading.card_says.join(", ")) }));
        return next_space(h, c, r);
    }
    let latest = |inn: bool, did: &str| gov.counted.iter().rev().find(|a| craftworks_gov::entry_move(a).is_some_and(|(i, d)| i == inn && d == did)).and_then(|a| a.get("id")?.as_str().map(|id| <[u8; 32]>::from(sha2::Sha256::digest(id.as_bytes()))));
    let mut adds = Vec::new();
    for (did, id, leaf) in &r.reading.beekem {
        if placed.contains(id) || !list.listed(did) {
            continue;
        }
        let act = if *did == list.owner { latest(true, did).unwrap_or(NO_ACT) } else if let Some(a) = latest(true, did) { a } else { continue };
        adds.push((did.clone(), *id, *leaf, act));
    }
    let mut removes = Vec::new();
    for id in &placed {
        let Some(did) = list.by_member.get(id) else { continue };
        if list.listed(did) {
            continue;
        }
        if let Some(act) = latest(false, did) {
            removes.push((did.clone(), *id, act));
        }
    }
    if adds.is_empty() && removes.is_empty() {
        followed(h, &r.member, &r.space, "keys", &format!("{}: the tree places the list ({} placed, {} waiting, {refused} refused)", m.name, placed.len(), pending.len()));
        return next_space(h, c, r);
    }
    // (A delegate has no randomness of its own: a key change draws from upkeep's pool.)
    let Some(rand) = identity::upkeep_random(h) else {
        followed(h, &r.member, &r.space, "keys-wait", &format!("{}: {} to add, {} to remove — no randomness yet (a page stirs the pool)", m.name, adds.len(), removes.len()));
        return next_space(h, c, r);
    };
    let mut ops = Vec::new();
    for (did, id, leaf, act) in &adds {
        match g.add_member_raw(id, leaf, act) {
            Ok(Some(op)) => ops.push(op),
            Ok(None) => {}
            Err(why) => followed(h, &r.member, &r.space, "keys-add", &format!("{}: {}… not added — {why}", m.name, did.get(12..20).unwrap_or(did))),
        }
    }
    for (did, id, act) in &removes {
        match g.remove_member(id, act) {
            Ok(Some(op)) => ops.push(op),
            Ok(None) => {}
            Err(why) => followed(h, &r.member, &r.space, "keys-remove", &format!("{}: {}… not removed — {why}", m.name, did.get(12..20).unwrap_or(did))),
        }
    }
    match g.rotate_seeded(rand) {
        Ok(op) => ops.push(op),
        Err(why) => followed(h, &r.member, &r.space, "keys-rotate", &format!("{}: no key change — {why}", m.name)),
    }
    for (n, k) in g.generations() {
        identity::keep_epoch(h, &r.member, Some(r.space), n, &k);
    }
    let rows: Vec<(Vec<u8>, Option<Vec<u8>>)> = ops
        .iter()
        .filter(|op| 2 * op.len() <= ROW)
        .map(|op| (hex(&sha2::Sha256::digest(op)).into_bytes(), Some(hex(op).into_bytes())))
        .collect();
    let large = ops.len() - rows.len();
    let names = |v: Vec<&String>| v.iter().map(|d| d.get(12..20).unwrap_or(d).to_string()).collect::<Vec<_>>().join(", ");
    followed(
        h,
        &r.member,
        &r.space,
        "keys-plan",
        &format!("{}: {} added ({}), {} removed ({}), one key change{}", m.name, adds.len(), names(adds.iter().map(|a| &a.0).collect()), removes.len(), names(removes.iter().map(|a| &a.0).collect()), if large > 0 { format!(" — {large} operation(s) too large for a row: a page's") } else { String::new() }),
    );
    write(h, c, r, m, rows, false)
}

/// ONE STEP of writing the operations into this member's `pub-keyops` (listing first when the feed is new, a flush when
/// due — then the rows again once answered).
pub(super) fn write<H: Host>(h: &mut H, c: &Held, mut r: Round, m: &Mandate, rows: Vec<(Vec<u8>, Option<Vec<u8>>)>, blocks_put: bool) -> Vec<Io> {
    let nonce: [u8; 12] = identity::upkeep_random(h).map(|b| b[..12].try_into().expect("12")).unwrap_or([0; 12]);
    match write::rows(h, &r.member, &c.codes(), &mut r.reading, "pub-keyops", &rows, nonce, blocks_put, crate::upkeep::creates()) {
        Ok(Next::Done(sent)) => {
            r.step = Step::Keys { rows: Vec::new(), waiting: sent.confirm, blocks: false };
            keep(h, Some(&r));
            sent.io
        }
        Ok(Next::More(sent)) => {
            r.step = Step::Keys { rows, waiting: sent.confirm, blocks: sent.blocks };
            keep(h, Some(&r));
            sent.io
        }
        Err(why) => {
            followed(h, &r.member, &r.space, "keys-write", &format!("{}: the key operations were not written — {why}", m.name));
            next_space(h, c, r)
        }
    }
}
