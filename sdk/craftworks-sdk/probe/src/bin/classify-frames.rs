//! THE ONE DECODER of what a page SENT its node (the realnet harness's A-side
//! check, builder#— ruling): raw client→node frames in, one classification
//! per request out. No other place decodes these; the harness captures bytes
//! (CDP, every target) and never reads a tag.
//!
//! - A frame is deserialised with freenet-stdlib's OWN request types (the
//!   format's owner): `ClientRequest` → `ContractRequest` / `DelegateRequest`.
//!   A request the SDK chunked (`StreamChunk`) is reassembled with stdlib's
//!   own `ReassemblyBuffer`, per socket, and then classified.
//! - A delegate message's payload goes to `signer_proto::decode_request`
//!   (re-exported by `signer`): SG02 and the signer's request kind.
//! - A PUT is classified by its contract CODE: the Block contract's code hash
//!   (`--block-code <block.wasm>`) or any other -- except an SDK LOAD PIECE
//!   (sdk#347's repair after load), with `--pieces <pieces.json> --webapp-code
//!   <webapp.wasm>`: a PUT at an address the published pieces list names is a
//!   REPAIR only when its state IS that piece's container, proved the way the
//!   list's address was made: `wire::webapp::address(webapp code, the state
//!   sent)` equals it (the address commits to `blake3(state)`, so only the
//!   exact container the manifest's piece was hashed into gives it). Any other
//!   state at a piece's address FAILS, as does any other non-Block PUT.
//!
//! What a VIEW-ONLY step may send, and what FAILS it (the ruling):
//! GET/SUBSCRIBE, and signer QUERIES (Register, Held), pass; a Block-code PUT
//! and a VERIFIED load-piece PUT are REPORTED (repairs; content-addressed, so a
//! view may send one); a PUT of ANY other contract, an UPDATE, a delegate
//! REGISTRATION and a signer Sign / Provision FAIL.
//!
//! In:  JSONL lines `{"t":…, "window":…, "socket":…, "data":"<base64>"}`.
//! Out: JSONL lines `{"t", "window", "socket", "op", "contract"?, "code"?,
//!       "signer"?, "verdict": "pass"|"report"|"fail", "why"?}`; a frame that
//!       does not decode is `op: "undecodable"`, verdict `fail` (never skipped).
//!
//! usage: classify-frames --block-code <block.wasm> [--pieces <pieces.json> --webapp-code <webapp.wasm>] < frames.jsonl > classified.jsonl
use anyhow::{bail, Context, Result};
use base64::Engine as _;
use freenet_stdlib::client_api::{ClientRequest, ContractRequest, DelegateRequest};
use freenet_stdlib::prelude::*;
use std::collections::BTreeMap;
use std::io::{BufRead, Write};

#[derive(serde::Deserialize)]
struct Frame {
    t: serde_json::Value,
    #[serde(default)]
    window: serde_json::Value,
    socket: String,
    data: String,
}

fn hex(b: &[u8]) -> String {
    core_types::hex::encode(b)
}

/// What a PUT is judged against: the Block code, and the SDK's published load
/// pieces (address -> `bundle#index`) with the `webapp` code they are PUT under.
struct Known {
    block: CodeHash,
    pieces: BTreeMap<String, String>,
    webapp: Option<Vec<u8>>,
}

/// One request's classification (without the frame's own fields).
fn classify(req: &ClientRequest<'_>, known: &Known) -> Vec<serde_json::Value> {
    use serde_json::json;
    match req {
        ClientRequest::ContractOp(ContractRequest::Put { contract, state, .. }) => {
            let key = contract.key();
            let at = key.to_string();
            if key.code_hash() == &known.block {
                return vec![json!({ "op": "put", "contract": at, "code": "block", "verdict": "report", "why": "a Block-contract PUT (repair; content-addressed)" })];
            }
            if let (Some(piece), Some(webapp)) = (known.pieces.get(&at), &known.webapp) {
                let exact = wire::webapp::address(webapp, state.as_ref()) == at;
                return vec![json!({
                    "op": "put",
                    "contract": at,
                    "code": "piece",
                    "piece": piece,
                    "verdict": if exact { "report" } else { "fail" },
                    "why": if exact { "a verified SDK load piece (repair after load, sdk#347)" } else { "a PUT at a load piece's address whose state is NOT that piece" },
                })];
            }
            vec![json!({ "op": "put", "contract": at, "code": "other", "verdict": "fail", "why": "a PUT of a non-Block contract" })]
        }
        ClientRequest::ContractOp(ContractRequest::Update { key, .. }) => {
            vec![json!({ "op": "update", "contract": key.to_string(), "verdict": "fail", "why": "an UPDATE" })]
        }
        ClientRequest::ContractOp(ContractRequest::Get { key, .. }) => {
            vec![json!({ "op": "get", "contract": key.to_string(), "verdict": "pass" })]
        }
        ClientRequest::ContractOp(ContractRequest::Subscribe { key, .. }) => {
            vec![json!({ "op": "subscribe", "contract": key.to_string(), "verdict": "pass" })]
        }
        ClientRequest::ContractOp(other) => {
            vec![json!({ "op": "contract-other", "detail": format!("{other:?}").chars().take(80).collect::<String>(), "verdict": "fail", "why": "a contract request this check does not know" })]
        }
        ClientRequest::DelegateOp(DelegateRequest::RegisterDelegate { .. }) => {
            vec![json!({ "op": "register-delegate", "verdict": "fail", "why": "a delegate REGISTRATION" })]
        }
        ClientRequest::DelegateOp(DelegateRequest::ApplicationMessages { inbound, .. }) => inbound
            .iter()
            .map(|m| match m {
                InboundDelegateMsg::ApplicationMessage(am) => match signer::decode_request(&am.payload) {
                    Some((_, r)) => {
                        let (name, fail) = match r {
                            signer::Request::Sign { .. } => ("sign", true),
                            signer::Request::Provision { .. } => ("provision", true),
                            signer::Request::Held { .. } => ("held", false),
                            signer::Request::Register => ("register-query", false),
                        };
                        json!({
                            "op": "signer", "signer": name,
                            "verdict": if fail { "fail" } else { "pass" },
                            "why": if fail { format!("a signer {name}") } else { String::new() },
                        })
                    }
                    None => json!({ "op": "delegate-message", "verdict": "fail", "why": "a delegate message that is not the signer's (SG02)" }),
                },
                other => json!({ "op": "delegate-inbound", "detail": format!("{other:?}").chars().take(60).collect::<String>(), "verdict": "fail", "why": "a delegate message this check does not know" }),
            })
            .collect(),
        ClientRequest::DelegateOp(other) => {
            vec![json!({ "op": "delegate-other", "detail": format!("{other:?}").chars().take(80).collect::<String>(), "verdict": "fail", "why": "a delegate request this check does not know" })]
        }
        ClientRequest::Disconnect { .. } | ClientRequest::Close => vec![json!({ "op": "close", "verdict": "pass" })],
        other => vec![json!({ "op": "client-other", "detail": format!("{other:?}").chars().take(80).collect::<String>(), "verdict": "fail", "why": "a client request this check does not know" })],
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: classify-frames --block-code <block.wasm> [--pieces <pieces.json> --webapp-code <webapp.wasm>] < frames.jsonl";
    let opt = |name: &str| args.iter().position(|a| a == name).map(|i| args.get(i + 1).cloned().with_context(|| format!("{name} needs a path; {usage}"))).transpose();
    let block_path = opt("--block-code")?.context(usage)?;
    let block = *ContractCode::from(std::fs::read(&block_path).with_context(|| format!("reading {block_path}"))?).hash();
    let (pieces, webapp) = match (opt("--pieces")?, opt("--webapp-code")?) {
        (None, None) => (BTreeMap::new(), None),
        (Some(p), Some(w)) => {
            let list: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).with_context(|| format!("reading {p}"))?).with_context(|| format!("{p}: JSON"))?;
            let mut pieces = BTreeMap::new();
            for (bundle, spec) in list.as_object().with_context(|| format!("{p}: not an object of bundles"))? {
                for (i, piece) in spec["pieces"].as_array().with_context(|| format!("{p}: {bundle} has no pieces"))?.iter().enumerate() {
                    let a = piece["address"].as_str().with_context(|| format!("{p}: {bundle} piece {i} has no address"))?;
                    pieces.insert(a.to_string(), format!("{bundle}#{i}"));
                }
            }
            if pieces.is_empty() {
                bail!("{p} names no pieces: nothing to judge a piece PUT against");
            }
            (pieces, Some(std::fs::read(&w).with_context(|| format!("reading {w}"))?))
        }
        _ => bail!("--pieces and --webapp-code go together: a piece is proved by its address under the webapp code; {usage}"),
    };
    let known = Known { block, pieces, webapp };
    let mut reqs = probe::frames::Requests::default();
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    let (mut frames, mut requests) = (0usize, 0usize);
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        frames += 1;
        let f: Frame = serde_json::from_str(&line).with_context(|| format!("frame line {frames}"))?;
        let emit = |out: &mut std::io::StdoutLock, rows: Vec<serde_json::Value>| -> Result<()> {
            for mut r in rows {
                let o = r.as_object_mut().expect("an object");
                o.insert("t".into(), f.t.clone());
                o.insert("window".into(), f.window.clone());
                o.insert("socket".into(), f.socket.clone().into());
                writeln!(out, "{r}")?;
            }
            Ok(())
        };
        let bytes = match base64::engine::general_purpose::STANDARD.decode(&f.data) {
            Ok(b) => b,
            Err(e) => {
                emit(&mut out, vec![serde_json::json!({ "op": "undecodable", "verdict": "fail", "why": format!("not base64: {e}") })])?;
                continue;
            }
        };
        // Through the probes' ONE decoder (probe::frames): a chunked request is reassembled per socket.
        match reqs.push(&f.socket, &bytes) {
            Ok(probe::frames::Frame::Partial) => continue,
            Ok(probe::frames::Frame::Whole(req)) => {
                requests += 1;
                emit(&mut out, classify(&req, &known))?;
            }
            Err(why) => emit(&mut out, vec![serde_json::json!({ "op": "undecodable", "bytes": bytes.len(), "head": hex(&bytes[..bytes.len().min(16)]), "verdict": "fail", "why": why })])?,
        }
    }
    eprintln!("classify-frames: {frames} frame(s), {requests} whole request(s)");
    Ok(())
}
