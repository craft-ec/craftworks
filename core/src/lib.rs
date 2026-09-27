//! The Craftworks app's one wasm package: the node's message framing (`wire`) and the signer's "whose node is this?"
//! question, and nothing else. It holds no key and makes no decision: it frames what the page asks and names what the
//! node answers, as JSON the components can show.

use craftec_register_contract::wire::{Authority, Params};
use serde_json::{json, Value};
use wire::signer::{frame_register_query, read_answer, SignerAnswer};
use wire::{AckKind, Incoming, Reassembler};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Who a signer's Register params name: the writer's public key and the record's label.
pub fn who_of(params: &[u8]) -> Value {
    match Params::parse(params) {
        Some(p) => {
            let key = match &p.authority {
                Authority::One(k) => json!({ "keys": [hex(&k.to_bytes())], "needs": 1 }),
                Authority::Quorum { k, keys } => {
                    json!({ "keys": keys.iter().map(|x| hex(&x.to_bytes())).collect::<Vec<_>>(), "needs": k })
                }
            };
            json!({ "identity": key, "label": String::from_utf8_lossy(&p.label), "params": hex(params) })
        }
        None => json!({ "unreadable": hex(params) }),
    }
}

/// What one frame from the node says, for the page. `Partial` frames say nothing yet.
pub fn describe(incoming: Incoming) -> Value {
    match incoming {
        Incoming::Partial => json!({ "kind": "partial" }),
        Incoming::Ack(AckKind::Registered(k)) => json!({ "kind": "registered", "delegate": k }),
        Incoming::Ack(a) => json!({ "kind": "ack", "what": format!("{a:?}") }),
        Incoming::DelegateMissing { key } => json!({ "kind": "delegate-missing", "delegate": key }),
        Incoming::DelegateThrottled { said } => json!({ "kind": "throttled", "said": said }),
        Incoming::EngineBytes(msgs) => {
            let answers: Vec<Value> = msgs
                .iter()
                .map(|m| match read_answer(m) {
                    Some((id, SignerAnswer::Register { params: Some(p) })) => {
                        json!({ "id": id, "answer": "register", "who": who_of(&p) })
                    }
                    Some((id, SignerAnswer::Register { params: None })) => {
                        json!({ "id": id, "answer": "register", "who": null })
                    }
                    Some((id, other)) => json!({ "id": id, "answer": format!("{other:?}") }),
                    None => json!({ "answer": "unreadable", "bytes": m.len() }),
                })
                .collect();
            json!({ "kind": "signer", "answers": answers })
        }
        Incoming::Refused(r) => json!({ "kind": "refused", "said": format!("{r:?}") }),
        Incoming::Unusable(u) => json!({ "kind": "unusable", "why": format!("{u:?}") }),
        other => json!({ "kind": "other", "what": format!("{other:?}").chars().take(200).collect::<String>() }),
    }
}

/// One page's connection state: the reassembler for chunked answers and the signer it asks.
pub struct Core {
    r: Reassembler,
    signer: Vec<u8>,
    next_id: u32,
    next_stream: u32,
}

impl Core {
    pub fn new(signer_wasm: &[u8]) -> Core {
        Core { r: Reassembler::new(), signer: signer_wasm.to_vec(), next_id: 1, next_stream: 1 }
    }

    pub fn signer_key(&self) -> String {
        wire::delegate_from_code(&self.signer).1.to_string()
    }

    fn stream(&mut self) -> u32 {
        self.next_stream += 1;
        self.next_stream
    }

    pub fn frames_register_signer(&mut self) -> Result<Vec<Vec<u8>>, String> {
        let (d, _) = wire::delegate_from_code(&self.signer);
        let s = self.stream();
        wire::frame_register_delegate(d, s)
    }

    /// Ask the signer whose Register it signs for. Returns the request id the answer will carry, and the frames.
    pub fn frames_who(&mut self) -> Result<(u32, Vec<Vec<u8>>), String> {
        let (_, key) = wire::delegate_from_code(&self.signer);
        let id = self.next_id;
        self.next_id += 1;
        let s = self.stream();
        Ok((id, frame_register_query(&key, id, s)?))
    }

    pub fn take(&mut self, bytes: &[u8]) -> Value {
        describe(wire::unframe(&mut self.r, bytes))
    }
}

#[cfg(target_arch = "wasm32")]
mod js {
    use super::*;
    use wasm_bindgen::prelude::*;

    fn frames(v: Vec<Vec<u8>>) -> js_sys::Array {
        v.into_iter().map(|f| JsValue::from(js_sys::Uint8Array::from(&f[..]))).collect()
    }

    #[wasm_bindgen]
    pub struct CraftworksCore(Core);

    #[wasm_bindgen]
    impl CraftworksCore {
        #[wasm_bindgen(constructor)]
        pub fn new(signer_wasm: &[u8]) -> CraftworksCore {
            CraftworksCore(Core::new(signer_wasm))
        }
        pub fn signer_key(&self) -> String {
            self.0.signer_key()
        }
        pub fn frames_register_signer(&mut self) -> Result<js_sys::Array, JsValue> {
            self.0.frames_register_signer().map(frames).map_err(|e| JsValue::from_str(&e))
        }
        /// `[id, frames]`.
        pub fn frames_who(&mut self) -> Result<js_sys::Array, JsValue> {
            let (id, f) = self.0.frames_who().map_err(|e| JsValue::from_str(&e))?;
            Ok([JsValue::from(id), JsValue::from(frames(f))].into_iter().collect())
        }
        /// What a frame from the node says, as JSON text.
        pub fn take(&mut self, bytes: &[u8]) -> String {
            self.0.take(bytes).to_string()
        }
    }

    /// The node's client-API URL on this machine (`wire::ws_url`: loopback only, native encoding).
    #[wasm_bindgen]
    pub fn ws_url(port: u16) -> Result<String, JsValue> {
        wire::ws_url("127.0.0.1", port).map_err(|e| JsValue::from_str(&e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn who_names_the_key_and_label_of_a_register() {
        let w = craftec_register_contract::testing::keyset(1, 1, true);
        let who = who_of(&w.params_bytes);
        assert_eq!(who["identity"]["keys"][0], hex(&w.signers[0].verifying_key().to_bytes()));
        assert_eq!(who["identity"]["needs"], 1);
        assert_eq!(who["label"], "head");
        // Bytes that are not params are said to be unreadable, never turned into an identity.
        assert!(who_of(b"nonsense").get("unreadable").is_some());
    }

    #[test]
    fn frames_for_the_signer_are_produced_and_ids_advance() {
        let mut c = Core::new(b"\0asm\x01\0\0\0");
        let (a, fa) = c.frames_who().unwrap();
        let (b, _) = c.frames_who().unwrap();
        assert_eq!((a, b), (1, 2));
        assert!(!fa.is_empty());
        assert!(!c.frames_register_signer().unwrap().is_empty());
    }
}
