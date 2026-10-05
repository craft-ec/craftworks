//! A whole ADMISSION round against a scripted network, with the real pieces: an MLS space group made by its owner, an
//! asker with a real key log, card (key package, inbox key) and request, the tail and bag formats. The end of it is
//! checked the way the asker's page would: their inbox item opens with their key and its welcome joins the group.

use super::upkeep::{self, Code, Io, Reply};
use craftworks_data as data;
use craftworks_identity::{self as identity, Host, Mandate};
use ed25519_dalek::{Signer, SigningKey};
use std::collections::HashMap;

#[derive(Default)]
struct Map(HashMap<Vec<u8>, Vec<u8>>);
impl Host for Map {
    fn get_secret(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.0.get(key).cloned()
    }
    fn set_secret(&mut self, key: &[u8], value: &[u8]) -> bool {
        self.0.insert(key.to_vec(), value.to_vec());
        true
    }
}

const BAG: &[u8] = b"the bag contract's code";
const TAIL: &[u8] = b"the tail contract's code";
const IDLOG: &[u8] = b"the key log contract's code";
const SPACE: [u8; 32] = [0x5A; 32];
const CODE: &str = "ab12-cd34-ef56-7890";
const MEMBER: [u8; 32] = [0x77; 32];
const NOW: u64 = 1_800_000_000_000;

/// An account from its words' entropy: DID, data seed, key log state.
struct Person {
    did: [u8; 32],
    data: [u8; 32],
    log: Vec<u8>,
}
fn person(e: u8) -> Person {
    let entropy = [e; 16];
    let ev = craftworks_account::inception(&entropy).unwrap();
    let did = ev.id();
    let log = craftworks_idlog_contract::Log { events: vec![ev] };
    Person { did, data: craftworks_account::data_seed(&entropy).unwrap(), log: log.encode() }
}
fn member_of(p: &Person) -> craftworks_mls::SpaceMember {
    let seed = identity::space_member_seed(&p.data);
    let public = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
    craftworks_mls::SpaceMember::new(&seed, identity::space_member_credential(&p.did, &p.data, &public), &[]).unwrap()
}
/// A person's CARD (a public tail under their data key): their inbox key and a key package.
fn card(p: &Person, kp: &[u8]) -> Vec<u8> {
    let key = SigningKey::from_bytes(&p.data);
    let mut o = data::Open::new(TAIL, &key.verifying_key().to_bytes(), "card");
    o.public = true;
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    let (seq, h) = o
        .prepare(vec![
            tail::Op::Set { key: b"inbox".to_vec(), value: hex(&identity::inbox_public(&p.data)).into_bytes() },
            tail::Op::Set { key: b"kp".to_vec(), value: serde_json::to_vec(&vec![hex(kp)]).unwrap() },
        ])
        .unwrap();
    let p = craftec_register_contract::wire::Params::parse(&o.params).unwrap();
    let Some(data::Send::Put(state)) = o.commit(key.sign(&p.signed_message(false, seq, &h)).to_bytes()) else { panic!("a first write") };
    state
}
fn bag(address: [u8; 32], items: &[serde_json::Value]) -> Vec<u8> {
    let items: Vec<Vec<u8>> = items.iter().map(|i| craftworks_bag_contract::grind(&address, i.to_string().as_bytes())).collect();
    craftworks_bag_contract::encode(&address, &items)
}
fn id(code: &[u8], params: &[u8]) -> [u8; 32] {
    upkeep::id_of(&contract_keys::code_hash(code), params)
}

struct World {
    h: Map,
    owner: Person,
    asker: Person,
    asker_member: craftworks_mls::SpaceMember,
    kp: Vec<u8>,
    epoch: u64,
}
/// The owner's node, upkeep handed a mandate for SPACE (code CODE, one use); a page last ticked 5 wake-ups ago.
fn world() -> World {
    let owner = person(0x11);
    let asker = person(0x22);
    let mut g = member_of(&owner).create_space(SPACE).unwrap();
    let asker_member = member_of(&asker);
    let kp = asker_member.key_package().unwrap();
    let mut h = Map::default();
    h.set_secret(identity::UPKEEP_BAG, BAG);
    h.set_secret(identity::UPKEEP_TAIL, TAIL);
    h.set_secret(identity::UPKEEP_IDLOG, &contract_keys::code_hash(IDLOG));
    h.set_secret(identity::UPKEEP_WAKEUPS, &5u64.to_le_bytes());
    identity::upkeep_stir(&mut h, &[7; 32]);
    let epoch = g.epoch();
    assert!(identity::keep_epoch(&mut h, &MEMBER, Some(SPACE), epoch, &g.epoch_secret().unwrap()));
    let me = craftworks_account::did(&owner.did);
    let m = Mandate {
        space: SPACE,
        name: "Makers".into(),
        kind: "server".into(),
        owner: me.clone(),
        nonce: Some("n1".into()),
        channel: "space-channel".into(),
        open: false,
        codes: vec![(CODE.into(), 0, 1)],
        bans: vec![],
        members: vec![me.clone()],
        epoch,
        state: g.save().unwrap(),
    };
    // The owner's page handed the mandate over at wake-up 5; it is 10 now (the page is away).
    identity::upkeep_set_mandate(&mut h, &MEMBER, &me, &[m]);
    identity::upkeep_set_tick(&mut h, &MEMBER);
    h.set_secret(identity::UPKEEP_WAKEUPS, &10u64.to_le_bytes());
    World { h, owner, asker, asker_member, kp, epoch }
}
fn join(p: &Person) -> serde_json::Value {
    serde_json::json!({ "kind": "join", "did": craftworks_account::did(&p.did), "at": 1 })
}

/// Drive a round to its end: each ask answered as `net` says. Returns every ask made.
fn run(w: &mut World, net: &mut dyn FnMut(&Io) -> Reply) -> Vec<Io> {
    let mut asked = Vec::new();
    let mut todo = upkeep::woke(&mut w.h, NOW);
    while let Some(io) = (!todo.is_empty()).then(|| todo.remove(0)) {
        let reply = net(&io);
        asked.push(io);
        todo.extend(upkeep::replied(&mut w.h, reply, NOW));
    }
    asked
}

/// The network as it is: the request bag, the asker's key log and card; epoch logs absent until put; every write taken.
fn network<'a>(w: &'a World, requests: Vec<serde_json::Value>) -> impl FnMut(&Io) -> Reply + 'a {
    let code_bag = id(BAG, &identity::invite_address(CODE));
    let log = id(IDLOG, &w.asker.did);
    let card_key = SigningKey::from_bytes(&w.asker.data).verifying_key().to_bytes();
    let card_id = data::Open::new(TAIL, &card_key, "card").id_bytes();
    let card_state = card(&w.asker, &w.kp);
    let bag_state = bag(identity::invite_address(CODE), &requests);
    move |io| match io {
        Io::Get { id, .. } if *id == code_bag => Reply::Got { id: *id, state: Some(bag_state.clone()) },
        Io::Get { id, .. } if *id == log => Reply::Got { id: *id, state: Some(w.asker.log.clone()) },
        Io::Get { id, .. } if *id == card_id => Reply::Got { id: *id, state: Some(card_state.clone()) },
        Io::Get { id, .. } => Reply::Got { id: *id, state: None },
        Io::Put { code: Code::Tail, params, .. } => Reply::Put { id: id(TAIL, params), ok: true },
        Io::Put { code: Code::Bag, params, .. } => Reply::Put { id: id(BAG, params), ok: true },
        Io::Put { code: Code::Sealed | Code::Piece, params, .. } => Reply::Put { id: id(b"sealed", params), ok: true },
        Io::Update { id, .. } => Reply::Updated { id: *id, ok: true },
    }
}

#[test]
fn an_asker_is_admitted_with_no_page_open_and_their_welcome_joins_them() {
    let mut w = world();
    let asker_did = craftworks_account::did(&w.asker.did);
    let reqs = vec![join(&w.asker)];
    let asked = {
        let w2 = &world_clone(&w);
        let mut net = network(w2, reqs);
        run(&mut w, &mut net)
    };
    // The commit into epoch e's log (absent here: a PUT), the next epoch's log, then the welcome.
    let puts: Vec<&Io> = asked.iter().filter(|x| matches!(x, Io::Put { .. } | Io::Update { .. })).collect();
    assert_eq!(puts.len(), 3, "commit, next log, welcome: {asked:?}");
    let Io::Put { code: Code::Bag, params, state } = puts[2] else { panic!("the welcome is last") };
    assert_eq!(params.as_slice(), identity::inbox_address(&w.asker.did));
    // THEIR PAGE: the inbox item opens with their inbox key, and its welcome joins them to the group.
    let items = craftworks_bag_contract::read(params, state).unwrap();
    let opened = identity::open_with(&identity::inbox_seed(&w.asker.data), craftworks_bag_contract::payload(&items[0])).unwrap();
    let item: serde_json::Value = serde_json::from_slice(&opened).unwrap();
    assert_eq!((item["kind"].as_str(), item["name"].as_str(), item["spaceKind"].as_str()), (Some("welcome"), Some("Makers"), Some("server")));
    assert_eq!(item["from"].as_str(), Some(craftworks_account::did(&w.owner.did).as_str()));
    assert_eq!(item["code"].as_str(), Some(CODE), "the welcome says which request it answers");
    let unhex = |s: &str| (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect::<Vec<u8>>();
    assert_eq!(unhex(item["space"].as_str().unwrap()), SPACE.to_vec());
    let joined = w.asker_member.join_space(SPACE, &unhex(item["welcome"].as_str().unwrap())).unwrap();
    assert_eq!(joined.epoch(), w.epoch + 1);
    // UPKEEP holds the group as it is now: the mandate's epoch and members, the code used up, the new epoch's secret
    // kept for the member, the admission for a page to write.
    let (_, spaces) = identity::upkeep_mandate(&w.h, &MEMBER).unwrap();
    assert_eq!(spaces[0].epoch, w.epoch + 1);
    assert!(spaces[0].members.contains(&asker_did));
    assert!(spaces[0].codes.is_empty(), "a one-use code is used up");
    assert_eq!(identity::epoch_secret(&w.h, &MEMBER, SPACE, w.epoch + 1), Some(joined.epoch_secret().unwrap()));
    let admitted = identity::upkeep_admitted(&w.h, &MEMBER);
    assert_eq!((admitted.len(), admitted[0].did.as_str(), admitted[0].code.as_str()), (1, asker_did.as_str(), CODE));
    assert!(identity::upkeep_moved(&w.h, &MEMBER, &SPACE));
    // The next round finds them in: nothing more is written.
    identity::Host::set_secret(&mut w.h, identity::UPKEEP_WAKEUPS, &20u64.to_le_bytes());
    let w2 = world_clone(&w);
    let reqs = vec![join(&w.asker)];
    let mut net = network(&w2, reqs);
    let again = run(&mut w, &mut net);
    assert!(again.iter().all(|x| matches!(x, Io::Get { .. })), "{again:?}");
}

#[test]
fn nobody_is_admitted_while_a_page_ticks_nor_the_banned_nor_the_members() {
    // A page handed the mandate over at this wake-up: upkeep stays out of its way.
    let mut w = world();
    identity::upkeep_set_tick(&mut w.h, &MEMBER);
    assert!(upkeep::woke(&mut w.h, NOW).is_empty());
    // Banned: read, not admitted.
    let mut w = world();
    let (me, mut spaces) = identity::upkeep_mandate(&w.h, &MEMBER).unwrap();
    spaces[0].bans.push(craftworks_account::did(&w.asker.did));
    identity::upkeep_set_mandate(&mut w.h, &MEMBER, &me, &spaces);
    let w2 = world_clone(&w);
    let reqs = vec![join(&w.asker)];
    let asked = run(&mut w, &mut network(&w2, reqs));
    assert_eq!(asked.len(), 1, "only the bag read: {asked:?}");
    assert!(identity::upkeep_admitted(&w.h, &MEMBER).is_empty());
    // An expired code's bag is not even read.
    let mut w = world();
    let (me, mut spaces) = identity::upkeep_mandate(&w.h, &MEMBER).unwrap();
    spaces[0].codes[0].1 = NOW - 1;
    identity::upkeep_set_mandate(&mut w.h, &MEMBER, &me, &spaces);
    assert!(upkeep::woke(&mut w.h, NOW).is_empty());
}

#[test]
fn a_commit_position_already_taken_stops_the_round_and_moves_nothing() {
    let mut w = world();
    let w2 = world_clone(&w);
    // Epoch e's log already holds a commit from e (another member moved the group first).
    let (_, spaces) = identity::upkeep_mandate(&w.h, &MEMBER).unwrap();
    let secret = identity::epoch_secret(&w.h, &MEMBER, SPACE, w.epoch).unwrap();
    let key = identity::epoch_log_key(&secret);
    let mut log = data::Open::new(TAIL, &key.verifying_key().to_bytes(), &spaces[0].channel);
    log.set_table_key(identity::epoch_table_key(&secret, &spaces[0].channel));
    let (seq, h) = log.prepare_row(format!("c/{:012}", w.epoch).as_bytes(), b"{\"commit\":\"00\"}").unwrap();
    let p = craftec_register_contract::wire::Params::parse(&log.params).unwrap();
    let Some(data::Send::Put(taken)) = log.commit(key.sign(&p.signed_message(false, seq, &h)).to_bytes()) else { panic!() };
    let log_id = log.id_bytes();
    let reqs = vec![join(&w.asker)];
    let mut inner = network(&w2, reqs);
    let asked = run(&mut w, &mut |io: &Io| match io {
        Io::Get { id, .. } if *id == log_id => Reply::Got { id: *id, state: Some(taken.clone()) },
        other => inner(other),
    });
    assert!(asked.iter().all(|x| matches!(x, Io::Get { .. })), "nothing written: {asked:?}");
    assert_eq!(identity::upkeep_mandate(&w.h, &MEMBER).unwrap().1[0].epoch, w.epoch, "the group did not move here");
    assert!(identity::upkeep_admitted(&w.h, &MEMBER).is_empty());
}

/// The network's view needs the people, not upkeep's secrets.
fn world_clone(w: &World) -> World {
    World {
        h: Map::default(),
        owner: Person { did: w.owner.did, data: w.owner.data, log: w.owner.log.clone() },
        asker: Person { did: w.asker.did, data: w.asker.data, log: w.asker.log.clone() },
        asker_member: member_of(&w.asker),
        kp: w.kp.clone(),
        epoch: w.epoch,
    }
}

#[test]
fn on_a_shared_node_each_person_is_upkept_by_their_own_mandate() {
    // The owner's page is away; a second person on the same node has their page open with a mandate of their own
    // (another space, same code): only the owner's round runs, and it touches only the owner's records.
    let mut w = world();
    let other: [u8; 32] = [0x88; 32];
    let (_, spaces) = identity::upkeep_mandate(&w.h, &MEMBER).unwrap();
    let mut theirs = spaces[0].clone();
    theirs.space = [0x6B; 32];
    identity::upkeep_set_mandate(&mut w.h, &other, "did:other", &[theirs]);
    identity::upkeep_set_tick(&mut w.h, &other);
    // Both pages open: nobody's spaces are run.
    identity::upkeep_set_tick(&mut w.h, &MEMBER);
    assert!(upkeep::woke(&mut w.h, NOW).is_empty(), "each person's own page keeps upkeep away from their spaces");
    // The owner's page goes away (five wake-ups pass; the other person's page ticks on).
    identity::Host::set_secret(&mut w.h, identity::UPKEEP_WAKEUPS, &15u64.to_le_bytes());
    identity::upkeep_set_tick(&mut w.h, &other);
    let w2 = world_clone(&w);
    let reqs = vec![join(&w.asker)];
    run(&mut w, &mut network(&w2, reqs));
    assert_eq!(identity::upkeep_admitted(&w.h, &MEMBER).len(), 1);
    assert!(identity::upkeep_admitted(&w.h, &other).is_empty(), "their page is open: their spaces are theirs to run");
    assert_eq!(identity::upkeep_mandate(&w.h, &other).unwrap().1[0].epoch, w.epoch, "their group untouched");
    // Only their page being open keeps upkeep away: the owner's absence alone does not stop them being served later.
    assert_eq!(identity::upkeep_members(&w.h), vec![MEMBER, other]);
}
