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
    pub idlog_hash: [u8; 32],
    pub bag_code: Vec<u8>,
    /// The beacon's code hash (its OUTPUTS LOG contracts are that code's: heights its state no longer keeps).
    pub beacon_hash: [u8; 32],
    pub beacon: [u8; 32],
    pub pool: [u8; 32],
    /// The standing contract's code hash (keepers' records: the statement's `kept`).
    pub standing_hash: [u8; 32],
}

pub struct Reader<'a> {
    net: &'a Net,
    rt: tokio::runtime::Runtime,
    beacon_state: RefCell<Option<B::State>>,
    defs: RefCell<HashMap<[u8; 32], Option<Definition>>>,
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
        self.defs.borrow_mut().entry(*t).or_insert_with(|| self.get(*t).and_then(|b| Definition::decode(&b))).clone()
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
    /// The stake CLAIMS of `epoch` (its claims registry).
    pub fn claims(&self, epoch: u64) -> Vec<StakeClaim> {
        let p = T::claims::Params { code_hash: self.net.code_hash, pool: self.net.pool, beacon: self.net.beacon, epoch };
        self.get(p.id()).and_then(|b| T::claims::Registry::decode(&b)).map_or(vec![], |r| r.claims)
    }
    pub fn sigs(&self, chain: &[u8; 32], seq: u64) -> Vec<witness::WitnessSig> {
        self.bag(wallet::witness_bag(chain, seq)).iter().filter_map(|p| wallet::bytes::read_witness_sig(p)).collect()
    }
    /// A staker's real stake in the pool at height `at`.
    pub fn stake_of(&self, owner: &[u8; 32], at: u64) -> Option<witness::Stake> {
        let tok = Tok { definition: self.net.pool, item: [0; 32] };
        let p = self.params(&tok, owner)?;
        let d = self.definition(&self.net.pool)?;
        wallet::stake_at(&p, &d, &self.chain(&tok, owner)?, at).ok()
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
            let n = items.len();
            out.extend(items);
            if n < craftworks_bag_contract::chained::ROOM {
                break;
            }
        }
        out
    }
    /// The WITNESS QUEUE of `epoch`.
    pub fn queue(&self, epoch: u64) -> Vec<Queued> {
        self.chained(craftworks_wallet::queue::address, epoch).iter().filter_map(|p| Queued::decode(p)).collect()
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
        let sigs = |c: &[u8; 32], sq: u64| self.sigs(c, sq);
        use craftworks_reputation::statement::{bag_address, lot_height, witness_place, Posted};
        let statements: Vec<craftworks_wallet::fees::Statement> = self.bag(bag_address(epoch)).iter().filter_map(|p| Posted::decode(p)).filter(|p| p.epoch == epoch).map(|p| craftworks_wallet::fees::Statement { place: witness_place(epoch), root: p.root, h: lot_height(epoch) }).collect();
        craftworks_wallet::fees::count(&queued, &statements, &craftworks_wallet::fees::Reads { pool: self.net.pool, entry: &entry, claims: &claims, out: &out, sigs: &sigs })
    }
    /// The accounts that KEPT in `epoch`: its keepers registry's keepers whose standing record proves it.
    pub fn kept(&self, epoch: u64, log_of: &dyn Fn(&[u8; 32]) -> Option<craftworks_idlog_contract::Log>) -> Vec<[u8; 32]> {
        let items = self.chained(craftworks_reputation::keepers::address, epoch);
        let listed = craftworks_reputation::keepers::listed(&items, log_of);
        let record_of = |key: &[u8; 32]| {
            let id = craftworks_standing_contract::Params::keeper(self.net.beacon_hash, self.net.beacon, key).id(&self.net.standing_hash);
            self.get(id).filter(|b| !b.is_empty()).and_then(|b| craftworks_standing::codec::decode(&b))
        };
        craftworks_standing::record::kept_accounts(&listed, &record_of, epoch)
    }

    /// The EPOCH STATEMENT of standing epoch `epoch` as this reader computes it (the one build: every vouch registry up
    /// to it, each item verified by its voucher's key log): its root.
    pub fn statement_root(&self, epoch: u64) -> Option<[u8; 32]> {
        use craftworks_reputation as R;
        let items: Vec<R::item::Item> = (0..=epoch).flat_map(|e| self.bag(R::item::registry_address(e))).filter_map(|b| R::item::Item::decode(&b)).collect();
        let log_of = |d: &R::Did| self.keys(d);
        let signed = epoch.checked_sub(1).map_or(Default::default(), |e| self.signed(e));
        let pots = R::statement::Pots { keeper: craftworks_wallet::craft::keeper_pot(epoch), allowance: (craftworks_allowance_contract::POT, craftworks_allowance_contract::CAP), witness: craftworks_wallet::craft::witness_pot(epoch) };
        let kept = epoch.checked_sub(1).map_or(Vec::new(), |e| self.kept(e, &log_of));
        let st = R::statement::build(epoch, &items, &log_of, &kept, &signed, pots);
        st.tree(&mut std::collections::BTreeMap::new()).map(|(root, _)| root)
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
        let sig = witness::WitnessSig { chain: place, seq: 0, entry: mine, sig: key.sign(&witness::WitnessSig::message(&place, 0, &mine)).to_bytes() };
        self.drop(wallet::witness_bag(&place, 0), &wallet::bytes::witness_sig(&sig)).map_err(|e| e.to_string())?;
        Ok(Some(mine))
    }

    /// WITNESS one queued entry with `key`: `Ok(Some(sig))` signed (and dropped into its witness bag), `Ok(None)` not
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
        let sigs_of = |c: &[u8; 32], sq: u64| self.sigs(c, sq);
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
        let sig = witness::WitnessSig { chain: p.id(), seq: q.seq, entry: hash, sig: key.sign(&witness::WitnessSig::message(&p.id(), q.seq, &hash)).to_bytes() };
        self.drop(wallet::witness_bag(&p.id(), q.seq), &wallet::bytes::witness_sig(&sig)).map_err(|e| e.to_string())?;
        Ok(Some(sig))
    }
}

fn hex6(b: &[u8; 32]) -> String {
    b[..6].iter().map(|x| format!("{x:02x}")).collect()
}

fn blake3_hash(b: &[u8]) -> [u8; 32] {
    *blake3::hash(b).as_bytes()
}
