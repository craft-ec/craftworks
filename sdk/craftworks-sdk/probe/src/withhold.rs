//! WITHHOLD PARITY's decision (`ws-withhold`, the live harness's BACKED_UP control), kept apart from its binary so it
//! is driven through the one proxy in a test: per page connection, the contracts of the PARITY-block PUTs it sent,
//! and the node's answers to exactly those dropped -- the page re-sends such a PUT until answered (rule 7) and here it
//! never is. Every dropped answer is a JSON line on stderr ([`withheld_line`], its shape pinned by a test).
use freenet_stdlib::client_api::{ClientError, ClientRequest, ContractRequest, ContractResponse, HostResponse};
use freenet_stdlib::prelude::ContractInstanceId;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;

#[derive(Default)]
pub struct Withhold {
    /// Per page connection: its requests' reassembly, and the contracts of the parity PUTs it sent.
    conns: Mutex<HashMap<u64, (crate::frames::Requests, HashSet<ContractInstanceId>)>>,
    /// Answers dropped so far.
    pub withheld: AtomicU64,
    /// Parity PUTs seen so far.
    pub parity_puts: AtomicU64,
    /// Where each dropped answer's line goes (stderr in the binary).
    pub log: crate::proxy::Log,
}

/// THE LOG LINE for one dropped answer. LOAD-BEARING: harness assertions read it.
pub fn withheld_line(id: &ContractInstanceId, total: u64, parity_puts_seen: u64) -> serde_json::Value {
    serde_json::json!({ "withheld": id.to_string(), "withheld_total": total, "parity_puts_seen": parity_puts_seen })
}

impl Withhold {
    /// Its lines to `log` instead of stderr (a test's capture).
    pub fn with_log(log: crate::proxy::Log) -> Withhold {
        Withhold { log, ..Default::default() }
    }
}

impl crate::proxy::Hooks for Withhold {
    fn up(&self, conn: u64, m: &Message) {
        let Message::Binary(b) = m else { return };
        let mut conns = self.conns.lock().unwrap();
        let (reqs, parity) = conns.entry(conn).or_default();
        if let Ok(crate::frames::Frame::Whole(ClientRequest::ContractOp(ContractRequest::Put { contract, state, .. }))) = reqs.push(&format!("conn-{conn}"), b) {
            if state.as_ref().first() == Some(&freenet_prolly::kind::PARITY) {
                parity.insert(*contract.key().id());
                self.parity_puts.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn down(&self, conn: u64, m: Message) -> Option<Message> {
        if let Message::Binary(b) = &m {
            if let Ok(Ok(HostResponse::ContractResponse(ContractResponse::PutResponse { key }))) = bincode::deserialize::<Result<HostResponse, ClientError>>(b) {
                if self.conns.lock().unwrap().get(&conn).is_some_and(|(_, p)| p.contains(key.id())) {
                    let n = self.withheld.fetch_add(1, Ordering::Relaxed) + 1;
                    self.log.say(&withheld_line(key.id(), n, self.parity_puts.load(Ordering::Relaxed)));
                    return None;
                }
            }
        }
        Some(m)
    }
}
