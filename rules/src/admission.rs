//! LETTING SOMEONE INTO A GROUP — the ONE rule the page and the identity delegate both run (FOUNDATION §3, "Members and keys";
//! owner 10-13: one implementation, the right key to the right invite). Every group kind (a space, an audience, a
//! circle, a conversation, a level group) lets people in by it; the kind supplies only who may let in.
//!
//! - THE REQUEST'S OWN KEY: a request carries a key package its asker's device made for it (`kp`), and a welcome is
//!   made with that key and no other — never one picked from the person's card (a pool every member picked from: two
//!   picked the same key, a stale card copy offered a spent one, and the welcome never opened).
//! - ONCE PER REQUEST: a request welcomed already (any member's record in the group's shared list) is never let in
//!   again — only a NEWER request (a new key) is. No member re-adds anyone on its own: the loop that drained cards and
//!   ran a space to epoch 478 (10-13, every member and every node's upkeep re-adding whoever looked missing, each time
//!   a key and an epoch) cannot happen.
//! - ONE ADMITTER: every member ranks those who may let in the same way for the request (`turn`): the first does it at
//!   once, the k-th only after k × `takeover` with nobody having done it.
//! - TWO KINDS OF ASKING, one rule: a JOIN (someone new, by a code, the open door or a level — the bag's policy says who
//!   may) and AGAIN (a member by the group's acts — invited, let in, or behind — entering its group: from the epoch its
//!   device holds, or holding none). Who is entitled is the caller's (the bag it read); the rest is here.
//!
//! Pure: over what the caller read (the request, the group's shared list of who was welcomed with which key, the keys
//! spent) — no network, no node types.

/// A key package's TAG (sha-256 of its hex, 16 bytes): how it is named in the shared lists.
pub fn tag(kp_hex: &str) -> [u8; 16] {
    craftworks_identity::kp_tag(kp_hex)
}

/// WHO DOES A THING (no claim, no message): every member ranks `dids` the same way for `id` — 0 does it now, k takes
/// it over after k × the takeover time; -1: `me` is not among them. FNV-1a over the text (the page's `ordering.turn`
/// is this function, through the core: one ranking).
pub fn turn(id: &str, dids: &[String], me: &str) -> i64 {
    let hash = |s: &str| -> u32 {
        let mut h: u32 = 2166136261;
        for c in s.encode_utf16() {
            h = (h ^ u32::from(c)).wrapping_mul(16777619);
        }
        h
    };
    let mut all: Vec<&String> = Vec::new();
    for d in dids {
        if !all.contains(&d) {
            all.push(d);
        }
    }
    all.sort_by(|a, b| hash(&format!("{id}|{b}")).cmp(&hash(&format!("{id}|{a}"))).then_with(|| a.cmp(b)));
    all.iter().position(|d| d.as_str() == me).map_or(-1, |i| i as i64)
}

/// What is asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    /// Someone new (by a code, the open door, a level).
    Join,
    /// A member by the acts entering its group: `from` the epoch its device holds on this group's branch (behind), or
    /// None (holds nothing usable here: invited, off the branch, its welcome never opened — an entry of theirs the
    /// group still holds is replaced in the same commit). `seen`: the epoch it saw the group at WHEN IT ASKED — behind
    /// is judged THEN (10-14, measured on fx: a request made at the group's own epoch read as "behind" once the group
    /// moved on for someone else, and its asker was re-added, an epoch for nothing). None: not let in again.
    Again { from: Option<u64>, seen: Option<u64> },
}

/// A request to be let in.
pub struct Request<'a> {
    pub did: &'a str,
    /// The key package its device made for it (hex). None: a request from before requests carried one.
    pub kp: Option<&'a str>,
    /// When it was made (the asker's clock, ms).
    pub at: u64,
    pub entry: Entry,
}

/// A WELCOME already made (the group's shared list: who, with which key, when) — by any member, page or node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Welcomed {
    pub did: String,
    pub kp: [u8; 16],
    pub at: u64,
}

/// What the deciding member sees.
pub struct View<'a> {
    pub me: &'a str,
    /// Who may let people in (the kind's rule: owner and admins, inviters, any member …).
    pub admitters: &'a [String],
    /// Keys spent (tags): used by any welcome, here or anywhere this account knows.
    pub spent: &'a [[u8; 16]],
    pub welcomed: &'a [Welcomed],
    /// Whether the group holds them now (an entry of theirs).
    pub held: bool,
    /// The group's epoch here.
    pub epoch: u64,
    pub now: u64,
    pub takeover_ms: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    /// Let them in with THIS key (the request's own).
    Admit { kp: String, tag: [u8; 16] },
    /// Not now, or not at all — and why (said, never silent).
    Skip(String),
}

pub fn decide(q: &Request, v: &View) -> Decision {
    let skip = |why: &str| Decision::Skip(why.to_string());
    let Some(kp) = q.kp.filter(|k| !k.is_empty()) else {
        return skip("the request carries no key of its own: it asks again from an updated app");
    };
    let t = tag(kp);
    if v.spent.contains(&t) || v.welcomed.iter().any(|w| w.kp == t) {
        return skip("its key was used already: it asks again with a new one");
    }
    // ONCE PER REQUEST: welcomed at or after this request was made — this request (or a newer one) was answered.
    if v.welcomed.iter().any(|w| w.did == q.did && w.at >= q.at) {
        return skip("this request was answered already: only a newer request is let in");
    }
    match q.entry {
        // A JOIN from someone the group HOLDS — an entry of theirs with no admission behind it (a stray left by the
        // 10-13 add loop) or a device that lost the group: let in as `Again { from: None }` is, its old entry replaced
        // in the same commit, once per request (above). Skipped before (10-16, the owner in Ivvor's New Space: "in the
        // group already" to every request, while their device held nothing — let in by neither side, ever).
        Entry::Again { from: Some(e), seen } if v.held => match seen {
            None => return skip("its request does not say where it saw the group: it asks again from an updated app"),
            Some(s) if e >= s => return skip("not behind when it asked"),
            Some(s) if s > v.epoch => return skip("it saw the group past this member's epoch: not this member's to answer"),
            _ => {}
        },
        _ => {}
    }
    let others: Vec<String> = v.admitters.iter().filter(|d| d.as_str() != q.did).cloned().collect();
    let k = turn(&format!("admit|{}|{}", q.did, q.at), &others, v.me);
    if k < 0 {
        return skip("not one who may let people in");
    }
    if v.now.saturating_sub(q.at) < (k as u64).saturating_mul(v.takeover_ms) {
        return skip("another member's turn first");
    }
    Decision::Admit { kp: kp.to_string(), tag: t }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view<'a>(me: &'a str, admitters: &'a [String], spent: &'a [[u8; 16]], welcomed: &'a [Welcomed], held: bool) -> View<'a> {
        View { me, admitters, spent, welcomed, held, epoch: 10, now: 1_000_000, takeover_ms: 120_000 }
    }
    fn admitters() -> Vec<String> {
        vec!["did:a".into(), "did:b".into(), "did:c".into()]
    }
    fn first(id: &str) -> String {
        let a = admitters();
        a.iter().find(|d| turn(id, &a, d) == 0).unwrap().clone()
    }

    #[test]
    fn the_request_key_is_the_one_used_and_none_without_it() {
        let a = admitters();
        let q = Request { did: "did:x", kp: Some("aa11"), at: 999_000, entry: Entry::Join };
        let me = first("admit|did:x|999000");
        assert_eq!(decide(&q, &view(&me, &a, &[], &[], false)), Decision::Admit { kp: "aa11".into(), tag: tag("aa11") });
        let old = Request { kp: None, ..q };
        assert!(matches!(decide(&old, &view(&me, &a, &[], &[], false)), Decision::Skip(_)), "no request key: never a card's");
    }

    #[test]
    fn once_per_request_the_loop_cannot_happen() {
        let a = admitters();
        let q = Request { did: "did:x", kp: Some("aa11"), at: 999_000, entry: Entry::Join };
        let me = first("admit|did:x|999000");
        // Welcomed once (by anyone): the same request again — by this member or any other, page or node — is never let in.
        let w = [Welcomed { did: "did:x".into(), kp: tag("aa11"), at: 999_500 }];
        for m in &a {
            assert!(matches!(decide(&q, &view(m, &a, &[], &w, false)), Decision::Skip(_)), "{m} re-added the same request");
        }
        // Its key spent elsewhere (another space): not used again either.
        assert!(matches!(decide(&q, &view(&me, &a, &[tag("aa11")], &[], false)), Decision::Skip(_)));
        // A NEWER request with a NEW key (its device asked again after the welcome failed): let in.
        let again = Request { kp: Some("bb22"), at: 999_900, ..q };
        let me2 = first("admit|did:x|999900");
        assert!(matches!(decide(&again, &view(&me2, &a, &[], &w, false)), Decision::Admit { .. }));
    }

    #[test]
    fn one_admitter_at_a_time() {
        let a = admitters();
        let q = Request { did: "did:x", kp: Some("aa11"), at: 999_000, entry: Entry::Join };
        let id = "admit|did:x|999000";
        let admits: Vec<&String> = a.iter().filter(|m| matches!(decide(&q, &view(m, &a, &[], &[], false)), Decision::Admit { .. })).collect();
        assert_eq!(admits.len(), 1, "only the first in turn, at once");
        assert_eq!(turn(id, &a, admits[0]), 0);
        // The second takes over after one takeover with nobody having done it.
        let second = a.iter().find(|d| turn(id, &a, d) == 1).unwrap();
        let mut v = view(second, &a, &[], &[], false);
        v.now = 999_000 + 120_000;
        assert!(matches!(decide(&q, &v), Decision::Admit { .. }));
        // Someone not among those who may let in: never.
        assert!(matches!(decide(&q, &view("did:z", &a, &[], &[], false)), Decision::Skip(_)));
    }

    #[test]
    fn a_member_behind_is_let_in_again_once_per_request() {
        let a = admitters();
        let q = Request { did: "did:x", kp: Some("cc33"), at: 999_000, entry: Entry::Again { from: Some(4), seen: Some(10) } };
        let me = first("admit|did:x|999000");
        assert!(matches!(decide(&q, &view(&me, &a, &[], &[], true)), Decision::Admit { .. }));
        let w = [Welcomed { did: "did:x".into(), kp: tag("cc33"), at: 999_100 }];
        assert!(matches!(decide(&q, &view(&me, &a, &[], &w, true)), Decision::Skip(_)), "the same stuck request answered once");
        // Not behind here: nothing to do.
        let ahead = Request { entry: Entry::Again { from: Some(10), seen: Some(10) }, ..q };
        assert!(matches!(decide(&ahead, &view(&me, &a, &[], &[], true)), Decision::Skip(_)));
        // HOLDING NOTHING USABLE (invited; off the branch): let in, its old entry (if any) replaced — once per request.
        let invited = Request { entry: Entry::Again { from: None, seen: None }, ..q };
        assert!(matches!(decide(&invited, &view(&me, &a, &[], &[], false)), Decision::Admit { .. }));
        assert!(matches!(decide(&invited, &view(&me, &a, &[], &[], true)), Decision::Admit { .. }));
        assert!(matches!(decide(&invited, &view(&me, &a, &[], &w, true)), Decision::Skip(_)), "answered once");
        // ASKED AT THE GROUP'S OWN EPOCH (not behind then), the group moved on since: still not let in (the 10-14 re-add).
        let then = Request { entry: Entry::Again { from: Some(9), seen: Some(9) }, ..q };
        assert!(matches!(decide(&then, &view(&me, &a, &[], &[], true)), Decision::Skip(_)), "not behind when it asked");
        let unsaid = Request { entry: Entry::Again { from: Some(4), seen: None }, ..q };
        assert!(matches!(decide(&unsaid, &view(&me, &a, &[], &[], true)), Decision::Skip(_)), "where it saw the group unsaid");
        // A JOIN from someone the group HOLDS (a stray entry, no admission behind it; or a device that lost it): let in,
        // its old entry replaced — once per request (10-16: skipped before, the asker never let in).
        let join = Request { entry: Entry::Join, ..q };
        assert!(matches!(decide(&join, &view(&me, &a, &[], &[], true)), Decision::Admit { .. }), "a held asker is let in again");
        assert!(matches!(decide(&join, &view(&me, &a, &[], &w, true)), Decision::Skip(_)), "answered once");
    }

    #[test]
    fn turn_matches_the_pages_ordering() {
        // ordering.js: FNV-1a over UTF-16 code units, highest first, ties by name — the values below computed there.
        let dids: Vec<String> = ["did:craftec:a", "did:craftec:b", "did:craftec:c", "did:craftec:7kqYgwompGP2j5Q6n8TYDyY3ymYq2GaxbG86rHkv4nBz"].map(String::from).to_vec();
        let ranks = |id: &str| dids.iter().map(|d| turn(id, &dids, d)).collect::<Vec<_>>();
        assert_eq!(ranks("k"), vec![0, 1, 2, 3]);
        assert_eq!(ranks("admit|did:x|999000"), vec![3, 1, 2, 0]);
        assert_eq!(turn("k", &dids, "did:craftec:z"), -1);
    }
}
