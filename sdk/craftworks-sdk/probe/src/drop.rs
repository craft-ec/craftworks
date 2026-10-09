//! DROP BLOCKS AT PUBLISH (sdk#479, REPAIR's real-network step): between a WRITER's page and its node, every `n`th
//! Block-contract PUT is NOT handed to the node -- and neither is any later re-send of it -- while the page is answered
//! with a `PutResponse` from here, as a node that acked the block and then lost it would. The writer publishes as
//! usual; the dropped blocks were never stored anywhere, so a read of the tree finds them MISSING on the network --
//! what REPAIR must put back. (ws-withhold drops only an ANSWER and ws-lose only lies to a reader: neither makes a
//! block absent.) A request split into chunks is held until it is whole, then handed on whole or dropped whole.
//! Every dropped block is a JSON line on stderr ([`dropped_line`]): the run's list of what the network lacks.
use crate::frames::{Frame, Requests};
use freenet_stdlib::client_api::{ClientError, ClientRequest, ContractRequest, ContractResponse, HostResponse};
use freenet_stdlib::prelude::{ContractInstanceId, ContractKey};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;

pub struct DropEvery {
    every: usize,
    state: Mutex<State>,
    pub log: crate::proxy::Log,
}

#[derive(Default)]
struct State {
    /// Per page connection: its requests' reassembly, and each chunked stream's frames held until it is whole.
    conns: HashMap<u64, (Requests, HashMap<u32, Vec<Message>>)>,
    /// Block PUTs seen (first sends), and the blocks dropped: a re-send of one is dropped too.
    seen: usize,
    dropped: HashSet<ContractInstanceId>,
}

/// THE LOG LINE for one dropped block. LOAD-BEARING: the run's evidence reads it.
pub fn dropped_line(id: &ContractInstanceId, block: &[u8; 32], kind: u8, total: usize, seen: usize) -> serde_json::Value {
    serde_json::json!({ "dropped": id.to_string(), "block": crate::lose::short(block), "kind": kind, "dropped_total": total, "block_puts_seen": seen })
}

impl DropEvery {
    /// Drop every `every`th Block PUT (`every` >= 1).
    pub fn new(every: usize) -> DropEvery {
        DropEvery { every: every.max(1), state: Mutex::default(), log: crate::proxy::Log::default() }
    }

    pub fn with_log(every: usize, log: crate::proxy::Log) -> DropEvery {
        DropEvery { log, ..DropEvery::new(every) }
    }

    /// The contracts dropped so far.
    pub fn dropped(&self) -> Vec<ContractInstanceId> {
        self.state.lock().unwrap().dropped.iter().copied().collect()
    }

    /// A whole request: the key to answer it under when it is DROPPED, `None` to hand it on.
    fn decide(&self, st: &mut State, req: &ClientRequest<'_>) -> Option<ContractKey> {
        let ClientRequest::ContractOp(ContractRequest::Put { contract, state, .. }) = req else { return None };
        let key = contract.key();
        // A BLOCK is a state whose (kind, body) hashes to the id its key names under the key's own contract code.
        let (cid, _) = wire::block::block_of_state(state.as_ref()).filter(|(cid, _)| {
            let code: Option<[u8; 32]> = key.code_hash().as_ref().try_into().ok();
            code.is_some_and(|code| contract_keys::instance(&code, cid).as_slice() == key.id().as_bytes())
        })?;
        if st.dropped.contains(key.id()) {
            return Some(key); // a re-send of a dropped block: dropped too
        }
        st.seen += 1;
        if !st.seen.is_multiple_of(self.every) {
            return None;
        }
        st.dropped.insert(*key.id());
        self.log.say(&dropped_line(key.id(), &cid, state.as_ref()[0], st.dropped.len(), st.seen));
        Some(key)
    }
}

impl crate::proxy::Hooks for DropEvery {
    fn pass_up(&self, conn: u64, m: Message) -> (Vec<Message>, Option<Message>) {
        let Message::Binary(b) = &m else { return (vec![m], None) };
        let chunk = match bincode::deserialize::<ClientRequest<'_>>(b) {
            Ok(ClientRequest::StreamChunk { stream_id, .. }) => Some(stream_id),
            Ok(_) => None,
            Err(_) => return (vec![m], None),
        };
        let mut st = self.state.lock().unwrap();
        let st = &mut *st;
        let (reqs, held) = st.conns.entry(conn).or_default();
        let whole = match reqs.push(&format!("conn-{conn}"), b) {
            Ok(Frame::Whole(r)) => r,
            Ok(Frame::Partial) => {
                held.entry(chunk.unwrap_or(u32::MAX)).or_default().push(m);
                return (Vec::new(), None);
            }
            Err(_) => return (vec![m], None),
        };
        let mut frames = chunk.and_then(|id| held.remove(&id)).unwrap_or_default();
        frames.push(m);
        match self.decide(st, &whole) {
            None => (frames, None),
            Some(key) => {
                let ack: Result<HostResponse, ClientError> = Ok(HostResponse::ContractResponse(ContractResponse::PutResponse { key }));
                (Vec::new(), bincode::serialize(&ack).ok().map(|b| Message::Binary(b.into())))
            }
        }
    }

    fn down(&self, _conn: u64, m: Message) -> Option<Message> {
        Some(m)
    }
}
