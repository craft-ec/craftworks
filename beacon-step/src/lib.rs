//! A PAGE'S HEAVY WORK, in a worker — THE BEACON'S NEXT STEP, and a keeper's SEALED answer hashes (TOKEN §5a: anyone extends it). The sequential work — `K` hashes, one
//! after another — runs off the page's thread; the step is the beacon contract's own (`State::extend`), so a page
//! computes exactly what the contract checks. Built with `--target no-modules`: a CLASSIC worker (the app's frame has
//! an opaque origin, where module workers from a blob fail) runs it.
use craftworks_beacon_contract as B;
use wasm_bindgen::prelude::*;

/// The beacon (its `params`) extended by one step from `state` (empty: none yet — its first), with `mix` (32 fresh random
/// bytes): the new state, encoded. Empty: the params or the state do not read.
#[wasm_bindgen]
pub fn next(params: &[u8], state: &[u8], mix: &[u8]) -> Vec<u8> {
    let (Some(p), Ok(mix)) = (B::Params::decode(params), <[u8; 32]>::try_from(mix)) else { return Vec::new() };
    let s = if state.is_empty() { B::State::default() } else {
        match B::wire::decode(state) {
            Some(s) => s,
            None => return Vec::new(),
        }
    };
    B::wire::encode(&s.extend(&p, mix))
}

/// A keeper's ANSWER HASH for one challenged piece (TOKEN §7a: `craftworks_standing::sealed_hash` — the seal is slow by
/// design, so a page runs it here): 32 bytes, or empty when an argument is not 32 bytes.
#[wasm_bindgen]
pub fn sealed_hash(key: &[u8], id: &[u8], piece: &[u8], out: &[u8], peer: &str) -> Vec<u8> {
    let (Ok(key), Ok(id), Ok(out)) = (<[u8; 32]>::try_from(key), <[u8; 32]>::try_from(id), <[u8; 32]>::try_from(out)) else { return Vec::new() };
    craftworks_standing::sealed_hash(&key, &id, piece, &out, peer).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_step_is_the_contracts_own_and_extends_by_one() {
        let p = B::Params { k: 1 << 10, segs: 16, sample: 2, window: 2, keep: 600, seed: [3; 32] };
        let one = next(&p.encode(), &[], &[1; 32]);
        let two = next(&p.encode(), &one, &[2; 32]);
        let s = B::wire::decode(&two).unwrap();
        assert_eq!(s.height(), 2);
        assert_eq!(B::wire::encode(&B::wire::decode(&one).unwrap().extend(&p, [2; 32])), two, "the contract's own extend");
        assert!(next(b"junk", &[], &[1; 32]).is_empty());
    }
}
