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

/// What a receive built: the account's chain to put (`None`: nothing to take now), what it took, how many sends to it
/// are PENDING (inside their margin: takeable once the beacon passes it — ask again then), and how many are UNWITNESSED
/// (past it, but their debit not yet ACCEPTED by its committee: not value yet — ask again later).
#[derive(Debug, Clone, PartialEq)]
pub struct Received {
    pub put: Option<Put>,
    pub amount: u128,
    pub pending: u32,
    pub unwitnessed: u32,
    /// Why not accepted yet (its acceptance's answer), when unwitnessed.
    pub why: Option<String>,
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

/// STEALTH HOLDINGS (TOKEN §7f): a token's chains this wallet found (each a one-time owner) and their balances.
#[derive(Debug, Clone, PartialEq)]
pub struct Private {
    pub definition: Definition,
    /// Each found chain's owner and balance (0: announced, not yet received).
    pub chains: Vec<([u8; 32], u128)>,
}

impl Private {
    pub fn balance(&self) -> u128 {
        self.chains.iter().map(|c| c.1).sum()
    }
}

/// What a private send built: the chains to put and each debit's announcement (for the payments bags).
#[derive(Debug, Clone, PartialEq)]
pub struct SentPrivately {
    pub puts: Vec<Put>,
    pub announces: Vec<stealth::Announce>,
}

use craftworks_wallet::stealth;

fn definition_of(got: Got, id: &[u8; 32]) -> Result<Result<Definition, [u8; 32]>, String> {
    match got(id) {
        None => Ok(Err(*id)),
        Some(b) if b.is_empty() => Err("the token is not there".into()),
        Some(b) => Ok(Ok(Definition::decode(&b).ok_or("not a token definition")?)),
    }
}

/// The chains `owners` (one-time owners this wallet found) of the token `id`, as read: (params, state — empty when
/// not there yet).
fn stealth_chains(ctx: &Ctx, got: Got, d: &Definition, id: &[u8; 32], owners: &[[u8; 32]]) -> Result<Result<Vec<(t::ChainParams, State)>, Vec<[u8; 32]>>, String> {
    let mut want = Vec::new();
    let mut out = Vec::new();
    for o in owners {
        let p = crate::tokens::chain(ctx, d, id, o, [0; 32]);
        match got(&p.id()) {
            None => want.push(p.id()),
            Some(b) if b.is_empty() => out.push((p, State::default())),
            Some(b) => out.push((p, State::decode(&b).ok_or("a chain that does not read")?)),
        }
    }
    Ok(if want.is_empty() { Ok(out) } else { Err(want) })
}

/// This wallet's PRIVATE holdings of the token `id`: the chains of the one-time owners it found.
pub fn private(ctx: &Ctx, got: Got, id: &[u8; 32], owners: &[[u8; 32]]) -> Result<Step<Private>, String> {
    let d = match definition_of(got, id)? {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(d) => d,
    };
    Ok(match stealth_chains(ctx, got, &d, id, owners)? {
        Err(w) => Step::Want(w),
        Ok(cs) => Step::Done(Private { chains: cs.iter().map(|(p, s)| (p.owner, s.balance())).collect(), definition: d }),
    })
}

/// SEND `amount` of the token `id` PRIVATELY to the wallet whose card keys are `to`, from this wallet's stealth chains
/// (`mine`: each found chain's announcement — its R and owner), stamped `now`: from as FEW chains as can pay it (the
/// largest first), each debit to a fresh one-time owner of the recipient's (`rand`: fresh randomness, 32 bytes a
/// debit) and announced. Signed by the one-time keys (`seed`: this wallet's) — no delegate, no DID.
#[allow(clippy::too_many_arguments)]
pub fn send_private(ctx: &Ctx, got: Got, id: &[u8; 32], seed: &[u8; 32], mine: &[([u8; 32], [u8; 32])], to: &stealth::Public, amount: u128, rand: &[u8; 32], now: u64) -> Result<Step<SentPrivately>, String> {
    let d = match definition_of(got, id)? {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(d) => d,
    };
    on_our_clock(ctx, &d)?;
    let owners: Vec<[u8; 32]> = mine.iter().map(|m| m.1).collect();
    let mut chains = match stealth_chains(ctx, got, &d, id, &owners)? {
        Err(w) => return Ok(Step::Want(w)),
        Ok(cs) => cs,
    };
    chains.sort_by_key(|(_, s)| std::cmp::Reverse(s.balance()));
    let keys = stealth::Keys::from_seed(seed);
    let none = craftworks_idlog_contract::Log { events: vec![] };
    let mut left = amount;
    let mut out = SentPrivately { puts: vec![], announces: vec![] };
    for (i, (p, st)) in chains.into_iter().enumerate() {
        if left == 0 {
            break;
        }
        let have = st.balance();
        // What this chain sends: all it can (its transfer fee on top), at most what is left.
        let fee = |x: u128| x + t::transfer_fee(&d, &p.owner, x);
        let mut part = left.min(have);
        while part > 0 && fee(part) > have {
            part -= 1;
        }
        if part == 0 {
            continue;
        }
        let r = mine.iter().find(|m| m.1 == p.owner).map(|m| m.0).ok_or("a chain this wallet did not find")?;
        let one = keys.find(&r, &p.owner).ok_or("a chain that is not this wallet's")?;
        let paid = stealth::pay(to, blake3::Hasher::new_derive_key("craftworks wallet private send").update(rand).update(&(i as u32).to_le_bytes()).finalize().as_bytes()).ok_or("the recipient's card keys are not keys")?;
        let held = craftworks_wallet::Held { params: p, definition: d.clone(), state: st, blocks: Default::default() };
        let entries = Wallet { device: &one, keys: &none }.debit(&held, Body::Send { to: paid.to, amount: part }, now).map_err(String::from)?;
        let nonce = entries.last().expect("a debit").nonce;
        let mut s = held.state;
        s.entries.extend(entries);
        out.puts.push(Put { params: held.params.encode(), state: s.encode() });
        out.announces.push(stealth::Announce { token: *id, from: held.params.owner, nonce, to: paid.to, r: paid.r });
        left -= part;
    }
    if left > 0 {
        return Err(format!("not enough: {} short", left));
    }
    Ok(Step::Done(out))
}

/// RECEIVE a payment this wallet found (`a`, its announcement) into its one-time chain: the credit from the paying
/// chain once its margin has passed — worked, no signature.
/// Taken only once the paying DEBIT is ACCEPTED (TOKEN §7a: `tokens::accepted` over `lot`) — a chain proves shape, its
/// witnesses that the money is real.
pub fn receive_private(ctx: &Ctx, lot: &crate::tokens::Lot, got: Got, a: &stealth::Announce, now: u64) -> Result<Step<Received>, String> {
    let d = match definition_of(got, &a.token)? {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(d) => d,
    };
    on_our_clock(ctx, &d)?;
    let (mine, from) = match stealth_chains(ctx, got, &d, &a.token, &[a.to, a.from])? {
        Err(w) => return Ok(Step::Want(w)),
        Ok(mut cs) => (cs.remove(0), cs.remove(0)),
    };
    let pending = from.1.entries.iter().filter(|e| e.nonce == a.nonce && e.stamp.saturating_add(d.margin) > now).count() as u32;
    // Final: its debit ACCEPTED, or nothing is taken yet.
    if pending == 0 {
        if let Some(debit) = from.1.entries.iter().find(|e| e.nonce == a.nonce) {
            match crate::tokens::accepted(lot, got, &from.0, &d, &from.1, debit.seq)? {
                Step::Want(w) => return Ok(Step::Want(w)),
                Step::Sign(m) => return Ok(Step::Sign(m)),
                Step::Done(Err(why)) => return Ok(Step::Done(Received { put: None, amount: 0, pending: 0, unwitnessed: 1, why: Some(why) })),
                Step::Done(Ok(_)) => {}
            }
        }
    }
    let none = craftworks_idlog_contract::Log { events: vec![] };
    // Credits are worked: the collecting needs no key — any device stands in.
    let anyone = craftworks_wallet::Asking::new([0; 32], vec![], 0, vec![]);
    let held = craftworks_wallet::Held { params: mine.0, definition: d, state: mine.1, blocks: Default::default() };
    let chains = [(a.from, from.1)];
    let entries = Wallet { device: &anyone, keys: &none }.collect(&held, &Sources { chains: &chains, blocks: &Default::default(), programs: &[], shards: &[] }, now).map_err(String::from)?;
    let amount: u128 = entries.iter().map(|e| if let Body::Receive { amount, .. } = e.body { amount } else { 0 }).sum();
    let mut s = held.state;
    s.entries.extend(entries);
    Ok(Step::Done(Received { put: (amount > 0).then(|| Put { params: held.params.encode(), state: s.encode() }), amount, pending, unwitnessed: 0, why: None }))
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
    fn send_from_an_id_receive_privately_once_witnessed() {
        let ctx = Ctx { code_hash: [9; 32], beacon: [7; 32], idlog: IDLOG };
        let (la, lb) = (log(101, 151), log(102, 152));
        let (a, b) = (la.events[0].id(), lb.events[0].id());
        let (na, nb) = (SigningKey::from_bytes(&[12; 32]).verifying_key().to_bytes(), SigningKey::from_bytes(&[13; 32]).verifying_key().to_bytes());
        let mut da = device(na, &[craftworks_identity::credential(&a, &na, &[7; 32], &[101; 32])], &la, vec![]).unwrap();
        let db = device(nb, &[craftworks_identity::credential(&b, &nb, &[7; 32], &[102; 32])], &lb, vec![]).unwrap();
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
        // A token on ANOTHER beacon: read (its chains are its own beacon's), never written here.
        let far = Definition { beacon: [8; 32], ..d.clone() };
        let fid = t::id(&ctx.code_hash, &far.encode());
        let mut g2 = got.clone();
        g2.insert(fid, far.encode());
        g2.insert(chain(&ctx, &far, &fid, &a, [0; 32]).id(), vec![]);
        g2.insert(chain(&ctx, &far, &fid, &a, [0; 32]).keys_id(), la.encode());
        assert_eq!(chain(&ctx, &far, &fid, &a, [0; 32]).beacon, [8; 32]);
        assert_eq!(send(&ctx, &|x| g2.get(x).cloned(), &fid, &a, &b, 1, 9, &da), Err(format!("this token runs on another clock (beacon {}), not this site's", crate::tokens::short(&[8; 32]))));
        // PRIVATELY, in epoch 1 (where the test's stakers stand): A (its DID chain) pays B's card — a fresh one-time owner,
        // announced; B finds it by scanning and receives it — inside the margin pending, past it UNWITNESSED until its
        // committee signs the debit, then taken — and pays A's card back from that stealth chain; A takes it the same way.
        let tl = crate::tokens::testlot::stakers(&ctx, &mut got);
        let e1 = crate::tokens::testlot::E1;
        let (kb, ka) = (stealth::Keys::from_seed(&[22; 32]), stealth::Keys::from_seed(&[21; 32]));
        let paid = stealth::pay(&kb.public(), &[1; 32]).unwrap();
        let g = got.clone();
        let sent = run(12, &mut da, &mut |dv| send(&ctx, &|x| g.get(x).cloned(), &id, &a, &paid.to, 100, e1 + 12, dv)).unwrap();
        got.insert(pa.id(), sent.put.state.clone());
        let ann = stealth::Announce { token: id, from: a, nonce: sent.nonce, to: paid.to, r: paid.r };
        let found = kb.scan([&ann]);
        assert_eq!(found.len(), 1);
        assert!(ka.scan([&ann]).is_empty());
        let pb2 = chain(&ctx, &d, &id, &paid.to, [0; 32]);
        got.insert(pb2.id(), vec![]);
        let take = |a: &stealth::Announce, now: u64, got: &HashMap<[u8; 32], Vec<u8>>| crate::tokens::testlot::done(&|g| receive_private(&ctx, &tl.lot, g, a, now), got);
        assert_eq!(take(&ann, e1 + 13, &got), Ok(Received { put: None, amount: 0, pending: 1, unwitnessed: 0, why: None }));
        let unwitnessed = Received { put: None, amount: 0, pending: 0, unwitnessed: 1, why: Some("an entry its committee has not witnessed".into()) };
        assert_eq!(take(&ann, e1 + 14, &got), Ok(unwitnessed.clone()), "not value before its witnesses");
        let debit = State::decode(&got[&pa.id()]).unwrap();
        crate::tokens::testlot::witness(&tl, &mut got, &pa, &d, &debit, debit.entries.last().unwrap().seq, 3);
        assert_eq!(take(&ann, e1 + 14, &got), Ok(unwitnessed), "three of six: short");
        crate::tokens::testlot::witness(&tl, &mut got, &pa, &d, &debit, debit.entries.last().unwrap().seq, 4);
        let r = take(&ann, e1 + 14, &got).unwrap();
        assert_eq!((r.amount, r.pending, r.unwitnessed), (100, 0, 0));
        got.insert(pb2.id(), r.put.unwrap().state);
        let g = got.clone();
        let mine = [(paid.r, paid.to)];
        let Ok(Step::Done(v)) = private(&ctx, &|x| g.get(x).cloned(), &id, &[paid.to]) else { panic!("private") };
        assert_eq!(v.balance(), 100);
        assert_eq!(send_private(&ctx, &|x| g.get(x).cloned(), &id, &[22; 32], &mine, &ka.public(), 101, &[2; 32], e1 + 15), Err("not enough: 1 short".into()));
        assert_eq!(send_private(&ctx, &|x| g.get(x).cloned(), &id, &[21; 32], &mine, &ka.public(), 10, &[2; 32], e1 + 15), Err("a chain that is not this wallet's".into()));
        let Ok(Step::Done(back)) = send_private(&ctx, &|x| g.get(x).cloned(), &id, &[22; 32], &mine, &ka.public(), 30, &[2; 32], e1 + 15) else { panic!("sent") };
        assert_eq!((back.puts.len(), back.announces.len()), (1, 1));
        let sb2 = State::decode(&back.puts[0].state).unwrap();
        let alice_chain = State::decode(&got[&pa.id()]).unwrap();
        assert_eq!(t::check(&pb2, &d, &sb2, &|_, o| (o == &a).then(|| alice_chain.clone()), &|_| None, None, &t::Extra::NONE), Ok(()));
        assert_eq!(sb2.balance(), 70);
        let back_ann = back.announces[0];
        assert_eq!(back_ann.from, paid.to);
        let found = ka.scan([&back_ann]);
        assert_eq!(found.len(), 1);
        got.insert(pb2.id(), back.puts[0].state.clone());
        got.insert(chain(&ctx, &d, &id, &back_ann.to, [0; 32]).id(), vec![]);
        crate::tokens::testlot::witness(&tl, &mut got, &pb2, &d, &sb2, sb2.entries.last().unwrap().seq, 6);
        assert_eq!(take(&back_ann, e1 + 18, &got).map(|r| r.amount), Ok(30));
        // Neither stealth chain names a DID: their owners are one-time keys' sets.
        assert!(![pb2.owner, back_ann.to].contains(&a) && ![pb2.owner, back_ann.to].contains(&b));
    }
}
