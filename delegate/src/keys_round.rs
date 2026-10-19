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
use crate::write::{self, Next};

/// The most cards a round reads that it has not learned yet (the rest the next round): a round stays under the
/// node's per-round cap on network operations.
pub(super) const CARDS_A_ROUND: usize = 8;
/// What this node LEARNED of a space's members (`member ‖ space` → `[(DID, member id, leaf)]`): never read again.
const KNOWN: &[u8] = b"identity_upkeep/beekem-known/";

/// When this node last ASKED for an account's card (`member ‖ space` → `[(DID, ms)]`): one with no card (an account
/// gone, a key log never made) is asked again only after `ASK_AGAIN_MS` — never every round.
const ASKED: &[u8] = b"identity_upkeep/beekem-asked/";
pub(super) const ASK_AGAIN_MS: u64 = 3_600_000;
pub(super) fn asked<H: Host>(h: &H, member: &[u8; 32], space: &[u8; 32]) -> Vec<(String, u64)> {
    h.get_secret(&[ASKED, &member[..], &space[..]].concat()).and_then(|b| bincode::deserialize(&b).ok()).unwrap_or_default()
}
pub(super) fn asking<H: Host>(h: &mut H, member: &[u8; 32], space: &[u8; 32], dids: &[String], now_ms: u64) {
    let mut all: Vec<(String, u64)> = asked(h, member, space).into_iter().filter(|(d, t)| !dids.contains(d) && now_ms.saturating_sub(*t) < ASK_AGAIN_MS).collect();
    all.extend(dids.iter().map(|d| (d.clone(), now_ms)));
    h.set_secret(&[ASKED, &member[..], &space[..]].concat(), &bincode::serialize(&all).expect("a list encodes"));
}

pub(super) fn known<H: Host>(h: &H, member: &[u8; 32], space: &[u8; 32]) -> Vec<(String, [u8; 32], [u8; 32])> {
    h.get_secret(&[KNOWN, &member[..], &space[..]].concat()).and_then(|b| bincode::deserialize(&b).ok()).unwrap_or_default()
}
fn learn<H: Host>(h: &mut H, member: &[u8; 32], space: &[u8; 32], read: &[(String, [u8; 32], [u8; 32])]) -> Vec<(String, [u8; 32], [u8; 32])> {
    let mut all = known(h, member, space);
    for m in read {
        if let Some(at) = all.iter().position(|(d, ..)| *d == m.0) {
            all[at] = m.clone();
        } else {
            all.push(m.clone());
        }
    }
    h.set_secret(&[KNOWN, &member[..], &space[..]].concat(), &bincode::serialize(&all).expect("a list encodes"));
    all
}

/// The largest operation a row carries (hex): a larger one is a page's to write as a file (the row names it).
const ROW: usize = 60000;
/// No act names the owner's own placement (its tree's first operation).

fn unhex(s: &[u8]) -> Option<Vec<u8>> {
    let s = std::str::from_utf8(s).ok()?;
    (0..s.len() / 2).map(|i| u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()).collect()
}

/// THE ROUND's BEEKEM STEP for this space (the follower's, after its read).
pub(super) fn reconcile<H: Host>(h: &mut H, c: &Held, r: Round, m: &Mandate, now_ms: u64) -> Vec<Io> {
    let Some(gov) = governance(h, &r, now_ms) else { return next_space(h, c, r) };
    let Some(seed) = identity::upkeep_space_member(h, &r.member) else {
        followed(h, &r.member, &r.space, craftworks_rules::steps::KEYS_NO_MEMBER, &format!("{}: the account's data key is not held here: its keys are a page's", m.name));
        return next_space(h, c, r);
    };
    let mut g = Group::empty(&r.space, Account::of(&seed));
    // Every member LEARNED, this read's added (a card read now replaces what was kept).
    let members = learn(h, &r.member, &r.space, &r.reading.beekem);
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
    let ops: Vec<Vec<u8>> = r.reading.rows("pub-keyops").values().filter_map(|row| unhex(&row.value)).collect();
    // (`Group::merge_all`: the one loop, the page's too.)
    let merged = g.merge_all(&ops, &list);
    let (refused, waiting) = (merged.refused.len(), merged.waiting);
    let why: Vec<String> = merged.refused.iter().take(2).map(|w| w.chars().take(80).collect()).collect();
    // 3. THE GENERATIONS: the space's epochs on this node.
    for (n, k) in g.generations() {
        identity::keep_epoch(h, &r.member, Some(r.space), n, &k);
    }
    let placed = g.members();
    // 4. RECONCILE — from a whole read only.
    if let Err(why) = whole(&r, &gov) {
        followed(h, &r.member, &r.space, craftworks_rules::steps::KEYS_SHORT, &format!("{}: {why} — the keys are not changed this round", m.name));
        return next_space(h, c, r);
    }
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
    let due = members.iter().any(|(d, id, _)| !placed.contains(id) && list.listed(d)) || placed.iter().any(|id| list.by_member.get(id).is_some_and(|d| !list.listed(d) && outs.contains_key(d)));
    if !due {
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
    for (n, k) in g.generations() {
        identity::keep_epoch(h, &r.member, Some(r.space), n, &k);
    }
    let rows: Vec<(Vec<u8>, Option<Vec<u8>>)> = ops
        .iter()
        .filter(|op| 2 * op.len() <= ROW)
        .map(|op| (hex(&sha2::Sha256::digest(op)).into_bytes(), Some(hex(op).into_bytes())))
        .collect();
    let large = ops.len() - rows.len();
    let names = |v: &[String]| v.iter().map(|d| d.get(12..20).unwrap_or(d).to_string()).collect::<Vec<_>>().join(", ");
    followed(
        h,
        &r.member,
        &r.space,
        craftworks_rules::steps::KEYS_RECONCILED,
        &format!("{}: {} added ({}), {} removed ({}), one key change{}{}", m.name, done.added.len(), names(&done.added), done.removed.len(), names(&done.removed), if large > 0 { format!(" — {large} operation(s) too large for a row: a page's") } else { String::new() }, if done.not.is_empty() { format!(" — {}", done.not.join("; ")) } else { String::new() }),
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
            followed(h, &r.member, &r.space, craftworks_rules::steps::KEYS_WRITE, &format!("{}: the key operations were not written — {why}", m.name));
            next_space(h, c, r)
        }
    }
}
