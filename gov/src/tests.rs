use super::*;

const O: &str = "did:o";
const A: &str = "did:a";
const M: &str = "did:m";
const X: &str = "did:x";

fn w() -> HashMap<String, String> {
    [("no", O), ("na", A), ("nm", M), ("nx", X)].iter().map(|(n, d)| (n.to_string(), d.to_string())).collect()
}
/// An act by `node`, made at `at`.
fn act(id: &str, node: &str, at: u64, v: Value) -> Row {
    let mut v = v;
    v["at"] = at.into();
    Row { id: id.into(), value: v.to_string(), writer: Some(node.into()) }
}
fn gov(rows: &[Row]) -> Gov {
    Gov::replay(rows, &w(), Some(O), 1e15)
}
use serde_json::json;

#[test]
fn the_owner_grants_and_only_the_owner() {
    let g = gov(&[
        act("1", "na", 1, json!({"act":"grant","did":M,"role":"admin"})),
        act("2", "no", 2, json!({"act":"grant","did":A,"role":"admin"})),
    ]);
    assert_eq!(g.role(A, true).as_deref(), Some("admin"));
    assert_eq!(g.role(M, true).as_deref(), Some("member"));
    assert_eq!(g.role(O, false).as_deref(), Some("owner"));
    assert_eq!(g.role(X, false), None);
    assert_eq!(g.counted.len(), 1);
}

#[test]
fn acts_count_in_time_order_not_row_order() {
    // The grant (at 1) comes before the admin's removal of M (at 2), whichever row is first.
    let rows = [act("b", "na", 2, json!({"act":"remove","did":M})), act("a", "no", 1, json!({"act":"grant","did":A,"role":"admin"}))];
    let g = gov(&rows);
    assert_eq!(g.role(M, true), None);
    assert!(!g.roster.contains(M));
    // Without the grant first, a member cannot remove a member.
    let g = gov(&[act("b", "na", 2, json!({"act":"remove","did":M}))]);
    assert_eq!(g.role(M, true).as_deref(), Some("member"));
}

#[test]
fn an_invite_admits_while_in_force_and_counts_its_uses() {
    let g = gov(&[
        act("1", "na", 1, json!({"act":"invite","code":"c1","expires":100,"uses":1})),
        act("2", "na", 2, json!({"act":"admitted","code":"c1","did":X})),
        act("3", "na", 3, json!({"act":"admitted","code":"c1","did":"did:y"})),
        act("4", "na", 200, json!({"act":"invite","code":"c2","expires":150})),
    ]);
    assert!(g.roster.contains(X));
    assert!(!g.roster.contains("did:y"), "one use only");
    assert_eq!(g.invites["c1"].admitted, vec![X.to_string()]);
    // c1 used up (whatever the time); c2 in force until it expires.
    assert_eq!(g.live_invites(50.0).iter().map(|i| i.code.as_str()).collect::<Vec<_>>(), vec!["c2"]);
    assert!(g.live_invites(150.0).is_empty(), "c2 expired");
}

#[test]
fn a_revoked_invite_admits_nobody_and_only_its_maker_or_a_moderator_revokes() {
    let g = gov(&[
        act("1", "na", 1, json!({"act":"invite","code":"c"})),
        act("2", "nm", 2, json!({"act":"revoke-invite","code":"c"})),
        act("3", "na", 3, json!({"act":"admitted","code":"c","did":X})),
    ]);
    assert!(g.roster.contains(X), "M is neither the maker nor a moderator");
    let g = gov(&[
        act("1", "na", 1, json!({"act":"invite","code":"c"})),
        act("2", "no", 2, json!({"act":"revoke-invite","code":"c"})),
        act("3", "na", 3, json!({"act":"admitted","code":"c","did":X})),
    ]);
    assert!(!g.roster.contains(X));
}

#[test]
fn open_joins_count_only_while_the_space_is_open() {
    let open = act("1", "no", 1, json!({"act":"policy","path":"","action":"join","who":"anyone"}));
    let join = act("2", "na", 2, json!({"act":"admitted","code":"open","did":X}));
    assert!(!gov(&[join.clone()]).roster.contains(X));
    let g = gov(&[open, join]);
    assert!(g.roster.contains(X));
    assert!(g.is_public());
}

#[test]
fn a_ban_keeps_them_out_until_unbanned() {
    let g = gov(&[
        act("1", "no", 1, json!({"act":"ban","did":X})),
        act("2", "na", 2, json!({"act":"added","did":X})),
    ]);
    assert!(!g.roster.contains(X) && g.bans.contains(X));
    let g = gov(&[
        act("1", "no", 1, json!({"act":"ban","did":X})),
        act("2", "no", 2, json!({"act":"unban","did":X})),
        act("3", "na", 3, json!({"act":"added","did":X})),
    ]);
    assert!(g.roster.contains(X) && !g.bans.contains(X));
}

#[test]
fn policies_inherit_along_the_path_and_are_judged_as_they_were_then() {
    let g = gov(&[
        act("1", "no", 10, json!({"act":"policy","path":"board","action":"post","who":"admins"})),
        act("2", "no", 20, json!({"act":"policy","path":"board/p/1","action":"post","who":"anyone"})),
        act("3", "no", 30, json!({"act":"policy","path":"board","action":"post","who":"inherit"})),
    ]);
    assert_eq!(g.effective("board/p/1", "post", f64::INFINITY), "anyone");
    assert_eq!(g.effective("board/p/2", "post", 15.0), "admins");
    assert_eq!(g.effective("board/p/2", "post", f64::INFINITY), "members", "inherit drops the override");
    assert_eq!(g.effective("board/p/2", "post", 5.0), "members");
}

#[test]
fn inviting_follows_the_invite_policy_as_it_was_then() {
    let g = gov(&[
        act("1", "no", 1, json!({"act":"policy","path":"","action":"invite","who":"admins"})),
        act("2", "nm", 2, json!({"act":"invite","code":"m"})),
        act("3", "na", 0, json!({"act":"invite","code":"early"})),
    ]);
    assert!(!g.invites.contains_key("m"), "members may not invite after the policy");
    assert!(g.invites.contains_key("early"), "before the policy they could");
}

#[test]
fn legacy_settings_read_as_policies_their_defaults_as_inherit() {
    let g = gov(&[
        act("1", "no", 1, json!({"act":"config","app":"board","key":"read","value":"public"})),
        act("2", "no", 2, json!({"act":"config","app":"chat","key":"post","value":"everyone"})),
    ]);
    assert_eq!(g.effective("board", "read", f64::INFINITY), "anyone");
    assert_eq!(g.policy_at("chat", "post", f64::INFINITY), None);
    assert_eq!(g.configs["board/read"], json!("public"));
    // Reading in public is the owner's.
    let g = gov(&[act("1", "no", 1, json!({"act":"grant","did":A,"role":"admin"})), act("2", "na", 2, json!({"act":"config","app":"board","key":"read","value":"public"}))]);
    assert!(!g.configs.contains_key("board/read"));
}

#[test]
fn rows_of_an_unknown_node_and_of_the_removed_do_not_count_and_removals_name_their_nodes() {
    let g = gov(&[
        act("1", "nz", 1, json!({"act":"app","app":"chat","on":true})),
        act("2", "no", 2, json!({"act":"remove","did":"did:gone","nodes":["ngone"]})),
        act("3", "ngone", 3, json!({"act":"invite","code":"late"})),
    ]);
    assert!(g.apps.is_empty());
    assert_eq!(g.learned, vec![("ngone".to_string(), "did:gone".to_string())]);
    assert!(!g.invites.contains_key("late"), "a removed member's act does not count");
}

#[test]
fn transfer_hands_the_space_on() {
    let g = gov(&[act("1", "no", 1, json!({"act":"transfer","did":A})), act("2", "no", 2, json!({"act":"grant","did":M,"role":"admin"}))]);
    assert_eq!(g.owner.as_deref(), Some(A));
    assert_eq!(g.role(O, true).as_deref(), Some("admin"));
    assert_eq!(g.role(M, true).as_deref(), Some("member"), "the old owner grants nothing");
}

#[test]
fn a_member_leaves_and_is_gone_until_added_back() {
    let g = gov(&[act("1", "nm", 1, json!({"act":"leave"}))]);
    assert_eq!(g.role(M, true), None, "left");
    assert!(g.gone.contains(M));
    assert_eq!(g.counted.len(), 1);
    // Only oneself; and not the owner (the space is handed on first).
    let g = gov(&[act("1", "na", 1, json!({"act":"leave","did":M})), act("2", "no", 2, json!({"act":"leave"}))]);
    assert!(g.gone.is_empty());
    assert_eq!(g.role(O, false).as_deref(), Some("owner"));
    // Added back: no longer gone.
    let g = gov(&[act("1", "nm", 1, json!({"act":"leave"})), act("2", "no", 2, json!({"act":"added","did":M}))]);
    assert!(!g.gone.contains(M));
    assert_eq!(g.role(M, true).as_deref(), Some("member"));
    // Removed and banned are gone too.
    let g = gov(&[act("1", "no", 1, json!({"act":"remove","did":M})), act("2", "no", 2, json!({"act":"ban","did":X}))]);
    assert_eq!(g.gone.iter().cloned().collect::<Vec<_>>(), vec![M.to_string(), X.to_string()]);
}

#[test]
fn a_leave_and_a_ban_name_their_nodes_so_their_rows_stay_theirs() {
    let rows = [
        act("1", "nm2", 1, json!({"act":"leave","did":M,"nodes":["nm2","nm3"]})),
        act("2", "no", 2, json!({"act":"ban","did":X,"nodes":["nx2"]})),
    ];
    let g = gov(&rows);
    let learned: HashMap<_, _> = g.learned.iter().cloned().collect();
    assert_eq!(learned.get("nm3").map(String::as_str), Some(M), "a leaver out of the group: its nodes from its own act");
    assert!(g.gone.contains(M));
    // Naming nodes it was not written by: nothing learned (and not counted: its writer is nobody's).
    let g = gov(&[act("1", "nz", 1, json!({"act":"leave","did":M,"nodes":["nq"]}))]);
    assert!(g.learned.is_empty() && g.gone.is_empty());
    assert_eq!(learned.get("nx2").map(String::as_str), Some(X));
}
