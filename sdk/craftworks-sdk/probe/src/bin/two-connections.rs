//! DO TWO CONNECTIONS TO ONE DELEGATE SHARE A CONTEXT?
//!
//! The whole shape of the two-tab acceptance rests on this. Two tabs on one
//! node invoke the SAME delegate — same code, same parameters, so the same
//! key — and the delegate is rebuilt from its context on every call (F32).
//! If that context is ONE thing, two tabs are served by one engine state and
//! tab B sees tab A's write because they are, underneath, the same engine.
//! If it is one per connection, they are two engines that happen to share a
//! node, and a client standing on an old root has no way to be moved.
//!
//! Measured rather than assumed: `probe-delegate` counts its calls IN ITS
//! CONTEXT, which is the only thing that survives a call. Connection A calls
//! it N times, then connection B — a separate websocket — calls it once. If
//! B's count continues A's, the context is shared.
//!
//! The control is in the same run: B's IN-MEMORY count, which must restart
//! at 1 whatever the context does, because linear memory is fresh every call
//! (F32). A run where both counts continue would mean the probe is not
//! measuring what it claims.

use anyhow::{Context, Result};
use freenet_stdlib::client_api::{ClientRequest, DelegateRequest, HostResponse, WebApi};
use freenet_stdlib::prelude::*;
use probe::node::{Mode, Node, TempTree};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::timeout;

const STEP: Duration = Duration::from_secs(10);

#[derive(Debug, Serialize, Deserialize)]
enum Ask {
    ReadState { id: [u8; 32] },
    SetSecret { key: Vec<u8>, len: usize },
    GetSecret { key: Vec<u8> },
    Nothing,
    LastPut,
}

#[derive(Debug, Serialize, Deserialize)]
enum Said {
    State {
        len: Option<usize>,
        head: Vec<u8>,
    },
    Secret {
        ok: bool,
        len: Option<usize>,
    },
    Nothing {
        calls_in_memory: u32,
        calls_in_context: u32,
    },
    Put {
        ok: bool,
        err: String,
    },
}

async fn ask(client: &mut WebApi, key: &DelegateKey, a: &Ask) -> Result<Option<Said>> {
    let payload = bincode::serialize(a)?;
    timeout(
        STEP,
        client.send(ClientRequest::DelegateOp(
            DelegateRequest::ApplicationMessages {
                key: key.clone(),
                params: vec![].into(),
                inbound: vec![InboundDelegateMsg::ApplicationMessage(
                    ApplicationMessage::new(payload),
                )],
            },
        )),
    )
    .await
    .map_err(|_| anyhow::anyhow!("the node stopped accepting requests"))??;

    let deadline = std::time::Instant::now() + STEP;
    while std::time::Instant::now() < deadline {
        match timeout(STEP, client.recv()).await {
            Ok(Ok(HostResponse::DelegateResponse { values, .. })) => {
                for v in values {
                    if let OutboundDelegateMsg::ApplicationMessage(m) = v {
                        if let Ok(s) = bincode::deserialize::<Said>(m.payload.as_ref()) {
                            return Ok(Some(s));
                        }
                    }
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(e)) => anyhow::bail!("node error: {e}"),
            Err(_) => break,
        }
    }
    Ok(None)
}

async fn connect(ws: &str) -> Result<WebApi> {
    let (stream, _) = tokio_tungstenite::connect_async(ws)
        .await
        .context("connecting to the node")?;
    Ok(WebApi::start(stream))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let wasm = std::env::args()
        .nth(1)
        .context("usage: two-connections <probe_delegate.wasm> [ws_port]")?;
    let port: u16 = std::env::args()
        .nth(2)
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(17509);
    let wasm = std::fs::read(&wasm).context("reading the probe delegate")?;

    let dir = std::env::temp_dir().join(format!("two-conn-{}", std::process::id()));
    let _tree = TempTree(dir.clone());
    // NETWORK mode, isolated: a delegate question answered in local mode is
    // not a platform fact (F35 was measured to differ between them).
    let node = Node::spawn_in(
        port,
        &dir,
        Mode::IsolatedNetwork {
            network_port: port + 20000,
        },
    )?;
    println!("node: isolated NETWORK mode on 127.0.0.1:{port}, pid recorded by the harness");

    // ---- connection A registers the delegate and calls it N times ----
    let mut a = connect(&node.ws()).await?;
    let key = probe::signer::register_delegate(&mut a, &wasm).await?;
    println!("delegate: registered, key {key}");

    const N: u32 = 5;
    let mut a_last = None;
    for _ in 0..N {
        a_last = ask(&mut a, &key, &Ask::Nothing).await?;
    }
    let (a_mem, a_ctx) = match a_last {
        Some(Said::Nothing {
            calls_in_memory,
            calls_in_context,
        }) => (calls_in_memory, calls_in_context),
        other => anyhow::bail!("connection A got no count: {other:?}"),
    };

    // ---- a SECOND connection, same delegate, same parameters ----
    let mut b = connect(&node.ws()).await?;
    let b_first = ask(&mut b, &key, &Ask::Nothing).await?;
    let (b_mem, b_ctx) = match b_first {
        Some(Said::Nothing {
            calls_in_memory,
            calls_in_context,
        }) => (calls_in_memory, calls_in_context),
        other => anyhow::bail!("connection B got no count: {other:?}"),
    };

    // ---- and back to A, to see whether B's call was visible to it ----
    let a_again = ask(&mut a, &key, &Ask::Nothing).await?;
    let a_ctx2 = match a_again {
        Some(Said::Nothing {
            calls_in_context, ..
        }) => calls_in_context,
        other => anyhow::bail!("connection A got no second count: {other:?}"),
    };

    println!("\n  connection | calls_in_context | calls_in_memory");
    println!("  -----------|------------------|----------------");
    println!("  A x{N}      | {a_ctx:<16} | {a_mem}");
    println!("  B x1       | {b_ctx:<16} | {b_mem}");
    println!("  A again    | {a_ctx2:<16} | -");

    // THE CONTROL: in-memory must restart every call whatever the context
    // does. If it did not, this probe is not measuring what it claims (F32).
    if a_mem != 1 || b_mem != 1 {
        println!("\n  CONTROL FAILED: calls_in_memory is {a_mem}/{b_mem}, not 1.");
        println!("  Linear memory is supposed to be fresh every call (F32); if it is not,");
        println!("  the context reading below is not trustworthy either.");
        return Ok(());
    }

    let shared = b_ctx > N;
    println!(
        "\n  VERDICT: the context is {} across connections.",
        if shared { "SHARED" } else { "PER-CONNECTION" }
    );
    if shared {
        println!(
            "  B's first call continued A's count ({b_ctx} after A reached {a_ctx}), and A's next\n  \
             call saw {a_ctx2}. Two tabs on one node are served by ONE engine state."
        );
    } else {
        println!(
            "  B's first call started its own count ({b_ctx} while A had reached {a_ctx}).\n  \
             Two tabs on one node are two engines, and a client standing on an old root\n  \
             has no way to be moved by another's write."
        );
    }
    Ok(())
}
