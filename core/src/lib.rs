//! The Craftworks app's one wasm package: the node's message framing (`wire`), the identity delegate's questions and
//! answers, and the account: its recovery words and the DID they name. It holds no key after a call returns and makes no decision: it
//! frames what the page asks and names what the node answers, as JSON the components can show.

pub mod account;
pub mod data;

use craftworks_identity::{decode_answer, encode_request, Answer, Request};
use serde_json::{json, Value};
use wire::{AckKind, Incoming, Reassembler};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// An identity answer, for the page.
pub fn answer_json(a: &Answer) -> Value {
    match a {
        Answer::Unlocked { public, did, data } => {
            json!({ "unlocked": { "member": hex(public), "did": hex(did), "data": data.map(|d| hex(&d)) } })
        }
        Answer::WrongPin { tries_left } => json!({ "wrongPin": { "triesLeft": tries_left } }),
        Answer::Locked => json!({ "locked": true }),
        Answer::LoggedOut => json!({ "loggedOut": true }),
        Answer::Signed { sig } => json!({ "signed": hex(sig) }),
        Answer::Exported { seed } => json!({ "exported": hex(seed) }),
        Answer::Handed { seed, did, data } => {
            json!({ "handed": { "seed": hex(seed), "did": hex(did), "data": data.map(|d| hex(&d)) } })
        }
        Answer::Granted { tables } => json!({ "granted": tables }),
        Answer::Grants { list } => {
            json!({ "grants": list.iter().map(|(app, t)| json!({ "app": wire::contract_id(*app).encode(), "table": t })).collect::<Vec<_>>() })
        }
        Answer::Revoked => json!({ "revoked": true }),
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
    /// Contract states the node sent (a GET's answer), by contract id: read by the account readers, never handed to
    /// the page as bytes.
    got: std::collections::HashMap<[u8; 32], Vec<u8>>,
    /// Open tails (this member's, to write; the account's other members', to read), by contract id.
    tails: std::collections::HashMap<[u8; 32], data::Open>,
    /// The Block contract's code: a table's tree blocks live in Block contracts.
    block_code: Vec<u8>,
    /// Tree blocks asked of the network: Block contract id -> (the tail that needs it, the block's id).
    wanted: std::collections::HashMap<[u8; 32], ([u8; 32], freenet_prolly::Cid)>,
}

impl Core {
    pub fn new(identity_wasm: &[u8]) -> Core {
        Core { r: Reassembler::new(), identity: identity_wasm.to_vec(), next_id: 1, next_stream: 1, got: Default::default(), tails: Default::default(), block_code: Vec::new(), wanted: Default::default() }
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
        self.frames_identity_at(key, req)
    }

    /// Ask an EARLIER build of the identity delegate, named `<key>:<code hash>` (base58, as the manifest's
    /// `identity_prior` lists them): only for `Handover`, moving a member to this build.
    pub fn frames_handover(&mut self, prior: &str, pin: String) -> Result<(u32, Vec<Vec<u8>>), String> {
        let b32 = |s: &str| -> Result<[u8; 32], String> {
            bs58::decode(s).into_vec().ok().and_then(|v| v.try_into().ok()).ok_or_else(|| format!("not a 32-byte base58 id: {s}"))
        };
        let (k, c) = prior.split_once(':').ok_or("an earlier build is <key>:<code hash>")?;
        let key = freenet_stdlib::prelude::DelegateKey::new(b32(k)?, freenet_stdlib::prelude::CodeHash::new(b32(c)?));
        self.frames_identity_at(key, &Request::Handover { pin })
    }

    fn frames_identity_at(&mut self, key: freenet_stdlib::prelude::DelegateKey, req: &Request) -> Result<(u32, Vec<Vec<u8>>), String> {
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

    /// Frames of a GET of contract `id` (`wire::frame_get`, no subscription).
    pub fn frames_get(&mut self, id: [u8; 32]) -> Result<Vec<Vec<u8>>, String> {
        let s = self.stream();
        wire::frame_get(wire::contract_id(id), false, s)
    }

    /// The state the node last sent for `id`.
    pub fn got(&self, id: &[u8; 32]) -> Option<&[u8]> {
        self.got.get(id).map(Vec::as_slice)
    }

    /// Open the account's table `table` under the data key `key` (idempotent). Returns its contract id.
    pub fn tail_open(&mut self, tail_code: &[u8], key: &[u8; 32], table: &str) -> [u8; 32] {
        let o = data::Open::new(tail_code, key, table);
        let id = o.id_bytes();
        self.tails.entry(id).or_insert(o);
        id
    }

    pub fn tail(&mut self, id: &[u8; 32]) -> Result<&mut data::Open, String> {
        self.tails.get_mut(id).ok_or_else(|| "that tail is not open".into())
    }

    /// Frames that send a committed write: the first as a PUT of the whole state, then UPDATEs of one delta each.
    pub fn frames_send(&mut self, id: &[u8; 32], send: data::Send) -> Result<Vec<Vec<u8>>, String> {
        let s = self.stream();
        let o = self.tail(id)?;
        match send {
            data::Send::Put(state) => {
                wire::frame_put(o.contract.clone(), freenet_stdlib::prelude::WrappedState::new(state), s)
            }
            data::Send::Update(delta) => wire::frame_update_delta(o.key(), delta, s),
        }
    }

    /// Frames of a GET of an open tail, subscribing (so other devices' writes arrive as they land).
    pub fn frames_tail_get(&mut self, id: &[u8; 32]) -> Result<Vec<Vec<u8>>, String> {
        let s = self.stream();
        wire::frame_get(wire::contract_id(*id), true, s)
    }

    fn tail_state(&mut self, id: [u8; 32], state: &[u8]) -> Option<Value> {
        self.tails.get_mut(&id)?.absorb(state);
        Some(self.tail_view(&id))
    }

    pub fn set_block_code(&mut self, code: &[u8]) {
        self.block_code = code.to_vec();
    }

    /// An open table as the page sees it: `{ kind: "tail", tail: { rows, … } }` once every tree block it needs is
    /// held; else `{ kind: "tail-need", blocks: [Block contract ids] }`, each to GET (its answer comes back through
    /// `take`, which feeds it in and gives the next view).
    pub fn tail_view(&mut self, id: &[u8; 32]) -> Value {
        let Some(o) = self.tails.get(id) else { return json!({ "kind": "error", "said": "that tail is not open" }) };
        match o.rows() {
            Ok(data::Step::Ready(rows)) => json!({ "kind": "tail", "id": hex(id), "tail": rows }),
            Ok(data::Step::Need(cids)) => {
                let blocks = self.want(id, &cids);
                json!({ "kind": "tail-need", "id": hex(id), "blocks": blocks })
            }
            Err(e) => json!({ "kind": "tail-unreadable", "id": hex(id), "said": e }),
        }
    }

    /// Record tree blocks `cids` as wanted by tail `id`; their Block contract ids (hex), to GET.
    fn want(&mut self, id: &[u8; 32], cids: &[freenet_prolly::Cid]) -> Vec<String> {
        cids.iter()
            .map(|cid| {
                let c = wire::block::contract_for(&self.block_code, cid);
                self.wanted.insert(c, (*id, *cid));
                hex(&c)
            })
            .collect()
    }

    /// FLUSH an open table: `Ready` with the frames that PUT its new tree blocks (send and see them all accepted
    /// FIRST) and the step to sign; or `Need` with Block contract ids to GET first.
    pub fn tail_flush(&mut self, id: &[u8; 32]) -> Result<FlushOut, String> {
        let step = self.tail(id)?.flush()?;
        match step {
            data::Step::Need(cids) => Ok(FlushOut::Need(self.want(id, &cids))),
            data::Step::Ready(f) => {
                let mut puts = Vec::new();
                for (cid, state) in f.blocks {
                    let s = self.stream();
                    let c = wire::block::block_contract(&self.block_code, &cid);
                    let name = c.key().id().encode();
                    puts.push((name, wire::frame_put(c, freenet_stdlib::prelude::WrappedState::new(state), s)?));
                }
                Ok(FlushOut::Ready { seq: f.seq, hash: f.hash, puts })
            }
        }
    }

    pub fn take(&mut self, bytes: &[u8]) -> Value {
        match wire::unframe(&mut self.r, bytes) {
            Incoming::Got { id, state } => {
                if let Some(v) = self.tail_state(id, &state) {
                    return v;
                }
                // A tree block a table asked for: into that table, and its next view.
                if let Some((tail, cid)) = self.wanted.remove(&id) {
                    if let Some(o) = self.tails.get_mut(&tail) {
                        if !o.absorb_block(&cid, &state) {
                            return json!({ "kind": "tail-unreadable", "id": hex(&tail), "said": format!("block {} is not what was asked", hex(&cid)) });
                        }
                    }
                    let mut v = self.tail_view(&tail);
                    v["block"] = json!(hex(&id)); // which fetch this answers
                    return v;
                }
                self.got.insert(id, state);
                json!({ "kind": "got", "id": hex(&id) })
            }
            Incoming::HeadChanged { key, state: Some(state) } => {
                match freenet_stdlib::prelude::ContractInstanceId::from_base58(&key).ok().and_then(|i| self.tail_state(*i, &state)) {
                    Some(v) => v,
                    None => json!({ "kind": "changed", "key": key }),
                }
            }
            Incoming::Ack(AckKind::Updated(key)) => json!({ "kind": "updated", "key": key }),
            Incoming::GetFailed { id, why } => json!({ "kind": "get-failed", "id": hex(&id), "why": format!("{why:?}") }),
            other => describe(other),
        }
    }
}

/// A flush, from the core: the Block contract ids (hex) to GET first, or the block PUTs (by name, with their frames)
/// and the step to sign.
pub enum FlushOut {
    Need(Vec<String>),
    Ready { seq: u64, hash: [u8; 32], puts: Vec<(String, Vec<Vec<u8>>)> },
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
        /// `data`: the account's data key seed (32 bytes), or empty.
        pub fn frames_provision(&mut self, seed: &[u8], did: &[u8], pin: String, data: &[u8]) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Provision { seed: b32(seed)?, did: b32(did)?, pin, data: data.to_vec() })
        }
        /// `[id, frames]` of a Handover asked of an earlier build (`<key>:<code hash>`).
        pub fn frames_handover_from(&mut self, prior: &str, pin: String) -> Result<js_sys::Array, JsValue> {
            let (id, f) = self.0.frames_handover(prior, pin).map_err(err)?;
            Ok([JsValue::from(id), JsValue::from(frames(f))].into_iter().collect())
        }
        pub fn frames_unlock(&mut self, pin: String) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Unlock { pin })
        }
        pub fn frames_lock(&mut self) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Lock)
        }
        pub fn frames_who(&mut self) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Who)
        }
        pub fn frames_sign(&mut self, params: &[u8], seq: u64, value_hash: &[u8]) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Sign { params: params.to_vec(), seq, value_hash: b32(value_hash)? })
        }
        pub fn frames_export(&mut self) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Export)
        }
        /// Leave to write these tables, asked in one prompt (the node may prompt the person; the answer can take a
        /// minute).
        pub fn frames_grant(&mut self, tables: Vec<String>) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Grant { tables })
        }
        pub fn frames_grants(&mut self) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Grants)
        }
        /// Withdraw a grant: `app` is the site's id in its text form.
        pub fn frames_revoke(&mut self, app: &str, table: String) -> Result<js_sys::Array, JsValue> {
            let app = freenet_stdlib::prelude::ContractInstanceId::from_base58(app).map_err(|e| err(format!("not a site id: {e}")))?;
            self.ask(Request::Revoke { app: *app, table })
        }
        /// For the next version of the identity delegate: the member this PIN opens (to its home app).
        pub fn frames_handover(&mut self, pin: String) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Handover { pin })
        }

        /// The account of these recovery words (their entropy), with `member` admitted to it: `{ did, didBytes,
        /// members: { id, frames } }`. The DID is the owner's public key, so the same words always name the same
        /// account; the member PUT merges into the account's Set. So this is both "register" and "log in with words".
        pub fn account(&mut self, set_code: &[u8], entropy: &[u8], member: &[u8], ts: f64) -> Result<js_sys::Object, JsValue> {
            let bad = || err("recovery entropy: 16 or 32 bytes".into());
            let owner = account::owner(entropy).ok_or_else(bad)?;
            let members = account::admit(set_code, entropy, &b32(member)?, ts as u64, "").ok_or_else(bad)?;
            let o = js_sys::Object::new();
            let set = |k: &str, v: JsValue| js_sys::Reflect::set(&o, &k.into(), &v).map(|_| ());
            set("did", account::did(&owner).into())?;
            set("didBytes", js_sys::Uint8Array::from(&owner[..]).into())?;
            let p = js_sys::Object::new();
            js_sys::Reflect::set(&p, &"id".into(), &members.id.clone().into())?;
            js_sys::Reflect::set(&p, &"frames".into(), &frames(self.0.frames_put(&members).map_err(err)?).into())?;
            set("members", p.into())?;
            Ok(o)
        }

        /// The account's data key seed from the words' entropy (handed once to the identity delegate, never kept).
        pub fn data_seed(entropy: &[u8]) -> Result<js_sys::Uint8Array, JsValue> {
            account::data_seed(entropy)
                .map(|d| js_sys::Uint8Array::from(&d[..]))
                .ok_or_else(|| err("recovery entropy: 16 or 32 bytes".into()))
        }

        /// The BIP39 words of 16 or 32 bytes of entropy.
        pub fn words_of(entropy: &[u8]) -> Result<String, JsValue> {
            account::words(entropy).ok_or_else(|| err("recovery entropy: 16 or 32 bytes".into()))
        }

        /// The entropy of 12 or 24 BIP39 words; an error names why not.
        pub fn entropy_of(words: &str) -> Result<js_sys::Uint8Array, JsValue> {
            account::entropy(words)
                .map(|e| js_sys::Uint8Array::from(&e[..]))
                .ok_or_else(|| err("these are not 12 or 24 recovery words (a word is misspelled, or one is missing)".into()))
        }

        /// `[id hex, frames]` of a GET of contract `id` (32 bytes).
        pub fn frames_get(&mut self, id: &[u8]) -> Result<js_sys::Array, JsValue> {
            let id = b32(id)?;
            let f = self.0.frames_get(id).map_err(err)?;
            Ok([JsValue::from(hex(&id)), JsValue::from(frames(f))].into_iter().collect())
        }

        /// The member Set's contract id (32 bytes) for an owner key.
        pub fn members_id(&self, set_code: &[u8], owner: &[u8]) -> Result<js_sys::Uint8Array, JsValue> {
            let p = account::members_address(set_code, &b32(owner)?).ok_or_else(|| err("not an owner key".into()))?;
            Ok(js_sys::Uint8Array::from(&p.id_bytes[..]))
        }

        /// The members the got member Set names, as JSON text `[{ key, name, since }]` (every signature checked).
        pub fn members(&self, set_code: &[u8], owner: &[u8]) -> Result<String, JsValue> {
            let owner = b32(owner)?;
            let id = account::members_address(set_code, &owner).ok_or_else(|| err("not an owner key".into()))?.id_bytes;
            let state = self.0.got(&id).ok_or_else(|| err("the member list has not been read".into()))?;
            let list = account::members(&owner, state).ok_or_else(|| err("the member list does not verify".into()))?;
            let v: Vec<Value> = list.iter().map(|m| json!({ "key": hex(&m.key), "name": m.name, "since": m.since })).collect();
            Ok(Value::from(v).to_string())
        }

        /// The 32 bytes of a contract id written in base58 (as in a `/v1/contract/web/<id>/` path).
        pub fn id_bytes(id: &str) -> Result<js_sys::Uint8Array, JsValue> {
            let i = freenet_stdlib::prelude::ContractInstanceId::from_base58(id).map_err(|e| err(format!("not a contract id: {e}")))?;
            Ok(js_sys::Uint8Array::from(&i[..]))
        }

        /// Open one of the account's tables under its data key: its contract id (hex).
        pub fn tail_open(&mut self, tail_code: &[u8], key: &[u8], table: &str) -> Result<String, JsValue> {
            if table.is_empty() || table.len() > 32 {
                return Err(err("a table's name is 1 to 32 bytes".into()));
            }
            Ok(hex(&self.0.tail_open(tail_code, &b32(key)?, table)))
        }

        /// `[id hex, frames]`: read an open tail and follow it.
        pub fn tail_get(&mut self, id: &[u8]) -> Result<js_sys::Array, JsValue> {
            let id = b32(id)?;
            let f = self.0.frames_tail_get(&id).map_err(err)?;
            Ok([JsValue::from(hex(&id)), JsValue::from(frames(f))].into_iter().collect())
        }

        /// Forget what this page holds of an open table (a write the node refused, so the local step never landed):
        /// read it again before the next write.
        pub fn tail_reset(&mut self, id: &[u8]) -> Result<(), JsValue> {
            self.0.tail(&b32(id)?).map_err(err)?.reset();
            Ok(())
        }

        /// Mark an open tail as absent from the network (a GET found none): its first write is a PUT.
        pub fn tail_absent(&mut self, id: &[u8]) -> Result<(), JsValue> {
            self.0.tail(&b32(id)?).map_err(err)?.on_network = false;
            Ok(())
        }

        /// The next write to an open tail: set `key` to `value` (empty value: delete). Returns `{ params, seq,
        /// valueHash }`, what the identity delegate signs; nothing moves until `tail_commit`.
        pub fn tail_prepare(&mut self, id: &[u8], key: &[u8], value: &[u8]) -> Result<js_sys::Object, JsValue> {
            let o = self.0.tail(&b32(id)?).map_err(err)?;
            let op = if value.is_empty() {
                tail::Op::Delete { key: key.to_vec() }
            } else {
                tail::Op::Set { key: key.to_vec(), value: value.to_vec() }
            };
            let (seq, hash) = o.prepare(vec![op]).ok_or_else(|| err("the tail refuses that write (too large?)".into()))?;
            let out = js_sys::Object::new();
            js_sys::Reflect::set(&out, &"params".into(), &js_sys::Uint8Array::from(&o.params[..]).into())?;
            js_sys::Reflect::set(&out, &"seq".into(), &JsValue::from(seq as f64))?;
            js_sys::Reflect::set(&out, &"valueHash".into(), &js_sys::Uint8Array::from(&hash[..]).into())?;
            Ok(out)
        }

        /// The delegate's signature (hex) for the prepared write: `[kind, frames]`, kind "put" or "update".
        pub fn tail_commit(&mut self, id: &[u8], sig_hex: &str) -> Result<js_sys::Array, JsValue> {
            let id = b32(id)?;
            let sig: [u8; 64] = unhex(sig_hex).and_then(|b| b.try_into().ok()).ok_or_else(|| err("not a signature".into()))?;
            let send = self.0.tail(&id).map_err(err)?.commit(sig).ok_or_else(|| err("the signature does not cover that write".into()))?;
            let kind = if matches!(send, data::Send::Put(_)) { "put" } else { "update" };
            let f = self.0.frames_send(&id, send).map_err(err)?;
            Ok([JsValue::from(kind), JsValue::from(frames(f))].into_iter().collect())
        }

        /// An open table's view, as JSON text: `{ kind: "tail", tail: { id, seq, rows: [{ key, value }], root,
        /// pending } }`, or `{ kind: "tail-need", blocks: [Block contract ids to GET] }`.
        pub fn tail_view(&mut self, id: &[u8]) -> Result<String, JsValue> {
            Ok(self.0.tail_view(&b32(id)?).to_string())
        }

        pub fn set_block_code(&mut self, code: &[u8]) {
            self.0.set_block_code(code);
        }

        /// How many rows wait in an open table's tail (a flush is due at `data::FLUSH_AT`).
        pub fn tail_pending(&mut self, id: &[u8]) -> Result<u32, JsValue> {
            Ok(self.0.tail(&b32(id)?).map_err(err)?.pending_rows() as u32)
        }

        pub fn flush_at() -> u32 {
            data::FLUSH_AT as u32
        }

        /// FLUSH an open table. `{ need: [Block contract ids] }` to GET first; or `{ params, seq, valueHash, puts:
        /// [[name, frames]] }`: send every put and see each accepted, THEN sign and `tail_commit` as for a write.
        pub fn tail_flush(&mut self, id: &[u8]) -> Result<js_sys::Object, JsValue> {
            let id = b32(id)?;
            let out = js_sys::Object::new();
            match self.0.tail_flush(&id).map_err(err)? {
                FlushOut::Need(blocks) => {
                    let a: js_sys::Array = blocks.into_iter().map(JsValue::from).collect();
                    js_sys::Reflect::set(&out, &"need".into(), &a.into())?;
                }
                FlushOut::Ready { seq, hash, puts } => {
                    let params = self.0.tail(&id).map_err(err)?.params.clone();
                    js_sys::Reflect::set(&out, &"params".into(), &js_sys::Uint8Array::from(&params[..]).into())?;
                    js_sys::Reflect::set(&out, &"seq".into(), &JsValue::from(seq as f64))?;
                    js_sys::Reflect::set(&out, &"valueHash".into(), &js_sys::Uint8Array::from(&hash[..]).into())?;
                    let a: js_sys::Array =
                        puts.into_iter().map(|(name, f)| -> JsValue { [JsValue::from(name), JsValue::from(frames(f))].into_iter().collect::<js_sys::Array>().into() }).collect();
                    js_sys::Reflect::set(&out, &"puts".into(), &a.into())?;
                }
            }
            Ok(out)
        }

        /// A contract id's base58 form, as the node names it in acks.
        pub fn id_name(id: &[u8]) -> Result<String, JsValue> {
            Ok(wire::contract_id(b32(id)?).encode())
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

    /// `did:craftec:<base58>` of the owner's public key (the DID the identity delegate answers, as bytes).
    #[wasm_bindgen]
    pub fn did_of(owner: &[u8]) -> Result<String, JsValue> {
        Ok(account::did(&b32(owner)?))
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
        let (b, _) = c.frames_identity(&Request::Who).unwrap();
        assert_eq!((a, b), (1, 2));
        assert!(!fa.is_empty());
        assert!(!c.frames_register_identity().unwrap().is_empty());
    }

    #[test]
    fn a_handover_is_addressed_to_the_earlier_build_it_names() {
        let mut c = Core::new(b"\0asm\x01\0\0\0");
        let prior = "AP3BuSjFrhJ45Fg7iTvsFb2ER8Hda3iXR3eXjTfrLvnQ:9gcqi8176H7WehFLSqsT1s6BZBxJD4DfX2X4g2Sma7r2";
        let (_, to_prior) = c.frames_handover(prior, "123456".into()).unwrap();
        let has = |f: &[Vec<u8>], k: &[u8]| f.iter().any(|b| b.windows(k.len()).any(|w| w == k));
        let key = bs58::decode("AP3BuSjFrhJ45Fg7iTvsFb2ER8Hda3iXR3eXjTfrLvnQ").into_vec().unwrap();
        assert!(has(&to_prior, &key), "addressed to the earlier build's key");
        let (_, to_this) = c.frames_identity(&Request::Handover { pin: "123456".into() }).unwrap();
        assert!(!has(&to_this, &key), "control: this build's own request does not carry it");
        assert!(c.frames_handover("no-colon", "1".into()).is_err());
        assert!(c.frames_handover("abc:def", "1".into()).is_err());
    }

    #[test]
    fn answers_read_as_json() {
        let a = answer_json(&Answer::Unlocked { public: [1; 32], did: [2; 32], data: Some([3; 32]) });
        assert_eq!(a["unlocked"]["data"], hex(&[3; 32]));
        assert_eq!(a["unlocked"]["did"], hex(&[2; 32]));
        assert_eq!(answer_json(&Answer::WrongPin { tries_left: 3 })["wrongPin"]["triesLeft"], 3);
        assert_eq!(bytes32("x", &hex(&[7; 32])).unwrap(), [7; 32]);
        assert!(bytes32("x", "zz").is_err());
    }
}
