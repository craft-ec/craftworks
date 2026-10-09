//! THE ONE PROXY, DRIVEN (engineer1 on sdk#525): both probes' hooks through `probe::proxy::serve` over real loopback
//! sockets, against a fake node -- so a change to the shared bridge cannot silently alter either probe's evidence.
//! * ws-withhold: the node's answer to a PARITY PUT never reaches the page; a DATA PUT's does.
//! * ws-lose: once the head names the root's group, the node's answer to a GET of the root reaches the page as the
//!   node's own NotFound for that contract.
//!
//! And the evidence lines each hook REALLY says, captured from its log and pinned byte for byte (Codex on #541: a test
//! of the formatters alone stays green with the hooks' own lines removed).
use freenet_prolly::kind;
use freenet_stdlib::client_api::{ClientError, ContractResponse, HostResponse};
use freenet_stdlib::prelude::*;
use futures::{SinkExt, StreamExt};
use probe::lose::{short, Lose, LoseHooks, Target};
use probe::proxy::{serve, Hooks, Log};
use probe::withhold::Withhold;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

/// A log that keeps every line a hook says, as the harness would read it from stderr.
fn captured() -> (Log, Arc<Mutex<Vec<String>>>) {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let keep = lines.clone();
    (Log::to(move |l| keep.lock().unwrap().push(l.to_string())), lines)
}

/// A free loopback port (never one of the owner's: `serve` refuses those anyway).
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// A fake node: for each binary frame a client sends, it sends back the next of `replies` (in order), as a node
/// answers each request.
async fn fake_node(replies: Vec<Vec<u8>>) -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (s, _) = l.accept().await.unwrap();
        let ws = tokio_tungstenite::accept_async(s).await.unwrap();
        let (mut tx, mut rx) = ws.split();
        let mut replies = replies.into_iter();
        while let Some(Ok(m)) = rx.next().await {
            if matches!(m, Message::Binary(_)) {
                if let Some(r) = replies.next() {
                    tx.send(Message::Binary(r.into())).await.unwrap();
                }
            }
        }
    });
    port
}

fn ok(r: HostResponse) -> Vec<u8> {
    bincode::serialize(&Ok::<HostResponse, ClientError>(r)).unwrap()
}

/// A contract of `code` under `params`, and its key.
fn contract(code: &[u8], params: &[u8]) -> (ContractContainer, ContractKey) {
    let c = ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(Arc::new(ContractCode::from(code.to_vec())), Parameters::from(params.to_vec()))));
    let key = c.key();
    (c, key)
}

/// The proxy with `hooks` in front of a fake node answering `replies`; the page's WebSocket to it.
async fn page_through(hooks: Arc<dyn Hooks>, replies: Vec<Vec<u8>>) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let node = fake_node(replies).await;
    let listen = free_port();
    tokio::spawn(serve(listen, node, serde_json::json!({}), hooks));
    for _ in 0..100 {
        if let Ok((ws, _)) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{listen}/v1/contract/command?encodingProtocol=native")).await {
            return ws;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("THE SETUP: the proxy never listened");
}

/// What the page hears next: an answer, SILENCE (nothing at all within the wait), or something else -- a malformed or
/// non-binary frame, an error, a closed socket -- which is never taken for either (Codex on #541: "no answer" had
/// folded all of them into silence).
#[derive(Debug)]
enum Heard {
    Answer(Box<HostResponse>),
    Silence,
    Other(String),
}

async fn next_answer(ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin)) -> Heard {
    match tokio::time::timeout(Duration::from_millis(700), ws.next()).await {
        Err(_) => Heard::Silence,
        Ok(Some(Ok(Message::Binary(b)))) => match bincode::deserialize::<Result<HostResponse, ClientError>>(&b) {
            Ok(Ok(r)) => Heard::Answer(Box::new(r)),
            other => Heard::Other(format!("an undecodable or error frame: {other:?}")),
        },
        Ok(other) => Heard::Other(format!("{other:?}")),
    }
}

/// The answer the page heard next, or the test fails naming what it heard instead.
async fn answer(ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin)) -> HostResponse {
    match next_answer(ws).await {
        Heard::Answer(r) => *r,
        Heard::Silence => panic!("the page heard no answer: silence"),
        Heard::Other(what) => panic!("the page heard no answer: {what}"),
    }
}

#[tokio::test]
async fn ws_withhold_drops_only_the_parity_puts_answer() {
    let (parity, pkey) = contract(b"block code", b"parity params");
    let (data, dkey) = contract(b"block code", b"data params");
    let (log, said) = captured();
    let hooks = Arc::new(Withhold::with_log(log));
    let replies = vec![
        ok(HostResponse::ContractResponse(ContractResponse::PutResponse { key: pkey })),
        ok(HostResponse::ContractResponse(ContractResponse::PutResponse { key: dkey })),
    ];
    let mut ws = page_through(hooks.clone(), replies).await;
    for f in wire::frame_put(parity, WrappedState::new([vec![kind::PARITY], b"p".to_vec()].concat()), 1).unwrap() {
        ws.send(Message::Binary(f.into())).await.unwrap();
    }
    let heard = next_answer(&mut ws).await;
    assert!(matches!(heard, Heard::Silence), "the parity PUT's answer was not withheld as SILENCE: {heard:?}");
    for f in wire::frame_put(data, WrappedState::new([vec![kind::RAW], b"d".to_vec()].concat()), 2).unwrap() {
        ws.send(Message::Binary(f.into())).await.unwrap();
    }
    match answer(&mut ws).await {
        HostResponse::ContractResponse(ContractResponse::PutResponse { key }) => assert_eq!(key, dkey),
        other => panic!("the data PUT's answer did not reach the page: {other:?}"),
    }
    assert_eq!(hooks.withheld.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(hooks.parity_puts.load(std::sync::atomic::Ordering::Relaxed), 1);
    // THE EVIDENCE the hook said: one line, for the parity PUT's contract, byte for byte.
    assert_eq!(*said.lock().unwrap(), vec![format!(r#"{{"parity_puts_seen":1,"withheld":"{}","withheld_total":1}}"#, pkey.id())]);
}

#[tokio::test]
async fn ws_lose_answers_the_lost_blocks_get_with_the_nodes_not_found() {
    // The head: a real signed Register state naming a root and its parity (the root's group of one).
    let body = b"the root node".to_vec();
    let root = freenet_prolly::block_id(kind::TREE_NODE, &body);
    let par: Vec<[u8; 32]> = (0..freenet_prolly::parity::PARITY as u8).map(|i| [40 + i; 32]).collect();
    let ledger = signer_proto::head::Ledger { parity: Some(par.concat()), ..Default::default() };
    let value = signer_proto::head::value(&root, &ledger);
    let sk = [7u8; 32];
    let vk = ed25519_dalek::SigningKey::from_bytes(&sk).verifying_key().to_bytes();
    let head_state = contract_keys::register::head_state(&wire::register_params(&vk, wire::HEAD_NAME), &sk, 3, &value).unwrap();
    let (_, head_key) = contract(b"register code", b"register params");
    // The root block's contract: the block code under the root's id, so its answer is recognised as a block.
    let (_, root_key) = contract(b"block code", &root);
    let (log, said) = captured();
    let hooks = Arc::new(LoseHooks { log, ..LoseHooks::new(Lose::new(Target::Root, freenet_prolly::parity::PARITY)) });
    let replies = vec![
        ok(HostResponse::ContractResponse(ContractResponse::GetResponse { key: head_key, contract: None, state: WrappedState::new(head_state) })),
        ok(HostResponse::ContractResponse(ContractResponse::GetResponse { key: root_key, contract: None, state: WrappedState::new([vec![kind::TREE_NODE], body].concat()) })),
    ];
    let mut ws = page_through(hooks, replies).await;
    for id in [*head_key.id(), *root_key.id()] {
        for f in wire::frame_get(id, false, 1).unwrap() {
            ws.send(Message::Binary(f.into())).await.unwrap();
        }
        let a = answer(&mut ws).await;
        if id == *root_key.id() {
            match a {
                HostResponse::ContractResponse(ContractResponse::NotFound { instance_id }) => assert_eq!(instance_id, id),
                other => panic!("the lost root's GET was answered {other:?}, not the node's NotFound"),
            }
        } else {
            assert!(matches!(a, HostResponse::ContractResponse(ContractResponse::GetResponse { .. })), "the head's answer was changed");
        }
    }
    // THE EVIDENCE the hook said, byte for byte: the root's group chosen from the head (the root and the first m - 1
    // parity lost, the last parity the one survivor), then the root's GET answered NotFound.
    let p = freenet_prolly::parity::PARITY;
    let mut lost: Vec<String> = std::iter::once(&root).chain(&par[..p - 1]).map(short).collect();
    lost.sort();
    let q = |v: &[String]| v.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(",");
    assert_eq!(
        *said.lock().unwrap(),
        vec![
            format!(r#"{{"chosen":"root","k":1,"lost":[{}],"lost_data":["{}"],"slots":{},"survivors":["{}"]}}"#, q(&lost), short(&root), 1 + p, short(&par[p - 1])),
            format!(r#"{{"not_found":"{}","not_found_total":1}}"#, short(&root)),
        ]
    );
}

/// A DATA GROUP'S CHOICE, A LOSS AND A SURVIVOR, THROUGH THE BRIDGE (Codex on #541: the fixture above never sends a
/// survivor or a data group, so those emissions were never driven). A real tree: the root's answer makes ws-lose choose
/// the root's group of children; a lost member's GET reaches the page as the node's NotFound; a SURVIVOR's reaches it
/// as its bytes, and is logged `answered`. The lines the hook says, byte for byte, are the three a run logs.
#[tokio::test]
async fn ws_lose_chooses_a_data_group_and_logs_a_loss_and_a_survivor_through_the_bridge() {
    use freenet_prolly::apply::{apply_into, Edit};
    use freenet_prolly::store::{Blocks, MemBlocks};
    let p = freenet_prolly::parity::PARITY;
    let mut b = MemBlocks::default();
    let empty = freenet_prolly::build::init(&mut b);
    let edits: Vec<(Vec<u8>, Edit)> = (0..3000u32).map(|i| (format!("row/{i:06}").into_bytes(), Edit::Put(vec![7u8; 40]))).collect();
    let applied = apply_into(&mut b, &empty, &edits).expect("the tree");
    let parity: std::collections::HashMap<[u8; 32], Vec<u8>> = applied.parity.into_iter().collect();
    let state_of = |id: &[u8; 32]| -> Vec<u8> {
        match b.get(id) {
            Some(n) => [vec![kind::TREE_NODE], n.to_vec()].concat(),
            None => [vec![kind::PARITY], parity.get(id).expect("a parity block").clone()].concat(),
        }
    };
    let root_state = state_of(&applied.root);
    // THE CHOICE ws-lose will make, made here on the same bytes (its rule is lose.rs's tests'): the lost member and a
    // survivor to ask for.
    let mut expect = Lose::new(Target::Data, p);
    let _ = expect.block(applied.root, &root_state);
    let c = expect.chosen().expect("THE SETUP: the root's answer chooses no group").clone();
    let lost = c.slots[0];
    let survivor = *c.slots.iter().find(|s| !c.lost.contains(*s)).expect("THE SETUP: no survivor");
    let get = |id: &[u8; 32]| {
        let (_, key) = contract(b"block code", id);
        ok(HostResponse::ContractResponse(ContractResponse::GetResponse { key, contract: None, state: WrappedState::new(state_of(id)) }))
    };
    let (log, said) = captured();
    let hooks = Arc::new(LoseHooks { log, ..LoseHooks::new(Lose::new(Target::Data, p)) });
    let mut ws = page_through(hooks, vec![get(&applied.root), get(&lost), get(&survivor)]).await;
    for (id, want_lost) in [(applied.root, false), (lost, true), (survivor, false)] {
        let (_, key) = contract(b"block code", &id);
        for f in wire::frame_get(*key.id(), false, 1).unwrap() {
            ws.send(Message::Binary(f.into())).await.unwrap();
        }
        match answer(&mut ws).await {
            HostResponse::ContractResponse(ContractResponse::NotFound { instance_id }) if want_lost => assert_eq!(instance_id, *key.id()),
            HostResponse::ContractResponse(ContractResponse::GetResponse { state, .. }) if !want_lost => assert_eq!(state.as_ref(), state_of(&id).as_slice()),
            other => panic!("block {} (lost: {want_lost}) was answered {other:?}", short(&id)),
        }
    }
    assert_eq!(
        *said.lock().unwrap(),
        vec![
            probe::lose::chosen_line("data", &c).to_string(),
            format!(r#"{{"not_found":"{}","not_found_total":1}}"#, short(&lost)),
            format!(r#"{{"answered":"{}","answered_total":1}}"#, short(&survivor)),
        ]
    );
}

/// THE REAL SINK (Codex on #541: every test above swaps the logger, so the default's stderr write was never run): the
/// default `Log` writes each line to STDERR, one JSON value per line -- run in a child of this test binary, whose
/// stderr is read here.
#[test]
fn the_default_log_writes_each_line_to_stderr() {
    const CHILD: &str = "PROBE_LOG_CHILD";
    let line = serde_json::json!({ "answered": "0102030405060708", "answered_total": 1 });
    if std::env::var(CHILD).is_ok() {
        Log::default().say(&line);
        return;
    }
    let out = std::process::Command::new(std::env::current_exe().expect("this test binary"))
        .args(["--exact", "the_default_log_writes_each_line_to_stderr", "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .output()
        .expect("the child ran");
    assert!(out.status.success(), "the child failed: {}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let want = line.to_string();
    assert!(stderr.lines().any(|l| l == want), "the default Log did not write the line to stderr:\n{stderr}");
    assert!(!String::from_utf8_lossy(&out.stdout).contains(&line.to_string()), "the default Log wrote to stdout");
}

/// A node that answers every WHOLE request it receives with a `PutResponse` for its key (a PUT) and COUNTS them, so a
/// test sees what reached the node, not only what the page was answered.
async fn counting_node(seen: Arc<Mutex<Vec<ContractInstanceId>>>) -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (s, _) = l.accept().await.unwrap();
        let ws = tokio_tungstenite::accept_async(s).await.unwrap();
        let (mut tx, mut rx) = ws.split();
        let mut reqs = probe::frames::Requests::default();
        while let Some(Ok(Message::Binary(b))) = rx.next().await {
            if let Ok(probe::frames::Frame::Whole(freenet_stdlib::client_api::ClientRequest::ContractOp(freenet_stdlib::client_api::ContractRequest::Put { contract, .. }))) = reqs.push("node", &b) {
                seen.lock().unwrap().push(*contract.key().id());
                tx.send(Message::Binary(ok(HostResponse::ContractResponse(ContractResponse::PutResponse { key: contract.key() })).into())).await.unwrap();
            }
        }
    });
    port
}

/// **ws-drop (sdk#479): a dropped block NEVER REACHES THE NODE, and the page is answered for it.** Every 2nd Block PUT
/// is dropped -- a CHUNKED one held until whole and dropped whole -- and so is a later re-send of a dropped block; a
/// kept chunked block reaches the node whole; a non-block PUT is never counted or dropped. The page is answered for
/// every PUT; the node saw only the kept ones; the hook's lines name exactly the dropped blocks, byte for byte.
#[tokio::test]
async fn ws_drop_keeps_every_nth_block_off_the_node_and_answers_the_page_for_it() {
    let code = b"block contract code".to_vec();
    let big = vec![7u8; wire::MAX_CHUNK * 2 + 10];
    let bodies: Vec<Vec<u8>> = vec![b"one".to_vec(), [b"two".to_vec(), big.clone()].concat(), [b"three".to_vec(), big.clone()].concat(), [b"four".to_vec(), big].concat()];
    let blocks: Vec<(freenet_prolly::Cid, ContractContainer, Vec<u8>)> = bodies
        .iter()
        .map(|b| {
            let cid = freenet_prolly::block_id(kind::RAW, b);
            (cid, wire::block::block_contract(&code, &cid), wire::block::block_state(&cid, b).unwrap())
        })
        .collect();
    let (other, other_key) = contract(b"an app container", b"");
    let (log, said) = captured();
    let hooks = Arc::new(probe::drop::DropEvery::with_log(2, log));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let node = counting_node(seen.clone()).await;
    let listen = free_port();
    tokio::spawn(serve(listen, node, serde_json::json!({}), hooks.clone()));
    let mut ws = None;
    for _ in 0..100 {
        if let Ok((w, _)) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{listen}/v1/contract/command?encodingProtocol=native")).await {
            ws = Some(w);
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut ws = ws.expect("THE SETUP: the proxy never listened");
    // The four blocks, the app container, then a RE-SEND of the second block.
    let mut sent: Vec<(ContractContainer, Vec<u8>)> = blocks.iter().map(|(_, c, s)| (c.clone(), s.clone())).collect();
    sent.push((other, b"state".to_vec()));
    sent.push((blocks[1].1.clone(), blocks[1].2.clone()));
    for (n, (c, s)) in sent.iter().enumerate() {
        let frames = wire::frame_put(c.clone(), WrappedState::new(s.clone()), n as u32 + 1).unwrap();
        if n == 1 {
            assert!(frames.len() > 1, "THE SETUP: the dropped block is not chunked");
        }
        for f in frames {
            ws.send(Message::Binary(f.into())).await.unwrap();
        }
        let a = answer(&mut ws).await;
        match a {
            HostResponse::ContractResponse(ContractResponse::PutResponse { key }) => assert_eq!(key.id(), c.key().id(), "PUT {n} answered under another key"),
            other => panic!("PUT {n} answered {other:?}"),
        }
    }
    let kept: Vec<ContractInstanceId> = vec![*blocks[0].1.key().id(), *blocks[2].1.key().id(), *other_key.id()];
    assert_eq!(*seen.lock().unwrap(), kept, "the node saw other than the kept PUTs");
    let ids: Vec<String> = [1usize, 3].iter().map(|i| blocks[*i].1.key().id().to_string()).collect();
    assert_eq!(
        *said.lock().unwrap(),
        vec![
            format!(r#"{{"block":"{}","block_puts_seen":2,"dropped":"{}","dropped_total":1,"kind":{}}}"#, short(&blocks[1].0), ids[0], kind::RAW),
            format!(r#"{{"block":"{}","block_puts_seen":4,"dropped":"{}","dropped_total":2,"kind":{}}}"#, short(&blocks[3].0), ids[1], kind::RAW),
        ],
        "the hook's lines are not exactly the dropped blocks"
    );
}
