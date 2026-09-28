//! BAG: an unordered SET of items at an address, with no signer — anyone may add one; nobody can take one out. Its first
//! use is a person's INBOX: items sealed to their inbox key, so only they open them. What keeps it from being filled
//! with junk is WORK: an item counts only if the hash of (address ‖ item) starts with [`MIN_WORK`] zero bits, cheap
//! for one message and dear for a million; past the size cap the bag keeps the items with the most work.
//!
//! Params = the address (32 bytes). State = `CWBG ‖ n (u16) ‖ n × (len (u32) ‖ item)`, items sorted by their hash,
//! each at most [`ITEM_MAX`], the whole at most [`STATE_MAX`] and [`ITEMS_MAX`] items. An item = `nonce (8) ‖ payload`.
//! Merge = the union, then (if over a cap) the items with the most work, ties to the lower hash — the same on every
//! host. Summary = the held items' hashes; delta = the items the other side lacks.

pub const MAGIC: &[u8; 4] = b"CWBG";
pub const MIN_WORK: u32 = 12;
pub const ITEM_MAX: usize = 16 * 1024;
pub const STATE_MAX: usize = 256 * 1024;
pub const ITEMS_MAX: usize = 1024;

/// An item's hash at an address (what orders the bag and what its work is counted on).
pub fn hash(address: &[u8], item: &[u8]) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key("craftworks 2026-09-28 bag item");
    h.update(address);
    h.update(item);
    *h.finalize().as_bytes()
}

/// Leading zero bits of an item's hash.
pub fn work(address: &[u8], item: &[u8]) -> u32 {
    let h = hash(address, item);
    let mut n = 0;
    for b in h {
        if b == 0 {
            n += 8;
        } else {
            return n + b.leading_zeros();
        }
    }
    n
}

/// An item for `payload` at `address`: a nonce found so that its work is at least [`MIN_WORK`] (the sender's cost).
pub fn grind(address: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut item = [&[0u8; 8][..], payload].concat();
    for nonce in 0u64.. {
        item[..8].copy_from_slice(&nonce.to_le_bytes());
        if work(address, &item) >= MIN_WORK {
            return item;
        }
    }
    unreachable!()
}

pub fn payload(item: &[u8]) -> &[u8] {
    item.get(8..).unwrap_or_default()
}

fn admitted(address: &[u8], item: &[u8]) -> bool {
    item.len() > 8 && item.len() <= ITEM_MAX && work(address, item) >= MIN_WORK
}

/// A state's items, if it is well formed and every item is admitted (sorted, unique, within the caps).
pub fn read(address: &[u8], state: &[u8]) -> Option<Vec<Vec<u8>>> {
    if address.len() != 32 || state.len() > STATE_MAX {
        return None;
    }
    let mut b = state.strip_prefix(MAGIC)?;
    let (n, r) = b.split_at_checked(2)?;
    b = r;
    let n = u16::from_le_bytes(n.try_into().ok()?) as usize;
    if n > ITEMS_MAX {
        return None;
    }
    let mut items = Vec::with_capacity(n);
    let mut last: Option<[u8; 32]> = None;
    for _ in 0..n {
        let (l, r) = b.split_at_checked(4)?;
        let l = u32::from_le_bytes(l.try_into().ok()?) as usize;
        let (item, r) = r.split_at_checked(l)?;
        b = r;
        if !admitted(address, item) {
            return None;
        }
        let h = hash(address, item);
        if last.is_some_and(|p| p >= h) {
            return None;
        }
        last = Some(h);
        items.push(item.to_vec());
    }
    b.is_empty().then_some(items)
}

pub fn encode(address: &[u8], items: &[Vec<u8>]) -> Vec<u8> {
    let mut sorted: Vec<&Vec<u8>> = items.iter().collect();
    sorted.sort_by_key(|i| hash(address, i));
    let mut b = MAGIC.to_vec();
    b.extend_from_slice(&(sorted.len() as u16).to_le_bytes());
    for i in sorted {
        b.extend_from_slice(&(i.len() as u32).to_le_bytes());
        b.extend_from_slice(i);
    }
    b
}

/// The UNION of `held` and `more` (only admitted items), then — over a cap — the items with the most work, ties to the
/// lower hash. The same result whatever the order the items came in.
pub fn merge(address: &[u8], held: &[Vec<u8>], more: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let mut all: std::collections::BTreeMap<[u8; 32], Vec<u8>> = std::collections::BTreeMap::new();
    for i in held.iter().chain(more).filter(|i| admitted(address, i)) {
        all.insert(hash(address, i), i.clone());
    }
    let mut ranked: Vec<([u8; 32], Vec<u8>)> = all.into_iter().collect();
    ranked.sort_by(|(ha, a), (hb, b)| work(address, b).cmp(&work(address, a)).then(ha.cmp(hb)));
    let mut out = Vec::new();
    let mut size = MAGIC.len() + 2;
    for (_, i) in ranked {
        if out.len() == ITEMS_MAX || size + 4 + i.len() > STATE_MAX {
            continue;
        }
        size += 4 + i.len();
        out.push(i);
    }
    out
}

#[cfg(feature = "freenet-main-contract")]
mod contract {
    use super::*;
    use freenet_stdlib::prelude::*;

    pub struct Bag;

    #[contract]
    impl ContractInterface for Bag {
        fn validate_state(parameters: Parameters<'static>, state: State<'static>, _related: RelatedContracts<'static>) -> Result<ValidateResult, ContractError> {
            Ok(if read(parameters.as_ref(), state.as_ref()).is_some() { ValidateResult::Valid } else { ValidateResult::Invalid })
        }

        fn update_state(parameters: Parameters<'static>, state: State<'static>, data: Vec<UpdateData<'static>>) -> Result<UpdateModification<'static>, ContractError> {
            let a = parameters.as_ref();
            let held = read(a, state.as_ref()).unwrap_or_default();
            let mut more = Vec::new();
            for item in data {
                let given: Option<&[u8]> = match &item {
                    UpdateData::State(s) => Some(s.as_ref()),
                    UpdateData::Delta(d) => Some(d.as_ref()),
                    UpdateData::StateAndDelta { delta, .. } => Some(delta.as_ref()),
                    _ => None,
                };
                // Anything unreadable is ignored, never fatal.
                if let Some(items) = given.and_then(|g| read(a, g)) {
                    more.extend(items);
                }
            }
            Ok(UpdateModification::valid(State::from(encode(a, &merge(a, &held, &more)))))
        }

        fn summarize_state(parameters: Parameters<'static>, state: State<'static>) -> Result<StateSummary<'static>, ContractError> {
            let a = parameters.as_ref();
            let items = read(a, state.as_ref()).ok_or(ContractError::InvalidState)?;
            Ok(StateSummary::from(items.iter().flat_map(|i| hash(a, i)).collect::<Vec<u8>>()))
        }

        fn get_state_delta(parameters: Parameters<'static>, state: State<'static>, summary: StateSummary<'static>) -> Result<StateDelta<'static>, ContractError> {
            let a = parameters.as_ref();
            let items = read(a, state.as_ref()).ok_or(ContractError::InvalidState)?;
            let have: std::collections::BTreeSet<&[u8]> = summary.as_ref().chunks_exact(32).collect();
            let missing: Vec<Vec<u8>> = items.into_iter().filter(|i| !have.contains(&hash(a, i)[..])).collect();
            Ok(StateDelta::from(encode(a, &missing)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: [u8; 32] = [9; 32];

    #[test]
    fn an_item_needs_work_and_the_bag_is_a_union_in_any_order() {
        let x = grind(&A, b"one");
        let y = grind(&A, b"two");
        assert!(work(&A, &x) >= MIN_WORK && payload(&x) == b"one");
        // Control: the same payload with no work is not admitted.
        let lazy = [&[0u8; 8][..], b"junk"].concat();
        let lazy = if work(&A, &lazy) >= MIN_WORK { [&[1u8; 8][..], b"junk"].concat() } else { lazy };
        assert!(work(&A, &lazy) < MIN_WORK);
        assert_eq!(merge(&A, &[], &[lazy]).len(), 0);
        let m1 = merge(&A, &[x.clone()], &[y.clone()]);
        let m2 = merge(&A, &[y.clone()], &[x.clone(), x.clone()]);
        assert_eq!(encode(&A, &m1), encode(&A, &m2), "a union, the same whatever the order");
        let state = encode(&A, &m1);
        assert_eq!(read(&A, &state).unwrap().len(), 2);
        // A state that is not sorted, or holds a duplicate, is refused.
        let mut dup = MAGIC.to_vec();
        dup.extend_from_slice(&2u16.to_le_bytes());
        for i in [&x, &x] {
            dup.extend_from_slice(&(i.len() as u32).to_le_bytes());
            dup.extend_from_slice(i);
        }
        assert!(read(&A, &dup).is_none());
    }

    #[test]
    fn over_the_cap_the_most_work_stays() {
        let big = |n: u8| grind(&A, &vec![n; ITEM_MAX - 8]);
        let items: Vec<Vec<u8>> = (0..20u8).map(big).collect();
        let kept = merge(&A, &[], &items);
        let size: usize = MAGIC.len() + 2 + kept.iter().map(|i| 4 + i.len()).sum::<usize>();
        assert!(size <= STATE_MAX && kept.len() < items.len(), "the cap holds");
        let least_kept = kept.iter().map(|i| work(&A, i)).min().unwrap();
        let most_dropped = items.iter().filter(|i| !kept.contains(i)).map(|i| work(&A, i)).max().unwrap();
        assert!(least_kept >= most_dropped, "nothing kept has less work than anything dropped");
    }
}
