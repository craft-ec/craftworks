//! THE NODE'S FOLLOWER (FOUNDATION "Members and keys", `upkeep.rs`) — whole rounds against a scripted network built
//! with the real pieces: a provisioned node identity (it signs its own act writes), real accounts (key logs, cards), the
//! space's acts in sealed feeds, an MLS group, the bags. Each test is one of the design's flows; the end is read the way
//! a member's page reads it: the acts replayed, the inbox opened, the welcome joined.

use super::table::{self, Codes, Reading};
use super::upkeep::{self, Code, Io, Reply};
use craftworks_data as data;
use craftworks_identity::{self as identity, Host, Mandate, Request};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::json;
use std::collections::{BTreeMap, HashMap};

const TAIL: &[u8] = b"the tail contract's code";
const BAG: &[u8] = b"the bag contract's code";
const SEALED: &[u8] = b"the sealed contract's code";
const BLOCK: &[u8] = b"the block contract's code";
const IDLOG: &[u8] = b"the key log contract's code";
const DEVICES: &[u8] = b"the devices contract's code";
const PIECE: &[u8] = b"the piece contract's code";
const SPACE: [u8; 32] = [0x5A; 32];
const SEED: [u8; 32] = [0x31; 32];
const CODE: &str = "ab12-cd34-ef56-7890";
const NOW: u64 = 1_800_000_000_000;

#[derive(Default, Clone)]
struct Secrets(HashMap<Vec<u8>, Vec<u8>>);
impl Host for Secrets {
    fn get_secret(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.0.get(key).cloned()
    }
    fn set_secret(&mut self, key: &[u8], value: &[u8]) -> bool {
        self.0.insert(key.to_vec(), value.to_vec());
        true
    }
}

fn codes() -> Codes<'static> {
    Codes { tail: TAIL, bag_hash: contract_keys::code_hash(BAG), sealed_hash: contract_keys::code_hash(SEALED), block_hash: contract_keys::code_hash(BLOCK), idlog_hash: contract_keys::code_hash(IDLOG), devices_hash: contract_keys::code_hash(DEVICES) }
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
}
fn id(code: &[u8], params: &[u8]) -> [u8; 32] {
    upkeep::id_of(&contract_keys::code_hash(code), params)
}

/// The network: contract id → state; a tail's params (its deltas applied by its contract); a bag merges what is put.
#[derive(Default, Clone)]
struct Net {
    st: HashMap<[u8; 32], Vec<u8>>,
    params: HashMap<[u8; 32], Vec<u8>>,
}
impl Net {
    fn seed_tail(&mut self, o: &data::Open) {
        self.params.insert(o.id_bytes(), o.params.clone());
        self.st.insert(o.id_bytes(), o.state());
    }
    fn answer(&mut self, io: &Io) -> Reply {
        use craftec_register_contract::wire::Params;
        match io {
            Io::Get { id, .. } => Reply::Got { id: *id, state: self.st.get(id).cloned() },
            Io::Put { code, params, state } => {
                let code_bytes: &[u8] = match code {
                    Code::Tail => TAIL,
                    Code::Bag => BAG,
                    Code::Sealed => SEALED,
                    Code::Piece => PIECE,
                };
                let id = id(code_bytes, params);
                if *code == Code::Tail {
                    self.params.insert(id, params.clone());
                }
                let st = if *code == Code::Bag {
                    let mut items = self.st.get(&id).and_then(|s| craftworks_bag_contract::read(params, s)).unwrap_or_default();
                    items.extend(craftworks_bag_contract::read(params, state).unwrap_or_default());
                    craftworks_bag_contract::encode(params, &items)
                } else {
                    state.clone()
                };
                self.st.insert(id, st);
                Reply::Put { id, ok: true }
            }
            Io::Update { id, delta } => {
                let p = Params::parse(&self.params[id]).unwrap();
                let held = self.st.get(id).and_then(|s| craftec_tail_contract::read(&self.params[id], s)).and_then(|(_, t)| t);
                let next = craftec_tail_contract::absorb(held.clone(), delta, &p);
                let ok = next.as_ref().map(|t| t.signed.seq) != held.as_ref().map(|t| t.signed.seq);
                if let Some(t) = next {
                    self.st.insert(*id, t.encode(&p.authority));
                }
                Reply::Updated { id: *id, ok }
            }
        }
    }
    fn bag_items(&self, address: [u8; 32]) -> Vec<Vec<u8>> {
        self.st.get(&id(BAG, &address)).and_then(|s| craftworks_bag_contract::read(&address, s)).unwrap_or_default().iter().map(|p| craftworks_bag_contract::payload(p).to_vec()).collect()
    }
    fn drop_in(&mut self, address: [u8; 32], item: &serde_json::Value) {
        let mut items = self.st.get(&id(BAG, &address)).and_then(|s| craftworks_bag_contract::read(&address, s)).unwrap_or_default();
        items.push(craftworks_bag_contract::grind(&address, item.to_string().as_bytes()));
        self.st.insert(id(BAG, &address), craftworks_bag_contract::encode(&address, &items));
    }
}

/// An ACCOUNT from its words' entropy: its DID, data seed, key log, and its space member (MLS: key packages whose
/// credential names the DID).
struct Person {
    entropy: [u8; 16],
    did: [u8; 32],
    data: [u8; 32],
    log: craftworks_idlog_contract::Log,
    sm: craftworks_mls::SpaceMember,
    mls_pub: [u8; 32],
}
fn person(e: u8) -> Person {
    let entropy = [e; 16];
    let ev = craftworks_account::inception(&entropy).unwrap();
    let did = ev.id();
    let data = craftworks_account::data_seed(&entropy).unwrap();
    let seed = identity::space_member_seed(&data);
    let mls_pub = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
    let sm = craftworks_mls::SpaceMember::new(&seed, identity::space_member_credential(&did, &data, &mls_pub), &[]).unwrap();
    Person { entropy, did, data, log: craftworks_idlog_contract::Log { events: vec![ev] }, sm, mls_pub }
}
impl Person {
    fn id(&self) -> String {
        craftworks_account::did(&self.did)
    }
    /// Their key log and CARD on the network: the inbox key, and the devices (`nodes`) that write for them.
    fn publish(&self, net: &mut Net, nodes: &[Vec<u8>]) {
        net.st.insert(id(IDLOG, &self.did), self.log.encode());
        let key = SigningKey::from_bytes(&self.data);
        let mut o = data::Open::new(TAIL, &key.verifying_key().to_bytes(), "card");
        o.public = true;
        let mut ops = vec![tail::Op::Set { key: b"inbox".to_vec(), value: hex(&identity::inbox_public(&self.data)).into_bytes() }];
        if !nodes.is_empty() {
            ops.push(tail::Op::Set { key: b"nodes".to_vec(), value: serde_json::to_vec(&nodes.iter().map(|n| hex(n)).collect::<Vec<_>>()).unwrap() });
        }
        let (seq, h) = o.prepare(ops).unwrap();
        let p = craftec_register_contract::wire::Params::parse(&o.params).unwrap();
        o.commit(key.sign(&p.signed_message(false, seq, &h)).to_bytes()).unwrap();
        net.st.insert(o.id_bytes(), o.state());
    }
    fn kp(&self) -> Vec<u8> {
        self.sm.key_package().unwrap()
    }
    /// What their page asks with: a join (`kind: join`) or a listed device's "my key" (`kind: key`).
    fn ask(&self, kind: &str, kp: &[u8]) -> serde_json::Value {
        json!({ "kind": kind, "did": self.id(), "at": 1, "kp": hex(kp) })
    }
    /// Their INBOX opened with their key: each item (JSON).
    fn inbox(&self, net: &Net) -> Vec<serde_json::Value> {
        net.bag_items(identity::inbox_address(&self.did)).iter().filter_map(|i| identity::open_with(&identity::inbox_seed(&self.data), i)).map(|b: Vec<u8>| serde_json::from_slice(&b).unwrap()).collect()
    }
}

/// A writer's FEED of a space table, as a page writes it: sealed with the table's key and the epochs'.
struct Feed {
    key: SigningKey,
    o: data::Open,
}
impl Feed {
    fn new(key: &SigningKey, table: &str, label: &str, epochs: &[(u64, [u8; 32])]) -> Feed {
        let mut o = data::Open::new(TAIL, &key.verifying_key().to_bytes(), label);
        o.set_table_key(identity::space_table_key(&SPACE, table));
        for (e, s) in epochs {
            o.epoch_key(*e, Some(identity::epoch_table_key(s, table)), true);
        }
        Feed { key: key.clone(), o }
    }
    fn put(&mut self, k: &str, v: &str) {
        let id = (self.key.verifying_key().to_bytes(), self.o.writer.seq() + 1);
        let env = craftworks_feed::envelope(id, None, Some(v.as_bytes()));
        let (seq, h) = self.o.prepare_row(k.as_bytes(), &env).unwrap();
        let p = craftec_register_contract::wire::Params::parse(&self.o.params).unwrap();
        self.o.commit(self.key.sign(&p.signed_message(false, seq, &h)).to_bytes()).unwrap();
    }
}
fn blinded(name: &str) -> String {
    let t = table::table_name(&SPACE, name);
    identity::blind_name(&identity::space_table_key(&SPACE, &t), &t)
}

/// THE OWNER'S NODE and the space: its group as `group` made it (the owner first), its acts as `acts` say (written by the
/// owner's device), the owner's page away. Every leaf's person and the owner published.
struct World {
    h: Secrets,
    net: Net,
    member: [u8; 32],
    owner: Person,
}
fn world(acts: &[serde_json::Value], group: impl FnOnce(&mut craftworks_mls::Account)) -> World {
    let owner = person(0x61);
    let mut h = Secrets::default();
    identity::serve(&mut h, Request::Provision { seed: SEED, did: owner.did, pin: "246810".into(), data: owner.data.to_vec() }, [9; 32]);
    let member = SigningKey::from_bytes(&SEED).verifying_key().to_bytes();
    let mut g = owner.sm.create_space(SPACE).unwrap();
    let s0 = g.epoch_secret().unwrap();
    assert!(identity::keep_epoch(&mut h, &member, Some(SPACE), 0, &s0));
    group(&mut g);
    let epoch = g.epoch();
    assert!(identity::keep_epoch(&mut h, &member, Some(SPACE), epoch, &g.epoch_secret().unwrap()));
    let me = owner.id();
    let m = Mandate {
        space: SPACE, name: "Makers".into(), kind: "server".into(), owner: me.clone(), nonce: None, channel: "ch".into(),
        open: false, codes: vec![], level: None, bans: vec![], members: vec![], epoch, state: g.save().unwrap(), admitters: vec![],
    };
    identity::upkeep_set_mandate(&mut h, &member, &me, &[m]);
    identity::upkeep_set_tick(&mut h, &member);
    h.set_secret(identity::UPKEEP_WAKEUPS, &30u64.to_le_bytes());
    identity::upkeep_stir(&mut h, &[7; 32]);
    for (k, v) in [(identity::UPKEEP_TAIL, TAIL), (identity::UPKEEP_BAG, BAG), (identity::UPKEEP_SEALED, SEALED), (identity::UPKEEP_PIECE, PIECE)] {
        h.set_secret(k, v);
    }
    h.set_secret(identity::UPKEEP_BLOCK, &contract_keys::code_hash(BLOCK));
    h.set_secret(identity::UPKEEP_DEVICES, &contract_keys::code_hash(DEVICES));
    h.set_secret(identity::UPKEEP_IDLOG, &contract_keys::code_hash(IDLOG));

    let mut net = Net::default();
    // The owner's device writes the space (its credential on the card: how a reader finds the space's writers).
    let cred = identity::credential(&owner.did, &member, &owner.mls_pub, &craftworks_account::owner_seed(&owner.entropy).unwrap());
    owner.publish(&mut net, &[cred]);
    let me_key = SigningKey::from_bytes(&SEED);
    let ep = [(0u64, s0)];
    let cat = table::table_name(&SPACE, "tables");
    let mut c = Feed::new(&me_key, &cat, &cat, &ep);
    c.put(&table::table_name(&SPACE, "acts"), r#"{"b":1}"#);
    net.seed_tail(&c.o);
    let mut a = Feed::new(&me_key, &table::table_name(&SPACE, "acts"), &blinded("acts"), &ep);
    for (i, act) in acts.iter().enumerate() {
        a.put(&format!("a{i}"), &act.to_string());
    }
    net.seed_tail(&a.o);
    // The writers list (the welcomes made): there, with this device's own entry.
    let address = table::sealed_bag_address(&SPACE, "writers");
    let item = table::seal_writers_item(&SPACE, &json!({ "w": hex(&member) }), [1; 12]);
    net.st.insert(id(BAG, &address), craftworks_bag_contract::encode(&address, &[craftworks_bag_contract::grind(&address, &item)]));
    World { h, net, member, owner }
}

impl World {
    /// A whole round: woken, every answer routed through upkeep. Every ask made.
    fn run(&mut self) -> Vec<Io> {
        let mut asked = Vec::new();
        let mut todo = upkeep::woke(&mut self.h, NOW);
        while let Some(io) = (!todo.is_empty()).then(|| todo.remove(0)) {
            assert!(asked.len() < 5000, "the round ends");
            let reply = self.net.answer(&io);
            asked.push(io);
            todo.extend(upkeep::replied(&mut self.h, reply, NOW));
        }
        // The next wake-up is a new one (a round never runs twice in one).
        let w = 10 + u64::from_le_bytes(self.h.get_secret(identity::UPKEEP_WAKEUPS).unwrap().try_into().unwrap());
        self.h.set_secret(identity::UPKEEP_WAKEUPS, &w.to_le_bytes());
        asked
    }
    fn mandate(&self) -> Mandate {
        identity::upkeep_mandate(&self.h, &self.member).unwrap().1.remove(0)
    }
    /// The group's entries, by DID.
    fn leaves(&self) -> Vec<String> {
        let g = craftworks_mls::Account::load(craftworks_mls::Rule::Space(SPACE), &self.mandate().state).unwrap();
        g.members().into_iter().map(|(_, _, cred)| craftworks_account::did(&identity::read_credential(&cred).unwrap().0)).collect()
    }
    /// The space's ACTS as a reader replays them (every epoch the node holds).
    fn gov(&self) -> craftworks_gov::Gov {
        let top = identity::latest_epoch(&self.h, &self.member, SPACE).unwrap();
        let epochs: BTreeMap<u64, [u8; 32]> = (0..=top).filter_map(|e| Some((e, identity::epoch_secret(&self.h, &self.member, SPACE, e)?))).collect();
        let (mut r, first) = Reading::new(&codes(), SPACE, epochs, vec![self.member], &["acts", "pub-acts"]);
        let mut todo = first;
        while let Some(io) = (!todo.is_empty()).then(|| todo.remove(0)) {
            let Io::Get { id, .. } = io else { panic!("a read only GETs") };
            todo.extend(r.got(&codes(), id, self.net.st.get(&id).cloned()));
        }
        crate::rekey::governance(&r, &[(self.member, self.owner.id())], &self.owner.id(), NOW)
    }
    fn said(&self) -> Vec<String> {
        self.h.0.iter().filter(|(k, _)| k.windows(4).any(|w| w == b"said")).map(|(_, v)| String::from_utf8_lossy(v).into_owned()).collect()
    }
    fn writes(asked: &[Io]) -> usize {
        asked.iter().filter(|x| !matches!(x, Io::Get { .. })).count()
    }
}

/// Joined by the welcome in their inbox: the group they hold (its epoch).
fn joined(p: &Person, net: &Net) -> u64 {
    let items = p.inbox(net);
    let w = items.iter().rev().find(|i| i["kind"] == "welcome").unwrap_or_else(|| panic!("a welcome in their inbox: {items:?}"));
    p.sm.join_space(SPACE, &unhex(w["welcome"].as_str().unwrap())).unwrap().epoch()
}

/// FLOW 1/2 — a new person joins by a code, no page open: the LIST first (an `admitted` act, written by this node, which
/// the replay counts), then ONE commit, the welcome they join by, their key answered in the writers list. The next round
/// finds nothing to do.
#[test]
fn a_join_by_code_is_listed_then_followed_and_its_welcome_joins_them() {
    let asker = person(0x22);
    let mut w = world(&[json!({ "act": "invite", "code": CODE, "uses": 1, "at": 100 })], |_| {});
    asker.publish(&mut w.net, &[]);
    w.net.drop_in(identity::invite_address(CODE), &asker.ask("join", &asker.kp()));
    w.run();
    let g = w.gov();
    assert!(g.roster.contains(&asker.id()), "listed by an admitted act: {:?} — said {:?}", g.roster, w.said());
    assert!(g.live_invites(NOW as f64).is_empty(), "the one-use code used up by it");
    assert_eq!(joined(&asker, &w.net), 1, "one commit");
    assert!(w.leaves().contains(&asker.id()));
    let answered: Vec<serde_json::Value> = w.net.bag_items(table::sealed_bag_address(&SPACE, "writers")).iter().filter_map(|i| table::open_bag_item(&SPACE, &Default::default(), "writers", i)).collect();
    assert!(answered.iter().any(|v| v["welcomed"] == asker.id().as_str()), "their key answered: {answered:?}");
    let again = w.run();
    assert_eq!(World::writes(&again), 0, "nothing more: {:?}", w.said());
}

/// FLOW 6 — a member removed by the acts: their entry out, one commit, no welcome.
#[test]
fn a_removed_member_is_taken_out_with_no_welcome() {
    let x = person(0x33);
    let kp = x.kp();
    let mut w = world(&[json!({ "act": "added", "did": x.id(), "at": 10 }), json!({ "act": "remove", "did": x.id(), "at": 20 })], |g| {
        g.follow(&[], &[kp.clone()]).unwrap();
    });
    x.publish(&mut w.net, &[]);
    let e = w.mandate().epoch;
    assert!(w.leaves().contains(&x.id()));
    w.run();
    assert_eq!(w.mandate().epoch, e + 1, "one commit: {:?}", w.said());
    assert!(!w.leaves().contains(&x.id()), "out");
    assert!(x.inbox(&w.net).is_empty(), "no welcome");
}

/// FLOW 10 — IVVOR's SPACE: A, B, C listed. The group holds C (current), an old entry of A's and a stray D (not listed);
/// B never got keys. A and B post "my key": ONE commit — A's old entry and D out, A and B in; C untouched. Both join by
/// the one welcome. Asked again with the same keys: answered once.
#[test]
fn the_keys_follow_the_list_in_one_commit_ivvors_space() {
    let (a, b, c, d) = (person(0x41), person(0x42), person(0x43), person(0x44));
    let (a_old, c_kp, d_kp) = (a.kp(), c.kp(), d.kp());
    let acts: Vec<serde_json::Value> = [&a, &b, &c].iter().map(|p| json!({ "act": "member", "did": p.id(), "at": 10 })).collect();
    let mut w = world(&acts, |g| {
        g.follow(&[], &[a_old.clone(), c_kp.clone(), d_kp.clone()]).unwrap();
    });
    for p in [&a, &b, &c, &d] {
        p.publish(&mut w.net, &[]);
    }
    let e = w.mandate().epoch;
    let key = identity::invite_address(&upkeep::bag_name("key", &SPACE));
    let (a_kp, b_kp) = (a.kp(), b.kp());
    w.net.drop_in(key, &a.ask("key", &a_kp));
    w.net.drop_in(key, &b.ask("key", &b_kp));
    w.run();
    assert_eq!(w.mandate().epoch, e + 1, "ONE commit: {:?}", w.said());
    let mut leaves = w.leaves();
    leaves.sort();
    let mut want = vec![w.owner.id(), a.id(), b.id(), c.id()];
    want.sort();
    assert_eq!(leaves, want, "A once (new), B in, C kept, D out");
    assert_eq!((joined(&a, &w.net), joined(&b, &w.net)), (e + 1, e + 1), "both by the one welcome");
    let again = w.run();
    assert_eq!(World::writes(&again), 0, "answered once: {:?}", w.said());
}

/// A SHORT READ takes nobody out: a removal in the acts, but one member's key log does not answer — the list may be
/// short, so the removal waits for a whole read.
#[test]
fn a_short_read_takes_nobody_out() {
    let (x, y) = (person(0x33), person(0x34));
    let (x_kp, y_kp) = (x.kp(), y.kp());
    let mut w = world(&[json!({ "act": "added", "did": x.id(), "at": 10 }), json!({ "act": "remove", "did": x.id(), "at": 20 })], |g| {
        g.follow(&[], &[x_kp.clone(), y_kp.clone()]).unwrap();
    });
    x.publish(&mut w.net, &[]); // (Y's key log never answers.)
    let asked = w.run();
    assert_eq!(World::writes(&asked), 0, "nothing moves: {:?}", w.said());
    assert!(w.leaves().contains(&x.id()));
    assert!(w.said().iter().any(|s| s.contains("SHORT")), "said why: {:?}", w.said());
}

/// A join carrying SOMEONE ELSE's key: not listed, nothing committed.
#[test]
fn a_join_with_someone_elses_key_is_not_listed_nor_let_in() {
    let (asker, other) = (person(0x22), person(0x23));
    let mut w = world(&[json!({ "act": "invite", "code": CODE, "uses": 1, "at": 100 })], |_| {});
    asker.publish(&mut w.net, &[]);
    other.publish(&mut w.net, &[]);
    w.net.drop_in(identity::invite_address(CODE), &asker.ask("join", &other.kp()));
    let asked = w.run();
    assert_eq!(World::writes(&asked), 0, "{:?}", w.said());
    assert!(!w.gov().roster.contains(&asker.id()));
    assert_eq!(w.mandate().epoch, 0);
}

/// "My key" from someone NOT listed is no join: nothing.
#[test]
fn my_key_from_someone_not_listed_lets_nobody_in() {
    let z = person(0x55);
    let mut w = world(&[], |_| {});
    z.publish(&mut w.net, &[]);
    w.net.drop_in(identity::invite_address(&upkeep::bag_name("key", &SPACE)), &z.ask("key", &z.kp()));
    let asked = w.run();
    assert_eq!(World::writes(&asked), 0, "{:?}", w.said());
    assert!(!w.leaves().contains(&z.id()));
}
