//! AN ORDERED LOG's ROWS — ONE rule (10-15, measured on fx: the node wrote a commit as `c/<n>` while pages read only
//! `c/<n>~<tag>`, so no page ever saw a node's commit — its members stayed an epoch behind, its joiner on a branch of
//! its own — and the node, checking `c/<n>` only, could commit where a page already had: a fork). A position's entries
//! are rows `<prefix><position, 12 digits>~<tag>`, each writer's under its own key (two writers racing both land), and
//! the position's entry is the LOWEST of them — the same one wherever both are read. The page's `ordering` tail (through
//! the core) and the identity delegate's upkeep both use these.

/// The rows of position `n` begin with this.
pub fn position_prefix(prefix: &str, n: u64) -> String {
    format!("{prefix}{n:012}~")
}

/// A writer's row for position `n` (`tag`: its own, so racing writers never overwrite each other).
pub fn row_key(prefix: &str, n: u64, tag: &str) -> String {
    format!("{}{tag}", position_prefix(prefix, n))
}

/// Position `n`'s entry among `rows` (key, value): the lowest value of its rows; None: the position is free.
pub fn entry_at<'a>(rows: impl IntoIterator<Item = (&'a [u8], &'a [u8])>, prefix: &str, n: u64) -> Option<&'a [u8]> {
    let p = position_prefix(prefix, n);
    rows.into_iter().filter(|(k, v)| k.starts_with(p.as_bytes()) && !v.is_empty()).map(|(_, v)| v).min()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_position_is_its_lowest_entry_whoever_wrote_it() {
        let a = row_key("c/", 7, "aa");
        let b = row_key("c/", 7, "bb");
        assert_eq!(a, "c/000000000007~aa");
        let rows = [(a.as_bytes(), b"{\"commit\":\"02\"}".as_slice()), (b.as_bytes(), b"{\"commit\":\"01\"}".as_slice()), (b"c/000000000008~cc".as_slice(), b"x".as_slice())];
        assert_eq!(entry_at(rows, "c/", 7), Some(b"{\"commit\":\"01\"}".as_slice()));
        assert_eq!(entry_at(rows, "c/", 9), None);
        // A row without a writer's tag (the node's before 10-15) is not a position's entry.
        let bare = [(b"c/000000000007".as_slice(), b"y".as_slice())];
        assert_eq!(entry_at(bare, "c/", 7), None);
    }
}
