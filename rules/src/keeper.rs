//! THE KEEPER LOOP in the page's core (TOKEN §7a: keepers are paid on proven keeping) — a keeper's STANDING RECORD kept
//! in its standing contract (one per keeper: a page's is named by its member key in hex — a page cannot learn its peer
//! id): each standing epoch its BINDING and its HOLDINGS (the pieces of the files it keeps, committed); at each height sampled for it, its ANSWER to the beacon's challenge (the asked
//! pieces' sealed hashes — sealed in the page's worker). Every record is signed by the node's member key through its
//! identity delegate (`TokenSign`, the identity's STANDING_DOMAIN): a step answers what to SIGN, or the PUT. Pure, over
//! `got` (a contract's state as read; `Some(empty)`: read and not there).

use crate::tokens::{there, Got, Step};
use craftworks_standing::{codec, record, Answer, Bind, Holdings};
use craftworks_standing_contract::Params;
use craftworks_token_contract as t;

/// Where a node's record is: the standing contract's code hash, the beacon (its code hash and id).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keep {
    pub standing: [u8; 32],
    pub beacon_code: [u8; 32],
    pub beacon: [u8; 32],
}

/// The record's PUT: its params and state.
#[derive(Debug, Clone, PartialEq)]
pub struct Put {
    pub params: Vec<u8>,
    pub state: Vec<u8>,
}

/// A node's holdings cover the whole ring (LOCATION is a soft rule: the arc applies when peers place it).
const WHOLE: (u64, u64) = (0, u64::MAX);

fn params(k: &Keep, peer: &str) -> Params {
    Params { beacon_code: k.beacon_code, beacon: k.beacon, peer: peer.into() }
}

/// The STANDING RECORD's contract id of the keeper whose key is `key` (its name: `craftworks_standing::keeper_peer`).
pub fn record_id(k: &Keep, key: &[u8; 32]) -> [u8; 32] {
    Params::keeper(k.beacon_code, k.beacon, key).id(&k.standing)
}

/// The keeper's record as read (empty when not there).
fn record_of(k: &Keep, got: Got, peer: &str) -> Result<Result<record::State, [u8; 32]>, String> {
    let id = t::id(&k.standing, &params(k, peer).encode());
    Ok(match there(got, &id) {
        Err(w) => Err(w),
        Ok(None) => Ok(record::State::default()),
        Ok(Some(b)) => Ok(codec::decode(&b).ok_or("a standing record that does not read")?),
    })
}

fn signed(sigs: &[(Vec<u8>, [u8; 64])], m: &[u8]) -> Option<[u8; 64]> {
    sigs.iter().find(|(x, _)| x == m).map(|x| x.1)
}

fn put(k: &Keep, peer: &str, held: &record::State, e: record::Epoch) -> Put {
    let merged = record::merge(held, &record::State { epochs: vec![e] });
    Put { params: params(k, peer).encode(), state: codec::encode(&merged) }
}

/// OPEN standing epoch `epoch` in the node's record: its binding and its holdings (`hosted`: what its node hosts, id and
/// size), both signed by `public` (the member key; `sigs`: the delegate's answers so far). `None`: already open.
pub fn open_epoch(k: &Keep, got: Got, public: &[u8; 32], epoch: u64, hosted: &[([u8; 32], u64)], sigs: &[(Vec<u8>, [u8; 64])]) -> Result<Step<Option<Put>>, String> {
    let peer = &craftworks_standing::keeper_peer(public);
    let held = match record_of(k, got, peer)? {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(s) => s,
    };
    if held.epochs.iter().any(|e| e.epoch == epoch && e.holdings.iter().any(|h| h.key == *public)) {
        return Ok(Step::Done(None));
    }
    let mut b = Bind::draft(*public, peer, epoch);
    let Some(sb) = signed(sigs, &b.signed()) else { return Ok(Step::Sign(b.signed())) };
    b.sig = sb;
    let (mut h, _) = Holdings::draft(*public, peer, epoch, hosted, WHOLE);
    let Some(sh) = signed(sigs, &h.signed()) else { return Ok(Step::Sign(h.signed())) };
    h.sig = sh;
    Ok(Step::Done(Some(put(k, peer, &held, record::Epoch { epoch, binds: vec![b], holdings: vec![h], ..Default::default() }))))
}

/// The holdings `public` committed for `epoch` in the record, and their ids (from `hosted`, recomputed: they must
/// commit to the same root). `None`: not open, or `hosted` moved since.
fn holdings(held: &record::State, public: &[u8; 32], peer: &str, epoch: u64, hosted: &[([u8; 32], u64)]) -> Option<(Holdings, Vec<[u8; 32]>)> {
    let h = held.epochs.iter().find(|e| e.epoch == epoch)?.holdings.iter().find(|h| h.key == *public)?.clone();
    let (again, ids) = Holdings::draft(*public, peer, epoch, hosted, WHOLE);
    (again.root == h.root).then_some((h, ids))
}

/// The PIECES height `height` (its beacon output `out`) asks this node for: `None` when not sampled for it, its epoch
/// not open, or answered already.
pub fn asked(k: &Keep, got: Got, public: &[u8; 32], height: u64, out: &[u8; 32], hosted: &[([u8; 32], u64)]) -> Result<Step<Option<Vec<[u8; 32]>>>, String> {
    let peer = &craftworks_standing::keeper_peer(public);
    if !craftworks_standing::sampled(out, peer) {
        return Ok(Step::Done(None));
    }
    let held = match record_of(k, got, peer)? {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(s) => s,
    };
    let epoch = craftworks_standing::epoch(height);
    if held.epochs.iter().any(|e| e.epoch == epoch && e.answers.iter().any(|a| a.height == height && a.key == *public)) {
        return Ok(Step::Done(None));
    }
    Ok(Step::Done(holdings(&held, public, peer, epoch, hosted).map(|(h, ids)| Answer::asked(&h, &ids, out))))
}

/// The ANSWER at `height` from the asked pieces' sealed hashes (`hashes`: id → hash), signed: the record to put
/// (`None`: nothing to answer — see [`asked`]).
#[allow(clippy::too_many_arguments)]
pub fn answer(k: &Keep, got: Got, public: &[u8; 32], height: u64, out: &[u8; 32], hosted: &[([u8; 32], u64)], hashes: &[([u8; 32], [u8; 32])], sigs: &[(Vec<u8>, [u8; 64])]) -> Result<Step<Option<Put>>, String> {
    let peer = &craftworks_standing::keeper_peer(public);
    let held = match record_of(k, got, peer)? {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(s) => s,
    };
    let epoch = craftworks_standing::epoch(height);
    let Some((h, ids)) = holdings(&held, public, peer, epoch, hosted) else { return Ok(Step::Done(None)) };
    let Some(mut a) = Answer::draft_hashed(&h, &ids, height, out, &|id| hashes.iter().find(|x| x.0 == *id).map(|x| x.1)) else { return Ok(Step::Done(None)) };
    let Some(s) = signed(sigs, &a.message()) else { return Ok(Step::Sign(a.message())) };
    a.sig = s;
    Ok(Step::Done(Some(put(k, peer, &held, record::Epoch { epoch, answers: vec![a], ..Default::default() }))))
}

// ---- THE KEEPER MINT (TOKEN §7d, K4): an account's keeper line of an ACCEPTED statement, minted as $CRAFT stake ----

/// What a keeper mint puts: $CRAFT's contracts first if the network has not got them (anyone may: its definition is
/// fixed, `craftworks_wallet::craft`), then the account's chain with the mint appended.
#[derive(Debug, Clone, PartialEq)]
pub struct Minted {
    pub craft: Vec<crate::tokens::Put>,
    pub chain: crate::tokens::Put,
    pub amount: u128,
    /// Its witness fees (spendable).
    pub fee: u128,
}

/// $CRAFT's definition here and its id.
pub fn craft(ctx: &crate::tokens::Ctx) -> (t::Definition, [u8; 32]) {
    let d = craftworks_wallet::craft::definition(ctx.code_hash, ctx.beacon, ctx.idlog);
    let id = t::id(&ctx.code_hash, &d.encode());
    (d, id)
}

/// MINT `did`'s keeper line of standing epoch `epoch`'s statement (`root`; `proof`, its line's proof) into its $CRAFT
/// chain, WORKED (anyone's: nobody need be online to be paid), at beacon height `now`. Only under the root ACCEPTED
/// for the epoch; `None`: no line (or 0), not accepted (yet), or minted already.
#[allow(clippy::too_many_arguments)]
pub fn mint(ctx: &crate::tokens::Ctx, lot: &crate::tokens::Lot, got: Got, did: &[u8; 32], epoch: u64, root: &[u8; 32], proof: &[u8], now: u64) -> Result<Step<Option<Minted>>, String> {
    let Some(line) = craftworks_reputation::statement::line_proven(root, did, proof) else { return Ok(Step::Done(None)) };
    if line.keeper == 0 && line.witness == 0 {
        return Ok(Step::Done(None));
    }
    let (d, token) = craft(ctx);
    match crate::tokens::statement_accepted(lot, got, ctx.code_hash, d.beacon, epoch, root)? {
        Step::Want(w) => return Ok(Step::Want(w)),
        Step::Sign(m) => return Ok(Step::Sign(m)),
        Step::Done(Err(_)) => return Ok(Step::Done(None)),
        Step::Done(Ok(_)) => {}
    }
    let craft_there = match there(got, &token) {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(b) => b.is_some(),
    };
    let cp = crate::tokens::chain(ctx, &d, &token, did, [0; 32]);
    let mut want = Vec::new();
    let Some((s, keys)) = crate::allowance::read_chain(got, &cp, did, &mut want)? else { return Ok(Step::Want(want)) };
    let nonce = t::keeper_nonce(&token, epoch);
    // (Minted already — unless that mint was SUPERSEDED, never witnessed: TOKEN §3d, minted again.)
    if s.entries.iter().any(|e| e.nonce == nonce && !t::is_superseded(&s, e.seq)) {
        return Ok(Step::Done(None));
    }
    let mut e = t::Entry { seq: s.entries.last().map_or(0, |e| e.seq + 1), prev: s.head(), nonce, stamp: now.max(s.entries.last().map_or(0, |e| e.stamp)), body: t::Body::KeeperMint { epoch, root: *root, proof: proof.to_vec(), amount: line.keeper, fee: line.witness }, balance: s.balance() + line.witness, auth: t::Auth::Work(0) };
    let m = e.message();
    e.auth = t::Auth::Work((0u64..).find(|w| t::work_ok(&m, *w)).unwrap());
    let mut st = s;
    st.entries.push(e);
    t::walk(&cp, &d, &st, &|_, _| None, &|_| None, Some(&keys), 0, usize::MAX, &t::Extra::NONE).map_err(|(_, w)| w.to_string())?;
    let craft = if craft_there { vec![] } else { d.contracts(&ctx.code_hash).into_iter().map(|(params, state)| crate::tokens::Put { params, state }).collect() };
    Ok(Step::Done(Some(Minted { craft, chain: crate::tokens::Put { params: cp.encode(), state: st.encode() }, amount: line.keeper, fee: line.witness })))
}

// ---- THE STAKE CLAIM (TOKEN §7a, S2): a keeper's $CRAFT stake standing in the next epoch's witness lots ----

/// What a stake claim puts: the $CRAFT chain with this device made its writer (if it was not), and the claims registry
/// of `epoch` with this account's claim.
#[derive(Debug, Clone, PartialEq)]
pub struct Claimed {
    pub chain: Option<crate::tokens::Put>,
    pub registry: crate::tokens::Put,
    pub epoch: u64,
    pub amount: u128,
}

/// CLAIM `did`'s $CRAFT stake for the next claims epoch whose registry is still open (it freezes `claims::LEAD` steps
/// before its epoch), witness key this device's (`dev`: the member key — made the chain's writer first, so the stake
/// counts for it), at beacon height `now`. `None`: nothing staked, or claimed already.
pub fn claim(ctx: &crate::tokens::Ctx, got: Got, did: &[u8; 32], now: u64, dev: &crate::tokens::Device) -> Result<Step<Option<Claimed>>, String> {
    let (_, token) = craft(ctx);
    // No $CRAFT on this network yet (nobody minted it): nothing staked.
    match there(got, &token) {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(None) => return Ok(Step::Done(None)),
        Ok(Some(_)) => {}
    }
    let (held, keys) = match crate::tokens::holding(ctx, got, &t::Tok { definition: token, item: [0; 32] }, did)? {
        Step::Done(Some(h)) => h,
        Step::Done(None) => return Ok(Step::Done(None)),
        Step::Want(w) => return Ok(Step::Want(w)),
        Step::Sign(m) => return Ok(Step::Sign(m)),
    };
    let staked = t::walk(&held.params, &held.definition, &held.state, &|_, _| None, &|_| None, Some(&keys), 0, usize::MAX, &t::Extra::NONE).map_or(0, |w| w.staked);
    if staked == 0 {
        return Ok(Step::Done(None));
    }
    let mut epoch = now / t::supply::EPOCH + 1;
    let rp = |e: u64| t::claims::Params { code_hash: ctx.code_hash, pool: token, beacon: held.definition.beacon, epoch: e };
    if now >= rp(epoch).frozen_at() {
        epoch += 1;
    }
    let reg = match there(got, &rp(epoch).id()) {
        Err(w) => return Ok(Step::Want(vec![w])),
        Ok(None) => t::claims::Registry::default(),
        Ok(Some(b)) => t::claims::Registry::decode(&b).ok_or("a claims registry that does not read")?,
    };
    if reg.claims.iter().any(|c| c.owner == *did && c.key == dev.public) {
        return Ok(Step::Done(None));
    }
    let asking = crate::tokens::device_of(dev);
    let adopted = craftworks_wallet::Wallet { device: &asking, keys: &keys }.adopt(&held, now);
    if let Some(m) = asking.next() {
        return Ok(Step::Sign(m));
    }
    let adopted = adopted.map_err(String::from)?;
    let message = t::draw::StakeClaim::message(&token, did, staked, epoch, &dev.public);
    let Some(sig) = dev.sigs.iter().find(|(m, _)| *m == message).map(|x| x.1) else { return Ok(Step::Sign(message)) };
    let mine = t::draw::StakeClaim { owner: *did, amount: staked, epoch, key: dev.public, sig };
    let registry = t::claims::merge(&reg, &t::claims::Registry { claims: vec![mine] });
    let chain = (!adopted.is_empty()).then(|| {
        let mut st = held.state.clone();
        st.entries.extend(adopted);
        crate::tokens::Put { params: held.params.encode(), state: st.encode() }
    });
    Ok(Step::Done(Some(Claimed { chain, registry: crate::tokens::Put { params: rp(epoch).encode(), state: registry.encode() }, epoch, amount: staked })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};
    use std::collections::HashMap;

    /// The delegate's loop over a step: sign what it asks with `key` (the member key).
    fn run<T>(key: &SigningKey, step: &mut dyn FnMut(&[(Vec<u8>, [u8; 64])]) -> Result<Step<T>, String>) -> T {
        let mut sigs = Vec::new();
        for _ in 0..8 {
            match step(&sigs).unwrap() {
                Step::Done(x) => return x,
                Step::Sign(m) => {
                    assert!(m.starts_with(craftworks_identity::STANDING_DOMAIN), "under the standing domain");
                    sigs.push((m.clone(), key.sign(&m).to_bytes()));
                }
                Step::Want(_) => panic!("a want"),
            }
        }
        panic!("no end")
    }

    #[test]
    fn a_keeper_opens_its_epoch_and_answers_a_sampled_height_in_a_record_that_holds() {
        let k = Keep { standing: [31; 32], beacon_code: [32; 32], beacon: [33; 32] };
        let key = SigningKey::from_bytes(&[44; 32]);
        let public = key.verifying_key().to_bytes();
        // A page's keeper: its member key in hex (64 characters, PEER_MAX).
        let peer_ = craftworks_standing::keeper_peer(&public);
        let peer = peer_.as_str();
        let piece = |id: &[u8; 32]| [id.to_vec(), vec![7u8; 100]].concat();
        let hosted: Vec<([u8; 32], u64)> = (0..40u32).map(|i| (*blake3::hash(&i.to_le_bytes()).as_bytes(), 132)).collect();
        let rid = record_id(&k, &public);
        let mut got: HashMap<[u8; 32], Vec<u8>> = HashMap::new();
        got.insert(rid, vec![]);
        // OPEN epoch 3: binding and holdings, both signed under the standing domain.
        let g = got.clone();
        let p = run(&key, &mut |s| open_epoch(&k, &|x| g.get(x).cloned(), &public, 3, &hosted, s)).expect("opened");
        got.insert(rid, p.state.clone());
        let g = got.clone();
        assert_eq!(run(&key, &mut |s| open_epoch(&k, &|x| g.get(x).cloned(), &public, 3, &hosted, s)), None, "opened once");
        // A SAMPLED height of epoch 3: the asked pieces, their sealed hashes, the answer.
        let out = |h: u64| *blake3::hash(&h.to_le_bytes()).as_bytes();
        let h = (3 * craftworks_standing::EPOCH..4 * craftworks_standing::EPOCH).find(|h| craftworks_standing::sampled(&out(*h), peer)).expect("a sampled height");
        let ask = |got: &HashMap<[u8; 32], Vec<u8>>, h: u64| match asked(&k, &|x| got.get(x).cloned(), &public, h, &out(h), &hosted).unwrap() {
            Step::Done(x) => x,
            _ => panic!("a step"),
        };
        let unsampled = (h + 1..).find(|x| !craftworks_standing::sampled(&out(*x), peer)).unwrap();
        assert_eq!(ask(&got, unsampled), None, "not sampled: nothing asked");
        let pieces = ask(&got, h).expect("asked");
        assert_eq!(pieces.len(), craftworks_standing::PICKS.min(40));
        let hashes: Vec<([u8; 32], [u8; 32])> = pieces.iter().map(|id| (*id, craftworks_standing::sealed_hash(&public, id, &piece(id), &out(h), peer))).collect();
        let g = got.clone();
        let p = run(&key, &mut |s| answer(&k, &|x| g.get(x).cloned(), &public, h, &out(h), &hosted, &hashes, s)).expect("answered");
        got.insert(rid, p.state.clone());
        assert_eq!(ask(&got, h), None, "answered once");
        // The RECORD holds, as the standing contract checks it; and each answer checks against a piece's real seal.
        let rec = codec::decode(&p.state).unwrap();
        assert_eq!(record::holds(peer, &rec, &|x| Some(out(x))), Ok(()));
        // A STRANGER's key bound into this keeper's record (audit 10-11: it crowded the keeper's answers out): refused.
        let stranger = SigningKey::from_bytes(&[45; 32]);
        let mut bind = Bind::draft(stranger.verifying_key().to_bytes(), peer, 3);
        bind.sig = stranger.sign(&bind.signed()).to_bytes();
        let mut crowded = rec.clone();
        crowded.epochs.iter_mut().find(|e| e.epoch == 3).unwrap().binds.push(bind);
        assert_eq!(record::holds(peer, &crowded, &|x| Some(out(x))), Err("a binding of another key in a keeper's record"));
        let e = rec.epochs.iter().find(|e| e.epoch == 3).unwrap();
        let (hd, a) = (&e.holdings[0], &e.answers[0]);
        assert_eq!(craftworks_standing::check(hd, a, &out(h), None, Some((0, &piece(&a.picks[0].id)))), Ok(()));
        // A wrong hash (a piece not held): its answer does not check.
        let mut bad = hashes.clone();
        bad[0].1 = [0; 32];
        let mut g2 = got.clone();
        g2.insert(rid, codec::encode(&record::State { epochs: rec.epochs.iter().map(|e| record::Epoch { answers: vec![], ..e.clone() }).collect() }));
        let p2 = run(&key, &mut |s| answer(&k, &|x| g2.get(x).cloned(), &public, h, &out(h), &hosted, &bad, s)).unwrap();
        let rec2 = codec::decode(&p2.state).unwrap();
        let a2 = &rec2.epochs.iter().find(|e| e.epoch == 3).unwrap().answers[0];
        let j = a2.picks.iter().position(|p| p.id == bad[0].0).unwrap();
        assert!(craftworks_standing::check(hd, a2, &out(h), None, Some((j, &piece(&bad[0].0)))).is_err());
        // KEPT (the statement's): only once KEEP_MIN sampled heights of the epoch are answered — one is not enough.
        let did = [9u8; 32];
        let record_in = |got: &HashMap<[u8; 32], Vec<u8>>| codec::decode(&got[&rid]).unwrap();
        let lot_out = out(9_999_999);
        let kept_of = |got: &HashMap<[u8; 32], Vec<u8>>, e: u64| record::kept_audited(&[(did, public)], &|key| (*key == public).then(|| record_in(got)), e, &lot_out, &|x| Some(out(x)), &|id| Some(piece(id)));
        assert!(!record::kept(&record_in(&got), &public, 3));
        assert_eq!(kept_of(&got, 3), Vec::<[u8; 32]>::new());
        let mut x = h;
        for _ in 1..record::KEEP_MIN {
            x = (x + 1..4 * craftworks_standing::EPOCH).find(|y| craftworks_standing::sampled(&out(*y), peer)).expect("another sampled height in the epoch");
            let ps = ask(&got, x).expect("asked");
            let hs: Vec<([u8; 32], [u8; 32])> = ps.iter().map(|id| (*id, craftworks_standing::sealed_hash(&public, id, &piece(id), &out(x), peer))).collect();
            let g = got.clone();
            let p = run(&key, &mut |s| answer(&k, &|y| g.get(y).cloned(), &public, x, &out(x), &hosted, &hs, s)).expect("answered");
            got.insert(rid, p.state);
        }
        assert!(record::kept(&record_in(&got), &public, 3));
        assert_eq!(kept_of(&got, 3), vec![did]);
        assert_eq!(kept_of(&got, 4), Vec::<[u8; 32]>::new(), "another epoch: not kept");
    }

    #[test]
    fn a_keeper_line_of_an_accepted_statement_mints_once_as_craft_stake() {
        use craftworks_reputation::statement::{Line, Statement};
        let ctx = crate::tokens::Ctx { code_hash: [9; 32], beacon: [7; 32], idlog: [6; 32] };
        let mut got: HashMap<[u8; 32], Vec<u8>> = HashMap::new();
        let tl = crate::tokens::testlot::stakers(&ctx, &mut got);
        let (keeper, idle) = ([21u8; 32], [22u8; 32]);
        let lines = [(keeper, Line { rep: 1, keeper: 500, allowance: 0, witness: 0, active: 0 }), (idle, Line { rep: 1, keeper: 0, allowance: 0, witness: 0, active: 0 })].into_iter().collect();
        let mut blocks = std::collections::BTreeMap::new();
        let (root, _) = Statement { epoch: 5, lines }.tree(&mut blocks).unwrap();
        let proof = |d: &[u8; 32]| freenet_prolly::proof::prove(&blocks, &root, d).unwrap().encode();
        let now = 5 * craftworks_standing::EPOCH + 10;
        let m = |got: &HashMap<[u8; 32], Vec<u8>>, who: &[u8; 32]| crate::tokens::testlot::done(&|g| mint(&ctx, &tl.lot, g, who, 5, &root, &proof(who), now), got);
        assert_eq!(m(&got, &keeper), Ok(None), "its statement not accepted yet");
        crate::tokens::testlot::witness_statement(&tl, &mut got, ctx.code_hash, ctx.beacon, 5, &root, 4);
        // WITNESS FEES for STATEMENTS (`wallet::fees`): the four seated signers of the accepted root each count once; a
        // root only two signed (short of the committee's need) counts nothing.
        {
            use craftworks_reputation::statement::{lot_height, witness_place};
            let place = |r: [u8; 32]| craftworks_wallet::fees::Statement { place: witness_place(5), root: r, h: lot_height(5) };
            let count = |g: &HashMap<[u8; 32], Vec<u8>>, st: &[craftworks_wallet::fees::Statement]| crate::tokens::testlot::done(&|gg| Ok(crate::witness::signed(&ctx, &tl.lot, gg, &[], st)), g).unwrap();
            let c = count(&got, &[place(root)]);
            assert_eq!((c.len(), c.values().sum::<u64>()), (4, 4), "{c:?}");
            let mut g2 = got.clone();
            crate::tokens::testlot::witness_statement(&tl, &mut g2, ctx.code_hash, ctx.beacon, 5, &[8; 32], 2);
            assert!(count(&g2, &[place([8; 32])]).is_empty(), "a root not accepted pays nothing");
        }
        let minted = m(&got, &keeper).unwrap().expect("minted");
        assert_eq!((minted.amount, minted.craft.len()), (500, 3), "its line; $CRAFT's contracts put with it (not on the network yet)");
        let (d, token) = craft(&ctx);
        let cp = crate::tokens::chain(&ctx, &d, &token, &keeper, [0; 32]);
        let st = t::State::decode(&minted.chain.state).unwrap();
        let w = t::walk(&cp, &d, &st, &|_, _| None, &|_| None, None, 0, usize::MAX, &t::Extra::NONE).unwrap();
        assert_eq!((w.staked, st.balance()), (500, 0), "LOCKED as stake");
        // Minted already: nothing again; $CRAFT there: not put again.
        let mut g2 = got.clone();
        g2.insert(cp.id(), minted.chain.state.clone());
        g2.insert(token, d.encode());
        assert_eq!(m(&g2, &keeper), Ok(None), "once an epoch");
        // A line of nothing mints nothing.
        assert_eq!(m(&got, &idle), Ok(None));
    }

    #[test]
    fn a_keeper_with_stake_claims_the_next_epoch_with_its_member_key_as_its_chains_writer() {
        use craftworks_reputation::statement::{Line, Statement};
        let ctx = crate::tokens::Ctx { code_hash: [9; 32], beacon: [7; 32], idlog: [6; 32] };
        let mut got: HashMap<[u8; 32], Vec<u8>> = HashMap::new();
        let tl = crate::tokens::testlot::stakers(&ctx, &mut got);
        // An account (its key log) and this node, its device.
        let next = SigningKey::from_bytes(&[152; 32]).verifying_key().to_bytes();
        let ev = craftworks_idlog_contract::Event::signed(0, [0; 32], &[102; 32], craftworks_idlog_contract::commit(&next), [1; 32], [1; 32], vec![]).unwrap();
        let log = craftworks_idlog_contract::Log { events: vec![ev] };
        let did = log.events[0].id();
        let node = SigningKey::from_bytes(&[13; 32]);
        let public = node.verifying_key().to_bytes();
        let mut dev = crate::tokens::device(public, &[craftworks_identity::credential(&did, &public, &[7; 32], &[102; 32])], &log, vec![]).unwrap();
        // Its keeper line minted (epoch 5), as a page mints it.
        let lines = [(did, Line { rep: 1, keeper: 500, allowance: 0, witness: 0, active: 0 })].into_iter().collect();
        let mut blocks = std::collections::BTreeMap::new();
        let (root, _) = Statement { epoch: 5, lines }.tree(&mut blocks).unwrap();
        let proof = freenet_prolly::proof::prove(&blocks, &root, &did).unwrap().encode();
        crate::tokens::testlot::witness_statement(&tl, &mut got, ctx.code_hash, ctx.beacon, 5, &root, 4);
        let now = 5 * craftworks_standing::EPOCH + 40;
        let (d, token) = craft(&ctx);
        let cp = crate::tokens::chain(&ctx, &d, &token, &did, [0; 32]);
        got.insert(cp.keys_id(), log.encode());
        let minted = crate::tokens::testlot::done(&|g| mint(&ctx, &tl.lot, g, &did, 5, &root, &proof, now), &got).unwrap().unwrap();
        got.insert(token, d.encode());
        got.insert(cp.id(), minted.chain.state.clone());
        // CLAIM: the device signs what it is asked (the writer entry, then the claim) — the page's run loop.
        let claimed = loop {
            let g = got.clone();
            match crate::tokens::testlot::done(&|gg| match claim(&ctx, gg, &did, now, &dev)? {
                Step::Sign(m) => Ok(Step::Done(Err(m))),
                Step::Done(x) => Ok(Step::Done(Ok(x))),
                Step::Want(w) => Ok(Step::Want(w)),
            }, &g).unwrap() {
                Err(m) => dev.sigs.push((m.clone(), node.sign(&m).to_bytes())),
                Ok(c) => break c.expect("claimed"),
            }
        };
        let ce = now / t::supply::EPOCH + 1;
        assert_eq!((claimed.epoch, claimed.amount), (ce, 500), "the next claims epoch, its whole stake");
        let reg = t::claims::Registry::decode(&claimed.registry.state).unwrap();
        let c = &reg.claims[0];
        assert!(c.holds(&token) && c.key == public && c.owner == did, "signed by the member key");
        // The member key is now its chain's writer: the stake counts for that key (what the lot checks).
        let st = t::State::decode(&claimed.chain.expect("a writer entry").state).unwrap();
        let (amount, keys) = t::wallet::stake_at(&cp, &d, &st, ce * t::supply::EPOCH).unwrap();
        assert!(amount >= c.amount && keys.contains(&public));
        // Claimed already: nothing again.
        let mut g2 = got.clone();
        g2.insert(t::claims::Params { code_hash: ctx.code_hash, pool: token, beacon: d.beacon, epoch: ce }.id(), claimed.registry.state.clone());
        g2.insert(cp.id(), st.encode());
        assert_eq!(crate::tokens::testlot::done(&|g| claim(&ctx, g, &did, now, &dev), &g2), Ok(None));
    }
}
