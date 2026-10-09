//! THE DETECT CHECK for sdk#433 (WORKAROUNDS; re-run at every freenet version bump): a private node is sent a Block
//! PUT whose state does not hash to its id -- the Block contract must refuse it as invalid -- and the node's words
//! must be one of `wire::VALIDATION_REFUSED`, on the version they were read on (`probe::verdict::refusal_text`).
//!
//! usage: PAGE_PORT=<port> PAGE_TMP=<dir> live-refusal-text <block.wasm>
use anyhow::{bail, Context, Result};
use futures::{SinkExt, StreamExt};
use probe::node::{Mode, Node, TempTree};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let v = std::process::Command::new("freenet").arg("--version").output().context("freenet --version")?;
    let version = String::from_utf8_lossy(&v.stdout).lines().next().unwrap_or_default().to_string();
    let port: u16 = std::env::var("PAGE_PORT").ok().and_then(|p| p.parse().ok()).context("PAGE_PORT=<port> is required")?;
    let tmp = std::env::var("PAGE_TMP").context("PAGE_TMP=<dir> is required")?;
    let block_code = std::fs::read(std::env::args().nth(1).context("usage: live-refusal-text <block.wasm>")?)?;
    let dir = std::path::PathBuf::from(&tmp).join(format!("live-refusal-text-{}", std::process::id()));
    let _tree = TempTree(dir.clone());
    let node = Node::spawn_in(port, &dir, Mode::IsolatedNetwork { network_port: port + 1 })?;
    let (mut sock, _) = tokio_tungstenite::connect_async(node.ws()).await.context("connecting")?;
    // A block whose state does not hash to its id: kind RAW over bytes the id is NOT the hash of.
    let id = [7u8; 32];
    let mut state = vec![freenet_prolly::kind::RAW];
    state.extend_from_slice(b"these bytes do not hash to the id");
    for f in wire::frame_put(wire::block::block_contract(&block_code, &id), freenet_stdlib::prelude::WrappedState::new(state), 1).map_err(anyhow::Error::msg)? {
        sock.send(Message::Binary(f.into())).await?;
    }
    let mut seen = wire::Reassembler::default();
    let mut said = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while said.is_none() && tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(5), sock.next()).await {
            Ok(Some(Ok(Message::Binary(b)))) => {
                if let Ok(Err(e)) = bincode::deserialize::<Result<freenet_stdlib::client_api::HostResponse, freenet_stdlib::client_api::ClientError>>(&b) {
                    println!("(the node's error, whole: {:?})", e.kind());
                }
                let incoming = wire::unframe(&mut seen, &b);
                match incoming {
                wire::Incoming::PutFailed { said: s, .. } => said = Some(s),
                // The keyless form 0.2.136/0.2.138 use (sdk#433): the FORMAT is pinned too, and the key it names
                // must be the one page-io keys this PUT by.
                wire::Incoming::PutFailedByText { key, said: s } => {
                    let ours = wire::block::block_contract(&block_code, &id).key().to_string();
                    if key != ours {
                        bail!("the node's text named contract {key}, not this PUT's {ours}: the format's key is not the one the page keys its PUTs by");
                    }
                    println!("the node's words, in the pinned format: key {key} (this PUT's), reason {s:?}");
                    said = Some(s)
                }
                wire::Incoming::Ack(_) => bail!("the node ACCEPTED a Block PUT whose state does not hash to its id"),
                other => println!("(another answer: {other:?})"),
                }
            }
            Ok(Some(Ok(_))) | Err(_) => {}
            Ok(Some(Err(e))) => bail!("the socket: {e}"),
            Ok(None) => bail!("the node closed the socket"),
        }
    }
    drop(node);
    match probe::verdict::refusal_text(&version, said.as_deref()) {
        Ok(line) => {
            println!("GREEN: {version}: {line}");
            Ok(())
        }
        Err(why) => {
            println!("RED: {why}");
            bail!(why)
        }
    }
}
