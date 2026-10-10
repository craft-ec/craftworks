//! THE WALLET APP's core (TOKEN §7c): see a token's balance, SEND, and RECEIVE (tokens are made by their issuers' tools,
//! not here: a page that made them would let anyone flood the network with tokens) — pure, over `got` (a
//! contract's state as read; `Some(empty)`: read and NOT THERE), in the steps every token action takes (`tokens::Step`:
//! read these, have the delegate sign this, done). The wallet core (`craftworks_wallet`) builds every entry; nothing
//! here decides a token rule.

use crate::tokens::{begin, device_of, holding, on_our_clock, Ctx, Device, Got, Put, Step};
use craftworks_token_contract as t;
use craftworks_wallet::{Sources, Wallet};
use t::{Body, Definition, State};

/// A token as this account sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub definition: Definition,
    /// What this account holds (its chain's balance; 0 when it has none — or, as the creator, its genesis not begun).
    pub balance: u128,
    /// Its chain is there (a creator's not begun: false, its genesis still to come with its first send).
    pub begun: bool,
}

/// What a send built: the sender's chain to put, and the debit's name (what the receiver's credit cites).
#[derive(Debug, Clone, PartialEq)]
pub struct Sent {
    pub put: Put,
    pub nonce: [u8; 16],
}

/// What a receive built: the account's chain to put (`None`: nothing to take now), what it took, and how many sends to
/// it are PENDING (inside their margin: takeable once the beacon passes it — ask again then).
#[derive(Debug, Clone, PartialEq)]
pub struct Received {
    pub put: Option<Put>,
    pub amount: u128,
    pub pending: u32,
}

fn token(id: &[u8; 32]) -> t::Tok {
    t::Tok { definition: *id, item: [0; 32] }
}

/// The token `id` as `owner` holds it.
pub fn view(ctx: &Ctx, got: Got, id: &[u8; 32], owner: &[u8; 32]) -> Result<Step<View>, String> {
    Ok(match holding(ctx, got, &token(id), owner)? {
        Step::Want(w) => Step::Want(w),
        Step::Sign(m) => Step::Sign(m),
        Step::Done(Some((h, _))) => Step::Done(View { begun: !h.state.entries.is_empty(), balance: h.state.balance(), definition: h.definition }),
        Step::Done(None) => {
            let Some(b) = got(id) else { return Ok(Step::Want(vec![*id])) };
            Step::Done(View { definition: Definition::decode(&b).ok_or("not a token definition")?, balance: 0, begun: false })
        }
    })
}

/// SEND `amount` of the token `id` from `from` to `to`, stamped `now` (the beacon's height): the creator's genesis
/// first when its chain is not begun, then the debit — signed by this node's device.
#[allow(clippy::too_many_arguments)]
pub fn send(ctx: &Ctx, got: Got, id: &[u8; 32], from: &[u8; 32], to: &[u8; 32], amount: u128, now: u64, dev: &Device) -> Result<Step<Sent>, String> {
    if from == to {
        return Err("a send to yourself".into());
    }
    let (mut held, keys) = match holding(ctx, got, &token(id), from)? {
        Step::Done(Some(h)) => h,
        Step::Done(None) => return Err("you hold none of this token".into()),
        Step::Want(w) => return Ok(Step::Want(w)),
        Step::Sign(m) => return Ok(Step::Sign(m)),
    };
    on_our_clock(ctx, &held.definition)?;
    let asking = device_of(dev);
    let w = Wallet { device: &asking, keys: &keys };
    if let Some(m) = begin(&w, &asking, &mut held, now)? {
        return Ok(Step::Sign(m));
    }
    let built = w.debit(&held, Body::Send { to: *to, amount }, now);
    if let Some(m) = asking.next() {
        return Ok(Step::Sign(m));
    }
    let entries = built.map_err(String::from)?;
    let nonce = entries.last().expect("a debit").nonce;
    let mut s = held.state;
    s.entries.extend(entries);
    Ok(Step::Done(Sent { put: Put { params: held.params.encode(), state: s.encode() }, nonce }))
}

/// RECEIVE what `senders` sent `owner` of the token `id` (their chains read; one not there is skipped), each once its
/// margin has passed (`now`: the beacon's height) — worked credits, no signature; the creator's genesis first when its
/// chain is not begun.
pub fn receive(ctx: &Ctx, got: Got, id: &[u8; 32], owner: &[u8; 32], senders: &[[u8; 32]], now: u64, dev: &Device) -> Result<Step<Received>, String> {
    let (mut held, keys) = match holding(ctx, got, &token(id), owner)? {
        Step::Done(Some(h)) => h,
        Step::Want(w) => return Ok(Step::Want(w)),
        Step::Sign(m) => return Ok(Step::Sign(m)),
        // No chain yet: the first credit begins it.
        Step::Done(None) => {
            let d = Definition::decode(&got(id).ok_or("the token was not read")?).ok_or("not a token definition")?;
            let p = crate::tokens::chain(ctx, &d, id, owner, [0; 32]);
            let Some(k) = got(&p.keys_id()) else { return Ok(Step::Want(vec![p.keys_id()])) };
            let keys = craftworks_idlog_contract::read(owner, &k).ok_or("the account's key log does not verify")?;
            (craftworks_wallet::Held { params: p, definition: d, state: State::default(), blocks: Default::default() }, keys)
        }
    };
    on_our_clock(ctx, &held.definition)?;
    let mut want = Vec::new();
    let mut chains = Vec::new();
    for s in senders.iter().filter(|s| *s != owner) {
        let cid = held.params.at(held.params.tok(), *s).id();
        match got(&cid) {
            None => want.push(cid),
            Some(b) if b.is_empty() => {}
            Some(b) => chains.push((*s, State::decode(&b).ok_or("a sender's chain does not read")?)),
        }
    }
    if !want.is_empty() {
        return Ok(Step::Want(want));
    }
    let margin = held.definition.margin;
    let pending = chains.iter().flat_map(|(_, c)| &c.entries).filter(|e| matches!(e.body, Body::Send { to, .. } if to == *owner) && e.stamp.saturating_add(margin) > now).count() as u32;
    let before = held.state.entries.len();
    let asking = device_of(dev);
    let w = Wallet { device: &asking, keys: &keys };
    if let Some(m) = begin(&w, &asking, &mut held, now)? {
        return Ok(Step::Sign(m));
    }
    let entries = w.collect(&held, &Sources { chains: &chains, blocks: &Default::default(), programs: &[], shards: &[] }, now);
    if let Some(m) = asking.next() {
        return Ok(Step::Sign(m));
    }
    let amount: u128 = entries.as_ref().map_err(|w| w.to_string())?.iter().map(|e| if let Body::Receive { amount, .. } = e.body { amount } else { 0 }).sum();
    let mut s = held.state;
    s.entries.extend(entries.map_err(String::from)?);
    // Only a credit is put — a creator's genesis alone waits for its first send.
    Ok(Step::Done(Received { put: (amount > 0 && s.entries.len() > before).then(|| Put { params: held.params.encode(), state: s.encode() }), amount, pending }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::{chain, device};
    use ed25519_dalek::{Signer as _, SigningKey};
    use std::collections::HashMap;

    const IDLOG: [u8; 32] = [6; 32];

    fn log(owner_seed: u8, next: u8) -> craftworks_idlog_contract::Log {
        let next = SigningKey::from_bytes(&[next; 32]).verifying_key().to_bytes();
        let e = craftworks_idlog_contract::Event::signed(0, [0; 32], &[owner_seed; 32], craftworks_idlog_contract::commit(&next), [1; 32], [1; 32], vec![]).unwrap();
        craftworks_idlog_contract::Log { events: vec![e] }
    }

    /// The page's loop: step, and on `Sign` have the "delegate" (the node key `node`) sign and step again.
    fn run<T>(node: u8, dev: &mut Device, step: &mut dyn FnMut(&Device) -> Result<Step<T>, String>) -> Result<T, String> {
        for _ in 0..8 {
            match step(dev)? {
                Step::Done(x) => return Ok(x),
                Step::Sign(m) => {
                    assert!(m.starts_with(t::ENTRY_DOMAIN));
                    dev.sigs.push((m.clone(), SigningKey::from_bytes(&[node; 32]).sign(&m).to_bytes()));
                }
                Step::Want(w) => panic!("wants {:?}", w.iter().map(|x| x[0]).collect::<Vec<_>>()),
            }
        }
        panic!("no end")
    }

    #[test]
    fn make_a_token_send_it_and_receive_it() {
        let ctx = Ctx { code_hash: [9; 32], beacon: [7; 32], idlog: IDLOG };
        let (la, lb) = (log(101, 151), log(102, 152));
        let (a, b) = (la.events[0].id(), lb.events[0].id());
        let (na, nb) = (SigningKey::from_bytes(&[12; 32]).verifying_key().to_bytes(), SigningKey::from_bytes(&[13; 32]).verifying_key().to_bytes());
        let mut da = device(na, &[craftworks_identity::credential(&a, &na, &[7; 32], &[101; 32])], &la, vec![]).unwrap();
        let mut db = device(nb, &[craftworks_identity::credential(&b, &nb, &[7; 32], &[102; 32])], &lb, vec![]).unwrap();
        // A token of A's: its whole supply A's, no mint authority.
        let d = Definition { code_hash: ctx.code_hash, decimals: 2, min_send: 1, genesis_owner: a, genesis_amount: 100_000, name: b"W".to_vec(), nonce: [1; 16], beacon: ctx.beacon, margin: 2, idlog: IDLOG, mint: None, freeze: None, ids: t::Ids::None, royalty: None, cap: 0, item: [0; 32], version: [0; 32], soulbound: false, transfer_fee: None, sale: None, keepers: None };
        let id = t::id(&ctx.code_hash, &d.encode());
        let (pa, pb) = (chain(&ctx, &d, &id, &a, [0; 32]), chain(&ctx, &d, &id, &b, [0; 32]));
        let mut got: HashMap<[u8; 32], Vec<u8>> = HashMap::new();
        got.insert(id, d.encode());
        // Nothing read yet: the chain and key log are wanted.
        let g = got.clone();
        assert_eq!(view(&ctx, &|x| g.get(x).cloned(), &id, &a), Ok(Step::Want(vec![pa.id(), pa.keys_id()])));
        for (k, v) in [(pa.id(), vec![]), (pa.keys_id(), la.encode()), (pb.id(), vec![]), (pb.keys_id(), lb.encode())] {
            got.insert(k, v);
        }
        let g = got.clone();
        // The creator, not begun: its whole supply to come; anyone else none.
        assert_eq!(view(&ctx, &|x| g.get(x).cloned(), &id, &a).map(|s| match s { Step::Done(v) => (v.balance, v.begun), _ => panic!() }), Ok((0, false)));
        assert_eq!(view(&ctx, &|x| g.get(x).cloned(), &id, &b).map(|s| match s { Step::Done(v) => (v.balance, v.begun), _ => panic!() }), Ok((0, false)));
        assert_eq!(send(&ctx, &|x| g.get(x).cloned(), &id, &b, &a, 1, 5, &db), Err("you hold none of this token".into()));
        assert_eq!(send(&ctx, &|x| g.get(x).cloned(), &id, &a, &a, 1, 5, &da), Err("a send to yourself".into()));
        // SEND 250 from the creator at height 5: its genesis first, then the debit — a chain the token takes.
        let sent = run(12, &mut da, &mut |dv| send(&ctx, &|x| g.get(x).cloned(), &id, &a, &b, 250, 5, dv)).unwrap();
        let ca = State::decode(&sent.put.state).unwrap();
        assert_eq!((ca.balance(), ca.entries.last().unwrap().nonce), (100_000 - 250, sent.nonce));
        assert_eq!(t::check(&pa, &d, &ca, &|_, _| None, &|_| None, Some(&la), &t::Extra::NONE), Ok(()));
        // Over the balance: refused.
        got.insert(pa.id(), sent.put.state.clone());
        let g = got.clone();
        assert_eq!(run(12, &mut da, &mut |dv| send(&ctx, &|x| g.get(x).cloned(), &id, &a, &b, 1_000_000, 6, dv)), Err("a debit over the balance".into()));
        // RECEIVE before the margin passes: nothing yet; after: the credit, a chain begun by it.
        assert_eq!(run(13, &mut db, &mut |dv| receive(&ctx, &|x| g.get(x).cloned(), &id, &b, &[a], 6, dv)), Ok(Received { put: None, amount: 0, pending: 1 }));
        let r = run(13, &mut db, &mut |dv| receive(&ctx, &|x| g.get(x).cloned(), &id, &b, &[a], 7, dv)).unwrap();
        let cb = State::decode(&r.put.as_ref().unwrap().state).unwrap();
        assert_eq!((r.amount, cb.balance(), r.pending), (250, 250, 0));
        assert_eq!(t::check(&pb, &d, &cb, &|_, o| (o == &a).then(|| ca.clone()), &|_| None, Some(&lb), &t::Extra::NONE), Ok(()));
        // Received once: again, nothing to take.
        got.insert(pb.id(), r.put.unwrap().state);
        let g = got.clone();
        assert_eq!(run(13, &mut db, &mut |dv| receive(&ctx, &|x| g.get(x).cloned(), &id, &b, &[a], 9, dv)), Ok(Received { put: None, amount: 0, pending: 0 }));
        assert_eq!(view(&ctx, &|x| g.get(x).cloned(), &id, &b).map(|s| match s { Step::Done(v) => (v.balance, v.begun), _ => panic!() }), Ok((250, true)));
        // A token on ANOTHER beacon: read (its chains are its own beacon's), never written here.
        let far = Definition { beacon: [8; 32], ..d.clone() };
        let fid = t::id(&ctx.code_hash, &far.encode());
        let mut g2 = got.clone();
        g2.insert(fid, far.encode());
        g2.insert(chain(&ctx, &far, &fid, &a, [0; 32]).id(), vec![]);
        g2.insert(chain(&ctx, &far, &fid, &a, [0; 32]).keys_id(), la.encode());
        assert_eq!(chain(&ctx, &far, &fid, &a, [0; 32]).beacon, [8; 32]);
        assert_eq!(send(&ctx, &|x| g2.get(x).cloned(), &fid, &a, &b, 1, 9, &da), Err(format!("this token runs on another clock (beacon {}), not this site's", crate::tokens::short(&[8; 32]))));
        // B sends some back; the creator's chain (begun) receives it.
        let back = run(13, &mut db, &mut |dv| send(&ctx, &|x| g.get(x).cloned(), &id, &b, &a, 50, 9, dv)).unwrap();
        got.insert(pb.id(), back.put.state.clone());
        let g = got.clone();
        let r = run(12, &mut da, &mut |dv| receive(&ctx, &|x| g.get(x).cloned(), &id, &a, &[b], 11, dv)).unwrap();
        assert_eq!((r.amount, State::decode(&r.put.unwrap().state).unwrap().balance()), (50, 100_000 - 250 + 50));
    }
}
