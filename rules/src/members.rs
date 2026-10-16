//! MEMBERS AND KEYS (FOUNDATION "Members and keys", owner 10-16): every encrypted thing is a MEMBER LIST and KEYS (an MLS
//! group) that let exactly those members read. The list is the only decision; the keys FOLLOW it. This is the one
//! follower's rule — the page runs it through the core, the identity delegate natively — returning ONE commit's plan:
//!
//! - every key entry (leaf) of someone NOT on the list: out;
//! - every listed member who CANNOT READ — said by its device posting a fresh key ("my key": a key package it holds) —
//!   in with that key, its old entries out in the same commit; each key answered ONCE (a welcome at or after it, or the
//!   key spent or welcomed already: not again). Nobody listed is left unable to read; nobody unlisted is let in.
//! - ONE follower at a time: the members rank the same way (`admission::turn` over the epoch), the k-th acting only
//!   after k × takeover since the oldest key it would answer — strays alone, the first only.
//!
//! (Before: two lists never forced to agree — the space's record and its keys — and ~50 commits of asks, re-welcomes and
//! catch-ups made to reconcile them after the fact; Ivvor's New Space at epoch 485 with none of its 7 members reading.)
use crate::admission::{tag, turn, Welcomed};

/// A key ENTRY in the group: its index and whose (the DID its credential names).
#[derive(Clone, Debug)]
pub struct Leaf {
    pub index: u32,
    pub did: String,
}

/// A key a member's device POSTED ("my key"): a key package (hex) it holds the secret of, and when.
#[derive(Clone, Debug)]
pub struct Key {
    pub did: String,
    pub kp: String,
    pub at: u64,
}

/// What the deciding member sees.
pub struct View<'a> {
    pub me: &'a str,
    /// THE MEMBER LIST (the kind's: a space's acts' roster, a level's card holders, a chat's started/added …).
    pub listed: &'a [String],
    pub leaves: &'a [Leaf],
    pub keys: &'a [Key],
    /// The group's shared list of welcomes made (by any member or node).
    pub welcomed: &'a [Welcomed],
    /// Key tags spent (used by any welcome this account knows).
    pub spent: &'a [[u8; 16]],
    /// Who follows (the listed members who hold the keys now): ranked by `turn`.
    pub followers: &'a [String],
    pub epoch: u64,
    pub now: u64,
    pub takeover_ms: u64,
}

/// One commit's plan: entries out, keys in (did, key package hex, its tag), and why each key was not taken.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub remove: Vec<u32>,
    pub add: Vec<(String, String, [u8; 16])>,
    pub skipped: Vec<(String, String)>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.remove.is_empty() && self.add.is_empty()
    }
}

pub fn follow(v: &View) -> Plan {
    let mut p = Plan::default();
    let listed: std::collections::BTreeSet<&str> = v.listed.iter().map(|s| s.as_str()).collect();
    // A follower is on the list itself (an outsider never acts on a group).
    if !listed.contains(v.me) {
        return p;
    }
    // KEYS IN: each listed member's NEWEST key, answered once.
    let mut newest: std::collections::BTreeMap<&str, &Key> = std::collections::BTreeMap::new();
    for k in v.keys.iter().filter(|k| !k.kp.is_empty()) {
        if newest.get(k.did.as_str()).is_none_or(|n| k.at > n.at) {
            newest.insert(k.did.as_str(), k);
        }
    }
    let mut remove: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    let mut oldest: Option<u64> = None;
    for (did, k) in newest {
        let t = tag(&k.kp);
        let why = if did == v.me {
            Some("this follower's own key: another member answers it")
        } else if !listed.contains(did) {
            Some("not on the member list: a key is only for a listed member")
        } else if v.spent.contains(&t) || v.welcomed.iter().any(|w| w.kp == t) {
            Some("its key was used already: it posts a new one")
        } else if v.welcomed.iter().any(|w| w.did == did && w.at >= k.at) {
            Some("answered already: a newer key is needed")
        } else {
            None
        };
        if let Some(why) = why {
            p.skipped.push((did.to_string(), why.to_string()));
            continue;
        }
        for l in v.leaves.iter().filter(|l| l.did == did) {
            remove.insert(l.index);
        }
        p.add.push((did.to_string(), k.kp.clone(), t));
        oldest = Some(oldest.map_or(k.at, |o| o.min(k.at)));
    }
    // ENTRIES OUT: anyone not on the list (never this follower's own).
    for l in v.leaves.iter().filter(|l| !listed.contains(l.did.as_str()) && l.did != v.me) {
        remove.insert(l.index);
    }
    p.remove = remove.into_iter().collect();
    if p.is_empty() {
        return p;
    }
    // ONE FOLLOWER at a time.
    // (`turn` ranks the followers, this one among them: -1 when it is not.)
    let k = turn(&format!("follow|{}", v.epoch), v.followers, v.me);
    let waited = oldest.map_or(0, |o| v.now.saturating_sub(o));
    let go = k == 0 || (k > 0 && oldest.is_some() && waited >= (k as u64).saturating_mul(v.takeover_ms));
    if !go {
        p.skipped.push(("*".into(), if k < 0 { "not a follower".into() } else { "another member's turn first".into() }));
        p.remove.clear();
        p.add.clear();
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }
    fn leaf(i: u32, d: &str) -> Leaf {
        Leaf { index: i, did: d.into() }
    }
    fn key(d: &str, kp: &str, at: u64) -> Key {
        Key { did: d.into(), kp: kp.into(), at }
    }
    // The follower that acts at once among `ids` for this epoch.
    fn first(ids: &[String], epoch: u64) -> String {
        ids.iter().find(|m| turn(&format!("follow|{epoch}"), ids, m) == 0).unwrap().clone()
    }
    fn view<'a>(me: &'a str, listed: &'a [String], leaves: &'a [Leaf], keys: &'a [Key], welcomed: &'a [Welcomed], followers: &'a [String]) -> View<'a> {
        View { me, listed, leaves, keys, welcomed, spent: &[], followers, epoch: 485, now: 1_000_000, takeover_ms: 120_000 }
    }

    #[test]
    fn ivvors_space_every_listed_member_who_cannot_read_in_at_once_and_a_stray_out() {
        // Ivvor's New Space as measured 10-16: Ivvor reads; four hold entries at old epochs, three never got keys; one
        // entry belongs to nobody listed. Each listed member's device posted its key.
        let listed = s(&["did:ivvor", "did:a", "did:b", "did:c", "did:d", "did:e", "did:f", "did:g"]);
        let leaves = vec![leaf(0, "did:ivvor"), leaf(1, "did:a"), leaf(2, "did:b"), leaf(3, "did:c"), leaf(4, "did:d"), leaf(9, "did:stray")];
        let keys: Vec<Key> = ["did:a", "did:b", "did:c", "did:d", "did:e", "did:f", "did:g"].iter().enumerate().map(|(i, d)| key(d, &format!("aa{i:02}"), 999_000)).collect();
        let followers = s(&["did:ivvor"]);
        let p = follow(&view("did:ivvor", &listed, &leaves, &keys, &[], &followers));
        assert_eq!(p.add.len(), 7, "all seven in, one commit: {:?}", p.skipped);
        assert_eq!(p.remove, vec![1, 2, 3, 4, 9], "their old entries and the stray out, the same commit");
    }

    #[test]
    fn each_key_answered_once_never_a_spent_one_never_an_unlisted_asker() {
        let listed = s(&["did:me", "did:x"]);
        let followers = s(&["did:me"]);
        let k = [key("did:x", "bb11", 999_000)];
        assert_eq!(follow(&view("did:me", &listed, &[], &k, &[], &followers)).add.len(), 1);
        // Answered: a welcome at or after the key.
        let w = [Welcomed { did: "did:x".into(), kp: tag("cc22"), at: 999_500 }];
        assert!(follow(&view("did:me", &listed, &[], &k, &w, &followers)).is_empty(), "answered once: the loop cannot happen");
        // The key itself welcomed (or spent): never again, whoever asks with it.
        let w2 = [Welcomed { did: "did:y".into(), kp: tag("bb11"), at: 1 }];
        assert!(follow(&view("did:me", &listed, &[], &k, &w2, &followers)).is_empty());
        // Someone NOT on the list posting a key: nothing (a list entry is the only way in).
        let k2 = [key("did:z", "dd33", 999_000)];
        let p = follow(&view("did:me", &listed, &[], &k2, &[], &followers));
        assert!(p.is_empty() && p.skipped.iter().any(|(d, _)| d == "did:z"));
    }

    #[test]
    fn a_newer_key_after_an_answer_is_answered_again_a_member_who_reads_posts_none() {
        let listed = s(&["did:me", "did:x"]);
        let followers = s(&["did:me"]);
        let w = [Welcomed { did: "did:x".into(), kp: tag("ee44"), at: 500 }];
        // Its welcome never opened: its device posts a NEWER key — answered, its entry replaced.
        let k = [key("did:x", "ee44", 400), key("did:x", "ff55", 900)];
        let p = follow(&view("did:me", &listed, &[leaf(3, "did:x")], &k, &w, &followers));
        assert_eq!(p.add.iter().map(|a| a.1.as_str()).collect::<Vec<_>>(), vec!["ff55"]);
        assert_eq!(p.remove, vec![3]);
        // A member that reads posts no key: nothing to do.
        assert!(follow(&view("did:me", &listed, &[leaf(0, "did:me"), leaf(3, "did:x")], &[], &w, &followers)).is_empty());
    }

    #[test]
    fn an_outsider_never_follows_and_one_follower_at_a_time() {
        let listed = s(&["did:a", "did:b", "did:c", "did:x"]);
        let k = [key("did:x", "aa11", 999_000)];
        assert!(follow(&view("did:out", &listed, &[], &k, &[], &listed)).is_empty(), "not on the list: never acts");
        let followers = s(&["did:a", "did:b", "did:c"]);
        let one = first(&followers, 485);
        let acted: Vec<&String> = followers.iter().filter(|m| !follow(&view(m, &listed, &[], &k, &[], &followers)).is_empty()).collect();
        assert_eq!(acted, vec![&one], "only the first in turn, at once");
        // The next after one takeover.
        let mut v = view(followers.iter().find(|m| **m != one).unwrap(), &listed, &[], &k, &[], &followers);
        v.now = 999_000 + 2 * 120_000;
        assert!(!follow(&v).is_empty(), "the next takes over once its turn's time passed");
    }
}
