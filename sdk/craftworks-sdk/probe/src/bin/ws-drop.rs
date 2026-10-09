//! DROP BLOCKS AT PUBLISH (`probe::drop`, sdk#479): a proxy between a WRITER's page and its node that hands on
//! everything except every `N`th Block-contract PUT (and any re-send of one), which it answers itself with a
//! `PutResponse`: those blocks are never stored on the network, as if the node acked them and lost them. A read of the
//! tree afterwards finds them MISSING -- REPAIR's real-network step. Each dropped block is a JSON line on stderr.
//!
//! usage: ws-drop <listen-port> <node-ws-port> --every N     (both on 127.0.0.1; `probe::node::RESERVED` refused)
use anyhow::{bail, Result};
use probe::drop::DropEvery;
use probe::proxy::serve;
use std::sync::Arc;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    const USAGE: &str = "usage: ws-drop <listen-port> <node-ws-port> --every N";
    let a: Vec<String> = std::env::args().skip(1).collect();
    let [listen, node, e, n] = a.as_slice() else { bail!(USAGE) };
    if e != "--every" {
        bail!(USAGE);
    }
    let every: usize = n.parse()?;
    if every < 2 {
        bail!("--every {every}: at least 2 (dropping every block publishes nothing); {USAGE}");
    }
    serve(listen.parse()?, node.parse()?, serde_json::json!({ "dropping": format!("every {every}th Block PUT") }), Arc::new(DropEvery::new(every))).await
}
