//! K8.5 THE NODE KEEPS THE ACCOUNT's TREE (`account_round`): a real account (key log, devices contract, this node's
//! device's MLS state), its channel as pages write it (device A's root, both devices' signed leaves), and a scripted
//! network. The node, A's page away, adds B (one key change), escrows the generations, keeps them as the account's.

use super::account_round;
use super::upkeep::{self, Code, Io, Reply};
use craftworks_data as data;
use craftworks_groups::{Account as Member, Group};
use craftworks_identity::{self as identity, hex, Host, Request};
use ed25519_dalek::{Signer, SigningKey};
use std::collections::HashMap;

const TAIL: &[u8] = b"the tail contract's code";
const BAG: &[u8] = b"the bag contract's code";
const SEALED: &[u8] = b"the sealed contract's code";
const BLOCK: &[u8] = b"the block contract's code";
const IDLOG: &[u8] = b"the key log contract's code";
const DEVICES: &[u8] = b"the devices contract's code";
const PIECE: &[u8] = b"the piece contract's code";
const SEED: [u8; 32] = [0x31; 32];
use crate::testhost::Map as Secrets;

fn id(code: &[u8], params: &[u8]) -> [u8; 32] {
    upkeep::id_of(&contract_keys::code_hash(code), params)
}

#[derive(Default)]
struct Net {
    st: HashMap<[u8; 32], Vec<u8>>,
    params: HashMap<[u8; 32], Vec<u8>>,
}
impl Net {
    fn answer(&mut self, io: &Io) -> Reply {
        use craftec_register_contract::wire::Params;
        match io {
            Io::Get { id, .. } => Reply::Got { id: *id, state: self.st.get(id).cloned() },
            Io::Put { code, params, state } => {
                let c: &[u8] = match code {
                    Code::Tail => TAIL,
                    Code::Bag => BAG,
                    Code::Sealed => SEALED,
                    Code::Piece => PIECE,
                };
                let i = id(c, params);
                self.st.insert(i, state.clone());
                Reply::Put { id: i, ok: true }
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
}

struct World {
    h: Secrets,
    member: [u8; 32],
    net: Net,
    did: [u8; 32],
    data: [u8; 32],
    a: Member,
    b: Member,
    root: Vec<u8>,
}

/// The channel as a page reads it: its rows (key → value).
fn channel_rows(w: &World) -> std::collections::BTreeMap<String, String> {
    let data_pub = SigningKey::from_bytes(&w.data).verifying_key().to_bytes();
    let name = account_round::channel_name(&w.did);
    let mut o = data::Open::new(TAIL, &data_pub, &name);
    o.set_table_key(identity::table_key(&w.data, &name, 0));
    o.absorb(&w.net.st[&o.id_bytes()]);
    let Ok(data::Step::Ready(rows)) = o.opened() else { panic!("the channel opens") };
    rows.into_iter().map(|(k, v)| (String::from_utf8(k).unwrap(), String::from_utf8(v).unwrap())).collect()
}

fn world(b_current: bool) -> World {
    let entropy = [0x77u8; 16];
    let ev = craftworks_account::inception(&entropy).unwrap();
    let did = ev.id();
    let log = craftworks_idlog_contract::Log { events: vec![ev] };
    let owner = craftworks_account::owner_seed(&entropy).unwrap();
    let data_seed = craftworks_account::data_seed(&entropy).unwrap();
    let mut h = Secrets::default();
    identity::serve(&mut h, Request::Provision { seed: SEED, did, pin: "246810".into(), data: data_seed.to_vec() }, [9; 32]);
    let member = SigningKey::from_bytes(&SEED).verifying_key().to_bytes();
    // THIS NODE's DEVICE (A): its MLS state kept here — its signer is A's member in the account's tree.
    let ident = craftworks_mls::AccountIdentity { did, owners: log.events.iter().map(|e| e.key).collect() };
    let mut mls = craftworks_mls::Account::create(ident, &owner, &member).unwrap();
    let state = mls.save().unwrap();
    h.set_secret(&[identity::MLS, &member[..]].concat(), &state);
    let a_seed = craftworks_mls::signer_seed_of(&state).unwrap();
    let a = Member::of(&a_seed);
    let b = Member::of(&[0xB2; 32]);
    // Upkeep's codes, its randomness, its clock.
    for (k, v) in [(identity::UPKEEP_TAIL, TAIL), (identity::UPKEEP_BAG, BAG), (identity::UPKEEP_SEALED, SEALED), (identity::UPKEEP_PIECE, PIECE)] {
        h.set_secret(k, v);
    }
    h.set_secret(identity::UPKEEP_BLOCK, &contract_keys::code_hash(BLOCK));
    h.set_secret(identity::UPKEEP_DEVICES, &contract_keys::code_hash(DEVICES));
    h.set_secret(identity::UPKEEP_IDLOG, &contract_keys::code_hash(IDLOG));
    h.set_secret(identity::UPKEEP_WAKEUPS, &30u64.to_le_bytes());
    // (ENLISTED as a page enlists it: its inbox watch.)
    identity::serve(&mut h, Request::UpkeepWatch { inbox: [0x1B; 32], seed: [7; 32], now: 1_800_000_000 }, [9; 32]);
    // THE DEVICES LIST: A, and B (current or not).
    let mut net = Net::default();
    net.st.insert(id(IDLOG, &did), log.encode());
    let mut creds = vec![identity::credential(&did, &member, &a.id.0.to_bytes(), &owner)];
    if b_current {
        creds.push(identity::credential(&did, &[0xB1; 32], &b.id.0.to_bytes(), &owner));
    }
    net.st.insert(id(DEVICES, &craftworks_devices_contract::params(&did, &contract_keys::code_hash(IDLOG))), craftworks_devices_contract::encode(&creds));
    // THE CHANNEL as pages write it: A's root, both leaves.
    let (_, root) = Group::create(&did, a.clone()).unwrap();
    let data_key = SigningKey::from_bytes(&data_seed);
    let name = account_round::channel_name(&did);
    let mut o = data::Open::new(TAIL, &data_key.verifying_key().to_bytes(), &name);
    o.set_table_key(identity::table_key(&data_seed, &name, 0));
    let leaf = |m: &Member| {
        let (l, s) = m.signed_leaf();
        serde_json::json!({ "leaf": hex(&l), "sig": hex(&s) }).to_string().into_bytes()
    };
    let rows = vec![
        (format!("o/{}", craftworks_rules::members::keyop_key(&root)).into_bytes(), hex(&root).into_bytes()),
        (format!("d/{}", hex(&a.id.0.to_bytes())).into_bytes(), leaf(&a)),
        (format!("d/{}", hex(&b.id.0.to_bytes())).into_bytes(), leaf(&b)),
    ];
    let (seq, hh) = o.prepare_rows(&rows).unwrap();
    let p = craftec_register_contract::wire::Params::parse(&o.params).unwrap();
    o.commit(data_key.sign(&p.signed_message(false, seq, &hh)).to_bytes()).unwrap();
    net.params.insert(o.id_bytes(), o.params.clone());
    net.st.insert(o.id_bytes(), o.state());
    World { h, member, net, did, data: data_seed, a, b, root }
}

/// One round driven to its end against the network; every message sent.
fn run(w: &mut World) -> Vec<Io> {
    let mut sent = Vec::new();
    let mut todo = account_round::start(&mut w.h, w.member);
    while let Some(io) = (!todo.is_empty()).then(|| todo.remove(0)) {
        let reply = w.net.answer(&io);
        sent.push(io);
        todo.extend(account_round::replied(&mut w.h, &reply));
    }
    sent
}

/// THE NODE ADDS A CURRENT DEVICE: A's node, its page away, adds B (B's leaf signed, B current) with one key change;
/// the new generation escrowed to the words (`g/`) and for the MLS epoch (`m/`); the generations kept as the account's
/// epochs. B, from the channel alone, is placed. Run again: nothing more is written.
#[test]
fn the_node_adds_a_current_device_to_the_accounts_tree() {
    let mut w = world(true);
    let sent = run(&mut w);
    assert_eq!(sent.iter().filter(|x| matches!(x, Io::Update { .. })).count(), 1, "one channel step: {sent:?}");
    let rows = channel_rows(&w);
    let ops: Vec<Vec<u8>> = rows.iter().filter(|(k, _)| k.starts_with("o/")).map(|(_, v)| identity::unhex(v).unwrap()).collect();
    assert!(ops.len() >= 3, "the root, B's add, a key change: {}", ops.len());
    // B, from the channel's operations alone, is placed.
    let mut gb = Group::empty(&w.did, w.b.clone());
    let check = craftworks_groups::AccountCheck { current: vec![w.a.id.0.to_bytes(), w.b.id.0.to_bytes()], ever: vec![], writers: vec![w.a.id.0.to_bytes()] };
    gb.merge_all(&ops, &check);
    assert!(gb.placed(), "B placed by the node");
    // Every generation escrowed (`g/`) and MLS-sealed (`m/`), and kept here as the account's epoch.
    let gens = gb.generations();
    for (n, k) in &gens {
        assert!(rows.contains_key(&format!("g/{n}")), "g/{n} escrowed");
        assert!(rows.keys().any(|r| r.starts_with(&format!("m/{n}/"))), "m/{n} sealed for the MLS epoch");
        assert_eq!(identity::account_epoch_secret(&w.h, &w.member, *n), Some(*k), "generation {n} kept as the account's epoch");
    }
    // (The escrow opens with the words.)
    let (n, k) = gens[0];
    let blob = identity::unhex(&rows[&format!("g/{n}")]).unwrap();
    assert_eq!(craftworks_account::escrow_open(&[0x77u8; 16], &blob), Some(k));
    // AGAIN: in step — nothing written.
    let again = run(&mut w);
    assert_eq!(again.iter().filter(|x| matches!(x, Io::Update { .. })).count(), 0, "nothing more: {again:?}");
    let _ = &w.root;
}

/// CONTROL: B NOT CURRENT (no credential) — its leaf in the channel, but it is not added.
#[test]
fn the_node_does_not_add_a_device_that_is_not_current() {
    let mut w = world(false);
    run(&mut w);
    let rows = channel_rows(&w);
    let ops: Vec<Vec<u8>> = rows.iter().filter(|(k, _)| k.starts_with("o/")).map(|(_, v)| identity::unhex(v).unwrap()).collect();
    let mut gb = Group::empty(&w.did, w.b.clone());
    let check = craftworks_groups::AccountCheck { current: vec![w.a.id.0.to_bytes(), w.b.id.0.to_bytes()], ever: vec![], writers: vec![w.a.id.0.to_bytes()] };
    gb.merge_all(&ops, &check);
    assert!(!gb.placed(), "B not added");
}

/// A RETIRED member (their page moved to a newer build, leaving this one an empty mandate): the round never starts for
/// them, and a build every member left reads nothing at a wake-up. CONTROL: before retiring, both do.
#[test]
fn a_retired_member_is_not_served_and_a_left_build_reads_nothing() {
    let mut c = world(true);
    assert!(!account_round::woke(&mut c.h).is_empty(), "control: the round starts for a member still here");
    assert_eq!(identity::upkeep_woke(&mut c.h), Some([0x1B; 32]), "control: the inbox read while a member is here");
    let mut w = world(true);
    identity::serve(&mut w.h, Request::UpkeepMandate { me: String::new(), spaces: Vec::new(), spent: Vec::new() }, [9; 32]);
    assert!(identity::upkeep_retired(&w.h, &w.member), "retired by the empty mandate");
    assert!(account_round::woke(&mut w.h).is_empty(), "no round for a retired member");
    assert_eq!(identity::upkeep_woke(&mut w.h), None, "no inbox read for a build every member left");
}
