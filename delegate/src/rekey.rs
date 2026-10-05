//! WHAT TO RE-KEY with no page open (R4c) — `file-keys.js`'s decisions, from a space read (`table::Reading`):
//!
//! - A REMOVAL is in (a counted `remove`, `ban` or `leave` act — the space's acts replayed by `gov`, the one
//!   implementation) and the group no longer holds who it removed: a NEW SALT (`salt/<n+1>`, the `salt` row naming it,
//!   the old one kept for burning). Every row keyed under an older salt is then due.
//! - A row keyed PUBLIC whose app no longer reads in public: due (sealed for the members).
//! - A row ADOPTED from another space (`n` -2): due (copied under this space's salt).
//!
//! WHO DOES A ROW: every member ranks the members for it the same way (FNV-1a of `id|did`, as pages do) — the first
//! does it now; the k-th once it has not moved for k × TAKEOVER (a member away). Rows only move forward, so a member
//! on an older view never undoes one, and two doing one make the same pieces (the key is derived).

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

use crate::table::Reading;

pub const TAKEOVER_MS: u64 = 2 * 60 * 1000;

/// A space's SALT (`files`' `salt` row): the secret, how many it has had, the removals it was made after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Salt {
    pub s: [u8; 32],
    pub n: i64,
    pub removals: usize,
}

/// A key row (`k/<id>` of `files`): what a re-key reads and changes.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyRow {
    pub id: String,
    pub v: serde_json::Map<String, Value>,
}
impl KeyRow {
    pub fn n(&self) -> i64 {
        self.v.get("n").and_then(Value::as_i64).unwrap_or(-2)
    }
    pub fn at(&self) -> u64 {
        self.v.get("at").and_then(Value::as_u64).unwrap_or(0)
    }
    pub fn app(&self) -> &str {
        self.v.get("app").and_then(Value::as_str).unwrap_or("drive")
    }
    pub fn public(&self) -> bool {
        self.v.get("pub").and_then(Value::as_bool).unwrap_or(false)
    }
}

/// One row due, and what it is due to become (public, or sealed under the salt now).
#[derive(Debug, Clone, PartialEq)]
pub struct Due {
    pub row: KeyRow,
    pub public: bool,
    /// This member's turn for it: now (0), or after `wait` ms more — from when it last MOVED (its row, its progress);
    /// a caller that knows when it first saw it due floors that too (`wait_from`).
    pub wait: u64,
    pub rank: usize,
    pub moved: u64,
}
impl Due {
    /// The wait from `seen` too (when this member first saw it due: a row due since a removal carries an old time).
    pub fn wait_from(&self, seen: u64, now_ms: u64) -> u64 {
        (self.rank as u64 * TAKEOVER_MS).saturating_sub(now_ms.saturating_sub(self.moved.max(seen)))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// A new salt is due: after the removals counted (the `removals` the new one is made after).
    pub rotate: Option<usize>,
    pub due: Vec<Due>,
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let v: Option<Vec<u8>> = (0..32).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()).collect();
    v?.try_into().ok()
}

/// `files`' salt row (the first shape: the secret's hex alone).
pub fn salt(files: &BTreeMap<Vec<u8>, craftworks_feed::Row>) -> Option<Salt> {
    let v = String::from_utf8(files.get(b"salt".as_slice())?.value.clone()).ok()?;
    if let Some(s) = unhex32(&v) {
        return Some(Salt { s, n: 0, removals: 0 });
    }
    let o: Value = serde_json::from_str(&v).ok()?;
    Some(Salt { s: unhex32(o["s"].as_str()?)?, n: o["n"].as_i64().unwrap_or(0), removals: o["removals"].as_u64().unwrap_or(0) as usize })
}

pub fn key_rows(files: &BTreeMap<Vec<u8>, craftworks_feed::Row>) -> Vec<KeyRow> {
    files
        .iter()
        .filter_map(|(k, r)| {
            let id = std::str::from_utf8(k).ok()?.strip_prefix("k/")?.to_string();
            let v: Value = serde_json::from_slice(&r.value).ok()?;
            let o = v.as_object()?.clone();
            (o.contains_key("key") && o.contains_key("root")).then_some(KeyRow { id, v: o })
        })
        .collect()
}

/// The space's acts REPLAYED (`gov`): its sealed `acts` and public `pub-acts` (the same act in both counts once, by
/// its key), each act's writer → DID from the roster (`writers`: writer key → DID).
pub fn governance(r: &Reading, writers: &[([u8; 32], String)], owner: &str, now_ms: u64) -> craftworks_gov::Gov {
    let mut rows: BTreeMap<Vec<u8>, craftworks_feed::Row> = r.rows("pub-acts");
    for (k, v) in r.rows("acts") {
        rows.insert(k, v);
    }
    let gov_rows: Vec<craftworks_gov::Row> = rows
        .into_iter()
        .map(|(k, row)| craftworks_gov::Row {
            id: String::from_utf8_lossy(&k).into_owned(),
            value: String::from_utf8_lossy(&row.value).into_owned(),
            writer: Some(crate::table::hex(&row.id.0)),
        })
        .collect();
    let map: HashMap<String, String> = writers.iter().map(|(w, d)| (crate::table::hex(w), d.clone())).collect();
    craftworks_gov::Gov::replay(&gov_rows, &map, Some(owner), now_ms as f64)
}

/// FNV-1a of a text (its UTF-16 code units, as a page's `hash` — ids and DIDs are ASCII).
fn fnv(s: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for u in s.encode_utf16() {
        h = (h ^ u as u32).wrapping_mul(16777619);
    }
    h
}
/// This member's place among `dids` for row `id`: 0 does it now.
pub fn rank(id: &str, dids: &[String], me: &str) -> usize {
    let mut all: Vec<&String> = dids.iter().collect();
    all.sort();
    all.dedup();
    all.sort_by(|a, b| fnv(&format!("{id}|{b}")).cmp(&fnv(&format!("{id}|{a}"))).then(a.cmp(b)));
    all.iter().position(|d| d.as_str() == me).unwrap_or(0)
}

/// THE PLAN for a space now: whether a new salt is due, and the rows due with this member's turn for each.
/// `members`: the group's DIDs now (who a removal must have taken out); `me`: this member's DID.
pub fn plan(r: &Reading, g: &craftworks_gov::Gov, members: &[String], me: &str, now_ms: u64) -> Plan {
    let files = r.rows("files");
    let st = salt(&files);
    let removals: Vec<&Value> = g.counted.iter().filter(|a| matches!(a["act"].as_str(), Some("remove" | "ban" | "leave"))).collect();
    let rotate = st.as_ref().and_then(|s| {
        if removals.len() <= s.removals {
            return None;
        }
        // The group must have moved without them first (a removal in the acts, its commit not yet seen: the next pass).
        let gone: Vec<&str> = removals[s.removals..].iter().filter_map(|a| a["did"].as_str().or_else(|| a["by"].as_str())).collect();
        (!members.iter().any(|m| gone.contains(&m.as_str()))).then_some(removals.len())
    });
    let n_now = st.as_ref().map(|s| s.n).unwrap_or(0) + i64::from(rotate.is_some());
    let reads_anyone = |app: &str| g.effective(app, "read", f64::INFINITY) == "anyone";
    let mut due = Vec::new();
    for row in key_rows(&files) {
        let public = row.public() && reads_anyone(row.app());
        let n = row.n();
        let is_due = n == -2 || (if n == -1 { !public } else { n < n_now });
        if !is_due {
            continue;
        }
        let public = if n == -2 { public } else { false };
        let k = rank(&row.id, members, me) as u64;
        let progress = files
            .get(format!("p/{}", row.id).as_bytes())
            .and_then(|p| serde_json::from_slice::<Value>(&p.value).ok())
            .and_then(|v| v["at"].as_u64())
            .unwrap_or(0);
        let moved = row.at().max(progress);
        let wait = (k * TAKEOVER_MS).saturating_sub(now_ms.saturating_sub(moved));
        due.push(Due { row, public, wait, rank: k as usize, moved });
    }
    Plan { rotate, due }
}

/// The ROWS a new salt writes: the old one kept (`salt/<n>`, for burning — when it is not kept already), the new one
/// (`salt/<n+1>`), and `salt`.
pub fn rotation(files: &BTreeMap<Vec<u8>, craftworks_feed::Row>, was: &Salt, new: [u8; 32], removals: usize) -> Vec<(Vec<u8>, Option<Vec<u8>>)> {
    let hex = crate::table::hex;
    let old = format!("salt/{}", was.n).into_bytes();
    let mut out = Vec::new();
    if !files.contains_key(&old) {
        out.push((old, Some(hex(&was.s).into_bytes())));
    }
    out.push((format!("salt/{}", was.n + 1).into_bytes(), Some(hex(&new).into_bytes())));
    out.push((b"salt".to_vec(), Some(serde_json::json!({ "s": hex(&new), "n": was.n + 1, "removals": removals }).to_string().into_bytes())));
    out
}
