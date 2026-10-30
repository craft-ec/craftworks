//! FILE RE-KEY — THE PLAN (page and node, one implementation): whether a space's files need a NEW SALT (a removal is in
//! and the group has moved without them), and which key rows are DUE with this member's turn for each.
//! (10-19, inventory #7/#8: the page had its own `due`/`rotateIfRemoved` — it ranked members the acts had put out,
//! and planned a rotation where the node would not; now both call this.)

use crate::admission;

/// A key row of `files` (`k/<id>`) as the plan reads it: its salt number (`n`: -2 adopted, -1 public), whether it is
/// public, its app (domain), and when it last MOVED (its row's time, its progress — the later).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub n: i64,
    pub public: bool,
    /// What the space's policy is asked about for it: `subject(dom, app)`.
    pub app: String,
    pub moved: u64,
}

/// A row's SUBJECT (page and node, the one rule): the DOMAIN of the item holding it (`dom`, written where the item
/// placed it) — the vocabulary every item's own policy is asked in; a row from before names its app instead (`drive`:
/// a file's).
pub fn subject(dom: Option<&str>, app: Option<&str>) -> String {
    match dom.filter(|d| !d.is_empty()) {
        Some(d) => d.to_string(),
        None => match app.unwrap_or("drive") {
            "drive" => "file".to_string(),
            a => a.to_string(),
        },
    }
}

/// One row due: what it becomes (public, or sealed under the salt now), this member's turn, the wait from `moved`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Due {
    pub id: String,
    pub public: bool,
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
pub fn plan(salt: Option<(i64, usize)>, out: &[String], members: &[String], gone: &[String], banned: &[String], rows: &[Row], reads_anyone: &dyn Fn(&str) -> bool, me: &str, _now_ms: u64) -> Plan {
    let rotate = salt.and_then(|(_, removals)| (out.len() > removals && !members.iter().any(|m| out[removals..].contains(m))).then_some(out.len()));
    let n_now = salt.map(|(n, _)| n).unwrap_or(0) + i64::from(rotate.is_some());
    let ranked: Vec<String> = members.iter().filter(|m| !gone.contains(m) && !banned.contains(m)).cloned().collect();
    let mut due = Vec::new();
    for row in rows {
        let public = row.public && reads_anyone(&row.app);
        let is_due = row.n == -2 || (if row.n == -1 { !public } else { row.n < n_now });
        // (Only one who may — never one put out. NO TURNS: a re-key is the same pieces whoever makes it — key and secret
        // from the file and the salt — so each who sees a row due re-keys it AT ONCE; 10-29: a 2-minute takeover timer
        // held the node behind a member whose page was closed.)
        if admission::turn(&row.id, &ranked, me) < 0 {
            continue;
        }
        if is_due {
            due.push(Due { id: row.id.clone(), public: row.n == -2 && public, moved: row.moved });
        }
    }
    Plan { rotate, due }
}

/// A space's SALT row of `files` (page and node): the secret (64 hex — the first shape is that alone), how many salts
/// it has had (`n`), the removals it was made after. None: not a salt.
pub fn parse_salt(v: &str) -> Option<([u8; 32], i64, usize)> {
    let hex32 = |s: &str| craftworks_identity::unhex32(s);
    if let Some(s) = hex32(v) {
        return Some((s, 0, 0));
    }
    let o: serde_json::Value = serde_json::from_str(v).ok()?;
    Some((hex32(o["s"].as_str()?)?, o["n"].as_i64().unwrap_or(0), o["removals"].as_u64().unwrap_or(0) as usize))
}

/// A KEY ROW WRITTEN (page and node, `parse_key_row`'s other half): `k/<id>` → its fields, noted `at` (ms).
pub fn key_row_write(id: &str, fields: &serde_json::Map<String, serde_json::Value>, at: u64) -> (String, String) {
    let mut v = fields.clone();
    v.remove("id");
    v.insert("at".into(), at.into());
    (format!("k/{id}"), serde_json::Value::Object(v).to_string())
}

/// A FILE's REFERENCE (page and node): what a put gives and an item or a key-operation row names — its id (its first
/// root), the space it is listed in (none: the account's own), its key and root, its pieces' burn hash, its size, name
/// and type.
#[allow(clippy::too_many_arguments)]
pub fn file_ref(id: &str, in_space: Option<&str>, key: &[u8; 32], root: &[u8; 32], burn: &[u8; 32], size: u64, name: &str, kind: &str) -> serde_json::Value {
    let h = craftworks_identity::hex;
    serde_json::json!({ "id": id, "in": in_space, "key": h(key), "root": h(root), "b": h(burn), "size": size, "name": name, "type": kind })
}

/// A KEY ROW of `files` (`k/<id>`, page and node): its fields with `id`, when it names a `key` and a `root` (non-empty).
pub fn parse_key_row(key: &str, value: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    let id = key.strip_prefix("k/")?;
    let mut o = serde_json::from_str::<serde_json::Value>(value).ok()?.as_object()?.clone();
    let named = |f: &str| o.get(f).and_then(serde_json::Value::as_str).is_some_and(|s| !s.is_empty());
    if !(named("key") && named("root")) {
        return None;
    }
    o.insert("id".into(), serde_json::Value::String(id.to_string()));
    Some(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, n: i64, public: bool) -> Row {
        Row { id: id.into(), n, public, app: subject(None, Some("drive")), moved: 0 }
    }
    #[test]
    fn a_rows_subject_is_its_items_domain() {
        assert_eq!(subject(Some("video"), Some("board")), "video", "the item's domain over the app it came from");
        assert_eq!((subject(None, Some("drive")), subject(None, None), subject(Some(""), Some("board"))), ("file".into(), "file".into(), "board".into()));
        // A public row, uploaded from Board, held by a VIDEO: the space reads its videos in public, not its posts.
        let r = Row { id: "v".into(), n: -1, public: true, app: subject(Some("video"), Some("board")), moved: 0 };
        let reads = |d: &str| d == "video";
        assert!(plan(None, &[], &["me".into()], &[], &[], std::slice::from_ref(&r), &reads, "me", 0).due.is_empty(), "public as its video is: not due");
        // CONTROL — asked by its app name, the same row is judged as a post (not public): due to be sealed.
        let by_app = Row { app: "board".into(), ..r };
        assert_eq!(plan(None, &[], &["me".into()], &[], &[], &[by_app], &reads, "me", 0).due.len(), 1);
    }
    #[test]
    fn a_key_row_written_reads_back() {
        let mut f = serde_json::Map::new();
        f.insert("id".into(), "x".into());
        f.insert("key".into(), "aa".into());
        f.insert("root".into(), "bb".into());
        let (k, v) = key_row_write("x", &f, 7);
        let back = parse_key_row(&k, &v).expect("reads back");
        assert_eq!((back["id"].as_str(), back["key"].as_str(), back["at"].as_u64()), (Some("x"), Some("aa"), Some(7)));
        let r = file_ref("x", None, &[1; 32], &[2; 32], &[0; 32], 9, "n", "t");
        assert_eq!((r["key"].as_str().map(str::len), r["in"].is_null(), r["size"].as_u64()), (Some(64), true, Some(9)));
    }
    #[test]
    fn salts_and_key_rows_parse_one_way() {
        let h = "ab".repeat(32);
        assert_eq!(parse_salt(&h).map(|x| (x.1, x.2)), Some((0, 0)), "the first shape: the secret alone");
        assert_eq!(parse_salt(&format!(r#"{{"s":"{h}","n":2,"removals":1}}"#)).map(|x| (x.1, x.2)), Some((2, 1)));
        assert_eq!(parse_salt(r#"{"s":"short","n":1}"#), None, "a secret that is not 64 hex: no salt");
        assert!(parse_key_row("k/x", r#"{"key":"aa","root":"bb","n":1}"#).is_some_and(|o| o["id"] == "x"));
        assert!(parse_key_row("k/x", r#"{"key":"","root":"bb"}"#).is_none() && parse_key_row("k/x", r#"{"key":null,"root":"bb"}"#).is_none());
        assert!(parse_key_row("p/x", r#"{"key":"aa","root":"bb"}"#).is_none(), "not a key row");
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
        assert!(!only.due.is_empty(), "{only:?}");
        let x = plan(Some((0, 1)), &s(&["did:x"]), &s(&["did:a", "did:x"]), &s(&["did:x"]), &[], &rows, &anyone, "did:x", 0);
        assert!(x.due.is_empty(), "the put-out member takes no turn");
    }
}
