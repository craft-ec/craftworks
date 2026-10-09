//! THE APP NAMESPACE (the forest ruling): a person has ONE tree, divided by
//! app. Every name an app uses — a domain, or a watch key (`domain` or
//! `domain#parent`) — is RELATIVE to it and stored as `<app>.<name>`, so an app
//! has no name for another app's data and cannot write it. Another app's data
//! is READ through the absolute `@app/name` form (`db.other(app)`); it is
//! never written. `.` separates the app from the name: an app id has none.
//!
//! Pure functions, so the rules are tested natively (tests/app_namespace.rs);
//! the web Session applies them at every name that crosses into it.

use crate::db::DbError;

/// The longest app id (layer 0's one statement).
pub use core_types::name::MAX_APP;

/// The longest name an app WRITES — a domain, relative to the app.
///
/// Checked BEFORE the app's prefix is added, so a refusal names the name the
/// app wrote, and an app id never eats into it (sdk#276: the prefixed
/// `<app>.<name>` was checked against 32, so a 17-character builder project id
/// left 14 for every domain, and an id at the maximum left none). The stored
/// name has its own bound, `db::MAX_DOMAIN`.
pub const MAX_NAME: usize = 32;

/// Is `app` an app id: 1–32 of a-z 0-9 _ - (no `.`, no `@`, no `/`).
pub fn check(app: &str) -> Result<(), DbError> {
    if core_types::name::app_ok(app) { Ok(()) } else { Err(DbError::Refused(format!("app id `{app}` must be 1–{MAX_APP} of a-z 0-9 _ -"))) }
}

/// Is `name` a name an app may use: 1–32 of a-z 0-9 _ - . — the same rule a
/// domain always had. A watch key (`domain#parent`) is checked by its domain.
pub fn check_name(name: &str) -> Result<(), DbError> {
    let domain = name.split_once('#').map_or(name, |(d, _)| d);
    if core_types::name::domain_ok(domain, MAX_NAME) {
        Ok(())
    } else {
        Err(DbError::NotDefined(format!("domain `{domain}` must be 1–{MAX_NAME} of a-z 0-9 _ - .")))
    }
}

/// A name this app READS, as the tree stores it: its own `<app>.name`, another
/// app's through `@other/name`, or — with no app — the name as given (data
/// from before apps, readable, never written).
pub fn read(app: Option<&str>, name: &str) -> Result<StoredName, DbError> {
    if let Some(abs) = name.strip_prefix('@') {
        let (other, rest) = abs.split_once('/').ok_or_else(|| DbError::Refused(format!("`{name}` is not `@app/name`")))?;
        check(other)?;
        check_name(rest)?;
        return Ok(StoredName(format!("{other}.{rest}")));
    }
    Ok(StoredName(match app {
        Some(a) => {
            check_name(name)?;
            format!("{a}.{name}")
        }
        // No app: a name as the tree stores it, which may carry an app's
        // prefix and so be longer than an app's own name; `db` bounds it.
        None => name.to_string(),
    }))
}

/// A name this app WRITES: only its own. Another app's (`@other/…`) is refused
/// by name, and so is every write by a session with no app.
pub fn write(app: Option<&str>, name: &str) -> Result<StoredName, DbError> {
    if name.starts_with('@') {
        return Err(DbError::Refused(format!("`{name}` is another app's: an app writes only its own")));
    }
    // THE RESERVED PREFIX (ARCHITECTURE §19): an app's definition domains are written only by the SDK's definition
    // doors (`Db::draft_put` / `draft_delete` / `publish_definition` / `mark_published`), never by an ordinary write.
    if core_types::name::reserved(name) {
        return Err(reserved_refusal(name));
    }
    match app {
        Some(a) => {
            check_name(name)?;
            Ok(StoredName(format!("{a}.{name}")))
        }
        None => Err(DbError::Refused(
            "no app: open the session with { app } before writing — a write with no app would land outside every app's space".into(),
        )),
    }
}

/// A stored name back to what this app calls it; `None` for another app's.
///
/// THE ONLY WAY to an [`AppName`] from anything the tree holds — so a name
/// headed for JavaScript has been through here, or it does not compile.
pub fn own(app: Option<&str>, stored: &StoredName) -> Option<AppName> {
    let s = stored.as_str();
    match app {
        Some(a) => s.strip_prefix(a).and_then(|r| r.strip_prefix('.')).map(|r| AppName(r.to_string())),
        None => Some(AppName(s.to_string())),
    }
}

/// A name as the TREE stores it — `<app>.<name>`, what record keys hold (or,
/// with no app, a name from before apps as given). What the core `Db` takes:
/// it derefs to `&str` for exactly that. It is NEVER what JavaScript holds;
/// [`own`] is the one way back, and [`to_js`] takes only [`AppName`]s.
///
/// Two types because both forms were `String` and #282 handed JavaScript the
/// stored one: no binding is keyed by it, so a tab's own write never
/// re-rendered. With two types that mistake does not compile:
///
/// ```compile_fail
/// use craftworks_sdk::app::{self, StoredName};
/// let stored: StoredName = app::write(Some("notes-app"), "notes").unwrap();
/// // A stored name where JavaScript's app name is expected:
/// let _ = app::to_js(&[stored]);
/// ```
///
/// The control, which DOES compile — the same name through [`own`]:
///
/// ```
/// use craftworks_sdk::app;
/// let stored = app::write(Some("notes-app"), "notes").unwrap();
/// let mine = app::own(Some("notes-app"), &stored).unwrap();
/// assert_eq!(app::to_js(&[mine]), r#"["notes"]"#);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoredName(String);

impl StoredName {
    /// A name read out of the TREE — a key's domain, a range's watch key, a
    /// schema's domain. Stored by definition: it came from where stored names
    /// live. Not for anything an app or JavaScript supplied (that goes through
    /// [`read`] / [`write`], which check and prefix it).
    pub fn of_tree(name: String) -> StoredName {
        StoredName(name)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for StoredName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::ops::Deref for StoredName {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

/// A name as the APP writes it — relative to its app; what JavaScript and its
/// bindings hold. Made ONLY by [`own`], from a stored name. Deliberately NOT
/// `Deref<str>`, so it is never mistaken for a stored name on the way in either.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AppName(String);

impl AppName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The ONE way names reach JavaScript: a JSON array of app names. A stored
/// name here is a compile error (see [`StoredName`]).
pub fn to_js(names: &[AppName]) -> String {
    serde_json::to_string(&names.iter().map(AppName::as_str).collect::<Vec<_>>()).unwrap_or_else(|_| "[]".into())
}

/// The refusal of an ordinary write under the reserved prefix: ONE wording, here and at the `Db`.
pub fn reserved_refusal(name: &str) -> DbError {
    DbError::Refused(format!(
        "`{name}` is under the reserved `{}.` prefix: an app's definition, written only by the SDK's definition doors (ARCHITECTURE §19)",
        core_types::name::RESERVED
    ))
}
