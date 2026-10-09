//! Run a released contract's real `block.wasm` and ask it to validate a state.
//!
//! Not a Rust copy of the contract's rules — the wasm the network runs. A
//! second implementation of "what the contract accepts" would agree with
//! itself and prove nothing, which is the whole reason the rev-equality gate
//! this replaces was only ever a proxy.
//!
//! The ABI is the freenet-stdlib one, reproduced here because there is no
//! host-side helper for it: `__frnt__initiate_buffer(capacity) -> i64` hands
//! back a `BufferBuilder` in the module's linear memory; the payload goes at
//! `start`; `validate_state(params, state, related) -> i64` points at a
//! `ContractInterfaceResult`.
//!
//! The FRAMING is the node's choice, made from the module's imports exactly as
//! freenet 0.2.136 makes it (`module_has_streaming_io`: any import from
//! `freenet_contract_io`). STREAMING (a freenet-stdlib-built contract): the
//! payload sits behind a 4-byte little-endian length and `last_write` counts
//! both. LEGACY (no such import: the Block contract's hand-written ABI,
//! freenet-contracts#44): the payload alone, and `last_write` is its length. A
//! host that framed every module one way would drive the other kind wrong and
//! read its answers as verdicts.

#![allow(dead_code)]

use wasmi::{Caller, Engine, Extern, Linker, Module, Store};

/// `BufferBuilder`, as the guest lays it out. `#[repr(C)]` with an i64 first,
/// so the u32 capacity is followed by four bytes of padding.
const BB_START: usize = 0;
const BB_CAPACITY: usize = 8;
const BB_LAST_READ: usize = 16;
const BB_LAST_WRITE: usize = 24;

/// `ContractInterfaceResult { ptr: i64, kind: i32, size: u32 }`.
const RES_PTR: usize = 0;
const RES_KIND: usize = 8;
const RES_SIZE: usize = 12;

pub struct Contract {
    store: Store<()>,
    memory: wasmi::Memory,
    initiate: wasmi::TypedFunc<i32, i64>,
    validate: wasmi::TypedFunc<(i64, i64, i64), i64>,
    /// The module imports from `freenet_contract_io`: the node streams to it.
    streaming: bool,
}

/// What the contract said about a state.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Valid,
    Invalid,
    /// It asked for other contracts first. Never expected for a Block.
    RequiresRelated,
    /// The call itself failed — a trap, or a result this build cannot read.
    /// Never treated as either answer: a gate that reads a crash as "invalid"
    /// passes its refusal control for the wrong reason.
    Broken(String),
}

impl Contract {
    pub fn load(wasm: &[u8]) -> Result<Contract, String> {
        let engine = Engine::default();
        let module = Module::new(&engine, wasm).map_err(|e| format!("not loadable: {e}"))?;
        let streaming = module
            .imports()
            .any(|i| i.module() == "freenet_contract_io");
        let mut store = Store::new(&engine, ());
        let mut linker = <Linker<()>>::new(&engine);
        // The one import. Returning 0 means "no more bytes": everything is
        // written up front, so a call that reaches here wanted a refill it
        // should never need, and 0 makes that a failed read rather than a
        // hang.
        linker
            .func_wrap(
                "freenet_contract_io",
                "__frnt__fill_buffer",
                |_: Caller<'_, ()>, _id: i64, _ptr: i64| -> i32 { 0 },
            )
            .map_err(|e| format!("linking fill_buffer: {e}"))?;
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| format!("instantiating: {e}"))?
            .start(&mut store)
            .map_err(|e| format!("starting: {e}"))?;
        let memory = match instance.get_export(&store, "memory") {
            Some(Extern::Memory(m)) => m,
            _ => return Err("the contract exports no memory".into()),
        };
        let initiate = instance
            .get_typed_func::<i32, i64>(&store, "__frnt__initiate_buffer")
            .map_err(|e| format!("__frnt__initiate_buffer: {e}"))?;
        let validate = instance
            .get_typed_func::<(i64, i64, i64), i64>(&store, "validate_state")
            .map_err(|e| format!("validate_state: {e}"))?;
        Ok(Contract {
            store,
            memory,
            initiate,
            validate,
            streaming,
        })
    }

    fn read(&self, at: usize, len: usize) -> Result<Vec<u8>, String> {
        let data = self.memory.data(&self.store);
        data.get(at..at + len)
            .map(|s| s.to_vec())
            .ok_or_else(|| format!("read {len} B at {at} is outside linear memory"))
    }

    fn u32_at(&self, at: usize) -> Result<u32, String> {
        let b = self.read(at, 4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i64_at(&self, at: usize) -> Result<i64, String> {
        let b = self.read(at, 8)?;
        Ok(i64::from_le_bytes(b.try_into().expect("8 bytes")))
    }

    /// Put `payload` into a fresh guest buffer and return its pointer.
    fn buffer(&mut self, payload: &[u8]) -> Result<i64, String> {
        let header = if self.streaming { 4 } else { 0 };
        let total = payload.len() + header;
        let ptr = self
            .initiate
            .call(&mut self.store, total as i32)
            .map_err(|e| format!("initiate_buffer: {e}"))?;
        let bb = ptr as usize;
        let start = self.i64_at(bb + BB_START)? as usize;
        let capacity = self.u32_at(bb + BB_CAPACITY)? as usize;
        if capacity < total {
            return Err(format!("buffer capacity {capacity} < {total}"));
        }
        let mut bytes = Vec::with_capacity(total);
        if self.streaming {
            bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        }
        bytes.extend_from_slice(payload);
        self.memory
            .write(&mut self.store, start, &bytes)
            .map_err(|e| format!("writing the payload: {e}"))?;
        // The guest reads `last_write` to know how much is there.
        let lw = self.i64_at(bb + BB_LAST_WRITE)? as usize;
        self.memory
            .write(&mut self.store, lw, &(total as u32).to_le_bytes())
            .map_err(|e| format!("writing last_write: {e}"))?;
        let lr = self.i64_at(bb + BB_LAST_READ)? as usize;
        self.memory
            .write(&mut self.store, lr, &0u32.to_le_bytes())
            .map_err(|e| format!("writing last_read: {e}"))?;
        Ok(ptr)
    }

    /// What does this contract say about `state` under `params`?
    pub fn validate(&mut self, params: &[u8], state: &[u8]) -> Verdict {
        let related = match bincode::serialize(&RelatedShim::default()) {
            Ok(b) => b,
            Err(e) => return Verdict::Broken(format!("encoding related: {e}")),
        };
        let (p, s, r) = match (
            self.buffer(params),
            self.buffer(state),
            self.buffer(&related),
        ) {
            (Ok(p), Ok(s), Ok(r)) => (p, s, r),
            (a, b, c) => {
                let e = [a.err(), b.err(), c.err()]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join("; ");
                return Verdict::Broken(e);
            }
        };
        let res = match self.validate.call(&mut self.store, (p, s, r)) {
            Ok(v) => v as usize,
            Err(e) => return Verdict::Broken(format!("validate_state trapped: {e}")),
        };
        let (ptr, kind, size) = match (
            self.i64_at(res + RES_PTR),
            self.u32_at(res + RES_KIND),
            self.u32_at(res + RES_SIZE),
        ) {
            (Ok(p), Ok(k), Ok(s)) => (p as usize, k, s as usize),
            _ => return Verdict::Broken("the result is outside linear memory".into()),
        };
        // ResultKind::ValidateState is 0; anything else is an error result,
        // and reading one as a verdict would turn a contract that refused to
        // answer into a contract that said no.
        if kind != 0 {
            let body = self.read(ptr, size).unwrap_or_default();
            return Verdict::Broken(format!(
                "the contract returned an error result (kind {kind}): {}",
                String::from_utf8_lossy(&body)
            ));
        }
        let body = match self.read(ptr, size) {
            Ok(b) => b,
            Err(e) => return Verdict::Broken(e),
        };
        if std::env::var("GATE_TRACE").is_ok() {
            eprintln!("    result: ptr={ptr} kind={kind} size={size} body={body:?}");
        }
        decode_verdict(&body)
    }
}

/// `Result<ValidateResult, ContractError>`, decoded EXACTLY.
///
/// Two little-endian u32s: the `Result` discriminant, then the
/// `ValidateResult` one. Written out rather than derived, because
/// `bincode::deserialize` accepts a PREFIX and returns what it managed —
/// handed these eight bytes for a bare `ValidateResult` it read the leading
/// `Ok` as `Valid` and threw away the half that says which. Every state then
/// came back valid, including one whose hash did not match its key, and the
/// harness looked like a working gate.
///
/// So: the length is checked, both words are read, and anything unexpected is
/// `Broken` rather than a verdict.
fn decode_verdict(body: &[u8]) -> Verdict {
    let word = |i: usize| -> Option<u32> {
        let b = body.get(i * 4..i * 4 + 4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    if body.len() != 8 {
        return Verdict::Broken(format!(
            "a verdict is 8 B (Result tag, then ValidateResult tag); got {} B: {body:?}",
            body.len()
        ));
    }
    match (word(0), word(1)) {
        (Some(0), Some(0)) => Verdict::Valid,
        (Some(0), Some(1)) => Verdict::Invalid,
        (Some(0), Some(2)) => Verdict::RequiresRelated,
        (Some(0), Some(v)) => Verdict::Broken(format!("unknown ValidateResult variant {v}")),
        (Some(1), _) => Verdict::Broken("the contract returned Err".into()),
        _ => Verdict::Broken(format!("undecodable verdict {body:?}")),
    }
}

/// `RelatedContracts::default()`, as bincode sees it: an empty map.
#[derive(Default, serde::Serialize)]
struct RelatedShim {
    map: std::collections::HashMap<[u8; 32], Option<Vec<u8>>>,
}
