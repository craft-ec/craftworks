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
    card_with(p, kp, None)
}
/// …with running contributions (`contrib`, JSON): what a level group checks.
fn card_with(p: &Person, kp: &[u8], contrib: Option<&str>) -> Vec<u8> {
    let key = SigningKey::from_bytes(&p.data);
    let mut o = data::Open::new(TAIL, &key.verifying_key().to_bytes(), "card");
    o.public = true;
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    let (seq, h) = o
        .prepare(vec![
            tail::Op::Set { key: b"inbox".to_vec(), value: hex(&identity::inbox_public(&p.data)).into_bytes() },
            tail::Op::Set { key: b"kp".to_vec(), value: serde_json::to_vec(&vec![hex(kp)]).unwrap() },
        ]
        .into_iter()
        .chain(contrib.map(|c| tail::Op::Set { key: b"contrib".to_vec(), value: c.as_bytes().to_vec() }))
        .collect())
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
    h.set_secret(identity::UPKEEP_BLOCK, &contract_keys::code_hash(b"the block contract's code"));
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
        level: None,
        bans: vec![],
        members: vec![me.clone()],
        epoch,
        state: g.save().unwrap(),
        admitters: vec![me.clone()],
    };
    // The owner's page handed the mandate over at wake-up 5; it is 10 now (the page is away).
    identity::upkeep_set_mandate(&mut h, &MEMBER, &me, &[m]);
    identity::upkeep_set_tick(&mut h, &MEMBER);
    h.set_secret(identity::UPKEEP_WAKEUPS, &10u64.to_le_bytes());
    World { h, owner, asker, asker_member, kp, epoch }
}
/// A join request: it carries the key package its device made for it (`rules::admission`).
fn join(p: &Person, kp: &[u8]) -> serde_json::Value {
    let hex: String = kp.iter().map(|x| format!("{x:02x}")).collect();
    serde_json::json!({ "kind": "join", "did": craftworks_account::did(&p.did), "at": 1, "kp": hex })
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
    let reqs = vec![join(&w.asker, &w.kp)];
    let asked = {
        let w2 = &world_clone(&w);
        let mut net = network(w2, reqs);
        run(&mut w, &mut net)
    };
    // The commit into epoch e's log (absent here: a PUT), the next epoch's log, then the welcome.
    let puts: Vec<&Io> = asked.iter().filter(|x| matches!(x, Io::Put { .. } | Io::Update { .. })).collect();
    assert_eq!(puts.len(), 4, "commit, next log, welcome, its record: {asked:?}");
    let Io::Put { code: Code::Bag, params, state } = puts[2] else { panic!("the welcome third") };
    assert_eq!(params.as_slice(), identity::inbox_address(&w.asker.did));
    // ITS RECORD in the group's shared list (the space's writers list, as a page writes it): who, with which key.
    let Io::Put { code: Code::Bag, params: wp, state: ws } = puts[3] else { panic!("the record last") };
    assert_eq!(wp.as_slice(), crate::table::sealed_bag_address(&SPACE, "writers"));
    let rec = crate::table::open_bag_item(&SPACE, &Default::default(), "writers", craftworks_bag_contract::payload(&craftworks_bag_contract::read(wp, ws).unwrap()[0])).unwrap();
    let hexs = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    assert_eq!((rec["welcomed"].as_str(), rec["kp"].as_str()), (Some(asker_did.as_str()), Some(hexs(&identity::kp_tag(&hexs(&w.kp))).as_str())));
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
    let reqs = vec![join(&w.asker, &w.kp)];
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
    let reqs = vec![join(&w.asker, &w.kp)];
    let asked = run(&mut w, &mut network(&w2, reqs));
    assert_eq!(asked.len(), 3, "only the bags read (the code's, the space's member bag, its writers list): {asked:?}");
    assert!(identity::upkeep_admitted(&w.h, &MEMBER).is_empty());
    // An expired code's bag is not even read (the space's stuck bag only).
    let mut w = world();
    let (me, mut spaces) = identity::upkeep_mandate(&w.h, &MEMBER).unwrap();
    spaces[0].codes[0].1 = NOW - 1;
    identity::upkeep_set_mandate(&mut w.h, &MEMBER, &me, &spaces);
    let code_bag = id(BAG, &identity::invite_address(CODE));
    let gets = upkeep::woke(&mut w.h, NOW);
    assert_eq!(gets.len(), 2, "the member bag and the writers list: {gets:?}");
    assert!(!gets.iter().any(|x| matches!(x, Io::Get { id, .. } if *id == code_bag)));
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
    // (A PAGE's row: under its own tag, `rules::ordering`.)
    let (seq, h) = log.prepare_row(craftworks_rules::ordering::row_key("c/", w.epoch, "a1b2c3d4e5f6").as_bytes(), b"{\"commit\":\"00\"}").unwrap();
    let p = craftec_register_contract::wire::Params::parse(&log.params).unwrap();
    let Some(data::Send::Put(taken)) = log.commit(key.sign(&p.signed_message(false, seq, &h)).to_bytes()) else { panic!() };
    let log_id = log.id_bytes();
    let reqs = vec![join(&w.asker, &w.kp)];
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
    let reqs = vec![join(&w.asker, &w.kp)];
    run(&mut w, &mut network(&w2, reqs));
    assert_eq!(identity::upkeep_admitted(&w.h, &MEMBER).len(), 1);
    assert!(identity::upkeep_admitted(&w.h, &other).is_empty(), "their page is open: their spaces are theirs to run");
    assert_eq!(identity::upkeep_mandate(&w.h, &other).unwrap().1[0].epoch, w.epoch, "their group untouched");
    // Only their page being open keeps upkeep away: the owner's absence alone does not stop them being served later.
    assert_eq!(identity::upkeep_members(&w.h), vec![MEMBER, other]);
}

/// A LEVEL GROUP (rewards §5a): the asker at `level <space>` is let in only while their card's contributions reach its
/// level — gov's one rule; a free asker is not, a PRO one is, and the admission says `level`.
#[test]
fn a_level_group_lets_in_only_whom_their_card_shows_at_its_level() {
    let run_with = |contrib: &str| {
        let mut w = world();
        let (me, mut spaces) = identity::upkeep_mandate(&w.h, &MEMBER).unwrap();
        spaces[0].codes.clear();
        spaces[0].level = Some("pro".into());
        identity::upkeep_set_mandate(&mut w.h, &MEMBER, &me, &spaces);
        let w2 = world_clone(&w);
        let level_bag = id(BAG, &identity::invite_address(&format!("level {}", SPACE.iter().map(|x| format!("{x:02x}")).collect::<String>())));
        let log = id(IDLOG, &w2.asker.did);
        let card_id = data::Open::new(TAIL, &SigningKey::from_bytes(&w2.asker.data).verifying_key().to_bytes(), "card").id_bytes();
        let card_state = card_with(&w2.asker, &w2.kp, Some(contrib));
        let name = format!("level {}", SPACE.iter().map(|x| format!("{x:02x}")).collect::<String>());
        let bag_state = bag(identity::invite_address(&name), &[join(&w2.asker, &w2.kp)]);
        let mut net = move |io: &Io| match io {
            Io::Get { id, .. } if *id == level_bag => Reply::Got { id: *id, state: Some(bag_state.clone()) },
            Io::Get { id, .. } if *id == log => Reply::Got { id: *id, state: Some(w2.asker.log.clone()) },
            Io::Get { id, .. } if *id == card_id => Reply::Got { id: *id, state: Some(card_state.clone()) },
            Io::Get { id, .. } => Reply::Got { id: *id, state: None },
            Io::Put { code: Code::Tail, params, .. } => Reply::Put { id: id(TAIL, params), ok: true },
            Io::Put { code: Code::Bag, params, .. } => Reply::Put { id: id(BAG, params), ok: true },
            Io::Put { code: Code::Sealed | Code::Piece, params, .. } => Reply::Put { id: id(b"sealed", params), ok: true },
            Io::Update { id, .. } => Reply::Updated { id: *id, ok: true },
        };
        let asked = run(&mut w, &mut net);
        (asked, identity::upkeep_admitted(&w.h, &MEMBER))
    };
    // FREE (1 a month): asked, card read, nothing written.
    let (asked, admitted) = run_with(&format!(r#"[{{"at":{NOW},"amount":1,"days":30}}]"#));
    assert!(asked.iter().all(|x| matches!(x, Io::Get { .. })), "a free asker is not let in: {asked:?}");
    assert!(admitted.is_empty());
    // PRO (6 a month): let in, by `level`.
    let (asked, admitted) = run_with(&format!(r#"[{{"at":{NOW},"amount":6,"days":30}}]"#));
    assert_eq!(asked.iter().filter(|x| !matches!(x, Io::Get { .. })).count(), 4, "commit, next log, welcome, its record: {asked:?}");
    assert_eq!((admitted.len(), admitted[0].code.as_str()), (1, "level"));
}

/// A MEMBER STUCK behind its own commit (its state after it never saved): it asks into the space's `stuck` bag, and
/// upkeep — no page open — takes its old entry out and adds it again in ONE commit; the new welcome joins it. Control:
/// an ask from an epoch the group is not past writes nothing.
#[test]
fn a_member_stuck_behind_its_own_commit_is_let_in_again_with_no_page_open() {
    let mut w = world();
    let asker_did = craftworks_account::did(&w.asker.did);
    // Let in first, the usual way.
    {
        let w2 = &world_clone(&w);
        let mut net = network(w2, vec![join(&w.asker, &w.kp)]);
        run(&mut w, &mut net);
    }
    let (_, spaces) = identity::upkeep_mandate(&w.h, &MEMBER).unwrap();
    let in_at = spaces[0].epoch;
    assert_eq!(in_at, w.epoch + 1);
    // Its card now offers a fresh key package (the used one is spent); its device is stuck at `w.epoch`.
    let kp2 = w.asker_member.key_package().unwrap();
    let stuck_bag = format!("stuck {}", SPACE.iter().map(|b| format!("{b:02x}")).collect::<String>());
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    let ask = |epoch: u64| serde_json::json!({ "kind": "stuck", "did": asker_did, "epoch": epoch, "seen": epoch + 1, "at": NOW + 1, "kp": hex(&kp2) });
    let net_for_kp = |w: &World, item: serde_json::Value, kp: &[u8]| {
        let bag_id = id(BAG, &identity::invite_address(&stuck_bag));
        let bag_state = bag(identity::invite_address(&stuck_bag), &[item]);
        let log = id(IDLOG, &w.asker.did);
        let card_key = SigningKey::from_bytes(&w.asker.data).verifying_key().to_bytes();
        let card_id = data::Open::new(TAIL, &card_key, "card").id_bytes();
        let card_state = card(&w.asker, kp);
        let log_state = w.asker.log.clone();
        move |io: &Io| match io {
            Io::Get { id, .. } if *id == bag_id => Reply::Got { id: *id, state: Some(bag_state.clone()) },
            Io::Get { id, .. } if *id == log => Reply::Got { id: *id, state: Some(log_state.clone()) },
            Io::Get { id, .. } if *id == card_id => Reply::Got { id: *id, state: Some(card_state.clone()) },
            Io::Get { id, .. } => Reply::Got { id: *id, state: None },
            Io::Put { code: Code::Tail, params, .. } => Reply::Put { id: id(TAIL, params), ok: true },
            Io::Put { code: Code::Bag, params, .. } => Reply::Put { id: id(BAG, params), ok: true },
            Io::Put { code: Code::Sealed | Code::Piece, params, .. } => Reply::Put { id: id(b"sealed", params), ok: true },
            Io::Update { id, .. } => Reply::Updated { id: *id, ok: true },
        }
    };
    let net_for = |w: &World, item: serde_json::Value| net_for_kp(w, item, &kp2);
    // CONTROL: an ask from the group's own epoch (not behind): nothing written.
    identity::Host::set_secret(&mut w.h, identity::UPKEEP_WAKEUPS, &20u64.to_le_bytes());
    let mut net = net_for(&w, ask(in_at));
    let asked = run(&mut w, &mut net);
    assert!(asked.iter().all(|x| matches!(x, Io::Get { .. })), "not behind: nothing written {asked:?}");
    // STUCK at the epoch before: let in again.
    identity::Host::set_secret(&mut w.h, identity::UPKEEP_WAKEUPS, &30u64.to_le_bytes());
    let mut net = net_for(&w, ask(w.epoch));
    let asked = run(&mut w, &mut net);
    let puts: Vec<&Io> = asked.iter().filter(|x| matches!(x, Io::Put { .. } | Io::Update { .. })).collect();
    assert_eq!(puts.len(), 4, "commit, next log, welcome, its record: {asked:?}");
    let Io::Put { code: Code::Bag, params, state } = puts[2] else { panic!("the welcome third") };
    let items = craftworks_bag_contract::read(params, state).unwrap();
    let opened = identity::open_with(&identity::inbox_seed(&w.asker.data), craftworks_bag_contract::payload(&items[0])).unwrap();
    let item: serde_json::Value = serde_json::from_slice(&opened).unwrap();
    // THE ONE WELCOME (`identity::welcome_message`, the page's format): an ask AGAIN answers no code; it names the key
    // its request carried, and hands the space's earlier epochs (its history).
    assert_eq!(item["code"].as_str(), None);
    assert_eq!(item["kp"].as_str(), Some(kp2.iter().map(|x| format!("{x:02x}")).collect::<String>()).map(|h| identity::kp_tag(&h)).map(|t| t.iter().map(|x| format!("{x:02x}")).collect::<String>()).as_deref());
    assert!(item["history"].as_array().is_some_and(|h| !h.is_empty()), "its history: {item}");
    let unhex = |s: &str| (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect::<Vec<u8>>();
    let joined = w.asker_member.join_space(SPACE, &unhex(item["welcome"].as_str().unwrap())).unwrap();
    assert_eq!(joined.epoch(), in_at + 1, "one commit: out and in together");
    // In the group ONCE (its old entry out), and once in the mandate's members.
    let did_bytes = craftworks_account::did_bytes(&asker_did).unwrap();
    let entries = joined.members().into_iter().filter(|(_, _, c)| c.get(4..36) == Some(did_bytes.as_slice())).count();
    println!("entries for the member after: {entries}");
    assert_eq!(entries, 1);
    let (_, spaces) = identity::upkeep_mandate(&w.h, &MEMBER).unwrap();
    assert_eq!(spaces[0].members.iter().filter(|d| **d == asker_did).count(), 1);
    // Asked again with the same ask: answered already, nothing written.
    identity::Host::set_secret(&mut w.h, identity::UPKEEP_WAKEUPS, &40u64.to_le_bytes());
    let mut net = net_for(&w, ask(w.epoch));
    let again = run(&mut w, &mut net);
    assert!(again.iter().all(|x| matches!(x, Io::Get { .. })), "{again:?}");
    // ONCE PER REQUEST (10-13: every member and every node re-added whoever looked missing, each a key and an epoch —
    // Ivvor's space at 478): a LATER ask with no key of its own, or with the key already used, writes nothing; the card's
    // fresh key package is never picked.
    let kp3 = w.asker_member.key_package().unwrap();
    for (n, kp) in [(50u64, None), (60, Some(hex(&kp2)))] {
        identity::Host::set_secret(&mut w.h, identity::UPKEEP_WAKEUPS, &n.to_le_bytes());
        let item = serde_json::json!({ "kind": "stuck", "did": asker_did, "epoch": w.epoch, "seen": w.epoch + 1, "at": NOW + 86_400_000, "kp": kp });
        let mut net = net_for_kp(&w, item, &kp3);
        let later = run(&mut w, &mut net);
        assert!(later.iter().all(|x| matches!(x, Io::Get { .. })), "a later ask, key {kp:?}: {later:?}");
    }
    // A NEW request with its own new key (its device still behind): let in, once.
    identity::Host::set_secret(&mut w.h, identity::UPKEEP_WAKEUPS, &70u64.to_le_bytes());
    let newer = serde_json::json!({ "kind": "stuck", "did": asker_did, "epoch": w.epoch, "seen": w.epoch + 1, "at": NOW + 86_400_000, "kp": hex(&kp3) });
    let mut net = net_for_kp(&w, newer.clone(), &kp3);
    let puts = run(&mut w, &mut net).into_iter().filter(|x| !matches!(x, Io::Get { .. })).count();
    assert_eq!(puts, 4, "a new request with its own key: let in");
    identity::Host::set_secret(&mut w.h, identity::UPKEEP_WAKEUPS, &80u64.to_le_bytes());
    let mut net = net_for_kp(&w, newer, &kp3);
    let again = run(&mut w, &mut net);
    assert!(again.iter().all(|x| matches!(x, Io::Get { .. })), "that request again: {again:?}");
}

/// A REQUEST NAMING SOMEONE ELSE: a request is an unsigned drop in a bag — one naming the asker's DID but carrying a key
/// package another account made is never let in (the welcome would let that other in under the asker's name).
/// Control: the same request with the asker's own key is.
#[test]
fn a_request_with_someone_elses_key_is_not_let_in() {
    let mut w = world();
    let thief = member_of(&person(0x33));
    let forged = thief.key_package().unwrap();
    let w2 = world_clone(&w);
    let mut net = network(&w2, vec![join(&w.asker, &forged)]);
    let asked = run(&mut w, &mut net);
    assert!(asked.iter().all(|x| matches!(x, Io::Get { .. })), "someone else's key: nothing written {asked:?}");
    // …and its STEP recorded for the page's `member` machine (so a health report says why).
    let d = identity::upkeep_decided(&w.h, &MEMBER);
    assert_eq!((d.len(), d[0].step.as_str()), (1, "skip"), "{d:?}");
    assert!(d[0].why.contains("not theirs"), "{d:?}");
    let mut w = world();
    let w2 = world_clone(&w);
    let mut net = network(&w2, vec![join(&w.asker, &w.kp)]);
    let asked = run(&mut w, &mut net);
    assert!(asked.iter().any(|x| matches!(x, Io::Put { .. } | Io::Update { .. })), "their own key: let in {asked:?}");
    let d = identity::upkeep_decided(&w.h, &MEMBER);
    assert_eq!(d.iter().map(|x| x.step.as_str()).collect::<Vec<_>>(), vec!["welcomed"], "{d:?}");
    assert_eq!(d[0].q, identity::kp_tag(&w.kp.iter().map(|x| format!("{x:02x}")).collect::<String>()), "keyed by the request's own key");
}

/// ONCE PER REQUEST ACROSS MEMBERS: a request ANOTHER member already answered (its record in the space's writers list,
/// as that member's page wrote it) is not let in again by this node — the record is read with no page of this account
/// open. Control: the same round with the list empty lets it in.
#[test]
fn a_request_another_member_answered_is_not_let_in_again() {
    let hexs = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    for recorded in [true, false] {
        let mut w = world();
        let asker_did = craftworks_account::did(&w.asker.did);
        let writers = crate::table::sealed_bag_address(&SPACE, "writers");
        let writers_id = id(BAG, &writers);
        let rec = serde_json::json!({ "w": "aa", "welcomed": asker_did, "kp": hexs(&identity::kp_tag(&hexs(&w.kp))), "at": 2 });
        let list = craftworks_bag_contract::encode(&writers, &[craftworks_bag_contract::grind(&writers, &crate::table::seal_writers_item(&SPACE, &rec, [5; 12]))]);
        let w2 = world_clone(&w);
        let mut base = network(&w2, vec![join(&w.asker, &w.kp)]);
        let mut net = |io: &Io| match io {
            Io::Get { id, .. } if *id == writers_id && recorded => Reply::Got { id: *id, state: Some(list.clone()) },
            _ => base(io),
        };
        let wrote = run(&mut w, &mut net).into_iter().filter(|x| !matches!(x, Io::Get { .. })).count();
        assert_eq!(wrote, if recorded { 0 } else { 4 }, "recorded by another member: {recorded}");
        let d = identity::upkeep_decided(&w.h, &MEMBER);
        if recorded {
            assert!(d.len() == 1 && d[0].step == "skip" && d[0].why.contains("used already"), "{d:?}");
        }
    }
}

/// ANOTHER CONTRACT'S ANSWER MID-ROUND (10-14, measured on fx: the inbox, read every wake-up, answered while a round
/// waited for a key log — its 6 bytes taken as the asker's log, and nobody was let in): before every answer the round
/// waits for, a stray one arrives; the round takes only its own, and the asker is let in.
#[test]
fn a_stray_answer_mid_round_is_not_taken_as_its_steps() {
    let mut w = world();
    let w2 = world_clone(&w);
    let mut net = network(&w2, vec![join(&w.asker, &w.kp)]);
    let stray = Reply::Got { id: [0xEE; 32], state: Some(vec![1, 2, 3, 4, 5, 6]) };
    let mut asked = Vec::new();
    let mut todo = upkeep::woke(&mut w.h, NOW);
    while let Some(io) = (!todo.is_empty()).then(|| todo.remove(0)) {
        todo.extend(upkeep::replied(&mut w.h, stray.clone(), NOW));
        let reply = net(&io);
        asked.push(io);
        todo.extend(upkeep::replied(&mut w.h, reply, NOW));
    }
    let wrote = asked.iter().filter(|x| !matches!(x, Io::Get { .. })).count();
    assert_eq!(wrote, 4, "commit, next log, welcome, its record — with a stray answer before each: {asked:?}");
    let d = identity::upkeep_decided(&w.h, &MEMBER);
    assert_eq!(d.iter().map(|x| x.step.as_str()).collect::<Vec<_>>(), vec!["welcomed"], "{d:?}");
}

/// A CARD WHOSE ROWS ARE IN ITS TREE (10-14, measured on fx: a page's card is flushed into blocks — "their card's rows
/// are in 1 tree block(s) this read does not follow", and nobody was let in with no page open): its blocks fetched,
/// the asker let in. Control: the same card with its blocks never answered — not let in, and the request's record
/// says so.
#[test]
fn a_card_flushed_into_its_tree_is_read_through_its_blocks() {
    for answered in [true, false] {
        let mut w = world();
        let key = SigningKey::from_bytes(&w.asker.data);
        let mut o = data::Open::new(TAIL, &key.verifying_key().to_bytes(), "card");
        o.public = true;
        let hexs = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        let sign = |o: &data::Open, seq: u64, h: [u8; 32]| {
            let p = craftec_register_contract::wire::Params::parse(&o.params).unwrap();
            key.sign(&p.signed_message(false, seq, &h)).to_bytes()
        };
        let (seq, h) = o.prepare(vec![tail::Op::Set { key: b"inbox".to_vec(), value: hexs(&identity::inbox_public(&w.asker.data)).into_bytes() }]).unwrap();
        o.commit(sign(&o, seq, h)).unwrap();
        let Ok(data::Step::Ready(f)) = o.flush() else { panic!("a flush of held rows is ready") };
        assert!(!f.sealed && !f.blocks.is_empty(), "a public tree, in blocks");
        let block_hash = contract_keys::code_hash(b"the block contract's code");
        let blocks: Vec<([u8; 32], Vec<u8>)> = f.blocks.iter().map(|(cid, st)| (upkeep::id_of(&block_hash, cid), st.clone())).collect();
        o.commit(sign(&o, f.seq, f.hash)).unwrap();
        let card_state = o.state();
        let card_id = o.id_bytes();
        let w2 = world_clone(&w);
        let mut base = network(&w2, vec![join(&w.asker, &w.kp)]);
        let mut net = |io: &Io| match io {
            Io::Get { id, .. } if *id == card_id => Reply::Got { id: *id, state: Some(card_state.clone()) },
            Io::Get { id, .. } if blocks.iter().any(|(b, _)| b == id) => Reply::Got { id: *id, state: answered.then(|| blocks.iter().find(|(b, _)| b == id).unwrap().1.clone()) },
            _ => base(io),
        };
        let wrote = run(&mut w, &mut net).into_iter().filter(|x| !matches!(x, Io::Get { .. })).count();
        let d = identity::upkeep_decided(&w.h, &MEMBER);
        if answered {
            assert_eq!(wrote, 4, "its blocks read: let in ({d:?})");
        } else {
            assert_eq!(wrote, 0, "its blocks never came: not let in");
            assert!(d.iter().any(|x| x.step == "fail" && x.why.contains("block")), "the record says why: {d:?}");
        }
    }
}

/// EACH PERSON IN TURN (10-14, measured on fx: a dozen accounts on one node, every wake-up went to the first of them
/// away — the owner whose request waited was never served): two people away, each with a bag to read — consecutive
/// rounds serve each in turn, and each one's last round is in its status (`round_at`).
#[test]
fn people_sharing_a_node_are_served_in_turn() {
    let mut w = world();
    let other: [u8; 32] = [0x01; 32]; // listed FIRST: before the fix, it took every round
    let (_, spaces) = identity::upkeep_mandate(&w.h, &MEMBER).unwrap();
    let mut theirs = spaces[0].clone();
    theirs.space = [0x6B; 32];
    identity::upkeep_set_mandate(&mut w.h, &other, "did:other", &[theirs]);
    identity::Host::set_secret(&mut w.h, identity::UPKEEP_WAKEUPS, &40u64.to_le_bytes());
    let served = |w: &mut World, wake: u64| {
        identity::Host::set_secret(&mut w.h, identity::UPKEEP_WAKEUPS, &wake.to_le_bytes());
        upkeep::woke(&mut w.h, NOW);
        let r = identity::Host::get_secret(&w.h, upkeep::ROUND).unwrap();
        identity::Host::set_secret(&mut w.h, upkeep::ROUND, &[]); // (its round ends)
        r[..32].to_vec() // whose round (its member, first)
    };
    let a = served(&mut w, 40);
    let b = served(&mut w, 41);
    let c = served(&mut w, 42);
    assert_ne!(a, b, "the second round serves the other person");
    assert_eq!(a, c, "and then the first again");
    let members = identity::upkeep_members(&w.h);
    assert!(members.iter().all(|m| identity::upkeep_round_at(&w.h, m).is_some()), "each one's last round recorded: {members:?}");
}
