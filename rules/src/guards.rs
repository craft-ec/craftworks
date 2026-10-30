//! THE GUARDS of membership and of keys following the member list — EACH ONCE (owner 10-16: "guards are rules like the
//! lifecycle, so everything is in one implementation"). Facts in, `Ok` or why not out. Their callers: `members::follow`
//! by name; the identity delegate's follower directly; the page through the core (`guard_*`). No caller keeps a copy of
//! any condition here.

pub type Verdict = Result<(), String>;
fn no(why: &str) -> Verdict {
    Err(why.to_string())
}

/// IN STANDING, by the space's acts (`gov`'s replay — who is OUT is its `gone`: removed, banned, left, lapsed, less
/// anyone added back since): never the banned; never one removed (back only by an invite: an `added`); one who left only
/// by asking after they left; one out by a LAPSE alone may ask again (they ask when they give again). Asked at `at`.
pub fn standing(g: &craftworks_gov::Gov, did: &str, at: u64) -> Verdict {
    if g.bans.contains(did) {
        return no("banned from this space");
    }
    // (What put them out, by the replay: `Gov::out_act`.)
    match g.out_act(did) {
        None => Ok(()),
        Some(("leave", left)) if at as f64 > left => Ok(()),
        Some(("leave", _)) => no("they left after asking: only a newer request"),
        Some(("lapsed", _)) => Ok(()),
        Some(_) => no("removed from this space: back only by an invite"),
    }
}

/// ON THE LIST TO BE: a member may put `did` on the list (an invite, a Let in) — never the banned (removed people come
/// back exactly this way).
pub fn may_list(g: &craftworks_gov::Gov, did: &str) -> Verdict {
    if g.may_be_added(did) {
        return Ok(());
    }
    no("banned from this space")
}

/// LISTED: on the member list (who acts on a group is one of its listed members).
pub fn listed(did: &str, list: &[String]) -> Verdict {
    if list.iter().any(|d| d == did) {
        return Ok(());
    }
    no("not on the member list")
}

/// LEVEL: their card's running contributions (`contrib`, JSON) reach the group's level `need` now.
pub fn level(contrib: Option<&str>, need: &str, now_ms: u64) -> Verdict {
    let (have, _) = craftworks_gov::level_from(contrib.unwrap_or("[]"), now_ms as f64);
    if craftworks_gov::level_reaches(have, need) {
        return Ok(());
    }
    Err(format!("their card shows {have}, not {need}"))
}

/// WHOLE READ: an entry is taken out of a group only on a WHOLE read of its list — a feed silent (`unread`), an act
/// named and not held (`missing`): the list may be short, and short would look like people leaving.
pub fn whole(unread: &[String], missing: usize) -> Verdict {
    if unread.is_empty() && missing == 0 {
        return Ok(());
    }
    Err(format!("the list read SHORT ({}{}): nobody out", unread.join("; "), if missing > 0 { format!("; {missing} act(s) named, not held") } else { String::new() }))
}

/// THE LIST READ WHOLE — what counts as unread, page and node the same: a listed member whose DEVICES did not read
/// (`devices`: `"<who>: why"` — their acts were not looked for) and a feed of the LIST's own tables that did not read
/// (`feeds`: `(table, why)` — only `…acts` and `…pub-acts` count: an unread key operation or departed row is not the
/// list). 10-19: the node counted every failed read of any table, the page none of a member's devices — each its own.
pub fn list_whole(devices: &[String], feeds: &[(String, String)], missing: usize) -> Verdict {
    let unread: Vec<String> = devices.iter().cloned().chain(feeds.iter().filter(|(t, _)| t.ends_with("-acts") || t == "acts").map(|(t, w)| format!("{t}: {w}"))).collect();
    whole(&unread, missing)
}

/// AN ENTRY's OUTCOME (page and node): APPLIED when the list check takes it — its answer never changes as more of the
/// list is read (EVER listed), so a short read applies it too; one it does not take is SKIPPED only on a whole read and
/// DEFERRED on a short one (10-19: the node decided nothing on a short read, the page applied what the check took).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryOutcome {
    Apply,
    Skip,
    Defer,
}
pub fn entry_outcome(taken: bool, whole: bool) -> EntryOutcome {
    match (taken, whole) {
        (true, _) => EntryOutcome::Apply,
        (false, true) => EntryOutcome::Skip,
        (false, false) => EntryOutcome::Defer,
    }
}

/// LEADS PAST: a group this device's own node moved (its removals) is taken only if it leads past this device's epoch
/// (an older one never takes it back).
pub fn leads_past(to: u64, held: Option<u64>) -> Verdict {
    match held {
        Some(h) if to <= h => Err(format!("that group is at epoch {to}, not past this device's {h}: not taken")),
        _ => Ok(()),
    }
}

/// WHERE A DEVICE's KEYS OF A SPACE STAND (the `group` lifecycle's state, from what its page reads): `held` its epoch
/// (none: no group here), `owner` the owner's epoch as announced, `on_branch` (false: another branch), `unreadable`
/// tables sealed with an epoch it lacks, `why` a reason it could not tell (then `unknown`: no move — a passing read
/// error is no reason to enter again). One of `held`, `ahead`, `behind`, `off-branch`, `unreadable`, `not-held`,
/// `unknown`. (A device enters by itself from `not-held`, `off-branch`, `unreadable` and `stuck`; never `behind`: the
/// logs bring it current.) The one classification: this device's own state, and what a page shows of another member's.
pub fn group_state(held: Option<u64>, owner: Option<u64>, on_branch: Option<bool>, unreadable: bool, why: bool) -> &'static str {
    match held {
        _ if why => "unknown",
        None => "not-held",
        // Another branch first: behind on a FORK reads nothing new from the logs (its next commit is not in them).
        _ if on_branch == Some(false) => "off-branch",
        Some(e) if owner.is_some_and(|o| e < o) => "behind",
        _ if unreadable => "unreadable",
        Some(e) if owner.is_some_and(|o| e > o) => "ahead",
        _ => "held",
    }
}
/// ON THE OWNER's BRANCH — the one test (this device's, and what a page shows of another member): at or past the owner's
/// newest announced epoch, by its fingerprint there; behind, by the owner's announcement AT ITS OWN epoch; else not
/// known. `owner`: the owner's announcements `(epoch, fingerprint)`, newest first; `fp_at(e)`: its own fingerprint at e
/// (None: does not read — NOT KNOWN, never "another branch": a read error would ask for keys).
pub fn on_branch(epoch: Option<u64>, owner: &[(u64, String)], fp_at: &dyn Fn(u64) -> Option<String>) -> Option<bool> {
    let e = epoch?;
    let (oe, of) = owner.first()?;
    let (at, want) = if e >= *oe { (*oe, of) } else { owner.iter().find(|(x, _)| *x == e).map(|(x, f)| (*x, f))? };
    fp_at(at).map(|fp| &fp == want)
}

/// Whether a device in group state `s` ENTERS BY ITSELF (reading the logs cannot fix it).
pub fn cannot_read(s: &str) -> bool {
    matches!(s, "not-held" | "off-branch" | "unreadable" | "stuck")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_guard_says_why_not() {
        assert!(leads_past(5, Some(5)).is_err() && leads_past(6, Some(5)).is_ok() && leads_past(1, None).is_ok());
        assert!(whole(&[], 0).is_ok() && whole(&["x: no key log".into()], 0).is_err() && whole(&[], 2).is_err());
        assert_eq!((entry_outcome(true, false), entry_outcome(false, true), entry_outcome(false, false)), (EntryOutcome::Apply, EntryOutcome::Skip, EntryOutcome::Defer));
        // THE LIST's own feeds count, other tables' do not; an unread member's devices count (page and node the same).
        let feeds = |t: &str| vec![(t.to_string(), "not read".to_string())];
        assert!(list_whole(&[], &feeds("x0cce26765a1f-acts"), 0).is_err() && list_whole(&[], &feeds("x0cce26765a1f-pub-acts"), 0).is_err());
        assert!(list_whole(&[], &feeds("x0cce26765a1f-pub-keyops"), 0).is_ok() && list_whole(&[], &feeds("x0cce26765a1f-departed"), 0).is_ok());
        assert!(list_whole(&["did:x: their card and devices contract not read".into()], &[], 0).is_err() && list_whole(&[], &[], 1).is_err());
        assert!(listed("a", &["a".into()]).is_ok() && listed("b", &["a".into()]).is_err());
    }

    #[test]
    fn the_states_a_device_enters_from() {
        assert_eq!(group_state(Some(3), Some(5), Some(true), false, false), "behind");
        assert_eq!(group_state(Some(3), Some(5), Some(false), false, false), "off-branch", "behind on a fork");
        assert_eq!(group_state(Some(5), Some(5), Some(false), false, false), "off-branch");
        assert_eq!(group_state(None, Some(5), None, false, false), "not-held");
        assert_eq!(group_state(Some(5), Some(5), Some(true), true, false), "unreadable");
        assert_eq!(group_state(Some(5), None, None, false, false), "held");
        assert_eq!(group_state(None, None, None, false, true), "unknown", "a read error: no state, no entry");
        assert_eq!(group_state(Some(7), Some(5), Some(true), false, false), "ahead");
        assert!(!cannot_read("behind") && cannot_read("stuck") && cannot_read("off-branch") && cannot_read("not-held"));
        let owner = [(9u64, "f9".to_string()), (5, "f5".to_string())];
        assert_eq!(on_branch(Some(9), &owner, &|_| Some("f9".into())), Some(true));
        assert_eq!(on_branch(Some(9), &owner, &|_| Some("xx".into())), Some(false));
        assert_eq!(on_branch(Some(9), &owner, &|_| None), None, "a fingerprint that does not read: not known");
        assert_eq!(on_branch(Some(5), &owner, &|e| (e == 5).then(|| "f5".into())), Some(true), "behind: by the owner's at its epoch");
        assert_eq!(on_branch(Some(7), &owner, &|_| Some("f7".into())), None, "behind with no announcement there: not known");
    }

    #[test]
    fn standing_by_the_acts() {
        use serde_json::json;
        let mut g = craftworks_gov::Gov::default();
        g.bans.insert("b".into());
        g.counted = vec![
            json!({ "act": "remove", "did": "r", "at": 10.0 }),
            json!({ "act": "leave", "by": "l", "at": 20.0 }),
            json!({ "act": "lapsed", "did": "p", "at": 30.0 }),
        ];
        g.gone = ["r".to_string(), "l".to_string(), "p".to_string()].into();
        assert!(standing(&g, "b", 99).is_err(), "banned");
        assert!(standing(&g, "r", 99).is_err(), "removed: an invite only");
        assert!(standing(&g, "l", 15).is_err() && standing(&g, "l", 25).is_ok(), "left: a newer request only");
        assert!(standing(&g, "p", 1).is_ok(), "lapsed: asks again");
        assert!(standing(&g, "n", 1).is_ok());
        g.gone.remove("r");
        assert!(standing(&g, "r", 99).is_ok(), "re-added");
    }
}
