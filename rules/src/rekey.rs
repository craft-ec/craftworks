//! FILE RE-KEY — THE PLAN (page and node, one implementation): whether a space's files need a NEW SALT (a removal is in
//! and the group has moved without them), and which key rows are DUE with this member's turn for each.
//! (10-19, inventory #7/#8: the page had its own `due`/`rotateIfRemoved` — it ranked members the acts had put out,
//! and planned a rotation where the node would not; now both call this.)

use crate::{admission, guards};

/// A key row of `files` (`k/<id>`) as the plan reads it: its salt number (`n`: -2 adopted, -1 public), whether it is
/// public, its app (domain), and when it last MOVED (its row's time, its progress — the later).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub n: i64,
    pub public: bool,
    pub app: String,
    pub moved: u64,
}

/// One row due: what it becomes (public, or sealed under the salt now), this member's turn, the wait from `moved`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Due {
    pub id: String,
    pub public: bool,
    pub rank: u64,
    pub wait: u64,
    pub moved: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// A new salt is due, made after this many removals.
    pub rotate: Option<usize>,
    pub due: Vec<Due>,
}

/// THE PLAN. `salt`: the space's salt `(n, removals made after)` — none, no rotation (the first salt is made where a
/// file is first put). `out`: the target of each act that put someone out, in order. `members`: the group's DIDs now (a
/// removal must have taken them out first). `gone`, `banned`: put out by the acts — never among those who take turns
/// (a group held from before a removal still lists them, and one ranked first would hold every row up).
/// `reads_anyone(app)`: whether that domain's content reads in public.
#[allow(clippy::too_many_arguments)]
pub fn plan(salt: Option<(i64, usize)>, out: &[String], members: &[String], gone: &[String], banned: &[String], rows: &[Row], reads_anyone: &dyn Fn(&str) -> bool, me: &str, now_ms: u64) -> Plan {
    let rotate = salt.and_then(|(_, removals)| (out.len() > removals && !members.iter().any(|m| out[removals..].contains(m))).then_some(out.len()));
    let n_now = salt.map(|(n, _)| n).unwrap_or(0) + i64::from(rotate.is_some());
    let ranked: Vec<String> = members.iter().filter(|m| !gone.contains(m) && !banned.contains(m)).cloned().collect();
    let mut due = Vec::new();
    for row in rows {
        let public = row.public && reads_anyone(if row.app == "drive" { "file" } else { &row.app });
        let is_due = row.n == -2 || (if row.n == -1 { !public } else { row.n < n_now });
        let Ok(rank) = u64::try_from(admission::turn(&row.id, &ranked, me)) else { continue };
        if is_due {
            due.push(Due { id: row.id.clone(), public: row.n == -2 && public, rank, wait: guards::turn_wait(rank, row.moved, now_ms, guards::TAKEOVER_MS), moved: row.moved });
        }
    }
    Plan { rotate, due }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, n: i64, public: bool) -> Row {
        Row { id: id.into(), n, public, app: "drive".into(), moved: 0 }
    }
    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    /// A removal in, the group moved without them: a new salt; rows under the old salt due. The group still holding them:
    /// not yet. No salt: no rotation. One put out is never ranked (it would hold every row up).
    #[test]
    fn the_plan_rotates_after_the_group_moved_and_never_ranks_the_put_out() {
        let anyone = |_: &str| false;
        let rows = [row("r1", 0, false), row("r2", -2, false)];
        let p = plan(Some((0, 0)), &s(&["did:x"]), &s(&["did:a", "did:b"]), &s(&["did:x"]), &[], &rows, &anyone, "did:a", 0);
        assert_eq!(p.rotate, Some(1));
        assert_eq!(p.due.len(), 2, "the old salt's row and the adopted one");
        let held = plan(Some((0, 0)), &s(&["did:x"]), &s(&["did:a", "did:x"]), &s(&["did:x"]), &[], &rows, &anyone, "did:a", 0);
        assert_eq!(held.rotate, None, "the group still holds them: not yet");
        assert_eq!(plan(None, &s(&["did:x"]), &s(&["did:a"]), &[], &[], &rows, &anyone, "did:a", 0).rotate, None, "no salt: no rotation");
        // The put-out member, still in the group view, is never ranked: did:a's rank among [did:a] alone is 0.
        let only = plan(Some((0, 1)), &s(&["did:x"]), &s(&["did:a", "did:x"]), &s(&["did:x"]), &[], &rows, &anyone, "did:a", 0);
        assert!(only.due.iter().all(|d| d.rank == 0), "{only:?}");
        let x = plan(Some((0, 1)), &s(&["did:x"]), &s(&["did:a", "did:x"]), &s(&["did:x"]), &[], &rows, &anyone, "did:x", 0);
        assert!(x.due.is_empty(), "the put-out member takes no turn");
    }
}
