//! A FIXED LIST's POSITIONAL PROOF: a binary merkle tree over a sorted list of ids — `root`, the `path` proving entry
//! `i`, and `verify` that an id is entry `i` of `count` under a root. The one home for "sample a fixed list by POSITION"
//! (a node's holdings, challenged at random indexes: `craftworks-standing`). A prolly tree (this crate's main tree) is
//! for KEYED MAPS that change; proving "the k-th key" there is a presence proof plus a count proof of what lies below
//! it — measured 10-10 at ~9–20 KB a pick against 224–480 B here (100–25,000 ids): too large for a list sampled many
//! times an epoch. (The hash domains are the standing crate's, where this was first written: its roots unchanged.)

fn leaf(id: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key("craftworks 2026-10-09 standing leaf");
    h.update(id);
    *h.finalize().as_bytes()
}
fn node(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key("craftworks 2026-10-09 standing node");
    h.update(a);
    h.update(b);
    *h.finalize().as_bytes()
}
fn level(xs: &[[u8; 32]]) -> Vec<[u8; 32]> {
    xs.chunks(2).map(|c| if c.len() == 2 { node(&c[0], &c[1]) } else { c[0] }).collect()
}

/// The root over `ids` (sorted, distinct): what [`Holdings`] commits to.
pub fn root(ids: &[[u8; 32]]) -> [u8; 32] {
    let mut xs: Vec<[u8; 32]> = ids.iter().map(leaf).collect();
    if xs.is_empty() {
        return [0; 32];
    }
    while xs.len() > 1 {
        xs = level(&xs);
    }
    xs[0]
}

/// The path proving `ids[i]` under [`root`]: the sibling at each level (none where a level's last node has none).
pub fn path(ids: &[[u8; 32]], mut i: usize) -> Vec<Option<[u8; 32]>> {
    let mut xs: Vec<[u8; 32]> = ids.iter().map(leaf).collect();
    let mut out = Vec::new();
    while xs.len() > 1 {
        let j = i ^ 1;
        out.push(xs.get(j).copied());
        xs = level(&xs);
        i /= 2;
    }
    out
}

/// `id` is entry `i` of `count` under `root`.
pub fn verify(root_: &[u8; 32], count: u32, mut i: usize, id: &[u8; 32], path: &[Option<[u8; 32]>]) -> bool {
    if i >= count as usize {
        return false;
    }
    let mut x = leaf(id);
    let mut n = count as usize;
    for p in path {
        if n <= 1 {
            return false;
        }
        x = match (i % 2, p) {
            (0, Some(s)) => node(&x, s),
            (1, Some(s)) => node(s, &x),
            // The last node of an odd level rises alone.
            (0, None) if i == n - 1 => x,
            _ => return false,
        };
        i /= 2;
        n = n.div_ceil(2);
    }
    n == 1 && &x == root_
}


#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u32) -> [u8; 32] {
        *blake3::hash(&n.to_le_bytes()).as_bytes()
    }

    #[test]
    fn every_entry_proves_under_its_root() {
        for n in [1usize, 2, 3, 5, 8, 13] {
            let mut ids: Vec<[u8; 32]> = (0..n as u32).map(id).collect();
            ids.sort();
            let r = root(&ids);
            for i in 0..n {
                assert!(verify(&r, n as u32, i, &ids[i], &path(&ids, i)), "{n} entries, entry {i}");
                assert!(!verify(&r, n as u32, i, &id(999), &path(&ids, i)), "another id");
                if n > 1 {
                    assert!(!verify(&r, n as u32, (i + 1) % n, &ids[i], &path(&ids, i)), "another place");
                }
            }
            assert!(!verify(&r, n as u32, n, &ids[0], &path(&ids, 0)), "past the count");
        }
    }
}
