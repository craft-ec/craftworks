//! ONE OWNER OF "A PIECE -> ITS WEB CONTAINER" (sdk#493, engineer4's finding): `wire::webapp::piece_container`. It was
//! written three times (the build's `load-pieces`, the loader's repair in js/pieces.js, a probe test), and a copy that
//! drifted would rebuild a piece at an address no starter names. This scans every Rust and JS source for a piece framed
//! by hand -- an app container whose file is named "piece" -- outside the owner. THE CONTROL: the scan finds the
//! owner's own framing, so an empty or broken read cannot pass.
use std::path::{Path, PathBuf};

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if p.is_dir() {
            if !matches!(name, "target" | "node_modules" | "pkg" | ".git") && !name.starts_with("target") {
                sources(&p, out);
            }
        } else if name.ends_with(".rs") || name.ends_with(".js") || name.ends_with(".mjs") {
            out.push(p);
        }
    }
}

/// A hand framing of a piece: an app container, or an `AppContainer.add`, naming the file "piece".
fn frames_a_piece(line: &str) -> bool {
    let l: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    l.contains("app_container(&[(\"piece\"") || l.contains(".add(\"piece\"") || l.contains("app_container(&[(PIECE_FILE")
}

#[test]
fn a_piece_is_framed_only_by_the_one_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("the workspace");
    let mut files = Vec::new();
    sources(root, &mut files);
    let me = Path::new(file!()).file_name().expect("this file");
    let mut owner = 0usize;
    let mut elsewhere = Vec::new();
    for f in &files {
        if f.file_name() == Some(me) {
            continue;
        }
        let text = std::fs::read_to_string(f).unwrap_or_default();
        for (i, line) in text.lines().enumerate() {
            if frames_a_piece(line) {
                if f.ends_with("wire/src/webapp.rs") {
                    owner += 1;
                } else {
                    elsewhere.push(format!("{}:{}", f.strip_prefix(root).unwrap_or(f).display(), i + 1));
                }
            }
        }
    }
    println!("{} source files scanned; the owner frames a piece {owner} time(s)", files.len());
    assert_eq!(owner, 1, "THE CONTROL: the scan did not find the owner's own framing (wire/src/webapp.rs)");
    assert!(elsewhere.is_empty(), "a piece framed by hand outside wire::webapp::piece_container: {elsewhere:?}");
}
