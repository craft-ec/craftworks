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

/// What a keeper's OWN read of a listed file found (`craftworks_files::found` on what did not answer), from its local
/// records: every page computes [`keepers_of`] the same; only which of its candidates a keeper spends its limit on
/// depends on what it read (the statement and standing read only its committed holdings, never its assignment).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    /// Read and there at its listed size: kept FIRST, so no number of new listings displaces it.
    Found,
    /// Read and not there (nothing serves it, or less than it claims): spends none of the limit.
    Absent,
    /// Not read yet: takes what the found files leave, in rank order, until read.
    Unread,
}

/// The files `me` is a keeper of (`keepers_of`), its highest-ranked first — its CANDIDATES.
pub fn candidates(me: &[u8; 32], files: &[([u8; 32], u64)], keepers: &[[u8; 32]], copies: usize) -> Vec<([u8; 32], u64)> {
    let mut mine: Vec<([u8; 32], ([u8; 32], u64))> = files.iter().filter(|(ak, _)| keepers_of(ak, keepers, copies).contains(me)).map(|f| (score(me, &f.0), *f)).collect();
    mine.sort_by(|a, b| b.0.cmp(&a.0));
    mine.dedup_by_key(|x| x.1 .0);
    mine.into_iter().map(|x| x.1).collect()
}

/// The files a keeper KEEPS of its ranked `candidates` within `limit` bytes (THE RULE against listings nothing serves):
/// those it FOUND first, then those not read yet, each in rank order, while their sizes fit (one that does not fit is
/// skipped; a smaller one after it may); those it found ABSENT never. So a fake listing costs a keeper one read, never
/// a place a file it found holds.
pub fn fill(candidates: &[([u8; 32], u64)], limit: u64, read: &dyn Fn(&[u8; 32]) -> Read) -> Vec<([u8; 32], u64)> {
    let mut total = 0u64;
    let mut out = Vec::new();
    for pass in [Read::Found, Read::Unread] {
        for f in candidates.iter().filter(|f| read(&f.0) == pass) {
            if total.saturating_add(f.1) <= limit {
                total += f.1;
                out.push(*f);
            }
        }
    }
    out
}

/// The files `me` is ASSIGNED with nothing read yet: its [`candidates`] [`fill`]ed, all unread.
pub fn assigned(me: &[u8; 32], files: &[([u8; 32], u64)], keepers: &[[u8; 32]], copies: usize, limit: u64) -> Vec<([u8; 32], u64)> {
    fill(&candidates(me, files, keepers, copies), limit, &|_| Read::Unread)
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

    #[test]
    fn fake_listings_never_displace_a_file_a_keeper_found() {
        let keepers: Vec<[u8; 32]> = (0..10u8).map(|i| [i; 32]).collect();
        let me = [5u8; 32];
        let file = |tag: u8, i: u32| (*blake3::hash(&[&[tag][..], &i.to_le_bytes()].concat()).as_bytes(), 1000u64);
        let real: Vec<([u8; 32], u64)> = (0..200).map(|i| file(0, i)).collect();
        let fakes: Vec<([u8; 32], u64)> = (0..5000).map(|i| file(1, i)).collect();
        let is_real = |ak: &[u8; 32]| real.iter().any(|r| r.0 == *ak);
        let limit = 20_000; // 20 files: fewer than its ~60 real candidates — the limit binds
        // What it keeps with no fakes listed at all, every file read.
        let honest = fill(&candidates(&me, &real, &keepers, COPIES), limit, &|_| Read::Found);
        assert_eq!(honest.len(), 20);
        // 25× as many fakes listed as real files: before any read, fakes take places…
        let listed: Vec<([u8; 32], u64)> = real.iter().chain(&fakes).copied().collect();
        let cands = candidates(&me, &listed, &keepers, COPIES);
        assert!(fill(&cands, limit, &|_| Read::Unread).iter().any(|f| !is_real(&f.0)));
        // …then the keeper reads what it was handed (its local records: found or absent) until nothing new is handed.
        let mut read: std::collections::BTreeMap<[u8; 32], Read> = Default::default();
        let mut reads = 0;
        loop {
            let now = fill(&cands, limit, &|ak| read.get(ak).copied().unwrap_or(Read::Unread));
            let unread: Vec<([u8; 32], u64)> = now.iter().filter(|f| !read.contains_key(&f.0)).copied().collect();
            if unread.is_empty() {
                // Every place is a real file — the very ones it keeps with no fakes listed: nothing displaced.
                let (mut a, mut b) = (now.clone(), honest.clone());
                a.sort();
                b.sort();
                assert_eq!(a, b, "fakes displaced real files");
                break;
            }
            for f in unread {
                reads += 1;
                read.insert(f.0, if is_real(&f.0) { Read::Found } else { Read::Absent });
            }
        }
        // Its cost: at most one read per candidate (each fake read once, then never handed again).
        assert!(reads <= cands.len(), "{reads}");
        // A NEW FLOOD of fakes (not read yet) takes no place a found file holds.
        let more: Vec<([u8; 32], u64)> = (0..20_000).map(|i| file(2, i)).collect();
        let listed: Vec<([u8; 32], u64)> = listed.iter().chain(&more).copied().collect();
        let after = fill(&candidates(&me, &listed, &keepers, COPIES), limit, &|ak| read.get(ak).copied().unwrap_or(Read::Unread));
        assert!(honest.iter().all(|f| after.contains(f)), "a found file displaced by unread listings");
    }
}
