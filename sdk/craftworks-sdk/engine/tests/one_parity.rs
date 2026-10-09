//! ONE OWNER FOR PARITY PER GROUP (the architect, #351): the engine and the page take the
//! TREE's number, `freenet_prolly::parity::PARITY` (re-exported as
//! `engine::PARITY`), and never writes it by hand. A hand-written 3 was right
//! only while the tree's number was 3; this fails the build on one.
//!
//! The codec's capability (`MAX_M`) is not the tree's parity per group, so the
//! engine's own code may not name it either.

use std::path::Path;

/// Spellings of a hand-written parity count, whitespace removed before matching.
const HAND_WRITTEN: [&str; 5] = ["3*g", "chunks_exact(3)", "[Cid;3]", "k+3", "threeparity"];

fn hits(text: &str, src: bool) -> Vec<String> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let squashed: String = line.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase();
        for p in HAND_WRITTEN {
            if squashed.contains(&p.to_lowercase()) {
                out.push(format!("line {}: `{p}` in {}", n + 1, line.trim()));
            }
        }
        if src && names_max_m(line) {
            out.push(format!("line {}: `MAX_M` (the codec's capability, not the tree's parity) in {}", n + 1, line.trim()));
        }
    }
    out
}

/// `MAX_M` as a whole word (`MAX_MESSAGE` is another name).
fn names_max_m(line: &str) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    line.match_indices("MAX_M").any(|(i, _)| {
        !line[..i].chars().next_back().is_some_and(word) && !line[i + "MAX_M".len()..].chars().next().is_some_and(word)
    })
}

fn scan(dir: &Path, src: bool, found: &mut Vec<String>) {
    for e in std::fs::read_dir(dir).expect("readable") {
        let path = e.expect("entry").path();
        if path.is_dir() {
            scan(&path, src, found);
        } else if path.extension().is_some_and(|x| x == "rs") && path.file_name().is_some_and(|n| n != "one_parity.rs") {
            let text = std::fs::read_to_string(&path).expect("utf-8");
            found.extend(hits(&text, src).into_iter().map(|h| format!("{}: {h}", path.display())));
        }
    }
}

#[test]
fn the_engine_writes_no_parity_count_by_hand() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = Vec::new();
    scan(&root.join("src"), true, &mut found);
    scan(&root.join("tests"), false, &mut found);
    // The PAGE counts group blocks too (a head's `after`, its held effects):
    // a page-side literal is the same copy. And every other crate's source
    // and tests (sdk#321: a page TEST sliced `ids[3 * g..3 * g + 3]` and read
    // the wrong parity once the tree's number moved).
    scan(&root.join("../page/src"), true, &mut found);
    for dir in ["page/tests", "page-io/src", "page-io/tests", "web/src", "web/tests", "testkit/src", "testkit/tests", "src", "tests", "wire/src", "signer/src", "probe/src"] {
        let d = root.join("..").join(dir);
        if d.is_dir() {
            scan(&d, dir.ends_with("src"), &mut found);
        }
    }
    assert!(found.is_empty(), "a hand-written parity count; use engine::PARITY (the tree's):\n{}", found.join("\n"));
}

/// The control: each spelling is found, so a pass above is a scan that looks.
#[test]
fn every_hand_written_spelling_is_caught() {
    for (text, src) in [
        ("let par = ids.get(3 * g..3 * g + 3)?;", false),
        ("for c in ids.chunks_exact(3) {}", false),
        ("pub type ParityIds = [Cid; 3];", false),
        ("/// any k of the group's k + 3", false),
        ("/// its three parity ids", false),
        ("let m = rs::MAX_M;", true),
    ] {
        assert!(!hits(text, src).is_empty(), "not caught: {text}");
    }
    assert!(hits("let m = rs::MAX_M;", false).is_empty(), "MAX_M is refused in src only");
    assert!(hits("/// the wire's `MAX_MESSAGE`", true).is_empty(), "MAX_MESSAGE is not MAX_M");
}
