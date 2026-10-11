//! THE TOOLS' NETWORK VIEW OF THE TOKEN (TOKEN §7a): everything a WITNESS (or a wallet checking acceptance) reads, from
//! a node, through node-io's one door — and [`Reader::witness`], one queued entry witnessed: its lot drawn, this key's
//! seat checked, the entry re-checked by induction (`wallet::may_witness`), signed. Reads block (a runtime of the
//! reader's own): use it off the async runtime (`spawn_blocking`).

use craftworks_bag_contract as bag;
use craftworks_beacon_contract as B;
use craftworks_token_contract as T;
use craftworks_wallet::queue::Queued;
use ed25519_dalek::{Signer, SigningKey};
use std::cell::RefCell;
use std::collections::HashMap;
use T::{draw::StakeClaim, wallet, witness, Definition, State, Tok};

/// Where and what: the node, the token's code, the key log's and bag's codes, the beacon, the stake pool's token.
pub struct Net {
    pub ws: String,
    pub code_hash: [u8; 32],
    /// The token's code (a witness PUTs its signature into its place's container: `witness::Params`).
    pub code: Vec<u8>,
    pub idlog_hash: [u8; 32],
    pub bag_code: Vec<u8>,
    /// The beacon's code hash (its OUTPUTS LOG contracts are that code's: heights its state no longer keeps).
    pub beacon_hash: [u8; 32],
    pub beacon: [u8; 32],
    pub pool: [u8; 32],
    /// The standing contract's code hash (keepers' records: the statement's `kept`).
    pub standing_hash: [u8; 32],
    /// The Block contract's code (a statement's tree: the previous statement's lines).
    pub block_code: Vec<u8>,
}

pub struct Reader<'a> {
    net: &'a Net,
    rt: tokio::runtime::Runtime,
    beacon_state: RefCell<Option<B::State>>,
    defs: RefCell<HashMap<[u8; 32], Definition>>,
}

impl<'a> Reader<'a> {
    pub fn new(net: &'a Net) -> anyhow::Result<Reader<'a>> {
        Ok(Reader { net, rt: tokio::runtime::Builder::new_current_thread().enable_all().build()?, beacon_state: RefCell::new(None), defs: RefCell::new(HashMap::new()) })
    }
    /// A contract's state (`None`: not there, or not answered).
    pub fn get(&self, id: [u8; 32]) -> Option<Vec<u8>> {
        self.rt.block_on(node_io::get(&self.net.ws, id)).ok().flatten()
    }
    /// A bag's items' PAYLOADS.
    pub fn bag(&self, addr: [u8; 32]) -> Vec<Vec<u8>> {
        let id = T::id(&blake3_hash(&self.net.bag_code), &addr);
        self.get(id).and_then(|s| bag::read(&addr, &s)).map(|items| items.iter().map(|i| bag::payload(i).to_vec()).collect()).unwrap_or_default()
    }
    /// DROP an item into a bag (its work ground here).
    pub fn drop(&self, addr: [u8; 32], payload: &[u8]) -> anyhow::Result<String> {
        let item = bag::grind(&addr, payload);
        Ok(self.rt.block_on(node_io::put(&self.net.ws, node_io::container(&self.net.bag_code, addr.to_vec()), bag::encode(&addr, &[item])))?.0)
    }
    fn beacon(&self, fresh: bool) -> Option<B::State> {
        if fresh || self.beacon_state.borrow().is_none() {
            *self.beacon_state.borrow_mut() = self.get(self.net.beacon).and_then(|b| B::wire::decode(&b));
        }
        self.beacon_state.borrow().clone()
    }
    /// The beacon's height now.
    pub fn height(&self) -> u64 {
        self.beacon(true).map_or(0, |s| s.height())
    }
    /// The beacon's OUTPUT at height `h`: from its state (read again once when not there), else its OUTPUTS LOG — a
    /// height its state no longer keeps (it keeps its newest steps only).
    pub fn out_at(&self, h: u64) -> Option<[u8; 32]> {
        let find = |s: &B::State| s.steps.iter().find(|st| st.height == h).map(|st| st.out);
        self.beacon(false).as_ref().and_then(find).or_else(|| self.beacon(true).as_ref().and_then(find)).or_else(|| {
            let lp = B::log::Params::of(self.net.beacon, h);
            let log = self.get(T::id(&self.net.beacon_hash, &lp.encode())).and_then(|b| B::log::Log::decode(&b))?;
            log.entries.iter().find(|(x, _)| *x == h).map(|(_, o)| *o)
        })
    }
    pub fn definition(&self, t: &[u8; 32]) -> Option<Definition> {
        // Only a definition READ is kept (it never changes); one not there yet (a token whose first entry makes it — $CRAFT
        // by its first keeper mint) is asked again next time — a kept "not there" refused every entry of it for good.
        if let Some(d) = self.defs.borrow().get(t) {
            return Some(d.clone());
        }
        let d = self.get(*t).and_then(|b| Definition::decode(&b))?;
        self.defs.borrow_mut().insert(*t, d.clone());
        Some(d)
    }
    /// A chain's params (the token's own beacon and key log).
    pub fn params(&self, t: &Tok, owner: &[u8; 32]) -> Option<T::ChainParams> {
        let d = self.definition(&t.definition)?;
        Some(T::ChainParams { code_hash: self.net.code_hash, definition: t.definition, owner: *owner, beacon: d.beacon, idlog: d.idlog, item: t.item })
    }
    pub fn chain(&self, t: &Tok, owner: &[u8; 32]) -> Option<State> {
        self.get(self.params(t, owner)?.id()).and_then(|b| State::decode(&b))
    }
    /// An account's KEY LOG (`None`: a set-owned chain's owner has none).
    pub fn keys(&self, did: &[u8; 32]) -> Option<craftworks_idlog_contract::Log> {
        self.get(T::id(&self.net.idlog_hash, did)).and_then(|b| craftworks_idlog_contract::read(did, &b))
    }
    /// The stake claims of `epoch` as its registry holds them (every one, backed or not).
    pub fn raw_claims(&self, epoch: u64) -> Vec<StakeClaim> {
        let p = T::claims::Params { code_hash: self.net.code_hash, pool: self.net.pool, beacon: self.net.beacon, epoch };
        self.get(p.id()).and_then(|b| T::claims::Registry::decode(&b)).map_or(vec![], |r| r.claims)
    }
    /// The VALID STAKERS of `epoch` (`wallet::valid_stakers`, TOKEN §7a: witnessed stake — what every lot is drawn over).
    pub fn claims(&self, epoch: u64) -> Vec<StakeClaim> {
        let Some(pd) = self.definition(&self.net.pool) else { return Vec::new() };
        let tok = Tok { definition: self.net.pool, item: [0; 32] };
        let registry = |e: u64| self.raw_claims(e);
        let chain = |o: &[u8; 32]| Some((self.params(&tok, o)?, self.chain(&tok, o)?));
        let out = |h: u64| self.out_at(h);
        let sigs = |c: &[u8; 32], q: u64, ep: u64| self.sigs(c, q, ep);
        let r = wallet::StakeReads { pool: self.net.pool, pool_def: &pd, registry: &registry, chain: &chain, out: &out, sigs: &sigs };
        wallet::valid_stakers(&r, epoch, &Default::default())
    }
    /// A PLACE'S SIGNATURES container under claims epoch `epoch` (`witness::Params`: its registry's keys may sign).
    pub fn place(&self, chain: &[u8; 32], seq: u64, epoch: u64) -> witness::Params {
        witness::Params { code_hash: self.net.code_hash, pool: self.net.pool, beacon: self.net.beacon, epoch, chain: *chain, seq }
    }
    pub fn sigs(&self, chain: &[u8; 32], seq: u64, epoch: u64) -> Vec<witness::WitnessSig> {
        let p = self.place(chain, seq, epoch);
        self.get(p.id()).map(|b| p.sigs(&b)).unwrap_or_default()
    }
    /// PUT `key`'s signature `sig` into its place's container `p` (made if not there; else merged).
    fn put_sig(&self, p: &witness::Params, key: &[u8; 32], sig: &witness::WitnessSig) -> anyhow::Result<String> {
        Ok(self.rt.block_on(node_io::put(&self.net.ws, node_io::container(&self.net.code, p.encode()), p.state(&[(*key, sig.clone())])))?.0)
    }
    /// A staker's stake as its VALID claim stands at height `at` (its claims epoch's).
    pub fn stake_of(&self, owner: &[u8; 32], at: u64) -> Option<witness::Stake> {
        wallet::stake_from(&self.claims(at / T::supply::EPOCH), owner)
    }
    pub fn freeze(&self, def: &[u8; 32]) -> Option<T::freeze::List> {
        let d = self.definition(def)?;
        let p = T::freeze::Params { code_hash: self.net.code_hash, definition: *def, beacon: d.beacon };
        self.get(p.id()).and_then(|b| T::freeze::List::decode(&b))
    }
    /// Every item of `epoch`'s CHAINED list `address_of` (`craftworks_bag_contract::chained`: k = 0 until one with room).
    pub fn chained(&self, address_of: fn(u64, u32) -> [u8; 32], epoch: u64) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for k in 0..64 {
            let items = self.bag(address_of(epoch, k));
            let full = craftworks_bag_contract::chained::full(items.iter().map(Vec::len));
            out.extend(items);
            if !full {
                break;
            }
        }
        out
    }
    /// The WITNESS QUEUE of `epoch`.
    pub fn queue(&self, epoch: u64) -> Vec<Queued> {
        self.chained(craftworks_wallet::queue::address, epoch).iter().filter_map(|p| Queued::decode(p)).collect()
    }
    /// The LINES of standing epoch `epoch`'s ACCEPTED statement (`wallet::fees::statement_accepted`), read from its
    /// public blocks — `None`: none accepted, or not readable.
    pub fn statement_lines(&self, epoch: u64) -> Option<std::collections::BTreeMap<[u8; 32], craftworks_reputation::statement::Line>> {
        use craftworks_reputation::statement::{bag_address, lines, lot_height, witness_place, Posted};
        let claims = |ep: u64| self.claims(ep);
        let out = |h: u64| self.out_at(h);
        let sigs = |c: &[u8; 32], sq: u64, ep: u64| self.sigs(c, sq, ep);
        let entry = |_: &Queued| None;
        let r = craftworks_wallet::fees::Reads { pool: self.net.pool, entry: &entry, claims: &claims, out: &out, sigs: &sigs };
        let mut sts: Vec<craftworks_wallet::fees::Statement> = self.bag(bag_address(epoch)).iter().filter_map(|p| Posted::decode(p)).filter(|p| p.epoch == epoch).map(|p| craftworks_wallet::fees::Statement { place: witness_place(epoch), root: p.root, h: lot_height(epoch) }).collect();
        sts.sort_by_key(|s| s.root);
        let st = sts.into_iter().find(|s| craftworks_wallet::fees::statement_accepted(&r, s))?;
        let block = |c: &[u8; 32]| self.get(contract_keys::block::contract_for(&self.net.block_code, c)).filter(|s| s.len() > 1).map(|s| s[1..].to_vec());
        lines(&st.root, &block).ok()
    }
    /// The WITNESS SIGNATURES each claimant gave on standing epoch `epoch`'s queued entries (`wallet::fees::count`).
    pub fn signed(&self, epoch: u64) -> std::collections::BTreeMap<[u8; 32], u64> {
        let queued = self.queue(epoch);
        let entry = |q: &Queued| {
            let d = self.definition(&q.token.definition)?;
            let p = self.params(&q.token, &q.owner)?;
            let s = self.chain(&q.token, &q.owner)?;
            let i = s.entries.iter().position(|e| e.seq == q.seq)?;
            Some((p.id(), s.entries[i].clone(), i.checked_sub(1).map(|j| s.entries[j].clone()), d.margin))
        };
        let claims = |ep: u64| self.claims(ep);
        let out = |h: u64| self.out_at(h);
        let sigs = |c: &[u8; 32], sq: u64, ep: u64| self.sigs(c, sq, ep);
        use craftworks_reputation::statement::{bag_address, lot_height, witness_place, Posted};
        let statements: Vec<craftworks_wallet::fees::Statement> = self.bag(bag_address(epoch)).iter().filter_map(|p| Posted::decode(p)).filter(|p| p.epoch == epoch).map(|p| craftworks_wallet::fees::Statement { place: witness_place(epoch), root: p.root, h: lot_height(epoch) }).collect();
        craftworks_wallet::fees::count(&queued, &statements, &craftworks_wallet::fees::Reads { pool: self.net.pool, entry: &entry, claims: &claims, out: &out, sigs: &sigs })
    }
    /// Each keeper LISTED in `epoch`'s registry, its record's key, and its record's answered heights per epoch
    /// (`None`: no record on the network) — what its node (page or delegate) has proved.
    pub fn keepers(&self, epoch: u64) -> Vec<([u8; 32], [u8; 32], Option<Vec<(u64, Vec<u64>)>>)> {
        let log_of = |d: &[u8; 32]| self.keys(d);
        let items = self.chained(craftworks_reputation::keepers::address, epoch);
        craftworks_reputation::keepers::listed(&items, &log_of)
            .into_iter()
            .map(|(did, key)| {
                let id = craftworks_standing_contract::Params::keeper(self.net.beacon_hash, self.net.beacon, &key).id(&self.net.standing_hash);
                let rec = self.get(id).filter(|b| !b.is_empty()).and_then(|b| craftworks_standing::codec::decode(&b));
                (did, key, rec.map(|r| r.epochs.iter().map(|e| (e.epoch, e.answers.iter().filter(|a| a.key == key).map(|a| a.height).collect())).collect()))
            })
            .collect()
    }
    /// The accounts that KEPT in `epoch`: its keepers registry's keepers whose standing record proves it AND passes
    /// its AUDITS (`standing::record::kept_audited`: drawn by the beacon's output at the lot of the statement that
    /// pays `epoch` — the next epoch's; each drawn pick's piece read here and re-sealed). `None`: that lot not drawn
    /// yet. A piece this reader cannot get (not on the network, or not answered) fails its keeper's audit.
    pub fn kept(&self, epoch: u64, log_of: &dyn Fn(&[u8; 32]) -> Option<craftworks_idlog_contract::Log>) -> Option<Vec<[u8; 32]>> {
        let items = self.chained(craftworks_reputation::keepers::address, epoch);
        let listed = craftworks_reputation::keepers::listed(&items, log_of);
        if listed.is_empty() {
            return Some(Vec::new());
        }
        let lot_out = self.out_at(craftworks_reputation::statement::lot_height(epoch + 1))?;
        let records: HashMap<[u8; 32], Option<craftworks_standing::record::State>> = listed
            .iter()
            .map(|(_, key)| {
                let id = craftworks_standing_contract::Params::keeper(self.net.beacon_hash, self.net.beacon, key).id(&self.net.standing_hash);
                (*key, self.get(id).filter(|b| !b.is_empty()).and_then(|b| craftworks_standing::codec::decode(&b)))
            })
            .collect();
        let record_of = |key: &[u8; 32]| records.get(key).cloned().flatten();
        let out = |h: u64| self.out_at(h);
        let piece_of = |id: &[u8; 32]| self.get(*id).filter(|b| !b.is_empty());
        Some(craftworks_standing::record::kept_audited(&listed, &record_of, epoch, &lot_out, &out, &piece_of))
    }

    /// The EPOCH STATEMENT of standing epoch `epoch` as this reader computes it (the one build: every vouch registry up
    /// to it, each item verified by its voucher's key log; KEPT audited: `kept`): its root. `None`: its lot not drawn yet.
    pub fn statement_root(&self, epoch: u64) -> Option<[u8; 32]> {
        // THE ONE BUILD (`rules::statement::build`, the page's and the delegate's — 10-11: this tool kept its own and
        // its epoch-20 root differed from the pages'), over what this reader reads: a contract not answered is taken as
        // not there (this reader cannot tell them apart; a page refuses to post then).
        use craftworks_reputation as R;
        use craftworks_reputation::statement::{bag_address, Posted, CARRY_BACK};
        let items: Vec<R::item::Item> = (0..=epoch).flat_map(|e| self.chained(R::item::registry_address, e)).filter_map(|b| R::item::Item::decode(&b)).collect();
        let keepers: Vec<Vec<u8>> = epoch.checked_sub(1).map_or(vec![], |e| self.chained(R::keepers::address, e));
        let misdeeds: Vec<(u64, Vec<u8>)> = (0..epoch).flat_map(|e| self.chained(craftworks_rules::misdeed::registry_address, e).into_iter().map(move |b| (e, b))).collect();
        let queued = epoch.checked_sub(1).map_or(vec![], |e| self.queue(e));
        let roots = |x: u64| self.bag(bag_address(x)).iter().filter_map(|p| Posted::decode(p)).filter(|p| p.epoch == x).map(|p| p.root).collect::<Vec<_>>();
        let posted = epoch.checked_sub(1).map_or(vec![], roots);
        let earlier: Vec<(u64, [u8; 32])> = if epoch >= 2 { (epoch.saturating_sub(CARRY_BACK)..=epoch - 2).rev().flat_map(|x| roots(x).into_iter().map(move |r| (x, r))).collect() } else { vec![] };
        let ctx = craftworks_rules::tokens::Ctx { code_hash: self.net.code_hash, beacon: self.net.beacon, idlog: self.net.idlog_hash };
        let lot = craftworks_rules::tokens::Lot { pool: self.net.pool, bag: blake3_hash(&self.net.bag_code), beacon_code: self.net.beacon_hash };
        let keep = craftworks_rules::keeper::Keep { standing: self.net.standing_hash, beacon_code: self.net.beacon_hash, beacon: self.net.beacon };
        let fresh = |_: &[u8; 32]| true;
        let block = |c: &[u8; 32]| self.get(contract_keys::block::contract_for(&self.net.block_code, c)).filter(|s| s.len() > 1).map(|s| s[1..].to_vec());
        let lines = |root: &[u8; 32]| R::statement::lines(root, &block).map_err(|_| Vec::new());
        let inputs = craftworks_rules::statement::Inputs { epoch, items: &items, misdeeds: &misdeeds, keepers: &keepers, queued: &queued, posted: &posted, earlier: &earlier, ctx: &ctx, lot: &lot, keep: &keep, fresh: &fresh, lines: &lines, audit: None };
        let got = |x: &[u8; 32]| Some(self.get(*x).unwrap_or_default());
        match craftworks_rules::statement::build(&inputs, &got).ok()? {
            craftworks_rules::tokens::Step::Done((st, _)) => st.tree(&mut std::collections::BTreeMap::new()).map(|(root, _)| root),
            _ => None,
        }
    }

    /// WITNESS the epoch statement of `epoch` with `key`: `Ok(Some(root))` signed — the root POSTED in its bag that this
    /// reader computes too —, `Ok(None)` not seated (or its lot not drawn, or that root not posted yet), `Err(why)`.
    pub fn witness_statement(&self, epoch: u64, key: &SigningKey) -> Result<Option<[u8; 32]>, String> {
        use craftworks_reputation::statement::{bag_address, lot_height, witness_place, Posted};
        let me = key.verifying_key().to_bytes();
        let h = lot_height(epoch);
        let Some(out) = self.out_at(h) else { return Ok(None) };
        let ce = h / T::supply::EPOCH;
        let claims = self.claims(ce);
        let valid = T::draw::valid(&self.net.pool, ce, &claims);
        let (k, base, _) = witness::seats(valid.len(), wallet::COMMITTEE);
        let place = witness_place(epoch);
        let members = T::draw::committee_of(&out, &place, 0, &valid, k);
        if !(members.iter().any(|m| claims[*m].key == me) || (base && me == witness::BASELINE)) {
            return Ok(None);
        }
        let posted: Vec<[u8; 32]> = self.bag(bag_address(epoch)).iter().filter_map(|b| Posted::decode(b)).filter(|p| p.epoch == epoch).map(|p| p.root).collect();
        let mine = self.statement_root(epoch).ok_or("the statement's tree does not build")?;
        if !posted.contains(&mine) {
            return if posted.is_empty() { Ok(None) } else { Err(format!("posted root(s) {} — not the one computed here ({})", posted.iter().map(|r| hex6(r)).collect::<Vec<_>>().join(", "), hex6(&mine))) };
        }
        let now = self.height();
        let sig = witness::WitnessSig { chain: place, seq: 0, entry: mine, at: now, sig: key.sign(&witness::WitnessSig::message(&place, 0, &mine, now)).to_bytes() };
        self.put_sig(&self.place(&place, 0, ce), &me, &sig).map_err(|e| e.to_string())?;
        Ok(Some(mine))
    }

    /// THE MISDEED WATCH (TOKEN §7d decision 3): the EQUIVOCATIONS among the witness signatures on standing epoch
    /// `epoch`'s queued entries and on its statement (`wallet::equivocations`, the token's own find), each as a
    /// self-proving registry item (`rules::misdeed::Equivocation`): the staker's credential for the key read from its
    /// pool chain's writer entries, the proof checked against its key log (one that does not hold is left out).
    pub fn misdeeds(&self, epoch: u64) -> Vec<craftworks_rules::misdeed::Equivocation> {
        use craftworks_reputation::statement::{lot_height, witness_place};
        let mut places: Vec<([u8; 32], u64, u64)> = Vec::new();
        for q in self.queue(epoch) {
            let (Some(d), Some(p), Some(s)) = (self.definition(&q.token.definition), self.params(&q.token, &q.owner), self.chain(&q.token, &q.owner)) else { continue };
            let Some(i) = s.entries.iter().position(|e| e.seq == q.seq) else { continue };
            let Ok((ce, _)) = wallet::lot_of(&s.entries[i], i.checked_sub(1).map(|j| &s.entries[j]), d.margin) else { continue };
            places.push((p.id(), q.seq, ce));
        }
        places.push((witness_place(epoch), 0, lot_height(epoch) / T::supply::EPOCH));
        places.sort();
        places.dedup();
        let pool = Tok { definition: self.net.pool, item: [0; 32] };
        let mut out: Vec<craftworks_rules::misdeed::Equivocation> = Vec::new();
        for (chain, seq, ce) in places {
            for (owner, body) in wallet::equivocations(&self.sigs(&chain, seq, ce), &self.raw_claims(ce)) {
                let T::Body::Slash { key, a, b } = body else { continue };
                if out.iter().any(|x| x.did == owner) {
                    continue;
                }
                let (Some(log), Some(st)) = (self.keys(&owner), self.chain(&pool, &owner)) else { continue };
                let found = st.entries.iter().rev().find_map(|e| match &e.body {
                    T::Body::Writer { cred, key_seq } => {
                        let x = craftworks_rules::misdeed::Equivocation { did: owner, cred: cred.clone(), key_seq: *key_seq, key, a: a.clone(), b: b.clone() };
                        x.holds(&log).then_some(x)
                    }
                    _ => None,
                });
                out.extend(found);
            }
        }
        out
    }

    /// POST `payload` into `epoch`'s CHAINED list `address_of` — into the first bag with room — unless it is there
    /// already: `Ok(None)` there already, `Ok(Some(answer))` the node's answer to the put.
    pub fn post_chained(&self, address_of: fn(u64, u32) -> [u8; 32], epoch: u64, payload: &[u8]) -> anyhow::Result<Option<String>> {
        for k in 0..64 {
            let addr = address_of(epoch, k);
            let items = self.bag(addr);
            if items.iter().any(|i| i == payload) {
                return Ok(None);
            }
            // (A bag not FULL takes it — readers go on past a full one only: `chained::full`.)
            if !craftworks_bag_contract::chained::full(items.iter().map(Vec::len)) {
                return self.drop(addr, payload).map(Some);
            }
        }
        anyhow::bail!("every bag of epoch {epoch} is full")
    }

    /// WITNESS one queued entry with `key`: `Ok(Some(sig))` signed (and put into its place's container), `Ok(None)` not
    /// seated in its committee (or its lot not drawn yet), `Err(why)` refused by the rule.
    pub fn witness(&self, q: &Queued, key: &SigningKey) -> Result<Option<witness::WitnessSig>, String> {
        let me = key.verifying_key().to_bytes();
        let d = self.definition(&q.token.definition).ok_or("the token's definition not read")?;
        let p = self.params(&q.token, &q.owner).ok_or("no chain params")?;
        let s = self.chain(&q.token, &q.owner).ok_or("the chain not read")?;
        let i = s.entries.iter().position(|e| e.seq == q.seq).ok_or("the entry not held")?;
        let e = &s.entries[i];
        let (epoch, h) = wallet::lot_of(e, i.checked_sub(1).map(|j| &s.entries[j]), d.margin).map_err(String::from)?;
        let Some(out) = self.out_at(h) else { return Ok(None) };
        let claims = self.claims(epoch);
        let valid = T::draw::valid(&self.net.pool, epoch, &claims);
        let (k, base, _) = witness::seats(valid.len(), wallet::COMMITTEE);
        let members = T::draw::committee_of(&out, &p.id(), q.seq, &valid, k);
        if !(members.iter().any(|m| claims[*m].key == me) || (base && me == witness::BASELINE)) {
            return Ok(None);
        }
        let claims_of = |ep: u64| self.claims(ep);
        let sigs_of = |c: &[u8; 32], sq: u64, ep: u64| self.sigs(c, sq, ep);
        let out_of = |hh: u64| self.out_at(hh);
        let stake = |o: &[u8; 32], at: u64| self.stake_of(o, at);
        let defs = |t: &[u8; 32]| self.definition(t);
        let program = |id: &[u8; 32]| self.get(*id);
        let freeze = |t: &[u8; 32]| self.freeze(t);
        let keys = |o: &[u8; 32]| self.keys(o);
        let src = wallet::Source { pool: self.net.pool, claims: &claims_of, sigs: &sigs_of, out: &out_of, stake_of: &stake, definition: &defs, program: &program, freeze: &freeze, keys: &keys, history: &|_, _| None };
        let cited = |t: &Tok, o: &[u8; 32]| self.chain(t, o);
        let supply = |k: u64| self.get(T::supply::Params { code_hash: self.net.code_hash, definition: q.token.definition, beacon: d.beacon, epoch: k }.id()).and_then(|b| T::supply::Shard::decode(&b));
        let owner_keys = self.keys(&q.owner);
        let holds = |st: &State| T::check(&p, &d, st, &cited, &supply, owner_keys.as_ref(), &T::Extra { ops: None, program: Some(&program) });
        let now = self.height();
        let hash = wallet::may_witness(&src, &p, &d, &s, q.seq, now, &holds, &cited).map_err(String::from)?;
        let sig = witness::WitnessSig { chain: p.id(), seq: q.seq, entry: hash, at: now, sig: key.sign(&witness::WitnessSig::message(&p.id(), q.seq, &hash, now)).to_bytes() };
        self.put_sig(&self.place(&p.id(), q.seq, epoch), &me, &sig).map_err(|e| e.to_string())?;
        Ok(Some(sig))
    }
}

fn hex6(b: &[u8; 32]) -> String {
    b[..6].iter().map(|x| format!("{x:02x}")).collect()
}

fn blake3_hash(b: &[u8]) -> [u8; 32] {
    *blake3::hash(b).as_bytes()
}
