use super::*;

fn bytes(n: usize, seed: u8) -> Vec<u8> {
    let mut x = seed as u32 ^ 0x9E37;
    (0..n)
        .map(|_| {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (x >> 16) as u8
        })
        .collect()
}
fn content(b: &[u8]) -> [u8; 32] {
    *blake3::hash(b).as_bytes()
}
/// A whole file coded: its plan, key, every generation's fragments, the index pieces and the root's hash.
struct Up {
    plan: Plan,
    key: [u8; 32],
    frags: Vec<Vec<(u8, Piece)>>,
    index: Vec<Piece>,
    root: [u8; 32],
}
fn upload(file: &[u8], salt: Option<&[u8; 32]>) -> Up {
    let plan = Plan::of(file.len() as u64);
    let key = content_key(&content(file), salt);
    let frags: Vec<_> = (0..plan.gens)
        .map(|g| {
            let (a, b) = plan.range(g);
            encode(&key, &plan, g, &file[a as usize..b as usize], EXTRA)
        })
        .collect();
    let stored: Vec<Listed> = frags.iter().map(|f| f.iter().map(|(j, p)| (*j, p.hash())).collect()).collect();
    let (index, root) = index(&key, &plan, &stored);
    Up { plan, key, frags, index, root }
}
/// Read it back as a reader holds only the reference: the root, the leaf, then fragments `pick(g)` per generation.
fn download(u: &Up, pick: impl Fn(u64, usize) -> Vec<usize>) -> Result<Vec<u8>, Error> {
    let at = |addr: [u8; 32]| u.index.iter().find(|p| p.address == addr).unwrap().state.clone();
    let root = Root::open(&u.key, &u.root, &at(root_address(&u.key)))?;
    assert_eq!(root.depth, 0);
    let mut out = Vec::new();
    for g in 0..root.plan.gens {
        let leaf = Root::leaf_of(g);
        let listed = open_leaf(&u.key, &root.plan, leaf, &root.children[leaf as usize], &at(index_address(&u.key, 0, leaf)))?;
        let l = listed[(g % GENS_PER_LEAF as u64) as usize].clone();
        let mut d = Decoder::new(&u.key, &root.plan, g, l);
        for i in pick(g, u.frags[g as usize].len()) {
            let (j, p) = &u.frags[g as usize][i];
            match d.add(*j, &p.state) {
                Ok(()) | Err(Error::Redundant) => {}
                Err(e) => return Err(e),
            }
            if d.done() {
                break;
            }
        }
        out.extend(d.plain()?);
    }
    Ok(out)
}

#[test]
fn chunks_adapt_so_a_file_codes_about_sixteen() {
    assert_eq!(Plan::of(100 * 1024).chunk, MIN_CHUNK);
    let p = Plan::of(2 * 1024 * 1024);
    assert_eq!((p.chunk, p.chunks, p.gens), (128 * 1024, 16, 1));
    let big = Plan::of(10 * 1024 * 1024 * 1024);
    assert_eq!(big.chunk, MAX_CHUNK);
    assert_eq!(big.gens, 10 * 1024 * 4 / GEN as u64);
}

#[test]
fn a_file_reads_back_from_its_first_sixteen_fragments_with_no_decoding() {
    let file = bytes(5 * 1024 * 1024 + 777, 1);
    let u = upload(&file, None);
    assert_eq!(download(&u, |_, n| (0..n).collect()).unwrap(), file);
}

#[test]
fn any_sixteen_of_twenty_four_rebuild_a_generation() {
    let file = bytes(3 * 1024 * 1024, 2);
    let u = upload(&file, None);
    // Only coded fragments first, then data ones — the worst order; and every chunk except a few lost.
    for lost in [vec![0, 1, 2, 3, 4, 5, 6, 7], vec![8, 9, 10, 11, 12, 13, 14, 15], vec![0, 3, 5, 9, 11, 14, 15, 2]] {
        let got = download(&u, |_, n| (0..n).rev().filter(|i| !lost.contains(i)).collect()).unwrap();
        assert_eq!(got, file, "lost {lost:?}");
    }
    // Fifteen are not enough.
    let d = download(&u, |_, _| (0..15).collect());
    assert!(d.is_err());
}

#[test]
fn a_minted_fragment_replaces_one_that_was_never_stored() {
    let file = bytes(1024 * 1024, 3);
    let plan = Plan::of(file.len() as u64);
    let key = content_key(&content(&file), None);
    let sent = encode(&key, &plan, 0, &file, EXTRA);
    let extra = mint(&key, &plan, 0, &file, 30);
    // Stored: every fragment but three data ones, plus the minted one.
    let mut stored: Vec<(u8, Piece)> = sent.into_iter().filter(|(j, _)| ![1, 4, 9].contains(j)).collect();
    stored.push((30, extra));
    let listed: Listed = stored.iter().map(|(j, p)| (*j, p.hash())).collect();
    let mut d = Decoder::new(&key, &plan, 0, listed);
    for (j, p) in stored.iter().rev() {
        let _ = d.add(*j, &p.state);
    }
    assert_eq!(d.plain().unwrap(), file);
}

#[test]
fn a_forged_fragment_or_index_is_refused() {
    let file = bytes(600 * 1024, 4);
    let u = upload(&file, None);
    let root_state = &u.index.last().unwrap().state;
    let mut bad = root_state.clone();
    *bad.last_mut().unwrap() ^= 1;
    assert_eq!(Root::open(&u.key, &u.root, &bad).unwrap_err(), Error::Forged);
    assert_eq!(Root::open(&[9; 32], &u.root, root_state).unwrap_err(), Error::Forged, "another key opens nothing");
    // A fragment with a flipped byte: not what the index lists.
    let listed: Listed = u.frags[0].iter().map(|(j, p)| (*j, p.hash())).collect();
    let mut d = Decoder::new(&u.key, &u.plan, 0, listed);
    let mut f = u.frags[0][0].1.state.clone();
    f[100] ^= 0xFF;
    assert_eq!(d.add(0, &f).unwrap_err(), Error::Forged);
    // A genuine one twice: the second adds nothing.
    d.add(0, &u.frags[0][0].1.state).unwrap();
    assert_eq!(d.add(0, &u.frags[0][0].1.state).unwrap_err(), Error::Redundant);
}

#[test]
fn a_seek_reads_one_chunk_alone() {
    let file = bytes(8 * 1024 * 1024 + 5, 5);
    let u = upload(&file, None);
    let i = 21u64; // generation 1, chunk 5
    let g = (i / GEN as u64) as usize;
    let listed: Listed = u.frags[g].iter().map(|(j, p)| (*j, p.hash())).collect();
    let state = &u.frags[g][(i % GEN as u64) as usize].1.state;
    let got = read_chunk(&u.key, &u.plan, &listed, i, state).unwrap();
    let from = i as usize * u.plan.chunk;
    assert_eq!(got, file[from..from + u.plan.chunk]);
}

#[test]
fn the_same_content_in_the_same_space_is_the_same_file_and_another_space_s_is_not() {
    let file = bytes(700 * 1024, 6);
    let salt_a = [1u8; 32];
    let (a1, a2, b) = (upload(&file, Some(&salt_a)), upload(&file, Some(&salt_a)), upload(&file, Some(&[2; 32])));
    assert_eq!(a1.key, a2.key);
    assert_eq!(a1.root, a2.root, "a resumed or repeated upload makes the very same file");
    assert_eq!(a1.frags[0][3].1, a2.frags[0][3].1);
    assert_ne!(a1.key, b.key, "nobody outside a space can test whether a file is in it");
    assert_ne!(a1.key, upload(&file, None).key, "a public file is keyed by its content alone");
    assert_eq!(upload(&file, None).key, upload(&file, None).key, "public files dedup network-wide");
}

#[test]
fn a_last_generation_of_a_few_chunks_codes_too() {
    let file = bytes(17 * MIN_CHUNK * 17 + 3, 7); // chunks not a multiple of 16
    let u = upload(&file, None);
    let last = u.plan.gens - 1;
    assert!(u.plan.k(last) < GEN);
    let k = u.plan.k(last);
    // Last generation: drop its data fragments, decode from coded ones only.
    let got = download(&u, |g, n| if g == last { (k..n).collect() } else { (0..n).collect() }).unwrap();
    assert_eq!(got, file);
}

#[test]
fn a_huge_file_s_index_grows_an_inner_level() {
    // No bytes needed to test the tree: a plan with more leaves than the root lists.
    let key = [3u8; 32];
    let gens = (GENS_PER_LEAF * (FANOUT + 5)) as u64;
    let plan = Plan { size: gens * (GEN * MAX_CHUNK) as u64, chunk: MAX_CHUNK, chunks: gens * GEN as u64, gens };
    let stored: Vec<Listed> = (0..gens).map(|g| vec![(0u8, *blake3::hash(&g.to_be_bytes()).as_bytes())]).collect();
    let (pieces, root_hash) = index(&key, &plan, &stored);
    let at = |a: [u8; 32]| pieces.iter().find(|p| p.address == a).unwrap().state.clone();
    let root = Root::open(&key, &root_hash, &at(root_address(&key))).unwrap();
    assert_eq!(root.depth, 1);
    // The last generation, through the inner level down to its leaf.
    let g = gens - 1;
    let leaf = Root::leaf_of(g);
    let path = root.path(leaf);
    assert_eq!(path, vec![(1, leaf / FANOUT as u64), (0, leaf)]);
    let inner = open_inner(&key, 1, path[0].1, &root.children[path[0].1 as usize], &at(index_address(&key, 1, path[0].1))).unwrap();
    let leaf_hash = inner[(leaf % FANOUT as u64) as usize];
    let listed = open_leaf(&key, &plan, leaf, &leaf_hash, &at(index_address(&key, 0, leaf))).unwrap();
    assert_eq!(listed[(g % GENS_PER_LEAF as u64) as usize], stored[g as usize]);
    // Every piece fits a sealed contract.
    assert!(pieces.iter().all(|p| p.state.len() <= 256 * 1024 + 1024), "{}", pieces.iter().map(|p| p.state.len()).max().unwrap());
}

#[test]
fn every_fragment_fits_a_sealed_contract() {
    let u = upload(&bytes(64 * 1024 * 1024 + 1, 8), None);
    let max = u.frags.iter().flatten().map(|(_, p)| p.state.len()).max().unwrap();
    assert!(max <= 256 * 1024 + 1024, "{max}");
}
