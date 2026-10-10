//! TOKENS in the page's core: what every token action shares — a step's answer (`Want` read these, `Sign` this, `Done`),
//! the context (token code, beacon, key log code), this node as the account's DEVICE, a chain's params, a put — so the
//! editions (`editions`) and the wallet (`wallet`) build on one set. Pure, over what the page has read.

use craftworks_token_contract as t;
use craftworks_wallet::{Asking, Held, Wallet};
use t::{ChainParams, Definition, State};

/// A contract's state as the page read it: `None` not read yet, `Some(empty)` read and not there.
pub type Got<'a> = &'a dyn Fn(&[u8; 32]) -> Option<Vec<u8>>;

/// One step of an edition action.
#[derive(Debug, PartialEq)]
pub enum Step<T> {
    /// Read these contracts, then ask again.
    Want(Vec<[u8; 32]>),
    /// Have the identity delegate sign this (`TokenSign`), then ask again with its signature.
    Sign(Vec<u8>),
    Done(T),
}

/// What it works with: the token's code (its hash names every token contract), the beacon (the clock every
/// chain is stamped by) and the key log's code hash (a chain's owner's keys).
pub struct Ctx {
    pub code_hash: [u8; 32],
    pub beacon: [u8; 32],
    pub idlog: [u8; 32],
}

/// The account's DEVICE as the page knows it: its node key, the credential its owner key gave it (from the account's
/// devices), the key seq of that owner key, and the signatures the delegate gave so far.
pub struct Device {
    pub public: [u8; 32],
    pub credential: Vec<u8>,
    pub key_seq: u32,
    pub sigs: Vec<(Vec<u8>, [u8; 64])>,
}

/// A PUT: a token contract (by its params) and its state.
#[derive(Debug, Clone, PartialEq)]
pub struct Put {
    pub params: Vec<u8>,
    pub state: Vec<u8>,
}

pub(crate) fn short(b: &[u8; 32]) -> String {
    b[..6].iter().map(|x| format!("{x:02x}")).collect()
}

pub(crate) fn there(got: Got, id: &[u8; 32]) -> Result<Option<Vec<u8>>, [u8; 32]> {
    match got(id) {
        None => Err(*id),
        Some(b) if b.is_empty() => Ok(None),
        Some(b) => Ok(Some(b)),
    }
}

/// A chain's params: `owner`'s chain of `token` (`item`: a numbered copy's, else zero) — on the TOKEN's own beacon and
/// key log (its definition's), whatever this page's clock.
pub fn chain(ctx: &Ctx, d: &Definition, token: &[u8; 32], owner: &[u8; 32], item: [u8; 32]) -> ChainParams {
    ChainParams { code_hash: ctx.code_hash, definition: *token, owner: *owner, beacon: d.beacon, idlog: d.idlog, item }
}

/// A token this page can WRITE: one stamped by this page's clock (`ctx.beacon` — the heights it reads are that
/// beacon's; a token on another beacon is read here, never written).
pub(crate) fn on_our_clock(ctx: &Ctx, d: &Definition) -> Result<(), String> {
    if d.beacon != ctx.beacon {
        return Err(format!("this token runs on another clock (beacon {}), not this site's", short(&d.beacon)));
    }
    Ok(())
}

pub(crate) fn device_of(dev: &Device) -> Asking {
    Asking::new(dev.public, dev.credential.clone(), dev.key_seq, dev.sigs.clone())
}

/// This node's DEVICE from the account's credentials as the page holds them (its group's): the one for this node key,
/// its key seq the key log's event whose key signed it (what the token checks a writer by).
pub fn device(public: [u8; 32], creds: &[Vec<u8>], keys: &craftworks_idlog_contract::Log, sigs: Vec<(Vec<u8>, [u8; 64])>) -> Result<Device, &'static str> {
    for c in creds {
        let Some((_, owner, node, _)) = craftworks_identity::read_credential(c) else { continue };
        if node != public {
            continue;
        }
        if let Some(seq) = keys.events.iter().position(|e| e.key == owner) {
            return Ok(Device { public, credential: c.clone(), key_seq: seq as u32, sigs });
        }
    }
    Err("no credential of this node on the account's key log")
}

/// An account's CHAIN of a token as read (`tok`: the token and, for a numbered copy, its item), with the token's
/// definition and the account's key log. A chain not there is EMPTY when the account is the token's creator with a
/// genesis (it begins it: [`begin`]); anyone else's chain not there is `Ok(None)` — it holds none.
pub(crate) fn holding(ctx: &Ctx, got: Got, tok: &t::Tok, owner: &[u8; 32]) -> Result<Step<Option<(Held, craftworks_idlog_contract::Log)>>, String> {
    let d = match there(got, &tok.definition) {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(None) => return Err(format!("the token {} is not there", short(&tok.definition))),
        Ok(Some(b)) => Definition::decode(&b).ok_or("not a token definition")?,
    };
    let p = chain(ctx, &d, &tok.definition, owner, tok.item);
    let mut want = Vec::new();
    let mut read = |x: [u8; 32]| match there(got, &x) {
        Err(w) => {
            want.push(w);
            None
        }
        Ok(b) => Some(b),
    };
    let (mine, keys) = (read(p.id()), read(p.keys_id()));
    let (Some(mine), Some(keys)) = (mine, keys) else { return Ok(Step::Want(want)) };
    let state = match mine {
        Some(b) => State::decode(&b).ok_or("the account's chain does not read")?,
        None if d.genesis_owner == *owner && d.genesis_amount > 0 => State::default(),
        None => return Ok(Step::Done(None)),
    };
    let keys = craftworks_idlog_contract::read(owner, &keys.ok_or("the account's key log is not there")?).ok_or("the account's key log does not verify")?;
    Ok(Step::Done(Some((Held { params: p, definition: d, state, blocks: Default::default() }, keys))))
}

/// BEGIN `held` if it is a creator's chain not begun: its genesis appended. `Some(message)`: the delegate signs it first.
pub(crate) fn begin(w: &Wallet, asking: &Asking, held: &mut Held, now: u64) -> Result<Option<Vec<u8>>, String> {
    if !held.state.entries.is_empty() || held.definition.genesis_owner != held.params.owner || held.definition.genesis_amount == 0 {
        return Ok(None);
    }
    let g = w.genesis(held, now);
    if let Some(m) = asking.next() {
        return Ok(Some(m));
    }
    held.state.entries.extend(g.map_err(String::from)?);
    Ok(None)
}
