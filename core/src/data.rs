//! DATA (ARCHITECTURE §1): a member's TAIL for an app. Each member writes its own tail, a one-writer contract whose
//! state is the tree root plus the rows written since the last flush. A person's data for an app is the overlay of
//! their members' tails.
//!
//! The tail's params are the Register's: the member's key and a label that begins with the APP's site id (the
//! identity delegate signs only such labels for that app). So the address is derived, never looked up.
//!
//! This holds each open tail's WRITER (`tail::Writer`, the SDK's one client of the contract) and feeds it every
//! state the node sends (a GET or a subscription push), verified in full. Signing is the identity delegate's: the
//! page takes `message` out, the delegate signs it, the signature comes back to `commit`.

use craftec_register_contract::wire::{Params, Signed};
use freenet_stdlib::prelude::{ContractContainer, ContractKey};
use serde_json::{json, Value};
use tail::{Op, Unsigned, Writer};

/// The label's name part after the app's 32-byte site id.
pub const TAIL_NAME: &[u8] = b"tail";

pub struct Open {
    pub params: Vec<u8>,
    pub contract: ContractContainer,
    pub writer: Writer,
    /// Whether the network holds this tail (a GET found it, or this page put it). Before, the first write is a PUT
    /// of the whole state; after, every write is an UPDATE carrying one delta.
    pub on_network: bool,
    pub pending: Option<Unsigned>,
}

impl Open {
    pub fn new(tail_code: &[u8], member: &[u8; 32], site: &[u8; 32]) -> Open {
        let params = wire::register_params(member, &[&site[..], TAIL_NAME].concat());
        let (_, contract, _) = wire::puts::contract(tail_code, &params, &[]);
        let writer = Writer::new(&params).expect("our own params parse");
        Open { params, contract, writer, on_network: false, pending: None }
    }

    pub fn id(&self) -> String {
        self.contract.key().id().encode()
    }

    pub fn id_bytes(&self) -> [u8; 32] {
        self.contract.key().id().as_bytes().try_into().expect("32 bytes")
    }

    pub fn key(&self) -> ContractKey {
        self.contract.key()
    }

    /// A state from the network. Taken only if it verifies and is not behind what this writer holds.
    pub fn absorb(&mut self, state: &[u8]) -> bool {
        self.on_network = true;
        match Writer::resume(&self.params, state) {
            Some(w) if w.seq() >= self.writer.seq() => {
                self.writer = w;
                true
            }
            _ => false,
        }
    }

    /// The next step: what to sign (the Register's message fields).
    pub fn prepare(&mut self, ops: Vec<Op>) -> Option<(u64, [u8; 32])> {
        let u = self.writer.prepare(ops)?;
        let out = (u.seq, u.body.hash());
        self.pending = Some(u);
        Some(out)
    }

    /// The signature for the prepared step. Returns what to send: `Put(state)` if the network has no tail yet, else
    /// `Update(delta)`.
    pub fn commit(&mut self, sig: [u8; 64]) -> Option<Send> {
        let u = self.pending.take()?;
        let signed = Signed { terminal: false, seq: u.seq, value_hash: u.body.hash(), bitmap: 0, sigs: vec![sig] };
        let delta = self.writer.commit(u, signed)?;
        Some(if self.on_network {
            Send::Update(delta)
        } else {
            self.on_network = true;
            Send::Put(self.writer.state())
        })
    }

    /// The rows, for the page: the tail's pending rows (the tree's come with the flush step).
    pub fn rows(&self) -> Value {
        let body = self.writer.body();
        let rows: Vec<Value> = body
            .entries
            .iter()
            .filter_map(|(k, e)| {
                e.value.as_ref().map(|v| json!({ "key": String::from_utf8_lossy(k), "value": String::from_utf8_lossy(v) }))
            })
            .collect();
        json!({ "id": self.id(), "seq": self.writer.seq(), "rows": rows, "root": body.root.map(|r| crate::hex(&r)) })
    }
}

pub enum Send {
    Put(Vec<u8>),
    Update(Vec<u8>),
}

/// Whether `params` are a tail of `site` (what the identity delegate will check), for a clear error before asking.
pub fn is_tail_of(params: &[u8], site: &[u8; 32]) -> bool {
    Params::parse(params).is_some_and(|p| p.label.starts_with(site))
}

#[cfg(test)]
mod tests {
    use super::*;
    use craftec_register_contract::wire::Signed as S;
    use ed25519_dalek::{Signer, SigningKey};

    const CODE: &[u8] = b"\0asm\x01\0\0\0";

    fn sign(key: &SigningKey, o: &Open, seq: u64, hash: [u8; 32]) -> [u8; 64] {
        let p = Params::parse(&o.params).unwrap();
        key.sign(&p.signed_message(false, seq, &hash)).to_bytes()
    }

    #[test]
    fn first_write_puts_the_state_then_deltas_and_another_writer_resumes() {
        let key = SigningKey::from_bytes(&[5; 32]);
        let member = key.verifying_key().to_bytes();
        let mut o = Open::new(CODE, &member, &[0xA1; 32]);
        assert!(is_tail_of(&o.params, &[0xA1; 32]));
        let (seq, h) = o.prepare(vec![Op::Set { key: b"a".to_vec(), value: b"1".to_vec() }]).unwrap();
        let Some(Send::Put(state)) = o.commit(sign(&key, &o, seq, h)) else { panic!("first write is a put") };
        let (seq, h) = o.prepare(vec![Op::Set { key: b"b".to_vec(), value: b"2".to_vec() }]).unwrap();
        assert!(matches!(o.commit(sign(&key, &o, seq, h)), Some(Send::Update(_))));
        assert_eq!(o.rows()["rows"].as_array().unwrap().len(), 2);
        // Another page of the same member reads the first state from the network.
        let mut other = Open::new(CODE, &member, &[0xA1; 32]);
        assert!(other.absorb(&state));
        assert_eq!(other.writer.seq(), 1);
        // An older state never moves a writer back.
        assert!(!o.absorb(&state));
        assert_eq!(o.writer.seq(), 2);
    }

    #[test]
    fn a_wrong_signature_sends_nothing() {
        let key = SigningKey::from_bytes(&[5; 32]);
        let mut o = Open::new(CODE, &key.verifying_key().to_bytes(), &[0xA1; 32]);
        let (seq, h) = o.prepare(vec![Op::Set { key: b"a".to_vec(), value: b"1".to_vec() }]).unwrap();
        let wrong = SigningKey::from_bytes(&[6; 32]);
        assert!(o.commit(sign(&wrong, &o, seq, h)).is_none());
        assert_eq!(o.writer.seq(), 0);
        let _ = S { terminal: false, seq: 0, value_hash: [0; 32], bitmap: 0, sigs: vec![] };
    }
}
