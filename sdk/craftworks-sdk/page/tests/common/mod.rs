//! THE ONE PLACE a source scan decides where production code ends (sdk#419). Two scans once cut differently -- the
//! one-door control at the first `#[cfg(test)]` in column 0, the one-removal-site scan (#407) at the first `#[cfg(test)]`
//! anywhere -- and #401's test-only FIELD cut the second early, so its counts read 0 over nothing. Both read here now.
#![allow(dead_code)]

/// The PRODUCTION part of a source file: everything before its first TOP-LEVEL test module -- a `#[cfg(test)]` in
/// column 0 whose item (past any further attributes) is a `mod`. A `#[cfg(test)]` on a field, a statement or a
/// top-level fn is not the end of production; [`items_after_the_cut`] says nothing else follows the cut.
pub fn production_of(src: &str) -> &str {
    let lines: Vec<&str> = src.lines().collect();
    let mut at = 0usize;
    for (i, l) in lines.iter().enumerate() {
        if *l == "#[cfg(test)]" {
            // The item: past further attributes, doc comments and blank lines.
            let item = lines[i + 1..].iter().find(|n| !n.starts_with("#[") && !n.trim_start().starts_with("//") && !n.trim().is_empty());
            if item.is_some_and(|n| is_mod(n)) {
                return &src[..at];
            }
        }
        at += l.len() + 1;
    }
    src
}

/// A module item line, whatever its visibility: `mod`, `pub mod`, `pub(crate) mod`, `pub(super) mod`.
fn is_mod(line: &str) -> bool {
    let rest = line.strip_prefix("pub").map_or(line, |r| r.strip_prefix(|c: char| c == '(').map_or(r, |r| r.split_once(')').map_or(r, |(_, t)| t)));
    rest.trim_start().starts_with("mod ")
}

/// The TOP-LEVEL items of `src` after its production cut that are NOT a `#[cfg(test)]` module: each `(line, text)`.
/// A top-level item is a line in column 0 that is not a comment, an attribute, a closing brace or blank; its
/// attributes are the `#[...]` lines right above it. Rust allows production items after a test module, and one
/// there would escape [`production_of`] -- so the part after the cut may hold test modules and nothing else.
pub fn items_after_the_cut(src: &str) -> Vec<(usize, String)> {
    let cut = production_of(src).lines().count();
    let lines: Vec<&str> = src.lines().collect();
    let mut attrs: Vec<&str> = Vec::new();
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate().skip(cut) {
        let top = !l.is_empty() && !l.starts_with(char::is_whitespace);
        if !top || l.starts_with('}') || l.starts_with("//") {
            continue;
        }
        if l.starts_with("#[") || l.starts_with("#![") {
            attrs.push(l);
            continue;
        }
        let test_mod = is_mod(l) && attrs.iter().any(|a| a.trim() == "#[cfg(test)]");
        if !test_mod {
            out.push((i + 1, l.to_string()));
        }
        attrs.clear();
    }
    out
}

/// A HAND-WRITTEN DRIVER'S CATCH-ALL over the page's ops (the architect on #413 x #424): every op waits on an answer,
/// so a driver that drops one stalls the page silently. A driver lists the ops it answers or means to leave
/// unanswered BY NAME; anything else ends here, loudly.
#[allow(dead_code)]
#[track_caller]
pub fn unanswered_op(op: &page::Op) -> ! {
    panic!("a test driver dropped an op the page waits on (a silent stall): {op:?}")
}
