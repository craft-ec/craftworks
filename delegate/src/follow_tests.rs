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

/// (The tests' one host: `testhost::Map`.)
use crate::testhost::Map as Secrets;

use craftworks_identity::hex;
fn unhex(s: &str) -> Vec<u8> {
    craftworks_identity::unhex(s).expect("hex")
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
    bee_world_with(acts, b, false)
}
/// …its first operation stored as a public FILE of the space, its row naming it (as a page stores a large one).
fn bee_world_with(acts: &[serde_json::Value], b: &Person, as_file: bool) -> World {
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
    if as_file {
        let refd = store_file(&mut w.net, &first);
        k.put(&hex(&sha2_of(&first)), &json!({ "file": refd }).to_string());
    } else {
        k.put(&hex(&sha2_of(&first)), &hex(&first));
    }
    w.net.seed_tail(&k.o);
    w
}

/// A PUBLIC FILE stored as a page stores one (its content's key, every part coded, its index): its reference.
fn store_file(net: &mut Net, bytes: &[u8]) -> serde_json::Value {
    use craftworks_files as files;
    let key = files::content_key(blake3::hash(bytes).as_bytes(), None);
    let plan = files::Plan::of(bytes.len() as u64);
    let per = plan.chunk * files::GEN;
    let mut stored = Vec::new();
    for g in 0..plan.gens {
        let part = &bytes[(g as usize * per).min(bytes.len())..((g as usize + 1) * per).min(bytes.len())];
        let pcs = files::encode(&key, &plan, g, part, files::EXTRA);
        for (_, p) in &pcs {
            net.st.insert(id(PIECE, &p.address), [&[2u8][..], &[0u8; 32], &p.state].concat());
        }
        stored.push(pcs.iter().map(|(j, p)| (*j, p.hash())).collect());
    }
    let (index, root) = files::index_hashed(&key, &plan, &stored);
    for p in index {
        net.st.insert(id(PIECE, &p.address), [&[2u8][..], &[0u8; 32], &p.state].concat());
    }
    craftworks_rules::rekey::file_ref(&hex(root), Some(&hex(SPACE)), &key, &root, &[0; 32], bytes.len() as u64, "file", "application/octet-stream")
}
/// A file READ BACK as a reader does, from the `files` crate alone (the test's own reader, not the node's).
fn read_back(net: &Net, refd: &serde_json::Value) -> Option<Vec<u8>> {
    use craftworks_files as files;
    let un = |k: &str| craftworks_identity::unhex32(refd[k].as_str()?);
    let (key, root_hash) = (un("key")?, un("root")?);
    let piece = |a: [u8; 32]| net.st.get(&id(PIECE, &a)).filter(|st| st.len() > 33).map(|st| st[33..].to_vec());
    let root = files::Root::open(&key, &root_hash, &piece(files::hashed_address(&key, &root_hash))?).ok()?;
    let mut out = Vec::new();
    for g in 0..root.plan.gens {
        let leaf = root.children[files::Root::leaf_of(g) as usize];
        let listed = files::open_leaf(&key, &root.plan, files::Root::leaf_of(g), &leaf, &piece(files::hashed_address(&key, &leaf))?).ok()?[files::Root::gen_slot(g)].clone();
        let mut d = files::Decoder::new(&key, &root.plan, g, listed.clone());
        for (j, _) in &listed {
            if let Some(st) = piece(files::fragment_address(&key, g, *j)) {
                let _ = d.add(*j, &st);
            }
        }
        out.extend(d.plain().ok()?);
    }
    Some(out)
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
    // (A row is its operation, or names its file: read back.)
    rows.values()
        .filter_map(|env| craftworks_feed::open(env).and_then(|r| r.value))
        .map(|v| match craftworks_rules::members::keyop_row(std::str::from_utf8(&v).unwrap()).expect("a key operation's row") {
            craftworks_rules::members::KeyopRow::Op(o) => o,
            craftworks_rules::members::KeyopRow::File(f) => read_back(&w.net, &f).expect("its file reads back"),
        })
        .collect()
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
    // (Every person these tests make: each one's published leaf — an add is taken only with it.)
    fn leaf_of(&self, m: &[u8; 32]) -> Option<[u8; 32]> {
        [0x35, 0x36, 0x52, 0x53, 0x54, 0x55, 0x61].into_iter().map(|e| person(e).bee()).find(|a| a.id.0.to_bytes() == *m).map(|a| a.leaf_key().to_bytes())
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
    // (The creation alone has no root: its one key change made — `Group::due`, a rootless tree — and nobody added.)
    assert_eq!(owner_ops(&c).len(), 2, "nobody added, one key change: {:?}", c.said());
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
/// CARDS_A_ROUND of their cards, the next for the rest, and none again before `members::CARD_ASK_AGAIN_MS`. (Before: every round re-read
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
    assert!(asked <= 20, "each asked once within CARD_ASK_AGAIN_MS: {asked}");
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



/// Every round of the node's (upkeep's, the re-key's, the reads') is STUCK by one rule (`upkeep::stuck`): not moved for
/// `STUCK` wake-ups, or for `STUCK_MS` of the node's time even when no wake-up came.
#[test]
fn one_stuck_rule_by_wakeups_or_time() {
    use craftworks_identity::{Host, UPKEEP_CLOCK, UPKEEP_WAKEUPS};
    let mut h = crate::testhost::Map(Default::default());
    let clock = |h: &mut crate::testhost::Map, secs: u64, at: u64| h.set_secret(UPKEEP_CLOCK, &[secs.to_le_bytes(), at.to_le_bytes()].concat());
    clock(&mut h, 1000, 10);
    h.set_secret(UPKEEP_WAKEUPS, &10u64.to_le_bytes());
    let (w, ms) = (10, craftworks_identity::upkeep_now_ms(&h));
    assert!(!crate::upkeep::stuck(&h, w, ms), "just moved");
    h.set_secret(UPKEEP_WAKEUPS, &(10 + crate::upkeep::STUCK - 1).to_le_bytes());
    clock(&mut h, 1000, 10 + crate::upkeep::STUCK - 1);
    assert!(!crate::upkeep::stuck(&h, w, ms), "one wake-up short, no time passed");
    h.set_secret(UPKEEP_WAKEUPS, &(10 + crate::upkeep::STUCK).to_le_bytes());
    clock(&mut h, 1000, 10 + crate::upkeep::STUCK);
    assert!(crate::upkeep::stuck(&h, w, ms), "STUCK wake-ups");
    h.set_secret(UPKEEP_WAKEUPS, &10u64.to_le_bytes());
    clock(&mut h, 1000 + 3 * 60, 10);
    assert!(crate::upkeep::stuck(&h, w, ms), "three minutes with no wake-up");
}


/// K5, a key operation STORED AS A FILE (a page stores a large one so: a row holds 64 KB): the node READS it
/// (`file_io::FileRead`) and merges it — here the owner's first, so without it the node is in no tree. B listed:
/// added. Control — the file's pieces gone: the node says the file did not read, and adds nobody.
#[test]
fn k5_a_key_operation_stored_as_a_file_is_read_and_merged() {
    let b = person(0x55);
    let acts = [json!({ "act": "added", "did": b.id(), "at": 10 })];
    let mut w = bee_world_with(&acts, &b, true);
    w.run();
    let said = w.said();
    assert!(said.iter().any(|s| s.starts_with("keys-reconciled: ") && s.contains("1 added")), "said: {said:?}");
    assert!(merged(&b, &owner_ops(&w)).placed(), "B placed: {said:?}");

    let mut c = bee_world_with(&acts, &b, true);
    c.net.st.retain(|_, st| !(st.len() > 33 && st[0] == 2 && st[1..33] == [0u8; 32]));
    c.run();
    let said = c.said();
    assert!(said.iter().any(|s| s.starts_with("keys-ops-unread: ") && s.contains("did not read")), "said: {said:?}");
    assert!(!said.iter().any(|s| s.contains("1 added")), "nobody added: {said:?}");
}

/// K5, a LARGE key operation made by the node (here every one, by the test's switch): STORED AS A FILE
/// (`file_io::FileWrite`, as a page's put), its row naming it, the file listed in the space's `files` table — and the
/// next round reads it back (in step, nothing written). On a network where a contract made here does not reach the
/// others (`upkeep::creates`; freenet ≤ 0.2.141): the change is left to a page, and no generation of it kept.
#[test]
fn k5_a_large_operation_is_stored_as_a_file_and_read_back() {
    let b = person(0x55);
    let acts = [json!({ "act": "added", "did": b.id(), "at": 10 })];
    crate::upkeep::ALL_AS_FILES.with(|f| f.set(true));
    // Where nothing made here reaches the others (freenet ≤ 0.2.141): left to a page, nothing kept.
    crate::upkeep::CREATES.with(|f| f.set(false));
    let mut c = bee_world(&acts, &b);
    let before = identity::kept_epochs(&c.h, &c.member, &Some(SPACE)).len();
    c.run();
    let said = c.said();
    assert!(said.iter().any(|s| s.starts_with("keys-wait: ") && s.contains("too large for a row")), "said: {said:?}");
    assert_eq!(identity::kept_epochs(&c.h, &c.member, &Some(SPACE)).len(), before, "no generation of it kept");
    assert!(!merged(&b, &owner_ops(&c)).placed(), "B not added here");
    // Where it does reach them.
    crate::upkeep::CREATES.with(|f| f.set(true));
    let mut w = bee_world(&acts, &b);
    w.run();
    crate::upkeep::ALL_AS_FILES.with(|f| f.set(false));
    let said = w.said();
    assert!(said.iter().any(|s| s.starts_with("keys-reconciled: ") && s.contains("stored as files")), "said: {said:?}");
    // B, reading every row (a file read back by the test's own reader), is placed.
    assert!(merged(&b, &owner_ops(&w)).placed(), "B placed: {said:?}");
    // Listed in the space's `files` table, as a page's put lists it.
    let name = table::table_name(&SPACE, "files");
    let label = identity::blind_name(&identity::space_table_key(&SPACE, &name), &name);
    let mut o = data::Open::new(TAIL, &w.member, &label);
    o.set_table_key(identity::space_table_key(&SPACE, &name));
    assert!(w.net.st.contains_key(&o.id_bytes()), "its files feed written: {:?}", w.said());
    // The next round: read back, in step.
    w.run();
    let said = w.said();
    crate::upkeep::CREATES.with(|f| f.set(crate::upkeep::NEW_CONTRACTS_REACH_THE_NETWORK));
    assert!(said.iter().any(|s| s.starts_with("keys-in-step: ")), "said: {said:?}");
}

/// THE ROUNDS' MODEL TEST (FOUNDATION "The node's rounds": R2, R3) — over the real follow round (and the files round
/// beside it: every answer reaches both), the node's clock a second on at every event: before EVERY answer, a STRAY of
/// every kind (a Got, a Put ok and refused, an Updated ok and refused — of a contract no step asked for); then, seeded,
/// the answer delivered or never coming. R2: a stray changes no round — not its step, not its moved time. R3: a round
/// whose answers stopped is dropped as stuck within `STUCK` wake-ups, its step recorded.
#[test]
fn rounds_model_strays_move_nothing_and_a_stalled_round_is_dropped() {
    use craftworks_identity::Host;
    let rounds = |h: &crate::testhost::Map| (h.get_secret(upkeep::ROUND), h.get_secret(crate::rekey_round::ROUND));
    let mut checked = (0, 0);
    for seed in 1..=16u64 {
        let b = person(0x55);
        let mut w = bee_world(&[json!({ "act": "added", "did": b.id(), "at": 10 })], &b);
        let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        let mut now = NOW;
        let mut tick = |h: &mut crate::testhost::Map| {
            now += 1000;
            identity::upkeep_set_clock(h, now / 1000);
            now
        };
        let t = tick(&mut w.h);
        let mut todo = upkeep::woke(&mut w.h, t);
        let mut dropped = false;
        let mut steps = 0;
        while let Some(io) = (!todo.is_empty()).then(|| todo.remove(0)) {
            steps += 1;
            assert!(steps < 5000, "seed {seed}: the round ends");
            for kind in 0..5u8 {
                let stray_id = {
                    let mut x = [0u8; 32];
                    x[..8].copy_from_slice(&next().to_le_bytes());
                    x[31] = 0xEE;
                    x
                };
                let stray = match kind {
                    0 => Reply::Got { id: stray_id, state: Some(vec![1, 2, 3]) },
                    1 => Reply::Put { id: stray_id, ok: true },
                    2 => Reply::Put { id: stray_id, ok: false },
                    3 => Reply::Updated { id: stray_id, ok: true },
                    _ => Reply::Updated { id: stray_id, ok: false },
                };
                let before = rounds(&w.h);
                let t = tick(&mut w.h);
                let out = upkeep::replied(&mut w.h, stray.clone(), t);
                assert!(out.is_empty(), "seed {seed}: a stray {stray:?} sent {out:?}");
                assert_eq!(rounds(&w.h), before, "seed {seed}: a stray {stray:?} changed a round (R2)");
                checked.0 += 1;
            }
            let reply = w.net.answer(&io);
            // The answer NEVER COMES (at most once a run, seeded): the rest of the round waits on it.
            if !dropped && next() % 6 == 0 {
                dropped = true;
                todo.clear();
                continue;
            }
            let t = tick(&mut w.h);
            todo.extend(upkeep::replied(&mut w.h, reply, t));
        }
        // R3: a round left waiting is dropped within STUCK wake-ups (by wake-ups alone: the clock not moved on).
        if dropped && rounds(&w.h).0.is_some_and(|b| !b.is_empty()) {
            let w0 = u64::from_le_bytes(w.h.get_secret(identity::UPKEEP_WAKEUPS).unwrap().try_into().unwrap());
            w.h.set_secret(identity::UPKEEP_WAKEUPS, &(w0 + crate::upkeep::STUCK).to_le_bytes());
            let _ = upkeep::woke(&mut w.h, now);
            assert!(w.said().iter().any(|s| s.starts_with("stalled: ")), "seed {seed}: a round left waiting was not dropped (R3): {:?}", w.said());
            checked.1 += 1;
        }
    }
    assert!(checked.0 >= 100 && checked.1 >= 3, "the model ran its cells: {checked:?} (strays, stalls)");
    println!("rounds model: {} strays, {} stalls checked", checked.0, checked.1);
}

/// K8.6 (owner 10-24: MLS-era support dropped): a space with NO BeeKEM tree is not followed — nothing asked of its MLS
/// epoch logs or doors (the catch-up read the NEXT epoch's log every round: a contract nobody made unless someone
/// committed, a 60–75 s not-found parking the delegate — 10-26, the owner's node), and said `not-migrated`.
#[test]
fn an_mls_era_space_is_not_followed() {
    let (x, y) = (person(0x35), person(0x36));
    let mut w = world(&[json!({ "act": "added", "did": x.id(), "at": 10 }), json!({ "act": "remove", "did": x.id(), "at": 20 })], &[&x, &y]);
    x.publish(&mut w.net, &[]);
    let e = w.mandate().epoch;
    let asked = w.run();
    assert_eq!(World::writes(&asked), 0, "nothing written: {:?}", w.said());
    assert_eq!(w.mandate().epoch, e, "its MLS group not moved");
    assert!(w.said().iter().any(|s| s.starts_with("not-migrated")), "said: {:?}", w.said());
}

/// DRIVEN BY CHANGE, NOT THE CLOCK (10-30): after a pass, a wake-up with nothing changed asks nothing; a subscribed
/// feed's notification begins a pass at once; and the deadline (`rekey_round::EVERY` wake-ups with none) still sweeps
/// — a lost notification is never lost for good.
#[test]
fn the_follower_runs_on_a_change_and_on_its_deadline_never_each_wake_up() {
    let b = person(0x52);
    let mut w = bee_world(&[json!({ "act": "added", "did": b.id(), "at": 10 })], &b);
    w.run();
    let at = u64::from_le_bytes(w.h.get_secret(identity::UPKEEP_WAKEUPS).unwrap().try_into().unwrap());
    // The next wake-up, one later — nothing changed: nothing asked.
    w.h.set_secret(identity::UPKEEP_WAKEUPS, &(at - 9).to_le_bytes());
    assert!(upkeep::woke(&mut w.h, NOW).is_empty(), "a quiet wake-up asks nothing");
    // A feed changed: a pass at once.
    assert!(!upkeep::notified(&mut w.h).is_empty(), "a change begins a pass");
    // (That pass ended; the deadline reached with none since: a pass.)
    crate::upkeep::tests_drop_round(&mut w.h);
    w.h.set_secret(identity::UPKEEP_WAKEUPS, &(at + crate::rekey_round::EVERY + 1).to_le_bytes());
    assert!(!upkeep::woke(&mut w.h, NOW).is_empty(), "the deadline sweeps");
}

/// THE CODES WITHOUT A PAGE (10-30): a node no page has handed the contracts' code holds this build's own — the files
/// the site's `*-wasm` packages are published from — so its rounds never stop waiting for a page.
#[test]
fn a_node_holds_the_contracts_code_with_no_page() {
    let h = crate::testhost::Map::default();
    let held = crate::rekey_round::Held::of(&h).expect("held with no page");
    assert_eq!(held.bag, contract_keys::code_hash(include_bytes!("../../contracts/bag.wasm")));
    assert_eq!(held.idlog, contract_keys::code_hash(include_bytes!("../../contracts/idlog.wasm")));
}
