//! PUTs of contracts the APP names — code, params and state it chose: the
//! contract and the key the node names it by. Where each PUT stands is the
//! PAGE's (`page::AppPut`): it sends, re-sends and ends it like every op.
//!
//! The builder publishes a web container this way (builder#104): the SDK
//! frames it and learns the outcome, and the app never touches the client API.
//!
//! **Matched by the KEY the answer names, never by position** (page-io):
//! acks and refusals arrive on one connection with no correlation id, beside
//! the engine's own PUTs; pairing by order would let a block's ack confirm the
//! app's container.
//!
//! **An ack is not durability** (see [`crate::Incoming::Ack`]): `Put` means the
//! node accepted it, not that anyone can read it. A publisher that must know
//! reads it back.

use freenet_stdlib::prelude::{
    ContractCode, ContractContainer, ContractWasmAPIVersion, Parameters, WrappedContract, WrappedState,
};

/// The contract, its key as the node NAMES it, and the state to put.
pub fn contract(code: &[u8], params: &[u8], state: &[u8]) -> (String, ContractContainer, WrappedState) {
    let c = ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(
        std::sync::Arc::new(ContractCode::from(code.to_vec())),
        Parameters::from(params.to_vec()),
    )));
    (c.key().id().encode(), c, WrappedState::new(state.to_vec()))
}
