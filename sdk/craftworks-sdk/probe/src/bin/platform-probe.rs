//! What a delegate can actually do on the pinned Freenet, measured.
//!
//! Each answer is a small number with its method printed beside it. This is
//! not a benchmark: one run, one node, one machine, stated as such — the
//! questions are yes/no and order-of-magnitude, and a number that needed a
//! distribution to be believed would not be one of them.
//!
//! It spawns its OWN node on its OWN port with its own temp tree, refuses the
//! ports the user's nodes are on, reaps the child from `Drop`, and removes the
//! tree whatever happens.

use anyhow::{bail, Context, Result};
use freenet_stdlib::client_api::{ClientRequest, DelegateRequest, HostResponse, WebApi};
use freenet_stdlib::prelude::*;
use probe::node::{Node, TempTree};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio::time::timeout;

/// Must match `probe-delegate`'s own enum. Duplicated rather than shared
/// because the delegate is a wasm cdylib and this is a host binary; the
/// bincode shape is the contract between them, and a mismatch shows up as a
/// dropped message rather than a silent wrong answer.
#[derive(Debug, Serialize, Deserialize)]
enum Ask {
    ReadState {
        id: [u8; 32],
    },
    SetSecret {
        key: Vec<u8>,
        len: usize,
    },
    GetSecret {
        key: Vec<u8>,
    },
    Nothing,
    LastPut,
    PutState {
        code: Vec<u8>,
        // The delegate's own enum has this field. It was missing here, so
        // every Q8 message failed to deserialize on the far side and was
        // DROPPED — the exact failure the note above predicts, and the
        // reason Q8's earlier answer meant nothing.
        params: Vec<u8>,
        state: Vec<u8>,
    },
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
        emitted: bool,
    },
    PutResult {
        ok: Option<bool>,
        err: String,
    },
}

const STEP: Duration = Duration::from_secs(10);

/// Collect any delegate replies that arrive on their own over `ms`.
///
/// A `PutContractResponse` reaches the delegate as its own inbound message,
/// so its reply is not the answer to any `ask` -- it turns up out of band or
/// not at all, and "not at all" is itself the finding.
async fn drain_said(client: &mut WebApi, ms: u64) -> Vec<Said> {
    let deadline = Instant::now() + Duration::from_millis(ms);
    let mut out = Vec::new();
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        match timeout(left, client.recv()).await {
            Ok(Ok(HostResponse::DelegateResponse { values, .. })) => {
                for v in values {
                    if let OutboundDelegateMsg::ApplicationMessage(m) = v {
                        if let Ok(s) = bincode::deserialize::<Said>(m.payload.as_slice()) {
                            out.push(s);
                        }
                    }
                }
            }
            Ok(Ok(_)) => continue,
            _ => break,
        }
    }
    out
}

/// Does a client GET for `key` come back with a state?
///
/// On a node with no peers this is the hosting question: a GET the node
/// cannot serve locally has nowhere else to ask.
async fn get_ok(client: &mut WebApi, key: &freenet_stdlib::prelude::ContractKey) -> bool {
    let id = *key.id();
    if timeout(
        STEP,
        client.send(ClientRequest::ContractOp(
            freenet_stdlib::client_api::ContractRequest::Get {
                key: id,
                return_contract_code: false,
                // Both off: a subscribe would REGISTER interest and make the
                // very thing being measured true.
                subscribe: false,
                blocking_subscribe: false,
            },
        )),
    )
    .await
    .is_err()
    {
        return false;
    }
    let deadline = Instant::now() + STEP;
    while Instant::now() < deadline {
        match timeout(STEP, client.recv()).await {
            Ok(Ok(HostResponse::ContractResponse(
                freenet_stdlib::client_api::ContractResponse::GetResponse { state, .. },
            ))) => return !state.as_ref().is_empty(),
            Ok(Ok(_)) => continue,
            _ => return false,
        }
    }
    false
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

    let deadline = Instant::now() + STEP;
    while Instant::now() < deadline {
        match timeout(STEP, client.recv()).await {
            Ok(Ok(HostResponse::DelegateResponse { values, .. })) => {
                for v in values {
                    if let OutboundDelegateMsg::ApplicationMessage(m) = v {
                        if let Ok(said) = bincode::deserialize::<Said>(m.payload.as_slice()) {
                            return Ok(Some(said));
                        }
                    }
                }
                // A response carrying nothing we understand is not an error:
                // the delegate drops what it cannot parse, by design.
                return Ok(None);
            }
            Ok(Ok(_)) => continue,
            Ok(Err(e)) => bail!("client error: {e}"),
            Err(_) => bail!("no response within {STEP:?}"),
        }
    }
    Ok(None)
}

#[tokio::main]
async fn main() -> Result<()> {
    let port: u16 = std::env::var("PROBE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(7711);
    let dir = std::env::temp_dir().join(format!("platform-probe-{}", std::process::id()));
    // PROBE_KEEP leaves the tree behind, for reading the node's own log when
    // a run needs attributing. Off by default: a probe that litters temp
    // trees is one nobody runs twice.
    let tree = TempTree(dir.clone());
    if std::env::var("PROBE_KEEP").is_ok() {
        std::mem::forget(tree);
        println!("node: PROBE_KEEP set — the temp tree will NOT be removed");
    }

    let wasm_path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: platform-probe <probe_delegate.wasm>");
        std::process::exit(2);
    });
    let wasm = std::fs::read(&wasm_path).context("reading the probe delegate")?;
    // The delegate must be one this node will load at all.
    // The same one call the import gate makes. Using `not_defined` alone
    // here checked only the subset half, so this probe would have loaded an
    // unwired delegate and reported every question against a module with no
    // entry point.
    probe::check(&wasm).map_err(|e| anyhow::anyhow!("the probe delegate is refused: {e}"))?;

    println!(
        "node: spawning on 127.0.0.1:{port} (temp tree {})",
        dir.display()
    );
    // PROBE_WS points at a node this probe did NOT spawn, so the same
    // questions can be asked of a node in a different MODE. It exists because
    // `freenet local local` and `freenet network` answer question 8
    // differently, and one probe that can address both is how that was
    // established rather than argued.
    let existing = std::env::var("PROBE_WS").ok();
    let node = match &existing {
        Some(ws) => {
            println!("node: using the node already at {ws} (not spawning one)");
            None
        }
        None => {
            // PROBE_NET spawns an ISOLATED network-mode node instead: the
            // only kind that answers a delegate-write question, and the kind
            // a live acceptance run needs. It joins nothing.
            let m = if std::env::var("PROBE_NET").is_ok() {
                probe::node::Mode::IsolatedNetwork {
                    network_port: port + 20000,
                }
            } else {
                probe::node::Mode::Local
            };
            Some(Node::spawn_in(port, &dir, m)?)
        }
    };
    let ws_url = match (&existing, &node) {
        (Some(ws), _) => ws.clone(),
        (None, Some(n)) => n.ws(),
        _ => unreachable!("one of the two is always set"),
    };
    // A node this probe spawned is `local local` by construction. A borrowed
    // one must be DECLARED, and an undeclared one is not assumed to be the
    // convenient case.
    let mode = match (&existing, &node) {
        // Read off the node this probe actually started, not off the request:
        // the two can differ, and the one that matters is what is running.
        (None, Some(n)) => match n.mode {
            probe::node::Mode::Local => "local".to_string(),
            probe::node::Mode::IsolatedNetwork { .. } => "network".to_string(),
        },
        _ => std::env::var("PROBE_MODE").unwrap_or_else(|_| "unstated".into()),
    };
    println!("node: mode = {mode}");
    println!(
        "note: every answer below was produced by a node in {mode} mode. \
         Question 8 is MEASURED to differ between modes -- a delegate PUT is \
         silently dropped in local mode and works in network mode -- so it is \
         refused outside network mode. The other questions have NOT been \
         compared across modes, so treat a local-mode answer to any of them \
         as unconfirmed rather than as a platform fact."
    );
    let (stream, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .context("connecting to the node")?;
    let mut client = WebApi::start(stream);

    let key = probe::signer::register_delegate(&mut client, &wasm).await?;
    println!("node: probe delegate registered, key {key}");

    // ---- (4) what a call costs ----
    let n = 50;
    let t0 = Instant::now();
    let mut last = None;
    for _ in 0..n {
        last = ask(&mut client, &key, &Ask::Nothing).await?;
    }
    let per_call = t0.elapsed() / n;
    match last {
        Some(Said::Nothing {
            calls_in_memory,
            calls_in_context,
        }) => {
            println!(
                "(4) per call: {:?} over {n} empty round trips \
                 | calls seen in wasm memory: {calls_in_memory} \
                 | calls recorded in context: {calls_in_context}",
                per_call
            );
            println!(
                "    memory persists between calls: {}",
                if calls_in_memory > 1 { "YES" } else { "NO" }
            );
        }
        other => println!("(4) per call: {per_call:?}; delegate said {other:?}"),
    }

    // ---- (5) the secret store ----
    for len in [4 * 1024usize, 1024 * 1024] {
        let k = format!("probe-{len}").into_bytes();
        let t = Instant::now();
        let set = ask(
            &mut client,
            &key,
            &Ask::SetSecret {
                key: k.clone(),
                len,
            },
        )
        .await?;
        let set_ms = t.elapsed();
        let t = Instant::now();
        let got = ask(&mut client, &key, &Ask::GetSecret { key: k.clone() }).await?;
        let get_ms = t.elapsed();
        println!("(5) secret {len} B: set {set_ms:?} {set:?} | get {get_ms:?} {got:?}");
    }

    // Only a node this probe spawned may be restarted. Against a borrowed
    // one the question is not asked, and says so: a skipped measurement that
    // prints nothing is one a reader counts as answered.
    let mut node = node;
    let mut client = match node.as_mut() {
        Some(n) => {
            println!("(5) restarting the node to see whether secrets survive...");
            drop(client);
            n.restart()?;
            let (stream, _) = tokio_tungstenite::connect_async(n.ws()).await?;
            WebApi::start(stream)
        }
        None => {
            println!(
                "(5) NOT ASKED: this probe did not spawn the node, and it \
                 restarts only its own"
            );
            client
        }
    };
    // A restarted node has forgotten the delegate registration, not the secret.
    let delegate = DelegateContainer::Wasm(DelegateWasmAPIVersion::V1(Delegate::from((
        &DelegateCode::from(wasm.clone()),
        &Parameters::from(vec![]),
    ))));
    let _ = timeout(
        STEP,
        client.send(ClientRequest::DelegateOp(
            DelegateRequest::RegisterDelegate {
                delegate,
                cipher: [0u8; 32],
                nonce: [0u8; 24],
            },
        )),
    )
    .await;
    let _ = timeout(STEP, client.recv()).await;
    let after = ask(
        &mut client,
        &key,
        &Ask::GetSecret {
            key: b"probe-4096".to_vec(),
        },
    )
    .await?;
    println!("(5) after restart, the 4 KiB secret reads back as: {after:?}");

    // ---- (1) is a state this node just accepted sync-readable, and when? ----
    //
    // The block contract validates `params == blake3(state)`, so a valid pair
    // is trivial to make and the node will accept it.
    let block_wasm = std::env::args().nth(2).unwrap_or_else(|| {
        format!(
            "{}/dev/craftworks/freenet-contracts/build/block.wasm",
            std::env::var("HOME").unwrap_or_default()
        )
    });
    let Ok(code_bytes) = std::fs::read(&block_wasm) else {
        println!("(1)(8) SKIPPED: no block contract at {block_wasm}");
        println!("done");
        return Ok(());
    };

    let mut state = vec![0u8]; // kind RAW
    state.extend_from_slice(b"probe-q1-state");
    let params: Vec<u8> = blake3::hash(&state).as_bytes().to_vec();
    let container = ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(
        std::sync::Arc::new(ContractCode::from(code_bytes.clone())),
        Parameters::from(params.clone()),
    )));
    let id: [u8; 32] = container
        .key()
        .id()
        .as_bytes()
        .try_into()
        .expect("32-byte instance id");

    let t = Instant::now();
    timeout(
        STEP,
        client.send(ClientRequest::ContractOp(
            freenet_stdlib::client_api::ContractRequest::Put {
                contract: container.clone(),
                state: WrappedState::from(state.clone()),
                related_contracts: RelatedContracts::default(),
                subscribe: false,
                blocking_subscribe: false,
            },
        )),
    )
    .await
    .map_err(|_| anyhow::anyhow!("put: node stopped accepting"))??;
    // Drain until the PUT is answered.
    let mut put_ok = false;
    let deadline = Instant::now() + STEP;
    while Instant::now() < deadline {
        match timeout(STEP, client.recv()).await {
            Ok(Ok(HostResponse::ContractResponse(
                freenet_stdlib::client_api::ContractResponse::PutResponse { .. },
            ))) => {
                put_ok = true;
                break;
            }
            Ok(Ok(_)) => continue,
            _ => break,
        }
    }
    let put_ms = t.elapsed();
    println!("(1) client PUT answered: {put_ok} after {put_ms:?}");

    // Poll the delegate's SYNC read on a stated grid.
    let grid = [0u64, 10, 25, 50, 100, 250, 500, 1000, 2000, 5000];
    let mut first_seen: Option<u64> = None;
    for ms in grid {
        if ms > 0 {
            tokio::time::sleep(Duration::from_millis(ms - first_seen.unwrap_or(0))).await;
        }
        let said = ask(&mut client, &key, &Ask::ReadState { id }).await?;
        let got = matches!(said, Some(Said::State { len: Some(_), .. }));
        println!("(1)   t+{ms:>5} ms after PUT ack: sync read sees it = {got}");
        if got {
            first_seen = Some(ms);
            break;
        }
        first_seen = Some(ms);
    }
    match first_seen {
        Some(ms) => println!("(1) first sync-readable at or before t+{ms} ms (grid: {grid:?})"),
        None => println!("(1) never sync-readable within the grid"),
    }

    // ---- (8) does a DELEGATE-originated PUT end up hosted and readable? ----
    //
    // NOT ASKED on a local-mode node. Measured: on `freenet local local` a
    // delegate PUT is never acknowledged, is not readable and is not served,
    // while the SAME probe against a network-mode node on the same machine
    // gets yes to all three -- and F21's own probe reports put=0/k here and
    // k/k there. So the local answer is not a NO, it is a node that does not
    // answer, and printing NO would have put a false platform fact in a log
    // that reads exactly like a real one. It already did, for one report.
    //
    // A node this probe spawned is `local local` by construction. A borrowed
    // one must be declared, and an undeclared one is not assumed to be the
    // convenient case.
    if mode != "network" {
        println!(
            "(8) NOT ASKED: this node is {}, and a delegate PUT is only \
             answered by a node in NETWORK mode. Point PROBE_WS at one and \
             set PROBE_MODE=network.",
            match mode.as_str() {
                "local" => "in local mode",
                _ => "of an unstated mode",
            }
        );
        println!("done");
        return Ok(());
    }
    let mut s8 = vec![0u8];
    s8.extend_from_slice(b"probe-q8-delegate-put");
    let p8: Vec<u8> = blake3::hash(&s8).as_bytes().to_vec();
    let c8 = ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(
        std::sync::Arc::new(ContractCode::from(code_bytes.clone())),
        Parameters::from(p8.clone()),
    )));
    let id8: [u8; 32] = c8
        .key()
        .id()
        .as_bytes()
        .try_into()
        .expect("32-byte instance id");
    println!("(8) delegate-put contract key: {}", c8.key());
    println!("(8) client-put  contract key: {}", container.key());
    let before = ask(&mut client, &key, &Ask::ReadState { id: id8 }).await?;
    println!("(8) before the delegate PUT, sync read sees it: {before:?}");
    let ran = ask(
        &mut client,
        &key,
        &Ask::PutState {
            code: code_bytes.clone(),
            params: p8.clone(),
            state: s8.clone(),
        },
    )
    .await?;
    assert!(
        ran.is_some(),
        "(8) the delegate answered nothing: the Ask shape does not match its \
         own enum, so the message was dropped and Q8 measured nothing"
    );
    println!("(8) the delegate ran and built the request: {ran:?}");
    // Drained FIRST. Every sync-read poll below is an `ask`, which consumes
    // whatever is waiting on the socket -- so a response that arrived early
    // would have been eaten and counted as "never came".
    let spontaneous = drain_said(&mut client, 2000).await;
    println!("(8a) delegate replies that arrived on their own: {spontaneous:?}");
    let mut sync_ok = false;
    for ms in [50u64, 200, 500, 1000, 3000] {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        let said = ask(&mut client, &key, &Ask::ReadState { id: id8 }).await?;
        sync_ok = matches!(said, Some(Said::State { len: Some(_), .. }));
        println!(
            "(8)   +{ms} ms after the delegate PUT: sync read sees it = {got}",
            got = sync_ok
        );
        if sync_ok {
            break;
        }
    }
    // The node's own verdict, rather than an inference from what is readable
    // afterwards. "Not readable" has two causes -- refused, or accepted and
    // not visible -- and only this tells them apart.
    let verdict = ask(&mut client, &key, &Ask::LastPut).await?;
    println!("(8a) the node's verdict on the delegate PUT: {verdict:?}");
    println!("(8a) sync-readable on the next call: {sync_ok}");

    // ---- (8b) is it HOSTED? ----
    //
    // Source says no: a delegate PUT goes to
    // `executor().upsert_contract_state_deferrable(...)` and never reaches
    // `register_local_hosting`, whose only production caller is the client
    // PUT operation. A GET for a held key is served locally only if
    // `has_local_interest` — hosting, a local client, or a downstream
    // subscriber — and a delegate PUT creates none of those.
    //
    // On a node with no peers that is decidable: a GET that cannot be served
    // locally has nowhere to go. The CONTROL is the same GET for the contract
    // put through the CLIENT path in (1), which does register hosting. Without
    // it, a failing GET would say nothing — it could just mean GETs do not
    // work on a solo node.
    let control = get_ok(&mut client, &container.key().clone()).await;
    let subject = get_ok(&mut client, &c8.key().clone()).await;
    println!("(8b) GET of the CLIENT-put contract  (control): {control}");
    println!("(8b) GET of the DELEGATE-put contract        : {subject}");
    match (control, subject, sync_ok) {
        (true, true, _) => {
            println!("(8b) hosted: YES — a delegate PUT is served locally like a client PUT")
        }
        (true, false, true) => println!(
            "(8b) hosted: NO — the bytes ARE local (8a said yes) but the node \
             does not host them, so a later GET leaves the machine"
        ),
        (true, false, false) => println!(
            "(8) a delegate PUT DID NOTHING OBSERVABLE on this build: emitted \
             by the delegate, no PutContractResponse by either channel, not \
             sync-readable, not gettable — while a client PUT in this same \
             run was sync-readable at t+0 ms and is gettable"
        ),
        (false, _, _) => println!(
            "(8b) INCONCLUSIVE — the control GET failed too, so this says \
             nothing about the delegate PUT"
        ),
    }

    // ---- (9) does a delegate-written state SURVIVE a node restart? ----
    //
    // The hosting cache is in-memory interest state; the contract store is
    // not. So "hosted" and "still there tomorrow" are different questions,
    // and a state that is hosted now may be readable-but-unhosted, or gone,
    // after a restart. Only a node this probe started can be restarted.
    match node.as_mut() {
        None => println!(
            "(9) NOT ASKED: this probe did not spawn the node, and it \
             restarts only its own"
        ),
        Some(n) => {
            println!("(9) restarting the node...");
            drop(client);
            n.restart()?;
            let (stream, _) = tokio_tungstenite::connect_async(n.ws()).await?;
            let mut client = WebApi::start(stream);
            // The delegate registration does not survive; the secrets and the
            // contract store do. Re-register so the sync read can be asked.
            let delegate = DelegateContainer::Wasm(DelegateWasmAPIVersion::V1(Delegate::from((
                &DelegateCode::from(wasm.clone()),
                &Parameters::from(vec![]),
            ))));
            let _ = timeout(
                STEP,
                client.send(ClientRequest::DelegateOp(
                    DelegateRequest::RegisterDelegate {
                        delegate,
                        cipher: [0u8; 32],
                        nonce: [0u8; 24],
                    },
                )),
            )
            .await;
            let _ = timeout(Duration::from_secs(2), client.recv()).await;

            let said = ask(&mut client, &key, &Ask::ReadState { id: id8 }).await?;
            let sync_after = matches!(said, Some(Said::State { len: Some(_), .. }));
            let get_after = get_ok(&mut client, &c8.key().clone()).await;
            // The control: the CLIENT-put state, through the same restart.
            // Without it, "gone" would not distinguish a delegate-written
            // state being dropped from the node losing everything.
            let control_after = get_ok(&mut client, &container.key().clone()).await;
            println!("(9) after restart — delegate-put: sync read {sync_after}, GET {get_after}");
            println!("(9) after restart — client-put  (control): GET {control_after}");
            match (control_after, get_after) {
                (true, true) => println!(
                    "(9) a delegate-written state survives a restart, like a \
                     client-written one"
                ),
                (true, false) => println!(
                    "(9) a delegate-written state does NOT survive a restart \
                     while a client-written one does — they are not on the \
                     same footing"
                ),
                (false, _) => println!(
                    "(9) INCONCLUSIVE — the control did not survive either, so \
                     this says nothing about the delegate-written one"
                ),
            }
        }
    }

    println!("done");
    Ok(())
}
