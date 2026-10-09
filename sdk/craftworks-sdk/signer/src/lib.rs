//! # The SIGNER (sdk#209)
//!
//! ENGINE-SHAPE §4–§6 (owner's GO, 2026-09-23). The head Register is NOT compare-and-set (F56, measured: two writers
//! at one seq are both told success; the lower value hash wins; fork evidence sticks). With one key per device, the
//! only thing that can keep that key from FORKING its own head is whoever holds the key refusing to sign twice. That
//! is this delegate, and it is ALL it is:
//!
//! **ONE verb:** `sign(prev, next) → Signed(state) | NotNext{current} | AlreadySigned(first state) | Refused(why)`
//! (plus `Provision`, which stores the key and what naming the Register and a block needs, as ONE record).
//!
//! - A LOCAL guard so one key never forks, NOT the arbiter. The head it signs is a CLAIM; finality is the attested
//!   CHECKPOINT (owner, 2026-09-23): once a seq is checkpointed it is CLOSED, a late competing claim is rejected there,
//!   and the page rebases onto the attested root.
//! - **No GETs, no contract ops, no tree logic, no memory beyond ONE record.** It reads only by the node's
//!   SYNCHRONOUS local read (`get_contract_state`), so it can never park (F50: only a cold GET/SUBSCRIBE parks a
//!   delegate), never strand, and has nothing to settle.
//! - **But it shares the executor's ONE default queue** (F51; `fair_queue.rs`: a delegate request names no contract,
//!   so it goes to the default queue on the serial `contract_handling` loop, capped at 100 waiting). It waits behind
//!   whatever iteration of another delegate's chain is running, and a burst from another delegate can get a sign
//!   request REFUSED "queue full" with no answer. The page treats a refusal as "ask again" (as sdk#196 1b does): a
//!   re-ask is safe by rule (b), which answers the first record.
//! - **The record is on disk before the reply is built.** `sign` replies `Signed` only after `set_secret(RECORD)`
//!   returned true, and the node's `store_secret` writes a tmp file, `sync_all`s and atomically renames BEFORE it
//!   returns (freenet-core v0.2.135 `wasm_runtime/secrets_store/store.rs:790–832`, read by the architect, sdk#210
//!   review). So "record lost after a signature left" -- the one ordering that would sign twice -- is excluded by
//!   SOURCE; `live-signer` confirms it once (SIGKILL the instant `Signed` arrived, restart, `AlreadySigned(first)`).
//!   A crash INSIDE the write loses a signature nobody received: no fork.
//!
//! [`decide`] is the rule, pure. [`serve`] is the whole request over a [`Host`] (secrets + the sync read), so every
//! ordering the issue names -- the race, identical re-ask, one signature per prev, a record that was not saved -- is
//! driven natively with nothing but a map. `delegate` (the entry the node calls) is `serve` over the real ctx.

use serde::{Deserialize, Serialize};

// The wire types live in `signer-proto`, so the page can speak them without linking this delegate.
pub use signer_proto::{
    decode_answer, decode_request, encode_answer, encode_request, request_id, Answer, Head, Label, Next,
    Request, Why, MAGIC, MAX_HELD, UNATTRIBUTED,
};

/// THE ONE RECORD the signer keeps: the last thing it signed, from which prev, and the exact bytes returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub prev: Head,
    pub next: Next,
    pub signed: Vec<u8>,
}

/// Everything the rule consults, and nothing else.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub provisioned: bool,
    /// Its one record (`None`: it has never signed).
    pub record: Option<Record>,
    /// The head by the SYNCHRONOUS local read of the Register (`None`: not held locally).
    pub head_read: Option<Head>,
    /// Whether the root block of `next.root` is held (the same sync read).
    pub root_held: bool,
}

/// What the entry must DO.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Sign `next`, write the record -- and reply `Signed` ONLY if that write succeeded.
    Sign,
    /// Reply with this and change nothing.
    Reply(Answer),
}

/// THE RULE, pure. Applied in this order:
/// (a) `next.seq == prev.seq + 1`, else `Refused(NotSuccessor)`; not provisioned → `Refused(NotProvisioned)`; a value
///     whose ledger is not the format (`signer_proto::head::check`) → `Refused(BadLedger)`.
/// (b) AT MOST ONE SIGNATURE PER PREV: `record.prev == prev` → `AlreadySigned(record.signed)`, identical re-ask or a
///     different `next` alike. Checked BEFORE (c), so a re-ask after the head moved on still gets its own bytes back.
/// (c) TRUTH = the later of `record.next` and `head_read`, by seq, and at an EQUAL seq the Register's (a read AHEAD
///     is another holder moving the head: the page rebases, nothing is displaced; a read BEHIND is the page's UPDATE
///     not landed yet: the record wins; a read at the record's seq with ANOTHER root is another device of this
///     identity whose head the Register's tie-break kept -- the same identity never forks (owner, sdk#225), so that
///     head is the truth, this signer signs on from it only, and never displaces it: sdk#210 review §1).
///     `prev != truth` → `NotNext{ current: truth }`. No truth at all: only the genesis (`prev.seq == 0`) is signable,
///     else `Refused(HeadUnknown)`. Stated limit: a node that does not HOLD the Register (a second device, a fresh
///     node) with no record signs seq 1 for a key whose head may be further on elsewhere. Harmless at the Register
///     (the higher seq wins) but the page's UPDATE then loses silently, and it learns so only from the head
///     subscription.
/// (d) the root block must be held AND hash to `next.root`, else `Refused(RootNotHeld)`.
/// Otherwise `Sign`.
pub fn decide(f: &Facts, prev: &Head, next: &Next) -> Decision {
    use Answer::*;
    if next.seq != prev.seq.wrapping_add(1) || prev.seq == u64::MAX {
        return Decision::Reply(Refused(Why::NotSuccessor));
    }
    if !f.provisioned {
        return Decision::Reply(Refused(Why::NotProvisioned));
    }
    // The value is the ONE format (signer_proto::head) or nothing is signed:
    // this is the one place a malformed ledger can be stopped.
    if signer_proto::head::check(&next.value()).is_err() {
        return Decision::Reply(Refused(Why::BadLedger));
    }
    if let Some(r) = &f.record {
        if r.prev == *prev {
            return Decision::Reply(AlreadySigned(r.signed.clone()));
        }
    }
    let signed_head = f.record.as_ref().map(|r| r.next.head());
    let truth = match (signed_head, f.head_read) {
        (Some(mine), Some(read)) if read.seq >= mine.seq => Some(read),
        (Some(mine), _) => Some(mine),
        (None, read) => read,
    };
    match truth {
        Some(t) if t != *prev => return Decision::Reply(NotNext { current: t }),
        None if prev.seq != 0 => return Decision::Reply(Refused(Why::HeadUnknown)),
        _ => {}
    }
    if !f.root_held {
        return Decision::Reply(Refused(Why::RootNotHeld));
    }
    Decision::Sign
}

/// Secret-store names.
pub const RECORD: &[u8] = b"signer_record";
/// THE PROVISIONING, one record written by ONE `set_secret` (sdk#334): the key and what names the Register and a
/// block. A host write is all or nothing per secret, so no failure can leave a key without its Register -- the
/// signer is unprovisioned, or provisioned whole. Read only through [`provisioned`].
pub const PROVISION: &[u8] = b"signer_provision";

/// What the signer is provisioned AS: the key, the Register it signs for (its params and its code's HASH), and the
/// Block contract's code HASH. Hashes, not code: the signer only NAMES contracts (`contract_keys::instance`), so the
/// record is a few hundred bytes whatever the code weighs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provisioned {
    pub key: Vec<u8>,
    pub register_params: Vec<u8>,
    pub register_code_hash: [u8; 32],
    pub block_code_hash: [u8; 32],
}

/// The record's layout version.
const PROVISION_VERSION: u8 = 1;

impl Provisioned {
    /// `version ‖ len u32 ‖ key ‖ len u32 ‖ register params ‖ register code hash [32] ‖ block code hash [32]`.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![PROVISION_VERSION];
        for field in [&self.key, &self.register_params] {
            out.extend_from_slice(&(field.len() as u32).to_le_bytes());
            out.extend_from_slice(field);
        }
        out.extend_from_slice(&self.register_code_hash);
        out.extend_from_slice(&self.block_code_hash);
        out
    }

    /// `None` for anything that is not exactly one record of this layout.
    pub fn decode(b: &[u8]) -> Option<Provisioned> {
        let (&v, mut r) = b.split_first()?;
        if v != PROVISION_VERSION {
            return None;
        }
        let mut field = || -> Option<Vec<u8>> {
            let (len, rest) = r.split_at_checked(4)?;
            let len = u32::from_le_bytes(len.try_into().ok()?) as usize;
            let (f, rest) = rest.split_at_checked(len)?;
            r = rest;
            Some(f.to_vec())
        };
        let key = field()?;
        let register_params = field()?;
        let (rh, rest) = r.split_at_checked(32)?;
        let (bh, rest) = rest.split_at_checked(32)?;
        if !rest.is_empty() {
            return None;
        }
        Some(Provisioned { key, register_params, register_code_hash: rh.try_into().ok()?, block_code_hash: bh.try_into().ok()? })
    }

    /// The Register this provisioning signs for.
    pub fn register_id(&self) -> [u8; 32] {
        contract_keys::instance(&self.register_code_hash, &self.register_params)
    }
}

/// THE ONE READER of the provisioning (sdk#334): what the Register query, Sign and Provision all consult.
pub fn provisioned<H: Host>(host: &H) -> Option<Provisioned> {
    host.get_secret(PROVISION).and_then(|b| Provisioned::decode(&b))
}

/// What the signer reaches, and nothing more: its secret store and the node's SYNCHRONOUS local read.
pub trait Host {
    fn get_secret(&self, key: &[u8]) -> Option<Vec<u8>>;
    fn set_secret(&mut self, key: &[u8], value: &[u8]) -> bool;
    fn contract_state(&self, instance_id: &[u8; 32]) -> Option<Vec<u8>>;
}

/// The Register's instance id, from its code and params (`contract_keys::instance` over the code's hash).
pub fn register_id(code: &[u8], params: &[u8]) -> [u8; 32] {
    contract_keys::instance(&contract_keys::code_hash(code), params)
}

/// One request, served.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Served {
    /// The request's id, echoed on the answer (SG02): what attributes it with several requests in flight.
    pub id: u32,
    pub answer: Answer,
}

/// WHO ASKED, as the node attests it (builder#117, rule 13). `Local`: no attestation -- the person's own tools on
/// their own node. `Served`: a web app the node serves, or a delegate (which may be relaying for one: the node then
/// REPLACES the app's origin with the delegate's). A site is never signed for `Served`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Local,
    Served,
}

/// One request, answered: `serve_full` without the id.
pub fn serve<H: Host>(host: &mut H, request: &[u8], origin: Origin) -> Answer {
    serve_full(host, request, origin).answer
}

/// The bytes the entry sends back for a served request: its answer, under ITS id.
pub fn reply(served: &Served) -> Vec<u8> {
    encode_answer(served.id, &served.answer)
}

/// One request, served: gather the facts, decide, and -- on `Sign` -- sign, save the record, reply.
pub fn serve_full<H: Host>(host: &mut H, request: &[u8], origin: Origin) -> Served {
    let Some((id, req)) = decode_request(request) else {
        return Served {
            id: request_id(request),
            answer: Answer::Refused(Why::Unreadable),
        };
    };
    let answer = match req {
        Request::Held { contracts } => held(host, &contracts),
        Request::Provision {
            signing_key,
            register_code,
            register_params,
            block_code,
        } => provision(
            host,
            signing_key,
            register_code,
            register_params,
            block_code,
        ),
        Request::Sign { prev, next, label } => sign(host, prev, next, label, origin),
        // The Register it signs for: provisioned whole, or nothing (sdk#334).
        Request::Register => Answer::Register {
            params: provisioned(host).map(|p| p.register_params),
        },
    };
    Served { id, answer }
}

/// Store the key and what names the Register and a block, as ONE record in ONE write (sdk#334, the table on the
/// issue): refused for another key, or the same key for another Register. A failed write leaves the provisioning as
/// it was -- none, or the old one whole -- and the same Provision again completes it.
fn provision<H: Host>(
    host: &mut H,
    signing_key: Vec<u8>,
    register_code: Vec<u8>,
    register_params: Vec<u8>,
    block_code: Vec<u8>,
) -> Answer {
    let next = Provisioned {
        key: signing_key,
        register_params,
        register_code_hash: contract_keys::code_hash(&register_code),
        block_code_hash: contract_keys::code_hash(&block_code),
    };
    if let Some(held) = provisioned(host) {
        if held.key != next.key {
            return Answer::Refused(Why::KeyAlreadyProvisioned);
        }
        // The same key re-provisioned: only for the SAME Register. The record belongs to the Register it was
        // signed for; another Register's requests must never be answered from it.
        if (&held.register_params, held.register_code_hash) != (&next.register_params, next.register_code_hash) {
            return Answer::Refused(Why::RegisterChanged);
        }
    }
    if host.set_secret(PROVISION, &next.encode()) {
        Answer::Provisioned
    } else {
        Answer::Refused(Why::NotProvisioned)
    }
}

/// READ-LOCAL: whether this node holds each contract's state, by the host's synchronous local read -- never a
/// network fetch, so it cannot park. Needs no provisioning: it reads, and says nothing but present / absent.
fn held<H: Host>(host: &H, contracts: &[[u8; 32]]) -> Answer {
    if contracts.is_empty() || contracts.len() > MAX_HELD {
        return Answer::Refused(Why::BlockCount {
            max: MAX_HELD as u32,
            got: contracts.len() as u32,
        });
    }
    Answer::Held {
        present: contracts
            .iter()
            .map(|c| host.contract_state(c).is_some())
            .collect(),
    }
}

/// A record `(seq, value)` read out of a Register state ONLY IF it verifies under exactly `params`: the Register
/// crate's own `read` (the format's one owner, the library the site contract links) parses the state and verifies
/// every signature in it, the record's and any fork evidence's, under the params' authority (mode 0 or a keyset).
/// A state carrying VALID fork evidence still yields its record: two publishers at one version is the race a site
/// is built to settle, the Register keeps the winner and the evidence (sticky, F56), and the signer needs only the
/// record as truth. This is what lets the signer take a page-named site contract as truth (architect, builder#117
/// Q1): an unverified record never counts.
pub fn verified_record(params: &[u8], state: &[u8]) -> Option<(u64, Vec<u8>)> {
    let (_, s) = craftec_register_contract::read(params, state)?;
    let r = s.record?;
    Some((r.decision().seq, r.value))
}

/// The secret a label's ONE record is kept under: the head's is `signer_record` (unchanged, so nothing migrates);
/// a site's is `signer_record/site:<app>` -- one record type, keyed by label (builder#117).
pub fn record_name(label: &Label) -> Vec<u8> {
    match label {
        Label::Head => RECORD.to_vec(),
        Label::Site { app, .. } => [RECORD, b"/site:", app.as_bytes()].concat(),
        Label::Obs => [RECORD, b"/obs"].concat(),
    }
}

/// THE sign verb, for every label: the label chooses the params, the record and the contract read as the truth,
/// and `decide` is the one rule over them (builder#117).
fn sign<H: Host>(host: &mut H, prev: Head, next: Next, label: Label, origin: Origin) -> Answer {
    // A site is signed only for the person's own tools (rule 13): a served app could republish its visitor's site.
    if matches!(label, Label::Site { .. }) && origin == Origin::Served {
        return Answer::Refused(Why::FromApp);
    }
    // THE ONE READER of the provisioning (sdk#334): whole, or nothing.
    let prov = provisioned(host);
    // The label's params: the head's own, or the same authority relabelled `site:<app>` (contract_keys::site).
    let params = match (&label, prov.as_ref().map(|p| &p.register_params)) {
        (Label::Head, p) => p.cloned(),
        (Label::Site { app, .. }, Some(p)) => match contract_keys::site::site_params(p, app) {
            Some(sp) => Some(sp),
            None => return Answer::Refused(Why::BadLabel),
        },
        (Label::Site { .. }, None) => None,
        // The observation tree's head: the same authority under the name `obs` (sdk#399).
        (Label::Obs, Some(p)) => match contract_keys::site::obs_params(p) {
            Some(op) => Some(op),
            None => return Answer::Refused(Why::BadLabel),
        },
        (Label::Obs, None) => None,
    };
    // FAIL CLOSED: a record that is there and does not decode is not "no record" -- read as none, any seq could be
    // signed again from a prev already signed from (sdk#332's review).
    let record: Option<Record> = match host.get_secret(&record_name(&label)) {
        None => None,
        Some(b) => match bincode::deserialize(&b) {
            Ok(r) => Some(r),
            Err(_) => return Answer::Refused(Why::CannotSign),
        },
    };
    let head_read = match &label {
        // The observation tree's head is read exactly as the data head, from ITS register (sdk#399).
        Label::Head | Label::Obs => match (&prov, &params) {
            (Some(pv), Some(p)) => host
                .contract_state(&contract_keys::instance(&pv.register_code_hash, p))
                // TOLERANT (signer_proto::head): the root is the value's first 32
                // bytes, whatever ledger follows.
                .and_then(|st| {
                    let (seq, v) = signer_proto::head::record_of(&st)?;
                    Some(Head { seq, root: signer_proto::head::read_value(v)?.root })
                }),
            _ => None,
        },
        // The page names the site contract; what it holds counts only if it VERIFIES under this site's params (Q1):
        // an unsigned record is nothing, a genuinely signed newer one is a legitimate skip.
        Label::Site { contract, .. } => params.as_deref().and_then(|p| {
            let st = host.contract_state(contract)?;
            let (meta, _web) = contract_keys::site::framing(&st)?;
            let (seq, value) = verified_record(p, meta)?;
            Some(Head { seq, root: value.as_slice().try_into().ok()? })
        }),
    };
    // Held AND the right block: the state is `kind ‖ body` and must hash to the root it is named by (as
    // entry.rs::block_state names a block), not merely be present. A HEAD fact: a site's value is blake3(web), and
    // no block holds it.
    let root_held = match &label {
        // The observation tree is a prolly tree like the data's: its root is a block too.
        Label::Head | Label::Obs => prov.as_ref().is_some_and(|p| {
            host.contract_state(&contract_keys::block::contract_deriver_of_hash(p.block_code_hash)(&next.root))
                .is_some_and(|s| matches!(s.split_first(), Some((&k, body)) if freenet_prolly::block_id(k, body) == next.root))
        }),
        Label::Site { .. } => true,
    };
    let facts = Facts {
        provisioned: prov.is_some(),
        record,
        head_read,
        root_held,
    };
    let rec_label = label;
    match decide(&facts, &prev, &next) {
        Decision::Reply(a) => a,
        Decision::Sign => {
            let (Some(p), Some(params)) = (prov, params) else {
                return Answer::Refused(Why::NotProvisioned);
            };
            let Ok(signed) =
                contract_keys::register::head_state(&params, &p.key, next.seq, &next.value())
            else {
                return Answer::Refused(Why::CannotSign);
            };
            let rec = Record {
                prev,
                next,
                signed: signed.clone(),
            };
            // THE ORDER: the record is written BEFORE the signature leaves. A reply without its record could be signed
            // again from the same prev after a restart -- two signatures at one seq, the fork this exists to prevent.
            let bytes = bincode::serialize(&rec).expect("a record encodes");
            if host.set_secret(&record_name(&rec_label), &bytes) {
                Answer::Signed(signed)
            } else {
                Answer::Refused(Why::RecordNotSaved)
            }
        }
    }
}

#[cfg(feature = "freenet-main-delegate")]
mod delegate;
