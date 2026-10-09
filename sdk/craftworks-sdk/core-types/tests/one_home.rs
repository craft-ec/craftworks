//! THE CONTROL (the owner, 2026-09-23: "provide me solution, control so that things won't happen"): hex and the
//! name rules have ONE home, `core_types::{hex, name}`. A hand-written hex encode or decode, or a hand-written
//! app / domain character rule, anywhere else in the workspace fails this test, so a new copy cannot merge -- it is
//! caught by the build, not by a reviewer noticing it.

use std::path::{Path, PathBuf};

/// A line writes hex or a name rule by hand when it contains any of these (a TRIPWIRE for the shapes known so
/// far, not a proof -- each shape has a control line below):
/// - a two-digit hex format, in any macro (`format!`, `write!`, ...);
/// - a base-16 parse;
/// - a hex digit written as a byte or char range;
/// - a name rule's character set, as a byte-string `contains` or a `matches!` over `'a'..='z'` with `_`.
fn hand_written(l: &str) -> bool {
    let hex_format = l.contains(":02x}") || l.contains(":02X}");
    let hex_parse = l.contains("from_str_radix") && l.contains("16)");
    let hex_range = ["b'a'..=b'f'", "b'A'..=b'F'", "'a'..='f'", "'A'..='F'"].iter().any(|r| l.contains(r));
    let name_set = [r#"b"_-".contains"#, r#"b"_-.".contains"#].iter().any(|r| l.contains(r))
        || ((l.contains("b'a'..=b'z'") || l.contains("'a'..='z'")) && (l.contains("b'_'") || l.contains("'_'")));
    hex_format || hex_parse || hex_range || name_set
}

/// Lines of `text` that write hex or a name rule by hand (comments are not code).
fn copies(text: &str) -> Vec<String> {
    text.lines().filter(|l| !l.trim_start().starts_with("//")).filter(|l| hand_written(l)).map(str::to_string).collect()
}

/// A Cargo manifest line that takes an external hex crate instead of core-types.
fn hex_crate(line: &str) -> bool {
    let l = line.trim_start();
    l.starts_with("hex =") || l.starts_with("hex=") || l.starts_with("hex.") || l.starts_with("const-hex") || l.starts_with("faster-hex")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if p.is_dir() {
            if !matches!(name, "target" | "node_modules" | "pkg" | ".git" | "core-types") && !name.starts_with('.') {
                rust_files(&p, out);
            }
        } else if name.ends_with(".rs") || name == "Cargo.toml" {
            out.push(p);
        }
    }
}

#[test]
fn hex_and_name_rules_are_written_only_in_core_types() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("the workspace");
    let mut files = Vec::new();
    rust_files(root, &mut files);
    // Not vacuous: the walk must have found the workspace's sources and manifests.
    let rs = files.iter().filter(|f| f.extension().is_some_and(|e| e == "rs")).count();
    let manifests = files.len() - rs;
    assert!(rs > 100 && manifests > 5, "the scan found {rs} .rs files and {manifests} manifests: it is not looking at the workspace");
    let found: Vec<String> = files
        .iter()
        .flat_map(|f| {
            let text = std::fs::read_to_string(f).unwrap_or_default();
            let found = if f.ends_with("Cargo.toml") { text.lines().filter(|l| hex_crate(l)).map(str::to_string).collect() } else { copies(&text) };
            found.into_iter().map(move |l| format!("{}: {}", f.strip_prefix(root).unwrap_or(f).display(), l.trim()))
        })
        .collect();
    assert!(found.is_empty(), "hex or a name rule written by hand outside core-types (use core_types::hex / ::name):\n{}", found.join("\n"));
}

/// The detector's control: each shape it exists for IS caught, and the calls it points to are not.
#[test]
fn the_detector_catches_each_copy_shape() {
    for caught in [
        r#"    b.iter().map(|x| format!("{x:02x}")).collect()"#,
        r#"    write!(s, "{b:02x}").unwrap();"#,
        r#"    let _ = write!(out, "{:02X}", b);"#,
        r#"    u8::from_str_radix(&s[i..i + 2], 16).ok()"#,
        r#"    b'0'..=b'9' | b'a'..=b'f' => Some(c - b'a' + 10),"#,
        r#"    c.is_ascii_digit() || ('a'..='f').contains(&c)"#,
        r#"    app.bytes().all(|b| b.is_ascii_lowercase() || b"_-".contains(&b))"#,
        r#"    d.bytes().all(|b| b.is_ascii_digit() || b"_-.".contains(&b))"#,
        r#"    app.bytes().all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'))"#,
    ] {
        assert_eq!(copies(caught).len(), 1, "a copy shape was not caught: {caught}");
    }
    for clean in ["    core_types::hex::encode(&b)", "    core_types::name::app_ok(app)", r#"    // format!("{x:02x}") in a comment is not code"#] {
        assert!(copies(clean).is_empty(), "the call it points to was flagged: {clean}");
    }
    assert!(hex_crate(r#"hex = "0.4""#) && hex_crate(r#"hex.workspace = true"#) && hex_crate(r#"const-hex = "1""#));
    assert!(!hex_crate(r#"core-types = { path = "../core-types" }"#) && !hex_crate(r#"blake3 = "1""#));
}

/// A line SPELLS a reserved name by hand (ARCHITECTURE §19): `craftworks.` inside a string or template -- after a
/// quote or backtick, or after a `.` (a name built behind an app id, `"{app}.craftworks.draft"`). The reserved names
/// are `core_types::name::SystemDomain`'s codes, and a door takes that type; a spelled one is a way around the doors.
fn spells_reserved(l: &str) -> bool {
    let code = l.split("//").next().unwrap_or("");
    ["\"craftworks.", "'craftworks.", "`craftworks.", ".craftworks.", "}craftworks."].iter().any(|p| code.contains(p))
}

fn source_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if p.is_dir() {
            // Tests may spell a reserved name: that is how they plant one. The one home is core-types.
            if !matches!(name, "target" | "node_modules" | "pkg" | ".git" | "core-types" | "tests" | "examples") && !name.starts_with('.') {
                source_files(&p, out);
            }
        } else if name.ends_with(".rs") || name.ends_with(".js") || name.ends_with(".mjs") {
            out.push(p);
        }
    }
}

/// THE RESERVED PREFIX HAS ONE HOME (app-as-data P2): no SDK source outside core-types spells `craftworks.<…>` --
/// every reserved name comes from `SystemDomain`, and only the definition doors write one. A hand-spelled name (a
/// door BYPASS: an ordinary write, a raw store write, a JavaScript constant) fails the build.
#[test]
fn the_reserved_prefix_is_spelled_only_in_core_types() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("the workspace");
    let mut files = Vec::new();
    source_files(root, &mut files);
    let js = files.iter().filter(|f| f.extension().is_some_and(|e| e == "js")).count();
    assert!(files.len() > 100 && js > 5, "the scan found {} files ({js} .js): it is not looking at the SDK", files.len());
    let found: Vec<String> = files
        .iter()
        .flat_map(|f| {
            let text = std::fs::read_to_string(f).unwrap_or_default();
            let in_tests = text.find("#[cfg(test)]").unwrap_or(text.len());
            let hits: Vec<String> = text[..in_tests].lines().filter(|l| !l.trim_start().starts_with("//") && !l.trim_start().starts_with('*')).filter(|l| spells_reserved(l)).map(|l| l.trim().to_string()).collect();
            hits.into_iter().map(move |l| format!("{}: {l}", f.strip_prefix(root).unwrap_or(f).display()))
        })
        .collect();
    assert!(found.is_empty(), "a reserved name spelled by hand outside core-types (use core_types::name::SystemDomain, through a definition door):\n{}", found.join("\n"));
}

/// The detector's control: each way to spell a reserved name IS caught -- the planted door bypasses -- and the
/// type-built and unrelated forms are not.
#[test]
fn the_reserved_detector_catches_each_spelling() {
    for caught in [
        r#"    db.put("notes.craftworks.draft", &f)?;"#,
        r#"    let name = format!("{app}.craftworks.app");"#,
        r#"    store.apply_batch(&[(record_key("craftworks.published", loc), Edit::Delete)])"#,
        r#"export const PUBLISHED_DOMAIN = "craftworks.published";"#,
        r#"    await db.put(`${app}.craftworks.draft`, row);"#,
        r#"    const d = 'craftworks.app';"#,
    ] {
        assert!(spells_reserved(caught), "a spelled reserved name was not caught: {caught}");
    }
    for clean in [
        "    let name = system_name(app, SystemDomain::Draft);",
        "    use craftworks_sdk::db;",
        "    const d = globalThis.__craftworks.db;",
        "    let s = \"craftworks_notes\";",
        r#"    let x = 1; // "craftworks.draft" in a trailing comment is not code"#,
    ] {
        assert!(!spells_reserved(clean), "a form that spells no reserved name was flagged: {clean}");
    }
}
