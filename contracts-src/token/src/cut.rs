//! THE WRITER'S CUT (TOKEN §2): from the writer's own whole record of its chain, the body of a cut to append after it,
//! with the blocks of its two trees — to publish, so the history stays walkable — and the evidence an entry citing
//! something behind a cut carries. Never in the contract: it builds; the contract only verifies.

use crate::{received_key, walk, Body, ChainParams, Definition, Entry, State, Why, MINTED, MINTER, RECEIVED};
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
            Body::FeeClaim { from, nonce, .. } => Some(crate::received_key(crate::FEE, from, nonce)),
            Body::Mint { nonce, .. } | Body::CappedMint { nonce, .. } => Some(received_key(MINTED, &MINTER, nonce)),
            Body::Vested { from, vest, step, .. } => Some(crate::vested_key(from, vest, *step)),
            Body::Payout { program, key, .. } => Some(crate::received_key(crate::PAYOUT, program, key)),
            Body::HolderPayout { program, sold, copy, .. } => Some(crate::received_key(crate::HOLDERS_SHARE, program, &crate::holder_nonce(sold, *copy))),
            _ => None,
        })
        .collect();
    k.sort();
    k.dedup();
    k
}

/// One tree's root and its root's parity ids.
pub type Rooted = (Cid, Vec<Cid>);

/// A tree from (key, value bytes), WITH PARITY — every group of nodes coded, and the root's own group of one — its
/// blocks (data, values, parity) added to `blocks`: a reader racing a block against its group never waits on a lost one.
fn tree(items: impl IntoIterator<Item = (Vec<u8>, Vec<u8>)>, blocks: &mut Blocks, why: Why) -> Result<Rooted, Why> {
    let mut b = TreeBuilder::new(|c, x: &[u8]| {
        blocks.insert(c, x.to_vec());
    });
    for (k, v) in items {
        b.push_bytes(&k, &v).map_err(|_| why)?;
    }
    let (root, parity) = b.finish_with_parity().map_err(|_| why)?;
    blocks.extend(parity);
    let root_bytes = blocks.get(&root).cloned().ok_or(why)?;
    let rp = engine::repair::root_parity(&root_bytes).ok_or(why)?;
    let ids = rp.iter().map(|(c, _)| *c).collect();
    blocks.extend(rp);
    Ok((root, ids))
}

/// The two trees of `entries` — the WHOLE chain from seq 0, nothing pruned: the history and the received set, each
/// rooted with its root's parity; their blocks added to `blocks`.
pub fn trees(entries: &[Entry], blocks: &mut Blocks) -> Result<(Rooted, Rooted), Why> {
    let history = tree(entries.iter().map(|e| (crate::history_key(e.seq).to_vec(), e.encode())), blocks, "a history that does not build")?;
    let received = tree(received_keys(entries).into_iter().map(|k| (k, Vec::new())), blocks, "a received set that does not build")?;
    Ok((history, received))
}

/// The body of a cut after `whole` (the writer's whole chain from seq 0) and the blocks of its trees: what the chain
/// carries there (checked: the walk of the shape rules, no citations — those were checked as each came), the roots.
pub fn make(p: &ChainParams, d: &Definition, whole: &State, keys: Option<&craftworks_idlog_contract::Log>) -> Result<(Body, Blocks), Why> {
    if whole.entries.first().is_some_and(|e| e.seq != 0) {
        return Err("a cut made from a chain that is not whole");
    }
    if whole.fork.is_some() {
        return Err("a frozen chain is never cut");
    }
    let c = walk(p, d, whole, &|_, _| None, &|_| None, keys, 0, usize::MAX, &crate::Extra::NONE).map_err(|(_, w)| w)?;
    if c.allowances.len() > 1024 {
        return Err("more to carry than a cut holds");
    }
    let mut blocks = Blocks::new();
    let ((history, history_parity), (received, received_parity)) = trees(&whole.entries, &mut blocks)?;
    Ok((Body::Cut { history, received, history_parity, received_parity, allowances: c.allowances, writer: c.writer, staked: c.staked, unbonding: c.unbonding, retired: c.retired }, blocks))
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

/// Why a history did not read: blocks not HELD yet (fetch them through the blocks door — raced against their groups,
/// rebuilt from parity — and read again), or refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read {
    Need(Vec<Cid>),
    Refused(Why),
}

/// A cut's HISTORY read back from what is HELD (`held`: a block by its id, never a network read): every entry before
/// it, seq 0 on. `Need`: the blocks to fetch first. Refused unless the leaves are exactly seqs 0, 1, 2… each holding
/// an entry of that seq, and every block the one its id names.
pub fn history(root: &Cid, held: &dyn Fn(&Cid) -> Option<Vec<u8>>) -> Result<Vec<Entry>, Read> {
    use freenet_prolly::range::{range, read_value, PageEnd, Range};
    use freenet_prolly::store::{MemBlocks, ReadError};
    use freenet_prolly::{block_id, kind};
    let mut store = MemBlocks::default();
    let mut out: Vec<Entry> = Vec::new();
    let mut after: Option<Vec<u8>> = None;
    loop {
        let (got, need, end, next) = {
            let page = range(&store, root, &Range { after: after.clone(), ..Range::default() }).map_err(|_| Read::Refused("a history that does not read"))?;
            let mut got = Vec::new();
            let mut need = page.need.clone();
            for (k, v) in &page.entries {
                match read_value(&store, *v) {
                    Ok(b) => got.push((k.clone(), b.to_vec())),
                    Err(ReadError::Need(c)) => need.extend(c),
                    Err(_) => return Err(Read::Refused("a history that does not read")),
                }
            }
            let complete = got.len() == page.entries.len();
            (if complete { got } else { Vec::new() }, need, if complete { page.end } else { PageEnd::Blocked }, page.next.clone())
        };
        for (k, b) in &got {
            let seq = out.len() as u64;
            let e = Entry::decode(b).ok_or(Read::Refused("a history entry that does not decode"))?;
            if k[..] != crate::history_key(seq) || e.seq != seq {
                return Err(Read::Refused("a history that is not every seq in order"));
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
                    return Err(Read::Refused("a history that does not read"));
                }
                // Taken from what is held; what is not held is NEEDED (never fetched here).
                let mut missing = Vec::new();
                for c in need {
                    match held(&c) {
                        Some(b) if block_id(kind::TREE_NODE, &b) == c || block_id(kind::RAW, &b) == c => store.insert(c, &b),
                        Some(_) => return Err(Read::Refused("a history block that is not the one named")),
                        None => missing.push(c),
                    }
                }
                if !missing.is_empty() {
                    return Err(Read::Need(missing));
                }
            }
        }
    }
}

/// The WHOLE chain behind a live copy: the history of its first cut (if it starts at one) and the live entries after.
pub fn whole(s: &State, held: &dyn Fn(&Cid) -> Option<Vec<u8>>) -> Result<State, Read> {
    match s.entries.first() {
        Some(Entry { seq, body: Body::Cut { history: root, .. }, .. }) if *seq > 0 => {
            let mut h = history(root, held)?;
            if h.len() as u64 != *seq {
                return Err(Read::Refused("a cut whose history is not everything before it"));
            }
            h.extend(s.entries.iter().cloned());
            Ok(State { entries: h, fork: s.fork.clone() })
        }
        _ => Ok(s.clone()),
    }
}

/// AUDIT the cuts of a whole chain (seq 0 on): each one's trees — roots and their parity — are the ones its prefix
/// builds. `Err`: the first that is not (its seq).
pub fn audit(whole: &State) -> Result<(), (u64, Why)> {
    for (i, e) in whole.entries.iter().enumerate() {
        if let Body::Cut { history, received, history_parity, received_parity, .. } = &e.body {
            let ((h, hp), (r, rp)) = trees(&whole.entries[..i], &mut Blocks::new()).map_err(|w| (e.seq, w))?;
            if h != *history || r != *received || hp != *history_parity || rp != *received_parity {
                return Err((e.seq, "a cut whose trees are not what came before it"));
            }
        }
    }
    Ok(())
}
