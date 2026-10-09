//! THE LINES SAID TO THE APP HAVE ONE WRITER AND ONE READER (sdk#482; engineer1's condition, the architect's ruling),
//! held by the source: in the page, page-io and the session, a line is kept only by `say` (which records it first), and
//! read only by `take_unusable` (which drains) -- the non-draining `unusable()` look is for tests, and no production
//! code calls it.

const SOURCES: &[(&str, &str)] = &[
    ("page/src/lib.rs", include_str!("../src/lib.rs")),
    ("page-io/src/lib.rs", include_str!("../../page-io/src/lib.rs")),
    ("web/src/session.rs", include_str!("../../web/src/session.rs")),
];

/// The production lines of `src`: every `#[cfg(test)] mod … { … }` block left out (by brace count).
fn production(src: &str) -> Vec<(usize, &str)> {
    let lines: Vec<&str> = src.lines().collect();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < lines.len() {
        if lines[i].trim() == "#[cfg(test)]" && lines.get(i + 1).is_some_and(|l| l.trim_start().starts_with("mod ")) {
            let mut depth = 0i64;
            let mut j = i + 1;
            loop {
                depth += lines[j].matches('{').count() as i64 - lines[j].matches('}').count() as i64;
                j += 1;
                if depth <= 0 && lines[j - 1].contains('}') || j >= lines.len() {
                    break;
                }
            }
            i = j;
            continue;
        }
        out.push((i + 1, lines[i]));
        i += 1;
    }
    out
}

#[test]
fn a_line_is_kept_only_by_say_and_read_only_by_take_unusable() {
    let mut peeks = Vec::new();
    let mut pushes = Vec::new();
    for (file, src) in SOURCES {
        let prod = production(src);
        assert!(prod.len() > 100, "THE SETUP: {file} has almost no production lines ({})", prod.len());
        for (n, l) in &prod {
            if l.contains(".unusable()") {
                peeks.push(format!("{file}:{n}: {}", l.trim()));
            }
            if l.contains("unusable.push(") {
                pushes.push(format!("{file}:{n}"));
            }
        }
    }
    assert!(peeks.is_empty(), "production code reads the lines without draining them: {peeks:#?}");
    // One `self.unusable.push(line)` per crate, inside its `say` (which records the line first).
    assert_eq!(pushes.len(), 3, "a line is kept other than by `say`: {pushes:#?}");

    // THE CONTROL: the scan sees a production peek and a stray push.
    let planted = "fn f(&mut self) {\n    let _ = self.io.unusable();\n    self.unusable.push(x);\n}\n#[cfg(test)]\nmod t {\n    fn g() { p.unusable(); }\n}\n";
    let prod = production(planted);
    assert_eq!(prod.iter().filter(|(_, l)| l.contains(".unusable()")).count(), 1, "the scan did not see a production peek, or saw a test one");
    assert_eq!(prod.iter().filter(|(_, l)| l.contains("unusable.push(")).count(), 1);
}
