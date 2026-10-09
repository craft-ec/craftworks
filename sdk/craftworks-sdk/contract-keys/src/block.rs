//! A Block contract's instance id, derived as the node derives it.
//!
//! A block's cid is the Block contract's PARAMS (`blake3(kind ‖ body)`); the
//! node names the INSTANCE from the contract's code AND those params, so the
//! two are different 32-byte values. The page, the signer and the probes all
//! name a block's contract through this: one derivation, no second copy.
//!
//! The derivation is restated here — `blake3(blake3(code) ‖ params)`,
//! freenet-stdlib's `CodeHash::from_code` then `generate_id` — rather than
//! linked, so this crate stays off the client-API allowlist (the boundary gate
//! in `probe`). What keeps the restatement honest is the test below, which
//! pins it to the stdlib's own `ContractContainer::key()` as a
//! dev-dependency: a different derivation fails there, not on a live node.

use freenet_prolly::Cid;

/// The contract instance a block lives in.
pub fn contract_for(code: &[u8], cid: &Cid) -> [u8; 32] {
    contract_deriver(code)(cid)
}

/// `contract_for` with the code hashed ONCE. Hashing ~100 KiB of contract code
/// per candidate is what a map was avoiding; hashing it once per call makes
/// each candidate one 64-byte blake3.
pub fn contract_deriver(code: &[u8]) -> impl Fn(&Cid) -> Cid {
    contract_deriver_of_hash(crate::code_hash(code))
}

/// `contract_deriver` from the code's HASH (sdk#334: the signer keeps the
/// hash, never the code).
pub fn contract_deriver_of_hash(code_hash: [u8; 32]) -> impl Fn(&Cid) -> Cid {
    move |cid: &Cid| crate::instance(&code_hash, cid)
}

/// The Block contract's STATE for a block: `kind ‖ body`, with the kind
/// RECOVERED by trying every kind in the one list ([`BlockKind::ALL`](core_types::kind::BlockKind::ALL)) — one
/// BLAKE3 pass each, and exact, because the id is a hash over the kind byte and only the right one can match.
/// `None`: the id does not hash these bytes under any kind, and the contract would refuse it.
pub fn block_state(id: &Cid, body: &[u8]) -> Option<Vec<u8>> {
    for k in core_types::kind::BlockKind::ALL.map(|k| k.byte()) {
        if freenet_prolly::block_id(k, body) == *id {
            let mut v = Vec::with_capacity(1 + body.len());
            v.push(k);
            v.extend_from_slice(body);
            return Some(v);
        }
    }
    None
}

/// The inverse, for a GET answer: a Block contract's state is `kind ‖ body`,
/// and the block id it holds is computed from it, never looked up — a state
/// that hashes to something nobody asked for is caught by the caller's
/// verify-by-id rather than trusted because a map said so.
pub fn block_of_state(state: &[u8]) -> Option<(Cid, &[u8])> {
    let (&kind, body) = state.split_first()?;
    Some((freenet_prolly::block_id(kind, body), body))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The node's derivation, from the node's own library.
    fn stdlib_contract_for(code: &[u8], cid: &Cid) -> [u8; 32] {
        use freenet_stdlib::prelude::*;
        let c = ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(
            std::sync::Arc::new(ContractCode::from(code.to_vec())),
            Parameters::from(cid.to_vec()),
        )));
        let mut out = [0u8; 32];
        out.copy_from_slice(&c.key().id().as_bytes()[..32]);
        out
    }

    #[test]
    fn the_derivation_is_the_node_s_derivation() {
        let code = vec![0x5Au8; 4096];
        let derive = contract_deriver(&code);
        for i in 0..8u8 {
            let cid = [i; 32];
            assert_eq!(derive(&cid), stdlib_contract_for(&code, &cid), "cid {i}");
            assert_eq!(contract_for(&code, &cid), stdlib_contract_for(&code, &cid), "cid {i}");
        }
        // And it depends on the code: another code, another id.
        assert_ne!(contract_deriver(&[1u8; 16])(&[0u8; 32]), derive(&[0u8; 32]));
    }
}
