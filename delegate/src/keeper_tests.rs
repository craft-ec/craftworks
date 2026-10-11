//! THE KEEPER ROUND with no page open, against a scripted network: a node's standing record opened by its page for
//! epoch 3 over 40 hosted pieces, the beacon past a height of epoch 3 sampled for it (its outputs in the beacon's log).
//! The page hands the keeper mandate over and leaves; upkeep seals one asked piece a wake-up and, once all are, puts
//! the answer as an UPDATE of the record — one the standing rules hold, its picks checking against the real seals.

use crate::testhost::Map;
use crate::upkeep::{Io, Reply};
use crate::{keeper_round, reads};
use craftworks_identity::{self as identity, Answer, Host, KeeperMandate, Request};
use craftworks_rules::keeper::{self, Keep};
use craftworks_rules::tokens::Step;
use craftworks_standing::{codec, record};
use ed25519_dalek::{Signer as _, SigningKey};
use std::collections::HashMap;

const APP: [u8; 32] = [0xA2; 32];

#[test]
fn an_away_page_s_node_answers_its_sampled_height_one_seal_a_wake_up() {
    let k = Keep { standing: [31; 32], beacon_code: [32; 32], beacon: [33; 32] };
    let seed = [44u8; 32];
    let key = SigningKey::from_bytes(&seed);
    let public = key.verifying_key().to_bytes();
    let peer = craftworks_standing::keeper_peer(&public);
    let piece = |id: &[u8; 32]| [id.to_vec(), vec![7u8; 100]].concat();
    let hosted: Vec<([u8; 32], u64)> = (0..40u32).map(|i| (*blake3::hash(&i.to_le_bytes()).as_bytes(), 132)).collect();
    let rid = keeper::record_id(&k, &public);
    let out = |h: u64| *blake3::hash(&h.to_le_bytes()).as_bytes();
    let mut got: HashMap<[u8; 32], Vec<u8>> = HashMap::new();
    // The PAGE opened epoch 3 (binding and holdings).
    got.insert(rid, vec![]);
    let mut sigs = Vec::new();
    let opened = loop {
        let g = got.clone();
        match keeper::open_epoch(&k, &|x| g.get(x).cloned(), &public, 3, &hosted, &sigs).unwrap() {
            Step::Sign(m) => sigs.push((m.clone(), key.sign(&m).to_bytes())),
            Step::Done(p) => break p.unwrap(),
            Step::Want(_) => panic!("a want"),
        }
    };
    got.insert(rid, opened.state);
    // The BEACON: a step at `top`, past the first sampled height of epoch 3; the outputs before it in its log.
    let e3 = 3 * craftworks_standing::EPOCH;
    let first = (e3..e3 + craftworks_standing::EPOCH).find(|h| craftworks_standing::sampled(&out(*h), &peer)).expect("a sampled height");
    let top = first + 1;
    let bp = craftworks_beacon_contract::Params { k: 16, segs: 1, sample: 1, window: 2, keep: 600, seed: [5; 32] };
    let st = craftworks_beacon_contract::State { steps: vec![craftworks_beacon_contract::Step::compute(&bp, [0; 32], top, [1; 32])], alts: vec![], frauds: vec![] };
    got.insert(k.beacon, craftworks_beacon_contract::wire::encode(&st));
    let lp = craftworks_beacon_contract::log::Params::of(k.beacon, first);
    let span = craftworks_beacon_contract::log::SPAN;
    let entries = (lp.index * span..top).filter(|h| *h > 0).map(|h| (h, out(h))).collect();
    got.insert(craftworks_token_contract::id(&k.beacon_code, &lp.encode()), craftworks_beacon_contract::log::Log { entries }.encode());
    for (id, _) in &hosted {
        got.insert(*id, piece(id));
    }
    // ITS NODE: the member provisioned; its page hands the keeper mandate over, then goes away.
    let mut h = Map::default();
    assert!(matches!(identity::serve(&mut h, Request::Provision { seed, did: [0x44; 32], pin: "246810".into(), data: vec![] }, APP), Answer::Unlocked { .. }));
    // (Listed already in epoch 3's keepers registry; not the seat node: this test is the answers'.)
    let bag_code = [34u8; 32];
    let listing = craftworks_reputation::keepers::Keeper { did: [0x44; 32], cred: vec![1, 2, 3], key_seq: 0 }.encode();
    let ka = craftworks_reputation::keepers::address(3, 0);
    got.insert(craftworks_token_contract::id(&bag_code, &ka), craftworks_bag_contract::encode(&ka, &[craftworks_bag_contract::grind(&ka, &listing)]));
    let mandate = KeeperMandate { standing: k.standing, beacon_code: k.beacon_code, beacon: k.beacon, epoch: 3, hosted: hosted.clone(), did: [0x44; 32], credential: vec![1, 2, 3], key_seq: 0, token: [35; 32], idlog: [36; 32], bag: bag_code, seat: false };
    assert_eq!(identity::serve(&mut h, Request::KeeperMandate { mandate }, APP), Answer::KeeperMandated);
    let wake = |h: &mut Map, n: u64, got: &HashMap<[u8; 32], Vec<u8>>| -> Vec<Io> {
        h.set_secret(identity::UPKEEP_WAKEUPS, &n.to_le_bytes());
        let mut sent = Vec::new();
        let mut todo = keeper_round::woke(h);
        while let Some(io) = (!todo.is_empty()).then(|| todo.remove(0)) {
            match io {
                Io::Get { id, .. } => todo.extend(keeper_round::replied(h, &Reply::Got { id, state: got.get(&id).cloned() })),
                other => sent.push(other),
            }
        }
        assert!(!reads::busy(h, keeper_round::ROUND), "the round ends each wake-up");
        sent
    };
    assert!(wake(&mut h, 0, &got).is_empty(), "the page open: no round");
    // AWAY: a piece sealed each wake-up, nothing put until every pick is; then the answer.
    let picks = keeper::asked(&k, &|x| got.get(x).cloned(), &public, first, &out(first), &hosted).unwrap();
    let Step::Done(Some(ids)) = picks else { panic!("asked") };
    let distinct = ids.iter().collect::<std::collections::BTreeSet<_>>().len();
    let mut put = None;
    for n in 0..distinct as u64 {
        let sent = wake(&mut h, crate::upkeep::PAGE_AWAY + n, &got);
        let sealed = keeper_round::at(&h, &public).pending.map_or(0, |p| p.hashes.len());
        if n + 1 < distinct as u64 {
            assert!(sent.is_empty(), "nothing put before every pick is sealed");
            assert_eq!(sealed, n as usize + 1, "one seal a wake-up");
        } else {
            put = sent.into_iter().next();
        }
    }
    let Some(Io::Update { id, delta }) = put else { panic!("the answer put") };
    assert_eq!(id, rid);
    let rec = codec::decode(&delta).unwrap();
    assert_eq!(record::holds(&peer, &rec, &|x| Some(out(x))), Ok(()));
    let e = rec.epochs.iter().find(|e| e.epoch == 3).unwrap();
    let a = e.answers.iter().find(|a| a.height == first).expect("its answer");
    assert_eq!(craftworks_standing::check(&e.holdings[0], a, &out(first), None, Some((0, &piece(&a.picks[0].id)))), Ok(()));
    assert_eq!(keeper_round::at(&h, &public).upto, first);
    // ANSWERED: the record with it, the next wake-up has nothing to answer.
    got.insert(rid, delta);
    assert!(wake(&mut h, crate::upkeep::PAGE_AWAY + 100, &got).is_empty(), "answered once");
}

/// LISTED WITH NO PAGE OPEN (audit 10-11): a node whose page is away lists its account in the epoch's keepers registry
/// (the page did, only while open: a closed page's node went unpaid though its delegate answered) — once.
#[test]
fn an_away_page_s_node_lists_its_account_in_the_epoch_s_keepers_registry() {
    let k = Keep { standing: [31; 32], beacon_code: [32; 32], beacon: [33; 32] };
    let seed = [44u8; 32];
    let bag_code = [34u8; 32];
    let mut got: HashMap<[u8; 32], Vec<u8>> = HashMap::new();
    let bp = craftworks_beacon_contract::Params { k: 16, segs: 1, sample: 1, window: 2, keep: 600, seed: [5; 32] };
    let top = 3 * craftworks_standing::EPOCH + 1;
    got.insert(k.beacon, craftworks_beacon_contract::wire::encode(&craftworks_beacon_contract::State { steps: vec![craftworks_beacon_contract::Step::compute(&bp, [0; 32], top, [1; 32])], alts: vec![], frauds: vec![] }));
    let ka = craftworks_reputation::keepers::address(3, 0);
    let kid = craftworks_token_contract::id(&bag_code, &ka);
    got.insert(kid, craftworks_bag_contract::encode(&ka, &[]));
    let mut h = Map::default();
    assert!(matches!(identity::serve(&mut h, Request::Provision { seed, did: [0x44; 32], pin: "246810".into(), data: vec![] }, APP), Answer::Unlocked { .. }));
    let mandate = KeeperMandate { standing: k.standing, beacon_code: k.beacon_code, beacon: k.beacon, epoch: 3, hosted: vec![], did: [0x44; 32], credential: vec![1, 2, 3], key_seq: 0, token: [35; 32], idlog: [36; 32], bag: bag_code, seat: false };
    assert_eq!(identity::serve(&mut h, Request::KeeperMandate { mandate }, APP), Answer::KeeperMandated);
    let wake = |h: &mut Map, n: u64, got: &HashMap<[u8; 32], Vec<u8>>| -> Vec<Io> {
        h.set_secret(identity::UPKEEP_WAKEUPS, &n.to_le_bytes());
        let mut sent = Vec::new();
        let mut todo = keeper_round::woke(h);
        while let Some(io) = (!todo.is_empty()).then(|| todo.remove(0)) {
            match io {
                Io::Get { id, .. } => todo.extend(keeper_round::replied(h, &Reply::Got { id, state: got.get(&id).cloned() })),
                other => sent.push(other),
            }
        }
        sent
    };
    let listing = craftworks_reputation::keepers::Keeper { did: [0x44; 32], cred: vec![1, 2, 3], key_seq: 0 }.encode();
    let sent = wake(&mut h, crate::upkeep::PAGE_AWAY, &got);
    let [Io::Put { code: crate::upkeep::Code::Bag, params, state }] = &sent[..] else { panic!("its listing put: {sent:?}") };
    assert_eq!(params[..], ka[..]);
    let items = craftworks_bag_contract::read(&ka, state).unwrap();
    assert_eq!(craftworks_bag_contract::payload(&items[0]), &listing[..]);
    // The registry with it (the node merged the put): listed — nothing more.
    got.insert(kid, state.clone());
    assert!(wake(&mut h, crate::upkeep::PAGE_AWAY + 1, &got).is_empty(), "listed once");
}
