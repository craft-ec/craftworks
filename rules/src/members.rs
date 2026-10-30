//! MEMBERS AND KEYS (FOUNDATION "Members and keys", the design of 10-17): every encrypted thing is a MEMBER LIST and KEYS
//! (an MLS group) that let exactly those members read. The list is the only decision; the keys follow it, by two rules
//! here — the page runs them through the core, the identity delegate natively:
//!
//! - IN, by the listed device itself: an MLS EXTERNAL COMMIT from the group's published info (`mls::enter`), its own old
//!   entry out in the same commit. Nobody answers anything; nobody else need be online.
//! - EVERY ENTRY CHECKED (`mls::entry_ok`, beside the group it checks): a reader takes a position's lowest entry that MLS applies AND the list allows —
//!   the committer listed, everyone it adds listed, an external commit taking out only the committer's own entry. An
//!   outsider's entry is skipped by every reader (it never holds a position).
//! - OUT: every member of an account the list no longer names, by whoever reconciles first (`groups::Group::reconcile`,
//!   page and node) — no turns (10-30: the MLS-era `follow` and its takeover wait removed, unused since BeeKEM).
//!
//! (Before, 10-03 → 10-16: members' nodes ANSWERED a device's posted key by re-adding it — asks, answers, windows and
//! turns; ~60 fixes and eight review rounds never closed it. MLS already lets a member enter by itself.)
use crate::admission::turn;

/// How many listed members besides the owner publish an epoch's GROUP INFO from their nodes (FOUNDATION "Members and
/// keys" 4): what a listed device with no key enters from when the owner's node is away — bounded, so a door holds a
/// few copies an epoch, never one per member.
pub const INFO_PUBLISHERS: i64 = 2;

/// Whether `me`'s node publishes epoch `epoch`'s group info: the OWNER's always; of the other members holding keys
/// (`followers`), the first `INFO_PUBLISHERS` in turn for that epoch (`admission::turn` — the same ranking everywhere).
/// AN EPOCH's DOOR ENTRIES (page and node): the opened door items that are an entry (`k: "enter"`) at that epoch,
/// each its entry text.
pub fn door_entries(items: &[serde_json::Value], epoch: u64) -> Vec<String> {
    items.iter().filter(|v| v["k"] == "enter" && v["e"].as_u64() == Some(epoch)).filter_map(|v| v["entry"].as_str().map(str::to_string)).collect()
}

/// THE FOLLOWERS: the listed accounts the group holds (page and node — who takes turns on the group's work).
pub fn followers(listed: &[String], in_group: &[String]) -> Vec<String> {
    listed.iter().filter(|d| in_group.contains(d)).cloned().collect()
}

pub fn publishes_info(me: &str, owner: &str, followers: &[String], epoch: u64) -> bool {
    if me == owner {
        return true;
    }
    let others: Vec<String> = followers.iter().filter(|d| d.as_str() != owner).cloned().collect();
    let k = turn(&format!("info|{epoch}"), &others, me);
    (0..INFO_PUBLISHERS).contains(&k)
}

/// A space's KEY OPERATIONS (`pub-keyops`, page and node one format): a row per operation, keyed by its SHA-256 (hex);
/// its value the operation (hex) — or, one too large for a row (a row holds 64 KB: a worst-case rotation from ~270
/// members), `{ "file": <ref> }`, a public file of the space whose bytes hash to the key.
pub const KEYOP_ROW: usize = 60000;
pub fn keyop_key(op: &[u8]) -> String {
    use sha2::Digest;
    craftworks_identity::hex(sha2::Sha256::digest(op))
}
/// Whether an operation goes in its row (else as a file the row names).
pub fn keyop_inline(op: &[u8]) -> bool {
    2 * op.len() <= KEYOP_ROW
}
/// A row's operation, or the file it names.
#[derive(Debug, Clone, PartialEq)]
pub enum KeyopRow {
    Op(Vec<u8>),
    File(serde_json::Value),
}
pub fn keyop_row(value: &str) -> Option<KeyopRow> {
    if value.starts_with('{') {
        let f = serde_json::from_str::<serde_json::Value>(value).ok()?.get("file")?.clone();
        return Some(KeyopRow::File(f));
    }
    craftworks_identity::unhex(value).map(KeyopRow::Op)
}
/// A file's bytes taken as its row's operation: only when they hash to the row's key.
pub fn keyop_file_ok(key: &str, bytes: &[u8]) -> bool {
    keyop_key(bytes) == key
}

/// An account's CARD (its group member) as read: kept an hour, then read again — a changed card is seen within it.
pub const CARD_KEEP_MS: u64 = 3_600_000;
/// An account with NO card or a refused one (gone; not opened since BeeKEM): asked again after ten minutes — never
/// every round (fx 10-18: every round re-reading every such card, past the node's per-round cap).
pub const CARD_ASK_AGAIN_MS: u64 = 600_000;
/// Whether an account's card is READ now (page and node): never read; or last read `at` (ms, `ok`: a member) longer ago
/// than the rule for what it read.
pub fn card_due(last: Option<(u64, bool)>, now_ms: u64) -> bool {
    match last {
        None => true,
        Some((at, ok)) => now_ms.saturating_sub(at) >= if ok { CARD_KEEP_MS } else { CARD_ASK_AGAIN_MS },
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_keyop_row_is_its_op_or_the_file_it_names() {
        use super::*;
        let op = vec![1u8, 2, 3];
        assert!(keyop_inline(&op) && !keyop_inline(&vec![0; KEYOP_ROW / 2 + 1]));
        assert_eq!(keyop_row(&craftworks_identity::hex(&op)), Some(KeyopRow::Op(op.clone())));
        assert_eq!(keyop_row(r#"{"file":{"r":"x"}}"#), Some(KeyopRow::File(serde_json::json!({ "r": "x" }))));
        assert_eq!(keyop_row("zz"), None);
        assert!(keyop_file_ok(&keyop_key(&op), &op) && !keyop_file_ok(&keyop_key(&op), &[9]));
    }
    #[test]
    fn a_card_is_read_again_by_what_it_read() {
        use super::*;
        assert!(card_due(None, 0));
        assert!(!card_due(Some((0, true)), CARD_KEEP_MS - 1) && card_due(Some((0, true)), CARD_KEEP_MS));
        assert!(!card_due(Some((0, false)), CARD_ASK_AGAIN_MS - 1) && card_due(Some((0, false)), CARD_ASK_AGAIN_MS));
    }
    use super::*;

    /// THE FOLLOWERS (page and node): the listed the group holds — one listed not in the group, or in it unlisted, is not.
    #[test]
    fn door_entries_are_that_epochs_enter_items() {
        let items = [serde_json::json!({ "k": "enter", "e": 3, "entry": "x" }), serde_json::json!({ "k": "enter", "e": 4, "entry": "y" }), serde_json::json!({ "k": "info", "e": 3 }), serde_json::json!({ "k": "enter", "e": 3, "entry": 1 })];
        assert_eq!(door_entries(&items, 3), vec!["x"]);
    }
    #[test]
    fn followers_are_the_listed_the_group_holds() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(followers(&s(&["a", "b", "c"]), &s(&["b", "c", "x"])), s(&["b", "c"]));
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }




    #[test]
    fn group_info_from_the_owner_and_two_members_in_turn() {
        let f = s(&["did:o", "did:a", "did:b", "did:c", "did:d"]);
        for epoch in [1, 7, 485] {
            assert!(publishes_info("did:o", "did:o", &f, epoch), "the owner always");
            let n = f.iter().filter(|m| m.as_str() != "did:o" && publishes_info(m, "did:o", &f, epoch)).count();
            assert_eq!(n, INFO_PUBLISHERS as usize, "exactly two others, epoch {epoch}");
        }
        assert!(!publishes_info("did:x", "did:o", &f, 7), "not holding keys: never");
        // The owner away (not among those holding keys): still two members publish.
        let away = s(&["did:a", "did:b", "did:c"]);
        assert_eq!(away.iter().filter(|m| publishes_info(m, "did:o", &away, 3)).count(), 2);
    }
}
