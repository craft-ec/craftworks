//! THE ASSIGNMENT (REWARDS §4 one funnel, KA): which keepers keep which REGISTERED file — no coordinator: each file's
//! keepers are the [`COPIES`] listed keepers ranking highest by `BLAKE3(keeper key ‖ file's address key)` (rendezvous:
//! everyone computes the same from the same lists; a keeper joining or leaving moves only the files it ranks for, so
//! the assignment holds from epoch to epoch while the keepers do). A keeper takes its files in its own rank order while
//! their sizes fit the limit its owner sets.

/// Copies of every registered file the network keeps (its base target; paid copies come later).
pub const COPIES: usize = 3;

fn score(key: &[u8; 32], address_key: &[u8; 32]) -> [u8; 32] {
    blake3::derive_key("craftworks 2026-10-10 keeper assignment", &[&key[..], address_key].concat())
}

/// The keepers of the file whose address key is `address_key`: the `copies` of `keepers` ranking highest.
pub fn keepers_of(address_key: &[u8; 32], keepers: &[[u8; 32]], copies: usize) -> Vec<[u8; 32]> {
    let mut ranked: Vec<([u8; 32], [u8; 32])> = keepers.iter().map(|k| (score(k, address_key), *k)).collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0));
    ranked.dedup_by_key(|x| x.1);
    ranked.into_iter().take(copies).map(|x| x.1).collect()
}

/// The files `me` is ASSIGNED of `files` (address key, size): those it is among the keepers of, highest-ranked
/// first, while their sizes fit `limit` (a file that does not fit is skipped; a smaller one after it may).
pub fn assigned(me: &[u8; 32], files: &[([u8; 32], u64)], keepers: &[[u8; 32]], copies: usize, limit: u64) -> Vec<([u8; 32], u64)> {
    let mut mine: Vec<([u8; 32], ([u8; 32], u64))> = files.iter().filter(|(ak, _)| keepers_of(ak, keepers, copies).contains(me)).map(|f| (score(me, &f.0), *f)).collect();
    mine.sort_by(|a, b| b.0.cmp(&a.0));
    let mut total = 0u64;
    let mut out = Vec::new();
    for (_, f) in mine {
        if total.saturating_add(f.1) <= limit {
            total += f.1;
            out.push(f);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_file_has_its_copies_and_a_keeper_leaving_moves_only_its_own() {
        let keepers: Vec<[u8; 32]> = (0..10u8).map(|i| [i; 32]).collect();
        let files: Vec<([u8; 32], u64)> = (0..200u32).map(|i| (*blake3::hash(&i.to_le_bytes()).as_bytes(), 1000)).collect();
        // Each file: COPIES distinct keepers; every keeper's assignment (no limit) is exactly the files it keeps.
        for (ak, _) in &files {
            let ks = keepers_of(ak, &keepers, COPIES);
            assert_eq!(ks.len(), COPIES);
            for k in &keepers {
                assert_eq!(ks.contains(k), assigned(k, &[(*ak, 1000)], &keepers, COPIES, u64::MAX).len() == 1);
            }
        }
        let load: Vec<usize> = keepers.iter().map(|k| assigned(k, &files, &keepers, COPIES, u64::MAX).len()).collect();
        assert_eq!(load.iter().sum::<usize>(), files.len() * COPIES);
        assert!(load.iter().all(|n| (30..=90).contains(n)), "spread: {load:?}");
        // Keeper 3 leaves: a file it did not keep keeps its keepers.
        let without: Vec<[u8; 32]> = keepers.iter().copied().filter(|k| *k != [3; 32]).collect();
        for (ak, _) in &files {
            let (before, after) = (keepers_of(ak, &keepers, COPIES), keepers_of(ak, &without, COPIES));
            if !before.contains(&[3; 32]) {
                assert_eq!(before, after);
            } else {
                assert_eq!(before.iter().filter(|k| after.contains(k)).count(), COPIES - 1);
            }
        }
        // The limit: highest-ranked first, while they fit.
        let me = [5u8; 32];
        let all = assigned(&me, &files, &keepers, COPIES, u64::MAX);
        let some = assigned(&me, &files, &keepers, COPIES, 3500);
        assert_eq!(some, all[..3].to_vec());
        assert!(assigned(&me, &files, &keepers, COPIES, 0).is_empty());
        // Fewer keepers than copies: every keeper keeps every file.
        assert_eq!(assigned(&me, &files, &[me], COPIES, u64::MAX).len(), files.len());
    }
}
