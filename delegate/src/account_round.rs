//! K8.5 THE ACCOUNT's TREE KEPT BY THE NODE, every page of the account away (FOUNDATION "THE ACCOUNT's GROUP ON
//! BEEKEM"): what `members.account()` does on a page, here. Per member whose page is away, one round:
//!
//!   1. READ its key log, its devices contract and the account's CHANNEL (the data key's tail, under its table key —
//!      and its tree's blocks once flushed).
//!   2. THE TREE rebuilt from the channel's operations (`o/`) through the account's check (`AccountCheck`: the devices
//!      list — current, and every device ever credentialed), as THIS device (its member: the signer of the account's
//!      MLS state kept here). Nothing kept between rounds.
//!   3. ITS GENERATIONS kept as the account's epochs: the node reads the account's new rows.
//!   4. PLACED here: the reconcile (each current device with a signed leaf added, each placed one no longer current
//!      taken out — one key change), concurrent heads resolved by the lowest placed device only, this device's own leaf
//!      (`d/`), each generation's escrows (`g/` to the words, `m/` for the account's current MLS epoch).
//!   5. WRITTEN to the channel in ONE step, signed with the data key behind the fork guard (`upkeep_sign_channel`). A
//!      channel due a flush is left to a page (a delegate's new contracts stay on its node).

use std::collections::BTreeMap;

use craftworks_data as data;
use craftworks_groups::{Account, AccountCheck, Group};
use craftworks_identity::{self as identity, Host};
use sha2::Digest;

use crate::reads::{self, Round};
use crate::rekey_round::Held;
use crate::upkeep::{id_of, Io, Reply};

pub const ROUND: &[u8] = b"identity_account_tree/round";
const LAST: &[u8] = b"identity_account_tree/last";
/// What an MLS-sealed generation's key is derived with (the page's `members.account`: the same bytes).
const MLS_SEALED: &[u8] = b"craftworks 2026-10-26 account generation, mls-sealed";

/// The account's CHANNEL's name: `x<the account's id, first 12>-log` (`space.tableOf`; the account's id is its DID).
pub fn channel_name(did: &[u8; 32]) -> String {
    let d = craftworks_account::did(did);
    format!("x{}-log", d.get(..12).unwrap_or(&d))
}

/// A WAKE-UP: when no round is in flight, one for the next member in turn whose page is away and whose node holds the
/// account's data key and MLS state.
pub fn woke<H: Host>(h: &mut H) -> Vec<Io> {
    if reads::busy(h, ROUND) {
        return Vec::new();
    }
    let all: Vec<[u8; 32]> = identity::upkeep_members(h)
        .into_iter()
        .filter(|m| identity::upkeep_since_tick(h, m) >= crate::upkeep::PAGE_AWAY && identity::upkeep_account(h, m).is_some() && identity::upkeep_account_mls(h, m).is_some())
        .collect();
    let Some(member) = reads::next(h, LAST, &all) else {
        reads::save(h, ROUND, None);
        return Vec::new();
    };
    start(h, member)
}
/// A round for `member` begun.
pub fn start<H: Host>(h: &mut H, member: [u8; 32]) -> Vec<Io> {
    drive(h, Round { member, started: identity::upkeep_wakeups(h), started_ms: identity::upkeep_now_ms(h), ..Round::default() })
}

/// An ANSWER from the network: kept if this round asked for it, then the round driven on.
pub fn replied<H: Host>(h: &mut H, reply: &Reply) -> Vec<Io> {
    let Reply::Got { id, state } = reply else { return Vec::new() };
    match reads::take(h, ROUND, id, state.clone()) {
        Some(r) => drive(h, r),
        None => Vec::new(),
    }
}

fn drive<H: Host>(h: &mut H, r: Round) -> Vec<Io> {
    let (out, want) = match step(h, &r) {
        Ok(x) => x,
        Err(why) => {
            crate::upkeep::followed(h, &r.member, &craftworks_rules::steps::ACCOUNT, craftworks_rules::steps::ACCOUNT_TREE, &why);
            (Vec::new(), Vec::new())
        }
    };
    reads::go(h, ROUND, r, out, want)
}

/// The state read of `id` this round: `Some(None)` read, not there; `None` not read yet.
fn got(r: &Round, id: &[u8; 32]) -> Option<Option<Vec<u8>>> {
    r.got.get(id).map(|s| (!s.is_empty()).then(|| s.clone()))
}

/// One pass over what is read: (what to send now, what to read first).
fn step<H: Host>(h: &mut H, r: &Round) -> Result<(Vec<Io>, Vec<[u8; 32]>), String> {
    let member = r.member;
    let held = Held::of(h).ok_or("the contracts' code is not held here yet")?;
    let c = held.codes();
    let (did, data_seed) = identity::upkeep_account(h, &member).ok_or("the account's data key is not held here")?;
    let mls_state = identity::upkeep_account_mls(h, &member).ok_or("the account's group is not held here")?;
    let seed = craftworks_mls::signer_seed_of(&mls_state).ok_or("the account's group state does not read")?;
    let me = Account::of(&seed);
    let my_id = me.id.0.to_bytes();

    // 1. THE READS: the key log, the devices contract, the channel.
    let log_id = id_of(&c.idlog_hash, &did);
    let dev_id = id_of(&c.devices_hash, &craftworks_devices_contract::params(&did, &c.idlog_hash));
    let data_pub = ed25519_dalek::SigningKey::from_bytes(&data_seed).verifying_key().to_bytes();
    let name = channel_name(&did);
    let mut o = data::Open::new(c.tail, &data_pub, &name);
    o.set_table_key(identity::table_key(&data_seed, &name, 0));
    let ch_id = o.id_bytes();
    let (Some(log), Some(dev), Some(ch)) = (got(r, &log_id), got(r, &dev_id), got(r, &ch_id)) else {
        return Ok((Vec::new(), vec![log_id, dev_id, ch_id]));
    };
    let log = log.and_then(|st| craftworks_idlog_contract::read(&did, &st)).ok_or("the account's key log does not read")?;
    let creds = dev.and_then(|st| craftworks_devices_contract::decode(&st)).ok_or("the account's devices contract does not read")?;
    let Some(ch) = ch else { return Err("the account's channel is not there".into()) };
    o.absorb(&ch);
    // (Its tree's blocks, once flushed: each fetched once.)
    let rows = loop {
        match o.opened() {
            Ok(data::Step::Ready(rows)) => break rows,
            Ok(data::Step::Need(cids)) => {
                let mut want = Vec::new();
                let mut fed = false;
                for cid in cids {
                    let id = crate::table::block_id(&o, &cid, &c.sealed_hash, &c.block_hash).ok_or("a block of the channel has no place")?;
                    match got(r, &id) {
                        Some(Some(st)) => fed |= o.absorb_block(&cid, &st),
                        Some(None) => return Err("a block of the account's channel is not there".into()),
                        None => want.push(id),
                    }
                }
                if !want.is_empty() {
                    return Ok((Vec::new(), want));
                }
                if !fed {
                    return Err("the account's channel's tree does not open".into());
                }
            }
            Ok(data::Step::Keys(_)) => return Err("the account's channel asks for keys it is never sealed with".into()),
            Err(why) => return Err(format!("the account's channel does not open: {why}")),
        }
    };
    let text = |k: &[u8]| String::from_utf8(k.to_vec()).ok();

    // 2. THE TREE through the account's check.
    let current = craftworks_account::device_members(&did, &log, &creds);
    let ever = craftworks_account::device_members_ever(&did, &log, &creds);
    let mut ops = Vec::new();
    let mut leaves: BTreeMap<[u8; 32], [u8; 32]> = BTreeMap::new();
    let mut have: Vec<String> = Vec::new();
    for (k, v) in &rows {
        let (Some(k), Some(v)) = (text(k), text(v)) else { continue };
        if let Some(rest) = k.strip_prefix("o/") {
            if let Some(craftworks_rules::members::KeyopRow::Op(op)) = craftworks_rules::members::keyop_row(&v) {
                ops.push(op);
            }
            have.push(format!("o/{rest}"));
        } else if let Some(m) = k.strip_prefix("d/").and_then(identity::unhex32) {
            let l: serde_json::Value = serde_json::from_str(&v).unwrap_or_default();
            let (Some(leaf), Some(sig)) = (l["leaf"].as_str().and_then(identity::unhex32), l["sig"].as_str().and_then(identity::unhex).and_then(|s| <[u8; 64]>::try_from(s).ok())) else { continue };
            if craftworks_groups::leaf_checks(&m, &leaf, &sig) {
                leaves.insert(m, leaf);
            }
        } else {
            have.push(k);
        }
    }
    let writers: Vec<[u8; 32]> = ops.iter().filter_map(|op| craftworks_groups::op_info(op)).filter(|(_, kind, preds, _)| *kind == "add" && preds.is_empty()).map(|(.., by)| by).collect();
    let check = AccountCheck { current: current.clone(), ever, writers };
    let mut g = Group::empty(&did, me);
    g.merge_all(&ops, &check);

    // 3. ITS GENERATIONS, kept as the account's epochs.
    for (n, k) in g.generations() {
        identity::keep_epoch(h, &member, None, n, &k);
    }
    let mut out_rows: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    // (This device's own leaf, signed: what an add encrypts to.)
    if !leaves.contains_key(&my_id) {
        let (leaf, sig) = Account::of(&seed).signed_leaf();
        out_rows.push((format!("d/{}", identity::hex(&my_id)).into_bytes(), serde_json::json!({ "leaf": identity::hex(&leaf), "sig": identity::hex(&sig) }).to_string().into_bytes()));
    }
    // 4. PLACED: the reconcile, the heads, the escrows.
    if g.placed() {
        let rand = identity::upkeep_random(h).ok_or("no randomness armed")?;
        let hexs = |b: &[u8; 32]| identity::hex(b);
        let cards: Vec<craftworks_groups::Card> = leaves.iter().map(|(m, l)| (hexs(m), *m, *l)).collect();
        let listed: Vec<String> = current.iter().map(hexs).collect();
        let ins: BTreeMap<String, [u8; 32]> = current.iter().map(|m| (hexs(m), [0; 32])).collect();
        let outs: BTreeMap<String, [u8; 32]> = leaves.keys().filter(|m| !current.contains(m)).map(|m| (hexs(m), [0; 32])).collect();
        let rec = g.reconcile(&cards, &|d| listed.iter().any(|l| l == d), &ins, &outs, rand);
        let mut new_ops = rec.ops;
        // (Only the LOWEST placed device resolves concurrent heads — two at once fed each other key changes.)
        if g.members().into_iter().min() == Some(my_id) && g.current_now().is_none() {
            if let Ok(op) = g.rotate_seeded(blake3::derive_key("craftworks account tree heads", &rand)) {
                new_ops.push(op);
            }
        }
        for op in &new_ops {
            out_rows.push((format!("o/{}", craftworks_rules::members::keyop_key(op)).into_bytes(), identity::hex(op).into_bytes()));
        }
        // EACH GENERATION's ESCROWS: to the words (`g/`), and for the account's current MLS epoch (`m/`).
        let mls = mls_epoch(&did, &log, &mls_state);
        let mut eph = blake3::derive_key("craftworks account tree escrow", &rand);
        for (n, k) in g.generations() {
            if !have.contains(&format!("g/{n}")) {
                eph = blake3::derive_key("craftworks account tree escrow next", &eph);
                out_rows.push((format!("g/{n}").into_bytes(), identity::hex(&craftworks_account::escrow_seal(&log.head().enc, &k, eph)).into_bytes()));
            }
            if let Some((e, s)) = mls {
                if !have.contains(&format!("m/{n}/{e}")) {
                    let key: [u8; 32] = sha2::Sha256::digest([MLS_SEALED, &s[..]].concat()).into();
                    let nonce: [u8; 12] = eph[..12].try_into().expect("12");
                    eph = blake3::derive_key("craftworks account tree escrow next", &eph);
                    out_rows.push((format!("m/{n}/{e}").into_bytes(), identity::hex(&identity::bag_seal(&key, e as u32, &k, nonce)).into_bytes()));
                }
            }
        }
    }
    // (Kept again: the key change this round made is a generation too.)
    for (n, k) in g.generations() {
        identity::keep_epoch(h, &member, None, n, &k);
    }
    if out_rows.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    // 5. WRITTEN in one step (a channel due a flush: left to a page).
    if data::flush_due(o.pending_rows(), out_rows.len()) {
        return Err("the account's channel is due a flush: a page does it".into());
    }
    let step = o.prepare_rows(&out_rows).ok_or("the account's channel would not take the rows")?;
    let io = crate::write::signed_with(h, &member, &mut o, step, identity::upkeep_sign_channel)?;
    Ok((vec![io], Vec::new()))
}

/// The account's CURRENT MLS epoch and its secret, from the state kept here (its owners: every key its key log has
/// had). `None`: the group does not load.
fn mls_epoch(did: &[u8; 32], log: &craftworks_idlog_contract::Log, state: &[u8]) -> Option<(u64, [u8; 32])> {
    let ident = craftworks_mls::AccountIdentity { did: *did, owners: log.events.iter().map(|e| e.key).collect() };
    let a = craftworks_mls::Account::load(craftworks_mls::Rule::Account(ident), state).ok()?;
    Some((a.epoch(), a.epoch_secret().ok()?))
}
