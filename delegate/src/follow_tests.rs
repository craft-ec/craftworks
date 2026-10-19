//! THE NODE'S FOLLOWER (FOUNDATION "Members and keys", the design of 10-17; `upkeep.rs`) — whole rounds against a
//! scripted network built with the real pieces: a provisioned node identity, real accounts (key logs, cards), the
//! space's acts in sealed feeds, an MLS group its members ENTERED by themselves, the open bags (writers list, doors).
//! Each test is one of the design's flows; the end is read the way a member's page reads it.

use super::table;
use super::upkeep::{self, Code, Io, Reply};
use craftworks_data as data;
use craftworks_identity::{self as identity, Host, Mandate, Request};
use craftworks_mls::Account;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::json;
use std::collections::HashMap;

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

/// An ACCOUNT from its words' entropy: its DID, data seed, key log, and its space member (MLS: its credential names the
/// DID).
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
    let sm = craftworks_mls::SpaceMember::new(&seed, identity::space_member_credential(&did, &data, &mls_pub)).unwrap();
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
    /// ENTERED BY ITSELF from the group's info (`aad`: its proof, if any): the commit it publishes.
    fn enter(&self, info: &[u8], aad: &[u8]) -> (Account, Vec<u8>) {
        self.sm.enter(SPACE, info, aad).unwrap()
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

/// THE OWNER'S NODE and the space: its group made by the owner, `inside` entered by themselves (each listed or not,
/// as the test makes them), its acts as `acts` say (written by the owner's device), the owner's page away.
struct World {
    h: Secrets,
    net: Net,
    member: [u8; 32],
    owner: Person,
}

fn world(acts: &[serde_json::Value], inside: &[&Person]) -> World {
    world_with(acts, &inside.iter().map(|p| (*p, &b""[..])).collect::<Vec<_>>())
}
/// …each `inside` entering with its own words (`aad`: a proof, for one entering by a code, the open door, a level).
fn world_with(acts: &[serde_json::Value], inside: &[(&Person, &[u8])]) -> World {
    let owner = person(0x61);
    let mut h = Secrets::default();
    identity::serve(&mut h, Request::Provision { seed: SEED, did: owner.did, pin: "246810".into(), data: owner.data.to_vec() }, [9; 32]);
    let member = SigningKey::from_bytes(&SEED).verifying_key().to_bytes();
    let mut g = owner.sm.create_space(SPACE).unwrap();
    let s0 = g.epoch_secret().unwrap();
    assert!(identity::keep_epoch(&mut h, &member, Some(SPACE), 0, &s0));
    for (p, aad) in inside {
        let (_, commit) = p.enter(&g.group_info().unwrap(), aad);
        g.process(&commit).unwrap();
        assert!(identity::keep_epoch(&mut h, &member, Some(SPACE), g.epoch(), &g.epoch_secret().unwrap()));
    }
    let epoch = g.epoch();
    let me = owner.id();
    let m = Mandate { space: SPACE, name: "Makers".into(), kind: "server".into(), owner: me.clone(), nonce: None, channel: "ch".into(), epoch, state: g.save().unwrap() };
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
    // The writers list: there, with this device's own entry.
    let address = table::sealed_bag_address(&SPACE, "writers");
    let item = table::seal_open_item(&SPACE, &json!({ "w": hex(&member) }), [1; 12]);
    net.st.insert(id(BAG, &address), craftworks_bag_contract::encode(&address, &[craftworks_bag_contract::grind(&address, &item)]));
    World { h, net, member, owner }
}

impl World {
    /// A whole round: woken, every answer routed through upkeep. Every ask made.
    fn run(&mut self) -> Vec<Io> {
        self.run_at(NOW)
    }
    /// …at `now` (the node's clock: a takeover's time passed).
    fn run_at(&mut self, now: u64) -> Vec<Io> {
        identity::upkeep_set_clock(&mut self.h, now / 1000);
        let mut asked = Vec::new();
        let mut todo = upkeep::woke(&mut self.h, now);
        while let Some(io) = (!todo.is_empty()).then(|| todo.remove(0)) {
            assert!(asked.len() < 5000, "the round ends");
            let reply = self.net.answer(&io);
            asked.push(io);
            todo.extend(upkeep::replied(&mut self.h, reply, now));
        }
        // The next wake-up is a new one (a round never runs twice in one).
        let w = 10 + u64::from_le_bytes(self.h.get_secret(identity::UPKEEP_WAKEUPS).unwrap().try_into().unwrap());
        self.h.set_secret(identity::UPKEEP_WAKEUPS, &w.to_le_bytes());
        asked
    }
    fn mandate(&self) -> Mandate {
        identity::upkeep_mandate(&self.h, &self.member).unwrap().1.remove(0)
    }
    fn group(&self) -> Account {
        Account::load(craftworks_mls::Rule::Space(SPACE), &self.mandate().state).unwrap()
    }
    /// The group's entries, by DID.
    fn leaves(&self) -> Vec<String> {
        self.group().members().into_iter().map(|(_, _, cred)| craftworks_account::did(&identity::read_credential(&cred).unwrap().0)).collect()
    }
    /// What the round recorded (its `followed` steps), as "step: why".
    fn said(&self) -> Vec<String> {
        identity::upkeep_decided(&self.h, &self.member).into_iter().map(|d| format!("{}: {}", d.step, d.why)).collect()
    }
    fn writes(asked: &[Io]) -> usize {
        asked.iter().filter(|x| !matches!(x, Io::Get { .. })).count()
    }
    /// An open bag's items (the writers list, a door), opened.
    fn open_items(&self, name: &str) -> Vec<serde_json::Value> {
        self.net.bag_items(table::sealed_bag_address(&SPACE, name)).iter().filter_map(|i| table::open_bag_item(&SPACE, &Default::default(), name, i)).collect()
    }
    /// A device's ENTRY put in its epoch's door (as the page's `keys.enterNow` puts it).
    fn door_entry(&mut self, epoch: u64, commit: &[u8]) {
        let address = table::sealed_bag_address(&SPACE, &format!("door-{epoch}"));
        let entry = json!({ "commit": hex(commit), "why": "commit enter" }).to_string();
        let item = table::seal_open_item(&SPACE, &json!({ "k": "enter", "e": epoch, "entry": entry }), [epoch as u8 + 2; 12]);
        let mut items = self.net.st.get(&id(BAG, &address)).and_then(|s| craftworks_bag_contract::read(&address, s)).unwrap_or_default();
        items.push(craftworks_bag_contract::grind(&address, &item));
        self.net.st.insert(id(BAG, &address), craftworks_bag_contract::encode(&address, &items));
    }
}

/// FLOW 6 — a member removed by the acts: their entry out, ONE commit; the new epoch's group info put in its door and
/// announced (what a listed device with no key enters from). The next round: nothing.
#[test]
fn a_removed_member_is_taken_out_and_the_new_info_published() {
    let x = person(0x33);
    let mut w = world(&[json!({ "act": "added", "did": x.id(), "at": 10 }), json!({ "act": "remove", "did": x.id(), "at": 20 })], &[&x]);
    x.publish(&mut w.net, &[]);
    let e = w.mandate().epoch;
    assert!(w.leaves().contains(&x.id()));
    w.run();
    assert_eq!(w.mandate().epoch, e + 1, "one commit: {:?}", w.said());
    assert!(!w.leaves().contains(&x.id()), "out");
    let parts: Vec<serde_json::Value> = w.open_items(&format!("door-{}", e + 1)).into_iter().filter(|v| v["k"] == "info").collect();
    assert!(!parts.is_empty(), "the new epoch's info in its door");
    let ih = parts[0]["d"].as_str().unwrap().to_string();
    let info: String = { let mut p = parts.clone(); p.sort_by_key(|v| v["i"].as_u64()); p.iter().map(|v| v["x"].as_str().unwrap().to_string()).collect() };
    assert_eq!(craftworks_mls::info_facts(&unhex(&info)).unwrap().epoch, e + 1, "whole, at the new epoch");
    assert!(w.open_items("writers").iter().any(|v| v["epoch"].as_u64() == Some(e + 1) && v["ih"] == ih.as_str()), "announced with its hash");
    let again = w.run();
    assert_eq!(World::writes(&again), 0, "nothing more: {:?}", w.said());
}

/// FLOW 9 — IVVOR'S SPACE: A, B, C listed. The group holds A, C and a stray D (not listed); B never held keys. ONE
/// commit takes D out; nobody is added (B enters by itself — the node never adds anyone).
#[test]
fn ivvors_space_the_stray_out_in_one_commit_nobody_added() {
    let (a, b, c, d) = (person(0x41), person(0x42), person(0x43), person(0x44));
    let acts: Vec<serde_json::Value> = [&a, &b, &c].iter().map(|p| json!({ "act": "member", "did": p.id(), "at": 10 })).collect();
    let mut w = world(&acts, &[&a, &c, &d]);
    for p in [&a, &b, &c, &d] {
        p.publish(&mut w.net, &[]);
    }
    let e = w.mandate().epoch;
    // (Whoever's turn: this node acts now, or once a takeover's time passed with nobody else acting.)
    w.run();
    if w.mandate().epoch == e {
        w.run_at(NOW + 3 * craftworks_rules::guards::TAKEOVER_MS);
    }
    assert_eq!(w.mandate().epoch, e + 1, "ONE commit: {:?}", w.said());
    let mut leaves = w.leaves();
    leaves.sort();
    let mut want = vec![w.owner.id(), a.id(), c.id()];
    want.sort();
    assert_eq!(leaves, want, "D out; B not added");
}

/// A SHORT READ decides nothing: a removal in the acts, but one member's key log does not answer — the list may be
/// short, so nothing is judged and nobody taken out.
#[test]
fn a_short_read_decides_nothing() {
    let (x, y) = (person(0x33), person(0x34));
    let mut w = world(&[json!({ "act": "added", "did": x.id(), "at": 10 }), json!({ "act": "remove", "did": x.id(), "at": 20 })], &[&x, &y]);
    x.publish(&mut w.net, &[]); // (Y's key log never answers.)
    let asked = w.run();
    assert_eq!(World::writes(&asked), 0, "nothing moves: {:?}", w.said());
    assert!(w.leaves().contains(&x.id()));
    assert!(w.said().iter().any(|s| s.starts_with("list-short")), "said why: {:?}", w.said());
}

/// CATCH-UP by the one rule: a LISTED device's entry in the door is taken; an OUTSIDER's at the same position is skipped
/// by the list check — whichever is lower. Control: with the outsider listed, its entry is taken when it is lowest.
#[test]
fn the_door_a_listed_entry_taken_an_outsiders_skipped() {
    let (b, z) = (person(0x52), person(0x5f));
    let acts = [json!({ "act": "member", "did": b.id(), "at": 10 })];
    let mut w = world(&acts, &[]);
    for p in [&b, &z] {
        p.publish(&mut w.net, &[]);
    }
    let e = w.mandate().epoch;
    let info = w.group().group_info().unwrap();
    let (bg, b_in) = b.enter(&info, b"");
    let (_, z_in) = z.enter(&info, b"");
    w.door_entry(e, &b_in);
    w.door_entry(e, &z_in);
    w.run();
    assert_eq!(w.mandate().epoch, e + 1, "{:?}", w.said());
    assert!(w.leaves().contains(&b.id()) && !w.leaves().contains(&z.id()), "B in, Z skipped: {:?}", w.said());
    assert_eq!(w.group().epoch_secret().unwrap(), bg.epoch_secret().unwrap(), "the node follows B's entry");
    // Control: Z listed too — the LOWER of the two entries is taken (the one order every reader keeps).
    let acts2 = [json!({ "act": "member", "did": b.id(), "at": 10 }), json!({ "act": "member", "did": z.id(), "at": 10 })];
    let mut w2 = world(&acts2, &[]);
    for p in [&b, &z] {
        p.publish(&mut w2.net, &[]);
    }
    let info = w2.group().group_info().unwrap();
    let (_, b_in) = b.enter(&info, b"");
    let (_, z_in) = z.enter(&info, b"");
    w2.door_entry(0, &b_in);
    w2.door_entry(0, &z_in);
    w2.run();
    let entry = |c: &[u8]| json!({ "commit": hex(c), "why": "commit enter" }).to_string();
    let lowest = if entry(&b_in) < entry(&z_in) { b.id() } else { z.id() };
    assert!(w2.leaves().contains(&lowest), "the lower entry taken: {:?}", w2.said());
}

/// SELF-ENTRY BY A CODE (FOUNDATION "Members and keys" 2): a person not listed enters with the code's proof bound to
/// their DID — taken; the same with a wrong proof — skipped.
#[test]
fn a_self_entry_with_a_code_proof_is_taken_a_wrong_one_skipped() {
    let z = person(0x5e);
    let acts = [json!({ "act": "invite", "code": CODE, "uses": 1, "at": 100 })];
    for (proof, taken) in [(craftworks_gov::join_proof(CODE, &z.id()), true), (craftworks_gov::join_proof("wrong-code", &z.id()), false)] {
        let mut w = world(&acts, &[]);
        z.publish(&mut w.net, &[]);
        let aad = json!({ "why": "code", "join": "code", "p": proof }).to_string();
        let (_, z_in) = z.enter(&w.group().group_info().unwrap(), aad.as_bytes());
        w.door_entry(0, &z_in);
        w.run();
        assert_eq!(w.leaves().contains(&z.id()), taken, "{:?}", w.said());
    }
}

/// ONE FOLLOWER AT A TIME: two listed members hold keys and a stray must go — the one ranked second waits (and says
/// when its turn comes); once the first stays away a takeover's time, it acts.
#[test]
fn one_follower_at_a_time_the_second_waits_then_takes_over() {
    // A listed member Q ranked FIRST for this epoch, the owner's node second.
    let stray = person(0x70);
    let q = (0x71u8..0xff).map(person).find(|q| {
        let ids = vec![person(0x61).id(), q.id()];
        let w = world(&[json!({ "act": "member", "did": q.id(), "at": 10 })], &[q, &stray]);
        craftworks_rules::admission::turn(&format!("follow|{}", w.mandate().epoch), &ids, &ids[0]) == 1
    });
    let q = q.expect("a member ranked first");
    let mut w = world(&[json!({ "act": "member", "did": q.id(), "at": 10 })], &[&q, &stray]);
    for p in [&q, &stray] {
        p.publish(&mut w.net, &[]);
    }
    let e = w.mandate().epoch;
    w.run();
    assert_eq!(w.mandate().epoch, e, "not its turn: no commit: {:?}", w.said());
    let Some(next) = (match identity::upkeep_status(&w.h, Some(&w.member)) { identity::Answer::Upkeep { next, .. } => next, _ => None }) else { panic!("it says when: {:?}", w.said()) };
    assert!(next > NOW, "its turn later");
    // Q stays away: after a takeover's time this node acts — one commit.
    w.run_at(next + 1);
    assert_eq!(w.mandate().epoch, e + 1, "taken over: {:?}", w.said());
}

/// AN ENTRY BY A PROOF is kept only while the acts have not put its account out: Z entered by the open door (no `join`
/// act yet) — kept; Z BANNED — taken out, its proof whatever it says. (Review 10-17: a lapsed or removed proof-entrant
/// kept the keys for ever.)
#[test]
fn a_proof_entry_is_kept_until_the_acts_put_it_out() {
    let z = person(0x5d);
    let open = json!({ "act": "policy", "path": "", "action": "join", "who": "anyone", "at": 5 });
    let aad = json!({ "why": "open", "join": "open" }).to_string();
    let mut w = world_with(&[open.clone()], &[(&z, aad.as_bytes())]);
    z.publish(&mut w.net, &[]);
    let e = w.mandate().epoch;
    w.run();
    assert_eq!((w.mandate().epoch, w.leaves().contains(&z.id())), (e, true), "control — its proof admits it: kept: {:?}", w.said());
    let mut w = world_with(&[open, json!({ "act": "ban", "did": z.id(), "at": 9 })], &[(&z, aad.as_bytes())]);
    z.publish(&mut w.net, &[]);
    w.run();
    if w.leaves().contains(&z.id()) {
        w.run_at(NOW + 3 * craftworks_rules::guards::TAKEOVER_MS);
    }
    assert!(!w.leaves().contains(&z.id()), "banned: out, whatever its leaf says: {:?}", w.said());
}

// ---- K5: a BeeKEM space kept to its list by the node (docs/proposals/members-and-keys-beekem.md, K5) ----

use craftworks_groups as groups;

impl Person {
    /// This account's BeeKEM member (one per account: from its space member seed).
    fn bee(&self) -> groups::Account {
        groups::Account::of(&identity::space_member_seed(&self.data))
    }
    /// Their CARD with their member on it, its rows FLUSHED into a tree (a card with many rows: what a node reads from
    /// blocks, fx 10-17 — before, it read such a card as having no member).
    fn publish_member_flushed(&self, net: &mut Net, nodes: &[Vec<u8>]) {
        self.publish_member(net, nodes);
        let key = SigningKey::from_bytes(&self.data);
        let mut o = data::Open::new(TAIL, &key.verifying_key().to_bytes(), "card");
        o.public = true;
        assert!(o.absorb(&net.st[&o.id_bytes()]));
        let Ok(data::Step::Ready(f)) = o.flush() else { panic!("the card flushes") };
        for (cid, st) in &f.blocks {
            let id = table::block_id(&o, cid, &contract_keys::code_hash(SEALED), &contract_keys::code_hash(BLOCK)).unwrap();
            net.st.insert(id, st.clone());
        }
        let p = craftec_register_contract::wire::Params::parse(&o.params).unwrap();
        o.commit(key.sign(&p.signed_message(false, f.seq, &f.hash)).to_bytes()).unwrap();
        net.st.insert(o.id_bytes(), o.state());
        // (Its rows are in the tree now: a reader that does not fetch blocks reads no member.)
        let mut bare = data::Open::new(TAIL, &key.verifying_key().to_bytes(), "card");
        bare.public = true;
        bare.absorb(&net.st[&o.id_bytes()]);
        assert!(matches!(bare.opened(), Ok(data::Step::Need(_))), "control: the card needs its blocks");
    }
    /// Their CARD with their BeeKEM member on it (`beekem`: `{ cred, leaf }` — the page's `directory.setMember`).
    fn publish_member(&self, net: &mut Net, nodes: &[Vec<u8>]) {
        net.st.insert(id(IDLOG, &self.did), self.log.encode());
        let key = SigningKey::from_bytes(&self.data);
        let mut o = data::Open::new(TAIL, &key.verifying_key().to_bytes(), "card");
        o.public = true;
        let leaf: [u8; 32] = *self.bee().leaf_key().as_bytes();
        let cred = identity::space_member_credential(&self.did, &self.data, &self.mls_pub);
        let mut ops = vec![tail::Op::Set { key: b"beekem".to_vec(), value: json!({ "cred": hex(&cred), "leaf": hex(&leaf) }).to_string().into_bytes() }];
        if !nodes.is_empty() {
            ops.push(tail::Op::Set { key: b"nodes".to_vec(), value: serde_json::to_vec(&nodes.iter().map(|n| hex(n)).collect::<Vec<_>>()).unwrap() });
        }
        let (seq, h) = o.prepare(ops).unwrap();
        let p = craftec_register_contract::wire::Params::parse(&o.params).unwrap();
        o.commit(key.sign(&p.signed_message(false, seq, &h)).to_bytes()).unwrap();
        net.st.insert(o.id_bytes(), o.state());
    }
}

/// The OWNER's tree made (its first operation in its device's `pub-keyops`, listed in its catalog); B's card names its
/// member. `acts` as the test says.
fn bee_world(acts: &[serde_json::Value], b: &Person) -> World {
    let mut w = world(acts, &[]);
    let me_key = SigningKey::from_bytes(&SEED);
    let cred = identity::credential(&w.owner.did, &w.member, &w.owner.mls_pub, &craftworks_account::owner_seed(&w.owner.entropy).unwrap());
    w.owner.publish_member(&mut w.net, &[cred]);
    b.publish_member(&mut w.net, &[]);
    let (_, first) = groups::Group::create(&SPACE, w.owner.bee()).unwrap();
    let s0 = identity::upkeep_mandate(&w.h, &w.member).map(|_| ()).unwrap();
    let _ = s0;
    let ep: Vec<(u64, [u8; 32])> = Vec::new();
    let cat = table::table_name(&SPACE, "tables");
    let mut c = Feed::new(&me_key, &cat, &cat, &[(0, epoch0(&w))]);
    c.put(&table::table_name(&SPACE, "acts"), r#"{"b":1}"#);
    c.put(&table::table_name(&SPACE, "pub-keyops"), r#"{"b":1}"#);
    w.net.seed_tail(&c.o);
    let name = table::table_name(&SPACE, "pub-keyops");
    let mut k = Feed::new(&me_key, &name, &name, &ep);
    k.o.public = true;
    k.put(&hex(&sha2_of(&first)), &hex(&first));
    w.net.seed_tail(&k.o);
    w
}
fn epoch0(w: &World) -> [u8; 32] {
    identity::upkeep_mandate(&w.h, &w.member).unwrap().1[0].clone().state.len();
    craftworks_mls::Account::load(craftworks_mls::Rule::Space(SPACE), &w.mandate().state).unwrap().epoch_secret().unwrap()
}
fn sha2_of(b: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(b).into()
}
/// Every operation in the owner device's `pub-keyops`, as a reader takes them.
fn owner_ops(w: &World) -> Vec<Vec<u8>> {
    let name = table::table_name(&SPACE, "pub-keyops");
    let mut o = data::Open::new(TAIL, &w.member, &name);
    o.public = true;
    o.absorb(&w.net.st[&o.id_bytes()]);
    let Ok(data::Step::Ready(rows)) = o.opened() else { panic!("the feed reads") };
    rows.values().filter_map(|env| craftworks_feed::open(env).and_then(|r| r.value)).map(|v| unhex(std::str::from_utf8(&v).unwrap())).collect()
}
/// B's page: merges every operation (no check needed for this reading: the operations are the node's own).
struct Anyone;
impl groups::Check for Anyone {
    fn account_of(&self, _: &[u8; 32]) -> Option<String> {
        Some("x".into())
    }
    fn listed(&self, _: &str) -> bool {
        true
    }
    fn allows(&self, _: &[u8; 32], _: bool, _: &[u8; 32]) -> bool {
        true
    }
    fn creator(&self) -> Option<String> {
        Some("x".into())
    }
}
fn merged(p: &Person, ops: &[Vec<u8>]) -> groups::Group {
    let mut g = groups::Group::empty(&SPACE, p.bee());
    let mut pending = ops.to_vec();
    for _ in 0..8 {
        pending.retain(|op| g.merge(op, &Anyone) == groups::Merged::Waiting);
    }
    g
}

/// FLOW 1 + 3 — B LISTED, every page closed: the OWNER's NODE adds B (by the act that lets it in) and changes the key;
/// B, reading the operations, is placed and derives the generation the node kept. The next round: nothing.
/// Control — B NOT listed: nobody added.
#[test]
fn k5_the_node_adds_a_listed_member_every_page_closed() {
    let b = person(0x52);
    let mut w = bee_world(&[json!({ "act": "added", "did": b.id(), "at": 10 })], &b);
    let before = owner_ops(&w).len();
    w.run();
    let ops = owner_ops(&w);
    assert_eq!(ops.len(), before + 2, "an add and a key change: {:?}", w.said());
    let mut gb = merged(&b, &ops);
    assert!(gb.placed(), "B placed: {:?}", w.said());
    let mut go = merged(&w.owner, &ops);
    let (n, k) = go.current_now().expect("a single head");
    assert!(gb.generations().contains(&(n, k)), "B reaches the generation the owner writes with");
    assert_eq!(identity::epoch_secret(&w.h, &w.member, SPACE, n), Some(k), "the node kept it as the space's epoch");
    let again = w.run();
    assert_eq!(owner_ops(&w).len(), ops.len(), "nothing more: {:?} {:?}", w.said(), World::writes(&again));

    // CONTROL: no act lets B in — nothing added.
    let mut c = bee_world(&[], &b);
    c.run();
    assert_eq!(owner_ops(&c).len(), 1, "nobody added: {:?}", c.said());
    assert!(!merged(&b, &owner_ops(&c)).placed());
}

/// FLOW 6 — B placed, then the acts put B out: the node removes B and changes the key; B does not reach the new
/// generation. Control: the owner does.
#[test]
fn k5_the_node_takes_out_a_member_the_list_put_out() {
    let b = person(0x53);
    let added = json!({ "act": "added", "did": b.id(), "at": 10 });
    let mut w = bee_world(&[added.clone()], &b);
    w.run();
    let placed = owner_ops(&w);
    assert!(merged(&b, &placed).placed());
    // The acts now put B out (the owner's device writes the removal).
    let me_key = SigningKey::from_bytes(&SEED);
    let mut a = Feed::new(&me_key, &table::table_name(&SPACE, "acts"), &blinded("acts"), &[(0, epoch0(&w))]);
    a.put("a0", &added.to_string());
    a.put("a1", &json!({ "act": "remove", "did": b.id(), "at": 20 }).to_string());
    w.net.seed_tail(&a.o);
    w.run();
    let ops = owner_ops(&w);
    assert_eq!(ops.len(), placed.len() + 2, "a removal and a key change: {:?}", w.said());
    let (n, k) = merged(&w.owner, &ops).current_now().expect("a single head");
    assert!(!merged(&b, &ops).generations().iter().any(|(x, _)| *x == n), "B does not reach the new generation");
    assert!(merged(&w.owner, &ops).generations().contains(&(n, k)), "control: the owner does");
}

/// fx 10-17: A CARD FLUSHED into blocks still names its member to the node — B added. (Before: the node read such a
/// card as having none, refused the owner's own placement and changed nothing.)
#[test]
fn k5_a_card_kept_in_blocks_is_read_whole() {
    let b = person(0x54);
    let mut w = bee_world(&[json!({ "act": "added", "did": b.id(), "at": 10 })], &b);
    b.publish_member_flushed(&mut w.net, &[]);
    let cred = identity::credential(&w.owner.did, &w.member, &w.owner.mls_pub, &craftworks_account::owner_seed(&w.owner.entropy).unwrap());
    w.owner.publish_member_flushed(&mut w.net, &[cred]);
    let before = owner_ops(&w).len();
    w.run();
    assert_eq!(owner_ops(&w).len(), before + 2, "B added from cards kept in blocks: {:?}", w.said());
    assert!(merged(&b, &owner_ops(&w)).placed());
}

/// fx 10-18: a space whose list names many accounts with NO card (gone; never made) — a round asks for at most
/// CARDS_A_ROUND of their cards, the next for the rest, and none again within the hour. (Before: every round re-read
/// everyone the list ever named — past the node's per-round cap, the delegate out of time on the live network.)
#[test]
fn k5_cards_a_round_are_bounded_and_not_asked_again() {
    let b = person(0x55);
    let gone: Vec<Person> = (0..20u8).map(|i| person(0x70 + i)).collect();
    let mut acts = vec![json!({ "act": "added", "did": b.id(), "at": 10 })];
    acts.extend(gone.iter().map(|p| json!({ "act": "added", "did": p.id(), "at": 11 })));
    let mut w = bee_world(&acts, &b);
    let logs_of = |asked: &[Io]| asked.iter().filter(|x| matches!(x, Io::Get { id: g, .. } if gone.iter().any(|p| *g == id(IDLOG, &p.did)))).count();
    let r1 = w.run();
    assert!(logs_of(&r1) <= 8, "round 1 asked {} key logs of the gone", logs_of(&r1));
    let r2 = w.run();
    let r3 = w.run();
    let r4 = w.run();
    let asked: usize = [&r2, &r3, &r4].iter().map(|r| logs_of(r)).sum::<usize>() + logs_of(&r1);
    assert!(asked <= 20, "each asked once within the hour: {asked}");
    assert!(logs_of(&r4) == 0, "after all were asked: none again ({}), {:?}", logs_of(&r4), w.said());
    assert!(merged(&b, &owner_ops(&w)).placed(), "B, who has a card, added meanwhile: {:?}", w.said());
}

/// 10-18, live: A BEEKEM SPACE whose key operations read EMPTY (its feeds not found from this node) is never taken for
/// an MLS space — the round says its operations did not read, and does not go to the MLS catch-up (a node's round
/// there waited on epoch 149's log, which nobody writes, and stalled 33 times). Control — a space with no generation
/// held (MLS) and the same empty read: the MLS path, nothing said of BeeKEM.
#[test]
fn k5_a_beekem_space_whose_operations_read_empty_is_not_caught_up_as_mls() {
    let b = person(0x55);
    let mut w = bee_world(&[json!({ "act": "added", "did": b.id(), "at": 10 })], &b);
    w.run();
    let (n, _) = merged(&w.owner, &owner_ops(&w)).current_now().expect("a single head");
    assert!(n >= groups::FIRST_GENERATION, "the node holds a generation ({n})");
    // The owner device's `pub-keyops` not found from this node.
    let name = table::table_name(&SPACE, "pub-keyops");
    let mut o = data::Open::new(TAIL, &w.member, &name);
    o.public = true;
    assert!(w.net.st.remove(&o.id_bytes()).is_some(), "the feed was there");
    w.run();
    let said = w.said();
    assert!(said.iter().any(|s| s.starts_with("keys-ops-unread: ") && s.contains("read empty though it holds a BeeKEM generation")), "said: {said:?}");

    // CONTROL: no generation held (an MLS space), the same empty read — the MLS path, nothing said of BeeKEM.
    let mut c = world(&[], &[]);
    c.run();
    let said = c.said();
    assert!(!said.iter().any(|s| s.contains("BeeKEM generation")), "said: {said:?}");
}

/// 10-19: A LISTED MEMBER's CARD THAT DOES NOT READ while their DEVICES CONTRACT does is not a short list — the one step
/// (`account::devices_step`) asks the contract before saying anything (the node had said "short" first, and decided
/// nothing for days). The removal by the acts goes ahead. Control (`a_short_read_decides_nothing`): Y's key log never
/// answers — short, nothing moves.
#[test]
fn an_unread_card_with_a_devices_contract_is_not_a_short_list() {
    let (x, y) = (person(0x35), person(0x36));
    let mut w = world(&[json!({ "act": "added", "did": x.id(), "at": 10 }), json!({ "act": "remove", "did": x.id(), "at": 20 })], &[&x, &y]);
    x.publish(&mut w.net, &[]);
    // Y: the key log and a devices contract naming Y's device — NO card.
    w.net.st.insert(id(IDLOG, &y.did), y.log.encode());
    let owner = craftworks_account::owner_seed(&y.entropy).unwrap();
    let cred = identity::credential(&y.did, &[0x77; 32], &[2u8; 32], &owner);
    let dev = craftworks_devices_contract::params(&y.did, &contract_keys::code_hash(IDLOG));
    w.net.st.insert(id(DEVICES, &dev), craftworks_devices_contract::encode(&[cred]));
    w.run();
    let said = w.said();
    assert!(!said.iter().any(|s| s.starts_with("list-short")), "not short: {said:?}");
    assert!(!w.leaves().contains(&x.id()), "the removal went ahead: {said:?}");
}

/// 10-19: ON A SHORT READ an entry the list check TAKES is applied all the same (its answer never changes as more is
/// read — `guards::entry_outcome`, the page's too); nobody is taken out. Y, a member, never answers (the read short).
/// Before: the node decided nothing on a short read, B's entry left waiting.
#[test]
fn a_short_read_still_applies_a_listed_entry() {
    let (b, y) = (person(0x57), person(0x58));
    let acts = [json!({ "act": "member", "did": b.id(), "at": 10 })];
    let mut w = world(&acts, &[&y]);
    b.publish(&mut w.net, &[]); // (Y's key log never answers: the read is short.)
    let e = w.mandate().epoch;
    let info = w.group().group_info().unwrap();
    let (bg, b_in) = b.enter(&info, b"");
    w.door_entry(e, &b_in);
    w.run();
    assert!(w.said().iter().any(|s| s.starts_with("list-short")), "short: {:?}", w.said());
    assert_eq!(w.mandate().epoch, e + 1, "B's entry applied: {:?}", w.said());
    assert_eq!(w.group().epoch_secret().unwrap(), bg.epoch_secret().unwrap(), "the node follows B's entry");
}
