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

// COMPOSED ROLES.
fn mods(perms: Value) -> Row {
    act("r1", "na", 10, json!({"act":"role","role":"mods","name":"Moderators","perms":perms}))
}
fn admin_a() -> Row {
    act("g", "no", 1, json!({"act":"grant","did":A,"role":"admin"}))
}

#[test]
fn an_admin_composes_a_role_and_its_holder_may_what_it_carries() {
    let g = gov(&[
        admin_a(),
        mods(json!(["moderate"])),
        act("as", "na", 11, json!({"act":"assign","did":M,"role":"mods"})),
        act("h1", "nm", 12, json!({"act":"hide","table":"t","item":"1"})),
        act("h2", "nx", 13, json!({"act":"hide","table":"t","item":"2"})),
    ]);
    assert_eq!(g.defined["mods"].name, "Moderators");
    assert!(g.may(M, Some("member"), "moderate"));
    assert!(!g.may(X, Some("member"), "moderate"));
    let hides: Vec<&str> = g.counted.iter().filter(|a| a["act"] == "hide").map(|a| a["item"].as_str().unwrap()).collect();
    assert_eq!(hides, ["1"]);
}

#[test]
fn a_member_composes_nothing_and_nobody_carries_more_than_they_hold() {
    let g = gov(&[
        act("r0", "nm", 5, json!({"act":"role","role":"x","name":"X","perms":["post"]})),
        admin_a(),
        // A role with "roles" and "moderate" only, given to M: M may not make or give one carrying "remove".
        act("r1", "na", 10, json!({"act":"role","role":"helpers","name":"Helpers","perms":["roles","moderate"]})),
        act("r2", "na", 11, json!({"act":"role","role":"bouncers","name":"Bouncers","perms":["remove"]})),
        act("a1", "na", 12, json!({"act":"assign","did":M,"role":"helpers"})),
        act("r3", "nm", 13, json!({"act":"role","role":"mine","name":"Mine","perms":["remove"]})),
        act("a2", "nm", 14, json!({"act":"assign","did":X,"role":"bouncers"})),
        act("r4", "nm", 15, json!({"act":"role","role":"ok","name":"Ok","perms":["moderate"]})),
        act("a3", "nm", 16, json!({"act":"assign","did":X,"role":"ok"})),
        // Nobody gives the owner a role.
        act("a4", "na", 17, json!({"act":"assign","did":O,"role":"helpers"})),
    ]);
    assert!(!g.defined.contains_key("x"));
    assert!(!g.defined.contains_key("mine"));
    assert!(g.defined.contains_key("ok"));
    assert!(!g.may(X, Some("member"), "remove"));
    assert!(g.may(X, Some("member"), "moderate"));
    assert!(g.held.get(O).is_none());
}

#[test]
fn a_policy_names_a_role_its_holders_and_admins_pass() {
    let g = gov(&[
        admin_a(),
        mods(json!(["moderate"])),
        act("as", "na", 11, json!({"act":"assign","did":M,"role":"mods"})),
        act("p", "na", 12, json!({"act":"policy","path":"chat/team","action":"post","who":"role:mods"})),
    ]);
    assert_eq!(g.effective("chat/team", "post", f64::INFINITY), "role:mods");
    assert!(g.passes_did("role:mods", M, Some("member")));
    assert!(g.passes_did("role:mods", A, Some("admin")));
    assert!(!g.passes_did("role:mods", X, Some("member")));
    assert!(!g.passes_did("role:mods", M, None));
}

#[test]
fn a_composed_remove_stands_above_members_and_below_admins() {
    let g = gov(&[
        admin_a(),
        act("r", "na", 10, json!({"act":"role","role":"b","name":"Bouncers","perms":["remove"]})),
        act("as", "na", 11, json!({"act":"assign","did":M,"role":"b"})),
        act("x1", "nm", 12, json!({"act":"remove","did":A})),
        act("x2", "nm", 13, json!({"act":"remove","did":X})),
    ]);
    assert_eq!(g.role(A, true).as_deref(), Some("admin"));
    assert_eq!(g.role(X, true), None);
}

#[test]
fn a_deleted_role_and_a_removed_member_hold_nothing() {
    let g = gov(&[
        admin_a(),
        mods(json!(["moderate"])),
        act("a1", "na", 11, json!({"act":"assign","did":M,"role":"mods"})),
        act("a2", "na", 12, json!({"act":"assign","did":X,"role":"mods"})),
        act("rm", "na", 13, json!({"act":"remove","did":X})),
        act("back", "na", 14, json!({"act":"added","did":X})),
        act("d", "na", 15, json!({"act":"role","role":"mods","on":false})),
    ]);
    assert!(!g.defined.contains_key("mods"));
    assert!(!g.may(M, Some("member"), "moderate"));
    assert!(!g.held.get(X).is_some_and(|h| h.contains("mods")));
}


/// APPS are named by their route — whatever the manifest lists, never a list here: every current app is taken, and
/// a name that is no app's (the control) is not.
#[test]
fn an_app_is_any_route_name_and_nothing_else() {
    let names = ["chat", "board", "note", "mail", "contact", "drive", "video", "audio", "caption"];
    let mut rows: Vec<Row> = names.iter().enumerate().map(|(i, n)| act(&format!("a{i}"), "no", i as u64 + 1, json!({"act":"app","app":n,"on":true}))).collect();
    rows.push(act("bad", "no", 99, json!({"act":"app","app":"Not An App!","on":true})));
    let g = gov(&rows);
    for n in names {
        assert_eq!(g.apps.get(n), Some(&true), "{n} added");
    }
    assert!(!g.apps.contains_key("Not An App!"), "a malformed name is refused");
}

/// A LIST of named people: a member makes it; `list:<id>` passes its people and its maker, nobody else — not even an
/// admin (the control: a role audience passes admins); and only its maker changes it.
#[test]
fn a_list_passes_its_people_and_maker_only() {
    let mk = |people: &[&str]| json!({"act":"list","list":"l1","people":people});
    let g = gov(&[
        act("1", "no", 1, json!({"act":"added","did":A})),
        act("2", "no", 2, json!({"act":"added","did":M})),
        act("3", "no", 3, json!({"act":"added","did":X})),
        act("4", "no", 4, json!({"act":"grant","did":A,"role":"admin"})),
        act("5", "nm", 5, mk(&[X])),
        // Another member changing M's list: refused.
        act("6", "na", 6, mk(&[A])),
    ]);
    let role = |d: &str| g.role(d, true);
    assert!(g.passes_did("list:l1", X, role(X).as_deref()), "listed");
    assert!(g.passes_did("list:l1", M, role(M).as_deref()), "its maker");
    assert!(!g.passes_did("list:l1", A, role(A).as_deref()), "an admin not listed does not pass a list");
    assert!(g.passes_did("admins", A, role(A).as_deref()), "control: the admin passes an admins audience");
    assert_eq!(g.lists["l1"].0, M, "the list is still its maker's");
}
