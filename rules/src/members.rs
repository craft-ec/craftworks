//! MEMBERS AND KEYS (FOUNDATION "Members and keys", the design of 10-17): every encrypted thing is a MEMBER LIST and KEYS
//! (an MLS group) that let exactly those members read. The list is the only decision; the keys follow it, by two rules
//! here — the page runs them through the core, the identity delegate natively:
//!
//! - IN, by the listed device itself: an MLS EXTERNAL COMMIT from the group's published info (`mls::enter`), its own old
//!   entry out in the same commit. Nobody answers anything; nobody else need be online.
//! - EVERY ENTRY CHECKED (`mls::entry_ok`, beside the group it checks): a reader takes a position's lowest entry that MLS applies AND the list allows —
//!   the committer listed, everyone it adds listed, an external commit taking out only the committer's own entry. An
//!   outsider's entry is skipped by every reader (it never holds a position).
//! - OUT (`follow`): every entry of an account the list no longer names — ONE commit, by ONE member at a time
//!   (`admission::turn`, taken over after `guards::TAKEOVER_MS`), only on a WHOLE read of the list (I6).
//!
//! (Before, 10-03 → 10-16: members' nodes ANSWERED a device's posted key by re-adding it — asks, answers, windows and
//! turns; ~60 fixes and eight review rounds never closed it. MLS already lets a member enter by itself.)
use crate::admission::turn;

/// A key ENTRY in the group: its index, whose (the DID its credential names), and whether the PROOF in its leaf (a
/// device that entered by itself with a code, the open door, a level) is one the acts admit now (`gov::Gov::admits`) —
/// such an entry counts as listed until its own `join` act is (FOUNDATION "Members and keys" 2).
#[derive(Clone, Debug)]
pub struct Leaf {
    pub index: u32,
    pub did: String,
    pub admitted: bool,
}

/// What the deciding member sees.
pub struct View<'a> {
    pub me: &'a str,
    /// THE MEMBER LIST (the kind's: a space's acts' roster, a level's card holders, a chat's started/added …).
    pub listed: &'a [String],
    /// The list was read WHOLE (`guards::whole`): only then is anyone taken out (I6).
    pub whole: bool,
    pub leaves: &'a [Leaf],
    /// Who follows (the listed members who hold the keys now): ranked by `turn`.
    pub followers: &'a [String],
    pub epoch: u64,
    /// When the list last put someone OUT (the act's time): a removal is taken over from then.
    pub out_since: u64,
    pub now: u64,
    pub takeover_ms: u64,
}

/// One commit's plan: entries out (of no one listed); or not now, and why.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub remove: Vec<u32>,
    /// Not this follower's turn (why).
    pub not_my_turn: Option<String>,
    /// Not this follower's turn yet: how long until it is (ms) — when to look again.
    pub wait: Option<u64>,
    /// Nothing decided: why (a short read of the list).
    pub not_now: Option<String>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.remove.is_empty()
    }
}

/// THE KEYS FOLLOW THE LIST OUT.
pub fn follow(v: &View) -> Plan {
    let mut p = Plan::default();
    // A follower is on the list itself (an outsider never acts on a group).
    if crate::guards::listed(v.me, v.listed).is_err() {
        return p;
    }
    let listed: std::collections::BTreeSet<&str> = v.listed.iter().map(|s| s.as_str()).collect();
    let out: Vec<u32> = v.leaves.iter().filter(|l| !listed.contains(l.did.as_str()) && !l.admitted && l.did != v.me).map(|l| l.index).collect();
    if out.is_empty() {
        return p;
    }
    // NOTHING PERMANENT FROM A SHORT READ (I6).
    if !v.whole {
        p.not_now = Some("the list was not read whole: nobody taken out from it".into());
        return p;
    }
    // ONE FOLLOWER at a time — from when the list put them out (a member first in turn and away is taken over).
    let k = turn(&format!("follow|{}", v.epoch), v.followers, v.me);
    if let Err(why) = crate::guards::my_turn(k, v.out_since, v.now, v.takeover_ms) {
        if k >= 0 {
            p.wait = Some(crate::guards::turn_wait(k as u64, v.out_since, v.now, v.takeover_ms));
        }
        p.not_my_turn = Some(why);
        return p;
    }
    p.remove = out;
    p
}

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
    fn leaf(i: u32, d: &str) -> Leaf {
        Leaf { index: i, did: d.into(), admitted: false }
    }
    fn first(ids: &[String], epoch: u64) -> String {
        ids.iter().find(|m| turn(&format!("follow|{epoch}"), ids, m) == 0).unwrap().clone()
    }
    fn view<'a>(me: &'a str, listed: &'a [String], leaves: &'a [Leaf], followers: &'a [String]) -> View<'a> {
        View { me, listed, whole: true, leaves, followers, epoch: 485, out_since: 1_000_000, now: 1_000_000, takeover_ms: 120_000 }
    }

    #[test]
    fn unlisted_entries_out_in_one_commit_listed_ones_stay() {
        let listed = s(&["did:ivvor", "did:a", "did:b"]);
        let leaves = vec![leaf(0, "did:ivvor"), leaf(1, "did:a"), leaf(4, "did:gone"), leaf(9, "did:stray")];
        let followers = s(&["did:ivvor"]);
        assert_eq!(follow(&view("did:ivvor", &listed, &leaves, &followers)).remove, vec![4, 9]);
        // Nothing unlisted: nothing to do (a listed member without keys enters by itself — never added here).
        assert!(follow(&view("did:ivvor", &listed, &leaves[..2], &followers)).is_empty());
        // An entry by a proof the acts admit (its `join` act not written yet): kept; control above — the same unadmitted: out.
        let mut by_proof = leaves.clone();
        by_proof[2].admitted = true;
        assert_eq!(follow(&view("did:ivvor", &listed, &by_proof, &followers)).remove, vec![9]);
    }

    #[test]
    fn a_short_read_takes_nobody_out() {
        let listed = s(&["did:me"]);
        let leaves = [leaf(0, "did:me"), leaf(3, "did:x")];
        let followers = s(&["did:me"]);
        let mut v = view("did:me", &listed, &leaves, &followers);
        assert_eq!(follow(&v).remove, vec![3], "control: read whole, out");
        v.whole = false;
        let p = follow(&v);
        assert!(p.is_empty() && p.not_now.is_some(), "short: nobody out");
    }

    #[test]
    fn an_outsider_never_follows_and_one_follower_at_a_time() {
        let listed = s(&["did:a", "did:b", "did:c"]);
        let leaves = [leaf(7, "did:x")];
        assert!(follow(&view("did:out", &listed, &leaves, &listed)).is_empty(), "not on the list: never acts");
        let one = first(&listed, 485);
        let acted: Vec<&String> = listed.iter().filter(|m| !follow(&view(m, &listed, &leaves, &listed)).is_empty()).collect();
        assert_eq!(acted, vec![&one], "only the first in turn, at once");
        let other = listed.iter().find(|m| **m != one).unwrap();
        let mut v = view(other, &listed, &leaves, &listed);
        assert!(follow(&v).wait.is_some(), "the others wait, and say how long");
        v.now += 2 * v.takeover_ms;
        assert_eq!(follow(&v).remove, vec![7], "taken over once the first stays away");
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
