//! NO SILENT DRIVER (the architect on #413 x #424): a hand-written test driver over the engine's `Effect`s or the
//! page's `Op`s that ends in a silent catch-all (`_ => {}`) turns every effect it does not know -- #424's
//! `ConfirmHeld`, the next one -- into a stall with no error: #413's read_repair driver dropped `ConfirmHeld` and its
//! write simply never published. So a driver's catch-all must NAME what it lets go (`common::no_answer_owed`, which
//! panics on an effect the engine waits on; `common::unanswered_op`), or list the variants it means to ignore.
//! Held by source, over engine/tests and page/tests.

use std::path::Path;

/// Every `_ => {}` / `_ => ()` whose match is over `Effect::` or `Op::` arms: `(file, line)`.
fn silent_catch_alls(file: &str, src: &str) -> Vec<(String, usize)> {
    let lines: Vec<&str> = src.lines().collect();
    let mut found = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim();
        if !(t.starts_with("_ => {}") || t.starts_with("_ => ()")) {
            continue;
        }
        // The arms above it, back to the enclosing `match` (a dozen lines covers every driver here).
        let from = i.saturating_sub(12);
        let arms = lines[from..i].iter().filter(|a| a.contains("=>"));
        if arms.clone().any(|a| a.contains("Effect::") || a.contains("Op::")) {
            found.push((file.to_string(), i + 1));
        }
    }
    found
}

fn scan(dir: &Path) -> Vec<(String, usize)> {
    let mut found = Vec::new();
    let mut files: Vec<_> = std::fs::read_dir(dir).expect("the tests dir").flatten().map(|e| e.path()).collect();
    files.extend(std::fs::read_dir(dir.join("common")).into_iter().flatten().flatten().map(|e| e.path()));
    for p in files {
        if p.extension().is_some_and(|x| x == "rs") {
            let src = std::fs::read_to_string(&p).expect("readable");
            found.extend(silent_catch_alls(&p.display().to_string(), &src));
        }
    }
    found
}

#[test]
fn drivers_have_no_silent_catch_all() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let (engine, page) = (root.join("engine/tests"), root.join("page/tests"));
    let files = std::fs::read_dir(&engine).expect("engine/tests").count() + std::fs::read_dir(&page).expect("page/tests").count();
    assert!(files > 20, "THE CONTROL: the scan found {files} files -- it is not looking where the drivers are");
    let mut found = scan(&engine);
    found.extend(scan(&page));
    assert!(found.is_empty(), "a test driver ends in a SILENT catch-all over Effect/Op (an unknown effect becomes a stall with no error): {found:?}");
}

/// THE CONTROLS: the scan flags a silent catch-all under Effect arms and under Op arms, and passes the named forms.
#[test]
fn the_scan_flags_a_silent_catch_all_and_passes_a_named_one() {
    let silent = "match f {\n    Effect::PutBlock { id, .. } => ack(id),\n    _ => {}\n}\n";
    let silent_op = "match op {\n    Op::Put { id, .. } => ack(id),\n    _ => (),\n}\n";
    let named = "match f {\n    Effect::PutBlock { id, .. } => ack(id),\n    other => common::no_answer_owed(&other),\n}\n";
    let other_match = "match c {\n    '{' => depth += 1,\n    _ => {}\n}\n";
    assert_eq!(silent_catch_alls("s", silent), vec![("s".to_string(), 3)], "a silent catch-all over Effect was not flagged");
    assert_eq!(silent_catch_alls("o", silent_op), vec![("o".to_string(), 3)], "a silent catch-all over Op was not flagged");
    assert!(silent_catch_alls("n", named).is_empty(), "a named catch-all was flagged");
    assert!(silent_catch_alls("c", other_match).is_empty(), "a match over something else was flagged");
}
