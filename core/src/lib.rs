//! The Craftworks app's one wasm package: the node's message framing (`wire`), the identity delegate's questions and
//! answers, and the account records a first login puts. It holds no key after a call returns and makes no decision: it
//! frames what the page asks and names what the node answers, as JSON the components can show.

pub mod account;

use craftworks_identity::{decode_answer, encode_request, Answer, Request};
use serde_json::{json, Value};
use wire::{AckKind, Incoming, Reassembler};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// An identity answer, for the page.
pub fn answer_json(a: &Answer) -> Value {
    match a {
        Answer::Unlocked { public, did } => json!({ "unlocked": { "member": hex(public), "did": hex(did) } }),
        Answer::WrongPin { tries_left } => json!({ "wrongPin": { "triesLeft": tries_left } }),
        Answer::Locked => json!({ "locked": true }),
        Answer::LoggedOut => json!({ "loggedOut": true }),
        Answer::Signed { sig } => json!({ "signed": hex(sig) }),
        Answer::Exported { seed } => json!({ "exported": hex(seed) }),
        Answer::Refused(why) => json!({ "refused": format!("{why:?}") }),
    }
}

/// What one frame from the node says, for the page. `Partial` frames say nothing yet.
pub fn describe(incoming: Incoming) -> Value {
    match incoming {
        Incoming::Partial => json!({ "kind": "partial" }),
        Incoming::Ack(AckKind::Registered(k)) => json!({ "kind": "registered", "delegate": k }),
        Incoming::Ack(AckKind::Put(k)) => json!({ "kind": "put", "key": k.to_string() }),
        Incoming::Ack(a) => json!({ "kind": "ack", "what": format!("{a:?}") }),
        Incoming::DelegateMissing { key } => json!({ "kind": "delegate-missing", "delegate": key }),
        Incoming::DelegateThrottled { said } => json!({ "kind": "throttled", "said": said }),
        Incoming::EngineBytes(msgs) => {
            let answers: Vec<Value> = msgs
                .iter()
                .map(|m| match decode_answer(m) {
                    Some((id, a)) => json!({ "id": id, "answer": answer_json(&a) }),
                    None => json!({ "answer": "unreadable", "bytes": m.len() }),
                })
                .collect();
            json!({ "kind": "identity", "answers": answers })
        }
        Incoming::Refused(r) => json!({ "kind": "refused", "said": format!("{r:?}") }),
        Incoming::Unusable(u) => json!({ "kind": "unusable", "why": format!("{u:?}") }),
        other => json!({ "kind": "other", "what": format!("{other:?}").chars().take(200).collect::<String>() }),
    }
}

/// One page's connection state: the reassembler for chunked answers and the identity delegate it asks.
pub struct Core {
    r: Reassembler,
    identity: Vec<u8>,
    next_id: u32,
    next_stream: u32,
}

impl Core {
    pub fn new(identity_wasm: &[u8]) -> Core {
        Core { r: Reassembler::new(), identity: identity_wasm.to_vec(), next_id: 1, next_stream: 1 }
    }

    pub fn identity_key(&self) -> String {
        wire::delegate_from_code(&self.identity).1.to_string()
    }

    fn stream(&mut self) -> u32 {
        self.next_stream += 1;
        self.next_stream
    }

    pub fn frames_register_identity(&mut self) -> Result<Vec<Vec<u8>>, String> {
        let (d, _) = wire::delegate_from_code(&self.identity);
        let s = self.stream();
        wire::frame_register_delegate(d, s)
    }

    /// Ask the identity delegate. Returns the id its answer will carry, and the frames.
    pub fn frames_identity(&mut self, req: &Request) -> Result<(u32, Vec<Vec<u8>>), String> {
        let (_, key) = wire::delegate_from_code(&self.identity);
        let id = self.next_id;
        self.next_id += 1;
        let s = self.stream();
        let params = freenet_stdlib::prelude::Parameters::from(Vec::new());
        Ok((id, wire::frame_delegate_op(&key, &params, encode_request(id, req), s)?))
    }

    /// Frames of a PUT (`wire::frame_put`).
    pub fn frames_put(&mut self, p: &account::Put) -> Result<Vec<Vec<u8>>, String> {
        let s = self.stream();
        wire::frame_put(p.contract.clone(), p.state.clone(), s)
    }

    pub fn take(&mut self, bytes: &[u8]) -> Value {
        describe(wire::unframe(&mut self.r, bytes))
    }
}

/// 32 bytes from hex, or an error naming what.
pub fn bytes32(what: &str, h: &str) -> Result<[u8; 32], String> {
    unhex(h).and_then(|b| b.try_into().ok()).ok_or_else(|| format!("{what}: not 32 bytes of hex"))
}

pub fn unhex(h: &str) -> Option<Vec<u8>> {
    if h.len() % 2 != 0 {
        return None;
    }
    (0..h.len()).step_by(2).map(|i| u8::from_str_radix(h.get(i..i + 2)?, 16).ok()).collect()
}

#[cfg(target_arch = "wasm32")]
mod js {
    use super::*;
    use wasm_bindgen::prelude::*;

    fn frames(v: Vec<Vec<u8>>) -> js_sys::Array {
        v.into_iter().map(|f| JsValue::from(js_sys::Uint8Array::from(&f[..]))).collect()
    }

    fn err(e: String) -> JsValue {
        JsValue::from_str(&e)
    }

    fn b32(v: &[u8]) -> Result<[u8; 32], JsValue> {
        v.try_into().map_err(|_| err("expected 32 bytes".into()))
    }

    #[wasm_bindgen]
    pub struct CraftworksCore(Core);

    #[wasm_bindgen]
    impl CraftworksCore {
        #[wasm_bindgen(constructor)]
        pub fn new(identity_wasm: &[u8]) -> CraftworksCore {
            CraftworksCore(Core::new(identity_wasm))
        }
        pub fn identity_key(&self) -> String {
            self.0.identity_key()
        }
        pub fn frames_register_identity(&mut self) -> Result<js_sys::Array, JsValue> {
            self.0.frames_register_identity().map(frames).map_err(err)
        }
        fn ask(&mut self, r: Request) -> Result<js_sys::Array, JsValue> {
            let (id, f) = self.0.frames_identity(&r).map_err(err)?;
            Ok([JsValue::from(id), JsValue::from(frames(f))].into_iter().collect())
        }
        /// `[id, frames]` for each identity request.
        pub fn frames_provision(&mut self, seed: &[u8], did: &[u8], pin: String, session: &[u8]) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Provision { seed: b32(seed)?, did: b32(did)?, pin, session: b32(session)? })
        }
        pub fn frames_unlock(&mut self, pin: String, session: &[u8]) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Unlock { pin, session: b32(session)? })
        }
        pub fn frames_lock(&mut self) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Lock)
        }
        pub fn frames_who(&mut self, session: &[u8]) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Who { session: b32(session)? })
        }
        pub fn frames_sign(&mut self, session: &[u8], params: &[u8], seq: u64, value_hash: &[u8]) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Sign { session: b32(session)?, params: params.to_vec(), seq, value_hash: b32(value_hash)? })
        }
        pub fn frames_export(&mut self, session: &[u8]) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Export { session: b32(session)? })
        }

        /// A new account (owner seat + vault) for the first member `member_public`: `{ did, didBytes, seat, vault }`,
        /// each put as `{ id, frames }`. The owner key exists only inside this call.
        #[allow(clippy::too_many_arguments)]
        pub fn new_account(
            &mut self,
            register_code: &[u8],
            owner_seed: &[u8],
            member_public: &[u8],
            passphrase: String,
            salt: &[u8],
            nonce: &[u8],
        ) -> Result<js_sys::Object, JsValue> {
            let salt: [u8; 16] = salt.try_into().map_err(|_| err("salt: 16 bytes".into()))?;
            let nonce: [u8; 12] = nonce.try_into().map_err(|_| err("nonce: 12 bytes".into()))?;
            let a = account::create(register_code, &b32(owner_seed)?, &b32(member_public)?, &passphrase, &salt, &nonce);
            let o = js_sys::Object::new();
            let set = |k: &str, v: JsValue| js_sys::Reflect::set(&o, &k.into(), &v).map(|_| ());
            set("did", a.did().into())?;
            set("didBytes", js_sys::Uint8Array::from(&a.seat.id_bytes[..]).into())?;
            for (name, put) in [("seat", &a.seat), ("vault", &a.vault)] {
                let p = js_sys::Object::new();
                js_sys::Reflect::set(&p, &"id".into(), &put.id.clone().into())?;
                js_sys::Reflect::set(&p, &"frames".into(), &frames(self.0.frames_put(put).map_err(err)?).into())?;
                set(name, p.into())?;
            }
            Ok(o)
        }

        /// The ed25519 public key of a seed.
        pub fn public_of(seed: &[u8]) -> Result<js_sys::Uint8Array, JsValue> {
            let k = ed25519_dalek::SigningKey::from_bytes(&b32(seed)?);
            Ok(js_sys::Uint8Array::from(&k.verifying_key().to_bytes()[..]))
        }

        /// What a frame from the node says, as JSON text.
        pub fn take(&mut self, bytes: &[u8]) -> String {
            self.0.take(bytes).to_string()
        }
    }

    /// `did:craftec:<id>` from the owner seat's 32-byte contract id (what the identity delegate answers).
    #[wasm_bindgen]
    pub fn did_of(id: &[u8]) -> Result<String, JsValue> {
        Ok(format!("did:craftec:{}", wire::contract_id(b32(id)?).encode()))
    }

    /// The node's client-API URL on this machine (`wire::ws_url`: loopback only, native encoding), at the host the page
    /// was served from (`localhost` and `127.0.0.1` are different origins to the node).
    #[wasm_bindgen]
    pub fn ws_url(host: &str, port: u16) -> Result<String, JsValue> {
        wire::ws_url(host, port).map_err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_requests_frame_and_ids_advance() {
        let mut c = Core::new(b"\0asm\x01\0\0\0");
        let (a, fa) = c.frames_identity(&Request::Lock).unwrap();
        let (b, _) = c.frames_identity(&Request::Who { session: [1; 32] }).unwrap();
        assert_eq!((a, b), (1, 2));
        assert!(!fa.is_empty());
        assert!(!c.frames_register_identity().unwrap().is_empty());
    }

    #[test]
    fn answers_read_as_json() {
        let a = answer_json(&Answer::Unlocked { public: [1; 32], did: [2; 32] });
        assert_eq!(a["unlocked"]["did"], hex(&[2; 32]));
        assert_eq!(answer_json(&Answer::WrongPin { tries_left: 3 })["wrongPin"]["triesLeft"], 3);
        assert_eq!(bytes32("x", &hex(&[7; 32])).unwrap(), [7; 32]);
        assert!(bytes32("x", "zz").is_err());
    }
}
