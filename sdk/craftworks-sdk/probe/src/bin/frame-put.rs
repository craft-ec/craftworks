//! A PUT frame of a NON-Block contract, as the SDK's own encoder makes it
//! (`wire::frame_put`), base64 per line: the realnet harness's MUTANT for the
//! A-side check ("a view step PUTs a Register → red"). The harness sends these
//! bytes from V's page on a socket of its own — only ever to V, the harness's
//! private node — and the check must fail that window. Never a second encoder.
//!
//! usage: frame-put <code-file> <params-hex> <state-hex>
use anyhow::{bail, Context, Result};
use base64::Engine as _;
use freenet_stdlib::prelude::*;
use std::sync::Arc;

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let [code, params, state] = a.as_slice() else { bail!("usage: frame-put <code-file> <params-hex> <state-hex>") };
    let code = std::fs::read(code).with_context(|| format!("reading {code}"))?;
    let params = core_types::hex::decode(params).context("params: hex")?;
    let state = core_types::hex::decode(state).context("state: hex")?;
    let c = ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(Arc::new(ContractCode::from(code)), Parameters::from(params))));
    for f in wire::frame_put(c, WrappedState::new(state), 1).map_err(anyhow::Error::msg)? {
        println!("{}", base64::engine::general_purpose::STANDARD.encode(f));
    }
    Ok(())
}
