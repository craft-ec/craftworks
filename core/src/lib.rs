//! The Craftworks app's one wasm package: the node's message framing (`wire`), the identity delegate's questions and
//! answers, and the account: its recovery words and the DID they name. It holds no key after a call returns and makes no decision: it
//! frames what the page asks and names what the node answers, as JSON the components can show.

/// The account: its own crate (shared with mls), under its old name here.
pub use craftworks_account as account;
/// A table as a tail: its own crate (the identity delegate drives it too).
pub use craftworks_data as data;

use craftworks_identity::{decode_answer, encode_request, Answer, Request};
use serde_json::{json, Value};
use wire::{AckKind, Incoming, Reassembler};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

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
        Answer::Exported { seed } => json!({ "exported": hex(seed) }),
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
            admitted,
            groups,
            stale,
            said,
        } => json!({ "upkeep": {
            "wakeups": wakeups, "inbox": inbox.map(|i| hex(&i)), "inboxLen": inbox_len, "now": now, "codes": codes.map(|c| hex(&c)),
            "admitted": admitted.iter().map(|a| json!({ "space": hex(&a.space), "did": a.did, "code": a.code, "at": a.at, "epoch": a.epoch, "kp": hex(&a.kp) })).collect::<Vec<_>>(),
            "groups": groups.iter().map(|(s, e, st)| json!({ "space": hex(s), "epoch": e, "state": hex(st) })).collect::<Vec<_>>(),
            "stale": stale.iter().map(|s| hex(s)).collect::<Vec<_>>(), "said": said,
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
                    None => json!({ "answer": "unreadable", "bytes": m.len() }),
                })
                .collect();
            json!({ "kind": "identity", "answers": answers })
        }
        Incoming::Refused(r) => json!({ "kind": "refused", "said": format!("{r:?}") }),
        Incoming::Unusable(u) => json!({ "kind": "unusable", "why": format!("{u:?}") }),
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
        if !matches!(
            req,
            Request::Handover { .. }
                | Request::HandoverKeys { .. }
                | Request::HandoverSpaces { .. }
        ) {
            return Err("an earlier build is asked only to hand a member over".into());
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

    /// The state the node sent for `id`, TAKEN (a file's fragments: read once, never kept here).
    pub fn take_got(&mut self, id: &[u8; 32]) -> Option<Vec<u8>> {
        self.got.remove(id)
    }

    /// A FILE PIECE to put, as `(id hex, frames)`: with a BURN HASH, the `piece` contract at its address (its state
    /// `LIVE ‖ burn hash ‖ piece`); without, the `sealed` contract (a file from before burning).
    pub fn frames_piece(
        &mut self,
        p: &craftworks_files::Piece,
        burn: Option<&[u8; 32]>,
    ) -> Result<(String, Vec<Vec<u8>>), String> {
        let s = self.stream();
        let (code, state) = match burn {
            Some(h) => (&self.piece_code, [&[2u8][..], h, &p.state].concat()),
            None => (&self.sealed_code, p.state.clone()),
        };
        let c = wire::block::block_contract(code, &p.address);
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

    /// KEEP a file piece: the state it is stored as (a GET's answer, whole) put again at `address` — the `piece`
    /// contract (`burnable`) or the `sealed` one. `(id hex, frames)`.
    pub fn frames_keep_piece(
        &mut self,
        address: &[u8; 32],
        burnable: bool,
        state: &[u8],
    ) -> Result<(String, Vec<Vec<u8>>), String> {
        let s = self.stream();
        let c = wire::block::block_contract(
            if burnable {
                &self.piece_code
            } else {
                &self.sealed_code
            },
            address,
        );
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

    /// The contract id (bytes) where a file piece at `address` is fetched: the `piece` contract's (`burnable`), else
    /// the `sealed` one's.
    pub fn piece_id(&self, address: &[u8; 32], burnable: bool) -> [u8; 32] {
        wire::block::contract_for(
            if burnable {
                &self.piece_code
            } else {
                &self.sealed_code
            },
            address,
        )
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
    pub fn tail_asset(&mut self, id: &[u8; 32]) -> Result<AssetOut, String> {
        match self.read(id, |o| o.asset())? {
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
                self.assets.insert(*id, groups);
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
        let o = self.tails.get(id).ok_or("that tail is not open")?;
        let Some((sealed, state)) = o.stored(cid, &groups) else {
            return Ok(None);
        };
        let c = if sealed {
            wire::block::block_contract(
                &self.sealed_code,
                &data::block_address(
                    &o.table_key.ok_or("this table's key is not held here")?,
                    cid,
                ),
            )
        } else {
            wire::block::block_contract(&self.block_code, cid)
        };
        let name = c.key().id().encode();
        let s = self.stream();
        Ok(Some((
            name,
            wire::frame_put(c, freenet_stdlib::prelude::WrappedState::new(state), s)?,
        )))
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
            return Err("its tree is sealed with a key this page does not hold".into());
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
            self.repairs.remove(lost_contract);
            self.wanted.remove(lost_contract);
        }
        Ok(held)
    }

    /// FLUSH an open table: `Ready` with the frames that PUT its new tree blocks (send and see them all accepted
    /// FIRST) and the step to sign; or `Need` with Block contract ids to GET first.
    pub fn tail_flush(&mut self, id: &[u8; 32]) -> Result<FlushOut, String> {
        let step = match self.read(id, |o| o.flush())? {
            Err(fetch) => return Ok(FlushOut::Need(fetch)),
            Ok(s) => s,
        };
        match step {
            data::Step::Need(_) => unreachable!(),
            data::Step::Keys(epochs) => Ok(FlushOut::Keys(epochs)),
            data::Step::Ready(f) => {
                let tk = if f.sealed {
                    Some(
                        self.tail(id)?
                            .table_key
                            .ok_or("this table's key is not held here")?,
                    )
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
                                let mut v = json!({ "kind": "tail-unreadable", "id": hex(&tail), "said": format!("block {} is not what was asked", hex(&cid)) });
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
                json!({ "kind": "got", "id": hex(&id) })
            }
            Incoming::HeadChanged {
                key,
                state: Some(state),
            } => {
                match freenet_stdlib::prelude::ContractInstanceId::from_base58(&key)
                    .ok()
                    .and_then(|i| self.tail_state(*i, &state))
                {
                    Some(v) => v,
                    None => json!({ "kind": "changed", "key": key }),
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

#[cfg(target_arch = "wasm32")]
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

pub fn unhex(h: &str) -> Option<Vec<u8>> {
    if h.len() % 2 != 0 {
        return None;
    }
    (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(h.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(target_arch = "wasm32")]
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
        ) -> Result<js_sys::Array, JsValue> {
            self.ask(Request::UpkeepCodes {
                bag: bag.to_vec(),
                tail: tail.to_vec(),
                idlog: *blake3::hash(idlog).as_bytes(),
            })
        }
        /// The codes' hash upkeep would hold for these (to hand them over only when they changed).
        pub fn upkeep_codes_hash(bag: &[u8], tail: &[u8], idlog: &[u8]) -> String {
            hex(&craftworks_identity::upkeep_codes_hash(
                bag,
                tail,
                blake3::hash(idlog).as_bytes(),
            ))
        }
        /// The MANDATE: `me` (the account's DID) and, per space, JSON `{ space, name, kind, owner, nonce, channel, open,
        /// codes: [[code, expires, left]], bans, members, epoch, state }` (ids and the state in hex).
        pub fn frames_upkeep_mandate(
            &mut self,
            me: &str,
            spaces: &str,
            spent: &str,
        ) -> Result<js_sys::Array, JsValue> {
            let v: Vec<serde_json::Value> =
                serde_json::from_str(spaces).map_err(|e| err(e.to_string()))?;
            let spaces = v
                .iter()
                .map(|m| -> Option<craftworks_identity::Mandate> {
                    let s = |k: &str| m.get(k).and_then(|x| x.as_str()).map(str::to_string);
                    let list = |k: &str| {
                        m.get(k).and_then(|x| x.as_array()).map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(str::to_string))
                                .collect::<Vec<_>>()
                        })
                    };
                    Some(craftworks_identity::Mandate {
                        space: unhex(&s("space")?)?.try_into().ok()?,
                        name: s("name").unwrap_or_default(),
                        kind: s("kind")?,
                        owner: s("owner")?,
                        nonce: s("nonce"),
                        channel: s("channel")?,
                        open: m.get("open").and_then(|x| x.as_bool()).unwrap_or(false),
                        codes: m
                            .get("codes")?
                            .as_array()?
                            .iter()
                            .filter_map(|c| {
                                Some((
                                    c.get(0)?.as_str()?.to_string(),
                                    c.get(1)?.as_f64()? as u64,
                                    c.get(2)?.as_f64()? as u32,
                                ))
                            })
                            .collect(),
                        bans: list("bans").unwrap_or_default(),
                        members: list("members")?,
                        epoch: m.get("epoch")?.as_f64()? as u64,
                        state: unhex(&s("state")?)?,
                    })
                })
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| err("a space of the mandate does not read".into()))?;
            let spent: Vec<[u8; 16]> = serde_json::from_str::<Vec<String>>(spent)
                .unwrap_or_default()
                .iter()
                .filter_map(|t| unhex(t)?.try_into().ok())
                .collect();
            self.ask(Request::UpkeepMandate {
                me: me.to_string(),
                spaces,
                spent,
            })
        }
        /// Admissions written as acts (JSON `[[space hex, did]]`): upkeep forgets them.
        pub fn frames_upkeep_ack(&mut self, admitted: &str) -> Result<js_sys::Array, JsValue> {
            let v: Vec<(String, String)> =
                serde_json::from_str(admitted).map_err(|e| err(e.to_string()))?;
            let admitted = v
                .into_iter()
                .filter_map(|(s, d)| Some((unhex(&s)?.try_into().ok()?, d)))
                .collect();
            self.ask(Request::UpkeepAck { admitted })
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
            let creds: Vec<Vec<u8>> = creds
                .iter()
                .map(|c| js_sys::Uint8Array::new(&c).to_vec())
                .collect();
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

        /// CHANGE THE RECOVERY WORDS from `old` to `new` (both entered on this page): the log with its two rotations and
        /// the new words' `whoami`, to PUT. (The account's nodes stay its MLS group's members: every owner key the log
        /// ever had signs for them.)
        pub fn change_words(
            &mut self,
            idlog_code: &[u8],
            register_code: &[u8],
            did: &[u8],
            old: &[u8],
            new: &[u8],
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
            Ok(o)
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

        /// MOVE (`data::Open::adopt`): the open tail `id` (new, empty) takes the open tail `from`'s state as its first
        /// step (a table moving to its blinded name): the step to sign, or null.
        pub fn tail_adopt(&mut self, id: &[u8], from: &[u8]) -> Result<JsValue, JsValue> {
            let (id, from) = (b32(id)?, b32(from)?);
            let f = self
                .0
                .tails
                .remove(&from)
                .ok_or_else(|| err("that tail is not open".into()))?;
            let out = self
                .0
                .tail(&id)
                .map(|o| o.adopt(&f).map(|(seq, hash)| (o.params.clone(), seq, hash)));
            self.0.tails.insert(from, f);
            match out.map_err(err)? {
                Some((params, seq, hash)) => Ok(prepared(&params, seq, hash)?.into()),
                None => Ok(JsValue::NULL),
            }
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

        /// FLUSH an open table. `{ need: [Block contract ids] }` to GET first; or `{ params, seq, valueHash, puts:
        /// [[name, frames]] }`: send every put and see each accepted, THEN sign and `tail_commit` as for a write.
        pub fn tail_flush(&mut self, id: &[u8]) -> Result<js_sys::Object, JsValue> {
            let id = b32(id)?;
            let out = js_sys::Object::new();
            match self.0.tail_flush(&id).map_err(err)? {
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
        /// contract hex]] }] }`.
        pub fn tail_asset(&mut self, id: &[u8]) -> Result<js_sys::Object, JsValue> {
            let id = b32(id)?;
            let out = js_sys::Object::new();
            match self.0.tail_asset(&id).map_err(err)? {
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

        /// What a frame from the node says, as JSON text.
        pub fn take(&mut self, bytes: &[u8]) -> String {
            self.0.take(bytes).to_string()
        }
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
    /// A burn hash from JS: 32 bytes (the `piece` contract), or none (empty: the `sealed` one, a file from before).
    fn burn_of(b: &[u8]) -> Result<Option<[u8; 32]>, JsValue> {
        if b.is_empty() {
            Ok(None)
        } else {
            b32(b).map(Some)
        }
    }

    fn pieces_js(
        core: &mut CraftworksCore,
        pieces: Vec<(u8, craftworks_files::Piece)>,
        burn: Option<[u8; 32]>,
    ) -> Result<js_sys::Array, JsValue> {
        let out = js_sys::Array::new();
        for (j, p) in pieces {
            let h = hex(&p.hash());
            let (id, f) = core.0.frames_piece(&p, burn.as_ref()).map_err(err)?;
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
        pub fn plain(&self) -> Result<js_sys::Uint8Array, JsValue> {
            Ok(js_sys::Uint8Array::from(
                &self.0.plain().map_err(file_err)?[..],
            ))
        }
    }

    #[wasm_bindgen]
    impl CraftworksCore {
        /// The state the node sent for a contract (hex id), taken: or null.
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
            pieces_js(self, pieces, burn_of(burn)?)
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
            Ok(pieces_js(self, vec![(j, p)], burn_of(burn)?)?.get(0).into())
        }
        /// The INDEX for the fragments stored (JSON `[[[j, hash hex]]]` per generation): `{ root, puts: [[id, frames]] }`,
        /// the root's put LAST. `hashed`: each index piece addressed by its hash (what a new upload writes).
        pub fn file_index(
            &mut self,
            key: &[u8],
            size: f64,
            stored: &str,
            burn: &[u8],
            hashed: bool,
        ) -> Result<js_sys::Object, JsValue> {
            let burn = burn_of(burn)?;
            let plan = craftworks_files::Plan::of(size as u64);
            let v: Vec<serde_json::Value> =
                serde_json::from_str(stored).map_err(|e| err(e.to_string()))?;
            let stored: Vec<craftworks_files::Listed> = v
                .iter()
                .map(|g| listed_of(&g.to_string()))
                .collect::<Result<_, _>>()?;
            let (pieces, root) = if hashed {
                craftworks_files::index_hashed(&b32(key)?, &plan, &stored)
            } else {
                craftworks_files::index(&b32(key)?, &plan, &stored)
            };
            let puts = js_sys::Array::new();
            for p in pieces {
                let (id, f) = self.0.frames_piece(&p, burn.as_ref()).map_err(err)?;
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
        /// Where a file's pieces are fetched (contract ids, hex): its root, an index piece, a fragment. `burnable`: a
        /// file with a burn hash (the `piece` contract). `hash` (hex): a hash-addressed file's index piece, by its hash
        /// (its parent's, the reference's for the root); empty: at its place.
        pub fn file_root_id(
            &self,
            key: &[u8],
            burnable: bool,
            hash: &str,
        ) -> Result<String, JsValue> {
            Ok(hex(&self.0.piece_id(
                &index_at(&b32(key)?, "root", 0, 0, hash)?,
                burnable,
            )))
        }
        pub fn file_index_id(
            &self,
            key: &[u8],
            level: u8,
            n: f64,
            burnable: bool,
            hash: &str,
        ) -> Result<String, JsValue> {
            Ok(hex(&self.0.piece_id(
                &index_at(&b32(key)?, "index", level as u64, n as u64, hash)?,
                burnable,
            )))
        }
        pub fn file_fragment_id(
            &self,
            key: &[u8],
            g: f64,
            j: u8,
            burnable: bool,
        ) -> Result<String, JsValue> {
            Ok(hex(&self.0.piece_id(
                &craftworks_files::fragment_address(&b32(key)?, g as u64, j),
                burnable,
            )))
        }
        /// KEEP a file's piece (`what` and `a`, `b` as `file_burn`): the state a GET gave, put again. `[id, frames]`.
        pub fn file_keep(
            &mut self,
            key: &[u8],
            what: &str,
            a: f64,
            b: f64,
            burnable: bool,
            state: &[u8],
            hash: &str,
        ) -> Result<js_sys::Array, JsValue> {
            let key = b32(key)?;
            let address = match what {
                "fragment" => craftworks_files::fragment_address(&key, a as u64, b as u8),
                _ => index_at(&key, what, a as u64, b as u64, hash)?,
            };
            let (id, f) = self
                .0
                .frames_keep_piece(&address, burnable, state)
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
                _ => index_at(&key, what, a as u64, b as u64, hash)?,
            };
            let (id, f) = self.0.frames_burn(&address, &b32(secret)?).map_err(err)?;
            Ok([JsValue::from(id), JsValue::from(frames(f))]
                .into_iter()
                .collect())
        }
    }

    /// Where a file's index piece is (`what`: "root", or "index" at `level`, `n`): by `hash` (hex) for a hash-addressed
    /// file, else at its place.
    fn index_at(
        key: &[u8; 32],
        what: &str,
        level: u64,
        n: u64,
        hash: &str,
    ) -> Result<[u8; 32], JsValue> {
        if !hash.is_empty() {
            return Ok(craftworks_files::hashed_address(
                key,
                &bytes32("index piece", hash).map_err(err)?,
            ));
        }
        match what {
            "root" => Ok(craftworks_files::root_address(key)),
            "index" => Ok(craftworks_files::index_address(key, level as u8, n)),
            _ => Err(err(format!("no such piece: {what}"))),
        }
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
    /// An inner index piece opened: its children's hashes.
    #[wasm_bindgen]
    pub fn file_inner(
        key: &[u8],
        level: u8,
        n: f64,
        hash: &str,
        state: &[u8],
    ) -> Result<Vec<String>, JsValue> {
        let h = bytes32("inner", hash).map_err(err)?;
        Ok(
            craftworks_files::open_inner(&b32(key)?, level, n as u64, &h, state)
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
        /// The actions a policy names.
        pub fn actions() -> Vec<String> {
            craftworks_gov::ACTIONS
                .iter()
                .map(|a| a.to_string())
                .collect()
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
        pub fn apps(&self) -> Vec<String> {
            craftworks_gov::APPS
                .iter()
                .filter(|a| self.0.apps.get(**a) == Some(&true))
                .map(|a| a.to_string())
                .collect()
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
