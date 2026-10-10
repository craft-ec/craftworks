//! TOKEN (docs/TOKEN.md): the standard's DEFINITION and ACCOUNT CHAIN — one code, two kinds by their params. Step 3,
//! the core: `genesis`, `send`, `receive`, `burn`; carried balances; the merge; strict equivocation; work on appends by
//! anyone. (Allowances, offers, locks, ids, the beacon's times: later steps.)
//!
//! An id is Freenet's: `blake3(code hash ‖ params)`, the code hash being `blake3(wasm)`. Every params names the code
//! hash, so a chain recomputes the id of every chain it cites (TOKEN §3a): a look-alike contract's "send" is not a send.
//!
//! WHAT THE CONTRACT PROVES (TOKEN §3a): a chain is WELL-FORMED — linked, signed, its carried balance right, no debit
//! over it, no receive twice — and the receives among its last [`RECENT`] entries are checked against their senders'
//! chains (with the definition: at most 10 related contracts, Freenet's limit). It does not prove an older receive:
//! value is checked by whoever receives it (wallets, checkpoints). Not yet: the received-set ACCUMULATOR (needed only
//! once old entries are pruned — this core keeps every entry and scans them).

use ed25519_dalek::{Signature, VerifyingKey};

pub const DEFINITION: u8 = 1;
pub const CHAIN: u8 = 2;
/// The citations among this many newest entries are checked against the chains they cite — a claim cites two (its
/// lock's and its fill's), so 4 × 2 + the definition stays within Freenet's 10 related contracts.
pub const RECENT: usize = 4;
/// Work on an entry anyone may append (a receive): this many leading zero bits of `blake3(message ‖ work)`.
pub const WORK_BITS: u32 = 16;
/// A token's name, at most (its length is one byte on the wire).
pub const NAME_MAX: usize = 64;
/// Beacon steps an unstaked amount waits before its release — slashable all the while (a witness's equivocation is
/// found and proved within it). TWO supply epochs: stake is counted for a lot as held at its epoch's START
/// (`wallet::stake_at`), so whatever was counted stays burnable for at least a full epoch after that epoch ends.
pub const UNBOND: u64 = 2 * supply::EPOCH;
/// What a chain's check reads beyond its chains, shard and key log: its owner's OPERATORS list (`operators`), and a
/// PROGRAM's state by its contract id (an escrow's release is checked against its verdict: `escrow`). `Extra::NONE`:
/// neither.
#[derive(Clone, Copy, Default)]
pub struct Extra<'a> {
    pub ops: Option<&'a operators::List>,
    pub program: Option<&'a dyn Fn(&[u8; 32]) -> Option<Vec<u8>>>,
}
impl Extra<'_> {
    pub const NONE: Extra<'static> = Extra { ops: None, program: None };
}

/// The longest entry a misdeed carries.
pub const MISDEED_MAX: usize = 16 * 1024;
/// The most steps one vesting schedule has (TOKEN §4a).
pub const VEST_STEPS: usize = 64;
/// Writer keys a chain retires while it holds stake, at most — each stays burnable ([`Body::Slash`]) until the stake is
/// all released; past the cap, a new writer waits until then.
pub const RETIRED_MAX: usize = 16;

/// A contract's instance id (Freenet's: the code's hash and the params) — `contract_keys::instance`, its one home.
pub fn id(code_hash: &[u8; 32], params: &[u8]) -> [u8; 32] {
    contract_keys::instance(code_hash, params)
}

// ---- reading bytes, exactly ----
pub(crate) struct R<'a>(pub(crate) &'a [u8]);
impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Some(a)
    }
    fn arr<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.take(N)?.try_into().ok()
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.arr()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.arr()?))
    }
    fn u128(&mut self) -> Option<u128> {
        Some(u128::from_le_bytes(self.arr()?))
    }
    fn done(&self) -> bool {
        self.0.is_empty()
    }
}

/// A token's DEFINITION: its rules, fixed. Its state is its params again (a related read returns state, not params).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    pub code_hash: [u8; 32],
    pub decimals: u8,
    /// The smallest send (spam costs: TOKEN §2).
    pub min_send: u128,
    /// `fixed(genesis)`: the one chain owner whose seq 0 may be the genesis, and its amount.
    pub genesis_owner: [u8; 32],
    pub genesis_amount: u128,
    pub name: Vec<u8>,
    pub nonce: [u8; 16],
    /// Its CLOCK: the beacon contract whose height stamps every entry (TOKEN §5a).
    pub beacon: [u8; 32],
    /// The FINALITY MARGIN, in beacon steps (TOKEN §3: 10 for real; tests use less).
    pub margin: u64,
    /// The KEY LOG's code hash: every owner is a DID, its keys in its key log (TOKEN §1).
    pub idlog: [u8; 32],
    /// Its first MINT authority (TOKEN §1a): none — the supply is fixed at its genesis.
    pub mint: Option<authority::KeySet>,
    /// Its first FREEZE authority: none — no chain of it is ever frozen.
    pub freeze: Option<authority::KeySet>,
    /// Its IDS (TOKEN §1b): none (fungible), unique (each item one, ERC-721) or many (ERC-1155).
    pub ids: Ids,
    /// Its ROYALTY: a share in basis points and the creator (a DID) it goes to — paid on every sale of it (§1b, §4).
    pub royalty: Option<(u16, [u8; 32])>,
    /// Its CAP (TOKEN §1c): the most there will ever be — genesis plus every mint (0: none). A capped token (and every
    /// numbered one) mints by NUMBER into records of [`supply::RECORD`] copies ([`supply::record_of`]), made as copies
    /// are minted, without end; a cap stops the numbers.
    pub cap: u128,
    /// The Craftworks ITEM it is of (§1c; zero: none) and the item's VERSION it pins (zero: the item as it is). An
    /// item may be a collection (a set item): grouping is the item's own metadata, not the token's (owner 10-10).
    pub item: [u8; 32],
    pub version: [u8; 32],
    /// NON-TRANSFERABLE (soulbound, TOKEN §1d): held where it is minted — no send, spend, allowance, escrow or vest of
    /// it; it can still be burned (and staked, which stays its owner's).
    pub soulbound: bool,
    /// Its TRANSFER FEE (TOKEN §1e, optional): (basis points, the most one transfer pays — 0: no most, the collector)
    /// — charged ON TOP of every send, spend, escrow and vest (the receiver gets the whole amount), claimed by its
    /// collector with a [`Body::FeeClaim`]. None when the collector itself transfers.
    pub transfer_fee: Option<(u16, u128, [u8; 32])>,
    /// SOLD ON DEMAND (TOKEN §1c, owner 10-10: "allow mint without owner"): (the token paid in, the price of one, the
    /// payee) — anyone pays the price to the payee and mints a copy, the owner offline. Its records are small
    /// ([`supply::PAID_RECORD`]): each paid mint's payment is read when its record is checked.
    pub sale: Option<(Tok, u128, [u8; 32])>,
    /// KEEPER ISSUANCE (TOKEN §7d, the `keepers` supply rule): each standing epoch's POT ([`Keepers::amount`]) is split
    /// by its STATEMENT among the accounts that proved keeping; each mints its line, locked as stake
    /// ([`Body::KeeperMint`]). `None`: no keeper issuance.
    pub keepers: Option<Keepers>,
}

/// The `keepers` supply rule (TOKEN §6, §7d): a POT of `rate` per standing epoch from epoch `start`, halving every
/// `half_life` epochs — split by each epoch's statement (who kept: the statement's `kept`, its one rule).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keepers {
    pub rate: u128,
    pub half_life: u64,
    pub start: u64,
}

impl Keepers {
    /// The POT of standing epoch `epoch` (0: before the start, or halved away).
    pub fn amount(&self, epoch: u64) -> u128 {
        if epoch < self.start {
            return 0;
        }
        let halvings = (epoch - self.start) / self.half_life;
        if halvings >= 128 { 0 } else { self.rate >> halvings }
    }
}

/// A keeper mint's NAME: one per token and standing epoch (an account's chain takes each once).
pub fn keeper_nonce(definition: &[u8; 32], epoch: u64) -> [u8; 16] {
    let mut h = blake3::Hasher::new_derive_key("craftworks 2026-10-10 keeper mint");
    h.update(definition);
    h.update(&epoch.to_le_bytes());
    h.finalize().as_bytes()[..16].try_into().unwrap()
}

/// The transfer fee `from` pays for moving `amount` of a token (TOKEN §1e): its share rounded up, at most its most.
pub fn transfer_fee(d: &Definition, from: &[u8; 32], amount: u128) -> u128 {
    let Some((bps, most, collector)) = d.transfer_fee else { return 0 };
    if *from == collector {
        return 0;
    }
    let bps = bps as u128;
    let fee = amount / 10_000 * bps + (amount % 10_000 * bps).div_ceil(10_000);
    if most > 0 { fee.min(most) } else { fee }
}

/// A token's ids (TOKEN §1b).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ids {
    None = 0,
    Unique = 1,
    Many = 2,
    /// NUMBERED copies (§1c): each item is copy `n` (1, 2, …, up to the cap), one of each — [`numbered_item`].
    Numbered = 3,
}

/// A NUMBERED token's item: copy `n` of it.
pub fn numbered_item(definition: &[u8; 32], n: u128) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key("craftworks 2026-10-10 token numbered item");
    h.update(definition);
    h.update(&n.to_be_bytes());
    *h.finalize().as_bytes()
}

/// A UNIQUE token's item: made by its mint (the shard's epoch and the mint's nonce), so each is minted once.
pub fn unique_item(definition: &[u8; 32], epoch: u64, nonce: &[u8; 16]) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key("craftworks 2026-10-09 token unique item");
    h.update(definition);
    h.update(&epoch.to_le_bytes());
    h.update(nonce);
    *h.finalize().as_bytes()
}

impl Definition {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = vec![DEFINITION];
        v.extend_from_slice(&self.code_hash);
        v.push(self.decimals);
        v.extend_from_slice(&self.min_send.to_le_bytes());
        v.extend_from_slice(&self.genesis_owner);
        v.extend_from_slice(&self.genesis_amount.to_le_bytes());
        v.push(self.name.len() as u8);
        v.extend_from_slice(&self.name);
        v.extend_from_slice(&self.nonce);
        v.extend_from_slice(&self.beacon);
        v.extend_from_slice(&self.margin.to_le_bytes());
        v.extend_from_slice(&self.idlog);
        authority::put_opt(&mut v, &self.mint);
        authority::put_opt(&mut v, &self.freeze);
        v.push(self.ids as u8);
        match self.royalty {
            None => v.push(0),
            Some((bps, creator)) => {
                v.push(1);
                v.extend_from_slice(&bps.to_le_bytes());
                v.extend_from_slice(&creator);
            }
        }
        v.extend_from_slice(&self.cap.to_le_bytes());
        v.extend_from_slice(&self.item);
        v.extend_from_slice(&self.version);
        v.push(self.soulbound as u8);
        match self.transfer_fee {
            None => v.push(0),
            Some((bps, most, collector)) => {
                v.push(1);
                v.extend_from_slice(&bps.to_le_bytes());
                v.extend_from_slice(&most.to_le_bytes());
                v.extend_from_slice(&collector);
            }
        }
        match &self.sale {
            None => v.push(0),
            Some((tok, price, payee)) => {
                v.push(1);
                tok.put(&mut v);
                v.extend_from_slice(&price.to_le_bytes());
                v.extend_from_slice(payee);
            }
        }
        match &self.keepers {
            None => v.push(0),
            Some(k) => {
                v.push(1);
                v.extend_from_slice(&k.rate.to_le_bytes());
                v.extend_from_slice(&k.half_life.to_le_bytes());
                v.extend_from_slice(&k.start.to_le_bytes());
            }
        }
        v
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = R(b);
        (r.u8()? == DEFINITION).then_some(())?;
        let code_hash = r.arr()?;
        let decimals = r.u8()?;
        let min_send = r.u128()?;
        let genesis_owner = r.arr()?;
        let genesis_amount = r.u128()?;
        let n = r.u8()? as usize;
        let name = r.take(n)?.to_vec();
        let nonce = r.arr()?;
        let beacon = r.arr()?;
        let margin = r.u64()?;
        let idlog = r.arr()?;
        let mint = authority::read_opt(&mut r)?;
        let freeze = authority::read_opt(&mut r)?;
        let ids = match r.u8()? {
            0 => Ids::None,
            1 => Ids::Unique,
            2 => Ids::Many,
            3 => Ids::Numbered,
            _ => return None,
        };
        let royalty = match r.u8()? {
            0 => None,
            1 => Some((u16::from_le_bytes(r.arr()?), r.arr()?)),
            _ => return None,
        };
        let (cap, item, version) = (r.u128()?, r.arr()?, r.arr()?);
        let soulbound = match r.u8()? {
            0 => false,
            1 => true,
            _ => return None,
        };
        let transfer_fee = match r.u8()? {
            0 => None,
            1 => Some((u16::from_le_bytes(r.arr()?), r.u128()?, r.arr()?)),
            _ => return None,
        };
        let sale = match r.u8()? {
            0 => None,
            1 => Some((Tok::read(&mut r)?, r.u128()?, r.arr()?)),
            _ => return None,
        };
        let keepers = match r.u8()? {
            0 => None,
            1 => Some(Keepers { rate: r.u128()?, half_life: r.u64()?, start: r.u64()? }),
            _ => return None,
        };
        let ok = r.done() && decimals <= DECIMALS && min_send > 0 && name.len() <= NAME_MAX && margin >= 1 && royalty.is_none_or(|(b, _)| b > 0 && b <= 10_000) && (cap == 0 || genesis_amount <= cap) && (cap == 0 || matches!(ids, Ids::None | Ids::Numbered))
            && royalty.is_none_or(|(_, to)| to != HOLDERS || (ids == Ids::Numbered && cap >= 2))
            && transfer_fee.is_none_or(|(b, _, c)| b > 0 && b <= 10_000 && c != HOLDERS)
            // Numbered copies are whole, sendable one at a time, and only minted (a genesis would hold numbers no copy has).
            && (ids != Ids::Numbered || (decimals == 0 && min_send == 1 && genesis_amount == 0))
            // A sale: of a capped (or numbered) token, at a price, to someone.
            && sale.is_none_or(|(_, price, payee)| price > 0 && payee != HOLDERS && (cap > 0 || ids == Ids::Numbered))
            // Keeper issuance: of a plain uncapped token, at a rate, halving at all.
            && keepers.is_none_or(|k| k.rate > 0 && k.half_life > 0 && cap == 0 && ids == Ids::None);
        ok.then_some(Self { code_hash, decimals, min_send, genesis_owner, genesis_amount, name, nonce, beacon, margin, idlog, mint, freeze, ids, royalty, cap, item, version, soulbound, transfer_fee, sale, keepers })
    }
    /// A TOKEN'S CONTRACTS, as made with it — `(params, first state)` of its DEFINITION, its AUTHORITY CHAIN and its
    /// FREEZE LIST (a debit's update reads the freeze list): the one list every tool and page puts.
    pub fn contracts(&self, code_hash: &[u8; 32]) -> [(Vec<u8>, Vec<u8>); 3] {
        let id = id(code_hash, &self.encode());
        let f = freeze::Params { code_hash: *code_hash, definition: id, beacon: self.beacon };
        [(self.encode(), self.encode()), (self.authority().encode(), authority::Chain::default().encode()), (f.encode(), freeze::List::default().encode())]
    }
    /// Minted by NUMBER into records ([`supply::RECORD`]): a cap, or numbered copies (each number once, ever).
    pub fn capped(&self) -> bool {
        self.cap > 0 || self.ids == Ids::Numbered
    }
    /// Its authority chain's params.
    pub fn authority(&self) -> authority::Params {
        authority::Params { code_hash: self.code_hash, definition: id(&self.code_hash, &self.encode()), beacon: self.beacon }
    }
}

/// A TOKEN as chains name each other (TOKEN §1b): its definition and, for a token of ids, the ITEM (zero: a fungible
/// token's one balance). A chain holds one token's one item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Tok {
    pub definition: [u8; 32],
    pub item: [u8; 32],
}

impl From<[u8; 32]> for Tok {
    fn from(definition: [u8; 32]) -> Tok {
        Tok { definition, item: [0; 32] }
    }
}

impl Tok {
    fn put(&self, v: &mut Vec<u8>) {
        v.extend_from_slice(&self.definition);
        v.extend_from_slice(&self.item);
    }
    fn read(r: &mut R) -> Option<Tok> {
        Some(Tok { definition: r.arr()?, item: r.arr()? })
    }
}

/// An ACCOUNT CHAIN's params: one token, one OWNER — a DID, written by one device of it (TOKEN §1).
/// Craftworks tokens' DECIMALS (owner 10-10, micro payments: Solana's 9 — a billion tokens is 10¹⁸ units, far inside
/// u128); a whole token is [`UNIT`] smallest units. The standard every Craftworks token is made with, and the MOST a
/// definition may have (owner 10-10: 18 does not fit — amounts are u128 and a split or a price multiplies by up to
/// 10¹²: at 18 decimals a large supply overflows; 0 for items: a copy is whole).
pub const DECIMALS: u8 = 9;
pub const UNIT: u128 = 1_000_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainParams {
    pub code_hash: [u8; 32],
    pub definition: [u8; 32],
    /// The owner's DID.
    pub owner: [u8; 32],
    /// The network's ONE beacon (TOKEN §5a) — in every chain's params, so a chain's id is computable for any token and
    /// `update_state` reads its definition and the beacon in one round.
    pub beacon: [u8; 32],
    /// The KEY LOG's code hash: the owner's key log is the contract at `id(idlog, owner)`, read in the same round.
    pub idlog: [u8; 32],
    /// The ITEM this chain holds (TOKEN §1b; zero: a fungible token).
    pub item: [u8; 32],
}

impl ChainParams {
    /// Its encoded length.
    pub const LEN: usize = 1 + 32 * 6;
    pub fn encode(&self) -> Vec<u8> {
        [&[CHAIN][..], &self.code_hash, &self.definition, &self.owner, &self.beacon, &self.idlog, &self.item].concat()
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = R(b);
        (r.u8()? == CHAIN).then_some(())?;
        let p = Self { code_hash: r.arr()?, definition: r.arr()?, owner: r.arr()?, beacon: r.arr()?, idlog: r.arr()?, item: r.arr()? };
        r.done().then_some(p)
    }
    pub fn id(&self) -> [u8; 32] {
        id(&self.code_hash, &self.encode())
    }
    /// The OWNER's key log: its contract id.
    pub fn keys_id(&self) -> [u8; 32] {
        id(&self.idlog, &self.owner)
    }
    /// The chain of `owner` for the same token (whose sends this chain cites).
    pub fn of(&self, owner: [u8; 32]) -> Self {
        Self { owner, ..*self }
    }
    /// The token (and item) this chain holds.
    pub fn tok(&self) -> Tok {
        Tok { definition: self.definition, item: self.item }
    }
    /// The chain of `owner` for token `t` (same code, same beacon).
    pub fn at(&self, t: Tok, owner: [u8; 32]) -> Self {
        Self { definition: t.definition, item: t.item, owner, ..*self }
    }
}

/// EVIDENCE a citing entry carries about what was PRUNED (TOKEN §2, the prune cut): `mine`, an absence proof that its
/// name is not in this chain's received set as of its last cut (needed when it cites something stamped at or before
/// that cut); `theirs`, for each cited chain that has cut past what is cited, (that chain's id, the cited entry's seq,
/// a proof against its cut's roots, the cited entry's bytes) — a PRESENCE proof that the history holds that entry at
/// that seq (inline, or — over 1 KB — as a value block whose id the bytes beside it must have), or (a refund) an
/// ABSENCE proof of a fill in its received set (no bytes).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Evidence {
    pub mine: Vec<u8>,
    pub theirs: Vec<([u8; 32], u64, Vec<u8>, Vec<u8>)>,
}


/// The received-set key of a name: what a cut's received set holds, what absence proofs are about.
pub fn received_key(kind: u8, who: &[u8; 32], nonce: &[u8; 16]) -> Vec<u8> {
    [&[kind][..], who, nonce].concat()
}
/// Received-set kinds: a send received, a mint credited.
pub const RECEIVED: u8 = b'r';
pub const MINTED: u8 = b'm';
/// A program's payout received (who: the program, nonce: the payout's key).
pub const PAYOUT: u8 = b'p';
/// A vesting step received (its key also carries the step: [`vested_key`]).
pub const VESTED: u8 = b'v';
/// A transfer fee claimed (who: the payer; nonce: its debit's).
pub const FEE: u8 = b'f';
/// A HOLDERS' share received (who: the program; nonce: [`holder_nonce`] of the fee's key and the copy).
pub const HOLDERS_SHARE: u8 = b'h';
/// The ROYALTY RECIPIENT that is an edition's HOLDERS (TOKEN §1c): the fee is shared by every OTHER copy's holder,
/// each claiming `fee / (cap − 1)` once ([`Body::HolderPayout`]). Only a numbered edition of at least 2 names it.
pub const HOLDERS: [u8; 32] = [0xff; 32];

/// A holders' share's name in the received set: the sale (its sold escrow) and the copy.
pub fn holder_nonce(sold: &escrow::EscrowRef, copy: u128) -> [u8; 16] {
    let mut h = blake3::Hasher::new_derive_key("craftworks 2026-10-10 token holders share");
    h.update(&escrow::fee_key(sold));
    h.update(&copy.to_be_bytes());
    h.finalize().as_bytes()[..16].try_into().unwrap()
}

/// What a chain held as of beacon `at` (its balance after its last entry stamped at or before `at`). `None`: the chain
/// does not show it — it starts after `at` (a cut, or no entry yet).
pub fn held_at(s: &State, at: u64) -> Option<u128> {
    s.entries.iter().take_while(|e| e.stamp <= at).last().map(|e| e.balance)
}
/// The received-set key of step `step` of the vest `nonce` in `from`'s chain.
pub fn vested_key(from: &[u8; 32], nonce: &[u8; 16], step: u32) -> Vec<u8> {
    [&received_key(VESTED, from, nonce)[..], &step.to_le_bytes()].concat()
}
/// The name a mint is received under (a mint has no sender chain).
pub const MINTER: [u8; 32] = [0; 32];
/// The most related contracts one validation or update asks for in its one round (the node's limit, measured 10-08:
/// depth 1, at most 10). A holder taking in new entries asks for the definition, the beacon and every chain they cite.
pub const MAX_RELATED: usize = 10;

/// What every chain entry's signed message BEGINS with: a device's key signs other things too (a member's tables), and
/// a signer that is asked only for token entries signs only messages that begin with this (TOKEN §7c).
pub const ENTRY_DOMAIN: &[u8] = craftworks_identity::TOKEN_ENTRY_DOMAIN;
/// The longest writer credential (`CWMB ‖ did ‖ owner ‖ node ‖ MLS key ‖ signature`, with room).
pub const CRED_MAX: usize = 512;
/// The history key of an entry: its seq, big-endian (the tree's order is the chain's). Its value: the entry's bytes.
pub fn history_key(seq: u64) -> [u8; 8] {
    seq.to_be_bytes()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    /// seq 0 of the definition's genesis owner only: the whole fixed supply.
    Genesis { amount: u128 },
    /// A debit to `to`'s chain (named by its owner key).
    Send { to: [u8; 32], amount: u128 },
    /// A credit citing the send (or spend, or fill) `nonce` in `from`'s chain — appended by ANYONE (with work);
    /// `cited`: that entry's stamp (appended no sooner than a margin after it: TOKEN §3).
    Receive { from: [u8; 32], nonce: [u8; 16], amount: u128, cited: u64, ev: Evidence },
    /// A debit with no credit anywhere.
    Burn { amount: u128 },
    /// The owner lets `spender` debit up to `limit` (replacing any earlier allowance; 0 revokes) — ERC-20 `approve`.
    Allow { spender: [u8; 32], limit: u128 },
    /// A debit to `to`'s chain written and signed by a SPENDER within their allowance — ERC-20 `transferFrom`.
    Spend { to: [u8; 32], amount: u128 },
    /// The chain's WRITER (TOKEN §1): one device of the owner DID — its CREDENTIAL (`identity::credential`: the DID,
    /// the device's node key, signed by an owner key of the DID's key log) and the seq of the key-log event whose key
    /// signed it. Signed by that node key (it holds it). From then on the node key signs the owner's entries.
    Writer { cred: Vec<u8>, key_seq: u32 },
    /// A MINT (TOKEN §1a): the mint `nonce` of the token's supply shard of beacon `minted`'s epoch, naming this owner
    /// and `amount` — credited here; appended by anyone (with work), once.
    Mint { nonce: [u8; 16], amount: u128, minted: u64, mint: supply::Mint, ev: Evidence },
    /// A mint of a CAPPED token (§1c): the same, its mint in the RECORD of its numbers ([`supply::record_of`]) — and
    /// ADMITTED there ([`supply::admitted`]). `nonce` is a NUMBER (big-endian): for a numbered token the copy's (and
    /// `amount` 1); for plain copies the mint's first.
    CappedMint { nonce: [u8; 16], amount: u128, minted: u64, record: u64, mint: supply::Mint, ev: Evidence },
    /// A FALSE MINT (TOKEN §7a): this chain's witness key signed a mint credit (`entry`, of chain (`tok`, `owner`))
    /// whose mint — carried in the credit, signatures and all — the token's mint authority never signed (as its
    /// definition and authority chain say at the mint's stamp). Every stake and unbonding amount BURNED. Appended by
    /// anyone (with work).
    FalseMint { key: [u8; 32], sig: witness::WitnessSig, tok: Tok, owner: [u8; 32], entry: Box<Entry> },
    /// STAKE (TOKEN §7a): `amount` of the balance locked as witness stake — what a witness's draw weighs, and what
    /// its equivocation burns.
    Stake { amount: u128 },
    /// `amount` of the stake set to come back: released ([`Body::Release`]) [`UNBOND`] beacon steps after this entry —
    /// slashable until then.
    Unstake { amount: u128 },
    /// The unstake `unstake` (its entry's nonce) back into the balance, its delay over — appended by anyone.
    Release { unstake: [u8; 16] },
    /// KEEPER ISSUANCE (TOKEN §7d): this account's KEEPER LINE of standing epoch `epoch`'s STATEMENT (`root`; `proof`,
    /// its line's prolly proof under it) minted, LOCKED as stake — `amount` exactly the line's. Appended by ANYONE with
    /// its work (the proof fixes the amount: nobody need be online); its witnesses accept it only under the statement
    /// root accepted for that epoch. Named by [`keeper_nonce`] (once per epoch).
    KeeperMint { epoch: u64, root: [u8; 32], proof: Vec<u8>, amount: u128 },
    /// An EQUIVOCATION by this chain's witness key (`key`, its writer's node key): two witness signatures at one place
    /// naming different entries — every stake and unbonding amount BURNED. Appended by anyone.
    Slash { key: [u8; 32], a: witness::WitnessSig, b: witness::WitnessSig },
    /// VESTING (TOKEN §4a): `to` is given the schedule's total, released in STEPS — (beacon height, amount), heights
    /// rising — each claimable from its height on, never taken back. A debit of the total, now.
    Vest { to: [u8; 32], schedule: Vec<(u64, u128)> },
    /// Step `step` of the vest `vest` in `from`'s chain — its `amount`, released at beacon `at` — credited here;
    /// appended by anyone (with work), once, at or after `at`; `cited`: the vest's stamp.
    Vested { from: [u8; 32], vest: [u8; 16], step: u32, amount: u128, at: u64, cited: u64, ev: Evidence },
    /// A WITNESS'S MISDEED (TOKEN §7a): this chain's witness key (`key`) signed `entry` of `owner`'s chain of token `tok`
    /// — a credit whose cited debit is NOT in the cited chain (fake money). Shown by reading that chain (one related
    /// contract), only where it can say: whole back to the debit's stamp and not frozen by a fork (a debit an honest
    /// witness saw cannot have vanished otherwise). Every stake and unbonding amount BURNED. Appended by anyone.
    Misdeed { key: [u8; 32], sig: witness::WitnessSig, tok: Tok, owner: [u8; 32], entry: Box<Entry> },
    /// An ESCROW (TOKEN §5): `amount` locked for the outside contract `program` (a swap, a pool — an app), on its
    /// `terms` (the program's to read). A debit now; back only as the program's verdict says — or, from beacon
    /// `until` on (0: never), RECLAIMED whole if the program has not taken it ([`Body::Reclaim`]).
    Escrow { program: [u8; 32], terms: Vec<u8>, amount: u128, until: u64 },
    /// The escrow `escrow`'s RELEASE (unescrow): what its program's verdict gives back (`refund`) credited, naming
    /// that verdict (`verdict`: its section's [`escrow::Section::hash`]) — so of two verdicts a program's copies might
    /// hold, only the one every escrow was released under is ever paid out. Appended by anyone (with work), once.
    Unescrow { escrow: [u8; 16], refund: u128, verdict: [u8; 32] },
    /// The escrow `escrow` RECLAIMED (TOKEN §4): its whole amount back, from its `until` on, because its program never
    /// took it — the program, as its witnesses read it, lists it not, or is not there. Appended by anyone (with work),
    /// once. Witnesses then refuse any release or payout of a program that lists it.
    Reclaim { escrow: [u8; 16] },
    /// A TRANSFER FEE claimed (TOKEN §1e) by the token's fee collector (this chain's owner): the fee `from`'s debit
    /// `nonce` (stamped `cited`; it moved `gross`) paid on top — `amount`, as the definition's fee of `gross`.
    /// Appended by anyone (with work), once, once the debit is final.
    FeeClaim { from: [u8; 32], nonce: [u8; 16], gross: u128, amount: u128, cited: u64, ev: Evidence },
    /// A program's PAYOUT `key` to this chain: `amount`, as its verdict says. Appended by anyone (with work), once.
    Payout { program: [u8; 32], key: [u8; 16], amount: u128, verdict: [u8; 32], ev: Evidence },
    /// A HOLDERS' SHARE (TOKEN §1c): of the SALE of the copy escrowed as `sold` in `program` — whose royalty fee the
    /// program pays to [`HOLDERS`] under [`escrow::fee_key`]`(sold)` — the share of copy `copy` of that edition
    /// (`fee / (cap − 1)`), credited here to who HELD that copy when the sold copy was escrowed (beacon `at`, the
    /// escrow's stamp: one holder of each copy then; witnesses check `at`). Once per copy and sale; the sold copy takes
    /// none; a copy minted later held nothing then. Anyone appends it (with work).
    HolderPayout { program: [u8; 32], sold: escrow::EscrowRef, copy: u128, at: u64, amount: u128, verdict: [u8; 32], ev: Evidence },
    /// A FALSE VERDICT (TOKEN §7a): this chain's witness key signed a payout (`entry`, of chain (`tok`, `owner`)) naming
    /// a verdict — and here is that verdict (`section`: its hash is the one named; nobody can make another), which
    /// does not pay it. Every stake and unbonding amount BURNED. Appended by anyone (with work); needs nothing read.
    FalseVerdict { key: [u8; 32], sig: witness::WitnessSig, tok: Tok, owner: [u8; 32], entry: Box<Entry>, section: Vec<u8> },
    /// A MULTISIG owner's MEMBERS (TOKEN §1): the key set that signs from here, on set seq `set_seq`. The first (seq 0)
    /// is the set the owner id names, signed by k of it; each later one, by k of the set before.
    Members { set: authority::KeySet, set_seq: u32 },
    /// A PRUNE CUT (owner-signed): everything before it may be dropped. It carries their result — the allowances, the
    /// open offers and locks (the balance is its own carried balance) — and the roots of two prolly trees: the
    /// HISTORY (every entry before it, by seq — its blocks published, so the chain stays walkable) and the RECEIVED SET (every name received, filled or claimed so far).
    /// Each root's PARITY ids too (its group of one, as a table's root carries them): a reader rebuilds a root that
    /// does not arrive from its parity, never waiting on it (the blocks door races a block against its group).
    Cut {
        history: [u8; 32],
        received: [u8; 32],
        history_parity: Vec<[u8; 32]>,
        received_parity: Vec<[u8; 32]>,
        allowances: Vec<([u8; 32], u128)>,
        /// The chain's signer at the cut.
        writer: Option<Signer>,
        /// Its STAKE and its UNBONDING amounts (nonce, amount, release height), as before the cut.
        staked: u128,
        unbonding: Vec<([u8; 16], u128, u64)>,
        /// Writer keys retired while stake was held — still burnable.
        retired: Vec<[u8; 32]>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Auth {
    /// The chain owner's signature over the entry's message.
    Sig([u8; 64]),
    /// Another key's signature (a spender's, within their allowance): `by` signed it.
    Delegated { by: [u8; 32], sig: [u8; 64] },
    /// Work: `blake3(message ‖ work)` has [`WORK_BITS`] leading zero bits.
    Work(u64),
    /// A MULTISIG owner's k signatures (key index in its set, signature).
    Multi(Vec<(u8, [u8; 64])>),
}

/// One ENTRY: its place, its link, its name, what it does, the BALANCE after it, and who vouches for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub seq: u64,
    pub prev: [u8; 32],
    /// Its NAME, kept if it is re-appended at another seq (TOKEN §3).
    pub nonce: [u8; 16],
    /// The BEACON HEIGHT it was written at: never below its predecessor's; near the beacon when appended (TOKEN §3c).
    pub stamp: u64,
    pub body: Body,
    pub balance: u128,
    pub auth: Auth,
}

fn put_ev(v: &mut Vec<u8>, ev: &Evidence) {
    v.extend_from_slice(&(ev.mine.len() as u32).to_le_bytes());
    v.extend_from_slice(&ev.mine);
    v.extend_from_slice(&(ev.theirs.len() as u32).to_le_bytes());
    for (c, seq, b, e) in &ev.theirs {
        v.extend_from_slice(c);
        v.extend_from_slice(&seq.to_le_bytes());
        v.extend_from_slice(&(b.len() as u32).to_le_bytes());
        v.extend_from_slice(b);
        v.extend_from_slice(&(e.len() as u32).to_le_bytes());
        v.extend_from_slice(e);
    }
}
/// Proofs are bounded (a 1M-name set's is ~17 KB: TOKEN §2), and so is how many an entry carries.
const PROOF_MAX: usize = 64 * 1024;
fn read_ev(r: &mut R) -> Option<Evidence> {
    let n = r.u32()? as usize;
    if n > PROOF_MAX {
        return None;
    }
    let mine = r.take(n)?.to_vec();
    let k = r.u32()? as usize;
    if k > 4 {
        return None;
    }
    let mut theirs = Vec::with_capacity(k);
    for _ in 0..k {
        let c = r.arr()?;
        let seq = r.u64()?;
        let n = r.u32()? as usize;
        if n > PROOF_MAX {
            return None;
        }
        let proof = r.take(n)?.to_vec();
        let n = r.u32()? as usize;
        if n > PROOF_MAX {
            return None;
        }
        theirs.push((c, seq, proof, r.take(n)?.to_vec()));
    }
    Some(Evidence { mine, theirs })
}

impl Entry {
    /// What is signed or worked on: everything but the auth.
    /// What its signer signs: [`ENTRY_DOMAIN`], then its content.
    pub fn message(&self) -> Vec<u8> {
        let mut v = ENTRY_DOMAIN.to_vec();
        v.extend_from_slice(&self.content());
        v
    }

    /// Its CONTENT: everything but its signature (what its encoding begins with).
    fn content(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(120);
        v.extend_from_slice(&self.seq.to_le_bytes());
        v.extend_from_slice(&self.prev);
        v.extend_from_slice(&self.nonce);
        v.extend_from_slice(&self.stamp.to_le_bytes());
        match &self.body {
            Body::Genesis { amount } => {
                v.push(0);
                v.extend_from_slice(&amount.to_le_bytes());
            }
            Body::Send { to, amount } => {
                v.push(1);
                v.extend_from_slice(to);
                v.extend_from_slice(&amount.to_le_bytes());
            }
            Body::Receive { from, nonce, amount, cited, ev } => {
                v.push(2);
                v.extend_from_slice(from);
                v.extend_from_slice(nonce);
                v.extend_from_slice(&amount.to_le_bytes());
                v.extend_from_slice(&cited.to_le_bytes());
                put_ev(&mut v, ev);
            }
            Body::Burn { amount } => {
                v.push(3);
                v.extend_from_slice(&amount.to_le_bytes());
            }
            Body::Allow { spender, limit } => {
                v.push(4);
                v.extend_from_slice(spender);
                v.extend_from_slice(&limit.to_le_bytes());
            }
            Body::Spend { to, amount } => {
                v.push(5);
                v.extend_from_slice(to);
                v.extend_from_slice(&amount.to_le_bytes());
            }
            Body::Mint { nonce, amount, minted, mint, ev } | Body::CappedMint { nonce, amount, minted, mint, ev, .. } => {
                v.push(if matches!(self.body, Body::Mint { .. }) { 13 } else { 28 });
                v.extend_from_slice(nonce);
                v.extend_from_slice(&amount.to_le_bytes());
                v.extend_from_slice(&minted.to_le_bytes());
                if let Body::CappedMint { record, .. } = &self.body {
                    v.extend_from_slice(&record.to_le_bytes());
                }
                mint.encode(&mut v);
                put_ev(&mut v, ev);
            }
            Body::Members { set, set_seq } => {
                v.push(14);
                set.encode(&mut v);
                v.extend_from_slice(&set_seq.to_le_bytes());
            }
            Body::Writer { cred, key_seq } => {
                v.push(12);
                v.extend_from_slice(&(cred.len() as u16).to_le_bytes());
                v.extend_from_slice(cred);
                v.extend_from_slice(&key_seq.to_le_bytes());
            }
            Body::Escrow { program, terms, amount, until } => {
                v.push(25);
                v.extend_from_slice(program);
                v.extend_from_slice(&(terms.len() as u16).to_le_bytes());
                v.extend_from_slice(terms);
                v.extend_from_slice(&amount.to_le_bytes());
                v.extend_from_slice(&until.to_le_bytes());
            }
            Body::Reclaim { escrow } => {
                v.push(30);
                v.extend_from_slice(escrow);
            }
            Body::FeeClaim { from, nonce, gross, amount, cited, ev } => {
                v.push(31);
                v.extend_from_slice(from);
                v.extend_from_slice(nonce);
                v.extend_from_slice(&gross.to_le_bytes());
                v.extend_from_slice(&amount.to_le_bytes());
                v.extend_from_slice(&cited.to_le_bytes());
                put_ev(&mut v, ev);
            }
            Body::Unescrow { escrow, refund, verdict } => {
                v.push(26);
                v.extend_from_slice(escrow);
                v.extend_from_slice(&refund.to_le_bytes());
                v.extend_from_slice(verdict);
            }
            Body::Payout { program, key, amount, verdict, ev } => {
                v.push(27);
                v.extend_from_slice(program);
                v.extend_from_slice(key);
                v.extend_from_slice(&amount.to_le_bytes());
                v.extend_from_slice(verdict);
                put_ev(&mut v, ev);
            }
            Body::HolderPayout { program, sold, copy, at, amount, verdict, ev } => {
                v.push(29);
                v.extend_from_slice(program);
                sold.0.put(&mut v);
                v.extend_from_slice(&sold.1);
                v.extend_from_slice(&sold.2);
                v.extend_from_slice(&copy.to_le_bytes());
                v.extend_from_slice(&at.to_le_bytes());
                v.extend_from_slice(&amount.to_le_bytes());
                v.extend_from_slice(verdict);
                put_ev(&mut v, ev);
            }
            Body::Vest { to, schedule } => {
                v.push(19);
                v.extend_from_slice(to);
                v.push(schedule.len() as u8);
                for (h, a) in schedule {
                    v.extend_from_slice(&h.to_le_bytes());
                    v.extend_from_slice(&a.to_le_bytes());
                }
            }
            Body::Vested { from, vest, step, amount, at, cited, ev } => {
                v.push(20);
                v.extend_from_slice(from);
                v.extend_from_slice(vest);
                v.extend_from_slice(&step.to_le_bytes());
                v.extend_from_slice(&amount.to_le_bytes());
                v.extend_from_slice(&at.to_le_bytes());
                v.extend_from_slice(&cited.to_le_bytes());
                put_ev(&mut v, ev);
            }
            Body::Stake { amount } => {
                v.push(15);
                v.extend_from_slice(&amount.to_le_bytes());
            }
            Body::KeeperMint { epoch, root, proof, amount } => {
                v.push(34);
                v.extend_from_slice(&epoch.to_le_bytes());
                v.extend_from_slice(root);
                v.extend_from_slice(&(proof.len() as u32).to_le_bytes());
                v.extend_from_slice(proof);
                v.extend_from_slice(&amount.to_le_bytes());
            }
            Body::Unstake { amount } => {
                v.push(16);
                v.extend_from_slice(&amount.to_le_bytes());
            }
            Body::Release { unstake } => {
                v.push(17);
                v.extend_from_slice(unstake);
            }
            Body::Slash { key, a, b } => {
                v.push(18);
                v.extend_from_slice(key);
                a.put(&mut v);
                b.put(&mut v);
            }
            Body::Misdeed { key, sig, tok, owner, entry } => {
                v.push(21);
                v.extend_from_slice(key);
                sig.put(&mut v);
                tok.put(&mut v);
                v.extend_from_slice(owner);
                let b = entry.encode();
                v.extend_from_slice(&(b.len() as u32).to_le_bytes());
                v.extend_from_slice(&b);
            }
            Body::FalseMint { key, sig, tok, owner, entry } => {
                v.push(33);
                v.extend_from_slice(key);
                sig.put(&mut v);
                tok.put(&mut v);
                v.extend_from_slice(owner);
                let b = entry.encode();
                v.extend_from_slice(&(b.len() as u32).to_le_bytes());
                v.extend_from_slice(&b);
            }
            Body::FalseVerdict { key, sig, tok, owner, entry, section } => {
                v.push(32);
                v.extend_from_slice(key);
                sig.put(&mut v);
                tok.put(&mut v);
                v.extend_from_slice(owner);
                for b in [entry.encode(), section.clone()] {
                    v.extend_from_slice(&(b.len() as u32).to_le_bytes());
                    v.extend_from_slice(&b);
                }
            }
            Body::Cut { history, received, history_parity, received_parity, allowances, writer, staked, unbonding, retired } => {
                v.push(11);
                v.extend_from_slice(history);
                v.extend_from_slice(received);
                for ids in [history_parity, received_parity] {
                    v.push(ids.len() as u8);
                    for id in ids {
                        v.extend_from_slice(id);
                    }
                }
                v.extend_from_slice(&(allowances.len() as u32).to_le_bytes());
                for (k, l) in allowances {
                    v.extend_from_slice(k);
                    v.extend_from_slice(&l.to_le_bytes());
                }
                match writer {
                    None => v.push(0),
                    Some(w) => w.encode(&mut v),
                }
                v.extend_from_slice(&staked.to_le_bytes());
                v.extend_from_slice(&(unbonding.len() as u32).to_le_bytes());
                for (n, a, at) in unbonding {
                    v.extend_from_slice(n);
                    v.extend_from_slice(&a.to_le_bytes());
                    v.extend_from_slice(&at.to_le_bytes());
                }
                v.push(retired.len() as u8);
                for k in retired {
                    v.extend_from_slice(k);
                }
            }
        }
        v.extend_from_slice(&self.balance.to_le_bytes());
        v
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut v = self.content();
        match &self.auth {
            Auth::Sig(s) => {
                v.push(0);
                v.extend_from_slice(s);
            }
            Auth::Work(w) => {
                v.push(1);
                v.extend_from_slice(&w.to_le_bytes());
            }
            Auth::Delegated { by, sig } => {
                v.push(2);
                v.extend_from_slice(by);
                v.extend_from_slice(sig);
            }
            Auth::Multi(sigs) => {
                v.push(3);
                authority::put_sigs(&mut v, sigs);
            }
        }
        v
    }
    fn read(r: &mut R) -> Option<Self> {
        let seq = r.u64()?;
        let prev = r.arr()?;
        let nonce = r.arr()?;
        let stamp = r.u64()?;
        let body = match r.u8()? {
            0 => Body::Genesis { amount: r.u128()? },
            1 => Body::Send { to: r.arr()?, amount: r.u128()? },
            2 => Body::Receive { from: r.arr()?, nonce: r.arr()?, amount: r.u128()?, cited: r.u64()?, ev: read_ev(r)? },
            3 => Body::Burn { amount: r.u128()? },
            4 => Body::Allow { spender: r.arr()?, limit: r.u128()? },
            5 => Body::Spend { to: r.arr()?, amount: r.u128()? },
            11 => {
                let history = r.arr()?;
                let received = r.arr()?;
                let mut parity = || -> Option<Vec<[u8; 32]>> {
                    let n = r.u8()? as usize;
                    (n <= 8).then_some(())?;
                    (0..n).map(|_| r.arr()).collect()
                };
                let (history_parity, received_parity) = (parity()?, parity()?);
                let count = |r: &mut R| -> Option<usize> { let n = r.u32()? as usize; (n <= 1024).then_some(n) };
                let n = count(r)?;
                let mut allowances = Vec::with_capacity(n);
                for _ in 0..n {
                    allowances.push((r.arr()?, r.u128()?));
                }
                let writer = match r.u8()? {
                    0 => None,
                    1 => Some(Signer::Device(r.arr()?, r.u32()?)),
                    2 => Some(Signer::Set(authority::KeySet::read(r)?, r.u32()?)),
                    _ => return None,
                };
                let staked = r.u128()?;
                let n = count(r)?;
                let mut unbonding = Vec::with_capacity(n);
                for _ in 0..n {
                    unbonding.push((r.arr()?, r.u128()?, r.u64()?));
                }
                let n = r.u8()? as usize;
                if n > RETIRED_MAX {
                    return None;
                }
                let retired = (0..n).map(|_| r.arr()).collect::<Option<Vec<[u8; 32]>>>()?;
                Body::Cut { history, received, history_parity, received_parity, allowances, writer, staked, unbonding, retired }
            }
            12 => {
                let n = u16::from_le_bytes(r.arr()?) as usize;
                if n > CRED_MAX {
                    return None;
                }
                Body::Writer { cred: r.take(n)?.to_vec(), key_seq: r.u32()? }
            }
            13 => Body::Mint { nonce: r.arr()?, amount: r.u128()?, minted: r.u64()?, mint: supply::Mint::read(r)?, ev: read_ev(r)? },
            28 => Body::CappedMint { nonce: r.arr()?, amount: r.u128()?, minted: r.u64()?, record: r.u64()?, mint: supply::Mint::read(r)?, ev: read_ev(r)? },
            33 => {
                let (key, sig, tok, owner) = (r.arr()?, witness::WitnessSig::read(r)?, Tok::read(r)?, r.arr()?);
                let n = r.u32()? as usize;
                if n > MISDEED_MAX {
                    return None;
                }
                Body::FalseMint { key, sig, tok, owner, entry: Box::new(Entry::decode(r.take(n)?)?) }
            }
            15 => Body::Stake { amount: r.u128()? },
            34 => {
                let (epoch, root) = (r.u64()?, r.arr()?);
                let n = u32::from_le_bytes(r.arr()?) as usize;
                if n > PROOF_MAX {
                    return None;
                }
                let proof = r.take(n)?.to_vec();
                Body::KeeperMint { epoch, root, proof, amount: r.u128()? }
            }
            16 => Body::Unstake { amount: r.u128()? },
            17 => Body::Release { unstake: r.arr()? },
            18 => Body::Slash { key: r.arr()?, a: witness::WitnessSig::read(r)?, b: witness::WitnessSig::read(r)? },
            14 => Body::Members { set: authority::KeySet::read(r)?, set_seq: r.u32()? },
            19 => {
                let to = r.arr()?;
                let n = r.u8()? as usize;
                if n > VEST_STEPS {
                    return None;
                }
                let schedule = (0..n).map(|_| Some((r.u64()?, r.u128()?))).collect::<Option<Vec<_>>>()?;
                Body::Vest { to, schedule }
            }
            21 => {
                let (key, sig, tok, owner) = (r.arr()?, witness::WitnessSig::read(r)?, Tok::read(r)?, r.arr()?);
                let n = r.u32()? as usize;
                if n > MISDEED_MAX {
                    return None;
                }
                Body::Misdeed { key, sig, tok, owner, entry: Box::new(Entry::decode(r.take(n)?)?) }
            }
            32 => {
                let (key, sig, tok, owner) = (r.arr()?, witness::WitnessSig::read(r)?, Tok::read(r)?, r.arr()?);
                let n = r.u32()? as usize;
                if n > MISDEED_MAX {
                    return None;
                }
                let entry = Box::new(Entry::decode(r.take(n)?)?);
                let m = r.u32()? as usize;
                if m > MISDEED_MAX * 64 {
                    return None;
                }
                Body::FalseVerdict { key, sig, tok, owner, entry, section: r.take(m)?.to_vec() }
            }
            25 => {
                let program = r.arr()?;
                let n = u16::from_le_bytes(r.arr()?) as usize;
                if n > escrow::TERMS_MAX {
                    return None;
                }
                Body::Escrow { program, terms: r.take(n)?.to_vec(), amount: r.u128()?, until: r.u64()? }
            }
            30 => Body::Reclaim { escrow: r.arr()? },
            31 => Body::FeeClaim { from: r.arr()?, nonce: r.arr()?, gross: r.u128()?, amount: r.u128()?, cited: r.u64()?, ev: read_ev(r)? },
            26 => Body::Unescrow { escrow: r.arr()?, refund: r.u128()?, verdict: r.arr()? },
            27 => Body::Payout { program: r.arr()?, key: r.arr()?, amount: r.u128()?, verdict: r.arr()?, ev: read_ev(r)? },
            29 => Body::HolderPayout { program: r.arr()?, sold: (Tok::read(r)?, r.arr()?, r.arr()?), copy: r.u128()?, at: r.u64()?, amount: r.u128()?, verdict: r.arr()?, ev: read_ev(r)? },
            20 => Body::Vested { from: r.arr()?, vest: r.arr()?, step: r.u32()?, amount: r.u128()?, at: r.u64()?, cited: r.u64()?, ev: read_ev(r)? },
            _ => return None,
        };
        let balance = r.u128()?;
        let auth = match r.u8()? {
            0 => Auth::Sig(r.arr()?),
            1 => Auth::Work(r.u64()?),
            2 => Auth::Delegated { by: r.arr()?, sig: r.arr()? },
            3 => Auth::Multi(authority::read_sigs(r)?),
            _ => return None,
        };
        Some(Self { seq, prev, nonce, stamp, body, balance, auth })
    }
    pub fn hash(&self) -> [u8; 32] {
        *blake3::hash(&self.encode()).as_bytes()
    }
    /// One entry from its bytes, exactly.
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = R(b);
        let e = Self::read(&mut r)?;
        r.done().then_some(e)
    }
    /// Written by the chain's writer (signed), not by anyone (worked).
    pub fn by_owner(&self) -> bool {
        matches!(self.auth, Auth::Sig(_) | Auth::Multi(_))
    }
    fn signed_by(&self, owner: &[u8; 32]) -> bool {
        let Auth::Sig(s) = &self.auth else { return false };
        sig_ok(owner, &self.message(), s)
    }
    /// Signed by `key` as a delegate (a spender): its signature, and its key named.
    fn delegated_by(&self, key: &[u8; 32]) -> bool {
        let Auth::Delegated { by, sig } = &self.auth else { return false };
        by == key && sig_ok(by, &self.message(), sig)
    }
    fn worked(&self) -> bool {
        let Auth::Work(w) = self.auth else { return false };
        work_ok(&self.message(), w)
    }
}

pub(crate) fn sig_ok(key: &[u8; 32], message: &[u8], sig: &[u8; 64]) -> bool {
    VerifyingKey::from_bytes(key).is_ok_and(|k| k.verify_strict(message, &Signature::from_bytes(sig)).is_ok())
}

pub fn work_ok(message: &[u8], work: u64) -> bool {
    let mut h = blake3::Hasher::new();
    h.update(message);
    h.update(&work.to_le_bytes());
    let d = h.finalize();
    let b = d.as_bytes();
    u128::from_be_bytes(b[..16].try_into().unwrap()).leading_zeros() >= WORK_BITS
}

/// Who signs a chain's OWNER entries (TOKEN §1): one DEVICE of the owner DID (its node key, on its key seq) — or, a
/// MULTISIG owner, a KEY SET (k of n keys, on its set seq). At one seq the higher seq wins (§3b).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Signer {
    Device([u8; 32], u32),
    Set(authority::KeySet, u32),
}

impl Signer {
    /// Its rank at a seq: the key seq of a device, the set seq of a key set.
    pub fn rank(&self) -> u32 {
        match self {
            Signer::Device(_, k) | Signer::Set(_, k) => *k,
        }
    }
    /// It signed `e` (a device: its signature; a set: k of its keys).
    pub fn signs(&self, e: &Entry) -> bool {
        match self {
            Signer::Device(node, _) => e.signed_by(node),
            Signer::Set(set, _) => matches!(&e.auth, Auth::Multi(sigs) if set.signed(&e.message(), sigs)),
        }
    }
    fn encode(&self, v: &mut Vec<u8>) {
        match self {
            Signer::Device(node, k) => {
                v.push(1);
                v.extend_from_slice(node);
                v.extend_from_slice(&k.to_le_bytes());
            }
            Signer::Set(set, k) => {
                v.push(2);
                set.encode(v);
                v.extend_from_slice(&k.to_le_bytes());
            }
        }
    }
}

/// A MULTISIG owner's id: its FIRST key set's hash — what the chain's params name as owner.
pub fn set_id(set: &authority::KeySet) -> [u8; 32] {
    let mut v = Vec::new();
    set.encode(&mut v);
    let mut h = blake3::Hasher::new_derive_key("craftworks 2026-10-09 token key set owner");
    h.update(&v);
    *h.finalize().as_bytes()
}

/// A chain's STATE: its entries — or, once its writer signed two at one seq, the entries before and the two
/// (EQUIVOCATION, TOKEN §3b: the chain is frozen; every debit after the fork is void).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct State {
    pub entries: Vec<Entry>,
    pub fork: Option<(Entry, Entry)>,
}

impl State {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = (self.entries.len() as u32).to_le_bytes().to_vec();
        let put = |v: &mut Vec<u8>, e: &Entry| {
            let b = e.encode();
            v.extend_from_slice(&(b.len() as u32).to_le_bytes());
            v.extend_from_slice(&b);
        };
        for e in &self.entries {
            put(&mut v, e);
        }
        match &self.fork {
            None => v.push(0),
            Some((a, b)) => {
                v.push(1);
                put(&mut v, a);
                put(&mut v, b);
            }
        }
        v
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.is_empty() {
            return Some(Self::default());
        }
        let mut r = R(b);
        let read = |r: &mut R| -> Option<Entry> {
            let n = r.u32()? as usize;
            let mut one = R(r.take(n)?);
            let e = Entry::read(&mut one)?;
            one.done().then_some(e)
        };
        let n = r.u32()? as usize;
        if n > b.len() {
            return None;
        }
        let mut entries = Vec::with_capacity(n);
        for _ in 0..n {
            entries.push(read(&mut r)?);
        }
        let fork = match r.u8()? {
            0 => None,
            1 => Some((read(&mut r)?, read(&mut r)?)),
            _ => return None,
        };
        r.done().then_some(Self { entries, fork })
    }
    pub fn head(&self) -> [u8; 32] {
        self.entries.last().map_or([0; 32], Entry::hash)
    }
    pub fn balance(&self) -> u128 {
        self.entries.last().map_or(0, |e| e.balance)
    }
}

/// The chains this chain must be shown — what the citations among its last [`RECENT`] entries cite — as
/// (definition, owner): a receive its sender's; a fill the taker's chain of the offer's token; a claim the taker's
/// chain (the lock) and this owner's chain of the lock's token (the fill); a refund the maker's chain of its lock's
/// token (no fill there).
pub fn cited_chains(p: &ChainParams, s: &State) -> Vec<(Tok, [u8; 32])> {
    cited_chains_from(p, s, s.entries.len().saturating_sub(RECENT))
}

/// The SUPPLY SHARDS the mints among the entries from index `from` on are in (their epochs; each once).
pub fn minted_epochs_from(s: &State, from: usize) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::new();
    for e in s.entries.iter().skip(from) {
        let at = match &e.body {
            Body::Mint { minted, .. } => supply::epoch(*minted),
            // (The record the credit NAMES — its size is its token's: checked against the definition in the walk.)
            Body::CappedMint { record, .. } => *record,
            _ => continue,
        };
        if !out.contains(&at) {
            out.push(at);
        }
    }
    out
}

/// The chains the citations of the entries from index `from` on name.
pub fn cited_chains_from(p: &ChainParams, s: &State, from: usize) -> Vec<(Tok, [u8; 32])> {
    let mut out: Vec<(Tok, [u8; 32])> = Vec::new();
    let mut add = |x: (Tok, [u8; 32])| {
        if !out.contains(&x) {
            out.push(x);
        }
    };
    for e in s.entries.iter().skip(from) {
        match &e.body {
            Body::Receive { from, .. } | Body::Vested { from, .. } | Body::FeeClaim { from, .. } => add((p.tok(), *from)),
            // The claimant's chain of the copy it holds.
            Body::HolderPayout { sold, copy, .. } => add((Tok { definition: sold.0.definition, item: numbered_item(&sold.0.definition, *copy) }, p.owner)),
            Body::Misdeed { tok, entry, .. } => {
                if let Some((from, _)) = misdeed_cites(entry) {
                    add((*tok, from));
                }
            }
            _ => {}
        }
    }
    out
}

/// What is unbonding in a chain: (unstake nonce, amount, release height) — from its unstakes, and its cut's.
pub fn unbonding_of(s: &State) -> Vec<([u8; 16], u128, u64)> {
    s.entries
        .iter()
        .flat_map(|e| match &e.body {
            Body::Unstake { amount } => vec![(e.nonce, *amount, e.stamp.saturating_add(UNBOND))],
            Body::Cut { unbonding, .. } => unbonding.clone(),
            _ => vec![],
        })
        .collect()
}


/// Is `pick`'s entry in chain `c` (id `cid`) — live, or, pruned behind its cut, shown by the evidence: a presence proof
/// that its history holds those entry bytes at the seq named.
fn find_cited(c: &State, cid: &[u8; 32], ev: &Evidence, pick: &dyn Fn(&Entry) -> bool) -> bool {
    if c.entries.iter().any(pick) {
        return true;
    }
    let Some(Entry { body: Body::Cut { history, .. }, .. }) = c.entries.first() else { return false };
    ev.theirs.iter().filter(|t| &t.0 == cid).any(|(_, seq, proof, bytes)| {
        let Some(x) = Entry::decode(bytes) else { return false };
        x.seq == *seq
            && x.seq < c.entries[0].seq
            && held(history, &history_key(*seq), proof, bytes)
            && pick(&x)
    })
}
/// Does `proof` show `key` holding exactly `bytes` under `root` (inline, or by a value block of their id and length)?
fn held(root: &[u8; 32], key: &[u8], proof: &[u8], bytes: &[u8]) -> bool {
    use freenet_prolly::proof::{ProvenOwned::Present, ProvenValue};
    match freenet_prolly::proof::verify_bytes(root, key, proof) {
        Ok(Present(ProvenValue::Inline(v))) => v == bytes,
        Ok(Present(ProvenValue::Ref { cid, len })) => len as usize == bytes.len() && cid == freenet_prolly::block_id(freenet_prolly::kind::RAW, bytes),
        _ => false,
    }
}
/// Does `proof` show `key` ABSENT under `root`?
fn absent(root: &[u8; 32], key: &[u8], proof: &[u8]) -> bool {
    matches!(freenet_prolly::proof::verify_bytes(root, key, proof), Ok(freenet_prolly::proof::ProvenOwned::Absent))
}

/// Why a chain is refused (named, for the tests and the node's log).
pub type Why = &'static str;

/// THE SHAPE RULES (TOKEN §3a, §3c) of chain `p` for definition `d` — true any time, from scratch. `chain(definition,
/// owner)` gives a chain the citations among the last [`RECENT`] entries cite (`None`: it does not exist).
pub fn check(p: &ChainParams, d: &Definition, s: &State, chain: &dyn Fn(&Tok, &[u8; 32]) -> Option<State>, supply: &dyn Fn(u64) -> Option<supply::Shard>, keys: Option<&craftworks_idlog_contract::Log>, x: &Extra) -> Result<(), Why> {
    check_window(p, d, s, chain, supply, keys, RECENT, x).map_err(|(_, why)| why)
}

/// The shape rules with the citations of the last `window` entries checked — the contract's [`RECENT`]; a WALLET's
/// every one (`usize::MAX`, TOKEN §3a). Refused: the entry where (its seq) and why.
#[allow(clippy::too_many_arguments)]
pub fn check_window(p: &ChainParams, d: &Definition, s: &State, chain: &dyn Fn(&Tok, &[u8; 32]) -> Option<State>, supply: &dyn Fn(u64) -> Option<supply::Shard>, keys: Option<&craftworks_idlog_contract::Log>, window: usize, x: &Extra) -> Result<(), (u64, Why)> {
    walk(p, d, s, chain, supply, keys, window, usize::MAX, x).map(|_| ())
}

/// What a chain carries at its end — what a cut after it must carry.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Carried {
    pub balance: u128,
    pub allowances: Vec<([u8; 32], u128)>,
    pub writer: Option<Signer>,
    pub staked: u128,
    pub unbonding: Vec<([u8; 16], u128, u64)>,
    /// Writer keys retired while stake was held (burnable until it is all released).
    pub retired: Vec<[u8; 32]>,
}

/// The shape rules, walked: what the chain carries at its end, or where and why it is refused. The citations of the last
/// `window` entries are checked; those of entries from index `strict_from` on (the NEW ones, as they are appended) must
/// SHOW what they cite — live, or by their evidence. An older one whose cited entry its chain's cut has since hidden is
/// taken as it was shown when appended (the wallet walks it whole).
/// `keys`: the OWNER's key log (the DID's), read and verified — needed where a `Writer` entry is checked.
#[allow(clippy::too_many_arguments)]
pub fn walk(p: &ChainParams, d: &Definition, s: &State, chain: &dyn Fn(&Tok, &[u8; 32]) -> Option<State>, supply: &dyn Fn(u64) -> Option<supply::Shard>, keys: Option<&craftworks_idlog_contract::Log>, window: usize, strict_from: usize, x: &Extra) -> Result<Carried, (u64, Why)> {
    let mut failed_at = s.entries.first().map_or(0, |e| e.seq);
    let r = (|| -> Result<Carried, Why> {
        if d.code_hash != p.code_hash {
            return Err("the definition is not this standard's");
        }
        // HOLDERS (and the freeze list's EVERYONE, the same bytes) is no one's: no chain of it (audit 10-10: a payout to it
        // would be a fee both stranded and claimed).
        if p.owner == HOLDERS {
            return Err("a chain of no one");
        }
        if d.beacon != p.beacon {
            return Err("a chain on another beacon than its token's");
        }
        if d.idlog != p.idlog {
            return Err("a chain on another key log than its token's");
        }
        if (d.ids == Ids::None) != (p.item == [0; 32]) {
            return Err("an item that is not its token's kind");
        }
        let mut balance: u128 = 0;
        // The SIGNER: one device of the owner DID, or the owner's key set; and whether anything but a signer came yet.
        let mut writer: Option<Signer> = None;
        let mut begun = false;
        let mut prev = [0u8; 32];
        let mut stamp: u64 = 0;
        // The names received, filled and claimed SINCE the last cut (before it: the cut's received set).
        let mut received: Vec<([u8; 32], [u8; 16])> = Vec::new();
        let mut vested: Vec<([u8; 32], [u8; 16], u32)> = Vec::new();
        let mut minted: Vec<[u8; 16]> = Vec::new();
        // ESCROWS not released yet (nonce, amount, program), and the payouts taken (program, key).
        let mut open_escrows: Vec<([u8; 16], u128, [u8; 32], u64)> = Vec::new();
        let mut paid: Vec<([u8; 32], [u8; 16])> = Vec::new();
        let mut shared: Vec<([u8; 32], [u8; 16])> = Vec::new();
        let mut fees: Vec<([u8; 32], [u8; 16])> = Vec::new();
        // The nonces of the transfers out (what a receive, a vesting step or a fee claim names one by).
        let mut debited: Vec<[u8; 16]> = Vec::new();
        // A program's standard section, read (`x.program`) and checked to hold.
        let section = |prog: &[u8; 32]| -> Result<escrow::Section, Why> {
            let b = x.program.and_then(|f| f(prog)).ok_or("the escrow's program is not there")?;
            let (sec, _) = escrow::Section::decode_prefix(&b).ok_or("a program with no escrow section")?;
            sec.holds()?;
            Ok(sec)
        };
        // ALLOWANCES (spender → what is left), OFFERS, LOCKS: carried across a cut by the cut itself.
        let mut allowed: Vec<([u8; 32], u128)> = Vec::new();
        // STAKE (TOKEN §7a): locked as witness stake, and what is on its way back (nonce, amount, release height).
        let mut staked: u128 = 0;
        // Standing epochs this chain minted keeper issuance for (each once).
        let mut kept: Vec<u64> = Vec::new();
        let mut unbonding: Vec<([u8; 16], u128, u64)> = Vec::new();
        let mut retired: Vec<[u8; 32]> = Vec::new();
        // The last cut: its stamp (what is stamped at or before it may be in its received set) and that set's root.
        let mut cut: Option<(u64, [u8; 32])> = None;
        let first = s.entries.first().map_or(0, |e| e.seq);
        let recent_from = s.entries.len().saturating_sub(window);
        let owner_signed = |e: &Entry, w: &Option<Signer>| if w.as_ref().is_some_and(|w| w.signs(e)) { Ok(()) } else { Err("an entry not signed by the chain's writer") };
        // A credential making a node key a DEVICE of this chain's owner (identity's one rule, `credential_node`), at entry
        // `i`. An entry HELD already (an update: those before what is new were taken by this holder, checked then) does
        // not ask for the key log again; anything new, and any check from scratch, checks it against the log. `why`: the
        // refusals for an unreadable credential, another account's, and one not of the key at its seq.
        let device = |cred: &[u8], key_seq: u32, i: usize, why: [Why; 3]| -> Result<[u8; 32], Why> {
            let held = strict_from <= s.entries.len() && i < strict_from;
            // (Whose it is first: another account's credential is refused before its key log is asked for.)
            let owner = match craftworks_identity::read_credential(cred) {
                None => return Err(why[0]),
                Some((did, _, _, _)) if did != p.owner => return Err(why[1]),
                Some((_, owner, _, _)) => owner,
            };
            let key_at = match keys {
                Some(log) => log.events.get(key_seq as usize).map(|ev| ev.key),
                None if held => Some(owner),
                None => return Err("the owner's key log is not there"),
            };
            craftworks_identity::credential_node(cred, &p.owner, key_at).map_err(|f| match f {
                craftworks_identity::CredFault::Unreadable => why[0],
                craftworks_identity::CredFault::OtherAccount => why[1],
                craftworks_identity::CredFault::NotThatKey => why[2],
            })
        };
        // Before a cut, a name stamped at or before it needs proof it is not in the cut's received set.
        let fresh = |cut: &Option<(u64, [u8; 32])>, cited: u64, key: Vec<u8>, ev: &Evidence| match cut {
            Some((at, root)) if cited <= *at => absent(root, &key, &ev.mine),
            _ => true,
        };
        for (i, e) in s.entries.iter().enumerate() {
            failed_at = e.seq;
            if e.seq != first + i as u64 {
                return Err("a seq out of place");
            }
            // A chain starts at seq 0 — or at its CUT (what came before pruned).
            let starts_at_cut = i == 0 && first > 0;
            if starts_at_cut && !matches!(e.body, Body::Cut { .. }) {
                return Err("a chain that starts mid-way without its cut");
            }
            if !starts_at_cut && e.prev != prev {
                return Err("a broken link");
            }
            if e.stamp < stamp {
                return Err("a stamp below its predecessor's");
            }
            let recent = i >= recent_from;
            let strict = i >= strict_from;
            // HIDDEN SINCE by a chain's cut: the cited chain cut at or after the stamp cited, and THIS entry was appended
            // before that cut (stamped no later than it, within the margin) — so it was shown when appended. One stamped
            // after must carry its evidence, even checked from scratch (a fresh copy cannot slip an unshown one in).
            let hidden_since = |c: &State, at: u64| {
                !strict && matches!(c.entries.first(), Some(x) if matches!(x.body, Body::Cut { .. }) && x.seq > 0 && x.stamp >= at && e.stamp <= x.stamp.saturating_add(d.margin))
            };
            // Shown: found live or by the evidence — or hidden since.
            let shown = |c: &State, cid: &[u8; 32], ev: &Evidence, at: u64, pick: &dyn Fn(&Entry) -> bool| find_cited(c, cid, ev, pick) || hidden_since(c, at);
            // What a transfer of `x` takes from the balance: `x` and its transfer fee (TOKEN §1e).
            let gross = |x: u128| x.checked_add(transfer_fee(d, &p.owner, x)).ok_or("a balance overflow");
            // A NON-TRANSFERABLE token never leaves the chain it is minted in.
            if d.soulbound && matches!(e.body, Body::Send { .. } | Body::Spend { .. } | Body::Allow { .. } | Body::Escrow { .. } | Body::Vest { .. }) {
                return Err("a transfer of a token that cannot be transferred");
            }
            // A transfer out is named by its nonce, once (audit 10-10: two under one nonce — the second's fee unclaimable).
            if matches!(e.body, Body::Send { .. } | Body::Spend { .. } | Body::Escrow { .. } | Body::Vest { .. }) {
                if debited.contains(&e.nonce) {
                    return Err("a transfer named as an earlier one");
                }
                debited.push(e.nonce);
            }
            balance = match &e.body {
                Body::Genesis { amount } => {
                    // The chain's first entry after its writer (from nothing: nothing before it but who writes).
                    if begun || first > 0 || d.ids != Ids::None || p.owner != d.genesis_owner || *amount != d.genesis_amount {
                        return Err("a genesis that is not the definition's");
                    }
                    owner_signed(e, &writer)?;
                    *amount
                }
                Body::Send { to, amount } => {
                    owner_signed(e, &writer)?;
                    if *to == p.owner {
                        return Err("a send to itself");
                    }
                    if *amount < d.min_send {
                        return Err("a send under the minimum");
                    }
                    balance.checked_sub(gross(*amount)?).ok_or("a debit over the balance")?
                }
                Body::Burn { amount } => {
                    owner_signed(e, &writer)?;
                    if *amount == 0 {
                        return Err("an empty burn");
                    }
                    balance.checked_sub(*amount).ok_or("a debit over the balance")?
                }
                Body::Allow { spender, limit } => {
                    owner_signed(e, &writer)?;
                    if matches!(&writer, Some(Signer::Device(node, _)) if spender == node) {
                        return Err("an allowance to itself");
                    }
                    allowed.retain(|(k, _)| k != spender);
                    if *limit > 0 {
                        allowed.push((*spender, *limit));
                    }
                    balance
                }
                Body::Spend { to, amount } => {
                    let Auth::Delegated { by, .. } = &e.auth else { return Err("a spend not signed by a spender") };
                    if !e.delegated_by(by) {
                        return Err("a spend not signed by a spender");
                    }
                    // Within an allowance of this chain; past it (or with none), by an OPERATOR of the owner for the
                    // whole token whose span covers this stamp (`ops`: the owner's operators list).
                    match allowed.iter_mut().find(|(k, _)| k == by) {
                        Some(left) if left.1 >= *amount => left.1 -= *amount,
                        _ if x.ops.is_some_and(|l| l.approved(by, e.stamp)) => {}
                        Some(_) => return Err("a spend over its allowance"),
                        None => return Err("a spend with no allowance"),
                    }
                    if *to == p.owner {
                        return Err("a send to itself");
                    }
                    if *amount < d.min_send {
                        return Err("a send under the minimum");
                    }
                    balance.checked_sub(gross(*amount)?).ok_or("a debit over the balance")?
                }
                Body::Receive { from, nonce, amount, cited, ev } => {
                    // (Never from itself: a send to itself is refused — and asking for its own chain as a related
                    // contract is an error at the node, not a refusal.)
                    if *from == p.owner {
                        return Err("a receive from itself");
                    }
                    if !e.worked() {
                        return Err("a receive without its work");
                    }
                    if *amount < d.min_send {
                        return Err("a receive under the minimum");
                    }
                    if received.contains(&(*from, *nonce)) || !fresh(&cut, *cited, received_key(RECEIVED, from, nonce), ev) {
                        return Err("a send received twice");
                    }
                    received.push((*from, *nonce));
                    if recent {
                        let cp = p.at(p.tok(), *from);
                        let c = chain(&p.tok(), from).ok_or("the sender's chain is not there")?;
                        // (A frozen sender's entries stop at its fork: a send after it — in the fork — is never found
                        // here, so every debit after an equivocation is void, TOKEN §3b.)
                        let pick = |x: &Entry| debit_for(x, &p.owner, e);
                        if !shown(&c, &cp.id(), ev, *cited, &pick) {
                            return Err("a receive whose send is not in its sender's chain");
                        }
                    }
                    balance.checked_add(*amount).ok_or("a balance overflow")?
                }
                Body::FeeClaim { from, nonce, gross: moved, amount, cited, ev } => {
                    if d.transfer_fee.is_none_or(|(_, _, c)| c != p.owner) {
                        return Err("a fee claimed by other than its collector");
                    }
                    if *from == p.owner {
                        return Err("a receive from itself");
                    }
                    if !e.worked() {
                        return Err("a receive without its work");
                    }
                    if *amount == 0 || *amount != transfer_fee(d, from, *moved) {
                        return Err("a fee that is not the definition's");
                    }
                    if fees.contains(&(*from, *nonce)) || !fresh(&cut, *cited, received_key(FEE, from, nonce), ev) {
                        return Err("a fee claimed twice");
                    }
                    fees.push((*from, *nonce));
                    if recent {
                        let cp = p.at(p.tok(), *from);
                        let c = chain(&p.tok(), from).ok_or("the sender's chain is not there")?;
                        let pick = |x: &Entry| debit_for(x, &p.owner, e);
                        if !shown(&c, &cp.id(), ev, *cited, &pick) {
                            return Err("a fee whose transfer is not in its payer's chain");
                        }
                    }
                    balance.checked_add(*amount).ok_or("a balance overflow")?
                }
                Body::Escrow { program: prog, terms, amount, until } => {
                    owner_signed(e, &writer)?;
                    if *amount < d.min_send || terms.len() > escrow::TERMS_MAX {
                        return Err("a send under the minimum");
                    }
                    if open_escrows.iter().any(|o| o.0 == e.nonce) {
                        return Err("an escrow named twice");
                    }
                    open_escrows.push((e.nonce, *amount, *prog, *until));
                    balance.checked_sub(gross(*amount)?).ok_or("a debit over the balance")?
                }
                Body::Unescrow { escrow: nonce_, refund, verdict } => {
                    if !e.worked() {
                        return Err("a release without its work");
                    }
                    let i = open_escrows.iter().position(|o| &o.0 == nonce_).ok_or("a release of no open escrow")?;
                    if *refund > open_escrows[i].1 {
                        return Err("a refund past what the escrow held");
                    }
                    if recent {
                        let sec = section(&open_escrows[i].2)?;
                        if sec.refund_of(&(p.tok(), p.owner, *nonce_)) != Some(*refund) || sec.hash() != *verdict {
                            return Err("a release that is not its program's verdict");
                        }
                    }
                    open_escrows.remove(i);
                    balance.checked_add(*refund).ok_or("a balance overflow")?
                }
                Body::Reclaim { escrow: nonce_ } => {
                    if !e.worked() {
                        return Err("a release without its work");
                    }
                    let i = open_escrows.iter().position(|o| &o.0 == nonce_).ok_or("a release of no open escrow")?;
                    let (_, amount, _, until) = open_escrows[i];
                    if until == 0 {
                        return Err("a reclaim of an escrow that has none");
                    }
                    // Its time, from scratch too (as a vesting step's): a first copy cannot reclaim early (audit 10-10).
                    if e.stamp < until {
                        return Err("a reclaim before its escrow's time");
                    }
                    // That its program never took it is the WITNESSES' to check: the program may not exist (a wrong
                    // id), and a node refuses outright an update asking for a contract that is not there (measured
                    // 10-10: "missing related contract") — so the chain never asks for it.
                    open_escrows.remove(i);
                    balance.checked_add(amount).ok_or("a balance overflow")?
                }
                Body::Payout { program: prog, key, amount, verdict, ev } => {
                    if !e.worked() {
                        return Err("a payout without its work");
                    }
                    // (No stamp of its own to place it before or after a cut — a verdict has none: after ANY cut, its absence proof.)
                    if paid.contains(&(*prog, *key)) || !fresh(&cut, 0, received_key(PAYOUT, prog, key), ev) {
                        return Err("a payout received twice");
                    }
                    paid.push((*prog, *key));
                    if recent {
                        let sec = section(prog)?;
                        if sec.pay(key) != Some((p.tok(), p.owner, *amount)) || sec.hash() != *verdict {
                            return Err("a payout that is not its program's verdict");
                        }
                        if !royalty_paid(d, p, &sec) {
                            return Err("a sale that does not pay its creator's royalty");
                        }
                    }
                    balance.checked_add(*amount).ok_or("a balance overflow")?
                }
                Body::HolderPayout { program: prog, sold, copy, at, amount, verdict, ev } => {
                    if !e.worked() {
                        return Err("a payout without its work");
                    }
                    let name = holder_nonce(sold, *copy);
                    if shared.contains(&(*prog, name)) || !fresh(&cut, 0, received_key(HOLDERS_SHARE, prog, &name), ev) {
                        return Err("a payout received twice");
                    }
                    shared.push((*prog, name));
                    if recent {
                        let edition = &sold.0.definition;
                        let sec = section(prog)?;
                        if sec.hash() != *verdict {
                            return Err("a holders' share that is not its program's verdict");
                        }
                        let db = x.program.and_then(|f| f(edition)).ok_or("the edition's definition is not there")?;
                        let de = Definition::decode(&db).filter(|_| id(&p.code_hash, &db) == *edition).ok_or("the edition's definition is not there")?;
                        if de.ids != Ids::Numbered || de.cap < 2 || de.royalty.is_none_or(|(_, to)| to != HOLDERS) {
                            return Err("a holders' share of an edition that shares none");
                        }
                        // The sale: the sold copy's escrow is the program's, and its fee is paid to the holders.
                        if !sec.escrows.iter().any(|(r, ..)| r == sold) {
                            return Err("a holders' share of a sale that is not its program's");
                        }
                        let item = numbered_item(edition, *copy);
                        if *copy == 0 || *copy > de.cap || item == sold.0.item {
                            return Err("a holders' share of a copy that takes none");
                        }
                        let Some((t, to, fee)) = sec.pay(&escrow::fee_key(sold)) else { return Err("a holders' share that is not its program's verdict") };
                        if t != p.tok() || to != HOLDERS || *amount != fee / (de.cap - 1) {
                            return Err("a holders' share that is not its program's verdict");
                        }
                        // Its holder WHEN THE COPY WAS SOLD (`at`): this owner's chain of the copy held it then.
                        let held = chain(&Tok { definition: *edition, item }, &p.owner).ok_or("a holders' share of a copy not held here")?;
                        if held_at(&held, *at) != Some(1) {
                            return Err("a holders' share of a copy not held here at the sale");
                        }
                    }
                    balance.checked_add(*amount).ok_or("a balance overflow")?
                }
                Body::Vest { to, schedule } => {
                    owner_signed(e, &writer)?;
                    if *to == p.owner {
                        return Err("a vest to itself");
                    }
                    if schedule.is_empty() || schedule.len() > VEST_STEPS || schedule.windows(2).any(|w| w[0].0 >= w[1].0) {
                        return Err("a vesting schedule that is not rising steps");
                    }
                    if schedule.iter().any(|(_, a)| *a < d.min_send) {
                        return Err("a send under the minimum");
                    }
                    let total = schedule.iter().try_fold(0u128, |t, (_, a)| t.checked_add(*a)).ok_or("a balance overflow")?;
                    balance.checked_sub(gross(total)?).ok_or("a debit over the balance")?
                }
                Body::Vested { from, vest, step, amount, at, cited, ev } => {
                    if *from == p.owner {
                        return Err("a receive from itself");
                    }
                    if !e.worked() {
                        return Err("a receive without its work");
                    }
                    // Its time, from scratch (a first copy is taken without the append rules): stamped at or after
                    // its step — and no witness signs an entry stamped ahead of the beacon (`wallet::may_witness`).
                    if e.stamp < *at {
                        return Err("a vesting step before its time");
                    }
                    if vested.contains(&(*from, *vest, *step)) || !fresh(&cut, *cited, vested_key(from, vest, *step), ev) {
                        return Err("a vesting step received twice");
                    }
                    vested.push((*from, *vest, *step));
                    if recent {
                        let cp = p.at(p.tok(), *from);
                        let c = chain(&p.tok(), from).ok_or("the sender's chain is not there")?;
                        let pick = |x: &Entry| debit_for(x, &p.owner, e);
                        if !shown(&c, &cp.id(), ev, *cited, &pick) {
                            return Err("a vesting step that is not in its giver's chain");
                        }
                    }
                    balance.checked_add(*amount).ok_or("a balance overflow")?
                }
                Body::Mint { nonce, amount, minted: at, mint, ev } | Body::CappedMint { nonce, amount, minted: at, mint, ev, .. } => {
                    let capped = matches!(e.body, Body::CappedMint { .. });
                    if capped != d.capped() {
                        return Err("a mint of the other kind than its token's");
                    }
                    if !e.worked() {
                        return Err("a mint without its work");
                    }
                    if *at > e.stamp {
                        return Err("a mint credited before it was minted");
                    }
                    if minted.contains(nonce) || !fresh(&cut, *at, received_key(MINTED, &MINTER, nonce), ev) {
                        return Err("a mint credited twice");
                    }
                    minted.push(*nonce);
                    // The MINT it carries is this credit's — to this owner, at this stamp, of these numbers (a numbered
                    // copy: one of its range): what a witness signs on, and what a false one is burned by.
                    let numbered = capped && d.ids == Ids::Numbered;
                    let n = u128::from_be_bytes(*nonce);
                    let mine = mint.to == p.owner
                        && mint.stamp == *at
                        && if numbered {
                            *amount == 1 && mint.item == [0; 32] && u128::from_be_bytes(mint.nonce) <= n && n - u128::from_be_bytes(mint.nonce) < mint.amount
                        } else {
                            mint.nonce == *nonce && mint.amount == *amount && mint.item == p.item
                        };
                    if !mine {
                        return Err("a mint credit that is not its mint's");
                    }
                    // In its epoch's supply shard ALREADY (the shard holds only the mint authority's signed mints).
                    if recent {
                        let at_shard = if capped {
                            let k = supply::record_of(d, u128::from_be_bytes(*nonce)).ok_or("a mint of numbers not its record's")?;
                            if !matches!(e.body, Body::CappedMint { record, .. } if record == k) {
                                return Err("a mint credit naming another record than its numbers'");
                            }
                            k
                        } else {
                            supply::epoch(*at)
                        };
                        let shard = supply(at_shard).ok_or("the mint's supply shard is not there")?;
                        let ok = if !capped {
                            shard.find(nonce) == Some(mint)
                        } else if numbered {
                            // Copy `n`: in an ADMITTED mint of a range of numbers holding it, to this owner; one copy.
                            p.item == numbered_item(&p.definition, n) && supply::covering(d, &shard, n) == Some(mint)
                        } else {
                            supply::admitted(d, &shard).contains(&mint)
                        };
                        if !ok {
                            return Err("a mint that is not in its supply shard");
                        }
                    }
                    balance.checked_add(*amount).ok_or("a balance overflow")?
                }
                Body::Writer { cred, key_seq } => {
                    let node = device(cred, *key_seq, i, ["a writer credential that does not hold", "a writer credential for another account", "a writer credential not signed by the account's key at that seq"])?;
                    match &writer {
                        Some(Signer::Set(..)) => return Err("a device writer on a key set's chain"),
                        Some(Signer::Device(_, k)) if key_seq < k => return Err("a writer on an older key than the one before"),
                        _ => {}
                    }
                    if !e.signed_by(&node) {
                        return Err("a writer entry not signed by its device");
                    }
                    // A key retired while stake is held stays burnable: a witness cannot escape its burn by a new writer.
                    // (With nothing staked or unbonding the list is emptied after every entry, below.)
                    if let Some(Signer::Device(old, _)) = &writer {
                        if *old != node && !retired.contains(old) {
                            if retired.len() >= RETIRED_MAX {
                                return Err("too many writer keys retired while staked");
                            }
                            retired.push(*old);
                        }
                    }
                    writer = Some(Signer::Device(node, *key_seq));
                    balance
                }
                Body::Members { set, set_seq } => {
                    if !set.sound() {
                        return Err("a key set that cannot sign");
                    }
                    match &writer {
                        // The first: the set the owner id names, signed by k of it.
                        None => {
                            if *set_seq != 0 || set_id(set) != p.owner {
                                return Err("a key set that is not the chain's owner");
                            }
                            owner_signed(e, &Some(Signer::Set(set.clone(), 0)))?;
                        }
                        // A change: the next set seq, signed by k of the set before.
                        Some(Signer::Set(_, k)) => {
                            if *set_seq != k + 1 {
                                return Err("a key set change out of place");
                            }
                            owner_signed(e, &writer)?;
                        }
                        Some(Signer::Device(..)) => return Err("a key set on a device's chain"),
                    }
                    writer = Some(Signer::Set(set.clone(), *set_seq));
                    balance
                }
                Body::KeeperMint { epoch, root, proof, amount } => {
                    // Anyone's, with its work: its LINE fixes what it mints (the witnesses check the root was the one
                    // accepted for the epoch).
                    if !e.worked() {
                        return Err("a keeper mint without its work");
                    }
                    d.keepers.ok_or("a keeper mint of a token without keeper issuance")?;
                    if e.nonce != keeper_nonce(&p.definition, *epoch) || kept.contains(epoch) {
                        return Err("a keeper mint named for another epoch, or twice");
                    }
                    kept.push(*epoch);
                    let line = craftworks_reputation::statement::line_proven(root, &p.owner, proof).ok_or("a keeper mint whose line does not hold under its root")?;
                    if *amount == 0 || *amount != line.keeper {
                        return Err("a keeper mint of another amount than its line's");
                    }
                    // Its statement exists once its epoch has begun (past the margin).
                    let begun = epoch.checked_mul(craftworks_standing::EPOCH).and_then(|x| x.checked_add(d.margin as u64));
                    if begun.is_none_or(|b| e.stamp < b) {
                        return Err("a keeper mint before its epoch's statement");
                    }
                    staked = staked.checked_add(*amount).ok_or("a balance overflow")?;
                    balance
                }
                Body::Stake { amount } => {
                    owner_signed(e, &writer)?;
                    if *amount == 0 {
                        return Err("a stake of nothing");
                    }
                    staked = staked.checked_add(*amount).ok_or("a balance overflow")?;
                    balance.checked_sub(*amount).ok_or("a debit over the balance")?
                }
                Body::Unstake { amount } => {
                    owner_signed(e, &writer)?;
                    if *amount == 0 || unbonding.iter().any(|u| u.0 == e.nonce) {
                        return Err("an unstake of nothing");
                    }
                    staked = staked.checked_sub(*amount).ok_or("an unstake over the stake")?;
                    unbonding.push((e.nonce, *amount, e.stamp.saturating_add(UNBOND)));
                    balance
                }
                Body::Release { unstake } => {
                    if !e.worked() {
                        return Err("a release without its work");
                    }
                    let i = unbonding.iter().position(|u| &u.0 == unstake).ok_or("a release of nothing unbonding")?;
                    if e.stamp < unbonding[i].2 {
                        return Err("a release before its delay");
                    }
                    let (_, amount, _) = unbonding.remove(i);
                    balance.checked_add(amount).ok_or("a balance overflow")?
                }
                Body::Slash { key, a, b } => {
                    if !e.worked() {
                        return Err("a slash without its work");
                    }
                    // The key whose witness signatures they are: this chain's writer (its node key), or one it retired
                    // while staked.
                    if !matches!(&writer, Some(Signer::Device(node, _)) if node == key) && !retired.contains(key) {
                        return Err("a slash of a key that is not this chain's");
                    }
                    if !witness::equivocation(key, a, b) {
                        return Err("a slash with no equivocation");
                    }
                    staked = 0;
                    unbonding.clear();
                    balance
                }
                Body::Misdeed { key, sig, tok, owner, entry } => {
                    if !e.worked() {
                        return Err("a slash without its work");
                    }
                    if !matches!(&writer, Some(Signer::Device(node, _)) if node == key) && !retired.contains(key) {
                        return Err("a slash of a key that is not this chain's");
                    }
                    if !sig.signed_by(key) || sig.entry != entry.hash() || sig.seq != entry.seq || sig.chain != p.at(*tok, *owner).id() {
                        return Err("a misdeed that is not this key's signature on that entry");
                    }
                    let (from, cited) = misdeed_cites(entry).ok_or("a misdeed of an entry it cannot show")?;
                    if recent {
                        let c = chain(tok, &from).ok_or("the sender's chain is not there")?;
                        // Only where the cited chain can say: unfrozen, and whole back to (before) the debit's stamp.
                        if c.fork.is_some() || !c.entries.first().is_some_and(|f| f.seq == 0 || f.stamp < cited) {
                            return Err("a misdeed the cited chain cannot show");
                        }
                        if c.entries.iter().any(|x| debit_for(x, owner, entry)) {
                            return Err("a misdeed whose debit is there");
                        }
                    }
                    staked = 0;
                    unbonding.clear();
                    balance
                }
                Body::FalseMint { key, sig, tok, owner, entry } => {
                    if !e.worked() {
                        return Err("a slash without its work");
                    }
                    if !matches!(&writer, Some(Signer::Device(node, _)) if node == key) && !retired.contains(key) {
                        return Err("a slash of a key that is not this chain's");
                    }
                    if !sig.signed_by(key) || sig.entry != entry.hash() || sig.seq != entry.seq || sig.chain != p.at(*tok, *owner).id() {
                        return Err("a misdeed that is not this key's signature on that entry");
                    }
                    let (Body::Mint { mint: m, .. } | Body::CappedMint { mint: m, .. }) = &entry.body else { return Err("a false mint of no mint credit") };
                    if recent {
                        // The token's mint authority at the mint's stamp (its definition and its authority chain — which
                        // only ever extends): did it sign this mint?
                        let db = x.program.and_then(|f| f(&tok.definition)).ok_or("the minted token's definition is not there")?;
                        let dm = Definition::decode(&db).filter(|_| id(&p.code_hash, &db) == tok.definition).ok_or("the minted token's definition is not there")?;
                        if dm.beacon != p.beacon {
                            return Err("a false mint of a token on another beacon");
                        }
                        let ap = authority::Params { code_hash: p.code_hash, definition: tok.definition, beacon: p.beacon };
                        let ch = x.program.and_then(|f| f(&ap.id())).and_then(|b| authority::Chain::decode(&b)).ok_or("the minted token's authority chain is not there")?;
                        if let Some((cid, nonce)) = m.paid {
                            // A PAID mint: no payment in its payer's chain — shown where the chain can say (unforked,
                            // whole back to before the mint), as a vanished debit is.
                            let (ptok, price, payee) = dm.sale.ok_or("a false mint that was paid")?;
                            let payer = ChainParams { code_hash: p.code_hash, definition: ptok.definition, owner: m.to, beacon: p.beacon, idlog: dm.idlog, item: ptok.item };
                            let c = (payer.id() == cid).then(|| x.program.and_then(|f| f(&cid)).and_then(|b| State::decode(&b))).flatten().ok_or("the payer's chain is not there")?;
                            if c.fork.is_some() || !c.entries.first().is_some_and(|f| f.seq == 0 || f.stamp < m.stamp) {
                                return Err("a misdeed the cited chain cannot show");
                            }
                            let owed = price.saturating_mul(m.amount);
                            if c.entries.iter().any(|y| y.nonce == nonce && y.stamp <= m.stamp && matches!(y.body, Body::Send { to, amount } if to == payee && amount == owed)) {
                                return Err("a false mint that was paid");
                            }
                        } else {
                            let (set, _) = authority::sets_at(&tok.definition, (&dm.mint, &dm.freeze), &ch, m.stamp)?;
                            if set.is_some_and(|set| set.signed(&m.message(&tok.definition), &m.sigs)) {
                                return Err("a false mint that its authority signed");
                            }
                        }
                    }
                    staked = 0;
                    unbonding.clear();
                    balance
                }
                Body::FalseVerdict { key, sig, tok, owner, entry, section } => {
                    if !e.worked() {
                        return Err("a slash without its work");
                    }
                    if !matches!(&writer, Some(Signer::Device(node, _)) if node == key) && !retired.contains(key) {
                        return Err("a slash of a key that is not this chain's");
                    }
                    if !sig.signed_by(key) || sig.entry != entry.hash() || sig.seq != entry.seq || sig.chain != p.at(*tok, *owner).id() {
                        return Err("a misdeed that is not this key's signature on that entry");
                    }
                    // The verdict the payout NAMED (its hash: no other section has it) — and it does not pay it.
                    let sec = escrow::Section::decode_prefix(section).map(|x| x.0).ok_or("a false verdict that is no section")?;
                    let pays = match &entry.body {
                        Body::Payout { key: k, amount, verdict, .. } if sec.hash() == *verdict => sec.pay(k) == Some((*tok, *owner, *amount)),
                        Body::HolderPayout { sold, verdict, .. } if sec.hash() == *verdict => {
                            sec.escrows.iter().any(|(r, ..)| r == sold) && sec.pay(&escrow::fee_key(sold)).is_some_and(|(t, to, _)| t == *tok && to == HOLDERS)
                        }
                        _ => return Err("a false verdict that is not the one the payout named"),
                    };
                    if pays {
                        return Err("a false verdict that pays it");
                    }
                    staked = 0;
                    unbonding.clear();
                    balance
                }
                Body::Cut { received: root, history: hroot, allowances, writer: w, staked: st, unbonding: ub, retired: rt, .. } => {
                    // (An escrow comes back before its chain is cut: a cut carries none.)
                    if !open_escrows.is_empty() {
                        return Err("a cut over an open escrow");
                    }
                    if starts_at_cut {
                        // The chain's writer as the cut carries it (who signs from here — and the cut itself).
                        writer = w.clone();
                    } else if writer != *w {
                        return Err("a cut that is not what came before it");
                    }
                    owner_signed(e, &writer)?;
                    if starts_at_cut {
                        // The chain's start: what came before is pruned — the cut's state is taken (checked by the
                        // nodes that held the prefix when it was written, and by wallets walking its history).
                        allowed = allowances.clone();
                        staked = *st;
                        unbonding = ub.clone();
                        retired = rt.clone();
                        e.balance
                    } else {
                        // The prefix is here: the cut must carry exactly what it computes.
                        if sorted(&allowed) != sorted(allowances) || staked != *st || sorted(&unbonding) != sorted(ub) || sorted(&retired) != sorted(rt) {
                            return Err("a cut that is not what came before it");
                        }
                        // Held from seq 0: its TREES too — the history and the received set it names must be the ones
                        // what came before builds (audit 10-10: an empty received root let a send be received again).
                        if first == 0 {
                            let (h, r) = cut::trees(&s.entries[..i], &mut cut::Blocks::new())?;
                            if h.0 != *hroot || r.0 != *root {
                                return Err("a cut that is not what came before it");
                            }
                        }
                        received.clear();
                        vested.clear();
                        balance
                    }
                    .tap(|_| cut = Some((e.stamp, *root)))
                }
            };
            if e.balance != balance {
                return Err("a carried balance that does not add up");
            }
            if !matches!(e.body, Body::Writer { .. } | Body::Members { .. }) {
                begun = true;
            }
            if staked == 0 && unbonding.is_empty() {
                retired.clear();
            }
            prev = e.hash();
            stamp = e.stamp;
        }
        failed_at = first + s.entries.len() as u64;
        if let Some((a, b)) = &s.fork {
            let n = first + s.entries.len() as u64;
            if a.seq != n || b.seq != n || a.prev != prev || b.prev != prev || a == b || a.hash() >= b.hash() {
                return Err("a fork that is not two writer entries at the next seq, in order");
            }
            if !writer.as_ref().is_some_and(|w| w.signs(a) && w.signs(b)) {
                return Err("a fork not signed twice by the chain's writer");
            }
        }
        Ok(Carried { balance, allowances: allowed, writer, staked, unbonding, retired })
    })();
    r.map_err(|why| (failed_at, why))
}

/// `x.tap(f)`: run `f` on it, give it back (a cut's arm records the cut and yields the balance).
fn sorted<T: Ord + Clone>(v: &[T]) -> Vec<T> {
    let mut v = v.to_vec();
    v.sort();
    v
}

trait Tap: Sized {
    fn tap(self, f: impl FnOnce(&Self)) -> Self {
        f(&self);
        self
    }
}
impl<T> Tap for T {}

/// THE APPEND RULES (TOKEN §3c), checked by a node that already HOLDS the chain as it takes in what it did not have
/// (`held` → `merged`), against the beacon's height NOW: each new entry's stamp within a margin of now; anything citing
/// another entry no sooner than a margin after it (final everywhere); a fill a margin before its lock's deadline; a
/// refund at or after it.
/// And (TOKEN §1) every new OWNER entry stands on the owner's CURRENT key (`keys`: the DID's key log): a new writer is
/// credentialed under it, and what a writer signs is signed under one credentialed under it — a rotation revokes every
/// older device for new writes.
pub fn append_check(d: &Definition, now: u64, held: &State, merged: &State, keys: Option<&craftworks_idlog_contract::Log>) -> Result<(), Why> {
    let had: Vec<[u8; 32]> = held.entries.iter().map(Entry::hash).collect();
    let m = d.margin;
    // No entry held PAST ITS FINALITY is replaced (an owner's recovery outranks a lost device's entry only within the
    // margin — after it, what others credited from it stands; audit 10-10: a late override undid an accepted send).
    for h in &held.entries {
        if merged.entries.iter().any(|x| x.seq == h.seq && x.hash() != h.hash()) && h.stamp.saturating_add(m) < now {
            return Err("an entry past its finality replaced");
        }
    }
    for (i, e) in merged.entries.iter().enumerate().filter(|(_, e)| !had.contains(&e.hash())) {
        if e.stamp > now || e.stamp.saturating_add(m) < now {
            return Err("a stamp far from the beacon");
        }
        on_current_key(merged, i, keys)?;
        timed(d, now, merged, e)?;
    }
    Ok(())
}

/// An OWNER entry (entry `i` of `s`) is on the account's CURRENT key (its key log's head) — the append rule, and a
/// witness's (audit 10-10: from scratch a writer on any past key holds, so a device stolen before a rotation could have
/// its entries witnessed). A KEY SET's chain names its own signers: no key log stands behind them.
pub fn on_current_key(s: &State, i: usize, keys: Option<&craftworks_idlog_contract::Log>) -> Result<(), Why> {
    let e = &s.entries[i];
    if !e.by_owner() || is_set_chain(s) {
        return Ok(());
    }
    let current = keys.ok_or("the owner's key log is not there")?.head().seq;
    let on = match &e.body {
        Body::Writer { key_seq, .. } => Some(*key_seq),
        // (A cut is signed by the writer it carries.)
        Body::Cut { writer, .. } => writer.as_ref().map(Signer::rank),
        _ => writer_at(s, i).as_ref().map(Signer::rank),
    };
    if on != Some(current) {
        return Err("an owner entry not on the account's current key");
    }
    Ok(())
}

/// What a holder KEEPS of `merged` (what it held, `held`, and what it was given): the append rules for what is new; the
/// shape rules with every NEW citation shown (live, or by its evidence — `chain` gives the cited chains) and a new cut
/// checked against what it summarises here (it must carry exactly what that computes); then what comes before the last
/// cut dropped.
#[allow(clippy::too_many_arguments)]
pub fn accept(p: &ChainParams, d: &Definition, now: u64, held: &State, merged: &State, chain: &dyn Fn(&Tok, &[u8; 32]) -> Option<State>, supply: &dyn Fn(u64) -> Option<supply::Shard>, keys: Option<&craftworks_idlog_contract::Log>, x: &Extra) -> Result<State, Why> {
    append_check(d, now, held, merged, keys)?;
    let from = new_from(held, merged);
    walk(p, d, merged, chain, supply, keys, merged.entries.len() - from, from, x).map_err(|(_, w)| w)?;
    Ok(prune(merged))
}

/// What a copy of `s` checked FROM SCRATCH must be shown, in one round: its definition, its owner's key log (if it
/// names a writer), every chain its recent citations name, its recent mints' supply shards, its operators list (if a
/// spend needs it), and the programs (and editions) of its recent releases and payouts.
pub fn validate_wants(p: &ChainParams, s: &State) -> Vec<[u8; 32]> {
    let mut want = vec![p.definition];
    if s.entries.iter().any(|e| matches!(e.body, Body::Writer { .. })) {
        want.push(p.keys_id());
    }
    want.extend(cited_chains(p, s).into_iter().map(|(d, o)| p.at(d, o).id()));
    let shard_id = |e: u64| supply::Params { code_hash: p.code_hash, definition: p.definition, beacon: p.beacon, epoch: e }.id();
    want.extend(minted_epochs_from(s, s.entries.len().saturating_sub(RECENT)).into_iter().map(shard_id));
    want.extend(needs_operators(s).then(|| operators::Params::of(p).id()));
    want.extend(programs_from(p, s, s.entries.len().saturating_sub(RECENT)));
    want.sort();
    want.dedup();
    want
}

/// Does anything new in `merged` (from index `from`) DEBIT the chain?
pub fn debits_from(merged: &State, from: usize) -> bool {
    (from..merged.entries.len()).any(|i| merged.entries[i].balance < if i == 0 { 0 } else { merged.entries[i - 1].balance })
}

/// THE TIMED RULES (TOKEN §3b, §4a), of entry `e` of `merged` with the beacon at `now`: a citation no sooner than a
/// margin after what it cites; a fill a margin before its lock's deadline; a refund at or after it; a release once
/// its delay is over. Appending checks them against the beacon now; a WITNESS checks them at the entry's own stamp
/// (`wallet::may_witness`) — so a first copy, which a node takes without the append rules, gains nothing by them.
pub fn timed(d: &Definition, now: u64, merged: &State, e: &Entry) -> Result<(), Why> {
    let m = d.margin;
    match &e.body {
        Body::Receive { cited, .. } | Body::Vested { cited, .. } | Body::FeeClaim { cited, .. } => {
            if e.stamp < cited.saturating_add(m) {
                return Err("an entry citing one not yet final");
            }
        }
        Body::Reclaim { escrow } => {
            let until = merged.entries.iter().find_map(|x| match &x.body {
                Body::Escrow { until, .. } if &x.nonce == escrow => Some(*until),
                _ => None,
            });
            if until.is_none_or(|u| e.stamp < u) {
                return Err("a reclaim before its escrow's time");
            }
        }
        Body::Release { unstake } => {
            // (Its release height from its unstake, or from the cut that carries it: the BEACON past it, not the
            // entry's own stamp.)
            if unbonding_of(merged).into_iter().find(|u| &u.0 == unstake).is_none_or(|u| now < u.2) {
                return Err("a release before its delay");
            }
        }
        _ => {}
    }
    Ok(())
}

/// The chain and stamp a credit's cited debit is in — what a MISDEED is shown against: a receive's send (spend, fill),
/// a vesting step's vest.
fn misdeed_cites(e: &Entry) -> Option<([u8; 32], u64)> {
    match &e.body {
        Body::Receive { from, cited, .. } | Body::Vested { from, cited, .. } | Body::FeeClaim { from, cited, .. } => Some((*from, *cited)),
        _ => None,
    }
}

/// Is `x` (in the cited chain) the debit the credit `e` of `owner`'s chain cites — as the contract matches it?
pub(crate) fn debit_for(x: &Entry, owner: &[u8; 32], e: &Entry) -> bool {
    match &e.body {
        Body::Receive { nonce, amount, cited, .. } => {
            x.nonce == *nonce && x.stamp == *cited && matches!(x.body, Body::Send { to, amount: a } | Body::Spend { to, amount: a } if to == *owner && a == *amount)
        }
        Body::Vested { vest, step, amount, at, cited, .. } => {
            x.nonce == *vest && x.stamp == *cited && matches!(&x.body, Body::Vest { to, schedule } if to == owner && schedule.get(*step as usize) == Some(&(*at, *amount)))
        }
        // A fee: any transfer of that amount (its receiver is not the collector's business).
        Body::FeeClaim { nonce, gross, cited, .. } => {
            x.nonce == *nonce
                && x.stamp == *cited
                && match &x.body {
                    Body::Send { amount, .. } | Body::Spend { amount, .. } | Body::Escrow { amount, .. } => amount == gross,
                    Body::Vest { schedule, .. } => schedule.iter().try_fold(0u128, |t, s| t.checked_add(s.1)) == Some(*gross),
                    _ => false,
                }
        }
        _ => false,
    }
}

/// The PROGRAMS the releases and payouts among the entries from index `from` on are checked against (each once).
pub fn programs_from(p: &ChainParams, s: &State, from: usize) -> Vec<[u8; 32]> {
    let mut out: Vec<[u8; 32]> = Vec::new();
    for (i, e) in s.entries.iter().enumerate().skip(from) {
        let prog = match &e.body {
            Body::Payout { program, .. } => Some(*program),
            Body::FalseMint { tok, entry, .. } => {
                // The minted token's definition and authority chain (read by id, as a program's state is) — and a paid
                // mint's payer chain.
                let a = authority::Params { code_hash: p.code_hash, definition: tok.definition, beacon: p.beacon }.id();
                let payer = match &entry.body {
                    Body::Mint { mint, .. } | Body::CappedMint { mint, .. } => mint.paid.map(|(c, _)| c),
                    _ => None,
                };
                for d in [tok.definition, a].into_iter().chain(payer) {
                    if !out.contains(&d) {
                        out.push(d);
                    }
                }
                None
            }
            Body::HolderPayout { program, sold, .. } => {
                // (Its edition's definition too: read by id, as a program's state is.)
                if !out.contains(&sold.0.definition) {
                    out.push(sold.0.definition);
                }
                Some(*program)
            }
            Body::Unescrow { escrow, .. } => s.entries[..i].iter().find_map(|x| match &x.body {
                Body::Escrow { program, .. } if &x.nonce == escrow => Some(*program),
                _ => None,
            }),
            _ => None,
        };
        if let Some(pr) = prog {
            if !out.contains(&pr) {
                out.push(pr);
            }
        }
    }
    out
}

/// Does `s` hold a spend its own allowances do not cover — one only an OPERATOR of the owner can stand behind? Only
/// then is the owner's operators list read (most owners never make one; asking for a contract that is not there costs
/// the node a lookup).
pub fn needs_operators(s: &State) -> bool {
    let mut allowed: Vec<([u8; 32], u128)> = Vec::new();
    for e in &s.entries {
        match &e.body {
            Body::Allow { spender, limit } => {
                allowed.retain(|(k, _)| k != spender);
                if *limit > 0 {
                    allowed.push((*spender, *limit));
                }
            }
            Body::Cut { allowances, .. } => allowed = allowances.clone(),
            Body::Spend { amount, .. } => {
                let Auth::Delegated { by, .. } = &e.auth else { continue };
                match allowed.iter_mut().find(|(k, _)| k == by) {
                    Some(left) if left.1 >= *amount => left.1 -= *amount,
                    _ => return true,
                }
            }
            _ => {}
        }
    }
    false
}

/// THE FREEZE RULE (an append rule, TOKEN §2): a token with a freeze authority appends no new DEBIT of an owner its
/// freeze list (as held) says is frozen.
pub fn freeze_check(p: &ChainParams, d: &Definition, held: &State, merged: &State, list: &freeze::List) -> Result<(), Why> {
    if d.freeze.is_some() && debits_from(merged, new_from(held, merged)) && list.frozen(&p.owner) {
        return Err("a debit of a frozen account");
    }
    Ok(())
}

/// A sale's ROYALTY (TOKEN §1b), for ANY program: an item of a token with a royalty, paid out of a program (bought),
/// only if that program also pays its CREATOR — in every token the item's sellers are paid in — at least its share of
/// what they are paid (a seller who IS the creator owes nothing). The sellers: the owners of the escrows of this item
/// the program holds. Read from the program's section alone: the token knows nothing of the program.
fn royalty_paid(d: &Definition, p: &ChainParams, sec: &escrow::Section) -> bool {
    let Some((bps, creator)) = d.royalty else { return true };
    let sellers: Vec<[u8; 32]> = sec.escrows.iter().filter(|((t, ..), ..)| *t == p.tok()).map(|((_, o, _), ..)| *o).filter(|o| *o != creator).collect();
    if sellers.is_empty() {
        return true;
    }
    // What the sellers were paid, in each OTHER token (the sold token coming back to a seller is no proceeds).
    let mut toks: Vec<Tok> = sec.pays.iter().filter(|(_, t, to, _)| sellers.contains(to) && *t != p.tok()).map(|(_, t, ..)| *t).collect();
    toks.sort();
    toks.dedup();
    toks.iter().all(|t| {
        let paid: u128 = sec.pays.iter().filter(|(_, x, to, _)| x == t && sellers.contains(to)).map(|(.., a)| *a).sum();
        let fee: u128 = sec.pays.iter().filter(|(_, x, to, _)| x == t && *to == creator).map(|(.., a)| *a).sum();
        fee.checked_mul(10_000).zip(paid.checked_mul(bps as u128)).is_some_and(|(have, owe)| have >= owe)
    })
}

/// The SIGNER that governs entry `i` of `s`: the last `Writer` or `Members` entry before it, or the one the cut it rests
/// on carries.
pub fn writer_at(s: &State, i: usize) -> Option<Signer> {
    s.entries[..i].iter().rev().find_map(|e| match &e.body {
        Body::Writer { cred, key_seq } => Some(craftworks_identity::read_credential(cred).map(|(_, _, node, _)| Signer::Device(node, *key_seq))),
        Body::Members { set, set_seq } => Some(Some(Signer::Set(set.clone(), *set_seq))),
        Body::Cut { writer, .. } => Some(writer.clone()),
        _ => None,
    })
    .flatten()
}

/// A MULTISIG owner's chain (its signers a key set, named in it — never a key log's).
pub fn is_set_chain(s: &State) -> bool {
    s.entries.iter().any(|e| matches!(&e.body, Body::Members { .. } | Body::Cut { writer: Some(Signer::Set(..)), .. }))
}

/// Where what is NEW in `merged` (not in `held`) begins: an index into it.
pub fn new_from(held: &State, merged: &State) -> usize {
    let had: Vec<[u8; 32]> = held.entries.iter().map(Entry::hash).collect();
    merged.entries.iter().position(|e| !had.contains(&e.hash())).unwrap_or(merged.entries.len())
}

/// THE MERGE (TOKEN §3, §3b): the same result in any order. A chain is a list of CELLS — its entries, then (frozen)
/// its fork. Where two copies first differ: an entry against nothing (one copy longer) — the longer; two entries by the
/// WRITER — equivocation, the chain frozen there with both; two entries not both the writer's — the lower hash, with
/// its branch; a fork against anything at that seq — the fork stands, with the two lowest writer entries known there
/// (a frozen chain stays frozen; the earliest fork wins).
pub fn merge(a: &State, b: &State) -> State {
    // Seqs, not positions: a copy may start at a cut. Cells are compared from the LATER start; what one copy still
    // holds before it is kept (so a holder checks a new cut against the entries it summarises — [`prune`] drops them
    // after).
    let first = |s: &State| s.entries.first().map_or(s.fork.as_ref().map_or(0, |f| f.0.seq), |e| e.seq);
    let end = |s: &State| first(s) + s.entries.len() as u64 + s.fork.is_some() as u64;
    let (fa, fb) = (first(a), first(b));
    let f = fa.max(fb);
    // One copy wholly before the other's start: what it held is pruned in the other — unless it is FROZEN (a fork is
    // never cut away: strict, TOKEN §3b).
    let empty = |s: &State| s.entries.is_empty() && s.fork.is_none();
    if empty(a) || empty(b) {
        return if empty(a) { b.clone() } else { a.clone() };
    }
    for (x, y) in [(a, b), (b, a)] {
        if x.fork.as_ref().is_some_and(|f| f.0.seq < first(y)) {
            return x.clone();
        }
    }
    for (x, y) in [(a, b), (b, a)] {
        if end(x) < first(y) {
            return y.clone();
        }
    }
    let h = if fa <= fb { a } else { b };
    let pre = &h.entries[..(f - first(h)) as usize];
    let trim = |s: &State| State { entries: s.entries[(f - first(s)) as usize..].to_vec(), fork: s.fork.clone() };
    let r = merge_aligned(pre, &trim(a), &trim(b));
    State { entries: pre.iter().chain(&r.entries).cloned().collect(), fork: r.fork }
}

/// Drop what comes before the chain's last cut (kept: the cut and on — and a fork, as is).
pub fn prune(s: &State) -> State {
    match s.entries.iter().rposition(|e| matches!(e.body, Body::Cut { .. })) {
        Some(i) if i > 0 => State { entries: s.entries[i..].to_vec(), fork: s.fork.clone() },
        _ => s.clone(),
    }
}

/// Merge two copies that start at the same seq, cell by cell (`pre`: what one copy still holds before them).
/// Two OWNER entries at one seq: the one whose writer stands on the HIGHER key seq wins (an account's recovery outranks
/// a lost device); on the SAME key seq they are an equivocation — the chain freezes (TOKEN §1, §3b).
fn merge_aligned(pre: &[Entry], a: &State, b: &State) -> State {
    enum Cell<'a> {
        E(&'a Entry),
        F(&'a Entry, &'a Entry),
    }
    fn cell(s: &State, i: usize) -> Option<Cell<'_>> {
        if i < s.entries.len() {
            Some(Cell::E(&s.entries[i]))
        } else if i == s.entries.len() {
            s.fork.as_ref().map(|(x, y)| Cell::F(x, y))
        } else {
            None
        }
    }
    let len = |s: &State| s.entries.len() + s.fork.is_some() as usize;
    let same = |x: &Option<Cell>, y: &Option<Cell>| match (x, y) {
        (Some(Cell::E(p)), Some(Cell::E(q))) => p == q,
        (Some(Cell::F(p1, p2)), Some(Cell::F(q1, q2))) => p1 == q1 && p2 == q2,
        (None, None) => true,
        _ => false,
    };
    let Some(i) = (0..len(a).max(len(b))).find(|&i| !same(&cell(a, i), &cell(b, i))) else { return a.clone() };
    // An owner entry's RANK at cell `i`: a writer entry's own key seq, else the key seq of the writer governing it.
    let governing = |i: usize| writer_at(&State { entries: pre.iter().chain(&a.entries[..i]).cloned().collect(), fork: None }, pre.len() + i).as_ref().map(Signer::rank);
    let rank = |e: &Entry, i: usize| match &e.body {
        Body::Writer { key_seq, .. } => Some(*key_seq),
        Body::Members { set_seq, .. } => Some(*set_seq),
        Body::Cut { writer, .. } => writer.as_ref().map(Signer::rank),
        _ => governing(i),
    };
    let frozen = |prefix: &[Entry], mut writers: Vec<Entry>| {
        writers.sort_by_key(Entry::hash);
        writers.dedup();
        State { entries: prefix.to_vec(), fork: Some((writers[0].clone(), writers[1].clone())) }
    };
    match (cell(a, i), cell(b, i)) {
        (Some(_), None) => a.clone(),
        (None, Some(_)) => b.clone(),
        (Some(Cell::E(x)), Some(Cell::E(y))) if x.by_owner() && y.by_owner() => match rank(x, i).cmp(&rank(y, i)) {
            std::cmp::Ordering::Greater => a.clone(),
            std::cmp::Ordering::Less => b.clone(),
            std::cmp::Ordering::Equal => frozen(&a.entries[..i], vec![x.clone(), y.clone()]),
        },
        (Some(Cell::E(x)), Some(Cell::E(y))) => if x.hash() < y.hash() { a.clone() } else { b.clone() },
        (Some(Cell::F(x1, x2)), Some(Cell::F(y1, y2))) => frozen(&a.entries[..i], vec![x1.clone(), x2.clone(), y1.clone(), y2.clone()]),
        (Some(Cell::F(x1, x2)), Some(Cell::E(e))) | (Some(Cell::E(e)), Some(Cell::F(x1, x2))) => {
            // A fork on a lower key than an owner entry here (a lost device's, against the recovered account's): the
            // owner entry's copy wins, the fork with it gone. On the same key: one more of the equivocation.
            let (fr, er) = (rank(x1, i).max(rank(x2, i)), rank(e, i));
            let e_side = if matches!(cell(a, i), Some(Cell::E(_))) { a } else { b };
            let f_side = if std::ptr::eq(e_side, a) { b } else { a };
            if e.by_owner() && er > fr {
                return e_side.clone();
            }
            if e.by_owner() && er < fr {
                return f_side.clone();
            }
            let mut w = vec![x1.clone(), x2.clone()];
            if e.by_owner() {
                w.push(e.clone());
            }
            frozen(&a.entries[..i], w)
        }
        (None, None) => a.clone(),
    }
}

/// The beacon's height in its state's bytes (`None`: not a beacon state).
pub fn beacon_height(b: &[u8]) -> Option<u64> {
    craftworks_beacon_contract::wire::decode(b).map(|s| s.height())
}

#[cfg(feature = "freenet-main-contract")]
mod contract {
    use super::{accept, authority, beacon_height, claims, freeze, operators, supply, check, cited_chains, cited_chains_from, id, merge, new_from, prune, ChainParams, Definition, DEFINITION};
    use freenet_stdlib::prelude::*;

    pub struct Token;

    /// A refusal that says why (the node logs it).
    fn why(w: super::Why) -> ContractError {
        ContractError::InvalidUpdateWithInfo { reason: w.into() }
    }

    fn related_state(related: &RelatedContracts<'static>, want: &[u8; 32]) -> Option<Option<Vec<u8>>> {
        related.states().find(|(id, _)| id.as_bytes() == want).map(|(_, s)| s.as_ref().map(|s| s.as_ref().to_vec()))
    }

    /// The definition named by an authority chain's params, checked to be the one its id names.
    fn authority_definition(p: &authority::Params, db: &[u8]) -> Option<Definition> {
        (id(&p.code_hash, db) == p.definition).then_some(())?;
        Definition::decode(db)
    }

    /// An AUTHORITY CHAIN holds: every change signed by the set it replaces, from the definition's first sets.
    fn authority_valid(params: &[u8], state: &[u8], related: &RelatedContracts<'static>) -> Result<ValidateResult, ContractError> {
        let (Some(p), Some(c)) = (authority::Params::decode(params), authority::Chain::decode(state)) else { return Ok(ValidateResult::Invalid) };
        let Some(db) = related_state(related, &p.definition) else { return Ok(ValidateResult::RequestRelated(vec![ContractInstanceId::new(p.definition)])) };
        let Some(d) = db.and_then(|db| authority_definition(&p, &db)) else { return Ok(ValidateResult::Invalid) };
        Ok(if authority::sets_at(&p.definition, (&d.mint, &d.freeze), &c, u64::MAX).is_ok() { ValidateResult::Valid } else { ValidateResult::Invalid })
    }

    /// An authority chain's UPDATE: merged; any change it did not hold checked against the definition and stamped
    /// within the margin of the beacon now (both asked for in one round).
    fn authority_update(params: &[u8], state: &[u8], data: Vec<UpdateData<'static>>) -> Result<UpdateModification<'static>, ContractError> {
        let p = authority::Params::decode(params).ok_or(ContractError::InvalidState)?;
        let held = authority::Chain::decode(state).ok_or(ContractError::InvalidState)?;
        let mut merged = held.clone();
        let mut related: Vec<([u8; 32], Vec<u8>)> = Vec::new();
        for d in data {
            let b: &[u8] = match &d {
                UpdateData::State(s) => s.as_ref(),
                UpdateData::Delta(x) => x.as_ref(),
                UpdateData::StateAndDelta { state, .. } => state.as_ref(),
                UpdateData::RelatedState { related_to, state } => {
                    let mut k = [0u8; 32];
                    k.copy_from_slice(related_to.as_bytes());
                    related.push((k, state.as_ref().to_vec()));
                    continue;
                }
                _ => continue,
            };
            merged = authority::merge(&merged, &authority::Chain::decode(b).ok_or(ContractError::InvalidUpdate)?);
        }
        if merged == held {
            return Ok(UpdateModification::valid(State::from(held.encode())));
        }
        let got = |w: &[u8; 32]| related.iter().find(|(k, _)| k == w).map(|(_, b)| b.clone());
        let (Some(db), Some(bb)) = (got(&p.definition), got(&p.beacon)) else {
            return UpdateModification::requires([p.definition, p.beacon].iter().map(|w| RelatedContract { contract_instance_id: ContractInstanceId::new(*w), mode: RelatedMode::StateOnce }).collect());
        };
        let d = authority_definition(&p, &db).ok_or(ContractError::InvalidUpdate)?;
        let now = beacon_height(&bb).ok_or(ContractError::InvalidUpdate)?;
        authority::sets_at(&p.definition, (&d.mint, &d.freeze), &merged, u64::MAX).map_err(why)?;
        authority::fresh(&held, &merged, now, d.margin).map_err(why)?;
        Ok(UpdateModification::valid(State::from(merged.encode())))
    }

    /// A SET SIGNED BY AN AUTHORITY (a supply shard, a freeze list) holds: read with its token's definition and
    /// authority chain, asked for together. `holds`: the set's own rule over them.
    fn signed_set_valid(a: authority::Params, related: &RelatedContracts<'static>, holds: &dyn Fn(&Definition, &authority::Chain) -> bool) -> Result<ValidateResult, ContractError> {
        let want = [a.definition, a.id()];
        let missing: Vec<ContractInstanceId> = want.iter().filter(|w| related_state(related, w).is_none()).map(|w| ContractInstanceId::new(*w)).collect();
        if !missing.is_empty() {
            return Ok(ValidateResult::RequestRelated(missing));
        }
        let Some(d) = related_state(related, &a.definition).flatten().and_then(|db| authority_definition(&a, &db)) else { return Ok(ValidateResult::Invalid) };
        let Some(chain) = related_state(related, &a.id()).flatten().and_then(|b| authority::Chain::decode(&b)) else { return Ok(ValidateResult::Invalid) };
        Ok(if holds(&d, &chain) { ValidateResult::Valid } else { ValidateResult::Invalid })
    }

    /// Its UPDATE: `merge` folds every given state into the held one (refusing what it cannot hold); anything new is
    /// checked by `check` against the definition, the authority chain and the beacon NOW (asked for in one round).
    fn signed_set_update<T: PartialEq>(a: authority::Params, held: T, data: Vec<UpdateData<'static>>, merge: &dyn Fn(&T, &[u8]) -> Option<T>, encode: &dyn Fn(&T) -> Vec<u8>, check: &dyn Fn(&Definition, &authority::Chain, &T, &T, u64) -> Result<(), super::Why>) -> Result<UpdateModification<'static>, ContractError> {
        let mut merged = None::<T>;
        let mut related: Vec<([u8; 32], Vec<u8>)> = Vec::new();
        for d in data {
            let b: &[u8] = match &d {
                UpdateData::State(s) => s.as_ref(),
                UpdateData::Delta(x) => x.as_ref(),
                UpdateData::StateAndDelta { state, .. } => state.as_ref(),
                UpdateData::RelatedState { related_to, state } => {
                    let mut k = [0u8; 32];
                    k.copy_from_slice(related_to.as_bytes());
                    related.push((k, state.as_ref().to_vec()));
                    continue;
                }
                _ => continue,
            };
            merged = Some(merge(merged.as_ref().unwrap_or(&held), b).ok_or(ContractError::InvalidUpdate)?);
        }
        let Some(merged) = merged.filter(|m| *m != held) else {
            return Ok(UpdateModification::valid(State::from(encode(&held))));
        };
        let want = [a.definition, a.id(), a.beacon];
        let got = |w: &[u8; 32]| related.iter().find(|(k, _)| k == w).map(|(_, b)| b.clone());
        if want.iter().any(|w| got(w).is_none()) {
            return UpdateModification::requires(want.iter().map(|w| RelatedContract { contract_instance_id: ContractInstanceId::new(*w), mode: RelatedMode::StateOnce }).collect());
        }
        let d = got(&a.definition).and_then(|db| authority_definition(&a, &db)).ok_or(ContractError::InvalidUpdate)?;
        let chain = got(&a.id()).and_then(|b| authority::Chain::decode(&b)).ok_or(ContractError::InvalidUpdate)?;
        let now = got(&a.beacon).and_then(|b| beacon_height(&b)).ok_or(ContractError::InvalidUpdate)?;
        check(&d, &chain, &held, &merged, now).map_err(why)?;
        Ok(UpdateModification::valid(State::from(encode(&merged))))
    }

    fn paid(p: &supply::Params, d: &Definition, m: &supply::Mint, got: &dyn Fn(&[u8; 32]) -> Option<Vec<u8>>) -> Result<(), super::Why> {
        let cid = m.paid.ok_or("a paid mint of no payment")?.0;
        supply::payment_ok(p, d, m, got(&cid).and_then(|b| super::State::decode(&b)).as_ref())?;
        if supply::spilled(p, m) {
            supply::spill_ok(d, p.epoch, got(&before(p)).and_then(|b| supply::Shard::decode(&b)).as_ref(), m)?;
        }
        Ok(())
    }

    /// The record before `p`'s (what a spilled paid mint is checked against).
    fn before(p: &supply::Params) -> [u8; 32] {
        supply::Params { epoch: p.epoch.saturating_sub(1), ..p.clone() }.id()
    }

    fn supply_valid(params: &[u8], state: &[u8], related: &RelatedContracts<'static>) -> Result<ValidateResult, ContractError> {
        let (Some(p), Some(sh)) = (supply::Params::decode(params), supply::Shard::decode(state)) else { return Ok(ValidateResult::Invalid) };
        // The payers of its paid mints (named by the mints themselves: asked in the same round as the definition).
        let mut payers: Vec<[u8; 32]> = sh.mints.iter().filter_map(|m| m.paid.map(|(c, _)| c)).collect();
        if sh.mints.iter().any(|m| supply::spilled(&p, m)) {
            payers.push(before(&p));
        }
        payers.sort();
        payers.dedup();
        if 2 + payers.len() > super::MAX_RELATED {
            return Ok(ValidateResult::Invalid);
        }
        let missing: Vec<ContractInstanceId> = payers.iter().filter(|c| related_state(related, c).is_none()).map(|c| ContractInstanceId::new(*c)).collect();
        if !missing.is_empty() && related_state(related, &p.definition).is_some() {
            return Ok(ValidateResult::RequestRelated(missing));
        }
        let got = |c: &[u8; 32]| related_state(related, c).flatten();
        match signed_set_valid(p.authority(), related, &|d, c| supply::holds(&p, d, c, &sh, &|m| paid(&p, d, m, &got)).is_ok())? {
            ValidateResult::RequestRelated(mut want) => {
                want.extend(missing);
                Ok(ValidateResult::RequestRelated(want))
            }
            r => Ok(r),
        }
    }

    fn supply_update(params: &[u8], state: &[u8], data: Vec<UpdateData<'static>>) -> Result<UpdateModification<'static>, ContractError> {
        let p = supply::Params::decode(params).ok_or(ContractError::InvalidState)?;
        let held = supply::Shard::decode(state).ok_or(ContractError::InvalidState)?;
        // (The payers of NEW paid mints are asked with the definition: the new mints name them.)
        let given: Vec<[u8; 32]> = data.iter().filter_map(|x| match x {
            UpdateData::State(s) => supply::Shard::decode(s.as_ref()),
            UpdateData::Delta(s) => supply::Shard::decode(s.as_ref()),
            UpdateData::StateAndDelta { state, .. } => supply::Shard::decode(state.as_ref()),
            _ => None,
        }).flat_map(|sh| sh.mints).filter(|m| !held.mints.contains(m)).flat_map(|m| m.paid.map(|(c, _)| c).into_iter().chain(supply::spilled(&p, &m).then(|| before(&p)))).collect();
        let given: Vec<[u8; 32]> = given.into_iter().fold(Vec::new(), |mut v, c| {
            if !v.contains(&c) {
                v.push(c);
            }
            v
        });
        let related: Vec<([u8; 32], Vec<u8>)> = data.iter().filter_map(|x| match x {
            UpdateData::RelatedState { related_to, state } => <[u8; 32]>::try_from(related_to.as_bytes()).ok().map(|k| (k, state.as_ref().to_vec())),
            _ => None,
        }).collect();
        if 3 + given.len() > super::MAX_RELATED {
            return Err(ContractError::InvalidUpdate);
        }
        if given.iter().any(|c| !related.iter().any(|(k, _)| k == c)) {
            let a = p.authority();
            let all = [a.definition, a.id(), a.beacon].into_iter().chain(given.iter().copied());
            return UpdateModification::requires(all.map(|c| RelatedContract { contract_instance_id: ContractInstanceId::new(c), mode: RelatedMode::StateOnce }).collect());
        }
        let got = |c: &[u8; 32]| related.iter().find(|(k, _)| k == c).map(|(_, b)| b.clone());
        signed_set_update(p.authority(), held.clone(), data, &|h, b| supply::merge(h, &supply::Shard::decode(b)?), &supply::Shard::encode, &|d, c, h, m, now| {
            // Paid mints already held were read when taken; the new ones are read now.
            supply::holds(&p, d, c, m, &|x| if h.mints.contains(x) { Ok(()) } else { paid(&p, d, x, &got) })?;
            supply::fresh(h, m, now, d.margin, d.capped())
        })
    }

    fn freeze_valid(params: &[u8], state: &[u8], related: &RelatedContracts<'static>) -> Result<ValidateResult, ContractError> {
        let (Some(p), Some(l)) = (freeze::Params::decode(params), freeze::List::decode(state)) else { return Ok(ValidateResult::Invalid) };
        signed_set_valid(p.authority(), related, &|d, c| freeze::holds(&p, (&d.mint, &d.freeze), c, &l).is_ok())
    }

    fn freeze_update(params: &[u8], state: &[u8], data: Vec<UpdateData<'static>>) -> Result<UpdateModification<'static>, ContractError> {
        let p = freeze::Params::decode(params).ok_or(ContractError::InvalidState)?;
        let held = freeze::List::decode(state).ok_or(ContractError::InvalidState)?;
        signed_set_update(p.authority(), held, data, &|h, b| freeze::merge(h, &freeze::List::decode(b)?), &freeze::List::encode, &|d, c, h, m, now| {
            freeze::holds(&p, (&d.mint, &d.freeze), c, m)?;
            freeze::fresh(h, m, now, d.margin)
        })
    }

    /// An OPERATORS list holds: every record well formed and signed by the owner's key (its key log read in the same
    /// round).
    fn operators_valid(params: &[u8], state: &[u8], related: &RelatedContracts<'static>) -> Result<ValidateResult, ContractError> {
        let (Some(p), Some(l)) = (operators::Params::decode(params), operators::List::decode(state)) else { return Ok(ValidateResult::Invalid) };
        let Some(lb) = related_state(related, &p.keys_id()) else { return Ok(ValidateResult::RequestRelated(vec![ContractInstanceId::new(p.keys_id())])) };
        let Some(log) = lb.and_then(|b| craftworks_idlog_contract::read(&p.owner, &b)) else { return Ok(ValidateResult::Invalid) };
        Ok(if operators::holds(&p, &log, &l).is_ok() { ValidateResult::Valid } else { ValidateResult::Invalid })
    }

    /// An operators list's UPDATE: merged; what is new on the owner's current key, only extending the past, near the
    /// beacon now (the definition — its margin —, the beacon and the key log in one round).
    fn operators_update(params: &[u8], state: &[u8], data: Vec<UpdateData<'static>>) -> Result<UpdateModification<'static>, ContractError> {
        let p = operators::Params::decode(params).ok_or(ContractError::InvalidState)?;
        let held = operators::List::decode(state).ok_or(ContractError::InvalidState)?;
        let mut merged = held.clone();
        let mut related: Vec<([u8; 32], Vec<u8>)> = Vec::new();
        for d in data {
            let b: &[u8] = match &d {
                UpdateData::State(s) => s.as_ref(),
                UpdateData::Delta(x) => x.as_ref(),
                UpdateData::StateAndDelta { state, .. } => state.as_ref(),
                UpdateData::RelatedState { related_to, state } => {
                    let mut k = [0u8; 32];
                    k.copy_from_slice(related_to.as_bytes());
                    related.push((k, state.as_ref().to_vec()));
                    continue;
                }
                _ => continue,
            };
            merged = operators::merge(&merged, &operators::List::decode(b).ok_or(ContractError::InvalidUpdate)?);
        }
        if merged == held {
            return Ok(UpdateModification::valid(State::from(held.encode())));
        }
        if merged.records.len() > operators::RECORDS_MAX {
            return Err(why("more operators than a list keeps"));
        }
        let want = [p.definition, p.beacon, p.keys_id()];
        let got = |w: &[u8; 32]| related.iter().find(|(k, _)| k == w).map(|(_, b)| b.clone());
        if want.iter().any(|w| got(w).is_none()) {
            return UpdateModification::requires(want.iter().map(|w| RelatedContract { contract_instance_id: ContractInstanceId::new(*w), mode: RelatedMode::StateOnce }).collect());
        }
        let d = got(&p.definition).filter(|db| id(&p.code_hash, db) == p.definition).and_then(|db| Definition::decode(&db)).ok_or(ContractError::InvalidUpdate)?;
        let now = got(&p.beacon).and_then(|b| beacon_height(&b)).ok_or(ContractError::InvalidUpdate)?;
        let log = got(&p.keys_id()).and_then(|b| craftworks_idlog_contract::read(&p.owner, &b)).ok_or(ContractError::InvalidUpdate)?;
        operators::holds(&p, &log, &merged).map_err(why)?;
        operators::fresh(&held, &merged, now, d.margin, log.head().seq).map_err(why)?;
        Ok(UpdateModification::valid(State::from(merged.encode())))
    }

    /// A CLAIMS REGISTRY holds: every claim of its epoch and signed.
    fn claims_valid(params: &[u8], state: &[u8]) -> Result<ValidateResult, ContractError> {
        let (Some(p), Some(r)) = (claims::Params::decode(params), claims::Registry::decode(state)) else { return Ok(ValidateResult::Invalid) };
        Ok(if claims::holds(&p, &r).is_ok() { ValidateResult::Valid } else { ValidateResult::Invalid })
    }

    /// A claims registry's UPDATE: merged while the beacon (read in the same round) is below its freeze; after, kept.
    fn claims_update(params: &[u8], state: &[u8], data: Vec<UpdateData<'static>>) -> Result<UpdateModification<'static>, ContractError> {
        let p = claims::Params::decode(params).ok_or(ContractError::InvalidState)?;
        let held = claims::Registry::decode(state).ok_or(ContractError::InvalidState)?;
        let mut given = claims::Registry::default();
        let mut beacon: Option<Vec<u8>> = None;
        for d in data {
            let b: &[u8] = match &d {
                UpdateData::State(s) => s.as_ref(),
                UpdateData::Delta(x) => x.as_ref(),
                UpdateData::StateAndDelta { state, .. } => state.as_ref(),
                UpdateData::RelatedState { related_to, state } => {
                    if related_to.as_bytes() == p.beacon {
                        beacon = Some(state.as_ref().to_vec());
                    }
                    continue;
                }
                _ => continue,
            };
            let g = claims::Registry::decode(b).ok_or(ContractError::InvalidUpdate)?;
            claims::holds(&p, &g).map_err(why)?;
            given = claims::merge(&given, &g);
        }
        if claims::merge(&held, &given) == held {
            return Ok(UpdateModification::valid(State::from(held.encode())));
        }
        let Some(bb) = beacon else {
            return UpdateModification::requires(vec![RelatedContract { contract_instance_id: ContractInstanceId::new(p.beacon), mode: RelatedMode::StateOnce }]);
        };
        let now = beacon_height(&bb).ok_or(ContractError::InvalidUpdate)?;
        let out = claims::take(&p, &held, &given, now);
        if out.claims.len() > claims::CLAIMS_MAX {
            return Err(why("more claims than a registry holds"));
        }
        Ok(UpdateModification::valid(State::from(out.encode())))
    }


    #[contract]
    impl ContractInterface for Token {
        fn validate_state(parameters: Parameters<'static>, state: State<'static>, related: RelatedContracts<'static>) -> Result<ValidateResult, ContractError> {
            let params = parameters.as_ref();
            if params.first() == Some(&super::authority::AUTHORITY) {
                return authority_valid(params, state.as_ref(), &related);
            }
            if params.first() == Some(&supply::SUPPLY) {
                return supply_valid(params, state.as_ref(), &related);
            }
            if params.first() == Some(&freeze::FROZEN) {
                return freeze_valid(params, state.as_ref(), &related);
            }
            if params.first() == Some(&operators::OPERATORS) {
                return operators_valid(params, state.as_ref(), &related);
            }
            if params.first() == Some(&claims::CLAIMS) {
                return claims_valid(params, state.as_ref());
            }
            // A DEFINITION: its state is its params, and they read as one.
            if params.first() == Some(&DEFINITION) {
                return Ok(if state.as_ref() == params && Definition::decode(params).is_some() { ValidateResult::Valid } else { ValidateResult::Invalid });
            }
            let Some(p) = ChainParams::decode(params) else { return Ok(ValidateResult::Invalid) };
            let Some(s) = super::State::decode(state.as_ref()) else { return Ok(ValidateResult::Invalid) };
            // What it must be shown: its definition and every chain its recent citations name — asked ALL AT ONCE
            // (one round only). Never itself (asking for itself is the node's error, not a refusal).
            let cited: Vec<[u8; 32]> = cited_chains(&p, &s).into_iter().map(|(d, o)| p.at(d, o).id()).collect();
            if cited.contains(&p.id()) {
                return Ok(ValidateResult::Invalid);
            }
            let want = super::validate_wants(&p, &s);
            let shard_id = |e: u64| supply::Params { code_hash: p.code_hash, definition: p.definition, beacon: p.beacon, epoch: e }.id();
            let ops_id = super::needs_operators(&s).then(|| operators::Params::of(&p).id());
            let missing: Vec<ContractInstanceId> = want.iter().filter(|w| related_state(&related, w).is_none()).map(|w| ContractInstanceId::new(*w)).collect();
            if !missing.is_empty() {
                return Ok(ValidateResult::RequestRelated(missing));
            }
            let Some(Some(db)) = related_state(&related, &p.definition) else { return Ok(ValidateResult::Invalid) };
            if id(&p.code_hash, &db) != p.definition {
                return Ok(ValidateResult::Invalid);
            }
            let Some(d) = Definition::decode(&db) else { return Ok(ValidateResult::Invalid) };
            let chain = |t: &super::Tok, owner: &[u8; 32]| related_state(&related, &p.at(*t, *owner).id()).flatten().and_then(|b| super::State::decode(&b));
            let keys = related_state(&related, &p.keys_id()).flatten().and_then(|b| craftworks_idlog_contract::read(&p.owner, &b));
            let shard = |e: u64| related_state(&related, &shard_id(e)).flatten().and_then(|b| supply::Shard::decode(&b));
            let ops = ops_id.and_then(|o| related_state(&related, &o)).flatten().and_then(|b| operators::List::decode(&b));
            let program = |id: &[u8; 32]| related_state(&related, id).flatten();
            Ok(if check(&p, &d, &s, &chain, &shard, keys.as_ref(), &super::Extra { ops: ops.as_ref(), program: Some(&program) }).is_ok() { ValidateResult::Valid } else { ValidateResult::Invalid })
        }

        fn update_state(parameters: Parameters<'static>, state: State<'static>, data: Vec<UpdateData<'static>>) -> Result<UpdateModification<'static>, ContractError> {
            if parameters.as_ref().first() == Some(&DEFINITION) {
                return Ok(UpdateModification::valid(state));
            }
            if parameters.as_ref().first() == Some(&super::authority::AUTHORITY) {
                return authority_update(parameters.as_ref(), state.as_ref(), data);
            }
            if parameters.as_ref().first() == Some(&supply::SUPPLY) {
                return supply_update(parameters.as_ref(), state.as_ref(), data);
            }
            if parameters.as_ref().first() == Some(&freeze::FROZEN) {
                return freeze_update(parameters.as_ref(), state.as_ref(), data);
            }
            if parameters.as_ref().first() == Some(&operators::OPERATORS) {
                return operators_update(parameters.as_ref(), state.as_ref(), data);
            }
            if parameters.as_ref().first() == Some(&claims::CLAIMS) {
                return claims_update(parameters.as_ref(), state.as_ref(), data);
            }
            let p = ChainParams::decode(parameters.as_ref()).ok_or(ContractError::InvalidState)?;
            let held = super::State::decode(state.as_ref()).ok_or(ContractError::InvalidState)?;
            let mut merged = held.clone();
            let mut related: Vec<([u8; 32], Vec<u8>)> = Vec::new();
            for d in data {
                let b: &[u8] = match &d {
                    UpdateData::State(s) => s.as_ref(),
                    UpdateData::Delta(x) => x.as_ref(),
                    UpdateData::StateAndDelta { state, .. } => state.as_ref(),
                    UpdateData::RelatedState { related_to, state } => {
                        let mut k = [0u8; 32];
                        k.copy_from_slice(related_to.as_bytes());
                        related.push((k, state.as_ref().to_vec()));
                        continue;
                    }
                    _ => continue,
                };
                let given = super::State::decode(b).ok_or(ContractError::InvalidUpdate)?;
                merged = merge(&merged, &given);
            }
            // THE APPEND RULES (TOKEN §3c) and every NEW citation shown: only for what is NEW here — against the
            // definition, the beacon NOW and the chains the new entries cite, all asked for in one round.
            let from = new_from(&held, &merged);
            if from < merged.entries.len() {
                let cited: Vec<[u8; 32]> = cited_chains_from(&p, &merged, from).into_iter().map(|(d, o)| p.at(d, o).id()).collect();
                if cited.contains(&p.id()) {
                    return Err(ContractError::InvalidUpdate);
                }
                // (The owner's key log only where a new entry is the owner's: a receive alone never waits on it.)
                let mut want = vec![p.definition, p.beacon];
                if merged.entries[from..].iter().any(super::Entry::by_owner) && !super::is_set_chain(&merged) {
                    want.push(p.keys_id());
                }
                want.extend(cited);
                let shard_id = |e: u64| supply::Params { code_hash: p.code_hash, definition: p.definition, beacon: p.beacon, epoch: e }.id();
                want.extend(super::minted_epochs_from(&merged, from).into_iter().map(shard_id));
                // A new DEBIT: the token's freeze list too (every token has one, made with its definition — one round
                // only: the node takes no second, so it is asked for before the definition says whether it freezes).
                let freeze_id = super::debits_from(&merged, from).then(|| super::freeze::Params { code_hash: p.code_hash, definition: p.definition, beacon: p.beacon }.id());
                want.extend(freeze_id);
                // The owner's OPERATORS list, only if a spend needs it (the whole chain is walked: an old one too).
                let ops_id = super::needs_operators(&merged).then(|| operators::Params::of(&p).id());
                want.extend(ops_id);
                // The PROGRAMS its new releases and payouts are checked against.
                want.extend(super::programs_from(&p, &merged, from));
                // ONE round holds at most MAX_RELATED (measured, the node's limit): more new citations than that are
                // refused here, plainly — the writer sends them in smaller batches. And the chain this leaves must stay
                // checkable FROM SCRATCH in one round too (audit 10-10: recent holders' shares, each asking for three,
                // made a chain no fresh copy could validate) — else the entry waits for a later window.
                if want.len() > super::MAX_RELATED || super::validate_wants(&p, &merged).len() > super::MAX_RELATED {
                    return Err(ContractError::InvalidUpdate);
                }
                let got = |w: &[u8; 32]| related.iter().find(|(k, _)| k == w).map(|(_, b)| b.clone());
                if want.iter().any(|w| got(w).is_none()) {
                    return UpdateModification::requires(want.iter().map(|w| RelatedContract { contract_instance_id: ContractInstanceId::new(*w), mode: RelatedMode::StateOnce }).collect());
                }
                let (db, bb) = (got(&p.definition).unwrap_or_default(), got(&p.beacon).unwrap_or_default());
                if id(&p.code_hash, &db) != p.definition {
                    return Err(ContractError::InvalidUpdate);
                }
                let d = Definition::decode(&db).ok_or(ContractError::InvalidUpdate)?;
                let now = beacon_height(&bb).ok_or(ContractError::InvalidUpdate)?;
                let chain = |t: &super::Tok, owner: &[u8; 32]| got(&p.at(*t, *owner).id()).and_then(|b| super::State::decode(&b));
                let keys = got(&p.keys_id()).and_then(|b| craftworks_idlog_contract::read(&p.owner, &b));
                if let Some(fl) = freeze_id {
                    let list = got(&fl).and_then(|b| super::freeze::List::decode(&b)).ok_or(ContractError::InvalidUpdate)?;
                    super::freeze_check(&p, &d, &held, &merged, &list).map_err(why)?;
                }
                let ops = ops_id.and_then(|o| got(&o)).and_then(|b| operators::List::decode(&b));
                return Ok(UpdateModification::valid(State::from(accept(&p, &d, now, &held, &merged, &chain, &|e| got(&shard_id(e)).and_then(|b| supply::Shard::decode(&b)), keys.as_ref(), &super::Extra { ops: ops.as_ref(), program: Some(&|id: &[u8; 32]| got(id)) }).map_err(why)?.encode())));
            }
            Ok(UpdateModification::valid(State::from(prune(&merged).encode())))
        }

        fn summarize_state(_parameters: Parameters<'static>, state: State<'static>) -> Result<StateSummary<'static>, ContractError> {
            Ok(StateSummary::from(blake3::hash(state.as_ref()).as_bytes().to_vec()))
        }

        fn get_state_delta(_parameters: Parameters<'static>, state: State<'static>, summary: StateSummary<'static>) -> Result<StateDelta<'static>, ContractError> {
            // The whole chain, unless they hold exactly this one (chains are small in this core).
            Ok(StateDelta::from(if summary.as_ref() == blake3::hash(state.as_ref()).as_bytes() { Vec::new() } else { state.as_ref().to_vec() }))
        }
    }
}

pub mod authority;
pub mod cut;
pub mod claims;
pub mod escrow;
pub mod freeze;
pub mod operators;
pub mod witness;
pub mod draw;
pub mod supply;
pub mod wallet;

#[cfg(test)]
mod tests;
