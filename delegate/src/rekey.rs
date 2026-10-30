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
    /// What the space's policy is asked about for it (`rules::rekey::subject`: its item's domain, else its app).
    pub fn subject(&self) -> String {
        craftworks_rules::rekey::subject(self.v.get("dom").and_then(Value::as_str), self.v.get("app").and_then(Value::as_str))
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
    pub moved: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// A new salt is due: after the removals counted (the `removals` the new one is made after).
    pub rotate: Option<usize>,
    pub due: Vec<Due>,
}

/// `files`' salt row (the first shape: the secret's hex alone).
pub fn salt(files: &BTreeMap<Vec<u8>, craftworks_feed::Row>) -> Option<Salt> {
    // (`rules::rekey::parse_salt`, the page's too.)
    let (s, n, removals) = craftworks_rules::rekey::parse_salt(std::str::from_utf8(&files.get(b"salt".as_slice())?.value).ok()?)?;
    Some(Salt { s, n, removals })
}

pub fn key_rows(files: &BTreeMap<Vec<u8>, craftworks_feed::Row>) -> Vec<KeyRow> {
    files
        .iter()
        // (`rules::rekey::parse_key_row`, the page's too.)
        .filter_map(|(k, r)| {
            let o = craftworks_rules::rekey::parse_key_row(std::str::from_utf8(k).ok()?, std::str::from_utf8(&r.value).ok()?)?;
            Some(KeyRow { id: o["id"].as_str()?.to_string(), v: o })
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


/// THE PLAN for a space now: whether a new salt is due, and the rows due with this member's turn for each.
/// `members`: the group's DIDs now (who a removal must have taken out); `me`: this member's DID.
pub fn plan(r: &Reading, g: &craftworks_gov::Gov, members: &[String], me: &str, now_ms: u64) -> Plan {
    // THE ONE PLAN (`rules::rekey::plan`, the page's too): here only what it reads — the salt, the removals, the rows.
    let files = r.rows("files");
    let out: Vec<String> = g.out_acts().iter().filter_map(|a| a["did"].as_str().or_else(|| a["by"].as_str()).map(str::to_string)).collect();
    let rows = key_rows(&files);
    let progress = |id: &str| files.get(format!("p/{id}").as_bytes()).and_then(|p| serde_json::from_slice::<Value>(&p.value).ok()).and_then(|v| v["at"].as_u64()).unwrap_or(0);
    let input: Vec<craftworks_rules::rekey::Row> = rows.iter().map(|k| craftworks_rules::rekey::Row { id: k.id.clone(), n: k.n(), public: k.public(), app: k.subject(), moved: k.at().max(progress(&k.id)) }).collect();
    let reads_anyone = |app: &str| g.effective(app, "read", f64::INFINITY) == "anyone";
    let (gone, bans): (Vec<String>, Vec<String>) = (g.gone.iter().cloned().collect(), g.bans.iter().cloned().collect());
    let p = craftworks_rules::rekey::plan(salt(&files).map(|s| (s.n, s.removals)), &out, members, &gone, &bans, &input, &reads_anyone, me, now_ms);
    let due = p.due.into_iter().filter_map(|d| Some(Due { row: rows.iter().find(|k| k.id == d.id)?.clone(), public: d.public, moved: d.moved })).collect();
    Plan { rotate: p.rotate, due }
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
