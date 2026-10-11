//! THE EPOCH STATEMENT witnessed with no page open, against a scripted network holding a real lot (`testlot`: six
//! stakers, six seats) and the inputs of epoch 5's statement: two keepers listed for epoch 4 (one answering with real
//! seals, one with junk hashes: the audits tell them apart), a vouch item, epoch 4's statement posted, witnessed (its
//! lines carried) and its tree in Block contracts. The page's path — `rules::statement::build` over the same inputs, as
//! core `statement_build` runs it — gives the root; the delegate builds the same root and signs it into the statement's
//! signatures container, its two audits sealed over two wake-ups. A posted root it does not reproduce is not signed.

use crate::upkeep::{Io, Reply};
use crate::witness_round;
use craftworks_identity::{self as identity, Answer, Host, Request, WitnessMandate};
use craftworks_reputation as R;
use craftworks_rules::keeper::{answer, asked, open_epoch, record_id, Keep};
use craftworks_rules::tokens::{testlot, Ctx, Step};
use craftworks_standing::{record, EPOCH};
use craftworks_token_contract as t;
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier};
use std::collections::{BTreeMap, HashMap};

use crate::testhost::Map;

const APP: [u8; 32] = [0xA1; 32];
/// The statement epoch, and the beacon's height (its statement due, its lot drawn: `lot_height(5)` = 5154).
const E: u64 = 5;
const HEIGHT: u64 = E * R::EPOCH + 120;
const STANDING: [u8; 32] = [43; 32];
const BLOCK: [u8; 32] = [44; 32];

struct World {
    ctx: Ctx,
    tl: testlot::TestLot,
    keep: Keep,
    got: HashMap<[u8; 32], Vec<u8>>,
    /// What the page's `statement` service reads for epoch 5 (`index.bag.chain` / `read`).
    items: Vec<R::item::Item>,
    keepers: Vec<Vec<u8>>,
    posted_before: Vec<[u8; 32]>,
}

/// The beacon's output at a height (testlot's outputs log).
fn out(h: u64) -> [u8; 32] {
    *blake3::hash(&h.to_le_bytes()).as_bytes()
}
fn piece(id: &[u8; 32]) -> Vec<u8> {
    [id.to_vec(), vec![7u8; 100]].concat()
}
fn bag_put(got: &mut HashMap<[u8; 32], Vec<u8>>, bag: &[u8; 32], address: [u8; 32], payloads: &[Vec<u8>]) {
    let items: Vec<Vec<u8>> = payloads.iter().map(|p| craftworks_bag_contract::grind(&address, p)).collect();
    got.insert(t::id(bag, &address), craftworks_bag_contract::encode(&address, &items));
}

fn world() -> World {
    let ctx = Ctx { code_hash: [9; 32], beacon: [7; 32], idlog: [6; 32] };
    let mut got = HashMap::new();
    let tl = testlot::stakers(&ctx, &mut got);
    // THE BEACON at HEIGHT (one step; the heights before from its outputs log).
    let bp = craftworks_beacon_contract::Params { k: 16, segs: 1, sample: 1, window: 2, keep: 600, seed: [5; 32] };
    let top = craftworks_beacon_contract::State { steps: vec![craftworks_beacon_contract::Step::compute(&bp, [0; 32], HEIGHT, [1; 32])], alts: vec![], frauds: vec![] };
    got.insert(ctx.beacon, craftworks_beacon_contract::wire::encode(&top));
    let keep = Keep { standing: STANDING, beacon_code: tl.lot.beacon_code, beacon: ctx.beacon };
    // TWO KEEPERS listed for epoch 4, each record made by the keeper loop (honest: real seals; else junk hashes).
    let hosted: Vec<([u8; 32], u64)> = (0..40u32).map(|i| (*blake3::hash(&i.to_le_bytes()).as_bytes(), 132)).collect();
    for (id, _) in &hosted {
        got.insert(*id, piece(id));
    }
    let mut keepers = Vec::new();
    let mut voucher = [0; 32];
    for (n, honest) in [(0u8, true), (1u8, false)] {
        let owner_seed = [160 + n; 32];
        let next = SigningKey::from_bytes(&[180 + n; 32]).verifying_key().to_bytes();
        let ev = craftworks_idlog_contract::Event::signed(0, [0; 32], &owner_seed, craftworks_idlog_contract::commit(&next), [1; 32], [1; 32], vec![]).unwrap();
        let log = craftworks_idlog_contract::Log { events: vec![ev] };
        let did = log.events[0].id();
        voucher = did;
        let key = SigningKey::from_bytes(&[200 + n; 32]);
        let public = key.verifying_key().to_bytes();
        got.insert(t::id(&ctx.idlog, &did), log.encode());
        keepers.push(R::keepers::Keeper { did, cred: craftworks_identity::credential(&did, &public, &[7; 32], &owner_seed), key_seq: 0 }.encode());
        let rid = record_id(&keep, &public);
        got.insert(rid, vec![]);
        let signing = |step: &dyn Fn(&[(Vec<u8>, [u8; 64])]) -> Result<Step<Option<craftworks_rules::keeper::Put>>, String>| {
            let mut sigs = Vec::new();
            loop {
                match step(&sigs).unwrap() {
                    Step::Done(p) => return p,
                    Step::Sign(m) => sigs.push((m.clone(), key.sign(&m).to_bytes())),
                    Step::Want(w) => panic!("want {w:?}"),
                }
            }
        };
        let g = got.clone();
        let p = signing(&|s| open_epoch(&keep, &|x| g.get(x).cloned(), &public, E - 1, &hosted, s)).expect("opened");
        got.insert(rid, p.state);
        let peer = craftworks_standing::keeper_peer(&public);
        let heights: Vec<u64> = ((E - 1) * EPOCH..E * EPOCH).filter(|h| craftworks_standing::sampled(&out(*h), &peer)).take(record::KEEP_MIN).collect();
        for h in heights {
            let g = got.clone();
            let Step::Done(Some(ids)) = asked(&keep, &|x| g.get(x).cloned(), &public, h, &out(h), &hosted).unwrap() else { panic!("asked") };
            let hashes: Vec<([u8; 32], [u8; 32])> = ids.iter().map(|id| (*id, if honest { craftworks_standing::sealed_hash(&public, id, &piece(id), &out(h), &peer) } else { [0xEE; 32] })).collect();
            let p = signing(&|s| answer(&keep, &|x| g.get(x).cloned(), &public, h, &out(h), &hosted, &hashes, s)).expect("answered");
            got.insert(rid, p.state);
        }
        assert!(record::kept(&craftworks_standing::codec::decode(&got[&rid]).unwrap(), &public, E - 1), "kept before its audit");
    }
    bag_put(&mut got, &tl.lot.bag, R::keepers::address(E - 1, 0), &keepers);
    // A VOUCH ITEM in epoch 3's registry (its signature junk: the rule drops it — the key log is read all the same).
    let items = vec![R::item::Item { from: voucher, to: [12; 32], at: 3 * R::EPOCH + 5, on: true, cred: vec![], key_seq: 0, sig: [0; 64] }];
    bag_put(&mut got, &tl.lot.bag, R::item::registry_address(3, 0), &[items[0].encode()]);
    // EPOCH 4's STATEMENT: posted, accepted (4 of its 6 seats signed it), its tree in Block contracts — its lines carry.
    let prev = R::statement::Statement { epoch: E - 1, lines: [([0x55; 32], R::statement::Line { active: 3, ..Default::default() })].into_iter().collect() };
    let mut blocks = BTreeMap::new();
    let (root4, parity) = prev.tree(&mut blocks).unwrap();
    let block_id = contract_keys::block::contract_deriver_of_hash(BLOCK);
    for (cid, body) in &blocks {
        got.insert(block_id(cid), contract_keys::block::block_state(cid, body).unwrap());
    }
    bag_put(&mut got, &tl.lot.bag, R::statement::bag_address(E - 1), &[R::statement::Posted { epoch: E - 1, root: root4, parity }.encode()]);
    testlot::witness_statement(&tl, &mut got, ctx.code_hash, ctx.beacon, E - 1, &root4, 4);
    World { ctx, tl, keep, got, items, keepers, posted_before: vec![root4] }
}

/// THE PAGE'S PATH: `rules::statement::build` over what the page's `statement` service reads (as core
/// `statement_build` calls it: each read fresh, the previous statement's lines from its Block contracts, every audit
/// sealed in the call), each read asked answered from `w.got` (not there: empty) — and the root of its tree.
fn page_root(w: &World) -> ([u8; 32], usize) {
    let block_id = contract_keys::block::contract_deriver_of_hash(BLOCK);
    let mut g = w.got.clone();
    for _ in 0..16 {
        let gg = g.clone();
        let got = |x: &[u8; 32]| gg.get(x).cloned();
        let fresh = |_: &[u8; 32]| true;
        let lines = |root: &[u8; 32]| R::statement::lines(root, &|c| got(&block_id(c)).filter(|s| s.len() > 1).map(|s| s[1..].to_vec())).map_err(|cids| cids.iter().map(&block_id).collect());
        let i = craftworks_rules::statement::Inputs { epoch: E, items: &w.items, keepers: &w.keepers, queued: &[], posted: &w.posted_before, earlier: &[], ctx: &w.ctx, lot: &w.tl.lot, keep: &w.keep, fresh: &fresh, lines: &lines, audit: None };
        match craftworks_rules::statement::build(&i, &got).unwrap() {
            Step::Done((st, kept)) => {
                assert!(st.lines.contains_key(&[0x55; 32]), "epoch 4's accepted lines carried (read from its Block contracts)");
                return (st.tree(&mut BTreeMap::new()).unwrap().0, kept);
            }
            Step::Want(want) => want.into_iter().for_each(|x| {
                g.entry(x).or_default();
            }),
            Step::Sign(_) => panic!("a build never signs"),
        }
    }
    panic!("the page's build never ended")
}

/// One wake-up's round driven to where it stops (its reads answered from `got`): every PUT and UPDATE it sent.
fn wake(h: &mut Map, got: &HashMap<[u8; 32], Vec<u8>>, n: u64) -> Vec<Io> {
    h.set_secret(identity::UPKEEP_WAKEUPS, &(crate::upkeep::PAGE_AWAY + n).to_le_bytes());
    let mut sent = Vec::new();
    let mut todo = witness_round::woke(h);
    let mut calls = 0;
    while let Some(io) = (!todo.is_empty()).then(|| todo.remove(0)) {
        calls += 1;
        assert!(calls < 10_000, "a round that never ends");
        match io {
            Io::Get { id, .. } => todo.extend(witness_round::replied(h, &Reply::Got { id, state: got.get(&id).cloned() })),
            io => sent.push(io),
        }
    }
    sent
}

/// Staker 2 (seated: six stakers, six seats) on this node, its page handing the mandate over and leaving.
fn member_node(w: &World) -> (Map, SigningKey) {
    let seed = [112u8; 32];
    let key = SigningKey::from_bytes(&seed);
    assert_eq!(key.verifying_key().to_bytes(), w.tl.keys[2].verifying_key().to_bytes());
    let mut h = Map::default();
    assert!(matches!(identity::serve(&mut h, Request::Provision { seed, did: [0x22; 32], pin: "246810".into(), data: vec![] }, APP), Answer::Unlocked { .. }));
    let mandate = WitnessMandate { token: w.ctx.code_hash, idlog: w.ctx.idlog, bag: w.tl.lot.bag, beacon_code: w.tl.lot.beacon_code, beacon: w.ctx.beacon, pool: w.tl.lot.pool, standing: STANDING, block: BLOCK };
    assert_eq!(identity::serve(&mut h, Request::WitnessMandate { mandate }, APP), Answer::WitnessMandated);
    (h, key)
}

/// Epoch 5's statement container's updates among `sent`.
fn statement_updates(w: &World, sent: &[Io]) -> Vec<Vec<u8>> {
    let at = craftworks_rules::witness::statement_at(&w.ctx, &w.tl.lot, E);
    sent.iter().filter_map(|io| match io {
        Io::Update { id, delta } if *id == at.id() => Some(delta.clone()),
        _ => None,
    }).collect()
}

#[test]
fn a_seated_member_away_from_their_page_signs_the_statement_root_the_page_builds() {
    let mut w = world();
    let (root, kept) = page_root(&w);
    assert_eq!(kept, 1, "the page's build: the honest keeper alone kept");
    bag_put(&mut w.got, &w.tl.lot.bag, R::statement::bag_address(E), &[R::statement::Posted { epoch: E, root, parity: vec![] }.encode()]);
    let (mut h, key) = member_node(&w);
    let said = |h: &Map| -> Vec<String> { h.0.iter().filter(|(k, _)| k.starts_with(b"identity_upkeep/said")).map(|(_, v)| String::from_utf8_lossy(v).into()).collect() };
    // WAKE-UP 1: every read in, one keeper's audits sealed (AUDITS_PER_CALL) — the build pauses, nothing signed.
    assert_eq!(crate::witness_statement::AUDITS_PER_CALL, 1);
    let first = wake(&mut h, &w.got, 1);
    assert!(statement_updates(&w, &first).is_empty(), "paused before its second keeper's seals: {first:?}; said {:?}", said(&h));
    // WAKE-UP 2: the round resumed with what it read, the last audit sealed, the root built and signed.
    let second = wake(&mut h, &w.got, 2);
    assert!(second.iter().all(|io| !matches!(io, Io::Get { .. })), "resumed without reading again");
    let [delta] = &statement_updates(&w, &second)[..] else { panic!("one statement signature: {second:?}; said {:?}", said(&h)) };
    let at = craftworks_rules::witness::statement_at(&w.ctx, &w.tl.lot, E);
    let held = t::witness::Signatures::decode(delta).expect("a container's state");
    assert!(t::witness::holds(&at, &held).is_ok() && held.items.len() == 1 && held.items[0].key == key.verifying_key().to_bytes(), "the member's one signature");
    let sig = at.sigs(delta).remove(0);
    assert_eq!((sig.chain, sig.seq, sig.entry), (R::statement::witness_place(E), 0, root), "the page's root, at the statement's place");
    key.verifying_key().verify(&identity::witness_message(&sig.chain, 0, &root, sig.at), &Signature::from_bytes(&sig.sig)).expect("signed by the member");
    // SETTLED: a later wake-up signs nothing more.
    assert!(statement_updates(&w, &wake(&mut h, &w.got, 3)).is_empty(), "a statement witnessed once");
}

/// THE NEGATIVE CONTROL: the root posted for epoch 5 is not the one the delegate builds — nothing signed, and said why.
#[test]
fn a_posted_root_the_delegate_does_not_build_is_not_signed() {
    let mut w = world();
    let (root, _) = page_root(&w);
    let other = [0xAB; 32];
    assert_ne!(root, other);
    bag_put(&mut w.got, &w.tl.lot.bag, R::statement::bag_address(E), &[R::statement::Posted { epoch: E, root: other, parity: vec![] }.encode()]);
    let (mut h, _) = member_node(&w);
    let mut sent = Vec::new();
    for n in 1..=4 {
        sent.extend(wake(&mut h, &w.got, n));
    }
    assert!(statement_updates(&w, &sent).is_empty(), "a root it did not build is never signed: {sent:?}");
    let said: Vec<String> = h.0.iter().filter(|(k, _)| k.starts_with(b"identity_upkeep/said")).map(|(_, v)| String::from_utf8_lossy(v).into()).collect();
    assert!(said.iter().any(|s| s.contains("statement of epoch 5") && s.contains("not signed")), "said: {said:?}");
}
