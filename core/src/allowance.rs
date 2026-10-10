//! THE WREN ALLOWANCE in the page's core (TOKEN §7d, decision 6): an epoch's PROGRAM (`craftworks_allowance_contract`)
//! — its VERDICT built from the treasury's escrows to it and the epoch statement's proofs, the treasury's RELEASE under
//! it, and an account's PAYOUT from it. Pure, over `got`; every entry the wallet core's (all worked: nothing signs).

use crate::tokens::{chain, there, Ctx, Got, Put, Step};
use craftworks_allowance_contract as A;
use craftworks_token_contract as t;
use craftworks_wallet::{Asking, Held, Sources, Wallet};
use t::{Body, Definition, State};

/// The epoch's PROGRAM: its params and its contract id (`code_hash`: the allowance contract's).
pub fn program(code_hash: &[u8; 32], token: &[u8; 32], epoch: u64) -> (A::Params, [u8; 32]) {
    let p = A::Params { token: t::Tok { definition: *token, item: [0; 32] }, epoch };
    let id = t::id(code_hash, &p.encode());
    (p, id)
}

/// What a verdict step built: the program's state to put (`None`: decided already, by someone), and the treasury's
/// chain with its release (`None`: nothing to release).
#[derive(Debug, Clone, PartialEq)]
pub struct Built {
    pub program: Option<(Vec<u8>, Vec<u8>)>,
    pub release: Option<Put>,
}

fn none() -> craftworks_idlog_contract::Log {
    craftworks_idlog_contract::Log { events: vec![] }
}

/// A chain as read with its owner's key log (an empty state when not there; an empty log when the owner has none).
pub(crate) fn read_chain(got: Got, p: &t::ChainParams, owner: &[u8; 32], want: &mut Vec<[u8; 32]>) -> Result<Option<(State, craftworks_idlog_contract::Log)>, String> {
    let (c, k) = (there(got, &p.id()), there(got, &p.keys_id()));
    let (Ok(c), Ok(k)) = (c, k) else {
        want.extend([p.id(), p.keys_id()].into_iter().filter(|x| got(x).is_none()));
        return Ok(None);
    };
    let state = c.map(|b| State::decode(&b).ok_or("a chain that does not read")).transpose()?.unwrap_or_default();
    let keys = match k {
        Some(b) => craftworks_idlog_contract::read(owner, &b).ok_or("a key log that does not verify")?,
        None => none(),
    };
    Ok(Some((state, keys)))
}

fn definition(got: Got, token: &[u8; 32]) -> Result<Result<Definition, [u8; 32]>, String> {
    Ok(match there(got, token) {
        Err(w) => Err(w),
        Ok(None) => return Err("the allowance's token is not there".into()),
        Ok(Some(b)) => Ok(Definition::decode(&b).ok_or("not a token definition")?),
    })
}

/// The VERDICT of `epoch`'s program: the treasury's escrows to it (its chain's `Escrow` entries naming it), each
/// account's allowance from the statement `root` (`lines`: DID, its proof), the rest refunded; then the treasury's
/// RELEASE under it. `None` when nothing is escrowed to it. A program decided already is not built again (its release
/// still is).
#[allow(clippy::too_many_arguments)]
/// Built only under a statement root its witnesses ACCEPTED (`tokens::statement_accepted`: anyone may post a root).
#[allow(clippy::too_many_arguments)]
pub fn verdict(ctx: &Ctx, lot: &crate::tokens::Lot, got: Got, code_hash: &[u8; 32], token: &[u8; 32], epoch: u64, root: &[u8; 32], lines: &[([u8; 32], Vec<u8>)], now: u64) -> Result<Step<Option<Built>>, String> {
    let d = match definition(got, token)? {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(d) => d,
    };
    let (params, pid) = program(code_hash, token, epoch);
    let tp = chain(ctx, &d, token, &d.genesis_owner, [0; 32]);
    let mut want = Vec::new();
    let held = read_chain(got, &tp, &d.genesis_owner, &mut want)?;
    let prog = match there(got, &pid) {
        Err(w) => {
            want.push(w);
            None
        }
        Ok(b) => Some(b.map(|b| A::State::decode(&b).ok_or("an allowance program that does not read")).transpose()?.unwrap_or_default()),
    };
    let (Some((ts, keys)), Some(prog)) = (held, prog) else { return Ok(Step::Want(want)) };
    let (state, made) = if prog.decided() {
        (prog, false)
    } else {
        match crate::tokens::statement_accepted(lot, got, ctx.code_hash, d.beacon, epoch, root)? {
            Step::Want(w) => return Ok(Step::Want(w)),
            Step::Sign(m) => return Ok(Step::Sign(m)),
            Step::Done(Err(_)) => return Ok(Step::Done(None)),
            Step::Done(Ok(_)) => {}
        }
        let tok = t::Tok { definition: *token, item: [0; 32] };
        let escrows: Vec<(t::escrow::EscrowRef, u128)> = ts.entries.iter().filter_map(|e| match &e.body {
            Body::Escrow { program, amount, terms, .. } if *program == pid && terms == A::TERMS => Some(((tok, d.genesis_owner, e.nonce), *amount)),
            _ => None,
        }).collect();
        if escrows.is_empty() {
            return Ok(Step::Done(None));
        }
        (A::verdict(&params, &escrows, root, lines).map_err(String::from)?, true)
    };
    let anyone = Asking::new([0; 32], vec![], 0, vec![]);
    let h = Held { params: tp, definition: d, state: ts, blocks: Default::default() };
    let programs = [(pid, state.encode())];
    let entries = Wallet { device: &anyone, keys: &keys }.collect(&h, &Sources { chains: &[], blocks: &Default::default(), programs: &programs, shards: &[] }, now).map_err(String::from)?;
    let release = (!entries.is_empty()).then(|| {
        let mut s = h.state.clone();
        s.entries.extend(entries);
        Put { params: h.params.encode(), state: s.encode() }
    });
    Ok(Step::Done(Some(Built { program: made.then(|| (params.encode(), state.encode())), release })))
}

/// `did`'s PAYOUT from `epoch`'s program, into its DID chain of the token (worked): the chain to put and the amount
/// (`None`: nothing owed it, not decided yet, or taken already).
/// Only under a program whose statement root was ACCEPTED.
#[allow(clippy::too_many_arguments)]
pub fn payout(ctx: &Ctx, lot: &crate::tokens::Lot, got: Got, code_hash: &[u8; 32], token: &[u8; 32], epoch: u64, did: &[u8; 32], now: u64) -> Result<Step<Option<(Put, u128)>>, String> {
    let d = match definition(got, token)? {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(d) => d,
    };
    let (_, pid) = program(code_hash, token, epoch);
    let prog = match there(got, &pid) {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(None) => return Ok(Step::Done(None)),
        Ok(Some(b)) => A::State::decode(&b).ok_or("an allowance program that does not read")?,
    };
    if !prog.section.pays.iter().any(|p| p.2 == *did) {
        return Ok(Step::Done(None));
    }
    match crate::tokens::statement_accepted(lot, got, ctx.code_hash, d.beacon, epoch, &prog.root)? {
        Step::Want(w) => return Ok(Step::Want(w)),
        Step::Sign(m) => return Ok(Step::Sign(m)),
        Step::Done(Err(_)) => return Ok(Step::Done(None)),
        Step::Done(Ok(_)) => {}
    }
    let cp = chain(ctx, &d, token, did, [0; 32]);
    let mut want = Vec::new();
    let Some((s, keys)) = read_chain(got, &cp, did, &mut want)? else { return Ok(Step::Want(want)) };
    let anyone = Asking::new([0; 32], vec![], 0, vec![]);
    let h = Held { params: cp, definition: d, state: s, blocks: Default::default() };
    let programs = [(pid, prog.encode())];
    let entries = Wallet { device: &anyone, keys: &keys }.collect(&h, &Sources { chains: &[], blocks: &Default::default(), programs: &programs, shards: &[] }, now).map_err(String::from)?;
    let amount: u128 = entries.iter().map(|e| if let Body::Payout { amount, .. } = e.body { amount } else { 0 }).sum();
    if amount == 0 {
        return Ok(Step::Done(None));
    }
    let mut st = h.state;
    st.entries.extend(entries);
    Ok(Step::Done(Some((Put { params: h.params.encode(), state: st.encode() }, amount))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::device;
    use ed25519_dalek::{Signer as _, SigningKey};
    use std::collections::{BTreeMap, HashMap};

    const CODE: [u8; 32] = [4; 32];

    #[test]
    fn the_treasury_escrows_the_verdict_pays_each_line_and_returns_the_rest() {
        let ctx = Ctx { code_hash: [9; 32], beacon: [7; 32], idlog: [6; 32] };
        // Wren: a DID, its key log, this node its device.
        let next = SigningKey::from_bytes(&[151; 32]).verifying_key().to_bytes();
        let ev = craftworks_idlog_contract::Event::signed(0, [0; 32], &[101; 32], craftworks_idlog_contract::commit(&next), [1; 32], [1; 32], vec![]).unwrap();
        let log = craftworks_idlog_contract::Log { events: vec![ev] };
        let wren = log.events[0].id();
        let node = SigningKey::from_bytes(&[12; 32]);
        let mut dev = device(node.verifying_key().to_bytes(), &[craftworks_identity::credential(&wren, &node.verifying_key().to_bytes(), &[7; 32], &[101; 32])], &log, vec![]).unwrap();
        let d = Definition { code_hash: ctx.code_hash, decimals: t::DECIMALS, min_send: 1, genesis_owner: wren, genesis_amount: 1_000 * t::UNIT, name: b"WREN".to_vec(), nonce: [0; 16], beacon: ctx.beacon, margin: 2, idlog: ctx.idlog, mint: None, freeze: None, ids: t::Ids::None, royalty: None, cap: 0, item: [0; 32], version: [0; 32], soulbound: false, transfer_fee: None, sale: None, keepers: None };
        let tok = t::id(&ctx.code_hash, &d.encode());
        let tp = chain(&ctx, &d, &tok, &wren, [0; 32]);
        let (_, pid) = program(&CODE, &tok, 5);
        // The treasury: its genesis, then the epoch's pot escrowed to the program (signed by the device: the run loop).
        let sign = |dev: &mut crate::tokens::Device, f: &dyn Fn(&Asking) -> Result<Vec<t::Entry>, &'static str>| -> Vec<t::Entry> {
            for _ in 0..6 {
                let a = crate::tokens::device_of(dev);
                let r = f(&a);
                match a.next() {
                    Some(m) => dev.sigs.push((m.clone(), node.sign(&m).to_bytes())),
                    None => return r.unwrap(),
                }
            }
            panic!("no end")
        };
        let mut h = Held { params: tp, definition: d.clone(), state: State::default(), blocks: Default::default() };
        h.state.entries = sign(&mut dev, &|a| Wallet { device: a, keys: &log }.genesis(&h, 1));
        let esc = sign(&mut dev, &|a| Wallet { device: a, keys: &log }.debit(&h, Body::Escrow { program: pid, terms: A::TERMS.to_vec(), amount: A::POT, until: 99 }, 2));
        h.state.entries.extend(esc);
        // The epoch's statement: two accounts owed (the root never: it is not listed).
        let (u1, u2) = ([21u8; 32], [22u8; 32]);
        let mut blocks = BTreeMap::new();
        let lines = [(u1, A::CAP), (u2, A::CAP / 3)];
        let (root, _) = engine::repair::build_tree(lines.iter().map(|(d, a)| (d.to_vec(), craftworks_reputation::statement::Line { rep: 1, keeper: 0, allowance: *a, witness: 0 }.encode())), &mut blocks).unwrap();
        let proofs: Vec<([u8; 32], Vec<u8>)> = lines.iter().map(|(d, _)| (*d, freenet_prolly::proof::prove(&blocks, &root, d).unwrap().encode())).collect();
        let mut got: HashMap<[u8; 32], Vec<u8>> = HashMap::new();
        for (k, v) in [(tok, d.encode()), (tp.id(), h.state.encode()), (tp.keys_id(), log.encode()), (pid, vec![])] {
            got.insert(k, v);
        }
        // THE STATEMENT's root not witnessed: no verdict; signed by its committee (the test's six stakers): built.
        let tl = crate::tokens::testlot::stakers(&ctx, &mut got);
        let v = |got: &HashMap<[u8; 32], Vec<u8>>, epoch: u64| crate::tokens::testlot::done(&|g| verdict(&ctx, &tl.lot, g, &CODE, &tok, epoch, &root, &proofs, 10), got);
        assert_eq!(v(&got, 5), Ok(None), "a root its witnesses have not signed pays nothing");
        crate::tokens::testlot::witness_statement(&tl, &mut got, ctx.code_hash, ctx.beacon, 5, &root, 3);
        assert_eq!(v(&got, 5), Ok(None), "three of six: short");
        crate::tokens::testlot::witness_statement(&tl, &mut got, ctx.code_hash, ctx.beacon, 5, &root, 4);
        let Ok(Some(b)) = v(&got, 5) else { panic!("a verdict") };
        let (params, st) = b.program.clone().expect("the program's state");
        let ps = A::State::decode(&st).unwrap();
        assert_eq!(A::check(&A::Params::decode(&params).unwrap(), &ps), Ok(()));
        assert_eq!(ps.section.pays.iter().map(|x| (x.2, x.3)).collect::<Vec<_>>(), vec![(u1, A::CAP), (u2, A::CAP / 3)]);
        // The treasury's release: the rest back, under that verdict — a chain the token takes.
        let rel = State::decode(&b.release.unwrap().state).unwrap();
        let x = t::Extra { ops: None, program: Some(&|id: &[u8; 32]| (id == &pid).then(|| st.clone())) };
        assert_eq!(t::check(&tp, &d, &rel, &|_, _| None, &|_| None, Some(&log), &x), Ok(()));
        assert_eq!(rel.balance(), 1_000 * t::UNIT - A::CAP - A::CAP / 3);
        // An account's payout, into its DID chain (worked); once.
        got.insert(pid, st.clone());
        let p1 = chain(&ctx, &d, &tok, &u1, [0; 32]);
        got.insert(p1.id(), vec![]);
        got.insert(p1.keys_id(), vec![]);
        let pay = |got: &HashMap<[u8; 32], Vec<u8>>, who: &[u8; 32], now: u64| crate::tokens::testlot::done(&|g| payout(&ctx, &tl.lot, g, &CODE, &tok, 5, who, now), got);
        let Ok(Some((put, amount))) = pay(&got, &u1, 10) else { panic!("a payout") };
        assert_eq!(amount, A::CAP);
        let c1 = State::decode(&put.state).unwrap();
        assert_eq!(t::check(&p1, &d, &c1, &|_, _| None, &|_| None, None, &x), Ok(()));
        got.insert(p1.id(), put.state);
        assert_eq!(pay(&got, &u1, 11), Ok(None));
        // Nobody else is paid; a decided program is not built again (its release is: nothing left to release here).
        assert_eq!(pay(&got, &[23; 32], 11), Ok(None));
        let mut g2 = got.clone();
        g2.insert(tp.id(), rel.encode());
        assert_eq!(v(&g2, 5), Ok(Some(Built { program: None, release: None })));
        // An epoch nothing was escrowed to (its root witnessed): no verdict.
        crate::tokens::testlot::witness_statement(&tl, &mut g2, ctx.code_hash, ctx.beacon, 6, &root, 6);
        assert_eq!(v(&g2, 6), Ok(None));
    }
}
