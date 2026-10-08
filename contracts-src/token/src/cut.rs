//! THE WRITER'S CUT (TOKEN §2): from the writer's own whole record of its chain, the body of a cut to append after it,
//! with the blocks of its two trees — to publish, so the history stays walkable — and the evidence an entry citing
//! something behind a cut carries. Never in the contract: it builds; the contract only verifies.

use crate::{received_key, walk, Body, ChainParams, Definition, Entry, State, Why, CLAIMED, FILLED, RECEIVED};
use freenet_prolly::{build::TreeBuilder, proof::prove, Cid};
use std::collections::BTreeMap;

/// The blocks of a cut's trees, by id.
pub type Blocks = BTreeMap<Cid, Vec<u8>>;

/// The received-set keys of a chain's entries (every name received, filled or claimed), in key order.
pub fn received_keys(entries: &[Entry]) -> Vec<Vec<u8>> {
    let mut k: Vec<Vec<u8>> = entries
        .iter()
        .filter_map(|e| match &e.body {
            Body::Receive { from, nonce, .. } => Some(received_key(RECEIVED, from, nonce)),
            Body::Fill { taker, lock, .. } => Some(received_key(FILLED, taker, lock)),
            Body::Claim { taker, lock, .. } => Some(received_key(CLAIMED, taker, lock)),
            _ => None,
        })
        .collect();
    k.sort();
    k.dedup();
    k
}

/// The two trees of `entries` — the WHOLE chain from seq 0, nothing pruned: (history root, received root), their blocks
/// added to `blocks`.
pub fn trees(entries: &[Entry], blocks: &mut Blocks) -> Result<(Cid, Cid), Why> {
    let mut h = TreeBuilder::new(|c, b: &[u8]| {
        blocks.insert(c, b.to_vec());
    });
    for e in entries {
        h.push_bytes(&crate::history_key(e.seq), &e.encode()).map_err(|_| "a history that does not build")?;
    }
    let history = h.finish().map_err(|_| "a history that does not build")?;
    let mut r = TreeBuilder::new(|c, b: &[u8]| {
        blocks.insert(c, b.to_vec());
    });
    for k in received_keys(entries) {
        r.push_bytes(&k, &[]).map_err(|_| "a received set that does not build")?;
    }
    let received = r.finish().map_err(|_| "a received set that does not build")?;
    Ok((history, received))
}

/// The body of a cut after `whole` (the writer's whole chain from seq 0) and the blocks of its trees: what the chain
/// carries there (checked: the walk of the shape rules, no citations — those were checked as each came), the roots.
pub fn make(p: &ChainParams, d: &Definition, whole: &State) -> Result<(Body, Blocks), Why> {
    if whole.entries.first().is_some_and(|e| e.seq != 0) {
        return Err("a cut made from a chain that is not whole");
    }
    if whole.fork.is_some() {
        return Err("a frozen chain is never cut");
    }
    let c = walk(p, d, whole, &|_, _| None, 0, usize::MAX).map_err(|(_, w)| w)?;
    if c.allowances.len() > 1024 || c.offers.len() > 1024 || c.locks.len() > 1024 {
        return Err("more to carry than a cut holds");
    }
    let mut blocks = Blocks::new();
    let (history, received) = trees(&whole.entries, &mut blocks)?;
    Ok((Body::Cut { history, received, allowances: c.allowances, offers: c.offers, locks: c.locks }, blocks))
}

/// A tree's blocks, to prove from.
struct Store<'a>(&'a Blocks);
impl freenet_prolly::store::Blocks for Store<'_> {
    fn get(&self, cid: &Cid) -> Option<&[u8]> {
        self.0.get(cid).map(Vec::as_slice)
    }
}

/// A proof of `key` under `root` (present or absent), from the tree's blocks.
pub fn proof(blocks: &Blocks, root: &Cid, key: &[u8]) -> Result<Vec<u8>, Why> {
    // (An entry stored by reference: the proof carries its id and length; the evidence carries its bytes.)
    Ok(prove(&Store(blocks), root, key).map_err(|_| "a tree whose blocks are not all here")?.encode())
}

/// A cut's HISTORY read back: every entry before it, seq 0 on, from its tree's blocks — fetched by id (`fetch`; each
/// block must be the one its id names), at most `max_blocks` of them. Refused unless the leaves are exactly seqs 0, 1,
/// 2… each holding an entry of that seq.
pub fn history(root: &Cid, fetch: &dyn Fn(&Cid) -> Option<Vec<u8>>, max_blocks: usize) -> Result<Vec<Entry>, Why> {
    use freenet_prolly::range::{range, read_value, PageEnd, Range};
    use freenet_prolly::store::{MemBlocks, ReadError};
    use freenet_prolly::{block_id, kind};
    let mut store = MemBlocks::default();
    let mut fetched = 0usize;
    let mut get = |store: &mut MemBlocks, need: Vec<Cid>| -> Result<(), Why> {
        for c in need {
            fetched += 1;
            if fetched > max_blocks {
                return Err("a history too long to read");
            }
            let b = fetch(&c).ok_or("a history whose blocks are not there")?;
            if block_id(kind::TREE_NODE, &b) != c && block_id(kind::RAW, &b) != c {
                return Err("a history block that is not the one named");
            }
            store.insert(c, &b);
        }
        Ok(())
    };
    let mut out: Vec<Entry> = Vec::new();
    let mut after: Option<Vec<u8>> = None;
    loop {
        // One page; its values resolved, or what to fetch first (the page is read again after).
        let (got, need, end, next) = {
            let page = range(&store, root, &Range { after: after.clone(), ..Range::default() }).map_err(|_| "a history that does not read")?;
            let mut got = Vec::new();
            let mut need = page.need.clone();
            for (k, v) in &page.entries {
                match read_value(&store, *v) {
                    Ok(b) => got.push((k.clone(), b.to_vec())),
                    Err(ReadError::Need(c)) => need.extend(c),
                    Err(_) => return Err("a history that does not read"),
                }
            }
            let values_missing = got.len() < page.entries.len();
            (if values_missing { Vec::new() } else { got }, need, if values_missing { PageEnd::Blocked } else { page.end }, page.next.clone())
        };
        for (k, b) in &got {
            let seq = out.len() as u64;
            let e = Entry::decode(b).ok_or("a history entry that does not decode")?;
            if k[..] != crate::history_key(seq) || e.seq != seq {
                return Err("a history that is not every seq in order");
            }
            out.push(e);
        }
        match end {
            PageEnd::EndOfTree | PageEnd::EndOfRange => return Ok(out),
            PageEnd::Limit => after = next,
            PageEnd::Blocked => {
                if let Some((k, _)) = got.last() {
                    after = Some(k.clone());
                }
                if need.is_empty() {
                    return Err("a history that does not read");
                }
                get(&mut store, need)?;
            }
        }
    }
}

/// The WHOLE chain behind a live copy: the history of its first cut (if it starts at one) and the live entries after.
pub fn whole(s: &State, fetch: &dyn Fn(&Cid) -> Option<Vec<u8>>, max_blocks: usize) -> Result<State, Why> {
    match s.entries.first() {
        Some(Entry { seq, body: Body::Cut { history: root, .. }, .. }) if *seq > 0 => {
            let mut h = history(root, fetch, max_blocks)?;
            if h.len() as u64 != *seq {
                return Err("a cut whose history is not everything before it");
            }
            h.extend(s.entries.iter().cloned());
            Ok(State { entries: h, fork: s.fork.clone() })
        }
        _ => Ok(s.clone()),
    }
}

/// AUDIT the cuts of a whole chain (seq 0 on): each one's trees are the ones its prefix builds. `Err`: the first that
/// is not (its seq).
pub fn audit(whole: &State) -> Result<(), (u64, Why)> {
    for (i, e) in whole.entries.iter().enumerate() {
        if let Body::Cut { history, received, .. } = &e.body {
            let (h, r) = trees(&whole.entries[..i], &mut Blocks::new()).map_err(|w| (e.seq, w))?;
            if h != *history || r != *received {
                return Err((e.seq, "a cut whose trees are not what came before it"));
            }
        }
    }
    Ok(())
}
