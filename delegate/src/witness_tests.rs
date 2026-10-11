//! THE WITNESS ROUND with no page open, against a scripted network holding a real lot (`testlot`: six stakers, their
//! claims, a beacon past epoch 1) and a new token's first entry stamped in epoch 1, queued. The member drawn for it
//! hands a witness mandate over and leaves; upkeep signs the entry through the guard and drops it into its bag.

use crate::upkeep::{Code, Io, Reply};
use crate::witness_round;
use craftworks_identity::{self as identity, Answer, Host, Request, WitnessMandate};
use craftworks_rules::tokens::{testlot, Ctx};
use craftworks_token_contract as t;
use ed25519_dalek::{Signature, SigningKey, Verifier};
use std::collections::HashMap;

use crate::testhost::Map;

const APP: [u8; 32] = [0xA1; 32];

/// Drive wake-ups' rounds to their end against `got`: every PUT returned.
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
    let mandate = WitnessMandate { token: ctx.code_hash, idlog: ctx.idlog, bag: tl.lot.bag, beacon_code: tl.lot.beacon_code, beacon: ctx.beacon, pool: tl.lot.pool };
    assert_eq!(identity::serve(&mut h, Request::WitnessMandate { mandate }, APP), Answer::WitnessMandated);
    // The page still ticking: no round (it witnesses for itself).
    assert!(run(&mut h, &got).is_empty(), "no round while the page is open");
    // The page away: the round signs the entry into its witness bag.
    h.set_secret(identity::UPKEEP_WAKEUPS, &(crate::upkeep::PAGE_AWAY + 1).to_le_bytes());
    let puts = run(&mut h, &got);
    let bag = t::wallet::witness_bag(&chain, q.seq);
    let [Io::Put { code: Code::Bag, params, state }] = &puts[..] else {
        let said: Vec<String> = h.0.iter().filter(|(k, _)| k.starts_with(b"identity_upkeep/said")).map(|(_, v)| String::from_utf8_lossy(v).into()).collect();
        panic!("one witness bag put: {puts:?}; said {said:?}")
    };
    assert_eq!(params[..], bag[..]);
    let items = craftworks_bag_contract::read(&bag, state).expect("a bag state");
    let sig = t::wallet::bytes::read_witness_sig(craftworks_bag_contract::payload(&items[0])).expect("a witness signature");
    assert_eq!((sig.chain, sig.seq, sig.entry), (chain, q.seq, entry));
    key.verifying_key().verify(&identity::witness_message(&chain, q.seq, &entry), &Signature::from_bytes(&sig.sig)).expect("signed by the member");
    // SETTLED: the next wake-up asks again but signs nothing more.
    h.set_secret(identity::UPKEEP_WAKEUPS, &(crate::upkeep::PAGE_AWAY + 2).to_le_bytes());
    assert!(run(&mut h, &got).is_empty(), "an entry witnessed once");
}
