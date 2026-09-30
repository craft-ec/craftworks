//! GOVERNANCE: who holds what authority IN A SPACE, from its ACTS (the space's `acts` and `pub-acts` tables). Every
//! reader replays the acts in one order (when made, then id) from the owner, keeping an act only if its signer's role
//! allowed it at that point: the same answer on every node. The signer of an act is its row's WRITER (a node key),
//! whose DID the caller knows (the space's group and the members' cards); a `remove` act names the removed member's
//! nodes, so their rows stay theirs.
//!
//! ACCESS is POLICIES `{ path, action, who }`, INHERITED along the path (`board/p/<id>` → `board` → the space "") —
//! the most specific wins, else the default (members) — and TIME-AWARE: judged as they were when something was made.
//! Pure: no clock (a caller passes `now`), no network.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const APPS: [&str; 6] = ["chat", "board", "notes", "drive", "videos", "subtitles"];
pub const ACTIONS: [&str; 7] = ["read", "post", "comment", "vote", "edit", "join", "invite"];
pub const WHO: [&str; 6] = ["anyone", "members", "admins", "owner", "nobody", "inherit"];

/// What a role may do (inviting is the space's `invite` policy, not a role's: see [`Gov::may_invite`]).
pub fn can_role(role: Option<&str>, what: &str) -> bool {
    let set: &[&str] = match role {
        Some("owner") => &["post", "invite", "channels", "moderate", "remove", "grant", "apps"],
        Some("admin") => &["post", "invite", "channels", "moderate", "remove", "apps"],
        Some("member") => &["post", "invite"],
        _ => &[],
    };
    set.contains(&what)
}
pub fn rank(role: Option<&str>) -> u8 {
    match role {
        Some("owner") => 3,
        Some("admin") => 2,
        Some("member") => 1,
        _ => 0,
    }
}
/// Does a role pass a policy's `who` (anyone, here, is any member: only members act in a space).
pub fn passes(who: &str, role: Option<&str>) -> bool {
    match (who, role) {
        ("nobody", _) | (_, None) => false,
        ("owner", r) => r == Some("owner"),
        ("admins", r) => rank(r) >= 2,
        _ => true,
    }
}

#[derive(Clone)]
/// An act's row: its key (the act's id), its value (the act, JSON), its writer (a node key, hex).
pub struct Row {
    pub id: String,
    pub value: String,
    pub writer: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Invite {
    pub code: String,
    pub by: String,
    pub at: f64,
    pub expires: f64,
    pub uses: f64,
    pub revoked: bool,
    pub admitted: Vec<String>,
}
impl Invite {
    /// In force at `at`: not revoked, not expired, uses left (0: no limit).
    pub fn live(&self, at: f64) -> bool {
        !self.revoked && (self.expires == 0.0 || at < self.expires) && (self.uses == 0.0 || (self.admitted.len() as f64) < self.uses)
    }
}

/// A space's governance as its acts have it.
#[derive(Default)]
pub struct Gov {
    pub owner: Option<String>,
    /// Roles set by acts: `Some(role)`, or `None` for one removed (or banned).
    pub roles: HashMap<String, Option<String>>,
    pub invites: BTreeMap<String, Invite>,
    /// Insertion order kept: `invites()` lists them as made.
    invite_order: Vec<String>,
    pub apps: HashMap<String, bool>,
    pub configs: HashMap<String, Value>,
    /// `path|action` → [(at, who)], in the order they counted; `None`: inherit.
    history: HashMap<String, Vec<(f64, Option<String>)>>,
    /// The DIDs the acts name as members (the owner, added, admitted, rostered), less the removed.
    pub roster: BTreeSet<String>,
    pub bans: BTreeSet<String>,
    /// Out of the space by the acts — removed, banned or left — and not added back since: their nodes are taken out of
    /// the group by whoever may remove (a leaver cannot commit their own removal).
    pub gone: BTreeSet<String>,
    /// The acts that counted, in order: each with `id` and `by` set.
    pub counted: Vec<Value>,
    /// Node → DID learned from `remove` acts (their nodes' rows stay theirs).
    pub learned: Vec<(String, String)>,
}

/// JavaScript's `Number(v) || 0` for what acts carry.
fn num(v: Option<&Value>) -> f64 {
    let n = match v {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() { 0.0 } else { t.parse().unwrap_or(0.0) }
        }
        Some(Value::Bool(true)) => 1.0,
        _ => 0.0,
    };
    if n.is_nan() { 0.0 } else { n }
}
/// An act's time where a missing one means "any time" (a default of the caller's).
fn at_or(v: &Value, dflt: f64) -> f64 {
    match v.get("at") {
        None | Some(Value::Null) => dflt,
        x => num(x),
    }
}
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        _ => true,
    }
}
fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}
/// A non-empty string field.
fn some_s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    s(v, k).filter(|x| !x.is_empty())
}
fn js_len(x: &str) -> usize {
    x.encode_utf16().count()
}
fn parent(path: &str) -> Vec<String> {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    (0..=parts.len()).rev().map(|i| parts[..i].join("/")).collect()
}

/// The settings from before policies (`config` acts), as policies — their defaults as INHERIT, never an override.
fn old_policy(app: &str, key: &str, value: &Value) -> Option<(&'static str, &'static str, Option<String>)> {
    let v = value.as_str();
    let when = |want: &str, who: &str| if v == Some(want) { Some(who.to_string()) } else { None };
    Some(match (app, key) {
        ("chat", "post") => ("chat", "post", when("admins", "admins")),
        ("board", "post") => ("board", "post", when("admins", "admins")),
        ("board", "read") => ("board", "read", when("public", "anyone")),
        ("notes", "edit") => ("notes", "edit", when("admins", "admins")),
        ("space", "join") => ("", "join", when("open", "anyone")),
        _ => return None,
    })
}

impl Gov {
    /// A policy set at exactly this path, as it was at `at` (`None`: none, or inherit).
    pub fn policy_at(&self, path: &str, action: &str, at: f64) -> Option<&str> {
        self.history.get(&format!("{path}|{action}"))?.iter().filter(|(t, _)| *t <= at).last()?.1.as_deref()
    }
    /// The EFFECTIVE policy: walked up the path to the space; else members.
    pub fn effective(&self, path: &str, action: &str, at: f64) -> &str {
        parent(path).iter().find_map(|p| self.policy_at(p, action, at)).unwrap_or("members")
    }
    /// Inviting (codes, adding by id, letting askers in): the space's `invite` policy as it was then.
    pub fn may_invite(&self, role: Option<&str>, at: f64) -> bool {
        passes(self.effective("", "invite", at), role)
    }
    /// The paths that have a policy for an action (any time).
    pub fn paths_with(&self, action: &str) -> Vec<String> {
        let tail = format!("|{action}");
        self.history.keys().filter_map(|k| k.strip_suffix(&tail).map(str::to_string)).collect()
    }
    /// The invite codes in force at `now`, as made.
    pub fn live_invites(&self, now: f64) -> Vec<&Invite> {
        self.invite_order.iter().filter_map(|c| self.invites.get(c)).filter(|i| i.live(now)).collect()
    }
    /// PUBLIC: joining is open to anyone, or an app reads in public.
    pub fn is_public(&self) -> bool {
        self.policy_at("", "join", f64::INFINITY) == Some("anyone")
            || self.paths_with("read").iter().any(|p| self.policy_at(p, "read", f64::INFINITY) == Some("anyone"))
    }
    /// A person's role: by the acts (`None` inside: removed), else a member if the group has them.
    pub fn role(&self, did: &str, in_group: bool) -> Option<String> {
        match self.roles.get(did) {
            Some(r) => r.clone(),
            None => in_group.then(|| "member".to_string()),
        }
    }
    fn set_policy(&mut self, path: &str, action: &str, who: Option<String>, at: f64) {
        self.history.entry(format!("{path}|{action}")).or_default().push((at, who));
    }

    /// THE REPLAY. `first`: the space's first owner (its id proves it). `writers`: node key → DID. `now`: the time an
    /// admission carrying none is judged at (the caller's clock).
    pub fn replay(rows: &[Row], writers: &HashMap<String, String>, first: Option<&str>, now: f64) -> Gov {
        let mut g = Gov { owner: first.map(str::to_string), ..Gov::default() };
        let parsed: Vec<(&Row, Value)> = rows.iter().filter_map(|r| Some((r, serde_json::from_str::<Value>(&r.value).ok().filter(Value::is_object)?))).collect();
        // Removed, banned and leaving members' nodes, as their act names them: theirs (what they wrote still counts as
        // theirs once they are out of the group). A leave names its own writer.
        let mut writers = writers.clone();
        for (row, v) in &parsed {
            let act = s(v, "act");
            let did = match act {
                Some("remove" | "ban") => some_s(v, "did").map(str::to_string),
                // Self-certifying: written by one of the nodes it names (a node already known stays whose it is).
                Some("leave") => some_s(v, "did")
                    .filter(|_| row.writer.as_deref().is_some_and(|w| v.get("nodes").and_then(Value::as_array).is_some_and(|ns| ns.iter().any(|n| n.as_str() == Some(w)))))
                    .map(str::to_string),
                _ => None,
            };
            if let (Some(did), Some(nodes)) = (did.as_deref(), v.get("nodes").and_then(Value::as_array)) {
                for n in nodes.iter().filter_map(Value::as_str) {
                    if !writers.contains_key(n) {
                        writers.insert(n.to_string(), did.to_string());
                        g.learned.push((n.to_string(), did.to_string()));
                    }
                }
            }
        }
        let mut acts: Vec<Value> = parsed
            .into_iter()
            .map(|(r, mut v)| {
                let by = r.writer.as_ref().and_then(|w| writers.get(w.get(..64).unwrap_or(w))).cloned();
                v["id"] = Value::String(r.id.clone());
                v["by"] = by.map(Value::String).unwrap_or(Value::Null);
                v
            })
            .collect();
        acts.sort_by(|a, b| num(a.get("at")).total_cmp(&num(b.get("at"))).then_with(|| s(a, "id").cmp(&s(b, "id"))));

        if let Some(o) = first {
            g.roles.insert(o.to_string(), Some("owner".into()));
            g.roster.insert(o.to_string());
        }
        let mut removed: BTreeSet<String> = BTreeSet::new();
        for a in acts {
            let Some(by) = s(&a, "by").map(str::to_string) else { continue };
            if removed.contains(&by) {
                continue;
            }
            let role_at = |g: &Gov, removed: &BTreeSet<String>, d: &str| -> Option<String> {
                if removed.contains(d) { None } else { Some(g.roles.get(d).cloned().flatten().unwrap_or_else(|| "member".into())) }
            };
            let r = role_at(&g, &removed, &by);
            let r = r.as_deref();
            let act = s(&a, "act").unwrap_or("");
            let did = some_s(&a, "did");
            let code = some_s(&a, "code");
            let inv_exists = code.is_some_and(|c| g.invites.contains_key(c));
            let at_inf = at_or(&a, f64::INFINITY);
            let invite_ok = g.may_invite(r, at_inf);
            let banned_did = did.is_some_and(|d| g.bans.contains(d));
            let ok = match act {
                "grant" => r == Some("owner") && did.is_some() && did != g.owner.as_deref() && matches!(s(&a, "role"), Some("admin" | "member")),
                "remove" | "ban" => {
                    did.is_some_and(|d| d != by && rank(r) > rank(role_at(&g, &removed, d).as_deref().or(Some("member")))) && can_role(r, "remove")
                }
                "unban" => can_role(r, "remove") && banned_did,
                // LEAVING: a member's own act (the owner hands the space on first).
                "leave" => r.is_some() && r != Some("owner") && did.is_none_or(|d| d == by),
                "added" | "member" => invite_ok && did.is_some() && !banned_did,
                "hide" => can_role(r, "moderate"),
                "transfer" => r == Some("owner") && did.is_some_and(|d| d != by && !removed.contains(d)),
                "invite" => invite_ok && code.is_some() && !inv_exists,
                "revoke-invite" => inv_exists && (g.invites[code.unwrap()].by == by || can_role(r, "moderate")),
                "admitted" => {
                    // In force when admitted (an act with no time: now).
                    let by_code = inv_exists && g.invites[code.unwrap()].live(at_or(&a, now));
                    invite_ok && did.is_some() && !banned_did && (by_code || (code == Some("open") && g.policy_at("", "join", f64::INFINITY) == Some("anyone")))
                }
                "app" => can_role(r, "apps") && s(&a, "app").is_some_and(|x| APPS.contains(&x)),
                "config" => {
                    can_role(r, "apps")
                        && s(&a, "app").is_some_and(|x| APPS.contains(&x) || x == "space")
                        && s(&a, "key").is_some_and(|k| js_len(k) <= 32 && (k != "read" || r == Some("owner")))
                }
                "policy" => {
                    can_role(r, "apps")
                        && s(&a, "path").is_some_and(|p| js_len(p) <= 120)
                        && s(&a, "action").is_some_and(|x| ACTIONS.contains(&x))
                        && s(&a, "who").is_some_and(|x| WHO.contains(&x))
                        && (s(&a, "action") != Some("read") || r == Some("owner"))
                }
                _ => false,
            };
            if !ok {
                continue;
            }
            let at0 = num(a.get("at"));
            match act {
                "grant" => {
                    g.roles.insert(did.unwrap().into(), s(&a, "role").map(str::to_string));
                }
                "remove" | "ban" => {
                    let d = did.unwrap().to_string();
                    removed.insert(d.clone());
                    g.roles.remove(&d);
                    if act == "ban" {
                        g.bans.insert(d);
                    }
                }
                "unban" => {
                    g.bans.remove(did.unwrap());
                }
                "leave" => {
                    removed.insert(by.clone());
                    g.roles.remove(&by);
                }
                "transfer" => {
                    if let Some(o) = g.owner.clone() {
                        g.roles.insert(o, Some("admin".into()));
                    }
                    g.owner = did.map(str::to_string);
                    g.roles.insert(did.unwrap().into(), Some("owner".into()));
                }
                "invite" => {
                    let c = code.unwrap().to_string();
                    g.invite_order.push(c.clone());
                    g.invites.insert(c.clone(), Invite { code: c, by: by.clone(), at: at0, expires: num(a.get("expires")), uses: num(a.get("uses")), revoked: false, admitted: vec![] });
                }
                "revoke-invite" => {
                    // As JavaScript had it: revoked by a truthy time.
                    g.invites.get_mut(code.unwrap()).unwrap().revoked = truthy(a.get("at"));
                }
                "app" => {
                    g.apps.insert(s(&a, "app").unwrap().into(), truthy(a.get("on")));
                }
                "config" => {
                    let (app, key) = (s(&a, "app").unwrap(), s(&a, "key").unwrap());
                    let value = a.get("value").cloned().unwrap_or(Value::Null);
                    if let Some((path, action, who)) = old_policy(app, key, &value) {
                        g.set_policy(path, action, who, at0);
                    }
                    g.configs.insert(format!("{app}/{key}"), value);
                }
                "policy" => {
                    let who = s(&a, "who").filter(|w| *w != "inherit").map(str::to_string);
                    g.set_policy(s(&a, "path").unwrap(), s(&a, "action").unwrap(), who, at0);
                }
                _ => {}
            }
            if act == "admitted" {
                if let Some(inv) = code.and_then(|c| g.invites.get_mut(c)) {
                    let d = did.unwrap().to_string();
                    if !inv.admitted.contains(&d) {
                        inv.admitted.push(d);
                    }
                }
            }
            if matches!(act, "added" | "admitted" | "member") {
                removed.remove(did.unwrap());
                g.roster.insert(did.unwrap().into());
            }
            g.counted.push(a);
        }
        for d in &removed {
            g.roles.insert(d.clone(), None);
            g.roster.remove(d);
        }
        g.gone = removed;
        g
    }
}

#[cfg(test)]
mod tests;
