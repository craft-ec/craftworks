//! WHICH PUTS WERE ANSWERED (the BACKED_UP reading, architect): a page's captured wire (wire-capture's JSONL: the
//! frames it SENT, and the frames its node SENT BACK), paired by contract. For every PUT: its block kind (a Block
//! contract's state starts with its kind byte; `4` is PARITY), when it was first sent, how many times, and when the
//! node answered it -- or that it never did.
//!
//! Requests are read through the probes' one decoder (probe::frames); answers are the client API's own HostResult.
//!
//! usage: put-acks <sent.jsonl> <received.jsonl>     one JSON line per PUT, then a summary line
use anyhow::{bail, Context, Result};
use base64::Engine as _;
use freenet_stdlib::client_api::{ClientError, ClientRequest, ContractRequest, ContractResponse, HostResponse};
use std::collections::BTreeMap;
use std::io::BufRead;

#[derive(serde::Deserialize)]
struct Line {
    t: u64,
    socket: String,
    data: String,
}

#[derive(Default, serde::Serialize)]
struct Put {
    kind: Option<u8>,
    first_sent: u64,
    sends: u32,
    acked: Option<u64>,
}

fn lines(path: &str) -> Result<Vec<Line>> {
    let f = std::fs::File::open(path).with_context(|| format!("reading {path}"))?;
    std::io::BufReader::new(f).lines().filter(|l| l.as_ref().map_or(true, |l| !l.trim().is_empty())).map(|l| Ok(serde_json::from_str(&l?)?)).collect()
}

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let [sent, received] = a.as_slice() else { bail!("usage: put-acks <sent.jsonl> <received.jsonl>") };
    let mut puts: BTreeMap<String, Put> = BTreeMap::new();
    let mut reqs = probe::frames::Requests::default();
    for l in lines(sent)? {
        let bytes = base64::engine::general_purpose::STANDARD.decode(&l.data)?;
        if let Ok(probe::frames::Frame::Whole(ClientRequest::ContractOp(ContractRequest::Put { contract, state, .. }))) = reqs.push(&l.socket, &bytes) {
            let p = puts.entry(contract.key().id().to_string()).or_default();
            if p.sends == 0 {
                p.first_sent = l.t;
                p.kind = state.as_ref().first().copied();
            }
            p.sends += 1;
        }
    }
    for l in lines(received)? {
        let bytes = base64::engine::general_purpose::STANDARD.decode(&l.data)?;
        if let Ok(Ok(HostResponse::ContractResponse(ContractResponse::PutResponse { key }))) = bincode::deserialize::<Result<HostResponse, ClientError>>(&bytes) {
            if let Some(p) = puts.get_mut(&key.id().to_string()) {
                p.acked.get_or_insert(l.t);
            }
        }
    }
    let mut by_kind: BTreeMap<String, (u32, u32, u64)> = BTreeMap::new();
    for (c, p) in &puts {
        println!("{}", serde_json::json!({ "contract": c, "put": p }));
        let k = match p.kind { Some(freenet_prolly::kind::PARITY) => "parity".to_string(), Some(k) => format!("kind {k}"), None => "empty".into() };
        let e = by_kind.entry(k).or_default();
        e.0 += 1;
        if let Some(t) = p.acked {
            e.1 += 1;
            e.2 = e.2.max(t);
        }
    }
    let summary: serde_json::Map<String, serde_json::Value> = by_kind
        .into_iter()
        .map(|(k, (n, acked, last))| (k, serde_json::json!({ "puts": n, "acked": acked, "never_acked": n - acked, "last_ack_t": if acked > 0 { Some(last) } else { None } })))
        .collect();
    println!("{}", serde_json::json!({ "summary": summary }));
    Ok(())
}
