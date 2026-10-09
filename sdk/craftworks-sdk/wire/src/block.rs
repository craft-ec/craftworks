//! A block as the network holds it: the contract it lives in, and the state
//! that contract stores.
//!
//! MOVED HERE from engine-delegate (`blocks::contract_for`,
//! `blocks::contract_deriver`, `entry::block_state`) for the engine-in-the-page
//! shape (ENGINE-SHAPE.md §6): the PAGE now puts and gets blocks itself, and
//! this crate is the one place allowed to know freenet's types. The page and
//! the signer must name a block's contract identically — a different
//! derivation is a different contract — so this derivation is pinned equal to
//! the signer's (`contract_keys::block`, which restates it without the
//! stdlib) by `wire/tests/block.rs`. engine-delegate, which had its own copy,
//! is deleted (the switch-over).

use freenet_prolly::Cid;
use freenet_stdlib::prelude::*;

/// The contract instance a block lives in.
///
/// A block's cid is the Block contract's PARAMS — `blake3(kind ‖ body)` — and
/// the instance id is derived by the node from the code AND the params, so
/// the two are different 32-byte values.
pub fn contract_for(code: &[u8], cid: &Cid) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(&block_contract(code, cid).key().id().as_bytes()[..32]);
    out
}

/// `contract_for` with the code hashed ONCE: matching answers to blocks
/// hashes ~100 KiB of contract code per candidate otherwise. The ONE
/// restatement of the node's derivation is `contract_keys` (sdk#334); this is
/// it, pinned equal to `contract_for` (the stdlib's own) by the tests.
pub fn contract_deriver(code: &[u8]) -> impl Fn(&Cid) -> Cid {
    contract_keys::block::contract_deriver(code)
}

/// The Block contract, instantiated for the block whose id is `cid`.
pub fn block_contract(code: &[u8], cid: &Cid) -> ContractContainer {
    ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(
        std::sync::Arc::new(ContractCode::from(code.to_vec())),
        Parameters::from(cid.to_vec()),
    )))
}

/// The Block contract's STATE for a block (`kind ‖ body`), and its inverse: sans-IO, so their ONE home is
/// `contract_keys::block` (a delegate reads and writes blocks too); re-exported here.
pub use contract_keys::block::{block_of_state, block_state};
