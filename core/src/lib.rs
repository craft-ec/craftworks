//! The Craftworks app's one wasm package: the node's message framing (`wire`), the identity delegate's questions and
//! answers, and the account: its recovery words and the DID they name. It holds no key after a call returns and makes no decision: it
//! frames what the page asks and names what the node answers, as JSON the components can show.

/// The account: its own crate (shared with mls), under its old name here.
pub use craftworks_account as account;
/// A table as a tail: its own crate (the identity delegate drives it too).
pub use craftworks_data as data;

use craftworks_identity::{decode_answer, encode_request, hex, Answer, Request};
pub use craftworks_identity::unhex;
use serde_json::{json, Value};
use wire::{AckKind, Incoming, Reassembler};


/// An identity answer, for the page.
pub fn answer_json(a: &Answer) -> Value {
    match a {
        Answer::Unlocked { public, did, data } => {
            json!({ "unlocked": { "member": hex(public), "did": hex(did), "data": data.map(|d| hex(&d)) } })
        }
        Answer::WrongPin { tries_left } => json!({ "wrongPin": { "triesLeft": tries_left } }),
        Answer::Locked => json!({ "locked": true }),
        Answer::LoggedOut => json!({ "loggedOut": true }),
        Answer::Signed { sig } => json!({ "signed": hex(sig) }),
        Answer::Handed { seed, did, data } => {
            json!({ "handed": { "seed": hex(seed), "did": hex(did), "data": data.map(|d| hex(&d)) } })
        }
        Answer::Granted { tables } => json!({ "granted": tables }),
        Answer::Grants { list } => {
            json!({ "grants": list.iter().map(|(app, t)| json!({ "app": wire::contract_id(*app).encode(), "table": t })).collect::<Vec<_>>() })
        }
        Answer::Revoked => json!({ "revoked": true }),
        Answer::Refused(why) => json!({ "refused": format!("{why:?}") }),
        Answer::TableKey { key } => json!({ "tableKey": hex(key) }),
        Answer::MlsSaved => json!({ "mlsSaved": true }),
        Answer::MlsState { state } => json!({ "mlsState": state.as_ref().map(|s| hex(s)) }),
        Answer::TableKeyAt { epoch, key } => json!({ "tableKey": hex(key), "epoch": epoch }),
        Answer::InboxKey { public } => json!({ "inboxKey": hex(public) }),
        Answer::SpaceMember {
            seed,
            public,
            credential,
        } => json!({ "spaceMember": {
            "seed": hex(seed), "public": hex(public), "credential": hex(credential),
        } }),
        Answer::Upkeep {
            wakeups,
            inbox,
            inbox_len,
            now,
            codes,
            groups,
            stale,
            decided,
            round_at,
            next,
        } => json!({ "upkeep": {
            "wakeups": wakeups, "inbox": inbox.map(|i| hex(&i)), "inboxLen": inbox_len, "now": now, "codes": codes.map(|c| hex(&c)),
            "groups": groups.iter().map(|(s, e, st)| json!({ "space": hex(s), "epoch": e, "state": hex(st) })).collect::<Vec<_>>(),
            "stale": stale.iter().map(|s| hex(s)).collect::<Vec<_>>(),
            "roundAt": round_at,
            "next": next,
            "decided": decided.iter().map(|d| json!({ "space": hex(&d.space), "did": d.did, "q": hex(&d.q), "at": d.at, "step": d.step, "why": d.why, "seq": d.seq })).collect::<Vec<_>>(),
        } }),
        Answer::HandedSpaces { spaces } => {
            json!({ "handedSpaces": spaces.iter().map(|(id, st, eps)| json!({
            "space": hex(id), "mls": st.as_ref().map(|s| hex(s)),
            "epochs": eps.iter().map(|(e, s)| json!([e, hex(s)])).collect::<Vec<_>>(),
        })).collect::<Vec<_>>() })
        }
        Answer::Opened { items } => {
            json!({ "opened": items.iter().map(|i| i.as_ref().map(|b| hex(b))).collect::<Vec<_>>() })
        }
        Answer::EpochSecrets { epochs } => json!({ "epochs": epochs.iter().map(|(e, s)| json!([e, hex(s)])).collect::<Vec<_>>() }),
        Answer::WitnessMandated => json!({ "witnessMandated": true }),
        Answer::KeeperMandated => json!({ "keeperMandated": true }),
        Answer::HandedWitnessed { places } => json!({ "handedWitnessed": places.iter().map(|(c, q, e)| json!([hex(c), q, hex(e)])).collect::<Vec<_>>() }),
        Answer::WitnessedKept { kept } => json!({ "witnessedKept": kept }),
        Answer::HandedKeys { mls, epochs } => json!({ "handedKeys": {
            "mls": mls.as_ref().map(|s| hex(s)),
            "epochs": epochs.iter().map(|(e, s)| json!([e, hex(s)])).collect::<Vec<_>>(),
        } }),
    }
}

/// What one frame from the node says, for the page. `Partial` frames say nothing yet.
pub fn describe(incoming: Incoming) -> Value {
    match incoming {
        Incoming::Partial => json!({ "kind": "partial" }),
        Incoming::Ack(AckKind::Registered(k)) => json!({ "kind": "registered", "delegate": k }),
        Incoming::Ack(AckKind::Put(k)) => json!({ "kind": "put", "key": k.to_string() }),
        Incoming::Ack(a) => json!({ "kind": "ack", "what": format!("{a:?}") }),
        Incoming::DelegateMissing { key } => json!({ "kind": "delegate-missing", "delegate": key }),
        Incoming::DelegateThrottled { said } => json!({ "kind": "throttled", "said": said }),
        Incoming::EngineBytes(msgs) => {
            let answers: Vec<Value> = msgs
                .iter()
                .map(|m| match decode_answer(m) {
                    Some((id, a)) => json!({ "id": id, "answer": answer_json(&a) }),
                    // An answer in ANOTHER build's layout (an earlier build asked to retire: its fields differ): its id
                    // still read — answered, though not readable here.
                    None => {
                        let id = m.strip_prefix(craftworks_identity::MAGIC).and_then(|b| b.get(..4)).map(|b| u32::from_le_bytes(b.try_into().expect("4")));
                        json!({ "id": id, "answer": "unreadable", "bytes": m.len() })
                    }
                })
                .collect();
            json!({ "kind": "identity", "answers": answers })
        }
        Incoming::Refused(r) => json!({ "kind": "refused", "said": format!("{r:?}") }),
        Incoming::Unusable(u) => json!({ "kind": "unusable", "why": format!("{u:?}") }),
        // The node about itself (storage proofs, TOKEN §7a): its peer id, its ring location (millionths) and every
        // contract it hosts, `[id hex, size]`.
        Incoming::NodeSelf { peer_id, location, hosted } => json!({
            "kind": "node-self",
            "peer": peer_id,
            "location": location,
            "hosted": hosted.iter().map(|(id, n)| json!([hex(id), n])).collect::<Vec<_>>(),
        }),
        Incoming::Peers(p) => json!({ "kind": "peers", "peers": p.iter().map(|(id, a)| json!([id, a])).collect::<Vec<_>>() }),
        other => {
            json!({ "kind": "other", "what": format!("{other:?}").chars().take(200).collect::<String>() })
        }
    }
}

/// One page's connection state: the reassembler for chunked answers and the identity delegate it asks.
pub struct Core {
    r: Reassembler,
    identity: Vec<u8>,
    next_id: u32,
    next_stream: u32,
    /// Contract states the node sent (a GET's answer), by contract id: read by the account readers, never handed to
    /// the page as bytes.
    got: std::collections::HashMap<[u8; 32], Vec<u8>>,
    /// Open tails (this member's, to write; the account's other members', to read), by contract id.
    tails: std::collections::HashMap<[u8; 32], data::Open>,
    /// The Block contract's code: a tree from before sealing whole lives in Block contracts.
    block_code: Vec<u8>,
    /// The Sealed contract's code: a table's tree blocks, sealed whole, each at its address.
    sealed_code: Vec<u8>,
    /// The `piece` contract's code: a file's pieces since burning (a burn hash in each).
    piece_code: Vec<u8>,
    /// The last ASSET listed per tail (`tail_asset`): its groups, for coding a parity block again.
    assets: std::collections::HashMap<[u8; 32], Vec<data::AssetGroup>>,
    /// Tree blocks asked of the network: Block contract id -> EVERY tail that needs it (with the block's id) — one
    /// answer serves them all.
    wanted: std::collections::HashMap<[u8; 32], Vec<([u8; 32], freenet_prolly::Cid)>>,
    /// READ ONCE: every tree block the node sent, by its contract — content-addressed, so any table (any version of
    /// it) that needs it again takes it from here, never asking the network twice.
    held: std::collections::HashMap<[u8; 32], Vec<u8>>,
    /// Repairs under way: the lost block's contract id -> (its tail, its group).
    repairs: std::collections::HashMap<[u8; 32], ([u8; 32], engine::repair::Group)>,
    /// A block REBUILT, by the contract it was lost at → its cid: what a read puts back (`tail_keep_put`) is named by
    /// that contract (owner 10-07: named so, the put found nothing and was never made — rebuilt blocks never went back).
    rebuilt: std::collections::HashMap<[u8; 32], freenet_prolly::Cid>,
}

impl Core {
    pub fn new(identity_wasm: &[u8]) -> Core {
        Core {
            r: Reassembler::new(),
            identity: identity_wasm.to_vec(),
            next_id: 1,
            next_stream: 1,
            got: Default::default(),
            tails: Default::default(),
            block_code: Vec::new(),
            sealed_code: Vec::new(),
            piece_code: Vec::new(),
            wanted: Default::default(),
            held: Default::default(),
            repairs: Default::default(),
            rebuilt: Default::default(),
            assets: Default::default(),
        }
    }

    pub fn identity_key(&self) -> String {
        wire::delegate_from_code(&self.identity).1.to_string()
    }

    fn stream(&mut self) -> u32 {
        self.next_stream += 1;
        self.next_stream
    }

    pub fn frames_register_identity(&mut self) -> Result<Vec<Vec<u8>>, String> {
        let (d, _) = wire::delegate_from_code(&self.identity);
        let s = self.stream();
        wire::frame_register_delegate(d, s)
    }

    /// Ask the identity delegate. Returns the id its answer will carry, and the frames.
    pub fn frames_identity(&mut self, req: &Request) -> Result<(u32, Vec<Vec<u8>>), String> {
        let (_, key) = wire::delegate_from_code(&self.identity);
        self.frames_identity_at(key, req)
    }

    /// Ask an EARLIER build of the identity delegate, named `<key>:<code hash>` (base58, as the manifest's
    /// `identity_prior` lists them): only to move a member to this build — `Handover`, then `HandoverKeys`.
    pub fn frames_prior(
        &mut self,
        prior: &str,
        req: &Request,
    ) -> Result<(u32, Vec<Vec<u8>>), String> {
        // Handing a member over, or RETIRING its upkeep there (its PIN, then an empty mandate: a build the member
        // left still wakes every minute, and would act on the mandate it last held).
        if !matches!(
            req,
            Request::Handover { .. }
                | Request::HandoverKeys { .. }
                | Request::HandoverSpaces { .. }
                | Request::HandoverWitnessed { .. }
                | Request::Unlock { .. }
                // A space's EPOCH SECRETS (read only; answered to the session the PIN opened there): what a handover
                // from that build left behind (10-18, v401 → v403: its handover walked only the numbers just below
                // the latest — the MLS epochs stayed there).
                | Request::EpochSecrets { .. }
        ) && !matches!(req, Request::UpkeepMandate { spaces, .. } if spaces.is_empty())
        {
            return Err("an earlier build is asked only to hand a member over, to give a space's earlier keys, or to retire its upkeep".into());
        }
        let b32 = |s: &str| -> Result<[u8; 32], String> {
            bs58::decode(s)
                .into_vec()
                .ok()
                .and_then(|v| v.try_into().ok())
                .ok_or_else(|| format!("not a 32-byte base58 id: {s}"))
        };
        let (k, c) = prior
            .split_once(':')
            .ok_or("an earlier build is <key>:<code hash>")?;
        let key = freenet_stdlib::prelude::DelegateKey::new(
            b32(k)?,
            freenet_stdlib::prelude::CodeHash::new(b32(c)?),
        );
        self.frames_identity_at(key, req)
    }

    pub fn frames_handover(
        &mut self,
        prior: &str,
        pin: String,
    ) -> Result<(u32, Vec<Vec<u8>>), String> {
        self.frames_prior(prior, &Request::Handover { pin })
    }

    fn frames_identity_at(
        &mut self,
        key: freenet_stdlib::prelude::DelegateKey,
        req: &Request,
    ) -> Result<(u32, Vec<Vec<u8>>), String> {
        let id = self.next_id;
        self.next_id += 1;
        let s = self.stream();
        let params = freenet_stdlib::prelude::Parameters::from(Vec::new());
        Ok((
            id,
            wire::frame_delegate_op(&key, &params, encode_request(id, req), s)?,
        ))
    }

    /// Frames of a PUT (`wire::frame_put`).
    pub fn frames_put(&mut self, p: &account::Put) -> Result<Vec<Vec<u8>>, String> {
        let s = self.stream();
        wire::frame_put(p.contract.clone(), p.state.clone(), s)
    }

    /// Frames asking the node about ITSELF (`wire::frame_node_self`): answered `{ kind: "node-self" }`.
    pub fn frames_node_self(&mut self) -> Result<Vec<Vec<u8>>, String> {
        let s = self.stream();
        wire::frame_node_self(s)
    }

    /// Frames asking the node for its connected peers (`wire::frame_connected_peers`): answered `{ kind: "peers" }`.
    pub fn frames_peers(&mut self) -> Result<Vec<Vec<u8>>, String> {
        let s = self.stream();
        wire::frame_connected_peers(s)
    }

    /// Frames of a GET of contract `id` (`wire::frame_get`, no subscription).
    pub fn frames_get(&mut self, id: [u8; 32]) -> Result<Vec<Vec<u8>>, String> {
        let s = self.stream();
        wire::frame_get(wire::contract_id(id), false, s)
    }

    /// Frames of a GET that also SUBSCRIBES this node to `id`: it asks the contract's peers, and keeps getting its
    /// updates (a node that holds a copy answers a plain GET from it).
    pub fn frames_follow(&mut self, id: [u8; 32]) -> Result<Vec<Vec<u8>>, String> {
        let s = self.stream();
        wire::frame_get(wire::contract_id(id), true, s)
    }

    /// The state the node last sent for `id`.
    pub fn got(&self, id: &[u8; 32]) -> Option<&[u8]> {
        self.got.get(id).map(Vec::as_slice)
    }

    /// Mark `id` READ AND NOT THERE (the node found no such contract): the edition steps tell it from not read yet.
    pub fn not_there(&mut self, id: [u8; 32]) {
        self.got.insert(id, Vec::new());
    }

    /// The state the node sent for `id`, TAKEN (a file's fragments: read once, never kept here).
    pub fn take_got(&mut self, id: &[u8; 32]) -> Option<Vec<u8>> {
        self.got.remove(id)
    }

    /// A FILE PIECE to put, as `(id hex, frames)`: the `piece` contract at its address, its state
    /// `LIVE ‖ burn hash ‖ piece`.
    pub fn frames_piece(
        &mut self,
        p: &craftworks_files::Piece,
        burn: &[u8; 32],
    ) -> Result<(String, Vec<Vec<u8>>), String> {
        let s = self.stream();
        let state = [&[2u8][..], burn, &p.state].concat();
        let c = wire::block::block_contract(&self.piece_code, &p.address);
        let name = c.key().id().encode();
        Ok((
            name,
            wire::frame_put(c, freenet_stdlib::prelude::WrappedState::new(state), s)?,
        ))
    }

    /// BURN the piece at `address`: its `piece` contract given `BURNED ‖ sha-256(secret) ‖ secret` (it takes it only
    /// if that is the burn hash its first write named). `(id hex, frames)`.
    pub fn frames_burn(
        &mut self,
        address: &[u8; 32],
        secret: &[u8; 32],
    ) -> Result<(String, Vec<Vec<u8>>), String> {
        use sha2::Digest;
        let s = self.stream();
        let state = [&[3u8][..], sha2::Sha256::digest(secret).as_slice(), secret].concat();
        let c = wire::block::block_contract(&self.piece_code, address);
        let name = c.key().id().encode();
        Ok((
            name,
            wire::frame_put(c, freenet_stdlib::prelude::WrappedState::new(state), s)?,
        ))
    }

    /// KEEP a file piece: the state it is stored as (a GET's answer, whole) put again at `address`, its `piece`
    /// contract. `(id hex, frames)`.
    pub fn frames_keep_piece(
        &mut self,
        address: &[u8; 32],
        state: &[u8],
    ) -> Result<(String, Vec<Vec<u8>>), String> {
        let s = self.stream();
        let c = wire::block::block_contract(&self.piece_code, address);
        let name = c.key().id().encode();
        Ok((
            name,
            wire::frame_put(
                c,
                freenet_stdlib::prelude::WrappedState::new(state.to_vec()),
                s,
            )?,
        ))
    }

    /// The contract id (bytes) where a file piece at `address` is fetched: its `piece` contract's.
    pub fn piece_id(&self, address: &[u8; 32]) -> [u8; 32] {
        wire::block::contract_for(&self.piece_code, address)
    }

    pub fn set_piece_code(&mut self, code: &[u8]) {
        self.piece_code = code.to_vec();
    }

    /// Open the account's table `table` under the data key `key` (idempotent). Returns its contract id.
    pub fn tail_open(&mut self, tail_code: &[u8], key: &[u8; 32], table: &str) -> [u8; 32] {
        let o = data::Open::new(tail_code, key, table);
        let id = o.id_bytes();
        self.tails.entry(id).or_insert(o);
        id
    }

    pub fn tail(&mut self, id: &[u8; 32]) -> Result<&mut data::Open, String> {
        self.tails
            .get_mut(id)
            .ok_or_else(|| "that tail is not open".into())
    }

    /// Frames that send a committed write: the first as a PUT of the whole state, then UPDATEs of one delta each.
    pub fn frames_send(&mut self, id: &[u8; 32], send: data::Send) -> Result<Vec<Vec<u8>>, String> {
        let s = self.stream();
        let o = self.tail(id)?;
        match send {
            data::Send::Put(state) => {
                let (_, c, state) = wire::puts::contract(&o.code, &o.params, &state);
                wire::frame_put(c, state, s)
            }
            data::Send::Update(delta) => wire::frame_update_delta(
                wire::puts::contract(&o.code, &o.params, &[]).1.key(),
                delta,
                s,
            ),
        }
    }

    /// Frames of a GET of an open tail, subscribing (so other devices' writes arrive as they land).
    pub fn frames_tail_get(&mut self, id: &[u8; 32]) -> Result<Vec<Vec<u8>>, String> {
        let s = self.stream();
        wire::frame_get(wire::contract_id(*id), true, s)
    }

    fn tail_state(&mut self, id: [u8; 32], state: &[u8]) -> Option<Value> {
        self.tails.get_mut(&id)?.absorb(state);
        Some(self.tail_view(&id))
    }

    pub fn set_block_code(&mut self, code: &[u8]) {
        self.block_code = code.to_vec();
    }

    pub fn set_sealed_code(&mut self, code: &[u8]) {
        self.sealed_code = code.to_vec();
    }

    /// An open table as the page sees it: `{ kind: "tail", tail: { rows, … } }` once every tree block it needs is
    /// held; else `{ kind: "tail-need", blocks: [Block contract ids] }`, each to GET (its answer comes back through
    /// `take`, which feeds it in and gives the next view).
    /// A PAGE of an open tail (phase 3, Reads: `data::Open::page`): `{ kind: "tail-page", rows: [[key hex, value hex]],
    /// next: hex | null }` once the blocks on its path are held; else `tail-need` / `tail-keys` as for a view.
    pub fn tail_page(
        &mut self,
        id: &[u8; 32],
        lo: Option<Vec<u8>>,
        hi: Option<Vec<u8>>,
        reverse: bool,
        after: Option<Vec<u8>>,
        limit: usize,
    ) -> Value {
        match self.read(id, |o| {
            o.page(lo.clone(), hi.clone(), reverse, after.clone(), limit)
        }) {
            Ok(Err(blocks)) => json!({ "kind": "tail-need", "id": hex(id), "blocks": blocks }),
            Ok(Ok(data::Step::Need(_))) => unreachable!(),
            Ok(Ok(data::Step::Ready(p))) => json!({
                "kind": "tail-page", "id": hex(id),
                "rows": p.rows.iter().map(|(k, v)| [hex(k), hex(v)]).collect::<Vec<_>>(),
                "next": p.next.as_deref().map(hex),
            }),
            Ok(Ok(data::Step::Keys(epochs))) => {
                json!({ "kind": "tail-keys", "id": hex(id), "epochs": epochs })
            }
            Err(e) => json!({ "kind": "tail-unreadable", "id": hex(id), "said": e }),
        }
    }

    pub fn tail_view(&mut self, id: &[u8; 32]) -> Value {
        if !self.tails.contains_key(id) {
            return json!({ "kind": "error", "said": "that tail is not open" });
        }
        match self.read(id, |o| o.rows()) {
            Ok(Ok(data::Step::Ready(rows))) => {
                json!({ "kind": "tail", "id": hex(id), "tail": rows })
            }
            Ok(Err(blocks)) => json!({ "kind": "tail-need", "id": hex(id), "blocks": blocks }),
            Ok(Ok(data::Step::Need(_))) => unreachable!(),
            Ok(Ok(data::Step::Keys(epochs))) => {
                json!({ "kind": "tail-keys", "id": hex(id), "epochs": epochs })
            }
            Err(e) => json!({ "kind": "tail-unreadable", "id": hex(id), "said": e }),
        }
    }

    /// A table's ASSET (phase 4, Lifecycle): its groups — each `k` members then parity — with every block's id and
    /// the contract it lives in (hex); or the blocks to GET first (as a read), or the epochs whose keys to get.
    /// `history`: instead, the blocks only its LOGGED roots reach (its history, `data::Open::history_asset`) — asked
    /// after the current asset, and added to what `tail_keep_put` makes from.
    pub fn tail_asset(&mut self, id: &[u8; 32], history: bool) -> Result<AssetOut, String> {
        match self.read(id, |o| if history { o.history_asset() } else { o.asset() })? {
            Err(fetch) => Ok(AssetOut::Need(fetch)),
            Ok(data::Step::Need(_)) => unreachable!(),
            Ok(data::Step::Keys(e)) => Ok(AssetOut::Keys(e)),
            Ok(data::Step::Ready(groups)) => {
                let o = self.tails.get(id).ok_or("that tail is not open")?;
                let mut out = Vec::new();
                for g in &groups {
                    let mut slots = Vec::new();
                    for c in &g.slots {
                        let (sealed, params) = o
                            .block_params(c)
                            .ok_or("this table's key is not held here")?;
                        slots.push((
                            hex(c),
                            hex(&wire::block::contract_for(
                                if sealed {
                                    &self.sealed_code
                                } else {
                                    &self.block_code
                                },
                                &params,
                            )),
                        ));
                    }
                    out.push((g.k, slots));
                }
                if history {
                    self.assets.entry(*id).or_default().extend(groups);
                } else {
                    self.assets.insert(*id, groups);
                }
                Ok(AssetOut::Ready(out))
            }
        }
    }

    /// Frames that PUT a block of a table's asset AGAIN (`data::Open::stored`: re-publishing it, or repairing it where
    /// it is missing): `(contract id base58, frames)`; `None`: it cannot be made here.
    pub fn tail_keep_put(
        &mut self,
        id: &[u8; 32],
        cid: &[u8; 32],
    ) -> Result<Option<(String, Vec<Vec<u8>>)>, String> {
        let groups = self.assets.get(id).cloned().unwrap_or_default();
        // A rebuilt block is named by the contract it was lost at: its cid from the rebuild.
        let cid = &self.rebuilt.get(cid).copied().unwrap_or(*cid);
        let o = self.tails.get(id).ok_or("that tail is not open")?;
        let Some((sealed, state)) = o.stored(cid, &groups) else {
            return Ok(None);
        };
        let c = if sealed {
            wire::block::block_contract(
                &self.sealed_code,
                &data::block_address(
                    &o.address_key().ok_or("this table's key is not held here")?,
                    cid,
                ),
            )
        } else {
            wire::block::block_contract(&self.block_code, cid)
        };
        self.block_put(c, state).map(Some)
    }

    /// A BLOCK's put — `(its contract's name, frames)` — the one way a block is framed (a table's, a statement's).
    fn block_put(&mut self, c: freenet_stdlib::prelude::ContractContainer, state: Vec<u8>) -> Result<(String, Vec<Vec<u8>>), String> {
        let name = c.key().id().encode();
        let s = self.stream();
        Ok((name, wire::frame_put(c, freenet_stdlib::prelude::WrappedState::new(state), s)?))
    }

    /// The LINES of the public statement under `root`, its blocks read from their Block contracts: `Err(ids)` — read
    /// these contracts first.
    pub fn statement_lines(&self, root: &freenet_prolly::Cid) -> Result<std::collections::BTreeMap<[u8; 32], craftworks_reputation::statement::Line>, Vec<[u8; 32]>> {
        let id_of = wire::block::contract_deriver(&self.block_code);
        // (A Block contract's state is `kind ‖ body`; the tree's block is its body.)
        let block = |c: &freenet_prolly::Cid| self.got(&id_of(c)).filter(|s| s.len() > 1).map(|s| s[1..].to_vec());
        craftworks_reputation::statement::lines(root, &block).map_err(|cids| cids.iter().map(|c| id_of(c)).collect())
    }

    /// A PUBLIC tree block's put (a Block contract by its cid, in the clear): a statement's.
    pub fn public_block_put(&mut self, cid: &freenet_prolly::Cid, body: &[u8]) -> Result<(String, Vec<Vec<u8>>), String> {
        let state = wire::block::block_state(cid, body).ok_or("a block of no kind")?;
        let c = wire::block::block_contract(&self.block_code, cid);
        self.block_put(c, state)
    }

    /// Frames that PUT the tail's signed state AGAIN (re-publishing it: the network keeps the newest).
    pub fn tail_keep_state(&mut self, id: &[u8; 32]) -> Result<Vec<Vec<u8>>, String> {
        let state = self.tail(id)?.state();
        self.frames_send(id, data::Send::Put(state))
    }

    /// Record tree blocks `cids` as wanted by tail `id`; the contract ids (hex) they live in, to GET: a sealed
    /// tree's Sealed contracts at their addresses, or a tree from before's Block contracts.
    /// A block this table cannot name (a tree sealed with a key not held here — a table opened as public whose tree
    /// was written sealed), or nothing to ask at all, is an ERROR: a read that asked for nothing would ask again forever.
    fn want(&mut self, id: &[u8; 32], cids: &[freenet_prolly::Cid]) -> Result<Vec<String>, String> {
        let o = self.tails.get(id).ok_or("that tail is not open")?;
        let at: Vec<_> = cids
            .iter()
            .filter_map(|c| Some((*c, o.block_params(c)?)))
            .collect();
        if at.is_empty() || at.len() < cids.len() {
            // WHICH key (the report carries it, so a support read can tell a missing epoch from another branch's key
            // or a table key not held): the tree's own key reference, as its tail names it.
            let named = o
                .writer
                .current()
                .and_then(|t| t.body.entries.get(data::SEALED_TREE).and_then(|e| e.value.clone()))
                .and_then(|v| data::KeyRef::of(&v).map(|x| x.0));
            return Err(match named {
                Some(data::KeyRef::Epoch(e)) => format!("its tree is sealed with the key of epoch {e}: not held here, or held from another branch"),
                Some(data::KeyRef::Table) => "its tree is sealed with the table's own key: not held here".into(),
                None => "its tree is sealed under no key this page reads".into(),
            });
        }
        let mut fetch = Vec::new();
        for (cid, (sealed, params)) in at {
            let c = wire::block::contract_for(
                if sealed {
                    &self.sealed_code
                } else {
                    &self.block_code
                },
                &params,
            );
            // Held already (another table's read, an older version's): taken from here, not asked again.
            if let Some(state) = self.held.get(&c) {
                if let Some(o) = self.tails.get_mut(id) {
                    if o.absorb_block(&cid, state) {
                        continue;
                    }
                }
            }
            let w = self.wanted.entry(c).or_default();
            if !w.contains(&(*id, cid)) {
                w.push((*id, cid));
            }
            fetch.push(hex(&c));
        }
        Ok(fetch)
    }

    /// A read step resolved against what is HELD: `Need` blocks already held are taken in and the read runs again —
    /// only blocks no one holds come back to fetch.
    fn read<T>(
        &mut self,
        id: &[u8; 32],
        mut step: impl FnMut(&mut data::Open) -> Result<data::Step<T>, String>,
    ) -> Result<Result<data::Step<T>, Vec<String>>, String> {
        loop {
            let o = self.tails.get_mut(id).ok_or("that tail is not open")?;
            match step(o)? {
                data::Step::Need(cids) => {
                    let fetch = self.want(id, &cids)?;
                    if !fetch.is_empty() {
                        return Ok(Err(fetch));
                    }
                }
                other => return Ok(Ok(other)),
            }
        }
    }

    /// A tree block's GROUP, found in what the table holds: its other blocks to GET at the same time as the block
    /// itself (Block contract ids, hex) — the RACE (rule 11): whichever comes first, the block or any k of its group,
    /// ends the read. Their answers come back through `take`. `Err`: nothing held names a group for it (a root written
    /// before root parity): it is fetched alone.
    pub fn tail_group(
        &mut self,
        id: &[u8; 32],
        lost_contract: &[u8; 32],
    ) -> Result<Vec<String>, String> {
        let cid = self
            .wanted
            .get(lost_contract)
            .and_then(|w| w.iter().find(|(t, _)| t == id).or(w.first()))
            .map(|(_, c)| *c)
            .or_else(|| self.repairs.get(lost_contract).map(|(_, g)| g.missing))
            .ok_or("that block was not asked for")?;
        let o = self.tails.get(id).ok_or("that tail is not open")?;
        let group = o
            .repair_group(&cid)
            .ok_or("nothing held names a group for that block: it cannot be rebuilt")?;
        let missing: Vec<freenet_prolly::Cid> = group
            .slots
            .iter()
            .filter(|c| **c != cid && !o.blocks.0.contains_key(*c))
            .copied()
            .collect();
        self.repairs.insert(*lost_contract, (*id, group));
        // EVERY MEMBER HELD already: nothing to ask — the block rebuilds from what is here (`tail_rebuild`). Never an
        // error: an empty ask taken for "no group" declared a rebuildable block lost (owner 10-07, `spacekeys`).
        if missing.is_empty() {
            return Ok(Vec::new());
        }
        self.want(id, &missing)
    }

    /// Settle a raced block: `Ok(true)` once it is held — it arrived itself, or enough of its group did and it was
    /// rebuilt (verified against its id); `Ok(false)` while neither is so yet. Held: the race is forgotten.
    pub fn tail_rebuild(&mut self, lost_contract: &[u8; 32]) -> Result<bool, String> {
        let (id, group) = self
            .repairs
            .get(lost_contract)
            .cloned()
            .ok_or("no race under way for that block")?;
        let o = self.tail(&id)?;
        let held = o.blocks.0.contains_key(&group.missing) || o.rebuild(&group).is_ok();
        if held {
            self.rebuilt.insert(*lost_contract, group.missing);
            self.repairs.remove(lost_contract);
            self.wanted.remove(lost_contract);
        }
        Ok(held)
    }

    /// FLUSH an open table: `Ready` with the frames that PUT its new tree blocks (send and see them all accepted
    /// FIRST) and the step to sign; or `Need` with Block contract ids to GET first.
    pub fn tail_flush(&mut self, id: &[u8; 32]) -> Result<FlushOut, String> {
        self.tail_flush_given(id, None)
    }

    /// REPLACE an open table with exactly `rows` (`data::Open::replace`: a repair): as a flush, its tree made of them.
    pub fn tail_replace(&mut self, id: &[u8; 32], rows: Vec<(Vec<u8>, Vec<u8>)>) -> Result<FlushOut, String> {
        self.tail_flush_given(id, Some(rows))
    }

    fn tail_flush_given(&mut self, id: &[u8; 32], given: Option<Vec<(Vec<u8>, Vec<u8>)>>) -> Result<FlushOut, String> {
        let step = match self.read(id, |o| match &given {
            Some(rows) => o.replace(rows.clone()),
            None => o.flush(),
        })? {
            Err(fetch) => return Ok(FlushOut::Need(fetch)),
            Ok(s) => s,
        };
        match step {
            data::Step::Need(_) => unreachable!(),
            data::Step::Keys(epochs) => Ok(FlushOut::Keys(epochs)),
            data::Step::Ready(f) => {
                let tk = if f.sealed {
                    Some(f.address_key.ok_or("this table's key is not held here")?)
                } else {
                    None
                };
                let mut puts = Vec::new();
                for (cid, state) in f.blocks {
                    let s = self.stream();
                    // Sealed whole, at its address — or (a public tree) in the clear, the Block contract its id names.
                    let c = match tk {
                        Some(tk) => wire::block::block_contract(
                            &self.sealed_code,
                            &data::block_address(&tk, &cid),
                        ),
                        None => wire::block::block_contract(&self.block_code, &cid),
                    };
                    let name = c.key().id().encode();
                    puts.push((
                        name,
                        wire::frame_put(c, freenet_stdlib::prelude::WrappedState::new(state), s)?,
                    ));
                }
                Ok(FlushOut::Ready {
                    seq: f.seq,
                    hash: f.hash,
                    puts,
                })
            }
        }
    }

    pub fn take(&mut self, bytes: &[u8]) -> Value {
        match wire::unframe(&mut self.r, bytes) {
            Incoming::Got { id, state } => {
                if let Some(v) = self.tail_state(id, &state) {
                    return v;
                }
                // A tree block: HELD (any table that needs it again takes it from here), into EVERY table that asked
                // for it, and the answer always names its block — whoever waits on it is answered.
                if let Some(wanters) = self.wanted.remove(&id) {
                    self.held.insert(id, state.clone());
                    let mut first = None;
                    for (tail, cid) in wanters {
                        if let Some(o) = self.tails.get_mut(&tail) {
                            if !o.absorb_block(&cid, &state) {
                                let mut v = json!({ "kind": "tail-unreadable", "id": hex(&tail), "said": format!("block {} is not what was asked: sealed tree {}, its header {:?}, this table writes with {:?}, {} bytes", hex(&cid), o.sealed_tree(), data::KeyRef::of(&state).map(|(k, _)| k), o.writes, state.len()) });
                                v["block"] = json!(hex(&id));
                                return v;
                            }
                        }
                        first.get_or_insert(tail);
                    }
                    if let Some(tail) = first {
                        let mut v = self.tail_view(&tail);
                        v["block"] = json!(hex(&id)); // which fetch this answers
                        return v;
                    }
                }
                if self.held.contains_key(&id) {
                    // A block already held (a second answer): still named, so its waiter is answered.
                    return json!({ "kind": "got", "id": hex(&id), "block": hex(&id) });
                }
                self.got.insert(id, state);
                // Named as a block answer too: a block whose race was already won (rebuilt from its group first) still
                // releases the request that waits on it.
                json!({ "kind": "got", "id": hex(&id), "block": hex(&id) })
            }
            Incoming::HeadChanged {
                key,
                state: Some(state),
            } => {
                let Some(id) = freenet_stdlib::prelude::ContractInstanceId::from_base58(&key).ok().map(|i| *i) else {
                    return json!({ "kind": "changed", "key": key });
                };
                match self.tail_state(id, &state) {
                    Some(v) => v,
                    // ANY OTHER followed contract (a bag, a key log, a register): its pushed state HELD as its copy
                    // (FOUNDATION §2 R6) — a read of it answers from here, never another GET (10-15: the state was dropped
                    // and every reader asked the node again).
                    None => {
                        self.got.insert(id, state);
                        json!({ "kind": "changed", "key": key, "id": hex(&id) })
                    }
                }
            }
            Incoming::Ack(AckKind::Updated(key)) => json!({ "kind": "updated", "key": key }),
            Incoming::GetFailed { id, why } => {
                json!({ "kind": "get-failed", "id": hex(&id), "why": format!("{why:?}") })
            }
            other => describe(other),
        }
    }
}

/// A table's asset, from the core: the contracts to GET first (hex), the epochs whose keys to get, or its groups —
/// `(k, [(block id hex, contract id hex)])`.
pub enum AssetOut {
    Need(Vec<String>),
    Keys(Vec<u64>),
    Ready(Vec<(usize, Vec<(String, String)>)>),
}

/// A flush, from the core: the Block contract ids (hex) to GET first, or the block PUTs (by name, with their frames)
/// and the step to sign.
pub enum FlushOut {
    Need(Vec<String>),
    /// The keys of these epochs, first (a row is sealed under one not held yet).
    Keys(Vec<u64>),
    Ready {
        seq: u64,
        hash: [u8; 32],
        puts: Vec<(String, Vec<Vec<u8>>)>,
    },
}

#[cfg(all(target_arch = "wasm32", feature = "page"))]
fn prepared(
    params: &[u8],
    seq: u64,
    hash: [u8; 32],
) -> Result<js_sys::Object, wasm_bindgen::JsValue> {
    let out = js_sys::Object::new();
    js_sys::Reflect::set(
        &out,
        &"params".into(),
        &js_sys::Uint8Array::from(params).into(),
    )?;
    js_sys::Reflect::set(
        &out,
        &"seq".into(),
        &wasm_bindgen::JsValue::from(seq as f64),
    )?;
    js_sys::Reflect::set(
        &out,
        &"valueHash".into(),
        &js_sys::Uint8Array::from(&hash[..]).into(),
    )?;
    Ok(out)
}

/// 32 bytes from hex, or an error naming what.
pub fn bytes32(what: &str, h: &str) -> Result<[u8; 32], String> {
    unhex(h)
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| format!("{what}: not 32 bytes of hex"))
}


// THE RULES (crate `craftworks-rules`: the identity delegate runs them too) — re-exported, one implementation.
pub use craftworks_rules::{allowance, editions, keeper, tokens, wallet, witness};

#[cfg(all(target_arch = "wasm32", feature = "page"))]
mod js {
    use super::*;
    use wasm_bindgen::prelude::*;

    fn frames(v: Vec<Vec<u8>>) -> js_sys::Array {
        v.into_iter()
            .map(|f| JsValue::from(js_sys::Uint8Array::from(&f[..])))
            .collect()
    }

    fn err(e: String) -> JsValue {
        JsValue::from_str(&e)
    }

    /// A space's id from the page: 32 bytes, or empty for the account.
    fn space_of(b: &[u8]) -> Result<Option<[u8; 32]>, JsValue> {
        if b.is_empty() {
            Ok(None)
        } else {
            b32(b).map(Some)
        }
    }

    /// A list of byte arrays from the page.
    fn bytes_list(a: &js_sys::Array) -> Vec<Vec<u8>> {
        a.iter().map(|c| js_sys::Uint8Array::new(&c).to_vec()).collect()
    }
    fn b32(v: &[u8]) -> Result<[u8; 32], JsValue> {
        v.try_into().map_err(|_| err("expected 32 bytes".into()))
    }

    #[wasm_bindgen]
    pub struct CraftworksCore(Core);

    #[wasm_bindgen]
    impl CraftworksCore {
        #[wasm_bindgen(constructor)]
        pub fn new(identity_wasm: &[u8]) -> CraftworksCore {
            CraftworksCore(Core::new(identity_wasm))
        }
        pub fn identity_key(&self) -> String {
            self.0.identity_key()
        }
        pub fn frames_register_identity(&mut self) -> Result<js_sys::Array, JsValue> {
            self.0.frames_register_identity().map(frames).map_err(err)
        }
        fn ask(&mut self, r: Request) -> Result<js_sys::Array, JsValue> {
            let (id, f) = self.0.frames_identity(&r).map_err(err)?;
            Ok([JsValue::from(id), JsValue::from(frames(f))]
                .into_iter()
                .collect())
        }
        /// `[id, frames]` for each identity request.
        /// `data`: the account's data key seed (32 bytes), or empty.
        pub fn frames_provision(
            &mut self,
            seed: &[u8],
            did: &[u8],
            pin: String,
            data: &[u8],
        ) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Provision {
                seed: b32(seed)?,
                did: b32(did)?,
                pin,
                data: data.to_vec(),
            })
        }
        /// `[id, frames]` of a Handover asked of an earlier build (`<key>:<code hash>`).
        pub fn frames_handover_from(
            &mut self,
            prior: &str,
            pin: String,
        ) -> Result<js_sys::Array, JsValue> {
            let (id, f) = self.0.frames_handover(prior, pin).map_err(err)?;
            Ok([JsValue::from(id), JsValue::from(frames(f))]
                .into_iter()
                .collect())
        }
        /// `[id, frames]` of a HandoverSpaces asked of an earlier build: the member's spaces' groups and epoch secrets.
        pub fn frames_handover_spaces_from(
            &mut self,
            prior: &str,
            pin: String,
        ) -> Result<js_sys::Array, JsValue> {
            let (id, f) = self
                .0
                .frames_prior(prior, &Request::HandoverSpaces { pin })
                .map_err(err)?;
            Ok([JsValue::from(id), JsValue::from(frames(f))]
                .into_iter()
                .collect())
        }
        /// `[id, frames]` of an Unlock asked of an earlier build (before retiring its upkeep there).
        pub fn frames_unlock_from(
            &mut self,
            prior: &str,
            pin: String,
        ) -> Result<js_sys::Array, JsValue> {
            let (id, f) = self
                .0
                .frames_prior(prior, &Request::Unlock { pin })
                .map_err(err)?;
            Ok([JsValue::from(id), JsValue::from(frames(f))]
                .into_iter()
                .collect())
        }
        /// `[id, frames]` asking an EARLIER build (unlocked there) for a space's epoch secrets below `below`
        /// (`EpochSecrets`): what a handover from it did not carry (10-18: its handover walked the numbers just below
        /// the latest — once epochs were BeeKEM generations, the MLS ones were left behind).
        pub fn frames_epoch_secrets_from(&mut self, prior: &str, space: &[u8], below: f64) -> Result<js_sys::Array, JsValue> {
            let (id, f) = self.0.frames_prior(prior, &Request::EpochSecrets { space: b32(space)?, below: below as u64 }).map_err(err)?;
            Ok([JsValue::from(id), JsValue::from(frames(f))].into_iter().collect())
        }
        /// `[id, frames]` asking an earlier build for the PLACES its member signed as a witness (`HandoverWitnessed`).
        pub fn frames_handover_witnessed_from(&mut self, prior: &str, pin: String) -> Result<js_sys::Array, JsValue> {
            let (id, f) = self.0.frames_prior(prior, &Request::HandoverWitnessed { pin }).map_err(err)?;
            Ok([JsValue::from(id), JsValue::from(frames(f))].into_iter().collect())
        }
        /// KEEP an earlier build's witnessed places in this one's guard (`[[chain hex, seq, entry hex]]`).
        pub fn frames_witnessed_keep(&mut self, places: JsValue) -> Result<js_sys::Array, JsValue> {
            let places = js_sys::Array::from(&places)
                .iter()
                .map(|x| {
                    let x = js_sys::Array::from(&x);
                    Ok((bytes32("chain", &x.get(0).as_string().unwrap_or_default()).map_err(err)?, x.get(1).as_f64().unwrap_or(0.0) as u64, bytes32("entry", &x.get(2).as_string().unwrap_or_default()).map_err(err)?))
                })
                .collect::<Result<Vec<_>, JsValue>>()?;
            self.ask(Request::WitnessedKeep { places })
        }
        /// `[id, frames]` of an EMPTY mandate handed to an earlier build (its session's member): its upkeep does nothing
        /// more for them.
        pub fn frames_retire_upkeep_from(&mut self, prior: &str) -> Result<js_sys::Array, JsValue> {
            let (id, f) = self
                .0
                .frames_prior(prior, &Request::UpkeepMandate { me: String::new(), spaces: Vec::new(), spent: Vec::new() })
                .map_err(err)?;
            Ok([JsValue::from(id), JsValue::from(frames(f))]
                .into_iter()
                .collect())
        }
        /// `[id, frames]` of a HandoverKeys asked of an earlier build: the member's group state and epoch secrets.
        pub fn frames_handover_keys_from(
            &mut self,
            prior: &str,
            pin: String,
        ) -> Result<js_sys::Array, JsValue> {
            let (id, f) = self
                .0
                .frames_prior(prior, &Request::HandoverKeys { pin })
                .map_err(err)?;
            Ok([JsValue::from(id), JsValue::from(frames(f))]
                .into_iter()
                .collect())
        }
        pub fn frames_unlock(&mut self, pin: String) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Unlock { pin })
        }
        pub fn frames_lock(&mut self) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Lock)
        }
        /// Forget the session's member (a node removed from its account): home site only.
        pub fn frames_forget(&mut self) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Forget)
        }
        pub fn frames_who(&mut self) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Who)
        }
        /// Sign a record: `space` (32 bytes, or empty: the account) names whose epoch logs may be meant.
        /// `table`: the table's name when its label is blinded (empty: the label is the name).
        pub fn frames_sign(
            &mut self,
            params: &[u8],
            seq: u64,
            value_hash: &[u8],
            space: &[u8],
            table: &str,
        ) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Sign {
                params: params.to_vec(),
                seq,
                value_hash: b32(value_hash)?,
                space: space_of(space)?,
                table: (!table.is_empty()).then(|| table.to_string()),
            })
        }
        /// Hand the delegate the WITNESS MANDATE (`WitnessMandate`: what its round needs to witness with no page open) —
        /// the codes' hashes are of `token`, `idlog`, `bag`, `beacon_code`, `standing` (their bytes) and of this core's
        /// Block code (`set_block_code`: the statement's tree blocks), the beacon's and pool's ids.
        #[allow(clippy::too_many_arguments)]
        pub fn frames_witness_mandate(&mut self, token: &[u8], idlog: &[u8], bag: &[u8], beacon_code: &[u8], beacon: &[u8], pool: &[u8], standing: &[u8]) -> Result<js_sys::Array, JsValue> {
            let h = |b: &[u8]| *blake3::hash(b).as_bytes();
            if self.0.block_code.is_empty() {
                return Err(err("the Block code is not set (set_block_code)".into()));
            }
            // (A Block contract id: `contract_keys::block::contract_deriver_of_hash` of this hash — `code_hash` is BLAKE3.)
            let block = h(&self.0.block_code);
            self.ask(Request::WitnessMandate { mandate: craftworks_identity::WitnessMandate { token: h(token), idlog: h(idlog), bag: h(bag), beacon_code: h(beacon_code), beacon: b32(beacon)?, pool: b32(pool)?, standing: h(standing), block } })
        }
        /// KEEP WITH NO PAGE OPEN (`KeeperMandate`: the home site only): where this node's standing record is, the epoch
        /// this page opened and what it hosts (`[[id hex, size]]`, as `keeper_open` took it); the account (`did`, this
        /// device `public` and its `creds`: its key log read first) its listing and stake claim are made for, the codes
        /// they need, and whether this node holds the account's witness `seat`.
        #[allow(clippy::too_many_arguments)]
        pub fn frames_keeper_mandate(&mut self, standing_code: &[u8], beacon_code: &[u8], beacon: &[u8], epoch: f64, hosted: JsValue, token_code: &[u8], idlog_code: &[u8], bag_code: &[u8], did: &[u8], public: &[u8], creds: js_sys::Array, seat: bool) -> Result<js_sys::Array, JsValue> {
            let k = keeper_ctx(standing_code, beacon_code, beacon)?;
            let did = b32(did)?;
            let dev = account_device(&self.0, idlog_code, &did, public, &creds, &js_sys::Array::new())?.ok_or_else(|| err("the account's key log has not been read".into()))?;
            let h = |b: &[u8]| *blake3::hash(b).as_bytes();
            self.ask(Request::KeeperMandate {
                mandate: craftworks_identity::KeeperMandate { standing: k.standing, beacon_code: k.beacon_code, beacon: k.beacon, epoch: epoch as u64, hosted: hosted_of(&hosted)?, did, credential: dev.credential, key_seq: dev.key_seq, token: h(token_code), idlog: h(idlog_code), bag: h(bag_code), seat },
            })
        }
        /// WITNESS an entry (`WitnessSign`: the home site only; the delegate records the place first and never signs
        /// another entry there): `entry` (its hash, or a statement's root) at (`chain`, `seq`), signed at beacon height
        /// `at` (the page's, now — `witness_put` carries the same).
        pub fn frames_witness_sign(&mut self, chain: &[u8], seq: f64, entry: &[u8], at: f64) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::WitnessSign { chain: b32(chain)?, seq: seq as u64, entry: b32(entry)?, at: at as u64 })
        }
        /// Sign a TOKEN ENTRY's message (`TokenSign`: the home site only, a message under the token's entry domain).
        pub fn frames_token_sign(&mut self, message: &[u8]) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::TokenSign { message: message.to_vec() })
        }
        /// Leave to write these tables, asked in one prompt (the node may prompt the person; the answer can take a
        /// minute).
        pub fn frames_grant(&mut self, tables: Vec<String>) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Grant { tables })
        }
        /// Keep this member's MLS state of a space's group (`space` 32 bytes, or empty: the account) and its epoch's
        /// secret (the home site only).
        pub fn frames_mls_save(
            &mut self,
            state: &[u8],
            epoch: f64,
            secret: &[u8],
            space: &[u8],
        ) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::MlsSave {
                space: space_of(space)?,
                state: state.to_vec(),
                epoch: epoch as u64,
                secret: b32(secret)?,
            })
        }
        /// Keep an earlier epoch's secret of a space's group (recovered from escrow, or walked).
        /// The secrets of a space's epochs below `below` this member holds (for a member let in later).
        pub fn frames_epoch_secrets(&mut self, space: &[u8], below: f64) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::EpochSecrets { space: b32(space)?, below: below as u64 })
        }
        pub fn frames_epoch_keep(
            &mut self,
            epoch: f64,
            secret: &[u8],
            space: &[u8],
        ) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::EpochKeep {
                space: space_of(space)?,
                epoch: epoch as u64,
                secret: b32(secret)?,
            })
        }
        pub fn frames_inbox_key(&mut self) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::InboxKey)
        }
        /// The DID's member for spaces (the home site only): its MLS seed, keys and credential.
        pub fn frames_space_member(&mut self) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::SpaceMember)
        }
        /// UPKEEP with no page open: the account's inbox contract (its instance id), for the delegate to watch and read
        /// at each wake-up (the home site only); and what upkeep has done.
        /// The inbox to watch, with the page's randomness and time (the delegate has neither).
        pub fn frames_upkeep_watch(
            &mut self,
            inbox: &[u8],
            seed: &[u8],
            now: f64,
        ) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::UpkeepWatch {
                inbox: b32(inbox)?,
                seed: b32(seed)?,
                now: (now / 1000.0) as u64,
            })
        }
        pub fn frames_upkeep_status(&mut self) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::UpkeepStatus)
        }
        /// The contracts upkeep writes (bag, tail: their code) and reads (the key log: its code's hash).
        pub fn frames_upkeep_codes(
            &mut self,
            bag: &[u8],
            tail: &[u8],
            idlog: &[u8],
            sealed: &[u8],
            piece: &[u8],
            block: &[u8],
            devices: &[u8],
        ) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::UpkeepCodes {
                bag: bag.to_vec(),
                tail: tail.to_vec(),
                idlog: *blake3::hash(idlog).as_bytes(),
                sealed: sealed.to_vec(),
                piece: piece.to_vec(),
                block: *blake3::hash(block).as_bytes(),
                devices: *blake3::hash(devices).as_bytes(),
            })
        }
        /// The codes' hash upkeep would hold for these (to hand them over only when they changed).
        pub fn upkeep_codes_hash(bag: &[u8], tail: &[u8], idlog: &[u8], sealed: &[u8], piece: &[u8], block: &[u8], devices: &[u8]) -> String {
            hex(&craftworks_identity::upkeep_codes_hash(
                bag,
                tail,
                blake3::hash(idlog).as_bytes(),
                sealed,
                piece,
                blake3::hash(block).as_bytes(),
                blake3::hash(devices).as_bytes(),
            ))
        }
        /// The MANDATE: `me` (the account's DID) and, per space, JSON `{ space, name, kind, owner, nonce, channel,
        /// epoch, state }` (ids and the state in hex).
        pub fn frames_upkeep_mandate(&mut self, me: &str, spaces: &str) -> Result<js_sys::Array, JsValue> {
            let v: Vec<serde_json::Value> =
                serde_json::from_str(spaces).map_err(|e| err(e.to_string()))?;
            let spaces = v
                .iter()
                .map(|m| -> Option<craftworks_identity::Mandate> {
                    let s = |k: &str| m.get(k).and_then(|x| x.as_str()).map(str::to_string);
                    Some(craftworks_identity::Mandate {
                        space: unhex(&s("space")?)?.try_into().ok()?,
                        name: s("name").unwrap_or_default(),
                        kind: s("kind")?,
                        owner: s("owner")?,
                        nonce: s("nonce"),
                        channel: s("channel")?,
                        epoch: m.get("epoch")?.as_f64()? as u64,
                        state: unhex(&s("state")?)?,
                    })
                })
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| err("a space of the mandate does not read".into()))?;
            self.ask(Request::UpkeepMandate { me: me.to_string(), spaces, spent: Vec::new() })
        }
        /// THE FOLLOWER NOW (FOUNDATION "Members and keys"): the node runs its round at once (someone the list put out,
        /// its turn come) — the one place the unlisted are taken out; a page never does it itself.
        pub fn frames_follow_now(&mut self, now_ms: f64) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::FollowNow { now: (now_ms / 1000.0) as u64 })
        }
        /// The groups the page loaded (JSON `[space hex]`): no longer reported as moved.
        pub fn frames_upkeep_ack(&mut self, groups: &str) -> Result<js_sys::Array, JsValue> {
            let v: Vec<String> = serde_json::from_str(groups).map_err(|e| err(e.to_string()))?;
            self.ask(Request::UpkeepAck { groups: v.iter().filter_map(|s| unhex(s)?.try_into().ok()).collect() })
        }
        /// Open items sealed to the account's inbox key (the home site only).
        pub fn frames_inbox_open(
            &mut self,
            items: js_sys::Array,
        ) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::InboxOpen {
                items: items
                    .iter()
                    .map(|i| js_sys::Uint8Array::new(&i).to_vec())
                    .collect(),
            })
        }
        pub fn frames_mls_load(&mut self, space: &[u8]) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::MlsLoad {
                space: space_of(space)?,
            })
        }
        /// A table's key in an MLS epoch (`epoch` < 0: the newest held) of a space's group.
        pub fn frames_table_key_at(
            &mut self,
            table: String,
            epoch: f64,
            space: &[u8],
        ) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::TableKeyAt {
                table,
                epoch: (epoch >= 0.0).then_some(epoch as u64),
                space: space_of(space)?,
            })
        }
        /// The key that seals table `table` (generation `gen`): given to a granted site only.
        pub fn frames_table_key(
            &mut self,
            table: String,
            gen: u8,
        ) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::TableKey { table, gen })
        }
        pub fn frames_grants(&mut self) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Grants)
        }
        /// Withdraw a grant: `app` is the site's id in its text form.
        pub fn frames_revoke(
            &mut self,
            app: &str,
            table: String,
        ) -> Result<js_sys::Array, JsValue> {
            let app = freenet_stdlib::prelude::ContractInstanceId::from_base58(app)
                .map_err(|e| err(format!("not a site id: {e}")))?;
            self.ask(Request::Revoke { app: *app, table })
        }
        /// For the next version of the identity delegate: the member this PIN opens (to its home app).
        pub fn frames_handover(&mut self, pin: String) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::Handover { pin })
        }

        /// The account of these recovery words (their entropy), with `member` admitted to it: `{ did, didBytes,
        /// members: { id, frames } }`. The DID is the owner's public key, so the same words always name the same
        /// account; the member PUT merges into the account's Set. So this is both "register" and "log in with words".
        /// `{ id, frames }` of a PUT, for the page.
        fn put_obj(&mut self, p: &account::Put) -> Result<JsValue, JsValue> {
            let o = js_sys::Object::new();
            js_sys::Reflect::set(&o, &"id".into(), &p.id.clone().into())?;
            js_sys::Reflect::set(
                &o,
                &"idBytes".into(),
                &js_sys::Uint8Array::from(&p.id_bytes[..]).into(),
            )?;
            js_sys::Reflect::set(
                &o,
                &"frames".into(),
                &frames(self.0.frames_put(p).map_err(err)?).into(),
            )?;
            Ok(o.into())
        }

        /// What words W may hold, before anything is read: their `whoami` Register (to GET: it names the DID if W
        /// were rotated in) and the DID of their own inception (if they are an account's original words).
        pub fn words_plan(
            &mut self,
            register_code: &[u8],
            entropy: &[u8],
        ) -> Result<js_sys::Object, JsValue> {
            let bad = || err("recovery entropy: 16 or 32 bytes".into());
            let who = account::whoami_address(register_code, entropy).ok_or_else(bad)?;
            let e = account::inception(entropy).ok_or_else(bad)?;
            let o = js_sys::Object::new();
            js_sys::Reflect::set(
                &o,
                &"whoamiId".into(),
                &js_sys::Uint8Array::from(&who.id_bytes[..]).into(),
            )?;
            js_sys::Reflect::set(
                &o,
                &"inceptionDid".into(),
                &js_sys::Uint8Array::from(&e.id()[..]).into(),
            )?;
            Ok(o)
        }

        /// The DID the got `whoami` Register of W names, or null.
        pub fn whoami_did(&self, register_code: &[u8], entropy: &[u8]) -> Result<JsValue, JsValue> {
            let id = account::whoami_address(register_code, entropy)
                .ok_or_else(|| err("recovery entropy: 16 or 32 bytes".into()))?
                .id_bytes;
            Ok(
                match self
                    .0
                    .got(&id)
                    .and_then(|st| account::whoami_did(entropy, st))
                {
                    Some(d) => js_sys::Uint8Array::from(&d[..]).into(),
                    None => JsValue::NULL,
                },
            )
        }

        /// The key event log's contract id (32 bytes) for a DID.
        pub fn idlog_id(idlog_code: &[u8], did: &[u8]) -> Result<js_sys::Uint8Array, JsValue> {
            Ok(js_sys::Uint8Array::from(
                &account::idlog_put(idlog_code, &b32(did)?, None).id_bytes[..],
            ))
        }

        /// The got log of `did`, verified; or, for W's own DID with nothing on the network, W's inception (to PUT).
        fn log_of(
            &self,
            idlog_code: &[u8],
            did: &[u8; 32],
            entropy: &[u8],
        ) -> Result<craftworks_idlog_contract::Log, JsValue> {
            let id = account::idlog_put(idlog_code, did, None).id_bytes;
            if let Some(st) = self.0.got(&id) {
                return craftworks_idlog_contract::read(did, st)
                    .ok_or_else(|| err("the account's key log does not verify".into()));
            }
            match account::inception(entropy) {
                Some(e) if e.id() == *did => Ok(craftworks_idlog_contract::Log { events: vec![e] }),
                _ => Err(err("the account's key log is not on the network".into())),
            }
        }

        /// WHO BELONGS to the account (`account::members`): the node keys (hex) that the gathered credentials (bytes) make
        /// members, less the removals that count (`[[by, node, epoch]]`, each found in its remover's own feed), checked
        /// against the account's (got) key log.
        pub fn account_members(
            &self,
            idlog_code: &[u8],
            did: &[u8],
            creds: js_sys::Array,
            removals: js_sys::Array,
        ) -> Result<js_sys::Array, JsValue> {
            let did = b32(did)?;
            let id = account::idlog_put(idlog_code, &did, None).id_bytes;
            let st = self
                .0
                .got(&id)
                .ok_or_else(|| err("the account's key log has not been read".into()))?;
            let log = craftworks_idlog_contract::read(&did, st)
                .ok_or_else(|| err("the account's key log does not verify".into()))?;
            let creds = bytes_list(&creds);
            let mut rs = Vec::new();
            for r in removals.iter() {
                let r = js_sys::Array::from(&r);
                rs.push((
                    b32(&js_sys::Uint8Array::new(&r.get(0)).to_vec())?,
                    b32(&js_sys::Uint8Array::new(&r.get(1)).to_vec())?,
                    r.get(2).as_f64().unwrap_or(0.0) as u64,
                ));
            }
            Ok(account::members(&did, &log, &creds, &rs)
                .iter()
                .map(|n| JsValue::from(hex(n)))
                .collect())
        }

        /// An account's current PUBLIC keys from its (got) key log's head: `{ data, enc }` (hex) — its data key (its
        /// tables and card are addressed by it) and its encryption key (what is sealed to the account).
        pub fn idlog_keys(&self, idlog_code: &[u8], did: &[u8]) -> Result<js_sys::Object, JsValue> {
            let did = b32(did)?;
            let id = account::idlog_put(idlog_code, &did, None).id_bytes;
            let st = self
                .0
                .got(&id)
                .ok_or_else(|| err("the account's key log has not been read".into()))?;
            let log = craftworks_idlog_contract::read(&did, st)
                .ok_or_else(|| err("the account's key log does not verify".into()))?;
            let o = js_sys::Object::new();
            js_sys::Reflect::set(&o, &"data".into(), &JsValue::from(hex(&log.head().data)))?;
            js_sys::Reflect::set(&o, &"enc".into(), &JsValue::from(hex(&log.head().enc)))?;
            Ok(o)
        }

        /// `{ events, changes }` of the account's (got) key log: its events, and how many times its keys changed (each
        /// change of the words is two events).
        pub fn idlog_info(&self, idlog_code: &[u8], did: &[u8]) -> Result<js_sys::Object, JsValue> {
            let did = b32(did)?;
            let id = account::idlog_put(idlog_code, &did, None).id_bytes;
            let st = self
                .0
                .got(&id)
                .ok_or_else(|| err("the account's key log has not been read".into()))?;
            let log = craftworks_idlog_contract::read(&did, st)
                .ok_or_else(|| err("the account's key log does not verify".into()))?;
            let o = js_sys::Object::new();
            js_sys::Reflect::set(
                &o,
                &"events".into(),
                &JsValue::from(log.events.len() as u32),
            )?;
            js_sys::Reflect::set(
                &o,
                &"changes".into(),
                &JsValue::from(((log.events.len() - 1) / 2) as u32),
            )?;
            Ok(o)
        }

        /// The account's key log as the network holds it (read first): for the `mls` package, which verifies it again.
        pub fn idlog_state(
            &self,
            idlog_code: &[u8],
            did: &[u8],
        ) -> Result<js_sys::Uint8Array, JsValue> {
            let id = account::idlog_put(idlog_code, &b32(did)?, None).id_bytes;
            let st = self
                .0
                .got(&id)
                .ok_or_else(|| err("the account's key log has not been read".into()))?;
            Ok(js_sys::Uint8Array::from(st))
        }

        /// JOIN the account words W hold: the log (PUT again: re-publishing a signed log is always safe, and keeps it
        /// on the network) and the data key's seed out of the vault. Refused if W are not the account's current words.
        /// (Which NODES are the account's is its MLS group's roster: the `keys` capability.)
        pub fn join_account(
            &mut self,
            idlog_code: &[u8],
            entropy: &[u8],
            did: &[u8],
        ) -> Result<js_sys::Object, JsValue> {
            let did = b32(did)?;
            let log = self.log_of(idlog_code, &did, entropy)?;
            let (_, _, data) = account::open_log(entropy, &log).ok_or_else(|| {
                err("these recovery words were replaced by newer ones: use the newest words".into())
            })?;
            let o = js_sys::Object::new();
            let set = |k: &str, v: JsValue| js_sys::Reflect::set(&o, &k.into(), &v).map(|_| ());
            set("did", account::did(&did).into())?;
            set("didBytes", js_sys::Uint8Array::from(&did[..]).into())?;
            set("data", js_sys::Uint8Array::from(&data[..]).into())?;
            let lp = account::idlog_put(idlog_code, &did, Some(&log));
            set("log", self.put_obj(&lp)?)?;
            Ok(o)
        }

        /// Whether `entropy` is the account `did`'s CURRENT words (its key log read first): what a change of words checks
        /// before it goes on.
        pub fn words_open(&mut self, idlog_code: &[u8], did: &[u8], entropy: &[u8]) -> Result<bool, JsValue> {
            let did = b32(did)?;
            let log = self.log_of(idlog_code, &did, entropy)?;
            Ok(account::open_log(entropy, &log).is_some())
        }

        /// CHANGE THE RECOVERY WORDS from `old` to `new` (both entered on this page): the log with its two rotations and
        /// the new words' `whoami`, to PUT. (The account's nodes stay its MLS group's members: every owner key the log
        /// ever had signs for them.)
        /// CHANGE THE WORDS: the key log's two rotations, the new words' whoami — and the account's devices (`creds`: the
        /// credentials of the devices it has now) credentialed AGAIN under the new words, into its devices contract: on
        /// the new key they alone count (owner 10-09).
        pub fn change_words(
            &mut self,
            idlog_code: &[u8],
            register_code: &[u8],
            devices_code: &[u8],
            did: &[u8],
            old: &[u8],
            new: &[u8],
            creds: js_sys::Array,
        ) -> Result<js_sys::Object, JsValue> {
            let did = b32(did)?;
            let log = self.log_of(idlog_code, &did, old)?;
            let next = account::change_words(&log, old, new).ok_or_else(|| {
                err("the current recovery words are needed to change them (these are not)".into())
            })?;
            let o = js_sys::Object::new();
            let set = |k: &str, v: JsValue| js_sys::Reflect::set(&o, &k.into(), &v).map(|_| ());
            let lp = account::idlog_put(idlog_code, &did, Some(&next));
            set("log", self.put_obj(&lp)?)?;
            let who = account::whoami_put(register_code, new, &did)
                .ok_or_else(|| err("whoami".into()))?;
            set("whoami", self.put_obj(&who)?)?;
            let creds = bytes_list(&creds);
            let again = account::reissue(&did, &next, new, &creds).ok_or_else(|| err("a device's credential is not this account's".into()))?;
            set("devices", self.put_obj(&account::devices_put(devices_code, idlog_code, &did, &again))?)?;
            Ok(o)
        }

        /// REMOVE A DEVICE (owner 10-09: with the words): the key ROTATES (the same words' next key) and the devices that
        /// STAY (`keep`: their credentials) are credentialed again — the removed one, on the old key, stops counting
        /// everywhere, and cannot be credentialed again without the words.
        pub fn remove_device(
            &mut self,
            idlog_code: &[u8],
            devices_code: &[u8],
            did: &[u8],
            words: &[u8],
            keep: js_sys::Array,
        ) -> Result<js_sys::Object, JsValue> {
            let did = b32(did)?;
            let log = self.log_of(idlog_code, &did, words)?;
            let next = account::rotate(&log, words).ok_or_else(|| err("these are not this account's current recovery words".into()))?;
            let again = account::reissue(&did, &next, words, &bytes_list(&keep)).ok_or_else(|| err("a device's credential is not this account's".into()))?;
            let o = js_sys::Object::new();
            let set = |k: &str, v: JsValue| js_sys::Reflect::set(&o, &k.into(), &v).map(|_| ());
            set("log", self.put_obj(&account::idlog_put(idlog_code, &did, Some(&next)))?)?;
            set("devices", self.put_obj(&account::devices_put(devices_code, idlog_code, &did, &again))?)?;
            Ok(o)
        }

        /// The account's DEVICES contract with these credentials (each device puts its own: the node keeps the union).
        pub fn devices_put(&mut self, devices_code: &[u8], idlog_code: &[u8], did: &[u8], creds: js_sys::Array) -> Result<JsValue, JsValue> {
            let did = b32(did)?;
            self.put_obj(&account::devices_put(devices_code, idlog_code, &did, &bytes_list(&creds)))
        }

        /// The account's devices NOW — the one resolution (`craftworks_account::devices`, the node's too): its devices
        /// contract (when read: `got`) on the key log's current key, else its CARD's credentials (`card`: bytes each).
        /// Node keys, hex. The key log read first.
        pub fn account_devices(&self, devices_code: &[u8], idlog_code: &[u8], did: &[u8], card: js_sys::Array) -> Result<js_sys::Array, JsValue> {
            let did = b32(did)?;
            let dev = account::devices_put(devices_code, idlog_code, &did, &[]).id_bytes;
            let contract = match self.0.got(&dev) {
                Some(st) => Some(craftworks_devices_contract::decode(st).ok_or_else(|| err("the account's devices do not read".into()))?),
                None => None,
            };
            let id = account::idlog_put(idlog_code, &did, None).id_bytes;
            let st = self.0.got(&id).ok_or_else(|| err("the account's key log has not been read".into()))?;
            let log = craftworks_idlog_contract::read(&did, st).ok_or_else(|| err("the account's key log does not verify".into()))?;
            Ok(account::devices(&did, &log, contract.as_deref(), &bytes_list(&card)).iter().map(|n| JsValue::from(hex(n))).collect())
        }

        /// A member's DEVICES, one STEP (`craftworks_account::devices_step`, the node's too): their key log and devices
        /// contract as read here (`got`), their card's credentials (`card`: null — not read) and whether it says they
        /// have a contract (`marked`); `asked`: the contract was asked for. → JSON `{ "readDevices": true }` (ask it, then
        /// step again) or `{ "devices": [hex], "unread": why | null }`.
        pub fn account_devices_step(&self, devices_code: &[u8], idlog_code: &[u8], did: &[u8], card: JsValue, marked: bool, asked: bool) -> Result<String, JsValue> {
            let did = b32(did)?;
            let log = self.0.got(&account::idlog_put(idlog_code, &did, None).id_bytes).and_then(|st| craftworks_idlog_contract::read(&did, st));
            let dev = account::devices_put(devices_code, idlog_code, &did, &[]).id_bytes;
            let contract = asked.then(|| self.0.got(&dev).and_then(craftworks_devices_contract::decode));
            let creds = (!card.is_null() && !card.is_undefined()).then(|| bytes_list(&js_sys::Array::from(&card)));
            Ok(match account::devices_step(&did, log.as_ref(), creds.as_deref().map(|c| (c, marked)), contract.as_ref().map(|c| c.as_deref())) {
                account::DevicesStep::ReadDevices => serde_json::json!({ "readDevices": true }),
                account::DevicesStep::Done { devices, unread } => serde_json::json!({ "devices": devices.iter().map(|n| hex(n)).collect::<Vec<_>>(), "unread": unread }),
            }
            .to_string())
        }

        /// EVERY DEVICE the account has had (`craftworks_account::devices_ever`), its devices contract read (`got`):
        /// node keys, hex. Empty where the contract was not read.
        pub fn account_devices_ever(&self, devices_code: &[u8], idlog_code: &[u8], did: &[u8]) -> Result<js_sys::Array, JsValue> {
            let did = b32(did)?;
            let dev = account::devices_put(devices_code, idlog_code, &did, &[]).id_bytes;
            let Some(st) = self.0.got(&dev) else { return Ok(js_sys::Array::new()) };
            let creds = craftworks_devices_contract::decode(st).ok_or_else(|| err("the account's devices do not read".into()))?;
            Ok(account::devices_ever(&did, &creds).iter().map(|n| JsValue::from(hex(n))).collect())
        }

        /// WHO WROTE A SPACE's ROWS — the one map (`craftworks_account::writers`, the node's too): `members` JSON
        /// `[[did, [node hex…]]…]` (each member's current devices), `departed` JSON `[[node hex, row value]…]` (the
        /// space's `departed` table). → JSON `[{ node, did, caps }]` (`caps`: table → seq, a departed writer's; null).
        pub fn space_writers(members: &str, departed: &str) -> Result<String, JsValue> {
            let m: Vec<(String, Vec<String>)> = serde_json::from_str(members).map_err(|e| err(format!("members: {e}")))?;
            let d: Vec<(String, String)> = serde_json::from_str(departed).map_err(|e| err(format!("departed: {e}")))?;
            let m: Vec<(String, Vec<[u8; 32]>)> = m.into_iter().map(|(did, ns)| (did, ns.iter().filter_map(|h| craftworks_identity::unhex32(h)).collect())).collect();
            let out: Vec<serde_json::Value> = account::writers(&m, &d).into_iter().map(|w| serde_json::json!({ "node": hex(&w.node), "did": w.did, "caps": w.caps })).collect();
            Ok(serde_json::Value::Array(out).to_string())
        }

        /// WHOSE DEVICES a space's tables are read from (`craftworks_account::reading_dids`, the node's too): `group`
        /// JSON `[did…]`, the space's owner. → JSON `[did…]`.
        pub fn reading_dids(group: &str, owner: Option<String>) -> Result<String, JsValue> {
            let g: Vec<String> = serde_json::from_str(group).map_err(|e| err(format!("group: {e}")))?;
            Ok(serde_json::to_string(&account::reading_dids(&g, owner.as_deref())).expect("a list encodes"))
        }
        /// THE WRITERS A TABLE IS READ FROM (`craftworks_account::table_writers`, the node's too): hex JSON lists of the
        /// members' devices, the writers bag's listed, the open list's writers for this table; `blind`, `public`; `me`.
        pub fn table_writers(devices: &str, listed: &str, blind: bool, named: &str, public: bool, me: &str) -> Result<String, JsValue> {
            let hexes = |j: &str, what: &str| -> Result<Vec<[u8; 32]>, JsValue> { Ok(serde_json::from_str::<Vec<String>>(j).map_err(|e| err(format!("{what}: {e}")))?.iter().filter_map(|h| craftworks_identity::unhex32(h)).collect()) };
            let listed: std::collections::BTreeSet<[u8; 32]> = hexes(listed, "listed")?.into_iter().collect();
            let me = craftworks_identity::unhex32(me).unwrap_or([0; 32]);
            let out = account::table_writers(&hexes(devices, "devices")?, &listed, blind, &hexes(named, "named")?, public, &me);
            Ok(serde_json::to_string(&out.iter().map(|n| hex(n)).collect::<Vec<_>>()).expect("a list encodes"))
        }

        /// The account's data key seed from the words' entropy (handed once to the identity delegate, never kept).
        pub fn data_seed(entropy: &[u8]) -> Result<js_sys::Uint8Array, JsValue> {
            account::data_seed(entropy)
                .map(|d| js_sys::Uint8Array::from(&d[..]))
                .ok_or_else(|| err("recovery entropy: 16 or 32 bytes".into()))
        }

        /// The BIP39 words of 16 or 32 bytes of entropy.
        pub fn words_of(entropy: &[u8]) -> Result<String, JsValue> {
            account::words(entropy).ok_or_else(|| err("recovery entropy: 16 or 32 bytes".into()))
        }

        /// RECOVERY: the words' entropy sealed under a passphrase for the account `did` (salt 16 and nonce 24 bytes of
        /// fresh randomness from the page) — `account::passphrase_seal`.
        pub fn recovery_seal(
            passphrase: &str,
            did: &[u8],
            entropy: &[u8],
            salt: &[u8],
            nonce: &[u8],
        ) -> Result<js_sys::Uint8Array, JsValue> {
            let salt: [u8; 16] = salt
                .try_into()
                .map_err(|_| err("a salt is 16 bytes".into()))?;
            let nonce: [u8; 24] = nonce
                .try_into()
                .map_err(|_| err("a nonce is 24 bytes".into()))?;
            let sealed = account::passphrase_seal(passphrase, &b32(did)?, entropy, salt, nonce)
                .ok_or_else(|| err("could not seal the words".into()))?;
            Ok(js_sys::Uint8Array::from(&sealed[..]))
        }
        /// The words' entropy out of a recovery copy, with its passphrase; an error if it does not open.
        pub fn recovery_open(
            passphrase: &str,
            did: &[u8],
            sealed: &[u8],
        ) -> Result<js_sys::Uint8Array, JsValue> {
            let e = account::passphrase_open(passphrase, &b32(did)?, sealed).ok_or_else(|| {
                err("that passphrase does not open this account's recovery".into())
            })?;
            Ok(js_sys::Uint8Array::from(&e[..]))
        }

        /// The entropy of 12 or 24 BIP39 words; an error names why not.
        pub fn entropy_of(words: &str) -> Result<js_sys::Uint8Array, JsValue> {
            account::entropy(words)
                .map(|e| js_sys::Uint8Array::from(&e[..]))
                .ok_or_else(|| err("these are not 12 or 24 recovery words (a word is misspelled, or one is missing)".into()))
        }

        /// Frames asking the node about itself: its peer id, ring location and hosted contracts.
        pub fn frames_node_self(&mut self) -> Result<js_sys::Array, JsValue> {
            Ok(frames(self.0.frames_node_self().map_err(err)?))
        }

        /// Frames asking the node for its connected peers.
        pub fn frames_peers(&mut self) -> Result<js_sys::Array, JsValue> {
            Ok(frames(self.0.frames_peers().map_err(err)?))
        }

        /// `[id hex, frames]` of a GET of contract `id` (32 bytes).
        pub fn frames_get(&mut self, id: &[u8]) -> Result<js_sys::Array, JsValue> {
            let id = b32(id)?;
            let f = self.0.frames_get(id).map_err(err)?;
            Ok([JsValue::from(hex(&id)), JsValue::from(frames(f))]
                .into_iter()
                .collect())
        }

        /// `[id hex, frames]` of a GET of contract `id` that subscribes this node to it (its newest, and what follows).
        pub fn frames_follow(&mut self, id: &[u8]) -> Result<js_sys::Array, JsValue> {
            let id = b32(id)?;
            let f = self.0.frames_follow(id).map_err(err)?;
            Ok([JsValue::from(hex(&id)), JsValue::from(frames(f))]
                .into_iter()
                .collect())
        }

        /// A person's INBOX: its address (the bag's params) from their DID — every sender computes the same.
        pub fn inbox_address(did: &[u8]) -> Result<js_sys::Uint8Array, JsValue> {
            Ok(js_sys::Uint8Array::from(
                &craftworks_identity::inbox_address(&b32(did)?)[..],
            ))
        }

        /// An INVITE CODE's bag address (`identity::invite_address`; `open <space id>`: an open space's).
        pub fn invite_address(code: &str) -> js_sys::Uint8Array {
            js_sys::Uint8Array::from(&craftworks_identity::invite_address(code)[..])
        }

        /// SEAL `data` to a public key (`identity::seal_to`), with a one-time key the page draws (`eph`, 32 bytes).
        pub fn seal_to(
            public: &[u8],
            data: &[u8],
            eph: &[u8],
        ) -> Result<js_sys::Uint8Array, JsValue> {
            Ok(js_sys::Uint8Array::from(
                &craftworks_identity::seal_to(&b32(public)?, data, b32(eph)?)[..],
            ))
        }

        /// `[id hex, frames]`: ADD an item carrying `payload` to the bag at `address` (the work found here: the sender's
        /// cost), as a PUT the host merges into what it holds. An empty payload: the empty bag (made).
        pub fn bag_add(
            &mut self,
            bag_code: &[u8],
            address: &[u8],
            payload: &[u8],
        ) -> Result<js_sys::Array, JsValue> {
            let items = if payload.is_empty() {
                Vec::new()
            } else {
                vec![craftworks_bag_contract::grind(address, payload)]
            };
            let state = craftworks_bag_contract::encode(address, &items);
            let (_, c, w) = wire::puts::contract(bag_code, address, &state);
            let id: [u8; 32] = c
                .key()
                .id()
                .as_bytes()
                .try_into()
                .map_err(|_| err("a contract id is 32 bytes".into()))?;
            let s = self.0.stream();
            let f = wire::frame_put(c, w, s).map_err(err)?;
            Ok([JsValue::from(hex(&id)), JsValue::from(frames(f))]
                .into_iter()
                .collect())
        }

        /// The bag at `address`'s contract id (hex): to GET it.
        pub fn bag_id(bag_code: &[u8], address: &[u8]) -> Result<String, JsValue> {
            let (_, c, _) = wire::puts::contract(bag_code, address, &[]);
            Ok(hex(&c.key().id().as_bytes()[..32]))
        }

        /// The PAYLOADS of the bag the node last sent (by contract id, hex), each as bytes; null if none was got.
        pub fn bag_payloads(&self, address: &[u8], id_hex: &str) -> Result<JsValue, JsValue> {
            let id = bytes32("bag", id_hex).map_err(err)?;
            let Some(st) = self.0.got(&id) else {
                return Ok(JsValue::NULL);
            };
            let items = craftworks_bag_contract::read(address, st)
                .ok_or_else(|| err("that bag does not read".into()))?;
            Ok(items
                .iter()
                .map(|i| {
                    JsValue::from(js_sys::Uint8Array::from(craftworks_bag_contract::payload(
                        i,
                    )))
                })
                .collect::<js_sys::Array>()
                .into())
        }

        /// The 32 bytes of a contract id written in base58 (as in a `/v1/contract/web/<id>/` path).
        pub fn id_bytes(id: &str) -> Result<js_sys::Uint8Array, JsValue> {
            let i = freenet_stdlib::prelude::ContractInstanceId::from_base58(id)
                .map_err(|e| err(format!("not a contract id: {e}")))?;
            Ok(js_sys::Uint8Array::from(&i[..]))
        }

        /// Open one of the account's tables under its data key: its contract id (hex).
        pub fn tail_open(
            &mut self,
            tail_code: &[u8],
            key: &[u8],
            table: &str,
        ) -> Result<String, JsValue> {
            if table.is_empty() || table.len() > 32 {
                return Err(err("a table's name is 1 to 32 bytes".into()));
            }
            Ok(hex(&self.0.tail_open(tail_code, &b32(key)?, table)))
        }

        /// `[id hex, frames]`: read an open tail and follow it.
        pub fn tail_get(&mut self, id: &[u8]) -> Result<js_sys::Array, JsValue> {
            let id = b32(id)?;
            let f = self.0.frames_tail_get(&id).map_err(err)?;
            Ok([JsValue::from(hex(&id)), JsValue::from(frames(f))]
                .into_iter()
                .collect())
        }

        /// Forget what this page holds of an open table (a write the node refused, so the local step never landed):
        /// read it again before the next write.
        pub fn tail_reset(&mut self, id: &[u8]) -> Result<(), JsValue> {
            self.0.tail(&b32(id)?).map_err(err)?.reset();
            Ok(())
        }

        /// Mark an open tail as absent from the network (a GET found none): its first write is a PUT.
        pub fn tail_absent(&mut self, id: &[u8]) -> Result<(), JsValue> {
            self.0.tail(&b32(id)?).map_err(err)?.on_network = false;
            Ok(())
        }

        /// The next write to an open tail: set `key` to `value` (empty value: delete). Returns `{ params, seq,
        /// valueHash }`, what the identity delegate signs; nothing moves until `tail_commit`.
        pub fn tail_prepare(
            &mut self,
            id: &[u8],
            key: &[u8],
            value: &[u8],
        ) -> Result<js_sys::Object, JsValue> {
            let o = self.0.tail(&b32(id)?).map_err(err)?;
            if o.writes.is_none() && !o.public {
                return Err(err(
                    "this table's key is not held here: it cannot be written".into(),
                ));
            }
            let (seq, hash) = o
                .prepare_row(key, value)
                .ok_or_else(|| err("the tail refuses that write (too large?)".into()))?;
            prepared(&o.params, seq, hash)
        }

        /// HISTORY (`data::ROOT_LOG`): the table's tree root now (hex, or null), and the roots it had — `[[at, root]]`,
        /// oldest first (each with when it was replaced).
        pub fn tail_root(&mut self, id: &[u8]) -> Result<JsValue, JsValue> {
            Ok(match self.0.tail(&b32(id)?).map_err(err)?.root() {
                Some(r) => JsValue::from(hex(&r)),
                None => JsValue::NULL,
            })
        }
        pub fn tail_root_log(&mut self, id: &[u8]) -> Result<js_sys::Array, JsValue> {
            Ok(self
                .0
                .tail(&b32(id)?)
                .map_err(err)?
                .root_log()
                .into_iter()
                .map(|(at, r)| -> JsValue { [JsValue::from(at as f64), JsValue::from(hex(&r))].into_iter().collect::<js_sys::Array>().into() })
                .collect())
        }
        /// The step LOGGING `root` (just replaced by a flush, at `at` ms) after `prior` (`tail_root_log` read BEFORE that
        /// flush: a flush drops the log), keeping what is newer than `keep_after` and at most `max`: `{ params, seq,
        /// valueHash }` to sign, or null (nothing to log).
        pub fn tail_log_root(&mut self, id: &[u8], prior: js_sys::Array, root: &str, at: f64, keep_after: f64, max: u32) -> Result<JsValue, JsValue> {
            let cid = |s: &str| -> Option<[u8; 32]> { (0..32).map(|i| u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()).collect::<Option<Vec<u8>>>()?.try_into().ok() };
            let prior: Vec<(u64, [u8; 32])> = prior
                .iter()
                .filter_map(|e| {
                    let a = js_sys::Array::from(&e);
                    Some((a.get(0).as_f64()? as u64, cid(&a.get(1).as_string()?)?))
                })
                .collect();
            let root = cid(root).ok_or_else(|| err("not a root".into()))?;
            let o = self.0.tail(&b32(id)?).map_err(err)?;
            match o.prepare_root_log(&prior, root, at as u64, keep_after as u64, max as usize) {
                Some((seq, hash)) => Ok(prepared(&o.params, seq, hash)?.into()),
                None => Ok(JsValue::NULL),
            }
        }
        /// A row's value in the tree at an earlier `root` (hex; its history): `{ value }` (bytes, or null: no such row
        /// then), or `{ fetch }` — Block contract ids to GET first (their answers through `take`), then ask again.
        pub fn tail_value_at(&mut self, id: &[u8], root: &str, key: &[u8]) -> Result<JsValue, JsValue> {
            let cid: [u8; 32] = (0..32)
                .map(|i| u8::from_str_radix(root.get(2 * i..2 * i + 2).unwrap_or("zz"), 16).ok())
                .collect::<Option<Vec<u8>>>()
                .and_then(|v| v.try_into().ok())
                .ok_or_else(|| err("not a root".into()))?;
            let out = js_sys::Object::new();
            match self.0.read(&b32(id)?, |o| o.value_at(&cid, key)).map_err(err)? {
                Err(fetch) => {
                    let _ = js_sys::Reflect::set(&out, &"fetch".into(), &fetch.into_iter().map(JsValue::from).collect::<js_sys::Array>());
                }
                Ok(data::Step::Ready(v)) => {
                    let _ = js_sys::Reflect::set(&out, &"value".into(), &v.map(|b| JsValue::from(js_sys::Uint8Array::from(&b[..]))).unwrap_or(JsValue::NULL));
                }
                Ok(_) => {
                    let _ = js_sys::Reflect::set(&out, &"value".into(), &JsValue::NULL);
                }
            }
            Ok(out.into())
        }

        /// SKIP AHEAD (`data::Open::skip_to`): the prepared step moved past `last` (the identity signed through it; the
        /// network, which answered, holds less): the step to sign, or null.
        pub fn tail_skip(&mut self, id: &[u8], last: f64) -> Result<JsValue, JsValue> {
            let o = self.0.tail(&b32(id)?).map_err(err)?;
            match o.skip_to(last as u64) {
                Some((seq, hash)) => Ok(prepared(&o.params, seq, hash)?.into()),
                None => Ok(JsValue::NULL),
            }
        }

        /// The table's own key (from the identity delegate): rows and blocks under it read, its blocks' addresses come
        /// from it, and — until an epoch's key is given — writes are sealed with it.
        /// An open tail's rows AS STORED (bytes: a feed's are versions, for the `feed` package), `[[key, value]]`, once
        /// read to the end; `null` while its view still needs blocks or keys (settle it, then ask again).
        pub fn tail_raw(&mut self, id: &[u8]) -> Result<JsValue, JsValue> {
            match self.0.tail(&b32(id)?).map_err(err)?.opened().map_err(err)? {
                data::Step::Ready(rows) => Ok(rows
                    .into_iter()
                    .map(|(k, v)| -> JsValue {
                        [
                            JsValue::from(js_sys::Uint8Array::from(&k[..])),
                            JsValue::from(js_sys::Uint8Array::from(&v[..])),
                        ]
                        .into_iter()
                        .collect::<js_sys::Array>()
                        .into()
                    })
                    .collect::<js_sys::Array>()
                    .into()),
                _ => Ok(JsValue::NULL),
            }
        }

        /// The key an open tail is under (its writer's), and the sequence its next write will have.
        pub fn tail_next(&mut self, id: &[u8]) -> Result<f64, JsValue> {
            Ok((self.0.tail(&b32(id)?).map_err(err)?.writer.seq() + 1) as f64)
        }

        /// Make an open tail PUBLIC (a person's card): rows in the clear.
        pub fn tail_public(&mut self, id: &[u8]) -> Result<(), JsValue> {
            self.0.tail(&b32(id)?).map_err(err)?.public = true;
            Ok(())
        }

        pub fn tail_seal(&mut self, id: &[u8], key: &[u8]) -> Result<(), JsValue> {
            self.0
                .tail(&b32(id)?)
                .map_err(err)?
                .set_table_key(b32(key)?);
            Ok(())
        }

        /// An epoch's key for an open table (from the identity delegate; empty: it has none for that epoch). `current`:
        /// new rows and blocks are sealed with it.
        pub fn tail_epoch_key(
            &mut self,
            id: &[u8],
            epoch: f64,
            key: &[u8],
            current: bool,
        ) -> Result<(), JsValue> {
            let k = if key.is_empty() {
                None
            } else {
                Some(b32(key)?)
            };
            self.0
                .tail(&b32(id)?)
                .map_err(err)?
                .epoch_key(epoch as u64, k, current);
            Ok(())
        }

        /// An ALTERNATE key of an epoch for an open table (a group branch that lost the race for that epoch, `keys`'
        /// heal): read with, never written with — rows opened with it are sealed over to the epoch's own key.
        pub fn tail_epoch_key_also(
            &mut self,
            id: &[u8],
            epoch: f64,
            key: &[u8],
        ) -> Result<(), JsValue> {
            let k = b32(key)?;
            self.0
                .tail(&b32(id)?)
                .map_err(err)?
                .epoch_key_also(epoch as u64, k);
            Ok(())
        }

        /// SEAL OVER up to `n` of the table's plaintext rows from before sealing, as one step: `{ params, seq,
        /// valueHash }` to sign as for a write, or null when none are left.
        pub fn tail_migrate(&mut self, id: &[u8], n: u32) -> Result<JsValue, JsValue> {
            let o = self.0.tail(&b32(id)?).map_err(err)?;
            match o.migrate(n as usize) {
                Some((seq, hash)) => Ok(prepared(&o.params, seq, hash)?.into()),
                None => Ok(JsValue::NULL),
            }
        }

        /// The delegate's signature (hex) for the prepared write: `[kind, frames]`, kind "put" or "update".
        pub fn tail_commit(&mut self, id: &[u8], sig_hex: &str) -> Result<js_sys::Array, JsValue> {
            let id = b32(id)?;
            let sig: [u8; 64] = unhex(sig_hex)
                .and_then(|b| b.try_into().ok())
                .ok_or_else(|| err("not a signature".into()))?;
            // READ ONCE: a commit replaces the table's held blocks with what its write staged (taken BEFORE any block
            // that arrived since) — the blocks read meanwhile kept, never dropped to be fetched again.
            let open = self.0.tail(&id).map_err(err)?;
            let held = open.blocks.0.clone();
            let send = open
                .commit(sig)
                .ok_or_else(|| err("the signature does not cover that write".into()))?;
            for (cid, body) in held {
                open.blocks.0.entry(cid).or_insert(body);
            }
            let kind = if matches!(send, data::Send::Put(_)) {
                "put"
            } else {
                "update"
            };
            let f = self.0.frames_send(&id, send).map_err(err)?;
            Ok([JsValue::from(kind), JsValue::from(frames(f))]
                .into_iter()
                .collect())
        }

        /// An open table's view, as JSON text: `{ kind: "tail", tail: { id, seq, rows: [{ key, value }], root,
        /// pending } }`, or `{ kind: "tail-need", blocks: [Block contract ids to GET] }`.
        pub fn tail_view(&mut self, id: &[u8]) -> Result<String, JsValue> {
            Ok(self.0.tail_view(&b32(id)?).to_string())
        }
        /// A page of an open tail: `lo`/`hi`/`after` as bytes (empty: none), newest key first when `reverse`.
        pub fn tail_page(
            &mut self,
            id: &[u8],
            lo: &[u8],
            hi: &[u8],
            reverse: bool,
            after: &[u8],
            limit: u32,
        ) -> Result<String, JsValue> {
            let opt = |b: &[u8]| (!b.is_empty()).then(|| b.to_vec());
            Ok(self
                .0
                .tail_page(
                    &b32(id)?,
                    opt(lo),
                    opt(hi),
                    reverse,
                    opt(after),
                    limit as usize,
                )
                .to_string())
        }

        pub fn set_block_code(&mut self, code: &[u8]) {
            self.0.set_block_code(code);
        }

        pub fn set_sealed_code(&mut self, code: &[u8]) {
            self.0.set_sealed_code(code);
        }
        pub fn set_piece_code(&mut self, code: &[u8]) {
            self.0.set_piece_code(code);
        }

        /// A tree block's group: the Block contract ids (hex) to GET with it (the race).
        pub fn tail_group(&mut self, id: &[u8], block_hex: &str) -> Result<js_sys::Array, JsValue> {
            let b = bytes32("block", block_hex).map_err(err)?;
            Ok(self
                .0
                .tail_group(&b32(id)?, &b)
                .map_err(err)?
                .into_iter()
                .map(JsValue::from)
                .collect())
        }

        /// Settle a raced block: true once held (arrived, or rebuilt and verified from its group).
        pub fn tail_rebuild(&mut self, block_hex: &str) -> Result<bool, JsValue> {
            let b = bytes32("block", block_hex).map_err(err)?;
            self.0.tail_rebuild(&b).map_err(err)
        }

        /// How many rows wait in an open table's tail (a flush is due at `data::FLUSH_AT`).
        pub fn tail_pending(&mut self, id: &[u8]) -> Result<u32, JsValue> {
            Ok(self.0.tail(&b32(id)?).map_err(err)?.pending_rows() as u32)
        }

        pub fn flush_at() -> u32 {
            data::FLUSH_AT as u32
        }
        /// Whether a flush is due after a write (`data::flush_due`, the node's too).
        pub fn flush_due(pending: u32) -> bool {
            data::flush_due(pending as usize, 0)
        }
        /// A catalog row's PLACE (`data::catalog_place`, the node's too): "listed", "blinded" or "none".
        pub fn catalog_place(v: &str) -> String {
            match data::catalog_place(v.as_bytes()) {
                data::Place::Listed => "listed",
                data::Place::Blinded => "blinded",
                data::Place::None => "none",
            }
            .into()
        }
        /// A catalog row for a place ("listed", "blinded", "none"), noted `at` (ms).
        pub fn catalog_value(place: &str, at: f64) -> Result<String, JsValue> {
            let p = match place {
                "listed" => data::Place::Listed,
                "blinded" => data::Place::Blinded,
                "none" => data::Place::None,
                _ => return Err(err(format!("no place {place:?}"))),
            };
            Ok(String::from_utf8(data::catalog_value(p, Some(at as u64))).expect("JSON"))
        }

        /// FLUSH an open table. `{ need: [Block contract ids] }` to GET first; or `{ params, seq, valueHash, puts:
        /// [[name, frames]] }`: send every put and see each accepted, THEN sign and `tail_commit` as for a write.
        pub fn tail_flush(&mut self, id: &[u8]) -> Result<js_sys::Object, JsValue> {
            self.flush_out(id, None)
        }

        /// REPLACE an open table with exactly `rows` (`[[key bytes, value bytes]]`) — a REPAIR for a table no key held
        /// here opens (its tree sealed over by another branch of the group); answered as `tail_flush` is.
        pub fn tail_replace(&mut self, id: &[u8], rows: js_sys::Array) -> Result<js_sys::Object, JsValue> {
            let mut given = Vec::new();
            for r in rows.iter() {
                let pair: js_sys::Array = r.dyn_into()?;
                let k = js_sys::Uint8Array::new(&pair.get(0)).to_vec();
                let v = js_sys::Uint8Array::new(&pair.get(1)).to_vec();
                given.push((k, v));
            }
            self.flush_out(id, Some(given))
        }

        fn flush_out(&mut self, id: &[u8], given: Option<Vec<(Vec<u8>, Vec<u8>)>>) -> Result<js_sys::Object, JsValue> {
            let id = b32(id)?;
            let out = js_sys::Object::new();
            match (match given {
                Some(rows) => self.0.tail_replace(&id, rows),
                None => self.0.tail_flush(&id),
            })
            .map_err(err)?
            {
                FlushOut::Need(blocks) => {
                    let a: js_sys::Array = blocks.into_iter().map(JsValue::from).collect();
                    js_sys::Reflect::set(&out, &"need".into(), &a.into())?;
                }
                FlushOut::Keys(epochs) => {
                    let a: js_sys::Array = epochs
                        .into_iter()
                        .map(|e| JsValue::from(e as f64))
                        .collect();
                    js_sys::Reflect::set(&out, &"keys".into(), &a.into())?;
                }
                FlushOut::Ready { seq, hash, puts } => {
                    let params = self.0.tail(&id).map_err(err)?.params.clone();
                    js_sys::Reflect::set(
                        &out,
                        &"params".into(),
                        &js_sys::Uint8Array::from(&params[..]).into(),
                    )?;
                    js_sys::Reflect::set(&out, &"seq".into(), &JsValue::from(seq as f64))?;
                    js_sys::Reflect::set(
                        &out,
                        &"valueHash".into(),
                        &js_sys::Uint8Array::from(&hash[..]).into(),
                    )?;
                    let a: js_sys::Array = puts
                        .into_iter()
                        .map(|(name, f)| -> JsValue {
                            [JsValue::from(name), JsValue::from(frames(f))]
                                .into_iter()
                                .collect::<js_sys::Array>()
                                .into()
                        })
                        .collect();
                    js_sys::Reflect::set(&out, &"puts".into(), &a.into())?;
                }
            }
            Ok(out)
        }

        /// A table's ASSET: `{ need: [contract hex] }`, `{ keys: [epoch] }`, or `{ groups: [{ k, slots: [[block hex,
        /// contract hex]] }] }`. `history`: the blocks only its logged roots reach.
        pub fn tail_asset(&mut self, id: &[u8], history: bool) -> Result<js_sys::Object, JsValue> {
            let id = b32(id)?;
            let out = js_sys::Object::new();
            match self.0.tail_asset(&id, history).map_err(err)? {
                AssetOut::Need(c) => {
                    js_sys::Reflect::set(
                        &out,
                        &"need".into(),
                        &c.into_iter()
                            .map(JsValue::from)
                            .collect::<js_sys::Array>()
                            .into(),
                    )?;
                }
                AssetOut::Keys(e) => {
                    js_sys::Reflect::set(
                        &out,
                        &"keys".into(),
                        &e.into_iter()
                            .map(|x| JsValue::from(x as f64))
                            .collect::<js_sys::Array>()
                            .into(),
                    )?;
                }
                AssetOut::Ready(groups) => {
                    let a = js_sys::Array::new();
                    for (k, slots) in groups {
                        let g = js_sys::Object::new();
                        js_sys::Reflect::set(&g, &"k".into(), &JsValue::from(k as f64))?;
                        let sl: js_sys::Array = slots
                            .into_iter()
                            .map(|(b, c)| -> JsValue {
                                [JsValue::from(b), JsValue::from(c)]
                                    .into_iter()
                                    .collect::<js_sys::Array>()
                                    .into()
                            })
                            .collect();
                        js_sys::Reflect::set(&g, &"slots".into(), &sl.into())?;
                        a.push(&g);
                    }
                    js_sys::Reflect::set(&out, &"groups".into(), &a.into())?;
                }
            }
            Ok(out)
        }

        /// A block of the asset put again: `[name, frames]`, or null when it cannot be made here.
        pub fn tail_keep_put(&mut self, id: &[u8], block_hex: &str) -> Result<JsValue, JsValue> {
            let b = bytes32("block", block_hex).map_err(err)?;
            Ok(match self.0.tail_keep_put(&b32(id)?, &b).map_err(err)? {
                Some((name, f)) => [JsValue::from(name), JsValue::from(frames(f))]
                    .into_iter()
                    .collect::<js_sys::Array>()
                    .into(),
                None => JsValue::NULL,
            })
        }

        /// The tail's signed state put again.
        pub fn tail_keep_state(&mut self, id: &[u8]) -> Result<js_sys::Array, JsValue> {
            Ok(frames(self.0.tail_keep_state(&b32(id)?).map_err(err)?))
        }

        /// A contract id's base58 form, as the node names it in acks.
        pub fn id_name(id: &[u8]) -> Result<String, JsValue> {
            Ok(wire::contract_id(b32(id)?).encode())
        }

        /// The ed25519 public key of a seed.
        pub fn public_of(seed: &[u8]) -> Result<js_sys::Uint8Array, JsValue> {
            let k = ed25519_dalek::SigningKey::from_bytes(&b32(seed)?);
            Ok(js_sys::Uint8Array::from(&k.verifying_key().to_bytes()[..]))
        }

        /// A KEY a page holds itself (a space's mail key: its secret kept where only who may use it read it), the
        /// counterparts of the identity's: the public key to SEAL to (`seal_to`), opening what was sealed to it, and
        /// signing / checking with it (ed25519).
        pub fn box_public(seed: &[u8]) -> Result<js_sys::Uint8Array, JsValue> {
            let s = x25519_dalek::StaticSecret::from(b32(seed)?);
            Ok(js_sys::Uint8Array::from(&x25519_dalek::PublicKey::from(&s).to_bytes()[..]))
        }
        pub fn open_with(seed: &[u8], blob: &[u8]) -> Result<js_sys::Uint8Array, JsValue> {
            craftworks_identity::open_with(&b32(seed)?, blob)
                .map(|v| js_sys::Uint8Array::from(&v[..]))
                .ok_or_else(|| err("it does not open with that key".into()))
        }
        pub fn sign_with(seed: &[u8], message: &[u8]) -> Result<js_sys::Uint8Array, JsValue> {
            use ed25519_dalek::Signer;
            let k = ed25519_dalek::SigningKey::from_bytes(&b32(seed)?);
            Ok(js_sys::Uint8Array::from(&k.sign(message).to_bytes()[..]))
        }
        pub fn verify_with(public: &[u8], message: &[u8], sig: &[u8]) -> bool {
            let (Ok(p), Ok(s)) = (<[u8; 32]>::try_from(public), <[u8; 64]>::try_from(sig)) else { return false };
            ed25519_dalek::VerifyingKey::from_bytes(&p).is_ok_and(|k| k.verify_strict(message, &ed25519_dalek::Signature::from_bytes(&s)).is_ok())
        }

        /// What a frame from the node says, as JSON text.
        pub fn take(&mut self, bytes: &[u8]) -> String {
            self.0.take(bytes).to_string()
        }
    }

    /// WHO DOES A THING (`rules::admission::turn`, the one ranking — `ordering.turn` is this): `dids` JSON `[did…]`.
    #[wasm_bindgen]
    pub fn admission_turn(id: &str, dids: &str, me: &str) -> i32 {
        let dids: Vec<String> = serde_json::from_str(dids).unwrap_or_default();
        craftworks_rules::admission::turn(id, &dids, me) as i32
    }

    /// AN ORDERED LOG's rows (`rules::ordering`, the identity delegate's too): the rows of position `n` begin with this.
    #[wasm_bindgen]
    pub fn ordering_position_prefix(prefix: &str, n: f64) -> String {
        craftworks_rules::ordering::position_prefix(prefix, n as u64)
    }

    /// …and its ENTRIES among those rows' values (JSON `[value…]`), in the one order every reader takes them
    /// (`rules::ordering::order`): JSON `[value…]` — a reader takes the first valid for it.
    #[wasm_bindgen]
    pub fn ordering_entries(values: &str) -> String {
        let vs: Vec<String> = serde_json::from_str(values).unwrap_or_default();
        let ordered: Vec<&str> = craftworks_rules::ordering::order(vs.iter().map(|v| v.as_bytes())).into_iter().filter_map(|v| std::str::from_utf8(v).ok()).collect();
        serde_json::to_string(&ordered).unwrap_or_else(|_| "[]".into())
    }

    /// ON THE OWNER's BRANCH (`craftworks_rules::guards::on_branch`): `epoch` (none: not held), `owner` JSON
    /// `[[epoch, fingerprint]]` newest first, `fps` JSON `{ epoch: fingerprint }` (its own, as read) — true, false, or
    /// `undefined` (not known).
    #[wasm_bindgen]
    pub fn on_branch(epoch: Option<f64>, owner: &str, fps: &str) -> Option<bool> {
        let owner: Vec<(u64, String)> = serde_json::from_str::<Vec<(f64, String)>>(owner).unwrap_or_default().into_iter().map(|(e, f)| (e as u64, f)).collect();
        let fps: std::collections::HashMap<String, String> = serde_json::from_str(fps).unwrap_or_default();
        craftworks_rules::guards::on_branch(epoch.map(|e| e as u64), &owner, &|e| fps.get(&e.to_string()).cloned())
    }
    /// WHERE A DEVICE's KEYS OF A SPACE STAND (`craftworks_rules::guards::group_state`): the `group` lifecycle's event.
    #[wasm_bindgen]
    pub fn group_state(held: Option<f64>, owner: Option<f64>, on_branch: Option<bool>, unreadable: bool, why: bool) -> String {
        craftworks_rules::guards::group_state(held.map(|e| e as u64), owner.map(|e| e as u64), on_branch, unreadable, why).into()
    }
    /// Whether a device in that state enters by itself (`guards::cannot_read`).
    #[wasm_bindgen]
    pub fn group_cannot_read(state: &str) -> bool {
        craftworks_rules::guards::cannot_read(state)
    }
    /// The one TAKEOVER (ms): how long one in turn has before another takes over (`guards::TAKEOVER_MS`).
    #[wasm_bindgen]
    pub fn takeover_ms() -> f64 {
        craftworks_rules::guards::TAKEOVER_MS as f64
    }
    /// THE TURN's WAIT (`guards::turn_wait`): ranked `k`, since `since` — ms left before it acts.
    #[wasm_bindgen]
    pub fn turn_wait(k: f64, since: f64, now: f64) -> f64 {
        craftworks_rules::guards::turn_wait(k.max(0.0) as u64, since as u64, now as u64, craftworks_rules::guards::TAKEOVER_MS) as f64
    }
    /// WHETHER THIS ACCOUNT PUBLISHES an epoch's group info (`members::publishes_info` over `members::followers`, the
    /// node's too): the owner always; else two followers in turn. `listed`, `in_group`: JSON DID lists.
    #[wasm_bindgen]
    pub fn publishes_info(me: &str, owner: &str, listed: &str, in_group: &str, epoch: f64) -> Result<bool, JsValue> {
        let l: Vec<String> = serde_json::from_str(listed).map_err(|e| JsValue::from_str(&format!("listed: {e}")))?;
        let g: Vec<String> = serde_json::from_str(in_group).map_err(|e| JsValue::from_str(&format!("in_group: {e}")))?;
        Ok(craftworks_rules::members::publishes_info(me, owner, &craftworks_rules::members::followers(&l, &g), epoch as u64))
    }
    /// A SALT ROW (`rules::rekey::parse_salt`, the node's too): JSON `{ s: hex, n, removals }` or undefined.
    #[wasm_bindgen]
    pub fn file_salt_parse(v: &str) -> Option<String> {
        craftworks_rules::rekey::parse_salt(v).map(|(s, n, removals)| json!({ "s": hex(&s), "n": n, "removals": removals }).to_string())
    }
    /// A KEY ROW WRITTEN (`rules::rekey::key_row_write`, the node's too): `[key, value]` for its fields (JSON).
    #[wasm_bindgen]
    pub fn file_key_row_write(id: &str, fields: &str, at: f64) -> Result<Vec<String>, JsValue> {
        let f = serde_json::from_str(fields).map_err(|e| err(format!("fields: {e}")))?;
        let (k, v) = craftworks_rules::rekey::key_row_write(id, &f, at as u64);
        Ok(vec![k, v])
    }
    /// A FILE's REFERENCE (`rules::rekey::file_ref`, the node's too): JSON.
    #[wasm_bindgen]
    #[allow(clippy::too_many_arguments)]
    pub fn file_ref(id: &str, in_space: Option<String>, key: &[u8], root: &str, burn: &str, size: f64, name: &str, kind: &str) -> Result<String, JsValue> {
        let b = |h: &str, what: &str| craftworks_identity::unhex32(h).ok_or_else(|| err(format!("{what}: 32 bytes of hex")));
        Ok(craftworks_rules::rekey::file_ref(id, in_space.as_deref(), &b32(key)?, &b(root, "root")?, &b(burn, "burn")?, size as u64, name, kind).to_string())
    }
    /// A KEY ROW (`rules::rekey::parse_key_row`, the node's too): JSON of its fields with `id`, or undefined.
    #[wasm_bindgen]
    pub fn file_key_row(key: &str, value: &str) -> Option<String> {
        craftworks_rules::rekey::parse_key_row(key, value).map(|o| Value::Object(o).to_string())
    }
    /// THE FILE RE-KEY PLAN (`rules::rekey::plan`, the node's too). JSON in: `{ salt: {n, removals} | null, out: [did],
    /// members: [did], gone: [did], banned: [did], rows: [{id, n, pub, app, moved}], anyone: [app], me, now }`. JSON out:
    /// `{ rotate: removals | null, due: [{id, pub, rank, wait, moved}] }`.
    #[wasm_bindgen]
    pub fn rekey_plan(input: &str) -> Result<String, JsValue> {
        let v: Value = serde_json::from_str(input).map_err(|e| JsValue::from_str(&format!("rekey_plan: {e}")))?;
        let strs = |k: &str| -> Vec<String> { v[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default() };
        let salt = v["salt"].as_object().map(|o| (o.get("n").and_then(Value::as_i64).unwrap_or(0), o.get("removals").and_then(Value::as_u64).unwrap_or(0) as usize));
        let rows: Vec<craftworks_rules::rekey::Row> = v["rows"].as_array().map(|a| a.iter().filter_map(|r| Some(craftworks_rules::rekey::Row { id: r["id"].as_str()?.to_string(), n: r["n"].as_i64().unwrap_or(-2), public: r["pub"].as_bool().unwrap_or(false), app: craftworks_rules::rekey::subject(r["dom"].as_str(), r["app"].as_str()), moved: r["moved"].as_u64().unwrap_or(0) })).collect()).unwrap_or_default();
        let anyone = strs("anyone");
        let p = craftworks_rules::rekey::plan(salt, &strs("out"), &strs("members"), &strs("gone"), &strs("banned"), &rows, &|app| anyone.iter().any(|a| a == app), v["me"].as_str().unwrap_or(""), v["now"].as_u64().unwrap_or(0));
        Ok(json!({ "rotate": p.rotate, "due": p.due.iter().map(|d| json!({ "id": d.id, "pub": d.public, "rank": d.rank, "wait": d.wait, "moved": d.moved })).collect::<Vec<_>>() }).to_string())
    }
    /// AN ACCOUNT's BEEKEM MEMBER from its card (`account::beekem_member`, the node's too): its DID, its data key (hex),
    /// the card's `beekem` value → JSON `{ id, leaf }` (hex) or `{ why }`.
    #[wasm_bindgen]
    pub fn beekem_member(did: &[u8], data_key: &str, value: &str) -> String {
        let (Ok(did), Some(dk)) = (<[u8; 32]>::try_from(did), craftworks_identity::unhex32(data_key)) else { return json!({ "why": "its key log not read" }).to_string() };
        match craftworks_account::beekem_member(&did, &dk, value.as_bytes()) {
            Ok((id, leaf)) => json!({ "id": hex(&id), "leaf": hex(&leaf) }).to_string(),
            Err(why) => json!({ "why": why }).to_string(),
        }
    }
    /// A space's KEY OPERATIONS' rows (`members::keyop_*`, the node's too): an operation's row key (hex).
    #[wasm_bindgen]
    pub fn keyop_key(op: &[u8]) -> String {
        craftworks_rules::members::keyop_key(op)
    }
    /// Whether an operation goes in its row (else as a public file the row names).
    #[wasm_bindgen]
    pub fn keyop_inline(op: &[u8]) -> bool {
        craftworks_rules::members::keyop_inline(op)
    }
    /// A row's value: JSON `{ op: hex }` or `{ file: ref }`, or undefined (not a key operation's row).
    #[wasm_bindgen]
    pub fn keyop_row(value: &str) -> Option<String> {
        use craftworks_rules::members::KeyopRow;
        craftworks_rules::members::keyop_row(value).map(|r| match r {
            KeyopRow::Op(op) => json!({ "op": hex(&op) }),
            KeyopRow::File(f) => json!({ "file": f }),
        }.to_string())
    }
    /// A file's bytes taken as its row's operation: only when they hash to the row's key.
    #[wasm_bindgen]
    pub fn keyop_file_ok(key: &str, bytes: &[u8]) -> bool {
        craftworks_rules::members::keyop_file_ok(key, bytes)
    }
    /// WHETHER AN ACCOUNT's CARD IS READ NOW (`members::card_due`, the node's too): `at` < 0: never read; `ok`: it named
    /// a member.
    #[wasm_bindgen]
    pub fn card_due(at: f64, ok: bool, now: f64) -> bool {
        craftworks_rules::members::card_due((at >= 0.0).then_some((at as u64, ok)), now as u64)
    }
    /// AN EPOCH's DOOR ENTRIES (`members::door_entries`, the node's too): JSON items → JSON `[entry]`.
    #[wasm_bindgen]
    pub fn door_entries(items: &str, epoch: f64) -> String {
        let v: Vec<Value> = serde_json::from_str(items).unwrap_or_default();
        serde_json::to_string(&craftworks_rules::members::door_entries(&v, epoch as u64)).expect("a list encodes")
    }
    /// THE GROUP's STEPS (`rules::steps::ALL`, the node's names): JSON `[[name, quiet]…]` — the page's table built from it.
    #[wasm_bindgen]
    pub fn group_steps() -> String {
        serde_json::to_string(craftworks_rules::steps::ALL).expect("a list encodes")
    }
    /// AN ENTRY's OUTCOME (`guards::entry_outcome`, the node's too): "apply", "skip" or "defer".
    #[wasm_bindgen]
    pub fn guard_entry_outcome(taken: bool, whole: bool) -> String {
        format!("{:?}", craftworks_rules::guards::entry_outcome(taken, whole)).to_lowercase()
    }
    /// THE LIST READ WHOLE (`guards::list_whole`, the node's too): `devices` JSON `["<did>: why"…]` (listed members
    /// whose devices did not read), `feeds` JSON `[[table, why]…]`, `missing` acts named and not held. `undefined`: whole.
    #[wasm_bindgen]
    pub fn guard_list_whole(devices: &str, feeds: &str, missing: f64) -> Result<Option<String>, JsValue> {
        let d: Vec<String> = serde_json::from_str(devices).map_err(|e| JsValue::from_str(&format!("devices: {e}")))?;
        let f: Vec<(String, String)> = serde_json::from_str(feeds).map_err(|e| JsValue::from_str(&format!("feeds: {e}")))?;
        Ok(craftworks_rules::guards::list_whole(&d, &f, missing as usize).err())
    }
    /// THE GUARD "a lapse's turn" (`guards::lapse_turn`): why not now, or `undefined`.
    #[wasm_bindgen]
    pub fn guard_lapse_turn(k: f64, since: f64, now: f64) -> Option<String> {
        craftworks_rules::guards::lapse_turn(k as i64, since as u64, now as u64, craftworks_rules::guards::TAKEOVER_MS).err()
    }
    /// AN MLS MEMBER's CREDENTIAL read (`identity::read_credential`, its signature checked): JSON `{ did, owner, node }`
    /// (the account, its owner key, the DEVICE's node key — hex), or `undefined`.
    #[wasm_bindgen]
    pub fn credential_parts(cred: &[u8]) -> Option<String> {
        // (`member`: the signing key it names — an account's MLS key, the same key its BeeKEM member id is.)
        craftworks_identity::read_credential(cred).map(|(did, owner, node, member)| serde_json::json!({ "did": account::did(&did), "owner": hex(&owner), "node": hex(&node), "member": hex(&member) }).to_string())
    }
    /// Whether an act changes who may hold a group's keys (`gov::moves_entries`): "in", "out", or undefined.
    #[wasm_bindgen]
    pub fn moves_entries(act: &str) -> Option<String> {
        craftworks_gov::moves_entries(act).map(|i| if i { "in" } else { "out" }.to_string())
    }
    /// WHOSE CARDS the keys' check needs (`gov::check_dids`, the node's too): JSON roster, the owner, JSON acts.
    #[wasm_bindgen]
    pub fn check_dids(roster: &str, owner: &str, acts: &str) -> Result<String, JsValue> {
        let r: Vec<String> = serde_json::from_str(roster).map_err(|e| JsValue::from_str(&format!("roster: {e}")))?;
        let a: Vec<Value> = serde_json::from_str(acts).map_err(|e| JsValue::from_str(&format!("acts: {e}")))?;
        Ok(serde_json::to_string(&craftworks_gov::check_dids(&r, owner, &a)).expect("a list encodes"))
    }
    /// WHOM a counted act lets in or puts out (`gov::entry_move`, the node's too): `[in?, did]`, or undefined.
    #[wasm_bindgen]
    pub fn entry_move(act: &str) -> Option<String> {
        let a: serde_json::Value = serde_json::from_str(act).ok()?;
        craftworks_gov::entry_move(&a).map(|(i, d)| serde_json::json!([i, d]).to_string())
    }
    /// AN INVITE CODE's PROOF for one DID (`gov::join_proof`): what a self-entry names in the clear.
    #[wasm_bindgen]
    pub fn join_proof(code: &str, did: &str) -> String {
        craftworks_gov::join_proof(code, did)
    }
    /// WHOSE an MLS member's credential is (`identity::read_credential`): its DID, or `undefined`.
    #[wasm_bindgen]
    pub fn credential_did(cred: &[u8]) -> Option<String> {
        craftworks_identity::read_credential(cred).map(|(did, ..)| account::did(&did))
    }
    /// THE GUARD "level reached" (`craftworks_rules::guards::level`): why not, or `undefined`.
    #[wasm_bindgen]
    pub fn guard_level(contrib: &str, need: &str, now: f64) -> Option<String> {
        craftworks_rules::guards::level(Some(contrib), need, now as u64).err()
    }
    /// THE GUARD "leads past" (`craftworks_rules::guards::leads_past`): a group its own node moved, at epoch `to`, this
    /// device holding `held` — why not taken, or `undefined`.
    #[wasm_bindgen]
    pub fn guard_leads_past(to: f64, held: Option<f64>) -> Option<String> {
        craftworks_rules::guards::leads_past(to as u64, held.map(|h| h as u64)).err()
    }
    /// A REQUEST BAG's NAME (`craftworks_rules::admission::bag_name`, the node's): `code` a code, or "open".
    #[wasm_bindgen]
    pub fn bag_name(code: &str, space: &[u8]) -> Result<String, JsValue> {
        Ok(craftworks_rules::admission::bag_name(code, &b32(space)?))
    }
    /// A LEVEL GROUP's level from its join policy (`craftworks_gov::level_of`: `level:pro` → `pro`), or `undefined`.
    #[wasm_bindgen]
    pub fn level_of_policy(policy: &str) -> Option<String> {
        craftworks_gov::level_of(policy).map(str::to_string)
    }

    /// `did:craftec:<base58>` of the owner's public key (the DID the identity delegate answers, as bytes).
    #[wasm_bindgen]
    pub fn did_of(owner: &[u8]) -> Result<String, JsValue> {
        Ok(account::did(&b32(owner)?))
    }

    /// A DID's 32 bytes from its text (`did:craftec:<base58>`, or the base58 alone).
    #[wasm_bindgen]
    pub fn did_bytes(text: &str) -> Result<js_sys::Uint8Array, JsValue> {
        Ok(js_sys::Uint8Array::from(
            &account::did_bytes(text).ok_or_else(|| err("not a DID".into()))?[..],
        ))
    }

    /// The names of an account space's own tables — the identity's constants (the delegate's rules name them), given to
    /// the `space` capability: `{ catalog, members, channel }`.
    #[wasm_bindgen]
    pub fn account_tables() -> String {
        serde_json::json!({ "catalog": craftworks_identity::CATALOG, "members": craftworks_identity::MEMBERS, "channel": craftworks_identity::CHANNEL }).to_string()
    }

    /// A table's BLINDED NAME (`identity::blind_name`): what its tail's label carries, from the table's key.
    #[wasm_bindgen]
    pub fn blind_name(table_key: &[u8], table: &str) -> Result<String, JsValue> {
        let k: [u8; 32] = table_key
            .try_into()
            .map_err(|_| err("a table key is 32 bytes".into()))?;
        Ok(craftworks_identity::blind_name(&k, table))
    }

    /// A table's key in an EPOCH, from that epoch's secret (`identity::epoch_table_key`, as the identity derives it) —
    /// for an epoch secret the identity no longer gives (a group branch that lost, kept by `keys`).
    #[wasm_bindgen]
    pub fn epoch_table_key(epoch_secret: &[u8], table: &str) -> Result<String, JsValue> {
        let s: [u8; 32] = epoch_secret
            .try_into()
            .map_err(|_| err("an epoch's secret is 32 bytes".into()))?;
        Ok(hex(&craftworks_identity::epoch_table_key(&s, table)))
    }

    /// A space's SEALED BAG items (`identity::bag_*`, the node's too). An OPEN bag (its writers list, an epoch's door)?
    #[wasm_bindgen]
    pub fn bag_is_open(name: &str) -> bool {
        craftworks_identity::bag_is_open(name)
    }
    /// The epoch an open bag's items are sealed at (`identity::BAG_OPEN`).
    #[wasm_bindgen]
    pub fn bag_open_epoch() -> u32 {
        craftworks_identity::BAG_OPEN
    }
    /// An open bag's key (hex): the space's own.
    #[wasm_bindgen]
    pub fn bag_open_key(space: &[u8]) -> Result<String, JsValue> {
        let s: [u8; 32] = space.try_into().map_err(|_| err("a space's id is 32 bytes".into()))?;
        Ok(hex(&craftworks_identity::bag_open_key(&s)))
    }
    /// An item SEALED: `plain` under `key` (hex), marked `epoch`; `nonce` 12 random bytes.
    #[wasm_bindgen]
    pub fn bag_seal(key: &str, epoch: u32, plain: &[u8], nonce: &[u8]) -> Result<Vec<u8>, JsValue> {
        let k = craftworks_identity::unhex32(key).ok_or_else(|| err("a bag key is 32 bytes (hex)".into()))?;
        let n: [u8; 12] = nonce.try_into().map_err(|_| err("a nonce is 12 bytes".into()))?;
        Ok(craftworks_identity::bag_seal(&k, epoch, plain, n))
    }
    /// The epoch an item says it was sealed at; undefined: not an item.
    #[wasm_bindgen]
    pub fn bag_item_epoch(item: &[u8]) -> Option<u32> {
        craftworks_identity::bag_item_epoch(item)
    }
    /// An item OPENED with `key` (hex); undefined: not this key's.
    #[wasm_bindgen]
    pub fn bag_open(key: &str, item: &[u8]) -> Option<Vec<u8>> {
        craftworks_identity::bag_open(&craftworks_identity::unhex32(key)?, item)
    }

    /// The address key (hex) of a space's table (`identity::space_table_key`).
    #[wasm_bindgen]
    pub fn space_table_key(space: &[u8], table: &str) -> Result<String, JsValue> {
        let s: [u8; 32] = space
            .try_into()
            .map_err(|_| err("a space's id is 32 bytes".into()))?;
        Ok(hex(&craftworks_identity::space_table_key(&s, table)))
    }

    /// The public key (hex) of an EPOCH's log: the tail of that epoch's MLS commits (`identity::epoch_log_key`, the one
    /// derivation the identity delegate signs with).
    #[wasm_bindgen]
    pub fn epoch_log_public(secret: &[u8]) -> Result<String, JsValue> {
        let s: [u8; 32] = secret
            .try_into()
            .map_err(|_| err("an epoch secret is 32 bytes".into()))?;
        Ok(hex(&craftworks_identity::epoch_log_key(&s)
            .verifying_key()
            .to_bytes()))
    }

    /// The node's client-API URL on this machine (`wire::ws_url`: loopback only, native encoding), at the host the page
    /// was served from (`localhost` and `127.0.0.1` are different origins to the node).
    #[wasm_bindgen]
    pub fn ws_url(host: &str, port: u16) -> Result<String, JsValue> {
        wire::ws_url(host, port).map_err(err)
    }

    /// FILES (ARCHITECTURE §6: `craftworks_files`): a file's pieces made and read here; the page moves them.
    fn plan_js(p: &craftworks_files::Plan) -> String {
        serde_json::json!({ "size": p.size, "chunk": p.chunk, "chunks": p.chunks, "gens": p.gens })
            .to_string()
    }
    fn listed_of(json: &str) -> Result<craftworks_files::Listed, JsValue> {
        let v: Vec<(u8, String)> = serde_json::from_str(json).map_err(|e| err(e.to_string()))?;
        v.into_iter()
            .map(|(j, h)| {
                Ok((
                    j,
                    unhex(&h)
                        .and_then(|b| b.try_into().ok())
                        .ok_or_else(|| err("a hash is 32 bytes".into()))?,
                ))
            })
            .collect()
    }
    fn file_err(e: craftworks_files::Error) -> JsValue {
        err(format!("{e:?}"))
    }
    /// `[j, id hex, frames]` per piece.
    fn pieces_js(
        core: &mut CraftworksCore,
        pieces: Vec<(u8, craftworks_files::Piece)>,
        burn: [u8; 32],
    ) -> Result<js_sys::Array, JsValue> {
        let out = js_sys::Array::new();
        for (j, p) in pieces {
            let h = hex(&p.hash());
            let (id, f) = core.0.frames_piece(&p, &burn).map_err(err)?;
            out.push(
                &[
                    JsValue::from(j),
                    JsValue::from(id),
                    JsValue::from(frames(f)),
                    JsValue::from(h),
                ]
                .into_iter()
                .collect::<js_sys::Array>(),
            );
        }
        Ok(out)
    }

    /// The BLAKE3 of a file's content, fed a slice at a time (a large file is never whole in memory).
    #[wasm_bindgen]
    pub struct FileHasher(blake3::Hasher);
    #[wasm_bindgen]
    impl FileHasher {
        #[wasm_bindgen(constructor)]
        pub fn new() -> FileHasher {
            FileHasher(blake3::Hasher::new())
        }
        pub fn update(&mut self, b: &[u8]) {
            self.0.update(b);
        }
        pub fn finish(&self) -> js_sys::Uint8Array {
            js_sys::Uint8Array::from(&self.0.finalize().as_bytes()[..])
        }
    }

    /// A file's generation being rebuilt (`craftworks_files::Decoder`).
    #[wasm_bindgen]
    pub struct FileDecoder(craftworks_files::Decoder);
    #[wasm_bindgen]
    impl FileDecoder {
        #[wasm_bindgen(constructor)]
        pub fn new(key: &[u8], size: f64, g: f64, listed: &str) -> Result<FileDecoder, JsValue> {
            let plan = craftworks_files::Plan::of(size as u64);
            Ok(FileDecoder(craftworks_files::Decoder::new(
                &b32(key)?,
                &plan,
                g as u64,
                listed_of(listed)?,
            )))
        }
        /// "ok", "redundant", or an error (forged, malformed).
        pub fn add(&mut self, j: u8, state: &[u8]) -> Result<String, JsValue> {
            match self.0.add(j, state) {
                Ok(()) => Ok("ok".into()),
                Err(craftworks_files::Error::Redundant) => Ok("redundant".into()),
                Err(e) => Err(file_err(e)),
            }
        }
        pub fn done(&self) -> bool {
            self.0.done()
        }
        pub fn rank(&self) -> u32 {
            self.0.rank() as u32
        }
        /// The listed fragments not back yet: what the next round asks (`files::ASK_ROUNDS` rounds, the node's too).
        pub fn missing(&self) -> Vec<u8> {
            self.0.missing()
        }
        pub fn k(&self) -> u32 {
            self.0.k() as u32
        }
        /// Fragment `j` came back empty (not there).
        pub fn none(&mut self, j: u8) {
            self.0.none(j)
        }
        /// The fragments not there: what a reader makes again.
        pub fn not_there(&self) -> Vec<u8> {
            self.0.not_there()
        }
        pub fn rounds() -> u32 {
            craftworks_files::ASK_ROUNDS as u32
        }
        pub fn plain(&self) -> Result<js_sys::Uint8Array, JsValue> {
            Ok(js_sys::Uint8Array::from(
                &self.0.plain().map_err(file_err)?[..],
            ))
        }
    }

    /// EDITIONS (`editions`): a step's answer for the page — `{ want: [hex] }`, `{ sign: bytes }` (have the identity
    /// delegate `TokenSign` it, then call again with it in `sigs`), or `{ done: … }`.
    fn token_step<T>(s: Result<tokens::Step<T>, String>, done: impl FnOnce(T) -> Result<serde_json::Value, JsValue>) -> Result<JsValue, JsValue> {
        let v = match s.map_err(err)? {
            tokens::Step::Want(w) => json!({ "want": w.iter().map(|x| hex(x)).collect::<Vec<_>>() }),
            tokens::Step::Sign(m) => return Ok(js_obj(&[("sign", js_sys::Uint8Array::from(&m[..]).into())])),
            tokens::Step::Done(x) => json!({ "done": done(x)? }),
        };
        js_sys::JSON::parse(&v.to_string())
    }
    fn js_obj(kv: &[(&str, JsValue)]) -> JsValue {
        let o = js_sys::Object::new();
        for (k, v) in kv {
            let _ = js_sys::Reflect::set(&o, &(*k).into(), v);
        }
        o.into()
    }
    /// Registry files as a page passes them: `[{ key: address key hex, size }]`.
    fn key_sizes(files: &JsValue) -> Result<Vec<([u8; 32], u64)>, JsValue> {
        js_sys::Array::from(files)
            .iter()
            .map(|f| Ok((bytes32("file", &js_sys::Reflect::get(&f, &"key".into())?.as_string().unwrap_or_default()).map_err(err)?, js_sys::Reflect::get(&f, &"size".into())?.as_f64().unwrap_or(0.0) as u64)))
            .collect()
    }
    fn key_sizes_js(files: &[([u8; 32], u64)]) -> JsValue {
        let out = js_sys::Array::new();
        for (ak, size) in files {
            out.push(&js_obj(&[("key", JsValue::from(hex(ak))), ("size", JsValue::from(*size as f64))]));
        }
        out.into()
    }
    /// The keeper's context: the standing contract's code, the beacon's code and id.
    fn keeper_ctx(standing_code: &[u8], beacon_code: &[u8], beacon: &[u8]) -> Result<keeper::Keep, JsValue> {
        Ok(keeper::Keep { standing: *blake3::hash(standing_code).as_bytes(), beacon_code: *blake3::hash(beacon_code).as_bytes(), beacon: b32(beacon)? })
    }
    /// `[[id hex, size]]` (the node's hosted contracts, as the node-self answer gives them).
    /// A WITNESS VERDICT as the page reads it: `{ want }`, or `{ done: { seated: false } | { refused } | { sign } }`.
    fn verdict_js(step: tokens::Step<witness::Verdict>) -> Result<JsValue, JsValue> {
        match step {
            tokens::Step::Want(w) => js_sys::JSON::parse(&json!({ "want": w.iter().map(|x| hex(x)).collect::<Vec<_>>() }).to_string()),
            tokens::Step::Sign(_) => Err(err("a witness step never signs here".into())),
            tokens::Step::Done(witness::Verdict::NotSeated) => js_sys::JSON::parse(&json!({ "done": { "seated": false } }).to_string()),
            tokens::Step::Done(witness::Verdict::Later(why)) => js_sys::JSON::parse(&json!({ "done": { "later": why } }).to_string()),
            tokens::Step::Done(witness::Verdict::Refused(why)) => js_sys::JSON::parse(&json!({ "done": { "refused": why } }).to_string()),
            tokens::Step::Done(witness::Verdict::Sign { at, entry, owner }) => {
                let sign = js_obj(&[("chain", js_sys::Uint8Array::from(&at.chain[..]).into()), ("seq", JsValue::from(at.seq as f64)), ("entry", js_sys::Uint8Array::from(&entry[..]).into()), ("params", js_sys::Uint8Array::from(&at.encode()[..]).into()), ("owner", js_sys::Uint8Array::from(&owner[..]).into())]);
                Ok(js_obj(&[("done", js_obj(&[("sign", sign)]))]))
            }
        }
    }
    fn hosted_of(v: &JsValue) -> Result<Vec<([u8; 32], u64)>, JsValue> {
        js_sys::Array::from(v).iter().map(|x| {
            let x = js_sys::Array::from(&x);
            Ok((bytes32("hosted", &x.get(0).as_string().unwrap_or_default()).map_err(err)?, x.get(1).as_f64().unwrap_or(0.0) as u64))
        }).collect()
    }
    /// `[[message, sig]]` the delegate gave.
    fn sigs_of(a: &js_sys::Array) -> Result<Vec<(Vec<u8>, [u8; 64])>, JsValue> {
        a.iter().map(|p| {
            let p = js_sys::Array::from(&p);
            Ok((js_sys::Uint8Array::new(&p.get(0)).to_vec(), js_sys::Uint8Array::new(&p.get(1)).to_vec().try_into().map_err(|_| err("a signature is 64 bytes".into()))?))
        }).collect()
    }
    /// The context: the token's code, the beacon's id, the key log's code.
    fn token_ctx(token_code: &[u8], beacon: &[u8], idlog_code: &[u8]) -> Result<tokens::Ctx, JsValue> {
        Ok(tokens::Ctx { code_hash: *blake3::hash(token_code).as_bytes(), beacon: b32(beacon)?, idlog: *blake3::hash(idlog_code).as_bytes() })
    }
    /// The page's DEVICE (token entries, vouches): its node key, the account's credentials (its group's: this node's is picked, its key seq
    /// from the key log `did`'s, as read), and the signatures so far (`[[message, sig]]`). `None`: the key log not read.
    fn account_device(core: &Core, idlog_code: &[u8], did: &[u8; 32], public: &[u8], creds: &js_sys::Array, sigs: &js_sys::Array) -> Result<Option<tokens::Device>, JsValue> {
        let id = account::idlog_put(idlog_code, did, None).id_bytes;
        let Some(st) = core.got(&id).filter(|b| !b.is_empty()) else { return Ok(None) };
        let log = craftworks_idlog_contract::read(did, st).ok_or_else(|| err("the account's key log does not verify".into()))?;
        let sigs = sigs
            .iter()
            .map(|p| {
                let p = js_sys::Array::from(&p);
                let m = js_sys::Uint8Array::new(&p.get(0)).to_vec();
                let s: [u8; 64] = js_sys::Uint8Array::new(&p.get(1)).to_vec().try_into().map_err(|_| err("a signature is 64 bytes".into()))?;
                Ok((m, s))
            })
            .collect::<Result<Vec<_>, JsValue>>()?;
        editions::device(b32(public)?, &bytes_list(creds), &log, sigs).map(Some).map_err(|e| err(e.into()))
    }
    /// `{ want: [the key log] }` — asked before a device can be made.
    fn want_log(idlog_code: &[u8], did: &[u8; 32]) -> Result<JsValue, JsValue> {
        js_sys::JSON::parse(&json!({ "want": [hex(&account::idlog_put(idlog_code, did, None).id_bytes)] }).to_string())
    }
    /// Owners (32 bytes each) from a JS array: this wallet's one-time owners.
    fn owners_of(a: &js_sys::Array) -> Result<Vec<[u8; 32]>, JsValue> {
        a.iter().map(|o| b32(&js_sys::Uint8Array::new(&o).to_vec())).collect()
    }
    fn edition_json(e: &editions::Edition, mine: &[[u8; 32]]) -> serde_json::Value {
        let d = &e.definition;
        json!({
            "id": hex(&e.id),
            "name": String::from_utf8_lossy(&d.name),
            "cap": d.cap.to_string(),
            "numbered": d.ids == craftworks_token_contract::Ids::Numbered,
            "item": hex(&d.item),
            "creator": hex(&d.genesis_owner),
            "royalty": d.royalty.map(|(b, to)| json!({ "bps": b, "to": hex(&to) })),
            "sale": d.sale.map(|(t, price, payee)| json!({ "token": hex(&t.definition), "price": price.to_string(), "payee": hex(&payee) })),
            "minted": e.minted.to_string(),
            "open": e.open,
            "mine": editions::copies_of(e, mine).iter().map(|n| n.to_string()).collect::<Vec<_>>(),
        })
    }

    #[wasm_bindgen]
    impl CraftworksCore {
        /// Mark a contract READ AND NOT THERE (its GET failed: no such contract).
        pub fn not_there(&mut self, id: &[u8]) -> Result<(), JsValue> {
            self.0.not_there(b32(id)?);
            Ok(())
        }
        /// The BEACON's contract id (hex) from its code and params (TOKEN §5a: one per network).
        pub fn beacon_id(beacon_code: &[u8], params: &[u8]) -> String {
            let (_, contract, _) = wire::puts::contract(beacon_code, params, &[]);
            hex(contract.key().id().as_bytes())
        }
        /// The PUTS of a beacon state a page extended (`beacon-step`): the beacon itself, and the OUTPUTS LOG's record of
        /// its final heights (the span of the newest final height, and the one before it at a span's start) — as the
        /// beacon's extender writes them. `[{ id, frames }]`; empty when the state does not read.
        pub fn beacon_puts(&mut self, beacon_code: &[u8], params: &[u8], state: &[u8]) -> Result<js_sys::Array, JsValue> {
            use craftworks_beacon_contract as B;
            let list = js_sys::Array::new();
            let (Some(p), Some(st)) = (B::Params::decode(params), B::wire::decode(state)) else { return Ok(list) };
            let mut puts = vec![(params.to_vec(), state.to_vec())];
            let (_, contract, _) = wire::puts::contract(beacon_code, params, &[]);
            let id: [u8; 32] = contract.key().id().as_bytes().try_into().map_err(|_| err("a contract id".into()))?;
            let fin = st.height().saturating_sub(p.window as u64);
            for idx in [fin.saturating_sub(1) / B::log::SPAN, fin / B::log::SPAN] {
                let lp = B::log::Params { beacon: id, index: idx };
                let rec = B::log::record(&lp, &st);
                if !rec.entries.is_empty() && (idx == fin / B::log::SPAN || fin % B::log::SPAN < 2) && !puts.iter().any(|(x, _)| *x == lp.encode()) {
                    puts.push((lp.encode(), rec.encode()));
                }
            }
            for (params, state) in puts {
                let (pid, contract, state) = wire::puts::contract(beacon_code, &params, &state);
                let id_bytes: [u8; 32] = contract.key().id().as_bytes().try_into().map_err(|_| err("a contract id".into()))?;
                list.push(&self.put_obj(&account::Put { id: pid, id_bytes, contract, state })?);
            }
            Ok(list)
        }
        /// The reputation's STARTING ROOTS (DID texts).
        pub fn reputation_roots() -> js_sys::Array {
            craftworks_reputation::ROOTS.iter().map(|d| JsValue::from(account::did(d))).collect()
        }
        /// A VOUCH's message to sign (`TokenSign`, the vouch domain): `from` vouches for (`on`) or withdraws from `to`, at
        /// beacon height `at`.
        pub fn vouch_message(from: &[u8], to: &[u8], at: f64, on: bool) -> Result<js_sys::Uint8Array, JsValue> {
            Ok(js_sys::Uint8Array::from(&craftworks_reputation::item::Item::message(&b32(from)?, &b32(to)?, at as u64, on)[..]))
        }
        /// The SIGNED VOUCH item (`reputation::item`): this node's credential and key seq picked from the account's
        /// credentials (the key log read: `{ want }` otherwise), the device's signature `sig` of `vouch_message`.
        /// `{ item: bytes }`.
        #[allow(clippy::too_many_arguments)]
        pub fn vouch_item(&self, idlog_code: &[u8], from: &[u8], to: &[u8], at: f64, on: bool, public: &[u8], creds: js_sys::Array, sig: &[u8]) -> Result<JsValue, JsValue> {
            let from = b32(from)?;
            let Some(dev) = account_device(&self.0, idlog_code, &from, public, &creds, &js_sys::Array::new())? else { return want_log(idlog_code, &from) };
            let sig: [u8; 64] = sig.try_into().map_err(|_| err("a signature is 64 bytes".into()))?;
            let item = craftworks_reputation::item::Item { from, to: b32(to)?, at: at as u64, on, cred: dev.credential, key_seq: dev.key_seq, sig };
            Ok(js_obj(&[("item", js_sys::Uint8Array::from(&item.encode()[..]).into())]))
        }
        /// This device's KEEPERS-REGISTRY item (`reputation::keepers`): the account, and the credential naming its member
        /// key `public` (its standing record's key) — `{ done: item }`, or `{ want }` (the account's key log) first.
        pub fn keeper_listing(&self, idlog_code: &[u8], did: &[u8], public: &[u8], creds: js_sys::Array) -> Result<JsValue, JsValue> {
            let did = b32(did)?;
            let Some(dev) = account_device(&self.0, idlog_code, &did, public, &creds, &js_sys::Array::new())? else { return want_log(idlog_code, &did) };
            let k = craftworks_reputation::keepers::Keeper { did, cred: dev.credential, key_seq: dev.key_seq };
            Ok(js_obj(&[("done", js_sys::Uint8Array::from(&k.encode()[..]).into())]))
        }
        /// Keeper `me`'s CANDIDATES (`craftworks_standing::assign::candidates`): of the registry's live files (`files`:
        /// `[{ key: address key hex, size }]`) and the listed keepers' record keys (`keepers`: hex), those it ranks among
        /// the copies of, highest first — `[{ key, size }]`. What it keeps of them: `keeper_fill`.
        pub fn keeper_candidates(me: &[u8], files: JsValue, keepers: js_sys::Array) -> Result<JsValue, JsValue> {
            let files = key_sizes(&files)?;
            let keepers: Vec<[u8; 32]> = keepers.iter().map(|k| bytes32("keeper", &k.as_string().unwrap_or_default()).map_err(err)).collect::<Result<_, _>>()?;
            Ok(key_sizes_js(&craftworks_standing::assign::candidates(&b32(me)?, &files, &keepers, craftworks_standing::assign::COPIES)))
        }
        /// The files a keeper KEEPS of its ranked `candidates` (`[{ key, size }]`) within `limit` bytes
        /// (`craftworks_standing::assign::fill`), by what its own reads found (`reads`: `[{ key, found }]`, one per file
        /// read; a file not in it is not read yet): found first, then unread, never absent — `[{ key, size }]`.
        pub fn keeper_fill(candidates: JsValue, limit: f64, reads: js_sys::Array) -> Result<JsValue, JsValue> {
            use craftworks_standing::assign::Read;
            let candidates = key_sizes(&candidates)?;
            let mut read = std::collections::BTreeMap::new();
            for r in reads.iter() {
                let key = bytes32("file", &js_sys::Reflect::get(&r, &"key".into())?.as_string().unwrap_or_default()).map_err(err)?;
                read.insert(key, if js_sys::Reflect::get(&r, &"found".into())?.as_bool() == Some(true) { Read::Found } else { Read::Absent });
            }
            Ok(key_sizes_js(&craftworks_standing::assign::fill(&candidates, limit as u64, &|ak| read.get(ak).copied().unwrap_or(Read::Unread))))
        }
        /// The KEEPERS REGISTRY's bag `k` of standing epoch `epoch` (its address).
        pub fn keepers_address(epoch: f64, k: u32) -> js_sys::Uint8Array {
            js_sys::Uint8Array::from(&craftworks_reputation::keepers::address(epoch as u64, k)[..])
        }
        /// The KEEPERS listed in a registry's items, verified against each account's key log (read into the core:
        /// `{ want: [hex] }` first): `{ done: [{ did, key }] }` (DID text, record key hex).
        pub fn keepers_listed(&self, idlog_code: &[u8], items: js_sys::Array) -> Result<JsValue, JsValue> {
            use craftworks_reputation::keepers as K;
            let items = bytes_list(&items);
            let mut want: Vec<String> = Vec::new();
            for k in items.iter().filter_map(|b| K::Keeper::decode(b)) {
                let id = account::idlog_put(idlog_code, &k.did, None).id_bytes;
                if self.0.got(&id).is_none() && !want.contains(&hex(&id)) {
                    want.push(hex(&id));
                }
            }
            if !want.is_empty() {
                return js_sys::JSON::parse(&json!({ "want": want }).to_string());
            }
            let log_of = |d: &[u8; 32]| {
                let id = account::idlog_put(idlog_code, d, None).id_bytes;
                self.0.got(&id).filter(|b| !b.is_empty()).and_then(|b| craftworks_idlog_contract::read(d, b))
            };
            let out: Vec<serde_json::Value> = K::listed(&items, &log_of).iter().map(|(d, key)| json!({ "did": account::did(d), "key": hex(key) })).collect();
            js_sys::JSON::parse(&json!({ "done": out }).to_string())
        }
        /// The VOUCH REGISTRY's bag address for standing epoch `epoch`.
        pub fn vouch_registry_address(epoch: f64, k: u32) -> js_sys::Uint8Array {
            js_sys::Uint8Array::from(&craftworks_reputation::item::registry_address(epoch as u64, k)[..])
        }
        /// The MISDEED REGISTRY's bag address for standing epoch `epoch` (`rules::misdeed`, TOKEN §7d decision 3): where
        /// anyone posts a self-proving misdeed proof (chained: `index.bag.chained`).
        pub fn misdeed_registry_address(epoch: f64, k: u32) -> js_sys::Uint8Array {
            js_sys::Uint8Array::from(&craftworks_rules::misdeed::registry_address(epoch as u64, k)[..])
        }
        /// An EQUIVOCATION proof's bytes, to post (`rules::misdeed::Equivocation`): account `did` (32 bytes), the credential
        /// naming its device `key` (its owner key at `key_seq`), and the two witness signatures by `key` at one place, each
        /// `{ chain, seq, entry, at, sig }` (hex; seq and at numbers). It is checked by every statement build, not here.
        pub fn misdeed_equivocation(did: &[u8], cred: &[u8], key_seq: u32, key: &[u8], a: JsValue, b: JsValue) -> Result<js_sys::Uint8Array, JsValue> {
            let sig = |v: &JsValue| -> Result<craftworks_token_contract::witness::WitnessSig, JsValue> {
                let f = |k: &str| js_sys::Reflect::get(v, &k.into()).ok();
                let h = |k: &str| f(k).and_then(|x| x.as_string()).ok_or_else(|| err(format!("a signature's {k}")));
                let n = |k: &str| f(k).and_then(|x| x.as_f64()).map(|x| x as u64).ok_or_else(|| err(format!("a signature's {k}")));
                let text = h("sig")?;
                let sig: [u8; 64] = (0..64).map(|i| text.get(2 * i..2 * i + 2).and_then(|x| u8::from_str_radix(x, 16).ok())).collect::<Option<Vec<u8>>>().and_then(|v| v.try_into().ok()).ok_or_else(|| err("a signature's sig: 64 bytes of hex".into()))?;
                Ok(craftworks_token_contract::witness::WitnessSig { chain: bytes32("chain", &h("chain")?).map_err(err)?, seq: n("seq")?, entry: bytes32("entry", &h("entry")?).map_err(err)?, at: n("at")?, sig })
            };
            let e = craftworks_rules::misdeed::Equivocation { did: b32(did)?, cred: cred.to_vec(), key_seq, key: b32(key)?, a: sig(&a)?, b: sig(&b)? };
            Ok(js_sys::Uint8Array::from(&e.encode()[..]))
        }
        /// A signed vouch item's `{ from, to, at, on }` (DID texts), or null.
        pub fn vouch_item_read(item: &[u8]) -> JsValue {
            match craftworks_reputation::item::Item::decode(item) {
                Some(i) => js_obj(&[("from", account::did(&i.from).into()), ("to", account::did(&i.to).into()), ("at", JsValue::from(i.at as f64)), ("on", JsValue::from(i.on))]),
                None => JsValue::NULL,
            }
        }
        /// REPUTATION AS OF height `now` from SIGNED vouch items (`reputation::item::standing` — each verified against its
        /// voucher's key log, read into the core: `{ want: [hex] }` first — then `reputation_at` over the roots):
        /// `{ rep: { did: per-mille }, promoted: [did], admitted: n }`.
        pub fn reputation_from_items(&self, idlog_code: &[u8], items: js_sys::Array, now: f64) -> Result<JsValue, JsValue> {
            use craftworks_reputation as R;
            let items: Vec<R::item::Item> = bytes_list(&items).iter().filter_map(|b| R::item::Item::decode(b)).collect();
            let mut want: Vec<String> = Vec::new();
            for i in &items {
                let id = account::idlog_put(idlog_code, &i.from, None).id_bytes;
                if self.0.got(&id).is_none() && !want.contains(&hex(&id)) {
                    want.push(hex(&id));
                }
            }
            if !want.is_empty() {
                return js_sys::JSON::parse(&json!({ "want": want }).to_string());
            }
            let log_of = |d: &R::Did| {
                let id = account::idlog_put(idlog_code, d, None).id_bytes;
                self.0.got(&id).filter(|b| !b.is_empty()).and_then(|b| craftworks_idlog_contract::read(d, b))
            };
            let vs = R::item::standing(&items, now as u64, &log_of);
            let roots = R::ROOTS.to_vec();
            let rep = R::reputation_at(&roots, &vs, now as u64);
            let c = R::counted(&vs);
            let t = R::tree(&roots, &c);
            let promoted = R::promoted(&roots, &c, &t, &rep);
            let text = |d: &R::Did| account::did(d);
            js_sys::JSON::parse(&json!({
                "rep": rep.iter().map(|(d, r)| (text(d), json!((r * 1000 / R::ONE) as u64))).collect::<serde_json::Map<String, Value>>(),
                "promoted": promoted.iter().map(text).collect::<Vec<_>>(),
                "admitted": rep.len(),
            }).to_string())
        }
        /// The EPOCH STATEMENT of standing epoch `epoch` (`reputation::statement`): reputation as of the epoch's START from
        /// the signed vouch items given (each verified by its voucher's key log: `{ want }` first), no pots yet (keepers
        /// and the allowance join when their tokens run). `{ root, posted: bytes, puts: [[name, frames]], lines: n }` —
        /// the tree's blocks to put through the blocks door, the posted root for the epoch's statement bag.
        /// `keepers`: the KEEPERS REGISTRY's items of the epoch before (`reputation::keepers`) — its KEPT accounts (each
        /// listed keeper whose standing record proves keeping then and passes its audits: `standing::record::kept_audited`; the
        /// standing contract's code and the beacon name the records). `fresh`: the contracts (hex) this build read from
        /// the network — a keeper's record not among them is asked for again, whatever this core holds (a record the
        /// page wrote or read earlier is a stale copy: its own, held since its last answer, left it out of KEPT).
        /// `misdeeds`: the MISDEED REGISTRY's items of every epoch before this one, `[[epoch, item]]` (`rules::misdeed`).
        #[allow(clippy::too_many_arguments)]
        pub fn statement_build(&mut self, idlog_code: &[u8], items: js_sys::Array, epoch: f64, keepers: js_sys::Array, standing_code: &[u8], beacon_code: &[u8], beacon: &[u8], token_code: &[u8], bag_code: &[u8], queued: js_sys::Array, posted: js_sys::Array, fresh: js_sys::Array, earlier: js_sys::Array, misdeeds: js_sys::Array) -> Result<JsValue, JsValue> {
            use craftworks_reputation as R;
            let epoch = epoch as u64;
            let items: Vec<R::item::Item> = bytes_list(&items).iter().filter_map(|b| R::item::Item::decode(b)).collect();
            let misdeeds: Vec<(u64, Vec<u8>)> = misdeeds.iter().filter_map(|x| {
                let x = js_sys::Array::from(&x);
                Some((x.get(0).as_f64()? as u64, js_sys::Uint8Array::new(&x.get(1)).to_vec()))
            }).collect();
            let keepers = bytes_list(&keepers);
            let queued: Vec<craftworks_wallet::queue::Queued> = bytes_list(&queued).iter().filter_map(|b| craftworks_wallet::queue::Queued::decode(b)).collect();
            let posted: Vec<[u8; 32]> = posted.iter().filter_map(|r| bytes32("root", &r.as_string().unwrap_or_default()).ok()).collect();
            let earlier: Vec<(u64, [u8; 32])> = earlier.iter().filter_map(|x| {
                let x = js_sys::Array::from(&x);
                Some((x.get(0).as_f64()? as u64, bytes32("root", &x.get(1).as_string()?).ok()?))
            }).collect();
            let fresh: Vec<String> = fresh.iter().filter_map(|x| x.as_string()).collect();
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let lot = tokens::Lot { pool: keeper::craft(&ctx).1, bag: *blake3::hash(bag_code).as_bytes(), beacon_code: *blake3::hash(beacon_code).as_bytes(), idlog: ctx.idlog };
            let k = keeper_ctx(standing_code, beacon_code, beacon)?;
            // THE BUILD (`rules::statement`: the delegate's too).
            let built = {
                let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
                let is_fresh = |id: &[u8; 32]| fresh.contains(&hex(id));
                let lines = |root: &[u8; 32]| self.0.statement_lines(root);
                let inputs = craftworks_rules::statement::Inputs { epoch, items: &items, misdeeds: &misdeeds, keepers: &keepers, queued: &queued, posted: &posted, earlier: &earlier, ctx: &ctx, lot: &lot, keep: &k, fresh: &is_fresh, lines: &lines, audit: None };
                craftworks_rules::statement::build(&inputs, &got).map_err(err)?
            };
            let (st, kept) = match built {
                tokens::Step::Want(w) => return js_sys::JSON::parse(&json!({ "want": w.iter().map(|x| hex(x)).collect::<Vec<_>>() }).to_string()),
                tokens::Step::Sign(_) => return Err(err("a statement never signs".into())),
                tokens::Step::Done(x) => x,
            };
            let pots = R::statement::Pots { keeper: craftworks_wallet::craft::keeper_pot(epoch), allowance: (craftworks_allowance_contract::POT, craftworks_allowance_contract::CAP), witness: craftworks_wallet::craft::witness_pot(epoch), halvings: craftworks_wallet::craft::halvings(epoch) };
            let mut blocks = std::collections::BTreeMap::new();
            let (root, parity) = st.tree(&mut blocks).ok_or_else(|| err("the statement's tree does not build".into()))?;
            let puts = js_sys::Array::new();
            for (cid, body) in &blocks {
                let (name, f) = self.0.public_block_put(cid, body).map_err(err)?;
                puts.push(&[JsValue::from(name), JsValue::from(frames(f))].into_iter().collect::<js_sys::Array>());
            }
            let posted = R::statement::Posted { epoch, root, parity }.encode();
            // What the epoch's ALLOWANCE pays: each line owed, with its proof under the root (`allowance_verdict`).
            let owed = js_sys::Array::new();
            for (d, _) in st.lines.iter().filter(|(_, l)| l.allowance > 0) {
                let proof = freenet_prolly::proof::prove(&blocks, &root, d).map_err(|_| err("a statement line that does not prove".into()))?.encode();
                owed.push(&[JsValue::from(js_sys::Uint8Array::from(&d[..])), js_sys::Uint8Array::from(&proof[..]).into()].into_iter().collect::<js_sys::Array>());
            }
            // What its KEEPER POT pays: each keeper line, with its proof (`keeper_mint`).
            let minted = js_sys::Array::new();
            for (d, _) in st.lines.iter().filter(|(_, l)| l.keeper > 0 || l.witness > 0) {
                let proof = freenet_prolly::proof::prove(&blocks, &root, d).map_err(|_| err("a statement line that does not prove".into()))?.encode();
                minted.push(&[JsValue::from(js_sys::Uint8Array::from(&d[..])), js_sys::Uint8Array::from(&proof[..]).into()].into_iter().collect::<js_sys::Array>());
            }
            // THE ISSUANCE it makes (the monitor, TOKEN §7d MONITORED): keepers paid, the total, the largest share.
            let pay: Vec<u128> = st.lines.values().map(|l| l.keeper).filter(|k| *k > 0).collect();
            let total: u128 = pay.iter().sum();
            let issuance = json!({ "keepers": pay.len(), "total": total.to_string(), "top": pay.iter().max().copied().unwrap_or(0).to_string(), "pot": pots.keeper.0.to_string() });
            // Each PAID line (what an account earned: its keeper share, its witness fees).
            let paid: serde_json::Value = st.lines.iter().filter(|(_, l)| l.keeper > 0 || l.witness > 0).map(|(d, l)| (hex(d), json!({ "keeper": l.keeper.to_string(), "witness": l.witness.to_string() }))).collect::<serde_json::Map<_, _>>().into();
            Ok(js_obj(&[("root", JsValue::from(hex(&root))), ("posted", js_sys::Uint8Array::from(&posted[..]).into()), ("puts", puts.into()), ("lines", JsValue::from(st.lines.len() as f64)), ("kept", JsValue::from(kept as f64)), ("owed", owed.into()), ("keepers", minted.into()), ("issuance", js_sys::JSON::parse(&issuance.to_string())?), ("paid", js_sys::JSON::parse(&paid.to_string())?)]))
        }
        /// The ALLOWANCE's verdict of `epoch` (`allowance::verdict`; `owed`: `[[did, proof]]` from `statement_build`, under
        /// `root` hex): `{ done: null }` (nothing escrowed to it) or `{ done: { puts: [{ id, frames }] } }` — the program's
        /// state (if this page decided it) and the treasury's release.
        #[allow(clippy::too_many_arguments)]
        pub fn allowance_verdict(&mut self, token_code: &[u8], allowance_code: &[u8], beacon: &[u8], idlog_code: &[u8], pool: &[u8], bag_code: &[u8], beacon_code: &[u8], token: &[u8], epoch: f64, root: &str, owed: js_sys::Array, now: f64) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let lot = tokens::Lot { pool: b32(pool)?, bag: *blake3::hash(bag_code).as_bytes(), beacon_code: *blake3::hash(beacon_code).as_bytes(), idlog: ctx.idlog };
            let ach = *blake3::hash(allowance_code).as_bytes();
            let root = bytes32("root", root).map_err(err)?;
            let lines: Vec<([u8; 32], Vec<u8>)> = owed.iter().map(|x| {
                let x = js_sys::Array::from(&x);
                Ok((b32(&js_sys::Uint8Array::new(&x.get(0)).to_vec())?, js_sys::Uint8Array::new(&x.get(1)).to_vec()))
            }).collect::<Result<_, JsValue>>()?;
            let step = {
                let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
                allowance::verdict(&ctx, &lot, &got, &ach, &b32(token)?, epoch as u64, &root, &lines, now as u64)
            };
            let mut built = None;
            let v = token_step(step, |b| {
                built = b;
                Ok(json!({}))
            })?;
            let Some(b) = built else { return Ok(v) };
            let list = js_sys::Array::new();
            if let Some((params, state)) = b.program {
                let (pid, contract, state) = wire::puts::contract(allowance_code, &params, &state);
                let id_bytes: [u8; 32] = contract.key().id().as_bytes().try_into().map_err(|_| err("a contract id".into()))?;
                list.push(&self.put_obj(&account::Put { id: pid, id_bytes, contract, state })?);
            }
            if let Some(p) = b.release {
                list.push(&self.chain_put_obj(token_code, &p)?);
            }
            js_sys::Reflect::set(&js_sys::Reflect::get(&v, &"done".into())?, &"puts".into(), &list)?;
            Ok(v)
        }
        /// `did`'s ALLOWANCE PAYOUT of `epoch` (`allowance::payout`): `{ done: null }` or `{ done: { amount, puts } }`.
        #[allow(clippy::too_many_arguments)]
        pub fn allowance_payout(&mut self, token_code: &[u8], allowance_code: &[u8], beacon: &[u8], idlog_code: &[u8], pool: &[u8], bag_code: &[u8], beacon_code: &[u8], token: &[u8], epoch: f64, did: &[u8], now: f64) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let lot = tokens::Lot { pool: b32(pool)?, bag: *blake3::hash(bag_code).as_bytes(), beacon_code: *blake3::hash(beacon_code).as_bytes(), idlog: ctx.idlog };
            let ach = *blake3::hash(allowance_code).as_bytes();
            let step = {
                let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
                allowance::payout(&ctx, &lot, &got, &ach, &b32(token)?, epoch as u64, &b32(did)?, now as u64)
            };
            let mut puts = Vec::new();
            let mut amount = None;
            let v = token_step(step, |r| {
                if let Some((p, a)) = r {
                    puts.push(p);
                    amount = Some(a);
                }
                Ok(json!({}))
            })?;
            let Some(a) = amount else { return Ok(v) };
            let v = self.attach_puts(v, token_code, &puts)?;
            js_sys::Reflect::set(&js_sys::Reflect::get(&v, &"done".into())?, &"amount".into(), &JsValue::from(a.to_string()))?;
            Ok(v)
        }
        /// MINT `did`'s keeper line of epoch `epoch`'s statement (`root` hex, `proof`) as $CRAFT stake (`keeper::mint`:
        /// worked, anyone's; only under the ACCEPTED root): `{ want }`, `{ sign }` (never), or `{ done: null | { puts,
        /// amount } }` — $CRAFT's contracts first when the network has not got them, then the chain (its witness queue).
        #[allow(clippy::too_many_arguments)]
        pub fn keeper_mint(&mut self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], pool: &[u8], bag_code: &[u8], beacon_code: &[u8], did: &[u8], epoch: f64, root: &str, proof: &[u8], now: f64) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let lot = tokens::Lot { pool: b32(pool)?, bag: *blake3::hash(bag_code).as_bytes(), beacon_code: *blake3::hash(beacon_code).as_bytes(), idlog: ctx.idlog };
            let root = bytes32("root", root).map_err(err)?;
            let step = {
                let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
                keeper::mint(&ctx, &lot, &got, &b32(did)?, epoch as u64, &root, proof, now as u64)
            };
            let mut minted = None;
            let v = token_step(step, |r| {
                let some = r.is_some();
                minted = r;
                Ok(if some { json!({}) } else { serde_json::Value::Null })
            })?;
            let Some(m) = minted else { return Ok(v) };
            let puts: Vec<tokens::Put> = m.craft.into_iter().chain([m.chain]).collect();
            let v = self.attach_puts(v, token_code, &puts)?;
            let done = js_sys::Reflect::get(&v, &"done".into())?;
            js_sys::Reflect::set(&done, &"amount".into(), &JsValue::from(m.amount.to_string()))?;
            js_sys::Reflect::set(&done, &"fee".into(), &JsValue::from(m.fee.to_string()))?;
            Ok(v)
        }
        /// CLAIM this account's $CRAFT stake for the next open claims epoch (`keeper::claim`: this node's member key
        /// `public` made its chain's writer first, then the claim signed — `{ sign }` steps): `{ want }`, `{ sign }`, or
        /// `{ done: null | { puts, epoch, amount } }` (the chain, then the claims registry).
        #[allow(clippy::too_many_arguments)]
        pub fn keeper_claim(&mut self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], did: &[u8], now: f64, public: &[u8], creds: js_sys::Array, sigs: js_sys::Array) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let did = b32(did)?;
            let Some(dev) = account_device(&self.0, idlog_code, &did, public, &creds, &sigs)? else { return want_log(idlog_code, &did) };
            let step = {
                let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
                keeper::claim(&ctx, &got, &did, now as u64, &dev)
            };
            let mut claimed = None;
            let v = token_step(step, |r| {
                let some = r.is_some();
                claimed = r;
                Ok(if some { json!({}) } else { serde_json::Value::Null })
            })?;
            let Some(c) = claimed else { return Ok(v) };
            let puts: Vec<tokens::Put> = vec![c.chain];
            let v = self.attach_puts(v, token_code, &puts)?;
            let done = js_sys::Reflect::get(&v, &"done".into())?;
            js_sys::Reflect::set(&done, &"epoch".into(), &JsValue::from(c.epoch as f64))?;
            js_sys::Reflect::set(&done, &"amount".into(), &JsValue::from(c.amount.to_string()))?;
            Ok(v)
        }
        /// THE PAGE'S WITNESS on one queued entry (`witness::entry`; `queued`: its bytes; `me`: this node's member key,
        /// its claim's; `standing_code`: the standing contract's code — a stake claim's writer's record is read for its
        /// liveness): `{ want }`, or `{ done: { seated: false } | { refused: why } | { sign: { chain, seq, entry, owner } } }`
        /// (bytes — what `WitnessSign` signs; `owner`: the staker whose seat it is, named in the container).
        #[allow(clippy::too_many_arguments)]
        pub fn witness_entry(&self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], pool: &[u8], bag_code: &[u8], beacon_code: &[u8], queued: &[u8], me: &[u8], now: f64, standing_code: &[u8]) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let lot = tokens::Lot { pool: b32(pool)?, bag: *blake3::hash(bag_code).as_bytes(), beacon_code: *blake3::hash(beacon_code).as_bytes(), idlog: ctx.idlog };
            let q = craftworks_wallet::queue::Queued::decode(queued).ok_or_else(|| err("not a queued entry".into()))?;
            let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
            verdict_js(witness::entry(&ctx, &lot, &got, &q, &b32(me)?, now as u64, blake3::hash(standing_code).as_bytes()).map_err(err)?)
        }
        /// An EPOCH STATEMENT's verdict for witness key `me` (`witness::statement`): the root THIS page computed for
        /// `epoch` (hex), signed at the statement's place when seated — as `witness_entry` answers.
        #[allow(clippy::too_many_arguments)]
        pub fn witness_statement(&self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], pool: &[u8], bag_code: &[u8], beacon_code: &[u8], epoch: f64, root: &str, me: &[u8]) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let lot = tokens::Lot { pool: b32(pool)?, bag: *blake3::hash(bag_code).as_bytes(), beacon_code: *blake3::hash(beacon_code).as_bytes(), idlog: ctx.idlog };
            let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
            verdict_js(witness::statement(&ctx, &lot, &got, epoch as u64, &bytes32("root", root).map_err(err)?, &b32(me)?))
        }
        /// The params of epoch `epoch`'s statement's SIGNATURES CONTAINER (`witness::statement_at`): its poster makes it
        /// empty (`witness_put` with no key) beside the root, so a witness with no page open adds its signature to it.
        #[allow(clippy::too_many_arguments)]
        pub fn statement_witness_params(token_code: &[u8], beacon: &[u8], idlog_code: &[u8], pool: &[u8], bag_code: &[u8], beacon_code: &[u8], epoch: f64) -> Result<js_sys::Uint8Array, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let lot = tokens::Lot { pool: b32(pool)?, bag: *blake3::hash(bag_code).as_bytes(), beacon_code: *blake3::hash(beacon_code).as_bytes(), idlog: ctx.idlog };
            Ok(js_sys::Uint8Array::from(&witness::statement_at(&ctx, &lot, epoch as u64).encode()[..]))
        }
        /// A PUT of a place's WITNESS SIGNATURES container (`params`: its `witness::Params`, as a verdict's `sign.params`
        /// or a chain put's `witnesses` give them): `{ id, idBytes, frames }` — with witness key `key`'s signature `sig`
        /// over `entry` signed at beacon height `at` (the delegate's, `frames_witness_sign`), for the staker `owner` (the
        /// verdict's `sign.owner`: whose claim lets the key sign there), or, `key` empty, the container made empty (its
        /// chain's writer makes it: a witness with no page open only adds to one already there).
        #[allow(clippy::too_many_arguments)]
        pub fn witness_put(&mut self, token_code: &[u8], params: &[u8], key: &[u8], owner: &[u8], entry: &[u8], sig: &[u8], at: f64) -> Result<JsValue, JsValue> {
            let wp = craftworks_token_contract::witness::Params::decode(params).ok_or_else(|| err("not a witness container's params".into()))?;
            let state = if key.is_empty() {
                wp.state(&[])
            } else {
                let s = craftworks_token_contract::witness::WitnessSig { chain: wp.chain, seq: wp.seq, entry: b32(entry)?, at: at as u64, sig: sig.try_into().map_err(|_| err("a signature is 64 bytes".into()))? };
                wp.state(&[(b32(key)?, b32(owner)?, s)])
            };
            let (id, contract, state) = wire::puts::contract(token_code, params, &state);
            let id_bytes: [u8; 32] = contract.key().id().as_bytes().try_into().map_err(|_| err("a contract id".into()))?;
            self.put_obj(&account::Put { id, id_bytes, contract, state })
        }
        // ---- THE EXPLORER (owner 10-11: "more transparent … like a block explorer"): what each public record holds ----

        /// A node's STANDING RECORD contract id (its member key `key`: `keeper::record_id`).
        pub fn keeper_record_id(standing_code: &[u8], beacon_code: &[u8], beacon: &[u8], key: &[u8]) -> Result<js_sys::Uint8Array, JsValue> {
            Ok(js_sys::Uint8Array::from(&keeper::record_id(&keeper_ctx(standing_code, beacon_code, beacon)?, &b32(key)?)[..]))
        }
        /// A standing record, per epoch: `[{ epoch, pieces, bytes, answers, last }]` (pieces and bytes committed; answered
        /// heights; the last answered — null: none), newest first.
        pub fn standing_read(state: &[u8]) -> JsValue {
            let out = js_sys::Array::new();
            let Some(rec) = craftworks_standing::codec::decode(state) else { return out.into() };
            let mut eps = rec.epochs.clone();
            eps.sort_by(|a, b| b.epoch.cmp(&a.epoch));
            for e in eps {
                let heights: std::collections::BTreeSet<u64> = e.answers.iter().map(|a| a.height).collect();
                let (pieces, bytes) = e.holdings.iter().fold((0u64, 0u64), |(p, b), h| (p + h.count as u64, b + h.bytes));
                out.push(&js_obj(&[("epoch", JsValue::from(e.epoch as f64)), ("pieces", JsValue::from(pieces as f64)), ("bytes", JsValue::from(bytes as f64)), ("answers", JsValue::from(heights.len() as f64)), ("last", heights.iter().last().map_or(JsValue::NULL, |h| JsValue::from(*h as f64)))]));
            }
            out.into()
        }
        /// The clock's lengths: `{ standing, claims, lead }` (heights a standing epoch, a claims epoch, and how long
        /// before its epoch a claims registry stops taking claims).
        pub fn clock() -> JsValue {
            js_obj(&[("standing", JsValue::from(craftworks_reputation::EPOCH as f64)), ("claims", JsValue::from(craftworks_token_contract::supply::EPOCH as f64)), ("lead", JsValue::from(craftworks_token_contract::pointers::LEAD as f64))])
        }
        /// A witness-queue item: `{ token, owner, seq }` (hex), or null.
        pub fn queued_read(b: &[u8]) -> JsValue {
            match craftworks_wallet::queue::Queued::decode(b) {
                Some(q) => js_obj(&[("token", JsValue::from(hex(&q.token.definition))), ("owner", JsValue::from(hex(&q.owner))), ("seq", JsValue::from(q.seq as f64))]),
                None => JsValue::NULL,
            }
        }
        /// THE STAKERS of claims epoch `epoch` — its VALID STAKERS, `V(E)` (`tokens::valid_of`: the one rule; stake pool
        /// $CRAFT): `{ want }` (hex), then `{ done: [{ owner, amount, key }] }` (hex; amount in units).
        #[allow(clippy::too_many_arguments)]
        pub fn stakers(&self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], bag_code: &[u8], beacon_code: &[u8], epoch: f64) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let lot = tokens::Lot { pool: keeper::craft(&ctx).1, bag: *blake3::hash(bag_code).as_bytes(), beacon_code: *blake3::hash(beacon_code).as_bytes(), idlog: ctx.idlog };
            let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
            token_step(Ok(tokens::valid_of(&ctx, &lot, &got, epoch as u64)), |v| Ok(json!(v.iter().map(|c| json!({ "owner": hex(&c.owner), "amount": c.amount.to_string(), "key": hex(&c.key) })).collect::<Vec<_>>())))
        }
        /// POINT to this account's claim for claims epoch `epoch` once accepted (`keeper::point`, dated `now`): `{ want }`,
        /// then `{ done: null }` (nothing to point to: not claimed, not accepted yet, pointed to already, or the epoch
        /// frozen) or `{ done: { put } }` (the pointer bag's put, `{ id, idBytes, frames }`: a token contract).
        #[allow(clippy::too_many_arguments)]
        pub fn keeper_point(&mut self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], bag_code: &[u8], beacon_code: &[u8], did: &[u8], epoch: f64, now: f64) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let lot = tokens::Lot { pool: keeper::craft(&ctx).1, bag: *blake3::hash(bag_code).as_bytes(), beacon_code: *blake3::hash(beacon_code).as_bytes(), idlog: ctx.idlog };
            let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
            let mut point = None;
            let step = keeper::point(&ctx, &lot, &got, &b32(did)?, epoch as u64, now as u64);
            let v = token_step(step, |p| {
                let some = p.is_some();
                point = p;
                Ok(if some { json!({}) } else { serde_json::Value::Null })
            })?;
            let Some(p) = point else { return Ok(v) };
            let done = js_sys::Reflect::get(&v, &"done".into())?;
            let (id, contract, state) = wire::puts::contract(token_code, &p.params, &p.state);
            let id_bytes: [u8; 32] = contract.key().id().as_bytes().try_into().map_err(|_| err("a contract id".into()))?;
            js_sys::Reflect::set(&done, &"put".into(), &self.put_obj(&account::Put { id, id_bytes, contract, state })?)?;
            Ok(v)
        }
        /// The PUT of bag `k` of the $CRAFT STAKE POINTERS of claims epoch `epoch` made EMPTY (`pointers::Params`): a page
        /// makes the coming epochs' first bags, so the delegate — which cannot make a contract — can add to them.
        pub fn stake_pointers_put(&mut self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], epoch: f64, k: u32) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let p = craftworks_token_contract::pointers::Params { code_hash: ctx.code_hash, pool: keeper::craft(&ctx).1, beacon: ctx.beacon, epoch: epoch as u64, k };
            let (id, contract, state) = wire::puts::contract(token_code, &p.encode(), &p.state(&[]));
            let id_bytes: [u8; 32] = contract.key().id().as_bytes().try_into().map_err(|_| err("a contract id".into()))?;
            self.put_obj(&account::Put { id, id_bytes, contract, state })
        }
        /// The claim window (`keeper::claim_epoch`): the claims epoch a claim made at height `h` is for, or null.
        pub fn claim_epoch(h: f64) -> JsValue {
            keeper::claim_epoch(h as u64).map_or(JsValue::NULL, |e| JsValue::from(e as f64))
        }
        /// A queued entry's STATUS (`queued`: its bytes): `{ want }`, or `{ done: { kind, stamp, signatures, accepted } |
        /// { kind, stamp, signatures, waiting: why } | { missing: why } }`.
        #[allow(clippy::too_many_arguments)]
        pub fn entry_status(&self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], pool: &[u8], bag_code: &[u8], beacon_code: &[u8], queued: &[u8]) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let lot = tokens::Lot { pool: b32(pool)?, bag: *blake3::hash(bag_code).as_bytes(), beacon_code: *blake3::hash(beacon_code).as_bytes(), idlog: ctx.idlog };
            let q = craftworks_wallet::queue::Queued::decode(queued).ok_or_else(|| err("not a queued entry".into()))?;
            let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
            let want = |w: Vec<[u8; 32]>| js_sys::JSON::parse(&json!({ "want": w.iter().map(|x| hex(x)).collect::<Vec<_>>() }).to_string());
            let missing = |why: &str| js_sys::JSON::parse(&json!({ "done": { "missing": why } }).to_string());
            let d = match tokens::there(&got, &q.token.definition) {
                Err(w) => return want(vec![w]),
                Ok(None) => return missing("its token is not on the network"),
                Ok(Some(b)) => craftworks_token_contract::Definition::decode(&b).ok_or_else(|| err("not a token definition".into()))?,
            };
            let p = craftworks_token_contract::ChainParams { code_hash: ctx.code_hash, definition: q.token.definition, owner: q.owner, beacon: d.beacon, idlog: d.idlog, item: q.token.item };
            let s = match tokens::there(&got, &p.id()) {
                Ok(s) => s,
                Err(w) => return want(vec![w]),
            };
            let Some(s) = s.and_then(|b| craftworks_token_contract::State::decode(&b)) else { return missing("its chain is not on the network") };
            let Some(i) = s.entries.iter().position(|e| e.seq == q.seq) else { return missing("its chain holds no such entry") };
            let e = &s.entries[i];
            // Its SIGNATURES: the container of its place under its lot's claims epoch.
            let (lot_epoch, _) = craftworks_token_contract::wallet::lot_of(e, i.checked_sub(1).map(|j| &s.entries[j]), d.margin).map_err(|w| err(w.into()))?;
            let wp = tokens::witness_at(ctx.code_hash, &lot, d.beacon, &p.id(), q.seq, lot_epoch);
            let signatures = match tokens::there(&got, &wp.id()) {
                Ok(b) => b.map_or(0, |b| wp.sigs(&b).len()),
                Err(w) => return want(vec![w]),
            };
            let kind = format!("{:?}", e.body).split(|c: char| c == ' ' || c == '{' || c == '(').next().unwrap_or("").to_string();
            let base = json!({ "kind": kind, "stamp": e.stamp, "signatures": signatures, "chain": hex(&p.id()) });
            match tokens::accepted(&lot, &got, &p, &d, &s, q.seq).map_err(err)? {
                tokens::Step::Want(w) => want(w),
                tokens::Step::Sign(_) => Err(err("a status never signs".into())),
                tokens::Step::Done(r) => {
                    let mut v = base;
                    match r {
                        Ok(n) => v["accepted"] = json!(n),
                        Err(why) => v["waiting"] = json!(why),
                    }
                    js_sys::JSON::parse(&json!({ "done": v }).to_string())
                }
            }
        }
        /// An epoch STATEMENT root's acceptance (`root` hex): `{ want }`, or `{ done: { accepted: n } | { waiting: why } }`.
        #[allow(clippy::too_many_arguments)]
        pub fn statement_status(&self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], pool: &[u8], bag_code: &[u8], beacon_code: &[u8], epoch: f64, root: &str) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let lot = tokens::Lot { pool: b32(pool)?, bag: *blake3::hash(bag_code).as_bytes(), beacon_code: *blake3::hash(beacon_code).as_bytes(), idlog: ctx.idlog };
            let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
            match tokens::statement_accepted(&lot, &got, ctx.code_hash, ctx.beacon, epoch as u64, &bytes32("root", root).map_err(err)?).map_err(err)? {
                tokens::Step::Want(w) => js_sys::JSON::parse(&json!({ "want": w.iter().map(|x| hex(x)).collect::<Vec<_>>() }).to_string()),
                tokens::Step::Sign(_) => Err(err("a status never signs".into())),
                tokens::Step::Done(Ok(n)) => js_sys::JSON::parse(&json!({ "done": { "accepted": n } }).to_string()),
                tokens::Step::Done(Err(why)) => js_sys::JSON::parse(&json!({ "done": { "waiting": why } }).to_string()),
            }
        }
        /// $CRAFT's token id on this site's network (hex).
        pub fn craft_id(token_code: &[u8], beacon: &[u8], idlog_code: &[u8]) -> Result<String, JsValue> {
            Ok(hex(&keeper::craft(&token_ctx(token_code, beacon, idlog_code)?).1))
        }
        /// The beacon's OUTPUT at `height` (from its state as read; null: not there).
        pub fn beacon_out(&self, beacon: &[u8], height: f64) -> Result<JsValue, JsValue> {
            let s = self.0.got(&b32(beacon)?).and_then(craftworks_beacon_contract::wire::decode);
            Ok(s.and_then(|s| s.steps.iter().find(|st| st.height == height as u64).map(|st| st.out)).map_or(JsValue::NULL, |o| js_sys::Uint8Array::from(&o[..]).into()))
        }
        /// The STANDING EPOCH of beacon height `height` (the keeper's: `craftworks_standing::EPOCH`).
        pub fn keeper_epoch(height: f64) -> f64 {
            craftworks_standing::epoch(height as u64) as f64
        }
        /// OPEN the node's standing epoch (`keeper::open_epoch`; `hosted`: `[[id hex, size]]` from the node): `{ sign }`,
        /// `{ want }`, or `{ done: null | { puts: [{ id, idBytes, frames }] } }`.
        #[allow(clippy::too_many_arguments)]
        pub fn keeper_open(&mut self, standing_code: &[u8], beacon_code: &[u8], beacon: &[u8], public: &[u8], epoch: f64, hosted: JsValue, sigs: js_sys::Array) -> Result<JsValue, JsValue> {
            let k = keeper_ctx(standing_code, beacon_code, beacon)?;
            let hosted = hosted_of(&hosted)?;
            let sigs = sigs_of(&sigs)?;
            let step = {
                let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
                keeper::open_epoch(&k, &got, &b32(public)?, epoch as u64, &hosted, &sigs)
            };
            self.keeper_step(standing_code, step)
        }
        /// The PIECES height `height` asks this node for (`keeper::asked`): `{ done: null | [id hex] }` (or `{ want }`).
        #[allow(clippy::too_many_arguments)]
        pub fn keeper_asked(&self, standing_code: &[u8], beacon_code: &[u8], beacon: &[u8], public: &[u8], height: f64, out: &[u8], hosted: JsValue) -> Result<JsValue, JsValue> {
            let k = keeper_ctx(standing_code, beacon_code, beacon)?;
            let hosted = hosted_of(&hosted)?;
            let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
            token_step(keeper::asked(&k, &got, &b32(public)?, height as u64, &b32(out)?, &hosted), |a| Ok(json!(a.map(|ids| ids.iter().map(|i| hex(i)).collect::<Vec<_>>()))))
        }
        /// ANSWER height `height` (`keeper::answer`; `hashes`: `[[id hex, hash bytes]]` — sealed in the worker).
        #[allow(clippy::too_many_arguments)]
        pub fn keeper_answer(&mut self, standing_code: &[u8], beacon_code: &[u8], beacon: &[u8], public: &[u8], height: f64, out: &[u8], hosted: JsValue, hashes: js_sys::Array, sigs: js_sys::Array) -> Result<JsValue, JsValue> {
            let k = keeper_ctx(standing_code, beacon_code, beacon)?;
            let hosted = hosted_of(&hosted)?;
            let sigs = sigs_of(&sigs)?;
            let hashes: Vec<([u8; 32], [u8; 32])> = hashes.iter().map(|x| {
                let x = js_sys::Array::from(&x);
                Ok((bytes32("id", &x.get(0).as_string().unwrap_or_default()).map_err(err)?, b32(&js_sys::Uint8Array::new(&x.get(1)).to_vec())?))
            }).collect::<Result<_, JsValue>>()?;
            let step = {
                let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
                keeper::answer(&k, &got, &b32(public)?, height as u64, &b32(out)?, &hosted, &hashes, &sigs)
            };
            self.keeper_step(standing_code, step)
        }
        /// The STATEMENT BAG's address of epoch `epoch`, and a posted root's `{ epoch, root }` (null: not one).
        /// The height an epoch statement's witnesses are drawn at (`statement::lot_height`).
        pub fn statement_lot_height(epoch: f64) -> f64 {
            craftworks_reputation::statement::lot_height(epoch as u64) as f64
        }
        /// How far back a statement carries activity from (`statement::CARRY_BACK`).
        pub fn statement_carry_back() -> f64 {
            craftworks_reputation::statement::CARRY_BACK as f64
        }
        pub fn statement_bag_address(epoch: f64) -> js_sys::Uint8Array {
            js_sys::Uint8Array::from(&craftworks_reputation::statement::bag_address(epoch as u64)[..])
        }
        pub fn statement_posted_read(b: &[u8]) -> JsValue {
            match craftworks_reputation::statement::Posted::decode(b) {
                Some(p) => js_obj(&[("epoch", JsValue::from(p.epoch as f64)), ("root", JsValue::from(hex(&p.root)))]),
                None => JsValue::NULL,
            }
        }
        /// A beacon STATE's height (null: not a beacon's state).
        pub fn beacon_state_height(state: &[u8]) -> JsValue {
            craftworks_beacon_contract::wire::decode(state).map_or(JsValue::NULL, |s| JsValue::from(s.height() as f64))
        }
        /// The beacon's height, from its state as read (null: not read).
        pub fn beacon_height(&self, beacon: &[u8]) -> Result<JsValue, JsValue> {
            Ok(self.0.got(&b32(beacon)?).and_then(craftworks_beacon_contract::wire::decode).map_or(JsValue::NULL, |s| JsValue::from(s.height() as f64)))
        }
        /// An EDITION (`editions::edition`): `{ done: { id, name, cap, numbered, item, creator, royalty, sale,
        /// minted, open, mine } }` (`mine`: the copies minted to any of `owners` — this wallet's one-time owners).
        pub fn edition_read(&self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], id: &[u8], owners: js_sys::Array) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let owners = owners_of(&owners)?;
            let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
            token_step(editions::edition(&ctx, &got, &b32(id)?), |e| Ok(edition_json(&e, &owners)))
        }
        /// BUY PRIVATELY (`editions::buy`): from this wallet's stealth chains of the sale's token (`seed`; `mine`: the
        /// announcements it found, bytes each) — `{ done: { puts: [{ id, frames }], record, first, amount } }`.
        #[allow(clippy::too_many_arguments)]
        pub fn edition_buy(&mut self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], id: &[u8], seed: &[u8], mine: js_sys::Array, amount: f64, now: f64) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let mine: Vec<([u8; 32], [u8; 32])> = mine.iter().filter_map(|m| craftworks_wallet::stealth::Announce::decode(&js_sys::Uint8Array::new(&m).to_vec())).map(|a| (a.r, a.to)).collect();
            let step = {
                let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
                editions::buy(&ctx, &got, &b32(id)?, &b32(seed)?, &mine, amount as u128, now as u64)
            };
            let mut puts = Vec::new();
            let v = token_step(step, |b| {
                puts = b.puts.clone();
                Ok(json!({ "record": b.record, "first": b.first.to_string(), "amount": b.amount.to_string(), "to": hex(&b.to) }))
            })?;
            self.attach_puts(v, token_code, &puts)
        }
        /// COLLECT copy `n` (`editions::collect_copy`) into `owner`'s chain (a one-time owner): `{ done: { puts } }` (no
        /// puts: nothing to collect). Worked: nothing signs.
        pub fn edition_collect(&mut self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], id: &[u8], owner: &[u8], n: f64, now: f64) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let step = {
                let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
                editions::collect_copy(&ctx, &got, &b32(id)?, &b32(owner)?, n as u128, now as u64)
            };
            let mut puts = Vec::new();
            let v = token_step(step, |p| {
                puts = p.into_iter().collect();
                Ok(json!({}))
            })?;
            self.attach_puts(v, token_code, &puts)
        }
        /// An edition's SALES as announcements to its payee (`editions::sales`): `{ done: [bytes] }` — what its author
        /// collects (`wallet_receive_private`).
        pub fn edition_sales(&self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], id: &[u8]) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
            Ok(match editions::edition(&ctx, &got, &b32(id)?).map_err(err)? {
                tokens::Step::Done(e) => js_obj(&[("done", editions::sales(&e).iter().map(|a| JsValue::from(js_sys::Uint8Array::from(&a.encode()[..]))).collect::<js_sys::Array>().into())]),
                tokens::Step::Want(w) => js_obj(&[("want", w.iter().map(|x| JsValue::from(hex(x))).collect::<js_sys::Array>().into())]),
                tokens::Step::Sign(_) => return Err(err("an edition's read signs nothing".into())),
            })
        }
        /// A NEW EDITION of the item `item_ref`, SOLD ON DEMAND (TOKEN §1c): `price` of the token `pay_token` a copy, to
        /// `payee` — the author's one-time owner (§7f), also its royalty's; `cap` copies (0: none — numbered only);
        /// numbered or plain; a royalty of `royalty_bps` on resales (0: none); `nonce` 16 fresh bytes. `{ id, puts: [{ id,
        /// frames }] }` — its definition, authority chain and freeze list.
        #[allow(clippy::too_many_arguments)]
        pub fn edition_create(&mut self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], item_ref: &str, payee: &[u8], name: &str, cap: f64, numbered: bool, pay_token: &[u8], price: f64, royalty_bps: u16, nonce: &[u8]) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let payee = b32(payee)?;
            let d = editions::new_edition(&ctx, item_ref, &payee, name, cap as u128, numbered, &b32(pay_token)?, price as u128, royalty_bps, nonce.try_into().map_err(|_| err("a nonce is 16 bytes".into()))?).map_err(|e| err(e.into()))?;
            self.token_contract_puts(token_code, &ctx, &d)
        }
        /// A token as `owner` holds it (`wallet::view`): `{ done: { id, name, decimals, creator, supply, balance, begun } }`
        /// (amounts decimal strings, in the smallest unit).
        pub fn wallet_view(&self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], id: &[u8], owner: &[u8]) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let id = b32(id)?;
            let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
            token_step(wallet::view(&ctx, &got, &id, &b32(owner)?), |v| {
                let d = &v.definition;
                Ok(json!({ "id": hex(&id), "name": String::from_utf8_lossy(&d.name), "decimals": d.decimals, "creator": hex(&d.genesis_owner), "supply": d.genesis_amount.to_string(), "balance": v.balance.to_string(), "begun": v.begun, "staked": v.staked.to_string(), "mints": v.mints.iter().map(|(e, a, f, s)| json!({ "epoch": e, "amount": a.to_string(), "fee": f.to_string(), "stamp": s })).collect::<Vec<_>>() }))
            })
        }
        /// SEND PRIVATELY from this account's DID chain (`wallet::send`, TOKEN §7f): `amount` (a decimal string) of the
        /// token `id` to the wallet whose CARD keys are `pay_scan`/`pay_spend` — a fresh one-time owner of theirs
        /// (`rand`: 32 fresh bytes) — at beacon height `now`: `{ done: { announce, puts } }` (`announce`: bytes, for the
        /// payments bags).
        #[allow(clippy::too_many_arguments)]
        pub fn wallet_send(&mut self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], id: &[u8], from: &[u8], pay_scan: &[u8], pay_spend: &[u8], amount: &str, rand: &[u8], now: f64, public: &[u8], creds: js_sys::Array, sigs: js_sys::Array) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let (from, id) = (b32(from)?, b32(id)?);
            let amount: u128 = amount.trim().parse().map_err(|_| err("an amount is a whole number".into()))?;
            let card = craftworks_wallet::stealth::Public { scan: b32(pay_scan)?, spend: b32(pay_spend)? };
            let paid = craftworks_wallet::stealth::pay(&card, &b32(rand)?).ok_or_else(|| err("their card's pay keys are not keys".into()))?;
            let Some(dev) = account_device(&self.0, idlog_code, &from, public, &creds, &sigs)? else { return want_log(idlog_code, &from) };
            let step = {
                let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
                wallet::send(&ctx, &got, &id, &from, &paid.to, amount, now as u64, &dev)
            };
            let mut puts = Vec::new();
            let mut announce = None;
            let v = token_step(step, |s| {
                puts = vec![s.put];
                announce = Some(craftworks_wallet::stealth::Announce { token: id, from, nonce: s.nonce, to: paid.to, r: paid.r }.encode());
                Ok(json!({}))
            })?;
            let v = self.attach_puts(v, token_code, &puts)?;
            if let Some(a) = announce {
                js_sys::Reflect::set(&js_sys::Reflect::get(&v, &"done".into())?, &"announce".into(), &js_sys::Uint8Array::from(&a[..]))?;
            }
            Ok(v)
        }
        /// The STANDING EPOCH's length in beacon heights (`reputation::EPOCH`: vouch registries, statements, payments bags).
        /// Heights into an epoch before its statement is due (`reputation::statement::SETTLE`).
        pub fn statement_settle() -> f64 {
            craftworks_reputation::statement::SETTLE as f64
        }
        pub fn standing_epoch() -> f64 {
            craftworks_reputation::EPOCH as f64
        }
        /// This wallet's card PAY KEYS from its seed: `{ scan, spend }` (hex).
        pub fn stealth_public(seed: &[u8]) -> Result<JsValue, JsValue> {
            let k = craftworks_wallet::stealth::Keys::from_seed(&b32(seed)?).public();
            Ok(js_obj(&[("scan", JsValue::from(hex(&k.scan))), ("spend", JsValue::from(hex(&k.spend)))]))
        }
        /// A fresh ONE-TIME OWNER of this wallet's own (`seed`; `rand`: 32 fresh bytes) for the token `token` — an
        /// edition's payee (TOKEN §7f): `{ to, bytes }` (`bytes`: as an announcement — its R and owner — the form a found
        /// payment is kept in).
        pub fn stealth_own(seed: &[u8], token: &[u8], rand: &[u8]) -> Result<JsValue, JsValue> {
            let keys = craftworks_wallet::stealth::Keys::from_seed(&b32(seed)?);
            let p = craftworks_wallet::stealth::pay(&keys.public(), &b32(rand)?).ok_or_else(|| err("not keys".into()))?;
            let a = craftworks_wallet::stealth::Announce { token: b32(token)?, from: [0; 32], nonce: [0; 16], to: p.to, r: p.r };
            Ok(js_obj(&[("to", JsValue::from(hex(&p.to))), ("bytes", js_sys::Uint8Array::from(&a.encode()[..]).into())]))
        }
        /// The WITNESS QUEUE bag `k` of standing epoch `epoch` (its address; TOKEN §7a).
        pub fn witness_queue_address(epoch: f64, k: u32) -> js_sys::Uint8Array {
            js_sys::Uint8Array::from(&craftworks_wallet::queue::address(epoch as u64, k)[..])
        }
        /// A CHAINED bag (payments, the witness queue, the files registry: `wallet::chained`) with fewer items than this
        /// takes one; one with this many is full — the next (k + 1) is the one to read or drop into.
        /// Is a chained bag of these items (payloads) FULL (`bag::chained::full`: by count or by bytes)?
        pub fn chained_full(items: js_sys::Array) -> bool {
            craftworks_wallet::chained::full(items.iter().map(|x| js_sys::Uint8Array::from(x).length() as usize))
        }
        /// The FILES REGISTRY's bag `k` of standing epoch `epoch` (its address).
        pub fn registry_address(epoch: f64, k: u32) -> js_sys::Uint8Array {
            js_sys::Uint8Array::from(&craftworks_wallet::registry::address(epoch as u64, k)[..])
        }
        /// A file's registry ITEM: its address key and size.
        pub fn registry_item(address_key: &[u8], size: f64) -> Result<js_sys::Uint8Array, JsValue> {
            Ok(js_sys::Uint8Array::from(&craftworks_wallet::registry::Listed { address_key: b32(address_key)?, size: size as u64 }.encode()[..]))
        }
        /// FOUND (`craftworks_files::found`): a registered file of `size` bytes whose pieces at `missing` (indexes into
        /// `registry_pieces`) did not answer is there at its listed size — every generation rebuildable. Monotone: a
        /// read may stop at the first `false`.
        pub fn registry_found(size: f64, missing: js_sys::Array) -> bool {
            let missing: Vec<usize> = missing.iter().filter_map(|i| i.as_f64()).map(|i| i as usize).collect();
            craftworks_files::found(size as u64, &missing)
        }
        /// The files LIVE from an epoch's registry items and the epoch before's: `[{ key: address key hex, size }]`.
        pub fn registry_live(this: js_sys::Array, before: js_sys::Array) -> JsValue {
            let v = |a: &js_sys::Array| a.iter().map(|i| js_sys::Uint8Array::new(&i).to_vec()).collect::<Vec<_>>();
            let live = craftworks_wallet::registry::live(&v(&this), &v(&before));
            let out = js_sys::Array::new();
            for l in live {
                out.push(&js_obj(&[("key", JsValue::from(hex(&l.address_key))), ("size", JsValue::from(l.size as f64))]));
            }
            out.into()
        }
        /// The PAYMENTS BAG `k` of standing epoch `epoch` (its address).
        pub fn stealth_payments_address(epoch: f64, k: u32) -> js_sys::Uint8Array {
            js_sys::Uint8Array::from(&craftworks_wallet::stealth::payments_address(epoch as u64, k)[..])
        }
        /// SCAN a bag's items (announcement bytes): the ones paying this wallet (`seed`), as `[{ token, to, bytes }]`.
        pub fn stealth_scan(seed: &[u8], items: js_sys::Array) -> Result<js_sys::Array, JsValue> {
            let keys = craftworks_wallet::stealth::Keys::from_seed(&b32(seed)?);
            let all: Vec<craftworks_wallet::stealth::Announce> = items.iter().filter_map(|i| craftworks_wallet::stealth::Announce::decode(&js_sys::Uint8Array::new(&i).to_vec())).collect();
            Ok(keys.scan(&all).into_iter().map(|(a, _)| js_obj(&[("token", JsValue::from(hex(&a.token))), ("to", JsValue::from(hex(&a.to))), ("bytes", js_sys::Uint8Array::from(&a.encode()[..]).into())])).collect())
        }
        /// This wallet's PRIVATE holdings of the token `id` (`wallet::private`; `owners`: the one-time owners it found,
        /// 32 bytes each): `{ done: { id, name, decimals, balance, chains: [{ owner, balance }] } }`.
        pub fn wallet_private(&self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], id: &[u8], owners: js_sys::Array) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let id = b32(id)?;
            let owners: Vec<[u8; 32]> = owners.iter().map(|o| b32(&js_sys::Uint8Array::new(&o).to_vec())).collect::<Result<_, _>>()?;
            let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
            token_step(wallet::private(&ctx, &got, &id, &owners), |v| {
                let d = &v.definition;
                Ok(json!({ "id": hex(&id), "name": String::from_utf8_lossy(&d.name), "decimals": d.decimals, "balance": v.balance().to_string(), "chains": v.chains.iter().map(|(o, b)| json!({ "owner": hex(o), "balance": b.to_string() })).collect::<Vec<_>>() }))
            })
        }
        /// SEND PRIVATELY from this wallet's stealth chains (`wallet::send_private`; `mine`: the announcements it found,
        /// bytes each): `{ done: { announces: [bytes], rest: [bytes], puts } }` — `rest`: each paying chain's rest, moved
        /// to a fresh address of this wallet's own (a chain pays once).
        #[allow(clippy::too_many_arguments)]
        pub fn wallet_send_private(&mut self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], id: &[u8], seed: &[u8], mine: js_sys::Array, pay_scan: &[u8], pay_spend: &[u8], amount: &str, rand: &[u8], now: f64) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let amount: u128 = amount.trim().parse().map_err(|_| err("an amount is a whole number".into()))?;
            let mine: Vec<([u8; 32], [u8; 32])> = mine.iter().filter_map(|m| craftworks_wallet::stealth::Announce::decode(&js_sys::Uint8Array::new(&m).to_vec())).map(|a| (a.r, a.to)).collect();
            let card = craftworks_wallet::stealth::Public { scan: b32(pay_scan)?, spend: b32(pay_spend)? };
            let step = {
                let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
                wallet::send_private(&ctx, &got, &b32(id)?, &b32(seed)?, &mine, &card, amount, &b32(rand)?, now as u64)
            };
            let mut puts = Vec::new();
            let mut anns = Vec::new();
            let mut rest = Vec::new();
            let v = token_step(step, |s| {
                puts = s.puts;
                anns = s.announces.iter().map(|a| a.encode()).collect();
                rest = s.rest.iter().map(|a| a.encode()).collect();
                Ok(json!({}))
            })?;
            let v = self.attach_puts(v, token_code, &puts)?;
            if let Ok(done) = js_sys::Reflect::get(&v, &"done".into()) {
                if !done.is_undefined() {
                    let list = |l: &[Vec<u8>]| l.iter().map(|a| JsValue::from(js_sys::Uint8Array::from(&a[..]))).collect::<js_sys::Array>();
                    js_sys::Reflect::set(&done, &"announces".into(), &list(&anns))?;
                    js_sys::Reflect::set(&done, &"rest".into(), &list(&rest))?;
                }
            }
            Ok(v)
        }
        /// A PAYMENT's STATUS (`wallet::status`; `announce`: its bytes; the lot as for a receive): `{ done: null }` (its
        /// debit not there) or `{ done: { amount, stamp, stage: "settling" | "witnessing" | "accepted", why, signed } }`.
        #[allow(clippy::too_many_arguments)]
        pub fn wallet_payment_status(&self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], pool: &[u8], bag_code: &[u8], beacon_code: &[u8], announce: &[u8], now: f64) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let lot = tokens::Lot { pool: b32(pool)?, bag: *blake3::hash(bag_code).as_bytes(), beacon_code: *blake3::hash(beacon_code).as_bytes(), idlog: ctx.idlog };
            let a = craftworks_wallet::stealth::Announce::decode(announce).ok_or_else(|| err("not an announcement".into()))?;
            let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
            token_step(wallet::status(&ctx, &lot, &got, &a, now as u64), |s| {
                Ok(match s {
                    None => serde_json::Value::Null,
                    Some(s) => {
                        let (stage, why, signed) = match s.stage {
                            wallet::Stage::Settling => ("settling", None, None),
                            wallet::Stage::Unwitnessed(w) => ("witnessing", Some(w), None),
                            wallet::Stage::Accepted(n) => ("accepted", None, Some(n)),
                            wallet::Stage::Missing(w) => ("missing", Some(w), None),
                            wallet::Stage::Superseded => ("superseded", Some("its sender superseded it: never witnessed, its money back with the sender".to_string()), None),
                        };
                        json!({ "amount": s.amount.to_string(), "stamp": s.stamp, "stage": stage, "why": why, "signed": signed })
                    }
                })
            })
        }
        /// The LAST entry of `owner`'s chain of the token `id` as a witness-queue item (`{ want }`, or `{ done: null |
        /// { queued } }`): what the page checks for a WEDGED tail (TOKEN §3d) on its own DID chains — entries others append
        /// there (a keeper mint, by any page) are in no queue row of this page's.
        pub fn wallet_tail(&self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], id: &[u8], owner: &[u8]) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let (id, owner) = (b32(id)?, b32(owner)?);
            let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
            let step = tokens::holding(&ctx, &got, &craftworks_token_contract::Tok { definition: id, item: [0; 32] }, &owner);
            let mut queued = None;
            let v = token_step(step, |h| {
                queued = h.and_then(|(h, _)| h.state.entries.last().map(|e| craftworks_wallet::queue::Queued { token: h.params.tok(), owner, seq: e.seq }.encode()));
                Ok(if queued.is_some() { json!({}) } else { serde_json::Value::Null })
            })?;
            if let Some(q) = queued {
                js_sys::Reflect::set(&js_sys::Reflect::get(&v, &"done".into())?, &"queued".into(), &js_sys::Uint8Array::from(&q[..]))?;
            }
            Ok(v)
        }
        /// May a tail whose first entry is stamped `stamp` be SUPERSEDED at beacon height `now` (TOKEN §3d,
        /// `may_supersede`): no witness can ever sign it again.
        pub fn may_supersede(stamp: f64, now: f64) -> bool {
            craftworks_token_contract::may_supersede(stamp as u64, now as u64)
        }
        /// SUPERSEDE the WEDGED tail holding the queued entry `queued` (its bytes; TOKEN §3d, `wallet::supersede` /
        /// `supersede_private`) at beacon height `now` — a DID chain signed by this node's device (`public`, `creds`,
        /// `sigs`: `{ sign }` steps), or, `seed` given (32 bytes), this wallet's stealth chain found by `announce`:
        /// `{ want }`, `{ sign }`, `{ done: { superseded: true, from, to, puts } | { superseded: false, why } }`.
        #[allow(clippy::too_many_arguments)]
        pub fn wallet_supersede(&mut self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], pool: &[u8], bag_code: &[u8], beacon_code: &[u8], queued: &[u8], now: f64, public: &[u8], creds: js_sys::Array, sigs: js_sys::Array, seed: &[u8], announce: &[u8]) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let lot = tokens::Lot { pool: b32(pool)?, bag: *blake3::hash(bag_code).as_bytes(), beacon_code: *blake3::hash(beacon_code).as_bytes(), idlog: ctx.idlog };
            let q = craftworks_wallet::queue::Queued::decode(queued).ok_or_else(|| err("not a queued entry".into()))?;
            let step = if seed.is_empty() {
                let Some(dev) = account_device(&self.0, idlog_code, &q.owner, public, &creds, &sigs)? else { return want_log(idlog_code, &q.owner) };
                let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
                wallet::supersede(&ctx, &lot, &got, &q.token.definition, &q.owner, q.seq, now as u64, &dev)
            } else {
                let a = craftworks_wallet::stealth::Announce::decode(announce).ok_or_else(|| err("not an announcement".into()))?;
                if a.to != q.owner || a.token != q.token.definition {
                    return Err(err("an announcement of another chain than the entry's".into()));
                }
                let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
                wallet::supersede_private(&ctx, &lot, &got, &a, &b32(seed)?, q.seq, now as u64)
            };
            let mut puts = Vec::new();
            let v = token_step(step, |r| {
                Ok(match r {
                    Ok(s) => {
                        let j = json!({ "superseded": true, "from": s.from, "to": s.to, "stamp": s.stamp, "keeperEpochs": s.keeper_epochs });
                        puts.push(s.put);
                        j
                    }
                    Err(why) => json!({ "superseded": false, "why": why }),
                })
            })?;
            self.attach_puts(v, token_code, &puts)
        }
        /// RECEIVE a payment this wallet found (`wallet::receive_private`; `announce`: its bytes) at beacon height `now`,
        /// taken only once its debit is ACCEPTED (the lot: `pool` the stake pool's token, the bag's and beacon's codes):
        /// `{ done: { amount, pending, unwitnessed, puts } }`.
        #[allow(clippy::too_many_arguments)]
        pub fn wallet_receive_private(&mut self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], pool: &[u8], bag_code: &[u8], beacon_code: &[u8], announce: &[u8], now: f64) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let lot = tokens::Lot { pool: b32(pool)?, bag: *blake3::hash(bag_code).as_bytes(), beacon_code: *blake3::hash(beacon_code).as_bytes(), idlog: ctx.idlog };
            let a = craftworks_wallet::stealth::Announce::decode(announce).ok_or_else(|| err("not an announcement".into()))?;
            let step = {
                let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
                wallet::receive_private(&ctx, &lot, &got, &a, now as u64)
            };
            let mut puts = Vec::new();
            let v = token_step(step, |r| {
                puts = r.put.into_iter().collect();
                Ok(json!({ "amount": r.amount.to_string(), "pending": r.pending, "unwitnessed": r.unwitnessed, "why": r.why }))
            })?;
            self.attach_puts(v, token_code, &puts)
        }
        /// The 32-byte id an edition names the item `item_ref` by (`editions::item_id`), hex.
        pub fn edition_item_id(item_ref: &str) -> String {
            hex(&editions::item_id(item_ref))
        }
        /// An edition AS THE ITEM SHOWS IT: `{ done: { …edition, shown: true } }`, or `shown: false` with `why` (of
        /// another item: not shown); `mine`: copies minted to any of `owners` (this wallet's one-time owners).
        #[allow(clippy::too_many_arguments)]
        pub fn edition_of_item(&self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], id: &[u8], item_ref: &str, owners: js_sys::Array) -> Result<JsValue, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let owners = owners_of(&owners)?;
            let got = |x: &[u8; 32]| self.0.got(x).map(<[u8]>::to_vec);
            token_step(editions::edition(&ctx, &got, &b32(id)?), |e| {
                let mut v = edition_json(&e, &owners);
                match editions::of_item(&e, item_ref) {
                    Ok(()) => v["shown"] = json!(true),
                    Err(why) => {
                        v["shown"] = json!(false);
                        v["why"] = json!(why);
                    }
                }
                Ok(v)
            })
        }
        /// The token chain id of `owner`'s chain of `token` (an edition's copy `n`, or 0: the token's own), hex.
        pub fn edition_chain_id(&self, token_code: &[u8], beacon: &[u8], idlog_code: &[u8], token: &[u8], owner: &[u8], n: f64) -> Result<String, JsValue> {
            let ctx = token_ctx(token_code, beacon, idlog_code)?;
            let token = b32(token)?;
            let item = if n > 0.0 { craftworks_token_contract::numbered_item(&token, n as u128) } else { [0; 32] };
            let d = craftworks_token_contract::Definition::decode(self.0.got(&token).ok_or_else(|| err("the token's definition is not read".into()))?).ok_or_else(|| err("not a token definition".into()))?;
            Ok(hex(&editions::chain(&ctx, &d, &token, &b32(owner)?, item).id()))
        }
    }

    impl CraftworksCore {
        /// `done.puts`: each put as `{ id, frames }`.
        /// A new token's `{ id, puts: [{ id, frames }] }`: its definition, authority chain and freeze list.
        fn token_contract_puts(&mut self, token_code: &[u8], ctx: &tokens::Ctx, d: &craftworks_token_contract::Definition) -> Result<JsValue, JsValue> {
            let id = craftworks_token_contract::id(&ctx.code_hash, &d.encode());
            let list = js_sys::Array::new();
            for (params, state) in d.contracts(&ctx.code_hash) {
                let (pid, contract, state) = wire::puts::contract(token_code, &params, &state);
                let id_bytes: [u8; 32] = contract.key().id().as_bytes().try_into().map_err(|_| err("a contract id".into()))?;
                list.push(&self.put_obj(&account::Put { id: pid, id_bytes, contract, state })?);
            }
            Ok(js_obj(&[("id", JsValue::from(hex(&id))), ("puts", list.into())]))
        }
        /// A TOKEN CHAIN put with its WITNESS QUEUE items (TOKEN §7a): `{ id, idBytes, frames, queue: [bytes],
        /// witnesses: [params] }` — one `Queued` per entry not in the state this core read (what this page wrote), for
        /// its writer to drop into the epoch's queue once the put is taken, and the params of that entry's WITNESS
        /// SIGNATURES container (`witness::Params`, under its lot's claims epoch; the stake pool $CRAFT's) for its writer
        /// to make (empty, `witness_put`): a witness with no page open (the identity delegate) can only add to one
        /// already on the network.
        fn chain_put_obj(&mut self, token_code: &[u8], p: &tokens::Put) -> Result<JsValue, JsValue> {
            let (id, contract, state) = wire::puts::contract(token_code, &p.params, &p.state);
            let id_bytes: [u8; 32] = contract.key().id().as_bytes().try_into().map_err(|_| err("a contract id".into()))?;
            let o = self.put_obj(&account::Put { id, id_bytes, contract, state })?;
            let (queue, witnesses) = (js_sys::Array::new(), js_sys::Array::new());
            if let (Some(cp), Some(st)) = (craftworks_token_contract::ChainParams::decode(&p.params), craftworks_token_contract::State::decode(&p.state)) {
                let before = self.0.got(&id_bytes).and_then(craftworks_token_contract::State::decode).and_then(|s| s.entries.last().map(|e| e.seq));
                let margin = self.0.got(&cp.definition).and_then(craftworks_token_contract::Definition::decode).map(|d| d.margin);
                let lot = tokens::Lot { pool: keeper::craft(&tokens::Ctx { code_hash: cp.code_hash, beacon: cp.beacon, idlog: cp.idlog }).1, bag: [0; 32], beacon_code: [0; 32], idlog: cp.idlog };
                for (i, e) in st.entries.iter().enumerate().filter(|(_, e)| before.is_none_or(|b| e.seq > b)) {
                    let q = craftworks_wallet::queue::Queued { token: cp.tok(), owner: cp.owner, seq: e.seq };
                    queue.push(&js_sys::Uint8Array::from(&q.encode()[..]));
                    let lot_epoch = margin.and_then(|m| craftworks_token_contract::wallet::lot_of(e, i.checked_sub(1).map(|j| &st.entries[j]), m).ok()).map(|x| x.0);
                    witnesses.push(&lot_epoch.map_or(JsValue::NULL, |ep| js_sys::Uint8Array::from(&tokens::witness_at(cp.code_hash, &lot, cp.beacon, &cp.id(), e.seq, ep).encode()[..]).into()));
                }
            }
            js_sys::Reflect::set(&o, &"queue".into(), &queue)?;
            js_sys::Reflect::set(&o, &"witnesses".into(), &witnesses)?;
            Ok(o)
        }
        /// A keeper step's answer, its put (the standing contract's) attached.
        fn keeper_step(&mut self, standing_code: &[u8], step: Result<tokens::Step<Option<keeper::Put>>, String>) -> Result<JsValue, JsValue> {
            let mut put = None;
            let v = token_step(step, |p| {
                let some = p.is_some();
                put = p;
                Ok(if some { json!({}) } else { serde_json::Value::Null })
            })?;
            if let Some(p) = put {
                let (id, contract, state) = wire::puts::contract(standing_code, &p.params, &p.state);
                let id_bytes: [u8; 32] = contract.key().id().as_bytes().try_into().map_err(|_| err("a contract id".into()))?;
                let list = js_sys::Array::new();
                list.push(&self.put_obj(&account::Put { id, id_bytes, contract, state })?);
                js_sys::Reflect::set(&js_sys::Reflect::get(&v, &"done".into())?, &"puts".into(), &list)?;
            }
            Ok(v)
        }
        fn attach_puts(&mut self, v: JsValue, token_code: &[u8], puts: &[tokens::Put]) -> Result<JsValue, JsValue> {
            if puts.is_empty() {
                return Ok(v);
            }
            let done = js_sys::Reflect::get(&v, &"done".into())?;
            let list = js_sys::Array::new();
            for p in puts {
                list.push(&self.chain_put_obj(token_code, p)?);
            }
            js_sys::Reflect::set(&done, &"puts".into(), &list)?;
            Ok(v)
        }
    }

    #[wasm_bindgen]
    impl CraftworksCore {
        /// A FOLLOWED contract's state as held (hex id) — its first read's, or its newest push's: or null. Not taken: it
        /// is the page's copy (FOUNDATION §2 R6), read as often as wanted with no GET.
        pub fn held_state(&self, id_hex: &str) -> Result<JsValue, JsValue> {
            let id = bytes32("held", id_hex).map_err(err)?;
            Ok(self.0.got(&id).map(|s| JsValue::from(js_sys::Uint8Array::from(s))).unwrap_or(JsValue::NULL))
        }

        /// The state the node sent for an immutable read (a block, a file piece: hex id), taken — freed once used: or null.
        pub fn take_got(&mut self, id_hex: &str) -> Result<JsValue, JsValue> {
            let id = bytes32("got", id_hex).map_err(err)?;
            Ok(self
                .0
                .take_got(&id)
                .map(|s| JsValue::from(js_sys::Uint8Array::from(&s[..])))
                .unwrap_or(JsValue::NULL))
        }
        /// A generation coded: `[[j, id hex, frames, hash hex]]` (fragments `0 .. k + extra`). `burn`: the file's burn
        /// hash (32 bytes: the `piece` contract; empty: the `sealed` one).
        pub fn file_encode(
            &mut self,
            key: &[u8],
            size: f64,
            g: f64,
            plain: &[u8],
            extra: u32,
            burn: &[u8],
        ) -> Result<js_sys::Array, JsValue> {
            let plan = craftworks_files::Plan::of(size as u64);
            let pieces =
                craftworks_files::encode(&b32(key)?, &plan, g as u64, plain, extra as usize);
            pieces_js(self, pieces, b32(burn)?)
        }
        /// One more fragment `j` (a slow or refused one replaced): `[j, id hex, frames, hash hex]`.
        pub fn file_mint(
            &mut self,
            key: &[u8],
            size: f64,
            g: f64,
            plain: &[u8],
            j: u8,
            burn: &[u8],
        ) -> Result<js_sys::Array, JsValue> {
            let plan = craftworks_files::Plan::of(size as u64);
            let p = craftworks_files::mint(&b32(key)?, &plan, g as u64, plain, j);
            Ok(pieces_js(self, vec![(j, p)], b32(burn)?)?.get(0).into())
        }
        /// The INDEX for the fragments stored (JSON `[[[j, hash hex]]]` per generation): `{ root, puts: [[id, frames]] }`,
        /// the root's put LAST; each index piece addressed by its hash.
        pub fn file_index(
            &mut self,
            key: &[u8],
            size: f64,
            stored: &str,
            burn: &[u8],
        ) -> Result<js_sys::Object, JsValue> {
            let burn = b32(burn)?;
            let plan = craftworks_files::Plan::of(size as u64);
            let v: Vec<serde_json::Value> =
                serde_json::from_str(stored).map_err(|e| err(e.to_string()))?;
            let stored: Vec<craftworks_files::Listed> = v
                .iter()
                .map(|g| listed_of(&g.to_string()))
                .collect::<Result<_, _>>()?;
            let (pieces, root) = craftworks_files::index_hashed(&b32(key)?, &plan, &stored);
            let puts = js_sys::Array::new();
            for p in pieces {
                let (id, f) = self.0.frames_piece(&p, &burn).map_err(err)?;
                puts.push(
                    &[JsValue::from(id), JsValue::from(frames(f))]
                        .into_iter()
                        .collect::<js_sys::Array>(),
                );
            }
            let o = js_sys::Object::new();
            js_sys::Reflect::set(&o, &"root".into(), &JsValue::from(hex(&root)))?;
            js_sys::Reflect::set(&o, &"puts".into(), &puts)?;
            Ok(o)
        }
        /// Where a file's pieces are fetched (contract ids, hex): its root, an index piece (by `hash`, hex: its
        /// parent's, the reference's for the root), a fragment.
        pub fn file_root_id(&self, key: &[u8], hash: &str) -> Result<String, JsValue> {
            Ok(hex(&self.0.piece_id(&index_at(&b32(key)?, hash)?)))
        }
        pub fn file_index_id(&self, key: &[u8], hash: &str) -> Result<String, JsValue> {
            Ok(hex(&self.0.piece_id(&index_at(&b32(key)?, hash)?)))
        }
        pub fn file_fragment_id(&self, key: &[u8], g: f64, j: u8) -> Result<String, JsValue> {
            Ok(hex(&self.0.piece_id(&craftworks_files::fragment_address(&b32(key)?, g as u64, j))))
        }
        /// A REGISTERED file's pieces from its address key and size alone (a keeper's, who cannot read its index): each
        /// generation's fragments as uploaded (`k` + `EXTRA`), `[[id hex, size]]`.
        pub fn registry_pieces(&self, address_key: &[u8], size: f64) -> Result<JsValue, JsValue> {
            let ak = b32(address_key)?;
            let plan = craftworks_files::Plan::of(size as u64);
            let out = js_sys::Array::new();
            for (g, j) in craftworks_files::registered(&plan) {
                let id = hex(&self.0.piece_id(&craftworks_files::fragment_at(&ak, g, j)));
                out.push(&js_sys::Array::of2(&JsValue::from(id), &JsValue::from(plan.chunk as f64)));
            }
            Ok(out.into())
        }
        /// A file's ADDRESS KEY (what the files registry lists: its pieces' addresses, nothing it opens).
        pub fn file_address_key(key: &[u8]) -> Result<js_sys::Uint8Array, JsValue> {
            Ok(js_sys::Uint8Array::from(&craftworks_files::address_key(&b32(key)?)[..]))
        }
        /// KEEP a file's piece (`what` and `a`, `b` as `file_burn`): the state a GET gave, put again. `[id, frames]`.
        pub fn file_keep(
            &mut self,
            key: &[u8],
            what: &str,
            a: f64,
            b: f64,
            state: &[u8],
            hash: &str,
        ) -> Result<js_sys::Array, JsValue> {
            let key = b32(key)?;
            let address = match what {
                "fragment" => craftworks_files::fragment_address(&key, a as u64, b as u8),
                _ => index_at(&key, hash)?,
            };
            let (id, f) = self
                .0
                .frames_keep_piece(&address, state)
                .map_err(err)?;
            Ok([JsValue::from(id), JsValue::from(frames(f))]
                .into_iter()
                .collect())
        }
        /// BURN a file's piece (`what`: "root", "index" with `a` = level and `b` = n, "fragment" with `a` = g and
        /// `b` = j) with its secret: `[id hex, frames]`.
        pub fn file_burn(
            &mut self,
            key: &[u8],
            what: &str,
            a: f64,
            b: f64,
            secret: &[u8],
            hash: &str,
        ) -> Result<js_sys::Array, JsValue> {
            let key = b32(key)?;
            let address = match what {
                "fragment" => craftworks_files::fragment_address(&key, a as u64, b as u8),
                _ => index_at(&key, hash)?,
            };
            let (id, f) = self.0.frames_burn(&address, &b32(secret)?).map_err(err)?;
            Ok([JsValue::from(id), JsValue::from(frames(f))]
                .into_iter()
                .collect())
        }
    }

    /// Where a file's index piece (the root, or one below it) is: by its `hash` (hex).
    fn index_at(key: &[u8; 32], hash: &str) -> Result<[u8; 32], JsValue> {
        Ok(craftworks_files::hashed_address(key, &bytes32("index piece", hash).map_err(err)?))
    }

    /// A file's plan: `{ size, chunk, chunks, gens }`.
    #[wasm_bindgen]
    pub fn file_plan(size: f64) -> String {
        plan_js(&craftworks_files::Plan::of(size as u64))
    }
    /// A file's key from its content hash, with its space's salt (none: a public file).
    #[wasm_bindgen]
    pub fn file_key(content: &[u8], salt: &[u8]) -> Result<js_sys::Uint8Array, JsValue> {
        let salt = if salt.is_empty() {
            None
        } else {
            Some(b32(salt)?)
        };
        Ok(js_sys::Uint8Array::from(
            &craftworks_files::content_key(&b32(content)?, salt.as_ref())[..],
        ))
    }
    /// A file's ROOT opened: `{ plan, depth, children: [hex] }`.
    #[wasm_bindgen]
    pub fn file_root(key: &[u8], root_hash: &str, state: &[u8]) -> Result<String, JsValue> {
        let h = bytes32("root", root_hash).map_err(err)?;
        let r = craftworks_files::Root::open(&b32(key)?, &h, state).map_err(file_err)?;
        Ok(serde_json::json!({
            "plan": serde_json::from_str::<serde_json::Value>(&plan_js(&r.plan)).unwrap(),
            "depth": r.depth, "children": r.children.iter().map(|c| hex(c)).collect::<Vec<_>>(),
        })
        .to_string())
    }
    /// HEX — the one implementation (`identity::hex` / `unhex`, every crate's) for every package of the page.
    #[wasm_bindgen]
    pub fn hex_of(b: &[u8]) -> String {
        hex(b)
    }
    /// Bytes from hex (whole bytes of hex digits only; anything else refused).
    #[wasm_bindgen]
    pub fn bytes_of(h: &str) -> Result<Vec<u8>, JsValue> {
        craftworks_identity::unhex(h).ok_or_else(|| err(format!("not hex: {:?}", h.chars().take(16).collect::<String>())))
    }
    /// The FILES constants the page uses (`files`'s): `{ inlineMax, gen, extra }`.
    #[wasm_bindgen]
    pub fn file_constants() -> String {
        json!({ "inlineMax": craftworks_files::INLINE_MAX, "gen": craftworks_files::GEN, "extra": craftworks_files::EXTRA }).to_string()
    }
    /// A generation's STORE, rateless (`files::store_next`, the node's too): "done", "failed", "wait", or the `j` to
    /// mint and put next.
    #[wasm_bindgen]
    pub fn file_store_next(size: f64, g: f64, stored: u32, next: u32, out: u32) -> JsValue {
        use craftworks_files::StoreNext;
        match craftworks_files::store_next(&craftworks_files::Plan::of(size as u64), g as u64, stored as usize, next as usize, out as usize) {
            StoreNext::Mint(j) => JsValue::from(j),
            StoreNext::Wait => JsValue::from_str("wait"),
            StoreNext::Done => JsValue::from_str("done"),
            StoreNext::Failed => JsValue::from_str("failed"),
        }
    }
    /// THE INDEX WALK (`files::Root`, the node's too). The leaf listing generation `g` and its place there:
    /// `[leaf, slot]`.
    #[wasm_bindgen]
    pub fn file_leaf_of(g: f64) -> Vec<f64> {
        vec![craftworks_files::Root::leaf_of(g as u64) as f64, craftworks_files::Root::gen_slot(g as u64) as f64]
    }
    /// Index piece `n` at `level`: its parent and its place there, `[parentLevel, parentN, slot]`.
    #[wasm_bindgen]
    pub fn file_parent(level: u8, n: f64) -> Vec<f64> {
        let ((l, p), slot) = craftworks_files::Root::parent(level, n as u64);
        vec![l as f64, p as f64, slot as f64]
    }
    /// Every index piece below a root of `depth` for a file of `size`: JSON `[[level, n]]`, leaves first.
    #[wasm_bindgen]
    pub fn file_index_pieces(size: f64, depth: u8) -> String {
        let r = craftworks_files::Root { plan: craftworks_files::Plan::of(size as u64), depth, children: Vec::new() };
        json!(r.index_pieces()).to_string()
    }
    /// An inner index piece opened: its children's hashes.
    #[wasm_bindgen]
    pub fn file_inner(key: &[u8], hash: &str, state: &[u8]) -> Result<Vec<String>, JsValue> {
        let h = bytes32("inner", hash).map_err(err)?;
        Ok(
            craftworks_files::open_inner(&b32(key)?, &h, state)
                .map_err(file_err)?
                .iter()
                .map(|c| hex(c))
                .collect(),
        )
    }
    /// A leaf opened: per generation it covers, `[[j, hash hex]]`.
    #[wasm_bindgen]
    pub fn file_leaf(
        key: &[u8],
        size: f64,
        n: f64,
        hash: &str,
        state: &[u8],
    ) -> Result<String, JsValue> {
        let h = bytes32("leaf", hash).map_err(err)?;
        let plan = craftworks_files::Plan::of(size as u64);
        let gens = craftworks_files::open_leaf(&b32(key)?, &plan, n as u64, &h, state)
            .map_err(file_err)?;
        Ok(serde_json::Value::Array(
            gens.iter()
                .map(|l| {
                    serde_json::json!(l
                        .iter()
                        .map(|(j, h)| serde_json::json!([j, hex(h)]))
                        .collect::<Vec<_>>())
                })
                .collect(),
        )
        .to_string())
    }
    /// ONE CHUNK alone (a seek): chunk `i` from its systematic fragment, checked.
    #[wasm_bindgen]
    pub fn file_read_chunk(
        key: &[u8],
        size: f64,
        listed: &str,
        i: f64,
        state: &[u8],
    ) -> Result<js_sys::Uint8Array, JsValue> {
        let plan = craftworks_files::Plan::of(size as u64);
        Ok(js_sys::Uint8Array::from(
            &craftworks_files::read_chunk(&b32(key)?, &plan, &listed_of(listed)?, i as u64, state)
                .map_err(file_err)?[..],
        ))
    }

    /// A person's LEVEL now from their card's contributions (`craftworks_gov::level_from`, the one rule: the identity
    /// delegate admits into a level group by it too): JSON `{ id, perMonth }`.
    #[wasm_bindgen]
    pub fn level_from(contrib: &str, now: f64) -> String {
        let (id, per_month) = craftworks_gov::level_from(contrib, now);
        serde_json::json!({ "id": id, "perMonth": per_month }).to_string()
    }

    /// Whether level `have` reaches `need` (`craftworks_gov::level_reaches`).
    #[wasm_bindgen]
    pub fn level_reaches(have: &str, need: &str) -> bool {
        craftworks_gov::level_reaches(have, need)
    }

    /// THE LEVELS' thresholds (`craftworks_gov::LEVELS`): JSON `[[id, per month at least]]`, highest first.
    #[wasm_bindgen]
    pub fn levels() -> String {
        serde_json::to_string(&craftworks_gov::LEVELS).unwrap_or_default()
    }

    /// A space's GOVERNANCE (`craftworks_gov`, the one replay: the identity delegate reads a space by it too).
    #[wasm_bindgen]
    pub struct Governance(craftworks_gov::Gov);

    #[wasm_bindgen]
    impl Governance {
        /// Replay acts: `rows` JSON `[[id, value, writer | null]]`, `writers` JSON `{ node: did }`, the first owner,
        /// now (ms).
        pub fn replay(
            rows: &str,
            writers: &str,
            first: Option<String>,
            now: f64,
        ) -> Result<Governance, JsValue> {
            let rows: Vec<(String, String, Option<String>)> =
                serde_json::from_str(rows).map_err(|e| err(e.to_string()))?;
            let writers: std::collections::HashMap<String, String> =
                serde_json::from_str(writers).map_err(|e| err(e.to_string()))?;
            let rows: Vec<craftworks_gov::Row> = rows
                .into_iter()
                .map(|(id, value, writer)| craftworks_gov::Row { id, value, writer })
                .collect();
            Ok(Governance(craftworks_gov::Gov::replay(
                &rows,
                &writers,
                first.as_deref(),
                now,
            )))
        }
        pub fn owner(&self) -> Option<String> {
            self.0.owner.clone()
        }
        /// By the acts (null: removed), else a member if the group has them.
        pub fn role(&self, did: &str, in_group: bool) -> Option<String> {
            self.0.role(did, in_group)
        }
        /// A valid policy `who` (`gov::valid_who`: the built-in ones, a composed role, a list, a level).
        pub fn valid_who(who: &str) -> bool {
            craftworks_gov::valid_who(who)
        }
        /// The actions a policy names.
        pub fn actions() -> Vec<String> {
            craftworks_gov::ACTIONS
                .iter()
                .map(|a| a.to_string())
                .collect()
        }
        /// The acts that put someone OUT (`Gov::out_acts`), JSON, in order.
        pub fn out_acts(&self) -> String {
            serde_json::Value::Array(self.0.out_acts().into_iter().cloned().collect()).to_string()
        }
        /// WHO MAY LET PEOPLE IN: the replay's own invite rule (`Gov::may_invite` — what its `admitted` acts are judged by;
        /// the node's follower ranks admitters by it). `role`: the person's role as known.
        pub fn may_invite(&self, role: Option<String>, at: f64) -> bool {
            self.0.may_invite(role.as_deref(), at)
        }
        /// THE GUARD "may list" (`craftworks_rules::guards::may_list`): why `did` may not be put on the list, or
        /// `undefined`.
        pub fn may_list(&self, did: &str) -> Option<String> {
            craftworks_rules::guards::may_list(&self.0, did).err()
        }
        /// THE PERMISSION an act needs (`gov::act_need`, the replay's one table): `grant` (the owner's), `remove`,
        /// `moderate`, `apps`, `roles`, `invite` — or `undefined` (a person's own act).
        pub fn act_need(&self, act: &str) -> Option<String> {
            craftworks_gov::act_need(&serde_json::from_str(act).unwrap_or_default()).map(str::to_string)
        }
        /// MAY `by` TAKE `did` OUT (`Gov::may_take_out`: the replay's remove rule).
        pub fn may_take_out(&self, by: &str, did: &str) -> bool {
            self.0.may_take_out(by, did)
        }
        /// The BASE ROLE `did` held at `at` (`Gov::role_at`): "owner", "admin", "member", or `undefined`.
        pub fn role_at(&self, did: &str, at: f64) -> Option<String> {
            self.0.role_at(did, at).map(str::to_string)
        }
        /// THE LIST an MLS entry is judged by (`mls::List`; the node's `GovList`): EVER on the list and not banned.
        pub fn entry_listed(&self, did: &str) -> bool {
            self.0.entry_listed(did)
        }
        /// …a self-entry's proof (JSON) by the list's rule now (`Gov::entry_admits`, the node's too).
        pub fn entry_admits(&self, did: &str, proof: &str, at: f64) -> bool {
            self.0.entry_admits(did, proof.as_bytes(), at)
        }
        /// …and whether a member's commit may take `did` out (`Gov::may_be_taken_out`).
        pub fn may_be_taken_out(&self, did: &str) -> bool {
            self.0.may_be_taken_out(did)
        }
        /// MAY `did` ENTER BY ITSELF with this proof (JSON: `{ join: "code", p } | { join: "open" } | { join: "level" }`)
        /// — `Gov::admits`, the replay's and every reader's: why not, or `undefined`.
        pub fn admits(&self, did: &str, proof: &str, at: f64) -> Option<String> {
            let v: serde_json::Value = serde_json::from_str(proof).unwrap_or_default();
            self.0.admits(did, &v, at).err()
        }
        /// THE GUARD "in standing" (`craftworks_rules::guards::standing`, the delegate's too): why `did`, asking at `at`,
        /// is not let in by a request — or `undefined`.
        pub fn standing(&self, did: &str, at: f64) -> Option<String> {
            craftworks_rules::guards::standing(&self.0, did, at as u64).err()
        }
        /// THE ONE STAMP of an act to write (`Gov::stamp`): the act (JSON) with its `deps` and `at`.
        pub fn stamp(&self, act: &str, now: f64) -> Result<String, JsValue> {
            let v: serde_json::Value = serde_json::from_str(act).map_err(|e| err(e.to_string()))?;
            Ok(self.0.stamp(v, now).to_string())
        }
        /// The ids counted acts name that are not among `have` (JSON [id]): `Gov::missing`.
        pub fn missing(&self, have: &str) -> Result<String, JsValue> {
            let have: std::collections::BTreeSet<String> = serde_json::from_str(have).map_err(|e| err(e.to_string()))?;
            Ok(serde_json::to_string(&self.0.missing(&have)).unwrap_or_default())
        }
        /// The acts that counted (JSON), all or of one kind.
        pub fn counted(&self, kind: Option<String>) -> String {
            serde_json::Value::Array(
                self.0
                    .counted
                    .iter()
                    .filter(|a| {
                        kind.as_deref()
                            .is_none_or(|k| a.get("act").and_then(|x| x.as_str()) == Some(k))
                    })
                    .cloned()
                    .collect(),
            )
            .to_string()
        }
        /// The invite codes in force at `now` (JSON: `[{ code, by, at, expires, uses, admitted }]`).
        pub fn invites(&self, now: f64) -> String {
            serde_json::Value::Array(
                self.0
                    .live_invites(now)
                    .iter()
                    .map(|i| serde_json::json!({ "code": i.code, "by": i.by, "at": i.at, "expires": i.expires, "uses": i.uses, "admitted": i.admitted }))
                    .collect(),
            )
            .to_string()
        }
        /// The LISTS of named people (JSON `{ id: { by, people } }`): audiences by name.
        pub fn lists(&self) -> String {
            serde_json::Value::Object(
                self.0.lists.iter().map(|(id, (by, people))| (id.clone(), serde_json::json!({ "by": by, "people": people }))).collect(),
            )
            .to_string()
        }
        /// The apps the space uses: every one its acts turned on (and not off since), by name.
        pub fn apps(&self) -> Vec<String> {
            let mut on: Vec<String> = self.0.apps.iter().filter(|(_, v)| **v).map(|(k, _)| k.clone()).collect();
            on.sort();
            on
        }
        /// An app's content setting (JSON), or none.
        pub fn config(&self, app: &str, key: &str) -> Option<String> {
            self.0
                .configs
                .get(&format!("{app}/{key}"))
                .map(|v| v.to_string())
        }
        pub fn effective(&self, path: &str, action: &str, at: f64) -> String {
            self.0.effective(path, action, at).to_string()
        }
        pub fn policy_at(&self, path: &str, action: &str, at: f64) -> Option<String> {
            self.0.policy_at(path, action, at).map(str::to_string)
        }
        pub fn banned(&self, did: &str) -> bool {
            self.0.bans.contains(did)
        }
        pub fn bans(&self) -> Vec<String> {
            self.0.bans.iter().cloned().collect()
        }
        /// Out by the acts (removed, banned, left) and not added back.
        pub fn gone(&self) -> Vec<String> {
            self.0.gone.iter().cloned().collect()
        }
        pub fn roster(&self) -> Vec<String> {
            self.0.roster.iter().cloned().collect()
        }
        pub fn is_public(&self) -> bool {
            self.0.is_public()
        }
        /// Node → DID learned from removals (JSON `[[node, did]]`).
        pub fn learned(&self) -> String {
            serde_json::to_string(&self.0.learned).expect("strings")
        }
        pub fn passes(who: &str, role: Option<String>) -> bool {
            craftworks_gov::passes(who, role.as_deref())
        }
        /// What a person may: their base role's and every composed role's they hold.
        pub fn may(&self, did: &str, in_group: bool, what: &str) -> bool {
            let r = self.0.role(did, in_group);
            self.0.may(did, r.as_deref(), what)
        }
        /// Does a person pass a policy's `who` (a composed role: by holding it; the owner and admins always).
        pub fn passes_did(&self, who: &str, did: &str, in_group: bool) -> bool {
            let r = self.0.role(did, in_group);
            self.0.passes_did(who, did, r.as_deref())
        }
        /// …AS IT WAS at `at` (`Gov::passes_did_at`: the base and composed roles held then).
        pub fn passes_did_at(&self, who: &str, did: &str, at: f64) -> bool {
            self.0.passes_did_at(who, did, at)
        }
        /// The composed roles (JSON `[{ id, name, perms }]`), and who holds which (JSON `{ did: [id] }`).
        pub fn roles_defined(&self) -> String {
            serde_json::Value::Array(
                self.0
                    .defined
                    .iter()
                    .map(
                        |(id, r)| serde_json::json!({ "id": id, "name": r.name, "perms": r.perms }),
                    )
                    .collect(),
            )
            .to_string()
        }
        pub fn roles_held(&self) -> String {
            serde_json::to_string(&self.0.held).expect("strings")
        }
        /// What a composed role may carry.
        pub fn perms() -> Vec<String> {
            craftworks_gov::PERMS
                .iter()
                .map(|p| p.to_string())
                .collect()
        }
        pub fn can_role(role: Option<String>, what: &str) -> bool {
            craftworks_gov::can_role(role.as_deref(), what)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_requests_frame_and_ids_advance() {
        let mut c = Core::new(b"\0asm\x01\0\0\0");
        let (a, fa) = c.frames_identity(&Request::Lock).unwrap();
        let (b, _) = c.frames_identity(&Request::Who).unwrap();
        assert_eq!((a, b), (1, 2));
        assert!(!fa.is_empty());
        assert!(!c.frames_register_identity().unwrap().is_empty());
    }

    /// An earlier build is asked for a space's EPOCH SECRETS (10-18: what its handover left behind) — framed to it;
    /// control: anything else (a grant, a lock) still refused there.
    #[test]
    fn an_earlier_build_is_asked_for_a_spaces_keys_and_nothing_else() {
        let mut c = Core::new(b"\0asm\x01\0\0\0");
        let prior = "AP3BuSjFrhJ45Fg7iTvsFb2ER8Hda3iXR3eXjTfrLvnQ:9gcqi8176H7WehFLSqsT1s6BZBxJD4DfX2X4g2Sma7r2";
        let (_, f) = c.frames_prior(prior, &Request::EpochSecrets { space: [5; 32], below: 1 << 24 }).expect("a space's keys may be asked of an earlier build");
        assert!(!f.is_empty());
        assert!(c.frames_prior(prior, &Request::Lock).is_err(), "control: a lock is not asked of an earlier build");
        assert!(c.frames_prior(prior, &Request::Grant { tables: vec!["x".into()] }).is_err(), "control: nor a grant");
    }

    #[test]
    fn a_handover_is_addressed_to_the_earlier_build_it_names() {
        let mut c = Core::new(b"\0asm\x01\0\0\0");
        let prior = "AP3BuSjFrhJ45Fg7iTvsFb2ER8Hda3iXR3eXjTfrLvnQ:9gcqi8176H7WehFLSqsT1s6BZBxJD4DfX2X4g2Sma7r2";
        let (_, to_prior) = c.frames_handover(prior, "123456".into()).unwrap();
        let has = |f: &[Vec<u8>], k: &[u8]| f.iter().any(|b| b.windows(k.len()).any(|w| w == k));
        let key = bs58::decode("AP3BuSjFrhJ45Fg7iTvsFb2ER8Hda3iXR3eXjTfrLvnQ")
            .into_vec()
            .unwrap();
        assert!(has(&to_prior, &key), "addressed to the earlier build's key");
        let (_, to_this) = c
            .frames_identity(&Request::Handover {
                pin: "123456".into(),
            })
            .unwrap();
        assert!(
            !has(&to_this, &key),
            "control: this build's own request does not carry it"
        );
        assert!(c.frames_handover("no-colon", "1".into()).is_err());
        assert!(c.frames_handover("abc:def", "1".into()).is_err());
    }

    /// A LOST block whose whole group is ALREADY HELD (owner 10-07: one block of `spacekeys` was not on the network; the
    /// page held every other member of its group, so nothing was left to fetch — and that empty fetch was taken for
    /// "no group": the block was declared lost, never rebuilt). The group is named, nothing asked, and it rebuilds.
    #[test]
    fn a_lost_block_whose_group_is_all_held_is_rebuilt_not_declared_lost() {
        use ed25519_dalek::{Signer, SigningKey};
        let code: &[u8] = b"\0asm\x01\0\0\0";
        let key = SigningKey::from_bytes(&[5; 32]);
        let member = key.verifying_key().to_bytes();
        let sign = |o: &data::Open, seq: u64, h: [u8; 32]| {
            let p = craftec_register_contract::wire::Params::parse(&o.params).unwrap();
            key.sign(&p.signed_message(false, seq, &h)).to_bytes()
        };
        // A WRITER makes a tree of several leaves (as the data crate's own repair test).
        let mut w = data::Open::new(code, &member, "notes");
        w.set_table_key([7; 32]);
        for round in 0..6 {
            for i in 0..data::FLUSH_AT {
                let (seq, h) = w.prepare_row(format!("{round:02}-{i:03}").as_bytes(), "x".repeat(200).as_bytes()).unwrap();
                let s = sign(&w, seq, h);
                w.commit(s).unwrap();
            }
            let Ok(data::Step::Ready(f)) = w.flush() else { panic!("ready") };
            let s = sign(&w, f.seq, f.hash);
            w.commit(s).unwrap();
        }
        let root = w.writer.body().root.unwrap();
        let node = freenet_prolly::store::load(&w.blocks, &root).unwrap();
        let lost = node.child(1).0;
        let by = w.writes.unwrap();
        let net = |c: &freenet_prolly::Cid| data::seal_block(&[7; 32], by, c, &contract_keys::block::block_state(c, w.blocks.0.get(c).unwrap()).unwrap());
        // A READER in the core: every block of the tree held but the lost one — its whole group with it.
        let mut c = Core::new(code);
        let mut r = data::Open::new(code, &member, "notes");
        r.set_table_key([7; 32]);
        assert!(r.absorb(&w.writer.state()));
        for b in w.blocks.0.keys().copied().collect::<Vec<_>>() {
            if b != lost {
                assert!(r.absorb_block(&b, &net(&b)), "a block absorbs");
            }
        }
        let group = r.repair_group(&lost);
        let id = [0x11; 32];
        c.tails.insert(id, r);
        let asked = c.want(&id, &[lost]).expect("the lost block is asked for");
        let lost_contract = bytes32("contract", &asked[0]).unwrap();
        assert!(group.is_some(), "the writer's tree names a group for it");
        let g = c.tail_group(&id, &lost_contract).expect("its group is named, though nothing of it is left to fetch");
        assert!(g.is_empty(), "every member held: nothing asked");
        assert_eq!(c.tail_rebuild(&lost_contract), Ok(true), "rebuilt from what is held");
        // …and PUT BACK: the read names it by the contract it was asked at (the network's address), never its cid.
        let put = c.tail_keep_put(&id, &lost_contract).expect("a put back is made");
        assert!(put.is_some(), "the rebuilt block is put back, under the address it was lost at");
        assert_eq!(put.unwrap().0, bs58::encode(lost_contract).into_string(), "at the very contract that was not there");
    }

    #[test]
    fn answers_read_as_json() {
        let a = answer_json(&Answer::Unlocked {
            public: [1; 32],
            did: [2; 32],
            data: Some([3; 32]),
        });
        assert_eq!(a["unlocked"]["data"], hex(&[3; 32]));
        assert_eq!(a["unlocked"]["did"], hex(&[2; 32]));
        assert_eq!(
            answer_json(&Answer::WrongPin { tries_left: 3 })["wrongPin"]["triesLeft"],
            3
        );
        assert_eq!(bytes32("x", &hex(&[7; 32])).unwrap(), [7; 32]);
        assert!(bytes32("x", "zz").is_err());
    }
}
