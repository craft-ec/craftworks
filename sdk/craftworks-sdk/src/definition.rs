//! AN APP'S DEFINITION, AS DATA (ARCHITECTURE §19, "An app is DATA in its owner's tree"; app-as-data P2): its
//! records' KEYS. A definition is `meta`, one `c/<component>` record per component and one `d/<domain>` record per
//! data domain, held in the app's reserved domains (`core_types::name::SystemDomain`) and written only by the
//! definition doors on [`crate::Db`] (`draft_put`, `draft_delete`, `publish_definition`). A key is parsed HERE and
//! nowhere else; any other spelling is refused.

use crate::db::DbError;

/// Which record of a definition.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DefKey {
    /// The app's own record: its name and settings.
    Meta,
    /// A component, by its id (`core_types::name::component_ok`).
    Component(String),
    /// A data domain's declaration, by the name the app gives it: a domain name an app may write
    /// (`crate::app::check_name`), so never a reserved one.
    Domain(String),
    /// A FILE of the app's code (app-as-data P5), by its path in the app (`core_types::name::file_path_ok`): its
    /// bytes are the record's `bytes`, written by the file door (`Db::draft_file`).
    File(String),
}

impl DefKey {
    /// `meta`, `c/<id>` or `d/<domain>` -- or refused, naming the spelling.
    pub fn parse(s: &str) -> Result<DefKey, DbError> {
        let bad = || DbError::Refused(format!("`{s}` is not a definition key: `meta`, `c/<component id>`, `d/<domain>` or `f/<file path>`"));
        match s.split_once('/') {
            None if s == "meta" => Ok(DefKey::Meta),
            Some(("c", id)) if core_types::name::component_ok(id) => {
                Ok(DefKey::Component(id.to_string()))
            }
            Some(("f", path)) if core_types::name::file_path_ok(path) => Ok(DefKey::File(path.to_string())),
            Some(("d", name)) => {
                crate::app::check_name(name)?;
                if core_types::name::reserved(name) {
                    return Err(bad());
                }
                Ok(DefKey::Domain(name.to_string()))
            }
            _ => Err(bad()),
        }
    }
}

impl std::fmt::Display for DefKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DefKey::Meta => f.write_str("meta"),
            DefKey::Component(id) => write!(f, "c/{id}"),
            DefKey::Domain(name) => write!(f, "d/{name}"),
            DefKey::File(path) => write!(f, "f/{path}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_its_one_spelling_or_refused() {
        for s in ["meta", "c/header", "c/C1-a_b", "d/notes", "d/notes.rows", "f/app.js", "f/components/header.js"] {
            assert_eq!(DefKey::parse(s).expect(s).to_string(), s, "`{s}` does not spell back");
        }
        let reserved = [format!("d/{}", core_types::name::SystemDomain::Draft.code()), format!("d/notes.{}", core_types::name::RESERVED)];
        for s in ["", "Meta", "meta/x", "c/", "c/a b", "c/a.b", "x/y", "d/", "d/A", "/meta", "f/", "f//a", "f/../x", "f/.x"].iter().map(|s| s.to_string()).chain(reserved) {
            let s = s.as_str();
            assert!(DefKey::parse(s).is_err(), "`{s}` was taken as a definition key");
        }
    }
}
