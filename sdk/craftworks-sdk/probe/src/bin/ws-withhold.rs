//! WITHHOLD PARITY (the live harness's BACKED_UP control, stock-take proposal 4): a proxy between a page and its node
//! that passes EVERYTHING through, except the node's ANSWER to a PUT of a PARITY block (a Block-contract state whose
//! kind byte is `freenet_prolly::kind::PARITY`). The page re-sends such a PUT until answered (rule 7); here it never
//! is. The data blocks are answered, so the page's commit reaches Published at k of every group -- and never
//! BACKED_UP. A harness assertion that "every write is BACKED_UP" must fail through this proxy; one that stays green
//! is not looking.
//!
//! Requests are read through the probes' one decoder (probe::frames). A connection that is not a WebSocket upgrade (the
//! node's HTTP: web containers, pieces) is piped through unchanged. Every withheld answer is counted on stderr.
//!
//! usage: ws-withhold <listen-port> <node-ws-port>          (both on 127.0.0.1; `probe::node::RESERVED` refused)
use anyhow::{bail, Result};
use probe::proxy::serve;
use probe::withhold::Withhold;
use std::sync::Arc;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let [listen, node] = a.as_slice() else { bail!("usage: ws-withhold <listen-port> <node-ws-port>") };
    let (listen, node) = (listen.parse()?, node.parse()?);
    serve(listen, node, serde_json::json!({ "withholding": "answers to PARITY-block PUTs" }), Arc::new(Withhold::default())).await
}
