//! The writer's and the reader's side of a Tail (ARCHITECTURE §1, Tail).
//!
//! The contract ([`craftec_tail_contract`]) decides what a host accepts. This crate is what a CLIENT does with it,
//! built from the same pieces so the two cannot disagree:
//!
//! - **Write:** [`Writer::prepare`] applies operations to the current body with the contract's own
//!   [`Body::apply`], and names the message to sign. The signer signs it; [`Writer::commit`] checks the signature
//!   covers that body and returns the delta to send. A delta is the ONLY way a tail moves.
//! - **Flush:** [`flush_into`] writes the pending rows into the prolly tree through the tree library's own `apply`,
//!   and returns the `Flush` operation that names the new root. It goes out as the next delta, AFTER the tree's
//!   blocks are put: a tail must never name a root whose blocks are not there.
//! - **Read:** [`get`] looks in the tail first, then in the tree at the tail's root.
//!
//! Pure: no I/O, no clock, no key. Putting blocks, sending deltas and signing belong to the caller.

pub use craftec_tail_contract::body::{MAX_BODY, MAX_KEY, MAX_VALUE};
pub use craftec_tail_contract::{Body, Entry, Op, Tail};

use craftec_register_contract::wire::{Params, Signed};
use freenet_prolly::apply::{apply_into, Applied, ApplyError, Edit};
use freenet_prolly::chunk::empty_leaf;
use freenet_prolly::range::read_value;
use freenet_prolly::store::{Blocks, BlocksMut, ReadError};

/// A step built and not yet signed: sign [`Unsigned::message`], then [`Writer::commit`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsigned {
    pub seq: u64,
    pub ops: Vec<Op>,
    pub body: Body,
    pub message: Vec<u8>,
}

/// One writer's view of its own tail: the last state it knows the network accepted.
#[derive(Clone, Debug)]
pub struct Writer {
    params: Params,
    current: Option<Tail>,
}

impl Writer {
    /// A writer for a tail nobody has written yet.
    pub fn new(params: &[u8]) -> Option<Writer> {
        Some(Writer {
            params: Params::parse(params)?,
            current: None,
        })
    }

    /// A writer resuming from a state read back from the network. The state is verified in full, as a host would.
    pub fn resume(params: &[u8], state: &[u8]) -> Option<Writer> {
        let (params, current) = craftec_tail_contract::read(params, state)?;
        Some(Writer { params, current })
    }

    pub fn current(&self) -> Option<&Tail> {
        self.current.as_ref()
    }

    pub fn seq(&self) -> u64 {
        self.current.as_ref().map_or(0, |t| t.seq())
    }

    pub fn body(&self) -> Body {
        self.current
            .as_ref()
            .map(|t| t.body.clone())
            .unwrap_or_default()
    }

    /// The whole state, as a host holds it.
    pub fn state(&self) -> Vec<u8> {
        self.current
            .as_ref()
            .map(|t| t.encode(&self.params.authority))
            .unwrap_or_default()
    }

    /// Apply `ops` as the next delta. `None` if the contract would refuse it.
    pub fn prepare(&self, ops: Vec<Op>) -> Option<Unsigned> {
        let seq = self.seq() + 1;
        let body = self.body().apply(&ops, seq)?;
        let message = craftec_tail_contract::message(&self.params, seq, &body.hash());
        Some(Unsigned {
            seq,
            ops,
            body,
            message,
        })
    }

    /// Take the signer's answer for `u`. `None` — and nothing moves — unless it is a valid signature of exactly that
    /// step on top of what this writer holds. Returns the delta to send.
    pub fn commit(&mut self, u: Unsigned, signed: Signed) -> Option<Vec<u8>> {
        if u.seq != self.seq() + 1
            || signed.seq != u.seq
            || signed.terminal
            || signed.value_hash != u.body.hash()
            || !signed.verify(&self.params)
        {
            return None;
        }
        let delta = craftec_tail_contract::encode_delta(&self.params.authority, &signed, &u.ops);
        self.current = Some(Tail {
            signed,
            body: u.body,
        });
        Some(delta)
    }
}

/// Write every pending row of `body` into the tree at `body.root` (an empty tree before the first flush), writing
/// the new blocks into `blocks`. Returns what the tree library applied — the new root, and the parity blocks the
/// caller owes — and the `Flush` operation covering every delta up to `through`.
///
/// `through` is the writer's current `seq`: the flush goes out as the NEXT delta, which the contract requires to be
/// above it. Rows written after `through` are not in the tree and stay in the tail.
pub fn flush_into<B: BlocksMut>(
    blocks: &mut B,
    body: &Body,
    through: u64,
) -> Result<(Applied, Op), ApplyError> {
    let root = match body.root {
        Some(r) => r,
        None => {
            let e = empty_leaf();
            blocks.insert_block(e.cid, &e.bytes);
            e.cid
        }
    };
    // A BTreeMap iterates in key order: exactly the sorted batch the tree library requires.
    let edits: Vec<(Vec<u8>, Edit)> = body
        .entries
        .iter()
        .filter(|(_, e)| e.seq <= through)
        .map(|(k, e)| {
            let edit = match &e.value {
                Some(v) => Edit::Put(v.clone()),
                None => Edit::Delete,
            };
            (k.clone(), edit)
        })
        .collect();
    let applied = apply_into(blocks, &root, &edits)?;
    let op = Op::Flush {
        root: applied.root,
        through,
    };
    Ok((applied, op))
}

/// The current value of `key`: the tail's pending row if it has one (a delete included), else the tree's.
/// `Err(ReadError::Need(ids))` names the tree blocks to fetch before asking again.
pub fn get<B: Blocks>(blocks: &B, body: &Body, key: &[u8]) -> Result<Option<Vec<u8>>, ReadError> {
    match body.get(key) {
        Some(v) => Ok(v.map(<[u8]>::to_vec)),
        None => match &body.root {
            None => Ok(None),
            Some(root) => match freenet_prolly::read::get(blocks, root, key)? {
                None => Ok(None),
                Some(v) => Ok(Some(read_value(blocks, v)?.to_vec())),
            },
        },
    }
}

#[cfg(test)]
mod tests;
