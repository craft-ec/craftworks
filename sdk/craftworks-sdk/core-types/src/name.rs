//! Names: the app id rule and the domain name rule, stated once.

/// The longest app id.
pub const MAX_APP: usize = 32;

/// An app id: 1–[`MAX_APP`] of `a-z 0-9 _ -` (no `.`, which separates an app
/// from its names, and no `@` or `/`).
pub fn app_ok(app: &str) -> bool {
    (1..=MAX_APP).contains(&app.len()) && app.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-".contains(&b))
}

/// A domain name of at most `max` bytes: 1–`max` of `a-z 0-9 _ - .`. The bound
/// differs by where the name sits (as an app writes it, or as stored with its
/// app's prefix); the characters do not.
pub fn domain_ok(name: &str, max: usize) -> bool {
    (1..=max).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-.".contains(&b))
}

/// The longest component id.
pub const MAX_COMPONENT: usize = 64;

/// A component's id in an app's definition (`c/<id>`, ARCHITECTURE §19): 1–[`MAX_COMPONENT`] of `A-Z a-z 0-9 _ -`.
pub fn component_ok(id: &str) -> bool {
    (1..=MAX_COMPONENT).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}

/// The longest file path in an app's definition.
pub const MAX_FILE_PATH: usize = 128;

/// A FILE's path in an app's definition (`f/<path>`, app-as-data P5: the app's code as data): 1–[`MAX_FILE_PATH`]
/// bytes of `/`-separated segments, each `[A-Za-z0-9_-][A-Za-z0-9_.-]*` -- so never empty, never `.` or `..`, never a
/// leading `/`: a path inside the app, and nothing else.
pub fn file_path_ok(path: &str) -> bool {
    (1..=MAX_FILE_PATH).contains(&path.len())
        && path.split('/').all(|seg| {
            let mut b = seg.bytes();
            b.next().is_some_and(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c)) && b.all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
        })
}

/// THE RESERVED SEGMENT (ARCHITECTURE §19, "An app is DATA in its owner's tree"): an app's definition lives in
/// reserved domains of the app, `craftworks.<…>`, which ONLY the SDK's definition doors write -- each named by a
/// [`SystemDomain`], never spelled by hand. The one statement of the word.
pub const RESERVED: &str = "craftworks";

/// Is `name` under the reserved prefix: is [`RESERVED`] its FIRST or SECOND dot-segment? Both, because a domain is
/// held two ways -- as the app wrote it (`craftworks.draft`, the in-tab db's form) and as the tree stores it, behind the
/// app's id (`notes.craftworks.draft`) -- and a check that sees only the name cannot tell which it has. So the one
/// rule, the same wherever it is asked: neither of the first two segments of any domain is `craftworks`.
pub fn reserved(name: &str) -> bool {
    let mut seg = name.split('.');
    seg.next() == Some(RESERVED) || seg.next() == Some(RESERVED)
}

crate::vocabulary! {
    /// A RESERVED DOMAIN of an app (§19): the only names under [`RESERVED`] there are, and the only way to one. The
    /// SDK's definition doors take this type, so a reserved name is never a string a caller built.
    /// * `Draft`: the definition as edited (every builder edit, `draft_put` / `draft_delete`).
    /// * `App`: the definition as PUBLISHED -- made equal to the draft by ONE write (`publish_definition`).
    /// * `Published`: the builder's per-domain "live" markers (`mark_published`), until app-as-data P5 derives
    ///   liveness from `App` and removes it.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum SystemDomain {
        Draft => "craftworks.draft",
        App => "craftworks.app",
        Published => "craftworks.published",
    }
}

/// A JavaScript module an app carries beside the SDK's `index.js` (its build's
/// `modules` list): one file name, `[A-Za-z0-9_-][A-Za-z0-9_.-]*.js`, so it
/// can never be a path (no `/`, no leading `.`).
pub fn module_ok(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".js") else { return false };
    let mut bytes = stem.bytes();
    let first_ok = bytes.next().is_some_and(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b));
    first_ok && bytes.all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_ids() {
        assert!(app_ok("notes") && app_ok("a_b-9") && app_ok(&"a".repeat(32)));
        assert!(!app_ok("") && !app_ok(&"a".repeat(33)) && !app_ok("a.b") && !app_ok("A") && !app_ok("a/b") && !app_ok("@a"));
    }

    #[test]
    fn modules() {
        assert!(module_ok("index.js") && module_ok("wrap.js") && module_ok("a.b-c_d.js") && module_ok("_x.js"));
        assert!(!module_ok("") && !module_ok(".js") && !module_ok("../x.js") && !module_ok("a/b.js") && !module_ok(".hidden.js"));
        assert!(!module_ok("x.wasm") && !module_ok("x.js.map") && !module_ok("x y.js"));
    }

    #[test]
    fn component_ids() {
        assert!(component_ok("header") && component_ok("C1-a_b") && component_ok(&"a".repeat(64)));
        assert!(!component_ok("") && !component_ok(&"a".repeat(65)) && !component_ok("a.b") && !component_ok("a b") && !component_ok("a/b"));
    }

    #[test]
    fn file_paths() {
        for ok in ["app.js", "components/header.js", "a/b/c.min.css", "_x-1.js", &"a".repeat(128)] {
            assert!(file_path_ok(ok), "`{ok}` is a file path");
        }
        for no in ["", "/app.js", "app.js/", "a//b", ".hidden", "a/../b", "a/./b", "..", "a b.js", "a\\b", &"a".repeat(129)] {
            assert!(!file_path_ok(no), "`{no}` is not a file path");
        }
    }

    #[test]
    fn reserved_names() {
        for r in ["craftworks", "craftworks.draft", "craftworks.x.y", "notes.craftworks", "notes.craftworks.app", "a.craftworks.b.c"] {
            assert!(reserved(r), "`{r}` is under the reserved prefix");
        }
        for ok in ["notes", "craftworks_notes", "craftworksx.y", "a.b.craftworks", "notes.rows", "xcraftworks.draft"] {
            assert!(!reserved(ok), "`{ok}` is not under the reserved prefix");
        }
        // Every system domain is under it, and is a legal domain name.
        for d in SystemDomain::ALL {
            assert!(reserved(d.code()) && domain_ok(d.code(), 32), "{d:?}");
            assert_eq!(d.code().split('.').next(), Some(RESERVED));
        }
    }

    #[test]
    fn domains() {
        assert!(domain_ok("notes.rows", 32) && domain_ok("a", 1));
        assert!(!domain_ok("", 32) && !domain_ok("ab", 1) && !domain_ok("a#b", 32) && !domain_ok("A", 32));
    }
}
