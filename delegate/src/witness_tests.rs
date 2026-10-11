//! THE WITNESS ROUND with no page open, against a scripted network holding a real lot (`testlot`: six stakers, their
//! claims, a beacon past epoch 1) and a new token's first entry stamped in epoch 1, queued. The member drawn for it
//! hands a witness mandate over and leaves; upkeep signs the entry through the guard and sends it to its place's
//! signatures container (`witness::Params`).

use crate::upkeep::{Io, Reply};
use crate::witness_round;
use craftworks_identity::{self as identity, Answer, Host, Request, WitnessMandate};
use craftworks_rules::tokens::{testlot, Ctx};
use craftworks_token_contract as t;
use ed25519_dalek::{Signature, SigningKey, Verifier};
use std::collections::HashMap;

use crate::testhost::Map;

const APP: [u8; 32] = [0xA1; 32];

/// The signatures container of the genesis entry `q` (of chain `chain`, as `got` holds it): under its lot's claims epoch.
fn container(ctx: &Ctx, tl: &testlot::TestLot, got: &HashMap<[u8; 32], Vec<u8>>, chain: &[u8; 32], seq: u64) -> t::witness::Params {
    let s = t::State::decode(&got[chain]).unwrap();
    let (epoch, _) = t::wallet::lot_of(&s.entries[0], None, 2).unwrap();
    craftworks_rules::tokens::witness_at(ctx.code_hash, &tl.lot, ctx.beacon, chain, seq, epoch)
}

/// Drive wake-ups' rounds to their end against `got`: every PUT and UPDATE returned.
fn run(h: &mut Map, got: &HashMap<[u8; 32], Vec<u8>>) -> Vec<Io> {
    let mut puts = Vec::new();
    let mut todo = witness_round::woke(h);
    let mut n = 0;
    while let Some(io) = (!todo.is_empty()).then(|| todo.remove(0)) {
        n += 1;
        assert!(n < 10_000, "a round that never ends");
        match io {
            Io::Get { id, .. } => todo.extend(witness_round::replied(h, &Reply::Got { id, state: got.get(&id).cloned() })),
            put => puts.push(put),
        }
    }
    puts
}

#[test]
fn a_drawn_member_away_from_their_page_witnesses_a_queued_entry_once() {
    let ctx = Ctx { code_hash: [9; 32], beacon: [7; 32], idlog: [6; 32] };
    let mut got = HashMap::new();
    let tl = testlot::stakers(&ctx, &mut got);
    let (q, chain, entry) = testlot::genesis_in_epoch1(&ctx, &mut got);
    // The BEACON just past the entry (fresh: a witness signs only lately stamped entries; the lot's log stays).
    let height = testlot::E1 + 20;
    let bp = craftworks_beacon_contract::Params { k: 16, segs: 1, sample: 1, window: 2, keep: 600, seed: [5; 32] };
    let top = craftworks_beacon_contract::State { steps: vec![craftworks_beacon_contract::Step::compute(&bp, [0; 32], height, [1; 32])], alts: vec![], frauds: vec![] };
    got.insert(ctx.beacon, craftworks_beacon_contract::wire::encode(&top));
    // QUEUED in the witness queue of the beacon's standing epoch.
    let address = craftworks_wallet::queue::address(height / craftworks_reputation::EPOCH, 0);
    let item = q.encode();
    got.insert(t::id(&tl.lot.bag, &address), craftworks_bag_contract::encode(&address, &[craftworks_bag_contract::grind(&address, &item)]));
    // STAKER 2 (seated: six stakers, six seats) on this node: provisioned, their page hands the mandate over.
    let seed = [112u8; 32];
    let key = SigningKey::from_bytes(&seed);
    let member = key.verifying_key().to_bytes();
    assert_eq!(member, tl.keys[2].verifying_key().to_bytes());
    let mut h = Map::default();
    let did = [0x22; 32];
    assert!(matches!(identity::serve(&mut h, Request::Provision { seed, did, pin: "246810".into(), data: vec![] }, APP), Answer::Unlocked { .. }));
    let mandate = WitnessMandate { token: ctx.code_hash, idlog: ctx.idlog, bag: tl.lot.bag, beacon_code: tl.lot.beacon_code, beacon: ctx.beacon, pool: tl.lot.pool, standing: [43; 32], block: [44; 32] };
    assert_eq!(identity::serve(&mut h, Request::WitnessMandate { mandate }, APP), Answer::WitnessMandated);
    // The page still ticking: no round (it witnesses for itself).
    assert!(run(&mut h, &got).is_empty(), "no round while the page is open");
    // The page away: the round signs the entry into its place's signatures container.
    h.set_secret(identity::UPKEEP_WAKEUPS, &(crate::upkeep::PAGE_AWAY + 1).to_le_bytes());
    let puts = run(&mut h, &got);
    let wp = container(&ctx, &tl, &got, &chain, q.seq);
    let [Io::Update { id, delta }] = &puts[..] else {
        let said: Vec<String> = h.0.iter().filter(|(k, _)| k.starts_with(b"identity_upkeep/said")).map(|(_, v)| String::from_utf8_lossy(v).into()).collect();
        panic!("one witness update: {puts:?}; said {said:?}")
    };
    assert_eq!(*id, wp.id());
    let held = t::witness::Signatures::decode(delta).expect("a container's state");
    assert!(t::witness::holds(&wp, &held).is_ok() && held.items.len() == 1 && held.items[0].key == member, "the member's one signature");
    let sig = wp.sigs(delta).remove(0);
    assert_eq!((sig.chain, sig.seq, sig.entry), (chain, q.seq, entry));
    key.verifying_key().verify(&identity::witness_message(&chain, q.seq, &entry, sig.at), &Signature::from_bytes(&sig.sig)).expect("signed by the member");
    // SETTLED: the next wake-up asks again but signs nothing more.
    h.set_secret(identity::UPKEEP_WAKEUPS, &(crate::upkeep::PAGE_AWAY + 2).to_le_bytes());
    assert!(run(&mut h, &got).is_empty(), "an entry witnessed once");
}

/// QUEUED BEFORE ITS LOT (audit 10-11): an entry read while its lot is not drawn yet is NOT settled — once the beacon
/// passes it, the same member signs it (it was settled for good, and never witnessed by a closed page).
#[test]
fn an_entry_queued_before_its_lot_is_drawn_is_signed_once_it_is() {
    let ctx = Ctx { code_hash: [9; 32], beacon: [7; 32], idlog: [6; 32] };
    let mut got = HashMap::new();
    let tl = testlot::stakers(&ctx, &mut got);
    let (q, chain, _) = testlot::genesis_in_epoch1(&ctx, &mut got);
    let bp = craftworks_beacon_contract::Params { k: 16, segs: 1, sample: 1, window: 2, keep: 600, seed: [5; 32] };
    let at = |height: u64| craftworks_beacon_contract::wire::encode(&craftworks_beacon_contract::State { steps: vec![craftworks_beacon_contract::Step::compute(&bp, [0; 32], height, [1; 32])], alts: vec![], frauds: vec![] });
    // The beacon BEFORE the genesis's lot (the epoch's first step).
    got.insert(ctx.beacon, at(testlot::E1));
    let address = craftworks_wallet::queue::address(testlot::E1 / craftworks_reputation::EPOCH, 0);
    let item = q.encode();
    got.insert(t::id(&tl.lot.bag, &address), craftworks_bag_contract::encode(&address, &[craftworks_bag_contract::grind(&address, &item)]));
    let seed = [112u8; 32];
    let mut h = Map::default();
    assert!(matches!(identity::serve(&mut h, Request::Provision { seed, did: [0x22; 32], pin: "246810".into(), data: vec![] }, APP), Answer::Unlocked { .. }));
    let mandate = WitnessMandate { token: ctx.code_hash, idlog: ctx.idlog, bag: tl.lot.bag, beacon_code: tl.lot.beacon_code, beacon: ctx.beacon, pool: tl.lot.pool, standing: [43; 32], block: [44; 32] };
    assert_eq!(identity::serve(&mut h, Request::WitnessMandate { mandate }, APP), Answer::WitnessMandated);
    h.set_secret(identity::UPKEEP_WAKEUPS, &(crate::upkeep::PAGE_AWAY + 1).to_le_bytes());
    assert!(run(&mut h, &got).is_empty(), "nothing to sign before its lot");
    // The beacon PAST it: signed.
    got.insert(ctx.beacon, at(testlot::E1 + 20));
    h.set_secret(identity::UPKEEP_WAKEUPS, &(crate::upkeep::PAGE_AWAY + 2).to_le_bytes());
    let puts = run(&mut h, &got);
    let wp = container(&ctx, &tl, &got, &chain, q.seq);
    assert!(matches!(&puts[..], [Io::Update { id, .. }] if *id == wp.id()), "signed once its lot is drawn: {puts:?}");
}
