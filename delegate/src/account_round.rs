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

pub const ROUND: &[u8] = b"identity_account_tree/round/";
/// THE GENERATIONS ON THIS NODE (FOUNDATION "WHO RECEIVES IS DECIDED BY THE ONE WHO SEALS", the page's
/// `keepGenerations` too): every one reached HELD for reading; the one WRITTEN with a clean tree's current one only
/// (`Group::writable` with our `check`; none: the writing mark stays).
pub(crate) fn keep_generations<H: Host>(h: &mut H, member: &[u8; 32], space: Option<[u8; 32]>, g: &mut Group, check: Option<&impl craftworks_groups::Check>) {
    for (n, k) in g.generations() {
        identity::hold_epoch(h, member, space, n, &k);
    }
    // (Judged with a check — given only for a tree its merge left WHOLE, nothing waiting: a tree stopped short of
    // operations is never marked from, 10-28 review — a clean tree's current generation; or, not clean, the mark
    // CLEARED: nothing written with a generation one put out holds.)
    if let Some(c) = check {
        let why = g.writable_why(c);
        if let (Some(sp), Err(e)) = (space, &why) {
            identity::upkeep_trace(h, &[format!("not marked clean [{}]: {e}", sp[..6].iter().map(|b| format!("{b:02x}")).collect::<String>())]);
        }
        let w = why.ok();
        identity::write_epoch(h, member, space, w.map(|(n, _)| n));
        // (A space's: WHICH ACCOUNTS the clean tree placed at that mark — the re-key's gate, `clean_at`.)
        if let (Some(sp), Some((n, _))) = (space, w) {
            let placed: Vec<String> = g.members().iter().filter_map(|m| c.account_of(m).or_else(|| g.whose(m).map(str::to_string))).collect();
            let moved = clean_at(h, member, &sp).map(|(was, _)| was) != Some(n);
            let _ = h.set_secret(&[CLEAN_AT, &member[..], &sp[..]].concat(), &bincode::serialize(&(n, placed)).expect("encodes"));
            // (A re-key held for this, or the generation MOVED — a removal's key change: the re-key begun now, never
            // the next ten-minute round.)
            crate::rekey_round::clean_now(h, moved);
        }
    }
}
/// A space's tree as last found CLEAN here: `(the generation marked, the accounts it placed)` (`keep_generations`).
const CLEAN_AT: &[u8] = b"identity_keys/clean_at/";
#[cfg(test)]
pub(crate) fn set_clean_at<H: Host>(h: &mut H, member: &[u8; 32], space: &[u8; 32], n: u64, placed: Vec<String>) {
    let _ = h.set_secret(&[CLEAN_AT, &member[..], &space[..]].concat(), &bincode::serialize(&(n, placed)).expect("encodes"));
}
pub(crate) fn clean_at<H: Host>(h: &H, member: &[u8; 32], space: &[u8; 32]) -> Option<(u64, Vec<String>)> {
    h.get_secret(&[CLEAN_AT, &member[..], &space[..]].concat()).and_then(|b| bincode::deserialize(&b).ok())
}

/// The label the new data seed is sealed under for the devices that stay (K8.7 — the page's `DATA_SEALED`, `members`).
const DATA_SEALED: &[u8] = b"craftworks 2026-10-27 account data seed, generation-sealed";
/// ONE ROUND PER MEMBER (a node serving several accounts serves each every wake-up — one in turn left a member waiting
/// as many minutes as the node had members: fx 10-27, a test node with a member per run never reached a new one).
fn round_of(member: &[u8; 32]) -> Vec<u8> {
    [ROUND, &member[..]].concat()
}
/// What an MLS-sealed generation's key is derived with (the page's `members.account`: the same bytes).
const MLS_SEALED: &[u8] = b"craftworks 2026-10-26 account generation, mls-sealed";

/// The account's CHANNEL's name: the identity's (`identity::CHANNEL` — the page's `space.tables.channel` for the
/// account; fx 10-27: a name computed as a space's, `x<id>-log`, named a contract nobody made).
pub fn channel_name(_did: &[u8; 32]) -> String {
    identity::CHANNEL.to_string()
}

/// A WAKE-UP: a round for EVERY member whose node holds the account's data key and this device's member, and whose own
/// round is not in flight.
pub fn woke<H: Host>(h: &mut H) -> Vec<Io> {
    let mut out = Vec::new();
    for member in identity::upkeep_members(h) {
        // (Why a member's round does not start, SAID each wake-up.) NOT KEPT OUT OF A PAGE's WAY (as a space's follower is
        // not): a page and the node reconciling at once merge, and only the lowest placed device resolves heads — waiting
        // for the page to be "away" counted completed wake-ups, which a busy node runs rarely (fx 10-27: 2 in 20 minutes,
        // a device placed after 9 minutes or not in 10).
        // A RETIRED member (moved to a newer build): this build never touches their account's tree again.
        if identity::upkeep_retired(h, &member) {
            continue;
        }
        let not: Option<String> = if identity::upkeep_account(h, &member).is_none() {
            Some("not started: the account's data key is not held here".into())
        } else if identity::upkeep_device_seed(h, &member).is_none() {
            Some("not started: this device has no member in the account's tree here".into())
        } else if reads::busy(h, &round_of(&member)) {
            Some("not started: its round is still waiting on an answer".into())
        } else {
            None
        };
        match not {
            Some(why) => identity::upkeep_account_tree_said(h, &member, &why),
            None => out.extend(start(h, member)),
        }
    }
    out
}
/// A round for `member` begun.
pub fn start<H: Host>(h: &mut H, member: [u8; 32]) -> Vec<Io> {
    drive(h, Round { member, started: identity::upkeep_wakeups(h), started_ms: identity::upkeep_now_ms(h), ..Round::default() })
}

/// An ANSWER from the network: to EVERY member's round that asked for it, each driven on.
pub fn replied<H: Host>(h: &mut H, reply: &Reply) -> Vec<Io> {
    let Reply::Got { id, state } = reply else { return Vec::new() };
    let mut out = Vec::new();
    for member in identity::upkeep_members(h) {
        if let Some(r) = reads::take(h, &round_of(&member), id, state.clone()) {
            out.extend(drive(h, r));
        }
    }
    out
}

fn drive<H: Host>(h: &mut H, r: Round) -> Vec<Io> {
    let (out, want) = match step(h, &r) {
        Ok((out, want, own)) => {
            // (What the step said of itself — a rotated data key adopted — else what it did.)
            let said = own.unwrap_or_else(|| if !want.is_empty() { format!("reading {} contract(s)", want.len()) } else if out.is_empty() { "in step: nothing to write".into() } else { "wrote to the channel".into() });
            identity::upkeep_account_tree_said(h, &r.member, &said);
            (out, want)
        }
        Err(why) => {
            identity::upkeep_account_tree_said(h, &r.member, &format!("stopped: {why}"));
            crate::upkeep::followed(h, &r.member, &craftworks_rules::steps::ACCOUNT, craftworks_rules::steps::ACCOUNT_TREE, &why);
            (Vec::new(), Vec::new())
        }
    };
    let name = round_of(&r.member);
    reads::go(h, &name, r, out, want)
}

/// A CHANNEL's rows from its state (its tree's blocks, once flushed, each fetched once): `Err(want)` — read these first.
fn open_rows(r: &Round, c: &crate::table::Codes, o: &mut data::Open, st: &[u8]) -> Result<Result<std::collections::BTreeMap<Vec<u8>, Vec<u8>>, Vec<[u8; 32]>>, String> {
    o.absorb(st);
    loop {
        match o.opened() {
            Ok(data::Step::Ready(rows)) => return Ok(Ok(rows.into_iter().collect())),
            Ok(data::Step::Need(cids)) => {
                let mut want = Vec::new();
                let mut fed = false;
                for cid in cids {
                    let id = crate::table::block_id(o, &cid, &c.sealed_hash, &c.block_hash).ok_or("a block of the channel has no place")?;
                    match got(r, &id) {
                        Some(Some(st)) => fed |= o.absorb_block(&cid, &st),
                        Some(None) => return Err("a block of the account's channel is not there".into()),
                        None => want.push(id),
                    }
                }
                if !want.is_empty() {
                    return Ok(Err(want));
                }
                if !fed {
                    return Err("the account's channel's tree does not open".into());
                }
            }
            Ok(data::Step::Keys(_)) => return Err("the account's channel asks for keys it is never sealed with".into()),
            Err(why) => return Err(format!("the account's channel does not open: {why}")),
        }
    }
}

/// The state read of `id` this round: `Some(None)` read, not there; `None` not read yet.
fn got(r: &Round, id: &[u8; 32]) -> Option<Option<Vec<u8>>> {
    r.got.get(id).map(|s| (!s.is_empty()).then(|| s.clone()))
}

/// One pass over what is read: (what to send now, what to read first).
fn step<H: Host>(h: &mut H, r: &Round) -> Result<(Vec<Io>, Vec<[u8; 32]>, Option<String>), String> {
    let member = r.member;
    let held = Held::of(h).ok_or("the contracts' code is not held here yet")?;
    let c = held.codes();
    let (did, data_seed) = identity::upkeep_account(h, &member).ok_or("the account's data key is not held here")?;
    // (The account's PRE-TREE history, if any: its last MLS epoch seals the `m/` copies.)
    let mls_state = identity::upkeep_account_mls(h, &member);
    let seed = identity::upkeep_device_seed(h, &member).ok_or("this device's member in the account's tree is not kept here")?;
    let me = Account::of(&seed);
    let my_id = me.id.0.to_bytes();

    // 1. THE READS: the key log, the devices contract, the channel.
    let log_id = id_of(&c.idlog_hash, &did);
    let dev_id = id_of(&c.devices_hash, &craftworks_devices_contract::params(&did, &c.idlog_hash));
    let data_pub = ed25519_dalek::SigningKey::from_bytes(&data_seed).verifying_key().to_bytes();
    let name = channel_name(&did);
    let mut o = data::Open::new(c.tail, &data_pub, &name);
    // (Its key the NAMES seed's — K8.7: it never moves; its address and authority are the data key's.)
    o.set_table_key(identity::table_key(&identity::upkeep_names(h, &member).unwrap_or(data_seed), &name, 0));
    let ch_id = o.id_bytes();
    let (Some(log), Some(dev), Some(ch)) = (got(r, &log_id), got(r, &dev_id), got(r, &ch_id)) else {
        return Ok((Vec::new(), vec![log_id, dev_id, ch_id], None));
    };
    let log = log.and_then(|st| craftworks_idlog_contract::read(&did, &st)).ok_or("the account's key log does not read")?;
    let creds = dev.and_then(|st| craftworks_devices_contract::decode(&st)).ok_or("the account's devices contract does not read")?;
    let Some(ch) = ch else { return Err("the account's channel is not there".into()) };
    let rows = match open_rows(r, &c, &mut o, &ch)? {
        Ok(rows) => rows,
        Err(want) => return Ok((Vec::new(), want, None)),
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
    let check = AccountCheck { current: current.clone(), ever, writers, leaves: leaves.iter().map(|(m, l)| (*m, *l)).collect() };
    let mut g = Group::empty(&did, me);
    let whole = g.merge_all(&ops, &check).waiting == 0;

    // 3. ITS GENERATIONS, kept as the account's epochs (held; the clean one written with).
    keep_generations(h, &member, None, &mut g, whole.then_some(&check));
    // 3b. THE DATA KEY ROTATED (K8.7: a removal elsewhere): this device STAYS — its new seed is in the channel the key
    // log's head names (only the new key writes it; its table key the names seed's), sealed with a generation of the
    // tree: that channel's operations merged too (a device behind by more than one rotation reaches the newest at
    // once), the seed the ROTATION's event names (`since`: a later change of words keeps the key) opened, checked
    // against the head, kept; the round ends here (the next wake-up reads the new channel).
    let head = log.head();
    if head.data != data_pub {
        use sha2::Digest;
        let since = log.events.iter().find(|e| e.data == head.data).map(|e| e.seq).unwrap_or(head.seq);
        let mut o2 = data::Open::new(c.tail, &head.data, &name);
        o2.set_table_key(identity::table_key(&identity::upkeep_names(h, &member).unwrap_or(data_seed), &name, 0));
        let id2 = o2.id_bytes();
        let Some(st2) = got(r, &id2) else { return Ok((Vec::new(), vec![id2], None)) };
        let said = match st2 {
            None => format!("the account's data key moved (key log {since}): its new channel is not there yet"),
            Some(st2) => {
                let rows2 = match open_rows(r, &c, &mut o2, &st2)? {
                    Ok(rows) => rows,
                    Err(want) => return Ok((Vec::new(), want, None)),
                };
                let ops2: Vec<Vec<u8>> = rows2
                    .iter()
                    .filter(|(k, _)| k.starts_with(b"o/"))
                    .filter_map(|(_, v)| match craftworks_rules::members::keyop_row(&text(v)?) {
                        Some(craftworks_rules::members::KeyopRow::Op(op)) => Some(op),
                        _ => None,
                    })
                    .collect();
                // (Judged with the NEW channel's signed leaves too — an add made there after the move names a leaf only it
                // holds; 10-28 re-review.)
                let mut lv2 = check.leaves.clone();
                for (k, v) in &rows2 {
                    let (Some(k), Some(v)) = (text(k), text(v)) else { continue };
                    let Some(m) = k.strip_prefix("d/").and_then(identity::unhex32) else { continue };
                    let l: serde_json::Value = serde_json::from_str(&v).unwrap_or_default();
                    let (Some(leaf), Some(sig)) = (l["leaf"].as_str().and_then(identity::unhex32), l["sig"].as_str().and_then(identity::unhex).and_then(|s| <[u8; 64]>::try_from(s).ok())) else { continue };
                    if craftworks_groups::leaf_checks(&m, &leaf, &sig) {
                        lv2.insert(m, leaf);
                    }
                }
                let check2 = AccountCheck { current: check.current.clone(), ever: check.ever.clone(), writers: check.writers.clone(), leaves: lv2 };
                g.merge_all(&ops2, &check2);
                let held: BTreeMap<String, [u8; 32]> = g.generations().into_iter().map(|(n, k)| (n.to_string(), k)).collect();
                let found = rows2.iter().find_map(|(k, v)| {
                    let k = text(k)?;
                    let mut p = k.split('/');
                    if p.next()? != "k" || p.next()?.parse::<u32>().ok()? != since {
                        return None;
                    }
                    let secret = held.get(p.next()?)?;
                    let key: [u8; 32] = sha2::Sha256::new().chain_update(DATA_SEALED).chain_update(secret).finalize().into();
                    let plain = identity::bag_open(&key, &identity::unhex(&text(v)?)?)?;
                    <[u8; 32]>::try_from(plain).ok().filter(|s| ed25519_dalek::SigningKey::from_bytes(s).verifying_key().to_bytes() == head.data)
                });
                match found {
                    Some(seed) if identity::upkeep_data_keep(h, &member, seed) => format!("the account's data key moved (key log {since}): its new seed kept — the channel follows it"),
                    Some(_) => "the account's data key moved: its new seed opened but not saved".to_string(),
                    None => format!("the account's data key moved (key log {since}): its new seed not opened here (this tree does not reach the removal's generation)"),
                }
            }
        };
        crate::upkeep::followed(h, &member, &craftworks_rules::steps::ACCOUNT, craftworks_rules::steps::ACCOUNT_TREE, &said);
        return Ok((Vec::new(), Vec::new(), Some(said)));
    }
    let mut out_rows: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    // (This device's own leaf, signed: what an add encrypts to.)
    if !leaves.contains_key(&my_id) {
        let (leaf, sig) = Account::of(&seed).signed_leaf();
        out_rows.push((format!("d/{}", identity::hex(&my_id)).into_bytes(), serde_json::json!({ "leaf": identity::hex(&leaf), "sig": identity::hex(&sig) }).to_string().into_bytes()));
    }
    // 4. PLACED: the reconcile, the heads, the escrows. (Not placed here: SAID — what the node did is never silent.)
    if !g.placed() {
        let why = format!("not placed in the tree it rebuilt ({} operation(s), {} member(s), {} current, {} leaf/leaves): nothing it can add", ops.len(), g.members().len(), current.len(), leaves.len());
        crate::upkeep::followed(h, &member, &craftworks_rules::steps::ACCOUNT, craftworks_rules::steps::ACCOUNT_TREE, &why);
    }
    let mut said = String::new();
    if g.placed() {
        let rand = identity::upkeep_random(h).ok_or("no randomness armed")?;
        let hexs = |b: &[u8; 32]| identity::hex(b);
        let cards: Vec<craftworks_groups::Card> = leaves.iter().map(|(m, l)| (hexs(m), *m, *l)).collect();
        let listed: Vec<String> = current.iter().map(hexs).collect();
        let ins: BTreeMap<String, [u8; 32]> = current.iter().map(|m| (hexs(m), [0; 32])).collect();
        let outs: BTreeMap<String, [u8; 32]> = leaves.keys().filter(|m| !current.contains(m)).map(|m| (hexs(m), [0; 32])).collect();
        let rec = g.reconcile(&cards, &|d| listed.iter().any(|l| l == d), &ins, &outs, rand);
        said = format!("{} added, {} removed{}", rec.added.len(), rec.removed.len(), if rec.not.is_empty() { format!(" — not: {}", rec.not.join("; ")) } else { String::new() });
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
        // EACH GENERATION's ESCROWS: to the words (`g/`), and for the account's current MLS epoch (`m/`) — only while
        // the MLS bridge is open (`account::mls_bridge_open`: no device ever credentialed out; one removed holds it).
        let mls = mls_state.as_deref().filter(|_| craftworks_account::mls_bridge_open(&did, &log, &creds)).and_then(|st| mls_epoch(&did, &log, st));
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
    keep_generations(h, &member, None, &mut g, whole.then_some(&check));
    if out_rows.is_empty() {
        return Ok((Vec::new(), Vec::new(), None));
    }
    // 5. WRITTEN in one step (a channel due a flush: left to a page).
    if data::flush_due(o.pending_rows(), out_rows.len()) {
        return Err("the account's channel is due a flush: a page does it".into());
    }
    let step = o.prepare_rows(&out_rows).ok_or("the account's channel would not take the rows")?;
    let io = crate::write::signed_with(h, &member, &mut o, step, identity::upkeep_sign_channel)?;
    crate::upkeep::followed(h, &member, &craftworks_rules::steps::ACCOUNT, craftworks_rules::steps::ACCOUNT_TREE, &format!("{} row(s) written to the channel ({said})", out_rows.len()));
    Ok((vec![io], Vec::new(), None))
}

/// The account's CURRENT MLS epoch and its secret, from the state kept here (its owners: every key its key log has
/// had). `None`: the group does not load.
fn mls_epoch(did: &[u8; 32], log: &craftworks_idlog_contract::Log, state: &[u8]) -> Option<(u64, [u8; 32])> {
    let ident = craftworks_mls::AccountIdentity { did: *did, owners: log.events.iter().map(|e| e.key).collect() };
    let a = craftworks_mls::Account::load(craftworks_mls::Rule::Account(ident), state).ok()?;
    Some((a.epoch(), a.epoch_secret().ok()?))
}
