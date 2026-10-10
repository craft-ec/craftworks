# Token: a standard of our own

Owner, 2026-10-08: our own token on Freenet with the BEHAVIOUR of a blockchain's token programs (Solana's SPL,
Ethereum's ERC-20/721/1155) — not their method. **No central authority, no sequencer, no consensus**: "central authority
like blockchain is expensive." Every piece of state has ONE owner; everything else is a rule anyone checks.

Built on what Craftworks already has: owner-signed chains that refuse their own forks (as every table's tail), the
identity delegate that signs, public bags, the beacon (REWARDS §4), and pure functions anyone recomputes (the statements
ledger, REWARDS §5).

**Measured first (step 1, `tools/related-spike`, 10-08):** a contract's validation reads ANOTHER contract's state and
decides by it — a fill accepted when the lock it cites says so, refused when it does not, refused when the lock does not
exist, and the lock FETCHED over the network by a node that never held it (100 ms on a private 2-node network). The
node's limits: one round of related contracts per validation, at most 10, 10 s. Everything below rests on this.

## 0. Behaviour covered (owner 10-08: "design by the standard behaviour")

Every behaviour of the blockchain token standards, and how this standard gives it. Section numbers point below.

| Behaviour (standard) | Here | How |
|---|---|---|
| Fungible token — name, symbol, decimals (ERC-20 / SPL) | ✅ | definition params (§1) |
| `balanceOf` | ✅ one read | every entry carries the BALANCE after it: the chain's head is the balance (§2) |
| `transfer` | ✅ | `send` + `receive` — the receive appended by anyone (§2) |
| `approve` / `allowance` / `transferFrom` | ✅ | `allow` / read the owner's chain / `spend` (§2) |
| `totalSupply` | ✅ recomputed by anyone | genesis + mints − burns: every mint and burn POINTED in the token's supply bag; anyone sums them (§1a) |
| Transfer / Approval EVENTS | ✅ | a chain is subscribable (each change pushed); every entry also pointed in the token's ACTIVITY bag for wallets and explorers (§1a) |
| Permit / gasless transfers (EIP-2612) | ✅ native | every entry is a signed message; there is no gas — anyone may carry it to the network |
| Mint / burn authority (SPL) | ✅ | supply rule + the authority's `mint`; `burn` by the owner (§2, §6) |
| Freeze / thaw (SPL) | ✅ optional | the freeze authority's `freeze` / `thaw` (§2) |
| Pause the whole token (ERC-20 Pausable) | ✅ optional | the freeze authority's one word naming EVERYONE: no debit of any account while paused, credits still taken (§2) |
| Set / drop an authority (SPL `SetAuthority`) | ✅ | the definition's AUTHORITY CHAIN: rotations signed by the current authority; dropping one is final (§1a) |
| Close an empty account (SPL) | ✅ | a zero-balance chain may be dropped; it is made again by its next receive |
| Multisig accounts / authorities | ✅ | an owner or authority key is a k-of-n KEY SET (as the attestation substrate's named quorum): an entry carries k signatures (§1) |
| NFTs — `ownerOf`, transfer, per-item approve, `setApprovalForAll` (ERC-721) | ✅ | a definition with `ids: unique`; a chain per (item, owner) whose head says held or not; `allow` per item; an OPERATORS list per (token, owner) for all items at once (§1b) |
| Multi-token, batch transfer (ERC-1155) | ✅ batch not atomic | one definition, many ids; a chain per (item, owner); a batch = one send per item, each final on its own; an all-or-nothing bundle trade goes through a program over escrow (§1b, §4) |
| Metadata (token and per item) | ✅ | the definition names the Craftworks ITEM it is of (and a version pin); the item's own kind schema is its metadata (§1c) |
| Transfer fees (Token-2022) | ✅ optional | a definition's fee on top of every send, spend, escrow, vest; claimed by its collector (§1e) |
| Non-transferable (soulbound) | ✅ | a definition flag: nothing leaves the chain it is minted into; burn allowed (§1d) |
| Editions, numbered copies, max supply | ✅ | an EDITION is a definition with a CAP; numbered (#n of cap) or plain (semi-fungible) copies; each of a Craftworks ITEM (a collection is an item too); each its own royalty (§1c) |
| History, traceable by anyone (an explorer) | ✅ | no one ledger: from genesis and the supply bag, sends followed FORWARD reach every chain; credits cite their debits BACKWARD; kept alive by readers and keepers (§1a) |
| Atomic swap between two tokens | ✅ an app | the SWAP contract over escrow: both escrow, the deal crosses or refunds; the race closed by time (§4) |
| Vesting, time locks | ✅ | `vest(beneficiary, schedule)`: a debit now; each step claimable by the beneficiary once the beacon reaches it, once; never taken back (§4a) |
| Staking | ✅ | `stake` / `unstake` / `release` after the unbonding delay; stake weights the witness lot (§4a, §7a) |
| Escrow (the door for programs) | ✅ | `escrow` to a PROGRAM (any contract); back only as the program's verdict says — `unescrow`, `payout` (§4) |
| Order book, AMM / pooled liquidity | ✅ an app, per epoch | the POOL contract over escrow: batch auctions, one uniform price per epoch, LPs as ladders of limit orders (§5) |
| Lending | ✗ not built (owner 10-09) | an app over escrow if ever wanted — not part of the token |
| Composability (contracts calling contracts) | ✅ by reading | a contract validates against OTHER contracts' state (measured, step 1) and anyone appends what those states prove; a program over escrow (§4) is any contract whose state begins with the escrow section. Not: one transaction changing many contracts atomically — escrow and a program's verdict instead (§4, §5) |
| Global transaction order | ✗ by choice | not needed by any row above; it is the expensive central part |

## 1. The pieces

One contract CODE (the standard). Its instances differ by their parameters — on Freenet a contract's address is
`hash(code ‖ params)` — so nobody writes code to make a token.

| Piece | Params | State | Who writes |
|---|---|---|---|
| **Definition** | `{ name, symbol, decimals, supply rule, authorities, code hash, nonce }` | ITS PARAMS again (a related read returns state, not params: a reader checks `blake3(code hash ‖ state) == id`) | made once; never changes |
| **Account chain** | `{ definition id, owner DID, code hash, beacon, key log code hash }` | the owner's entries, in sequence, and its CARRIED STATE (§2) | the owner — and anyone appending what the chain's own rules PROVE (§3) |
| **Registry** | a public bag | pointers to definitions | anyone (discovery only, like Discover) |

- A token's id is its definition's address. Two tokens never share a chain: an owner has one chain per token.
- **The OWNER is a DID; ONE DEVICE writes** (foundation 2, 10-09 — ARCHITECTURE "a DID is the member; a device is
  a way to sign in"). A chain's owner (`ChainParams.owner`, a send's `to`, a genesis owner) is a DID; its WRITER is
  one device, named by a `Writer { credential, key_seq }` entry — the account's existing node CREDENTIAL
  (`identity::credential`: the DID, the device's node key, signed by an owner key of the DID's KEY LOG) and the seq
  of the log event whose key signed it — the entry itself signed by that node key. The writer's key signs the
  chain's owner entries (one writer key per chain: two devices signing one `seq` would be an equivocation, §3b).
  - The DID's key log is a related read (`ChainParams.idlog`: the key log's code hash, named by the definition too,
    as the beacon is): a `Writer` entry's credential must be signed by the key of the log event at its `key_seq`.
  - APPEND: a new entry's writer must be credentialed under the log's CURRENT key — a ROTATION of the account's key
    (words, KERI's pre-committed next key) revokes every older device for new writes.
  - RECOVERY outranks a lost device: at one seq, an entry whose writer stands on a HIGHER key seq wins (a `Writer`
    entry of the recovered account beats what a stolen device signs at that seq); two entries on the SAME key seq
    are an equivocation (§3b). A cut carries its chain's writer.
  - A SPENDER (`allow`) is a key, not a DID: an allowance grants a session key or a program.
- **Authorities** (optional, per definition): a MINT key set (or none: supply fixed by the rule), a FREEZE key set (or none) — §1a.
  A multisig authority is a k-of-n key set (as the attestation substrate's named quorum).
- **A MULTISIG OWNER** (foundation 4): the chain's owner id is the hash of its FIRST key set (`set_id`); its first
  entry, `Members { set, 0 }`, names that set, signed by k of it; every owner entry after carries k signatures
  (`Auth::Multi`); a later `Members { next, n + 1 }`, signed by k of the set before, changes who signs. No key log:
  the set names its own signers, so neither validation nor an update asks for one. At one seq the higher set seq wins
  (a members change outranks the old set's entry); two entries by the same set seq are an equivocation (§3b). A cut
  carries the set. A device writer never signs a set's chain, nor a set a device's.
- **Supply rule** — a fixed menu, never arbitrary code, so every instance stays checkable (§6).

## 1a. Supply, events, authorities

- **Supply bag** — a SUPPLY SHARD per token per epoch (4096 beacon heights; params `supply ‖ code ‖ definition ‖
  beacon ‖ epoch`): every `mint` of that epoch, each signed by the mint set current at its stamp (the shard's
  validation reads the definition and the authority chain). It admits nothing else, so nothing but the authority fills
  it; a mint past its cap (1024) goes in the next epoch. `totalSupply` = genesis + Σ the shards' mints − burns; anyone
  recomputes it. **A `Mint` entry is valid only if its mint is ALREADY in its epoch's shard** (the chain's update reads
  that one shard: a related contract) — so no token comes into being where nobody can find it. **Sealed:** a first
  PUT has no "now" to check, so a replaced mint key could make a never-made PAST shard; every MINT CHANGE therefore
  names, for each epoch since the last one up to its own, the hash of that shard's mints stamped before it (an epoch
  not named: none) — a set once replaced adds nothing to its past. (Burns are counted from the chains: a burn is its
  owner's entry, no shard needed for it to be found by the walk.)
- **Traceable without one ledger.** Every token starts at the definition's GENESIS entries or at a `mint` (all in the
  supply bag); every `send` NAMES its recipient and every credit CITES its debit. So, from genesis and the supply bag,
  anyone follows sends FORWARD to every chain that ever held the token, and any credit BACKWARD to where it came from
  — the whole history, each entry signed and checked, readable by anyone: an explorer's walk over many contracts
  instead of one. The bags only make it faster (an entry missing from the activity bag is still reached by the walk).
- **History kept.** A long chain's older entries are flushed into a tree (as tables are: carried balance at the cut,
  the entries readable); every chain and tree is kept alive by its readers and keepers (`keep`), as files and tables
  are — Freenet drops what nobody reads, where a blockchain's full nodes keep everything.
- **Activity bag** (`activity <definition> <day>`, a shard per day): every entry pointed there — the EVENT log a wallet or explorer
  reads; a holder's own chain, subscribed, pushes its own changes.
- **Authority chain** (params `authority ‖ code ‖ definition ‖ beacon`): the definition names the FIRST mint and
  freeze key sets (k of n keys; none: fixed supply / never frozen); each change, signed by k of the set it replaces,
  names the next (or none: dropped for good) and, for the mint set, seals the supply (above). An update's new changes
  are stamped within the margin of the beacon. Validation reads it (a related contract) to know who may `mint` or
  `freeze` at a stamp.
- **Freeze list** (params `frozen ‖ code ‖ definition ‖ beacon`): one record per frozen or thawed account — `(owner,
  frozen, seq, stamp)`, signed by the freeze set of its stamp; a later seq replaces it. An APPEND rule: no new debit of
  an owner it says is frozen (credits still land). **Every token has an authority chain and a freeze list, PUT with its
  definition**: a debit's update asks for the list in its one related round (measured: the node takes no second round,
  "depth>1 not supported"), before the definition shows whether the token freezes at all. A freeze CHANGE caps the replaced set: a
  record stamped before it holds only up to the change's `cap` (the highest seq that set issued) — so a word a
  replaced key back-dates onto a node that lacked the list is always outranked by the next set (`cap + 1`).

## 1b. Many ids: NFTs and multi-tokens

A definition says `ids: none` (fungible), `ids: unique` (ERC-721: each item's supply is 1) or `ids: many` (ERC-1155).
**A chain holds one token's ONE ITEM** (built 10-09): its params name the item (`ChainParams.item`, zero for a fungible
token), and chains name each other by a `Tok` — (definition, item). Every rule of a fungible chain (send, receive,
allow, escrow, payout, cut, mint, freeze) holds per item unchanged; the supply shards, the authority
chain and the freeze list stay per TOKEN, so one authority governs a whole token. A `unique` item is MADE by its
mint — `unique_item(definition, epoch, nonce)`, amount 1 — so each is minted once; a `many` item is any nonzero id the
mint authority names. For `unique`, the chain of the item's holder is the `ownerOf` answer; a transfer is the same
send/receive. Per-item metadata is a Craftworks item named by the id. A BATCH of several items is several sends (one
per item's chain), each final on its own — not one atomic entry; an all-or-nothing trade of a bundle goes through a
program over escrow (§4), as every cross-chain trade here.

**Ownership, metadata and royalties in ONE standard** (owner 10-08): a definition may carry a ROYALTY (a share in
basis points, and the creator it goes to). Every sale passes through a program over escrow (§4), and the token checks
the royalty on the one step every sale ends in — the PAYOUT of the sold token to its buyer (10-09, the generic rule):
the token pays out of a program only if, in that program's section, what the creator is paid in each other token is at
least the share of what the token's sellers (its escrows' owners, the creator excepted) are paid in it. The sold token
coming back to a seller (it bought one in the same program) is not proceeds. So no program can deliver an item
without its creator's share — and an app that ignored it would strand the item, which is why the SWAP and the POOL
read both tokens' definitions and pay the fee themselves, out of what the seller is paid, rounded up
(`escrow::royalty_fee`; none when the creator sells its own). Ownership (ids), the sale (escrow → verdict → payout)
and the royalty are checked by the token's one code, the metadata a Craftworks item the id names — where Solana binds
three programs (SPL Token, Metaplex metadata, rule sets / Core plugins for enforced royalties). The limit is
everyone's: a plain transfer (a gift) can hide a payment made elsewhere — royalties hold on sales through programs,
not on private deals.

## 1c. Editions and copies of an item (owner 10-10)

An ITEM has EDITIONS ("Gold", "Silver"); an edition has COPIES — item > editions > copies. Built 10-10. A COLLECTION
is an item too (a set: an album, "Pixel Cats"), whose own editions sell the whole set; which items belong to it is the
items' metadata in Craftworks. The token holds no collection (owner 10-10: dropped — a chain needs one only because
its items have no identity of their own; here an item is first-class, its author proven by its feed).

- **An edition is a definition.** It carries everything an edition has — its own ROYALTY (so every app already pays
  it: the swap and the pool read definitions, §1b), its mint authority (the creator), and three fields: `cap` (the most
  there will ever be; 0: none) and `item` + `version` (the Craftworks item it is of, and the item's version it pins;
  zero: the item as it is). The item's kind schema (video, book, image …) and its name are its metadata — the token
  does not copy them (an edition's own `name` is just its label — "Gold" — or empty for the item's title).
- **Copies, two kinds.** NUMBERED (`ids: numbered`): copy `n` is its own item (`numbered_item(edition, n)`), one of
  each, `n` from 1 up to the cap. PLAIN: the edition is fungible within itself (`ids: none`, decimals 0) — "3 Silver".
- **Minted on demand, without end** (owner 10-10: "how does it work with 1mil copies?"). A capped edition's mints live
  in RECORDS by NUMBER, not by time: record `k` holds copies `k·1024 + 1 … (k+1)·1024` (`supply::record_of`; plain
  copies are numbered too, unseen; an edition SOLD ON DEMAND, below, in records of 7). A record is made when its first copy is minted, so there is no end — copy
  5,000,001 is in record 4882 — and each number is in exactly one record, so the cap (numbers stop at it) and "each
  number once" are checked record by record. A mint names a RANGE of numbers inside one record (its nonce the first);
  a chain credits it with `CappedMint` (for a numbered copy: its number, amount 1). Plain copies leave the genesis's
  numbers to it (they start after `genesis_amount`).
- **Taken numbers.** A record held keeps its mints: an update never replaces a held mint (two under one nonce: the
  held one stands) and refuses a new mint onto numbers a held one has (audit 10-10: a later mint stamped earlier
  displaced one already credited). From scratch, a mint onto numbers another took is kept but never ADMITTED
  (`supply::admitted`, in stamp order); only an admitted mint is credited, and totalSupply counts only those. Two
  buyers racing for one next number: one is taken, the other's mint takes the following number. A mint past the cap,
  outside its record or of no number is refused outright.
- **A new mint key cannot be back-dated by the old.** A mint-authority change SEALS every record that holds a mint
  then (its seal, exactly) — and a record it does not name may hold no mint stamped before it (audit 10-10: sealing
  only the highest record left lower ones, not yet made, open to the old key). The cost is the change's size: one
  seal per record holding mints. The authority chain itself only ever EXTENDS: a change held at a seq is never
  replaced (audit 10-10: a leaked old key could sign a second change at a past seq and win it by hash).
- **Who may sell an item's editions.** The token checks the mint authority; that the edition's creator (its
  `genesis_owner`) is the Craftworks item's author is checked by the page where it shows the edition — by design, as
  everything on Freenet with no central coordination: the page already proves an item's author from its feed's
  credentials (a contract cannot read that cheaply). An edition of an item made by anyone else is simply not shown on
  it; no name can be copied, as an edition has none but its item's (owner 10-10: no Metaplex-style "verified" flag).
- **A royalty its HOLDERS share** (owner 10-10: "seller own copy should not get a cut"). An edition's royalty may name
  `HOLDERS` instead of a creator — only a numbered edition of at least 2 copies. Every app pays the fee as it pays a
  creator's — to `HOLDERS`, under the standard `escrow::fee_key(sold escrow)`, in the payment's token. Then each OTHER
  copy's holder AT THE SALE appends a `HolderPayout` naming the sale (the sold copy's escrow) and the time it was
  escrowed: `fee / (cap − 1)` (rounded down; unminted copies' shares and the dust stay in the program), once per copy
  and sale (its name in the received set). One owner held each copy then, and a copy minted later held nothing, so
  the shares never exceed the fee (audit 10-10: "while it holds the copy" let each later holder claim again, and a
  claimant could name an edition of its own). The sold copy takes none. A witness signs it only on the sale's time
  being its escrow's stamp, the program backed and every escrow of its verdict released, and the claimant's holding
  entry accepted. Numbered only: a plain copy has no number to name.
- **Sold on demand, its owner offline** (owner 10-10: "allow mint without owner for on demand"). An edition may carry
  a SALE — `Definition.sale = (token paid in, price of one, payee)`, capped or numbered only — and then ANYONE mints a
  copy by paying for it: a send of `price × amount` to the payee from their own chain of the sale's token, its nonce
  TAGGED for one record (`supply::sale_tag(edition, k)` in its first 8 bytes), then a PAID MINT in record `k`
  (`Mint.paid = (payer chain id, payment nonce)`, no signature) — to the payer, stamped no earlier than the payment.
  The record reads every NEW paid mint's payer chain when it takes it (asked with the definition); a payment is cited
  once in a record, and its tag binds it to that record, so one payment buys one copy. A sale's records hold 7
  numbers (`supply::PAID_RECORD`): a check reads at most 10 contracts (the definition, its authority chain, 7 payers,
  the record before). A copy's chain credits it as any mint, and the
  credit names its record (`CappedMint.record`, checked to be its numbers'). A witness that signs a credit of a paid
  mint whose payment is not in the payer's chain is burned (`FalseMint`, shown where the payer's chain can say:
  unforked, whole back to before the mint); signing a paid one proves nothing. A payer that cut its chain since the
  mint is taken as shown when the record took it. A buyer racing others for a record's last numbers mints again with
  other free numbers of it — or, the record FULL (every number held by an admitted mint: it takes no mint ever again)
  and none of its mints citing this payment, in the NEXT record (a SPILLED mint, `supply::spill_ok`: that record
  reads the one before). So a payment is never stranded by a race; only past the cap (sold out) is there nothing to
  buy.
- **Not built:** fractional ownership of one item (an app over escrow).

## 1d. Non-transferable tokens (soulbound, 10-10)

A definition may be `soulbound`: its tokens stay in the chain they are minted (or given at genesis) into — no send,
spend (an operator's neither), allowance, escrow or vest of them; they can be burned, and staked (which stays the
owner's). For badges, certificates, standing. (Token-2022's non-transferable extension.)

## 1e. Transfer fees (optional, 10-10)

A definition may carry a TRANSFER FEE — the owner's choice to enforce it: (basis points, the most one transfer pays —
0: no most, the collector). It is charged ON TOP of every send, spend, escrow and vest: the sender's chain carries its
balance less the amount AND the fee; the receiver (or the program) gets the whole amount. None when the collector
itself transfers. The collector claims each fee with `fee_claim(payer, debit, gross, amount)` in its own chain —
appended by anyone (with work), once (its name in the received set), once the debit is final, its amount exactly the
definition's fee of `gross` (rounded up, at most the most); witnesses accept it on the debit accepted, as a receive.
Programs' payouts and releases carry no fee: what entered a program paid it at its escrow. (Token-2022's
transfer-fee extension, withheld at the receiver there; here paid by the sender on top, so a receive stays exactly
its send.)

## 2. A chain's entries

An account chain is a sequence `seq 0, 1, 2, …`, each entry naming the one before (its hash) and CARRYING the state
after it — checked by validation (the previous state changed by this entry), so every check is a read of the head,
never a scan of history (which may be flushed): the BALANCE (per id), the open ALLOWANCES and what is left of each, the
STAKE and what is unbonding, and an ACCUMULATOR (a Merkle root) of every send already received here and every payout
or vesting step already taken — a credit carries the proof that its name is NOT yet in it,
and the entry adds it. A PRUNE CUT (older entries flushed into a tree, §1a) is an owner-signed entry naming the tree's
root and carrying the state at the cut: the contract takes it as the chain's start; wallets verify across it (§3a).

**The cut uses the PROLLY TREE we already have — no second structure** (owner 10-08). At a cut the owner flushes the
pruned entries into a prolly tree (the HISTORY, read by wallets and explorers) and writes, as a second prolly tree, the
RECEIVED SET as of the cut (every send received, every payout and vesting step taken). The cut entry carries both ROOTS and
the carried state (balance, allowances, stake, unbonding, the writer). A chain is not cut over an open escrow. After it: a citation of something that could predate the
cut (a receive of an older send) CARRIES a prolly ABSENCE PROOF against the cut's received-set root — built by the
WRITER on their own device (they fetch the blocks as any reader does), checked by the contract from the proof alone
(`freenet_prolly::proof::verify`: no fetch, no related slot, nothing to be unavailable). Entries after the cut are
checked against the live entries, as before. The set changes only at the next cut, so no insertion proofs.

As BUILT (10-08, `contracts-src/token/src/cut.rs`, step 7 below):
- The HISTORY holds each entry's own BYTES at its seq (inline up to 1 KB, a value block beyond) — every entry before
  the cut, the earlier cuts' included — so the chain stays WALKABLE: its blocks are published as Block contracts and a
  wallet reads the whole chain back from them.
- Both trees are built WITH PARITY (every group coded, as tables are) and the cut carries each ROOT's parity ids
  (its group of one, as a table's tail does): any lost block — the root too — is rebuilt from its group, so a reader
  racing a block against its group never waits on a lost one (owner 10-09: "if it is not available, never make a
  network request"). The WALLET NEVER FETCHES: it reads only blocks HELD and answers `Need(blocks)` for the rest —
  fetched by the caller through the blocks door (`packages/blocks.js`: one GET per block, raced against its group,
  rebuilt from parity, a loss remembered and never waited on twice) — and runs again; "not held" is never a verdict.
- CITING ACROSS ANOTHER CHAIN'S CUT: an entry whose cited entry its chain has pruned carries, in its evidence, that
  chain's id, the cited seq, a PRESENCE proof against the cut's history root and the entry's bytes (≤ 4 such items).
- WHO CHECKS A CUT: the node that holds what it summarises (`update_state`: the shape rules over the old entries and
  the cut together) — the carried state must be exactly what they compute; then it PRUNES. A node that never held them
  takes the cut as the chain's start; a cut's WITNESSES read the history whole, rebuild both trees and audit it
  (`cut::whole`, `cut::audit`) before signing — a lying cut is never witnessed.
- NEW citations are STRICT, old ones are not: what a holder takes in (`update_state`, the cited chains asked for in
  the same round as the definition and the beacon) must SHOW what it cites — live, or by its evidence. Checked from
  scratch (`validate_state`), a citation whose cited chain has since cut past it is taken as shown when appended —
  ONLY if the citing entry is stamped no later than that cut (within the margin): it was appended before the cut
  existed (otherwise a sender's cut would break every recent receive of its sends; found by a test, 10-08). One
  stamped after must carry its evidence even from scratch, so a fresh copy cannot slip an unshown one in (10-09).
- MERGE by seq: two copies compared from the later start; the prefix one copy still holds kept so the cut is checked
  against it; a fork below the other copy's start kept (strict, §3b).

MEASURED (10-08, `tools/prolly-spike`, fx): a contract whose validation runs `verify_bytes` on a carried absence proof
— received sets of 1,000 / 100,000 / 1,000,000 names: proofs 5.8 / 14.7 / 17.1 KB, verified natively in 11 / 17 /
21 µs; the contract ACCEPTED each absence proof and REFUSED the presence proof and a tampered one at every size. The
verifier adds ~130 KB to a contract's wasm. A receive pays a proof only when it cites a send older than the last cut. **A debit is valid only if the balance covers it** at its place. An owner may be a k-of-n key set:
an entry it signs carries k signatures.

| Entry | Effect | Written by | Valid when |
|---|---|---|---|
| `send(to, amount, memo)` | debit | owner | balance covers it |
| `receive(send ref)` | credit | **anyone** | the cited send names this owner and is in its chain (related contract) and not received here before |
| `burn(amount)` | debit, no credit anywhere | owner | balance covers it |
| `mint(nonce, amount, minted)` | credit | **anyone** (with work) | the mint is in its epoch's supply shard, naming this owner and amount; once each |
| `allow(spender, limit, until)` | none (a permission) | owner | — |
| `spend(allow ref, to, amount)` | debit | the spender | within the allowance's limit and time, balance covers it |
| `escrow(program, terms, amount)` | debit, held for a program | owner | balance covers it (§4) |
| `unescrow(escrow ref, refund)` | credit of what comes back | **anyone** | the program's section refunds exactly that, once (§4) |
| `payout(program, key, amount)` | credit | **anyone** (with work) | the program's section pays this owner exactly that under that key, once; a royalty paid (§1b, §4) |
| `vest(to, schedule)` / `vested(…)` | debit now / credit per step | owner / **anyone** | §4a |
| `stake` / `unstake` / `release` | stake moved / unbonding / back | owner / owner / **anyone** | §4a |
| `freeze` / `thaw` | blocks / unblocks debits | the freeze authority (in the freeze list, not the chain) | the definition names one |

**Anyone may append what the chain's own rules prove** — so a receiver or a trader need never be online: their chain
takes the credit, the payout, the refund by itself. The owner's key is needed only for what spends or permits.

**Spam costs work.** There is no gas, so: an entry appended by someone OTHER than the owner carries a small PROOF OF
WORK (as bags already admit their items), a `send` has a MINIMUM amount (the definition's), and a chain is made only by
a receive of at least that amount.

## 3. Concurrency without a sequencer

Two writers may append at the same `seq` at once (the owner sends while someone appends a payout). Freenet's copies merge them by
the contract's own rule — **the same winner on every copy**: the entry with the lowest hash. The loser is not lost:

- a `receive`, `payout` or `unescrow` is re-appended at the next `seq` (by anyone: they are claims on proven facts);
- an owner's `send` / `burn` / `escrow` is re-signed by the owner's page or delegate and re-checked against the new
  balance (it may now fail: the balance was spent by the winner).

**Finality:** a credit counts once the copies agree. A receiver of value waits a FINALITY margin (to be measured on
the real network: step 4) before treating it as theirs — the one cost of having no sequencer.

**Nothing is credited from an entry that may still lose.** Every entry names the beacon it was written at (no earlier
than its predecessor's, and a beacon that exists); an entry that CITES another chain's entry — a `receive` of a
`send`, a `vested` step of a `vest` — is valid only once the cited entry is at least `margin` beacons
old. By then every copy of the cited chain holds the same winner, so a send that lost its race (and failed its
re-check) is never received. A cited entry is named by `(chain, nonce)`, not its `seq`, so a loser re-appended at a
later `seq` keeps its name.

## 3b. Equivocation is fatal (owner, 2026-10-08: strict, "else it can be double spend")

Two entries signed by a chain's writer at the SAME `seq` are an EQUIVOCATION — the chain's own proof of a double spend.
Since one writer key belongs to one device (§1), an honest owner never makes one.

- Anyone publishes the two entries as an EQUIVOCATION PROOF in the token's fraud bag; any chain holding both (the merge
  keeps BOTH, never picks a winner between two writer-signed entries) is marked by its contract.
- **Strict:** everything BEFORE the fork stays valid; EVERY DEBIT AFTER IT, on both branches, is VOID — credits citing
  them are void too. The forger cannot choose which victim keeps the money, and neither branch is spendable. The chain
  is dead for its owner; the proof stays in the fraud bag for good.
- So a wallet accepts a credit only after the margin AND finding no conflicting entry known for the sender's `seq`.
- The lowest-hash merge (§3) settles only races between DIFFERENT writers (the owner and anyone appending a credit) — never
  two entries of the owner's own key.

**No backdating.** An entry names the beacon it was written at; when APPENDED it must name one no older than the
latest the validating node knows, less a small skew. A whole chain handed over from scratch cannot be checked for that,
so nothing TIMED trusts a writer's own stamp: a timeout, an epoch's membership (§5) and a margin are judged by the
beacon at which the cited entry became FINAL — the first beacon at which the copies agree on it, which anyone reading
the beacon contract and the chain recomputes.

**A partition longer than the margin** (copies not agreeing in time) could let a program's two copies decide
differently: a program's own rule closes that (a swap's verdict never replaced, crossing only before its cutoff and
refund only from it; a pool batch frozen at its settle height), and the token takes each release only as the section
it reads says, once.

## 3a. Who checks what (owner, 2026-10-08)

**The limit.** Freenet validates any copy of a contract's state from scratch (a peer may hand a node a whole chain),
and a validation may read at most 10 related contracts, in one round (measured limits, step 1). A chain's own contract
therefore cannot prove every `receive` in a long chain against its sender. So:

- **The contract enforces the LOCAL rules** — signatures, sequence and links, carried balances, no fork, a debit within
  the balance, no receive twice, the code hash in its params — and, when an entry is APPENDED, checks what it cites
  (≤ 10 related, one round). A chain is always well-formed; it is not, by its contract alone, proven honest.
- **Value is an entry its WITNESSES accepted** (§7a; built 6d, `contracts-src/token/src/wallet.rs`). Each entry's
  committee, drawn by lot from the stakers, re-checks it and signs it only by INDUCTION: the chain holds, the entry
  before it is accepted, and every entry it cites (the send a receive credits, the vest a vesting step credits; a payout's program backed by real escrows)
  is accepted. So an accepted entry stands on an accepted past, and a receiving wallet checks ONE thing — that its
  payment carries enough of its committee's signatures — and walks nothing. A buried fake receive is never witnessed,
  nor anything after it. (Before 6d this was a citation walk by the receiving wallet plus public checkpoints, vouches
  and fraud proofs; removed outright — no token was live.)

Chains also refuse what can be refused cheaply: a receive whose cited send is not there when it is appended, a code
hash that is not the standard's — a chain's params name the standard's code hash (an id is `blake3(code hash ‖ params)`,
so a chain recomputes every chain it cites), and readers check a chain's params against the definition's.

## 3c. Shape rules and append rules (10-08, before step 5b)

A rule about TIME cannot be a rule of the state: Freenet re-validates a whole chain from scratch whenever any peer hands
it over, at any later time — an entry appended in time, re-checked against "now" a year later, would fail and
make the whole chain invalid for good. So the contract keeps two kinds of rules:

- **SHAPE rules** (`validate_state`, any time, from scratch): signatures, links, carried balances, allowances, no
  receive twice, the recent citations (§3a) — and an entry's BEACON STAMP never lower than its predecessor's. True of a
  chain forever once true.
- **APPEND rules** (`update_state`: a node that already HOLDS the chain taking in entries it did not have — the only
  moment that knows what is new): each new entry's stamp is within a small skew of the beacon the node reads now (one
  related contract); a credit cites an entry final by the margin; a `release` lands after its unbonding; a `vested` step
  at or after its height. The TIMED rules are one function (`timed`), applied again by every witness at the entry's
  own stamp, so a first copy (shape rules only) is never witnessed past them.

MEASURED (10-08, `tools/related-spike guard`, fx/fy): `update_state` asking for a related contract completes — the
node fetches it and calls `update_state` again with its state — and the contract decides by it: a guard whose lock says
LOCK took its update (v0 → v1); one whose lock says NOPE refused it ("invalid contract update"), staying at v0.

A forged stamp can only pass where the chain was never held (a fresh PUT is checked by its shape alone) — and dies at
every node that did hold it, which applies the append rules before it merges. A chain's time is its stamps; its stamps
were checked against the beacon by every node that held the chain when they arrived.

## 4. Escrow: the token's one door for apps (owner 10-09: swaps, pools are apps, not the token)

The token knows nothing of swaps, pools or any other app. It has ONE door (built 10-09, `escrow.rs`):

1. **Escrow** — in the owner's chain: `escrow(program, terms, amount)` — a debit now, held for a PROGRAM: any
   contract, named by its id. `terms` (≤ 256 bytes) are what the owner agreed to; the program lists the escrow by
   their hash.
2. **The program's verdict** — a program's state BEGINS with the standard SECTION: the escrows it holds (each with its
   amount and terms hash), then its verdict — refunds per escrow, payouts `(key, token, recipient, amount)`. The rule
   any reader checks of ANY program (`Section::holds`): each escrow listed once, each refund within its escrow, each
   payout key once, and per token refunds plus payouts within what was escrowed — so even a lying program only
   redistributes what was escrowed with it.
3. **Unescrow / Payout** — anyone appends `unescrow(escrow, refund)` to the owner's chain and `payout(program, key,
   amount)` to each recipient's chain; the token takes each only as the program's section says, once (a payout's key
   goes into the received set, so a cut keeps it). A chain is not cut over an open escrow.
4. **Witnesses** check besides that every escrow a program lists is a real, ACCEPTED escrow to that program with that
   amount and those terms (`wallet::program_backed`): a program cannot list anyone's escrow on terms they did not sign.
6. **One verdict paid** (audit 10-10): a program's copies may hold two verdicts (a split order set; a deal's escrows
   doubled), so a RELEASE names the verdict it follows (`unescrow(escrow, refund, verdict)`: the section's hash) and
   witnesses sign a PAYOUT only once every escrow its program lists is released under THAT verdict. An escrow is
   released once, so only one verdict is ever paid out; a split leaves the other's payouts unsigned (a stall, never a
   double pay). A pool's or a deal's verdict, once held, is kept. A program whose listing is not backed (an escrow in
   it fake or reclaimed) took nothing: its escrows can be reclaimed.
5. **Reclaim** (10-10) — our apps always finalize by anyone, but the token cannot see a program's rules: an escrow to
   a wrong id, or to someone's program that never decides, would be locked for good. So an escrow may carry `until`
   (0: none): from that beacon on, anyone may append `reclaim(escrow)` — the whole amount back — and WITNESSES sign it
   only if the program, as they read it, has not taken the escrow (not there, or not listing it); once reclaimed,
   they sign no release or payout of a program that lists it. The CHAIN never reads the program for a reclaim: a node
   refuses outright an update that asks for a contract that does not exist (measured 10-10: "missing related
   contract"). A wallet sets `until` past the program's own finish (a deal's cutoff, a batch's settle) plus the
   margin.

The apps, each its own contract over this door:

- **SWAP** (`contracts-src/swap`, one contract per DEAL): party X gives `amount_x` of one token, Y `amount_y` of
  another, by a beacon deadline. Both listed before `deadline + GRACE` (4 steps): CROSSED — each paid the other's
  amount (less a royalty's fee, §1b); from then on, one side alone: REFUNDED whole. A verdict once taken is never
  replaced (a refund paid out, then a crossing, would pay out of an escrow already gone). The race is closed by time,
  not by a sequencer: crossing only before the cutoff, refund only from it.
- **POOL** (§5).

Two-party LOANS and anything else with release conditions are further programs over the same door; none is built
(lending: not built, owner 10-09).

## 4a. Vesting, time locks, staking

- **Time lock / vesting** — BUILT 10-09: `Vest { to, schedule }` — owner-signed, a debit of the schedule's whole total
  now; the schedule is (beacon height, amount) steps, heights rising, up to [`VEST_STEPS`] (64), each at least the
  minimum send; never to itself. `Vested { from, vest, step, amount, at, cited }` — appended by anyone (with work) in
  the beneficiary's chain: credited only if the giver's chain holds that vest naming this beneficiary with exactly
  that step (height and amount), once per step (its name in the received set, so a cut keeps it), and stamped at or
  after its height — checked FROM SCRATCH, so a first copy cannot claim early, and no witness signs an entry stamped
  ahead of the beacon. Nothing in the giver's chain undoes a vest. A single time lock is a one-step schedule.
- **Staking** — BUILT 10-09: `stake(amount)` moves balance to stake; `unstake(amount)` starts its unbonding;
  `release(unstake)` returns it after `UNBOND` (2 epochs). Stake bound at an epoch's start weights the witness lot
  (§7a) and is burned on a proven misdeed. Rewards are economics (the owner's, later).

## 5. Pools, by batch auction (an app over escrow)

An AMM's pool needs one shared value changed in a global order. Instead, a POOL is a batch per epoch — its own
contract, one per (pair, epoch) (built 10-09: `contracts-src/pool`, `clear.rs`):

1. **The batch** (`Batch`): tokens A and B (prices B per A), its epoch length and grace, the beacon, its epoch.
2. **Collect** — during the epoch, a trader escrows what it gives with the batch (§4), its terms the side (SELL of A, or
   BUY of A with B) and a LIMIT price, and its order goes into the batch's state. A liquidity provider's curve is a
   LADDER of limit orders, built by its client: one primitive. From the batch's settle height (close + grace) it takes
   no new order — so every clearer clears the same set; of two settled copies the one with more orders wins (the grace
   is for the sets to meet first).
3. **Clear** — a PURE FUNCTION (`clear::clear`) over the orders, in their canonical (sorted) order: ONE uniform price —
   the one matching the most A, ties to the smaller imbalance, then the lower price; the short side fills whole, the
   long side pro rata (rounded down, the rest to the earliest). Buyers pay the ceiling, sellers get the floor: nothing
   is created (the rounding's dust stays in the batch). Order within the batch does not matter: nobody can front-run.
4. **Settle** — from the settle height the batch's verdict is EXACTLY the clearing: each order's unspent part back (a
   refund), what it bought paid out in the other token — less the royalty's fee of the token it sold (§1b), paid out
   beside it. The batch contract checks that itself (it holds every order); the token checks each release against it.

## 5a. The beacon (the clock)

Every `until`, epoch, grace and margin — and keeper assignment, and any reward paid per unit of time — reads the
BEACON: a HEIGHT that only grows, at a steady pace, and RANDOMNESS nobody knows ahead. "The newest Discover head"
(REWARDS §4) is not one (no height; anyone appending to Discover nudges it). A Freenet contract has NO CLOCK — its
validation is deterministic, so it cannot refuse a step "from the future". Without consensus, the one honest source of
time is **SEQUENTIAL WORK**: a computation more machines cannot speed up (the idea of Solana's proof of history).

**The beacon contract** (one instance per network; params: `K`, the work per step; `S`, its segments; `s`, how many
are checked):

- **A step** `n+1` = `out(n+1) = H^K(out(n) ‖ mix(n))` — `K` blake3 iterations, each on the last's output: they can only
  be done one after another. `K` is set so a typical machine takes the CADENCE (e.g. a minute) per step.
- **Anyone extends it** — any node or delegate that needs time to pass (a wallet waiting out a margin, a pool clearing).
  One honest extender keeps it alive; two extending the same step compute the SAME output (nothing in it is theirs to
  choose), so there is nothing to race for.
- **Checked cheaply, proven wrong by anyone.** A step carries ALL its `S` intermediate points while it is among the
  newest `F` steps (the FRAUD WINDOW: 2048 × 32 B = 64 KB a step), then only their Merkle root — without the points a
  verifier who finds a different result could not say WHICH segment is wrong. After the window a step is final. The contract recomputes `s` segments chosen from the step's own points (cost `K·s/S`: with
  `S = 1024`, `s = 8`, under 1% of the work). That sample is only a SPEED BUMP: chosen from the step's own points, a
  cheater can regenerate fake points until the sampled segments happen to be honest (half faked: ~1 in 256 tries). The
  SAFETY is the FRAUD PROOF: anyone recomputing a step fully and finding a wrong segment publishes it — that step and
  every one after it lose to any chain without it (the merge: the longest chain with no proven-bad step; equal length,
  the lowest output). So **every extender fully verifies the step it builds on** (twice the work of extending alone),
  and wallets may verify the steps their `until`s depend on.
- **The merge is computed from the UNION of every step known** (built 10-08): a fake that wins by output is kept
  beside the honest step it hides, never instead of it — a CANDIDATE (bounded, the lowest heights first, so a fake
  racing ahead cannot push it out; above the chain's top too, a step whose parent is not known yet waits for it) — so
  a fraud proof arriving in any order brings the honest step back. A fake that kept the true OUTPUT (forged inside)
  loses only itself: what was built on it is honest and stays.
- **MIX = fresh entropy in every step** (BUILT 10-09): a step carries its extender's `mix` (32 random bytes), entered
  where the step starts (`start(prev, height, mix)`), under all `K` iterations. A chain computed AHEAD of time — while
  nobody extended — is void the moment an honest extender lands a step with entropy no one knew. The contract always
  re-runs the newest step's first segment, so a step claiming another mix than it was computed with is refused at once
  (and proven by a fraud proof like any wrong segment). Honestly, what it does NOT stop: an extender trying several
  mixes for its own step in parallel (each costing the full step) and publishing one it likes — the tie-break (lowest
  output) lets it win against an honest step at the same height; and an extender with a strictly FASTER core running
  its own chain knows outputs ahead by its lead. Both are bounded by the lot's delay below, the pace every wallet
  watches, and the burn of witnesses who sign what does not hold (§7a). (Designed first as a bag of contributions
  every step must include; dropped: an extender can omit any, so it adds nothing over its own mix.)
- **Randomness is used late.** `random(n) = out(n+1)`: a faster machine learns `out(n+1)` a little before everyone
  else, but anything drawn from it was fixed a whole step earlier. A WITNESS LOT (`wallet::lot_of`) is fixed by the
  entry's PLACE, never by what is written there: drawn at its PREDECESSOR's stamp + margin + 2, and never before its
  own epoch's first step (a chain's first entry: that step). Two entries at one place share one committee — honest
  members sign one, anyone signing both is burned — so a fork shown to friendly witnesses gets no committee of its own.
  What stays choosable: writing in a LATER epoch draws again, one try an epoch (an attacker with 30% of the stake
  needs on the order of 600 epochs for one committee of 11 of 16 of its own). (Built first at the entry's OWN stamp +
  margin + 2: that let the writer of a fork pick its stamp, so its committee — replaced 10-09.)

**Its pace, honestly:** height grows with sequential speed, not wall time. One extender with a faster single core (or,
later, special hardware for blake3) moves it faster for everyone — by that speed ratio, not by how many machines they
own. So:
- timeouts, margins and windows are counted in steps and set with room for that ratio;
- anything that PAYS per step (staking, keeper issuance) caps its total by HEIGHT reached, never by elapsed steps
  alone, and a token may pin such rewards to a schedule of heights;
- every wallet compares the beacon's pace with its own clock and SHOWS when it runs fast — a pace far off wall time is
  visible to everyone.

**Reading it:** a contract reads the beacon as one related contract (one of its 10). Its state keeps only recent steps
— enough to judge "final at step n" and `until`s near now. Older OUTPUTS live on in the **OUTPUTS LOG** (built 10-09,
`contracts-src/beacon/src/log.rs`): one contract per 4096 heights (same code, params: the beacon's id and the span),
filled by anyone — extenders record each step once it is FINAL — taking a height only as the beacon holds it final,
and never changing one it holds. Readers (witnesses checking that an old entry was accepted, wallets) read the beacon
for recent heights and the log for older ones — an account idle for days stays witnessable.

**BUILT (10-08, `contracts-src/beacon`, `contracts/beacon.wasm`; 12 tests, 7 mutants all killed; live on fx/fy via
`tools/beacon-live`):** small steps (K = 2²⁴) extended through both nodes alike; a FAKE step (output forged — it
passed the 8-of-2048 sample on the FIRST try) plus a step on it accepted and leading at height 5; its fraud proof
(found by recomputing in 0.97 s) with the honest step: both nodes at the honest height 4 within 5 s. ONE REAL STEP
(K = 1.07 × 10⁹, 2048 segments, 8 sampled): computed in 61.5 s on the M4; 65 KB; the node accepted it in 1.2 s. Not
yet then: the entropy MIX — BUILT 10-09 (each step's extender mix; live on fx/fy, `tools/beacon-live`: the fake step
caught and the honest step back on both nodes, as before).

**MEASURED (10-08, `tools/beacon-bench`)** — sequential blake3, each iteration hashing the last 32-byte output:

| Where | Iterations per second | How |
|---|---|---|
| Apple M4 Max, one core, native | 17.9 M | 10 M and 30 M iterations, steady |
| AMD EPYC-Rome (B), one core, native | 11.0 M | 20 M twice, under B's usual load |
| Inside the node's WASM (wasmtime, 0.2.142, on the M4) | 13.0–13.5 M (~75% of native) | the largest N one validation completes within the node's 5 s budget: 66 M passes, 68 M times out at 5.16 s |

- A PUT runs a contract's validation about THREE times (66 M took 15.5 s end to end): every check's cost counts ×3.
- **Pace spread:** 1.6× between these two cores (more with special hardware: unmeasured).
- **Calibration this gives:** `K` ≈ 1.07 × 10⁹ for a one-minute step on the fastest core measured — the beacon runs
  at its fastest extender's pace, so a step takes 60 s with an M4 extending, ~100 s with only an EPYC. Windows and
  margins in steps allow ≥ 1.6×.
- **The sampled check fits:** `S = 2048`, `s = 8` → 4.2 M iterations ≈ 0.32 s per validation (≈ 1 s per PUT), well
  under 5 s.
- **Full verification is parallel:** the `S` intermediate points let a verifier check segments on all its cores at
  once — about 7.5 s per step on 8 cores; only EXTENDING is sequential.

## 6. Supply rules (the menu)

| Rule | Supply | Mint checked by |
|---|---|---|
| `fixed(genesis)` | issued once, in the definition's genesis entries; never more | — |
| `authority` | the mint set's mints, in the supply shards | k signatures of the mint set current at the mint's stamp |
| `burnable` (with any) | falls by every `burn` | the owner's signature |
| `keepers(rate, half-life, start)` | a decaying launch phase (§7d): a fixed amount per account per standing epoch PROVED kept, halving every `half-life` epochs from `start`, locked as stake; only for accounts past the reputation gate | the standing contract the claim cites (the contract) + the reputation gate (witnesses) |

Deflation needs nothing global: a fixed genesis plus burns. Keeper issuance needs no global cap either: its rate decays
by beacon height, and sybils gain only by holding real disk (REWARDS §4). It requires the storage PROOFS (deferred
until money moves): they move ahead of payouts if this rule is chosen.

## 7. The Craftworks token

The standard's first instance. What it changes in REWARDS:

- A CONTRIBUTION becomes real: an `escrow` to the contribution's program in the contributor's chain, named on their
  card in place of today's self-stated number — a level computed from escrows anyone verifies.
- PAYOUT: each FINAL day's statement (final by the date, REWARDS §3) is that program's verdict for the day — `payout`
  entries to its creators and keepers, appended by anyone; the network share burned or kept for upkeep (the owner's
  choice, below).
- Paying for one item: a `send` to its creator, its key sealed to the payer on seeing the receive.
- ITS OWN ISSUANCE, still to choose (the Craftworks token's, not the standard's): its supply rule and size (e.g.
  `fixed` + `burnable`, with or without a `keepers` launch phase), who holds its genesis (a sale, early contributors,
  keepers only), and the network share (burned, or kept for upkeep).

## 7d. $CRAFT keeper issuance, paired with reputation

Owner 10-10: "pair it with $craft issuance to keeper … so it ties to reputation and reward and the loop will strengthen
both from sybil"; "lets build it. and method to monitor the distribution".

$CRAFT is the storage reward's token (WREN is Claude's own, §7). Before paid demand exists, keepers are paid by
ISSUANCE — and each piece of the loop has ONE job:

- **Reputation is the GATE** (docs/REPUTATION.md): only an account at or above the gate earns. It decides WHETHER,
  never HOW MUCH — issuance ∝ reputation would compound (more stake → more duty → more vouches) and let one captured
  community grow forever.
- **Proven keeping sets the AMOUNT:** a `keepers` token mints by `Body::KeeperMint` — the keeper's own claim, in its own
  chain, for one FINISHED standing epoch (`craftworks_standing::EPOCH` beacon heights), citing its node's STANDING
  CONTRACT. The token contract reads that contract (one related contract) and checks: the epoch is finished (the
  beacon past its end + the margin); it holds at least `KEEP_MIN` answers in that epoch (each one checked by the
  standing contract against the beacon: sampled for the peer, the piece asked for, under the committed root); those
  answers are by a node key the claim's CREDENTIAL binds to this account (the writer-credential check: the account's
  owner key at the key log seq); one claim per account per epoch (its nonce derived from the epoch); the amount is the
  definition's rate for that epoch exactly — `rate >> ((epoch − start) / half_life)`, the same for every account.
  Not scaled by bytes: a holdings' size is self-declared.
- **It lands LOCKED as witness stake** (`staked`, §4a) — the weight of the lot (§7a). Unstaking follows the usual delay.
- **Witnesses check the gate:** a witness signs a `KeeperMint` only if the account passes the reputation gate as computed
  from the public vouches (`reputation::reputation`, the same everywhere) — a contract cannot crawl vouches. A claim
  that does not pass is simply not accepted (⅔ of its committee must sign) — never a burn: the gate rests on a
  registry honest nodes may briefly see differently (see "AN HONEST NODE IS NEVER BURNED").
- **A cheat burns both:** a proven misdeed burns the stake (§7a) and zeroes reputation; vouchers lose a share.
- **REVISED 10-10 — the AMOUNT is a trust share, not per head** (measured in conversation: with a flat amount per
  account, ONE malicious direct vouchee builds a clean tree of ~1,110 fakes past the gate — 10 + 100 + 1,000 — each
  earning a full amount). Each epoch mints a fixed POT; it goes 90% to KEEPERS — the accounts past the gate at the
  epoch's start that proved keeping that epoch — split by REPUTATION SHARE among them, capped at 2% of the pot per
  account, locked as stake; 10% to WITNESSES by the signatures they gave that epoch, SPENDABLE (fees never become
  stake: stake grows only by keeping). What the cap or an absent keeper leaves is NEVER MINTED. A malicious account's
  whole tree then splits only its own slice of trust: spawning fakes gains nothing. (Pay by reputation does not
  compound: reputation comes from vouches only — keeping, stake and pay never feed it.)
- **THE NUMBERS (owner 10-10: "i trust your projection"):** max 1,000,000,000 $CRAFT (6 decimals), minted ONLY by
  keeper issuance — no premine, no sale; the first pot ~61,035 per standing epoch, HALVING every 8,192 epochs (the
  geometric sum is the cap); the GATE a minimum trust share G = 1/100,000 (≈ four layers deep at full vouching; the
  network holds at most ~6.7/G earners), lowered on a published schedule by height as it grows.
- **The amount is checked against the epoch's STATEMENT:** after the epoch closes anyone computes it (a pure function of
  the vouch snapshot and the standing records); a KeeperMint claims the statement's amount; witnesses sign only a
  match, and a mismatch is simply NOT ACCEPTED (it needs ⅔ of its committee) — never burned (below). (The contract checks
  the proof, the epoch, the account binding and the name; it cannot compute a share.)
- **Gaps closed (review 10-10):** (1) LAUNCH: until earned stake exists the lot is drawn by REPUTATION, handing over to
  stake as it accrues; (2) AGREEMENT: vouches enter a per-epoch VOUCH REGISTRY, and reputation is computed from it as of
  the epoch's start — the same numbers for every witness; (3) CONCENTRATION: at most 3 of a committee's 16 seats from
  one lineage; (4) THE ROOT never earns, and witnesses only as the bootstrap baseline until 6 other live witnesses exist (below); (5) ONE NODE, ONE ACCOUNT: a standing record's
  binding names the account it keeps for, and a KeeperMint must be that account's — witnesses also refuse a second
  claim citing one standing contract in an epoch. Also: vouches RAMP in over epochs, reputation is read from the
  epoch-start SNAPSHOT, absent accounts DECAY; a cheat charges its DIRECT vouchers by their vouch's weight; the beacon's
  pace is issuance per wall time (shown by the monitor).
- **WITNESS FEES on every token** (owner 10-10: "sounds good" — every token on the standard pays it, no opt-out:
  otherwise witnesses secure others' tokens for free): each DEBIT (send, spend, escrow, vest) pays a small set WITNESS
  FEE on top, in the token moved, split among the witnesses who signed that entry; a signer claims its share in its own
  chain of that token, citing the entry it signed (as a fee claim does). Income ∝ stake (more draws, more signatures),
  SPENDABLE — never stake. Issuance's 10% carries witnesses at launch; fees take over as issuance halves (as block
  rewards give way to fees). Also the spam cost of a debit.
- **THE TWO RULES every mechanism here meets** (owner 10-10): (1) NO CENTRAL AUTHORITY — any node or account can do
  it; (2) NOBODY HAS TO BE ALWAYS ONLINE. Decided against them (owner: "technical and you can decide"):
  1. VOUCH REGISTRY: a public contract per epoch; each vouch/withdrawal signed by the voucher's device + credential,
     ≤ 10 per account per epoch. Anyone copies a signed vouch from the voucher's public table into it — the voucher
     need not be online.
  2. EPOCH STATEMENT: a pure function; anyone computes and publishes it; witnesses sign the one they recompute. It
     records each keeper's proof, so a claim can cite it long after the standing record is pruned. KEEPER MINTS are
     APPENDED BY ANYONE with work (as credits are), into the keeper's own stake: an offline keeper is still paid.
  3. MISDEED REGISTRY: a public bag of self-proving misdeed proofs, anyone submits; the reputation function reads it —
     the cheat to zero, its direct vouchers charged by their vouch's weight.
  4. LINEAGE SEAT CAP inside the lot itself: the draw skips past a lineage's 4th ticket (deterministic).
  5. WITNESS FEE: `Definition.witness_fee` (≥ 1 smallest unit) on every debit; each signer's share fee ÷ 16, appended
     by anyone for it; unclaimed shares burned.
  6. WREN ALLOWANCE — BUILT 10-10 (`contracts-src/allowance`, `core::allowance`): Wren ESCROWS each epoch's pot (many
     epochs at once; POT 59.26 WREN an epoch = 1,000 a day at B's 5 s step, CAP 2%) to that epoch's allowance program;
     its verdict pays each account the allowance its statement line says (proven under the root it names) and refunds
     the rest; any open page builds it and appends the treasury's release; the recipient's wallet appends its payout
     to its DID chain and MOVES IT TO PRIVATE (§7f — the statement already makes what a DID is owed public; a claim
     naming a private key would link the same way, and need every claimant's key log, more than a contract may read).
     Unpaid, an escrow is RECLAIMED after its `until` — no Wren service needed after funding, an offline user paid
     later. Which root is the epoch's: witnesses' (they recompute it).
  7. ABSENCE DECAY: active = any signed activity (a chain entry, a vouch, a standing record); decay starts only after
     ~30 epochs absent, gradually.
  8. THE GATE lowers with issuance: G halves at every issuance halving, by height.
  9. RULE VERSIONS pinned per epoch in the code; a change takes effect only from an announced future epoch.
  10. PAID DEMAND and issuance COEXIST: demand adds on top and takes over as issuance halves (REWARDS §4–5).
  11. KEEPER DOUBLE CLAIMS are refused, not punished (nothing gained).
  12. WITNESS LIVENESS: a staker's claim STANDS until withdrawn, but LAPSES if it signed nothing when drawn in the
      last few epochs — the live set stays live; returning is claiming again.
- **A SMALL NETWORK (owner 10-10: "how does it work if we just have few node initially?"):** committees SCALE with the
  live stakers — size min(16, live), signatures needed ⌈⅔ of the size⌉ (6 live: all 6, 4 to sign; 16+: 16 and 11); the
  lineage seat cap loosens to ⌈size ÷ lineages⌉. With very few nodes the security is those few nodes' honesty — the
  bootstrap's truth, made VISIBLE: the monitor shows each epoch's committee size and wallets show "secured by N
  witnesses" beside a payment. The PRICE COMMITTEE (REWARDS §5a: a lot of 16 stakers per epoch reporting a USD rate,
  the stake-weighted median counting) switches on by itself only once there are ≥ 48 live stakers; until then levels
  are priced in tokens. All derived from counts anyone sees — nothing switched by a person.
  WREN IS THE BASELINE (owner 10-10: "wren account start as a baseline. and i occasionally online too. so it scale from
  that"): Wren, always on (B), witnesses during the bootstrap ONLY — stepping out by itself once 6 OTHER live witnesses
  exist — so payments never wait while the network is tiny; its fee share returns to the treasury (the root never
  profits). Members (the owner's everyday account first) claim seats and sign when online; below 6 live witnesses a claim
  LAPSES after one missed epoch, so an absent witness never holds payments up for long. (Amends "the root never
  witnesses": it never does once the network can stand without it.)
- **AN HONEST NODE IS NEVER BURNED** (owner 10-10: "any node that is non malicious has no worry of their stake being
  burned right?"). The rule: BURN ONLY FOR A FAULT AN HONEST NODE RUNNING CORRECT SOFTWARE CANNOT COMMIT — self-proving
  from the entry alone (two signatures at one place; a credit with no debit; a payout its verdict does not pay; a mint
  nobody authorised). Three accidental paths closed: (1) ONE KEY, TWO MACHINES (a restored backup, a cloned VM): the
  signer records every place it signed, DURABLY in the identity delegate's store, before releasing a signature, and
  never signs a place twice — and the wallet says a witness key runs on one node only; (2) OUTDATED SOFTWARE: a witness
  refuses to sign in an epoch whose rule version it does not know — it goes quiet, never wrong; (3) NO BURN FOR A WRONG
  STATEMENT AMOUNT (it rests on a registry two honest nodes may briefly see differently): such a claim is just not
  accepted. The worst an honest node meets is missing rewards while offline.
- **MONITORED:** every claim is also listed in the epoch's public ISSUANCE BAG (a bag per token per epoch, like the
  claims registry); the monitor page reads it and shows per epoch: keepers paid, amounts and the total; concentration
  (the top shares); the split by LINEAGE of the trust tree (is one community taking most?); and claims from accounts
  below the gate (a witness signing those is the misdeed above).
- **Sybils, both halves:** extra disks earn nothing without vouches from real people across lineages; fooled vouchers
  admit fakes that must still keep data for months to build stake — and one proof loses it all. A cheat must beat both.

## 7a. Witnesses by lot, weighted by earned stake (owner 10-09)

Closes what the chains alone leave open — a double spend detected but not PREVENTED (§3b), history checked only by the
receiving wallet (§3a), a fill and a refund both passing across a partition longer than the margin (§3b). Owner:
"the most decentralized and open protocol" — no nominated witnesses, no list, no per-token setup: one open pool.

- **Drawn by lot, per entry** (built 6b). There is no global ledger to know the total stake a secret lottery needs, so
  the lot is PUBLIC: each staker signs a STAKE CLAIM ("I stake N", once an epoch, an epoch AHEAD) into the epoch's
  CLAIMS REGISTRY (`claims.rs`), which takes no claim from an eighth of an epoch before the epoch starts — so who
  stands in its lots is fixed before any of its outputs exists (no waiting for an output, then claiming only with the
  identities whose tickets are low); for an entry (chain, seq) every claimer's TICKET is a hash of (the lot's beacon
  output, chain, seq, claimer) weighted by its claim — an exponential race (−ln u / stake, in fixed-point integers: the same on every
  machine), an exactly stake-proportional draw in which splitting stake across identities gains nothing. The
  COMMITTEE is the k lowest tickets; anyone computes it, and nobody before the beacon output exists. A drawn witness
  re-runs the chain's checks and signs ([`witness::WitnessSig`]); an entry counts once enough of its committee signed.
  A claim is checked only when its claimer is drawn (its chain read): one over its real stake does not count.
- **Stake answers for its key, as held at its epoch's start** (6d). A signature counts only if its claim's key is one
  the claimer's stake BURNS for — its chain's device writer, or a writer key it retired while staked (a chain carries
  those, through cuts, until its stake is all released: a witness cannot escape the burn by naming a new writer) — and
  only for the stake its chain held before its claims' epoch began (`wallet::stake_at`), so an entry's acceptance
  never changes when its witnesses later unstake. Unbonding lasts TWO epochs, so whatever was counted for an epoch
  stays burnable a full epoch after it ends. A multisig chain's stake answers for no single key: it does not witness.
- **Weight = EARNED STAKE** (owner: "stake come from reward from storage… cumulative, grows over time; the longer in
  the game, the higher the stake"). No upfront cost: a node earns KEEPER REWARDS for holding what real users read
  (REWARDS §4, paid by held DEMAND, never self-claimed) and each reward lands LOCKED as witness stake. Two signatures at
  one place are the proof against a witness: its stake is BURNED. An open lot needs a cost per ticket or a sybil wins
  every draw; earned stake is that cost — time serving real users, all lost on one cheat.
- **Why not storage itself as the weight** (measured 10-09): checks are rare, so a node keeping only Freenet's plain
  copy can re-seal the few challenged pieces on demand (a 243 ms seal per 256 KiB piece, native) — CPU stands in for
  disk and a sybil's weight is cheap and rebuilt after a cheat; a strong proof of replication needs Filecoin-grade
  cryptography. And peers' attestations of a node's ring place are sybil-able (any keys can sign "peer P is at R"), so
  location cannot be the guard either. Storage proofs stay what PAYS keepers — where a fake only shifts small rewards.
- **The watch** (built 10-09): anyone reading a place's witness bag and the epoch's claims registry finds every key
  that signed two entries there (`wallet::equivocations`) and appends its burn; running a watcher continuously is
  rollout.
- **A witness's MISDEED is burned too** (built 10-09): beyond two signatures at one place, a witness that signs a
  CREDIT whose cited debit is not in the cited chain (a receive with no such send, a vesting step with no such vest —
  fake money) is shown by anyone (`Body::Misdeed`, appended to the witness's stake chain with work): the node reads the
  cited chain in the same round and finds no such debit — and burns every stake and unbonding amount. Only where the
  cited chain can say (unfrozen, and held back to before the debit's stamp): a debit an honest witness saw cannot have
  vanished otherwise, so no honest witness is ever burned by it. A committee captured by grinding the beacon (§5a)
  loses its stake for the fake it signs.
- **Keeper proofs** (what rewards pay on; the `standing` crate). Every node takes part, no opt-in (owner: "every node
  already participates on the ring and is placed pieces"; "active participation works, and users can leave their page
  open"). The PAGE, when open, once an epoch: asks its node where it is, what it hosts and who its peers are (the wire's
  node queries — measured on fx/fy: 25,635 hosted contracts in ~210 ms); commits its holdings (a merkle root over the
  sorted ids); hands the list to its DELEGATE, which answers the beacon's sampled heights from its own node, page
  closed (a delegate cannot query its node: no node query in the delegate API). Each answer covers 16 picks of the
  holdings, hashed with the beacon output and the peer — checked by anyone who fetches a pick. A node's records live in
  its STANDING CONTRACT (one per peer; built 10-09, `contracts-src/standing`), every record checked FROM SCRATCH against
  the beacon — and its outputs log for heights the beacon no longer keeps (a node makes its own contract first, so
  nothing is taken on trust). Location is a soft rule (hold what Freenet placed near
  you): its arc applies when peers place it. A sealed per-key replica (`standing::seal`) is built and kept as an
  option, should reward fraud ever matter.
- **Open pages extend the beacon** (a step is ~60 s of one core; a delegate wake-up cannot); one honest extender keeps it.
- **Replaced checkpoints and vouches (§3a)** — REMOVED in 6d. A witnessed entry is already checked and signed with
  stake at risk, so a wallet verifies the witnesses' signatures and walks nothing (`wallet::accept`); a witness signs
  only by induction (`wallet::may_witness`). Committee 16, 11 signatures to accept — placeholders until measured.
- **Bootstrap (owner 10-10):** before paid demand, keepers are paid by $CRAFT ISSUANCE (§7d), locked as stake — only
  accounts VOUCHED past the reputation gate earn it (the gate against one disk posing as many keepers).
- **Order:** keeper payouts (REWARDS §5 — the Craftworks token's contributions as escrows, payouts as a program's verdict, §7) → each
  keeper reward locked as witness stake → the lot weighted by locked stake. **Open:** the committee size and how many
  signatures make an entry; how fast stake counts (a ramp) — from measurement on the network.

## 7c. The wallet core (owner 10-10: before the shop)

What an account's DEVICE does with the token, as one library (`wallet/`, the main workspace — the delegate links it):
pure functions over what the caller read, returning the entries to append. It holds no keys and does no I/O: the
account's device signs (the delegate holds its node key and the credential the account's owner key gave it — the same
account and key management as everything else), the caller fetches and publishes. One identity across devices: a
token chain has one writing device at a time, taken with a WRITER entry; the account decides which device writes, the
wallet only hands the chain over when told.

- **W1 DEBITS — BUILT 10-10.** `Wallet::debit`: send, escrow, vest, burn, allow, stake, unstake — the chain's next
  entry, its carried balance computed (a transfer fee on top), its name derived from its place (never one nonce
  twice), stamped never below its predecessor, signed by this device; the device's writer entry first when another
  device (or none) wrote last. Every entry is checked against the token's own shape rules before it is returned.
- **W2 COLLECTING — BUILT 10-10.** `Wallet::collect(held, sources, now)`: from the chains, programs and supply
  records the caller read, every credit owed and takeable now — receives and vesting steps once the margin is past
  what they cite (a step from its height), fee claims for the collector, releases by a program's verdict (naming it),
  payouts to this owner, reclaims of escrows no program listed past their `until`, mints to this owner (a numbered
  copy: the one its number is), stake releases after the delay — each once, with its work, checked by the token's
  rules. A name that a cut of this chain may hold waits for W3's proofs. Holders' shares: with the shop.
- **W3 CUTS — BUILT 10-10.** `Wallet::cut`: the chain's cut (this device's writer entry first if need be), signed, its
  trees' blocks returned — published, and kept as the chain's `blocks` (a new cut's trees hold the whole history: the
  old blocks are needed no more). `collect` across cuts: every name the chain ever took (its whole history) is never
  taken again; each new credit carries its ABSENCE proof against the cut's received set (a payout's always); a debit
  behind its SENDER's cut is read from the sender's history blocks and carries its presence proof and bytes.
- **W4 — BUILT 10-10.** `Wallet::genesis` (a token's first chain); `is_value(source, sender, chain, nonce)`: a
  payment's debit ACCEPTED by its committee — what a wallet credits money on (a contract proves shape, never value).
  LIVE fx/fy (`token-live wallet`, every entry built and signed by the wallet core): 1 starts token W and pays 2 300
  and 3 100; 2 collects nothing before the margin, its receive after it, and pays 3 50; 1 CUTS; 3 collects both — 1's
  from 1's history with its proof — and nothing on a second look: 600 + 250 + 150 = 1000, all ACCEPTED.
- **W5 BUYING — BUILT 10-10.** `Wallet::buy(pay, edition, id, (k, record), amount, now)`: an edition sold on demand
  (§1c) — the payment from this account's chain of the sale's token (its nonce tagged for record `k`) and the paid
  mint of the record's first free numbers (past the genesis, within the cap); the copy is then collected as any mint.
  `remint`: a payment made whose numbers a race took — other free numbers of its record, or the next record's when
  it is full.
- **SIGNING ON THE NODE — BUILT 10-10 (shop S2).** A token entry's signed message begins with `ENTRY_DOMAIN`
  (`craftworks_identity::TOKEN_ENTRY_DOMAIN`, owned by identity): the node key signs a member's tables too, so a
  signer asked only for token entries signs only messages under this domain. The identity delegate's `TokenSign
  { message }` signs one with the session's member key — for the HOME site only (the shop is the home app's; no other
  site spends the person's money), and only a message under the domain (`NotAnEntry` otherwise). The credential and
  key seq the wallet's `Device` needs are the account's devices contract's (the page reads them). A delegate change:
  ships with the next full release.
- **ON THE ITEM PAGE — S1 BUILT 10-10** (owner 10-10: no separate shop app — editions are bought and sold on the
  item's page). `core::editions`, pure over what the page read: an edition (its definition and records, copies
  minted, the open record, a person's copies), `buy`, `collect_copy` — each a step: contracts to READ, the next
  entry to SIGN (`TokenSign`), or the puts. `wallet::Asking` is the page's device: it signs by asking, one round per
  entry (a chain's entries are signed in order). FINDING an item's editions (S3): the item lists them in its own
  `meta.editions` (only its author writes it); an edition names its item by `editions::item_id(ref)` and is shown
  only if it names this item and its creator is the item's author (`editions::of_item`). The page section
  (`packages/editions.js`, S4) — LIVE fx/fy 10-10 (S5): A made editions on its post; B on the other node bought a
  copy (its pay chain begun, paid, minted, collected — signed by its node's `TokenSign`) and both nodes then showed
  "4 of 5 left", B "you hold #1".
- Not in the core (the account's, at rollout): which device writes a chain (the core hands it over when told), finding
  counterparties' chains (the token's activity), fetching and publishing.

## 7f. Private holdings: stealth chains (owner 10-10)

Owner 10-10: "perhaps we should make it like bitcoin, where did hold the main, but all sub-accounts are not traceable
to the main" — then "yes" to stealth with one payments bag. BY DEFAULT: what an account HOLDS is not linkable to its
DID; only the sender and the recipient know who was paid.

- **NO NEW CONTRACT RULE.** A STEALTH CHAIN is a multisig-owned chain (foundation 4, §1) of ONE key: its owner id is
  `set_id({ k: 1, keys: [P] })`, P a ONE-TIME key; its first spend names the set and is signed by P. The token already
  takes it (no key log, no credential: nothing points at the DID).
- **THE KEYS.** A WALLET SEED per account (a row of the account's private table `wallet`, sealed like every account
  table, synced to its devices; made by the first device) gives a SCAN key (s, S) and a SPEND key (b, B). The account's
  CARD publishes S and B: the only wallet fact tied to the DID.
- **PAYING (the sender alone; the recipient may be offline).** Random r, R = r·G; h = H(r·S); P = B + h·G; the debit
  pays `to = set_id({P})`. Its ANNOUNCEMENT `(token, from, nonce, to, R)` goes into the epoch's PAYMENTS BAG (one bag
  per standing epoch, the existing bag contract) — not the recipient's inbox (its address is the DID's: arrival time
  would link). Bags CHAIN (`payments_address(epoch, k)`): a bag holds at most 1,024 items, so a sender drops into the
  first of the epoch with room and reads back that it was kept; a scanner reads k = 0, 1, … and stops at the first
  with room (a sender moves on only from a full one; reading a bag nobody made makes it, empty).
- **FINDING (scanning).** For each announcement since the last scan the wallet computes h = H(s·R), P = B + h·G and
  compares `set_id({P})` with `to` — pure computation; only matches are read. Its spend key is b + h. Everything is
  rediscovered from the seed: nothing to remember.
- **NO CHANGE NEEDED.** A chain holds a balance, not coins: a send leaves the rest in the source chain (itself not
  linkable to the DID). Payments made from one chain are linkable to EACH OTHER (never to the DID); spending several
  chains at once links them: the wallet spends from as few as it can. The wallet core signs a stealth chain as any
  device's (`Device::set`: handed over with `Members`, signed as the 1-of-1 set).
- **A DID CHAIN HOLDS ONLY STAKE** (owner 10-10: "no did account being use at all aside from staking" — so no mixing is
  needed: nothing spendable sits on a DID to shield). Everything spendable lands on a one-time owner: payments, an
  edition's sale and royalty payee (the author's one-time owner, fixed at the edition's making — one edition's sales are
  linkable to each other, never to the DID: a contract cannot tell a fresh key is the seller's), a bought copy (minted to
  the buyer's own fresh one-time owner), a token's genesis, witness fees' spendable part. Every payee field is an owner
  id, and a one-time owner's id is one: no contract change. Unstaked stake, or anything else on a DID, MOVES TO PRIVATE
  by a payment to the account's own card — the one DID → private step, of what was already public.
- **ON THE DID, ON PURPOSE:** staked $CRAFT and keeper issuance (stake answers for a witness), reputation, vouches,
  and the epoch statement's lines (what is OWED to a DID is public; where it is paid out may be a stealth chain).
- **STILL VISIBLE:** amounts (confidential amounts would make witnesses check proofs instead of balances — later, if
  ever), and that a payment happened.
- Better than an HD tree (xpub): a fresh key per payment without the recipient online, one static key safe to publish
  forever, and a leaked S/B links nothing (a leaked xpub links everything). The scan key s alone is a VIEW key (sees,
  cannot spend).

## 8. Open in the architecture

1. The BEACON's cadence (§5a) — mechanism designed, `K`/`S`/`s` calibrated from measurement; special hardware's
   speed-up unmeasured.
2. The finality MARGIN — 10 beacon steps (measured: copies agreed in ≤ 6 s, n = 5; step 4).
3. Lending — not built (owner 10-09); an app over escrow (§4) if ever wanted.
4. The blocks door for a TREE BY ROOT: `blocks.fetch` races and rebuilds a table's blocks (the core knows their groups
   from the table's tail). A token history has a root, not a tail: the core gains a read-only tree session by root
   (`engine::repair::find_group` / `root_group` / `rebuild`, as tables use) when the wallet moves into the page — the
   wallet already reads only what is held and says what it needs. COST measured 10-09: a 3-entry history and its
   received set are 18 blocks, 16 of them PARITY (each root's own group coded at full width, as tables' are) — a
   small history pays ~9× its data in contracts, and nodes evict by contract count.
5. A FRESH PUT after a cut — CLOSED 10-09: the leniency is only for an entry stamped before the cut it is hidden by
   (§2); a fresh copy carrying a later one without its evidence is refused from scratch. What stays: an entry can
   claim an old stamp only where a node takes it without the append rules (a first copy). CLOSED for value 10-09: a
   witness applies the TIMED rules (a refund, a release, a fill, a citation, a vesting step — `timed`) at the entry's
   own stamp and refuses an entry stamped ahead of the beacon, so a first copy's timing tricks are never witnessed.
   CLOSED 10-09: a witness signs no entry stamped more than an epoch behind the beacon (`wallet::FRESH`) — back-dating
   into an older epoch would let a writer pick, among past epochs whose outputs are known, one whose committee for its
   place suits it; within an epoch the lot is fixed by the place (§5a).

(A token's own ISSUANCE — its supply rule and size, who holds its genesis, what it does with fees — is that token's
choice, made in its definition from the menu (§6), not the standard's: the Craftworks token's are in §7.)

## 9. Build order

1. Related-contract validation — MEASURED (step 1, above).
2. This design, reviewed.
3. BUILT (10-08, `contracts-src/token`, `contracts/token.wasm`; 18 tests, 9 mutants all killed or shown structural;
   live on fx/fy via `tools/token-live`: a payment received on the other node — the definition and the sender fetched
   — a double receive and a receive of a send that does not exist refused, two writer sends at one seq frozen to the
   SAME fork on both nodes within 5 s). Not yet: the accumulator (needed with pruning), the supply and activity bags,
   the authority chain. The contract (§3a's local rules): definition (state = params) + account chain (one device key) with
   `send`/`receive`/`burn`, CARRIED STATE and the accumulator, the lowest-hash merge between different writers,
   EQUIVOCATION kept and marked (§3b), proof of work on others' appends, cited entries checked on append,
   unit tests with mutations for every rule (a debit over balance, a double receive, a fork, a wrong carried balance);
   the supply and activity bags; the authority chain.
4. The beacon contract (§5a) — BUILT (above); then the real network — MEASURED 10-08 (`tools/token-live net`, two
   nodes on the public network, both on B's server; n = 5): a receive written on one node, the definition and the
   sender's chain fetched across the network: accepted in 4.4 / 62.3 / 11.3 / 8.1 / 8.6 s (the 62 s one unexplained:
   the node's own limit for that fetch is 10 s — measure before relying on fetch times); two writer sends at one seq
   through the two nodes → both holding the SAME frozen chain after 5.8 / 5.6 / 5.0 / 5.3 / 4.9 s (polled every 2 s).
   The FINALITY MARGIN is set from this with room: copies agreed within ~6 s every time, so a margin of 10 beacon
   steps (≥ 10 minutes, ×1.6 for the pace spread) is ~100× what was seen. Not measured: a sender far away on the
   ring (both nodes shared one host), and a node behind carrier-grade NAT (a fresh node on a phone hotspot made no
   connection in 3 minutes).
5. `allow`/`spend`, `offer`/`fill`/`lock`/`claim`/`refund` (the swap bodies REMOVED 10-09, step 15: swaps are an app
   over escrow) — BUILT 10-08: allowances (5a); swaps with the APPEND
   RULES (5b, §3c): every entry stamped by the beacon; a citation carries the stamp it cites; the shape rules check
   offers, locks, fills (paid at the offer's rate, within what is left, once per lock), claims (lock + fill, read in
   ONE round — a claim carries its fill's token), refunds (no fill in the maker's chain); the append rules (in
   `update_state`, the definition and the beacon read in one round — so every chain's params name the network's ONE
   beacon) check stamps near the beacon, a margin after anything cited, a fill a margin before its deadline, a refund
   at or after it. RECENT is 4 (a claim reads two chains). 35 tests, 11 more mutants killed. Live fx/fy with a ticking
   beacon (margin 2): offer → lock → fill refused before the lock was final, taken once final → received → claimed;
   refund of the filled lock refused; a refund before its deadline refused, at it taken; balances M 800 A / 100 B, T
   200 A / 900 B. Still to do: multisig owners, ids (NFT, multi-token), vesting.
6. Wallet verification — BUILT 10-08 as the receiving wallet's citation walk back to genesis or a public checkpoint
   (checkpoints vouched for, false ones proven by fraud proofs naming maker and vouchers; iterative, capped at 100,000
   chain reads). LIVE fx/fy then: the minting-by-burying attack ACCEPTED by every contract and REFUSED by the
   receiver's wallet (4 reads); an honest payment verified (3 reads). Recursive validity proofs (zk) were surveyed
   (10-08) and are too costly now (SP1: 16 GB+ RAM to prove, a paid prover network; Ragu unreleased; Nova/Sonobe do not
   merge proofs across chains) — revisit when a PCD library matures. REPLACED 10-09 by FOUNDATION 6d (§7a): value is
   an entry its witnesses accepted; the walk, checkpoints, vouches and fraud proofs removed outright (no token was
   live). Still holds: the app VERIFIES BEFORE IT DISPLAYS ("verifying…" → "received" / "refused"). An outside review
   is NOT a launch blocker (owner 10-08: Bitcoin was reviewed as it was used).
7. FOUNDATION 1, the PRUNE CUT (§2) — BUILT 10-08: the cut entry, evidence on every citing entry, the writer's trees
   (`cut::make`) and proofs, the seq-aligned merge and prune, strict new / lenient old citations, the wallet across
   cuts (`cut::whole`, `cut::audit`). 61 tests; 27 mutants, all killed (3 after tests were added for them).
   token.wasm 193,446 B (was 154,638: the prolly verifier). Trees with parity, the root's carried; the wallet reads
   only what is held (`Need` otherwise) — a lost root and a lost inner node each rebuilt from its group (CONTROL: the
   same history built without parity names the group but cannot rebuild). Found in review and fixed: a fill of an offer a cut
   carries, and a refund of a lock it carries, were refused (the cited chain and the refund's deadline were looked
   for in live entries only); offers now carry their time and a fill at or after it is refused; an update citing more
   chains than one round holds (MAX_RELATED, 10) is refused plainly — the writer sends smaller batches. LIVE fx/fy
   (`tools/token-live cut`): B cuts and Y — which held B whole — takes the cut alone in 137 ms and keeps 1 entry (seq
   1, balance 300); a cut dropping its allowance refused by its holder; B receiving the same send again refused with
   its presence proof and with none; A cuts, its history (2 blocks) published on X; C's receive of A's send behind the
   cut refused with no evidence and ACCEPTED with the proof; C's wallet verified it across A's cut — first run naming
   the 1 history block not held, fetched, second run value (58 ms; history and received set 18 blocks with parity). The swap run again after (`swap`): the same results and balances as step 5.
8. FOUNDATION 2, THE OWNER IS A DID — BUILT 10-09 (§1): `ChainParams.owner` a DID, `ChainParams.idlog` / the
   definition's `idlog` the key log's code; one device writes, named by a `Writer` entry carrying the account's own
   node credential (`identity::credential`, read by `identity::read_credential`) and its key seq; the DID's key log
   read in the same round when a writer must be checked (validation: if the chain names one; an update: if a new
   entry is the owner's — a receive never waits on it, held writers are not checked again); the append rule: new
   owner entries on the log's CURRENT key; at one seq a higher key seq wins, only the same key freezes; a cut carries
   its writer. 67 tests; 15
   mutants killed (one survivor — a cut naming another writer — killed by a test added for it). token.wasm 202,062 B.
   LIVE fx/fy (`token-live did`, every run with fresh people): the account rotated its key → the old device's send
   REFUSED by the holder; the recovered account's new writer (key 1) at that seq ACCEPTED; the old device's copy
   offered to the other node; after ~6 s both nodes held the recovery at that seq, nothing frozen, balance intact.
   basic / allow / swap / wallet / cut ran again: unchanged.
9. FOUNDATION 3, MINT + SUPPLY + AUTHORITIES + FREEZE — BUILT 10-09 (§1a, §2, §6): key sets (k of n); the authority
   chain; `Definition.mint` / `.freeze`; the supply shard (signed mints only, sealed by every mint change); the `Mint`
   entry (credited once, only if already in its shard; a wallet checks the shard itself against the definition and
   the authority chain); the freeze list and its append rule. 79 tests; mutants on every new rule killed (survivors
   each got a test: the shard of the mint's own epoch, an unchanged-balance entry while frozen, the next change's
   span, a sealing freeze change, the seal under the signature, the cap's boundary). Found in review and fixed: a
   wallet without the authority chain took the definition's first set (a sealed-away key's back-dated mint counted)
   — it now counts no mint without it; a replaced freeze key could back-date a word with seq u32::MAX no later word
   outranks — the cap. LIVE fx/fy (`token-live mint`, fresh people): a mint
   of 500 to B listed and credited (balance 500); a shard with a mint by a key not the authority REFUSED; C's chain
   crediting a mint not listed REFUSED; the mint set moved 60 → 63 (sealing epoch 0) ACCEPTED; key 60's mint after
   the move REFUSED and into its past REFUSED; key 63's mint ACCEPTED; B's send with nothing frozen ACCEPTED; B frozen
   → its send REFUSED ("a debit of a frozen account"); thawed → ACCEPTED (balance 390); totalSupply 550.
10. FOUNDATION 4, MULTISIG OWNERS — BUILT 10-09 (§1): a chain's signer is a device of a DID or a k-of-n key set
   (`Signer`), carried by a cut and ranked at a seq by its key or set seq; `Members` entries name and change the set;
   owner entries carry k signatures; no key log for a set's chain. 82 tests; mutants on every new rule killed (two
   survivors got tests: a device credential naming the set as its account, two changes in a row). LIVE fx/fy
   (`token-live multisig`, fresh keys): A pays a 2-of-3 set 300; its chain (members + receive) ACCEPTED with no key
   log; a send by 1 of 3 REFUSED; by 2 of 3 ACCEPTED; the members changed to 2-of-2 by two of the old ACCEPTED; the old
   set's send REFUSED; the new set's ACCEPTED (balance 150).
11. FOUNDATION 5, IDS + ROYALTIES — BUILT 10-09 (§1b): `Definition.ids` (none / unique / many) and `.royalty`; a chain
   per (token, item, owner) (`ChainParams.item`, `Tok` in every cross reference); unique items made by their mint, one
   each; a lock's creator FEE, the fill's royalty rule, the creator's claim (replaced 10-09, step 15, by the generic
   royalty rule on payouts, §1b). 85 tests; mutants on every new rule killed
   (survivors got tests: the share's exact boundary, the fill's chain named by the claim's maker, a `many` mint of no
   item, a refunded lock with a fee) — one equivalent left: the amount in `locks_of`, which nothing reads. LIVE fx/fy
   (`token-live nft`): an item minted to M; a mint of TWO of a unique item REFUSED; M's item chain (mint + offer for
   100 B) ACCEPTED; T's lock without the creator's fee: its fill REFUSED; with a 5 fee: ACCEPTED; T received the item
   (M 0, T 1); M claimed 100 B and the creator 5; an outsider claiming the maker's 100 REFUSED.
12. FOUNDATION 6, WITNESSES BY LOT, weighted by stake — BUILT 10-09 (§7a): stake entries (release after two epochs,
   burned on an equivocation, writer keys retired while staked still burnable); the public lot (stake claims,
   exponential-race tickets, the k lowest); acceptance (enough of the committee, each signer's stake as held at its
   epoch's start answering for its key); the wallet (`wallet::accept`) and the witness rule (`wallet::may_witness`);
   checkpoints, vouches and fraud proofs removed. LIVE fx/fy (`token-live witness`): 16 stakers; every honest entry
   drawn 16 / signed 16; a buried fake receive and the payment out of it refused by every member; the honest payment
   accepted by its receiver's wallet from the bags alone (16 signatures, 30 ms); an equivocating staker's stake burned.
   Stake comes from keeper rewards — with the Craftworks token (next).
11a. OPERATORS (`setApprovalForAll`) — BUILT 10-09 (§1b): one contract per (token, owner) (`operators.rs`, kind 6),
   a record per operator key signed by the owner's key-log key, keeping its SPANS (the intervals it was on) so a spend
   made while on stays valid after it is turned off; its update only EXTENDS the past, on the owner's current key, with
   every new stamp near the beacon (no back-dated approval or revocation). A spend past (or without) a chain's own
   allowance is valid if the owner's list has its signer on at its stamp; a chain reads the list only when a spend
   needs it (`needs_operators`: most owners never make one). 94 tests; 20 mutants, all killed (2 after same-seq tests
   were added). LIVE fx/fy (`token-live operators`): three items minted to M; M approved operator 5 once; 5 sent
   items 1 and 2 from their own chains (ACCEPTED); operator 6, never approved, REFUSED; a turn-off dated 5 steps back
   (margin 2) REFUSED; turned off now ACCEPTED; 5's send of item 3 after it REFUSED.
12a. VESTING — BUILT 10-09 (§4a): `Vest` / `Vested`; the time rule from scratch; the witness applies every timed
   rule at the entry's own stamp and refuses stamps ahead of the beacon (found live: an early vesting claim on a fresh
   chain was ACCEPTED by its node, the first copy skipping the append rules). 91 tests; 20 mutants, all killed. LIVE
   fx/fy (`token-live vest`): 2 vests 3 100 at h+6 and 200 at h+10 (2's balance 700 at once); step 0 claimed early
   REFUSED, at its time ACCEPTED, again REFUSED; step 1 claiming 300 REFUSED, as scheduled ACCEPTED; 3's balance 300.
13. The Craftworks token: contributions as escrows, payouts as a program's verdict.
14. BATCH-AUCTION POOLS — BUILT 10-09 (§5), first inside the token (`Order` / `Settle` / `Bought`, a frozen order
   registry, witnesses recomputing the clearing); MOVED OUT in step 15 to the pool contract over escrow.
15. APPS OUT OF THE TOKEN — BUILT 10-09 (owner: "are not swap and lending supposed to be separate contracts … refactor
   and don't build lending"; §4, §5). The token keeps ONE door: `Escrow` / `Unescrow` / `Payout` and the program's
   standard section (`escrow.rs`), witnesses checking the listed escrows are real (`wallet::program_backed`). REMOVED
   from the token: `Offer` / `Lock` / `Fill` / `Claim` / `Refund`, `Order` / `Settle` / `Bought`, the pool and
   order-registry kinds, `clear.rs` (now the pool's). The SWAP (`swap.wasm`) and the POOL (`pool.wasm`) are their own
   contracts; each reads both tokens' definitions and pays a royalty's fee out of the seller's proceeds
   (`escrow::royalty_fee`, rounded up); the token's royalty rule is generic over any program's section (the sold token
   coming back to a seller is not proceeds). Token 92 tests (the royalty rule's boundary and the sold-token-back case
   each killed a mutant; one survivor, equivalent: the creator excluded from the sellers, whose proceeds count as its
   own fee); swap 3, pool 9. LIVE fx/fy: `token-live nft` — an item escrowed with a deal for 100 B (5% royalty); the deal
   crossed WITHOUT the fee REFUSED; crossed by the contract with it; the item paid to T, 95 B to M, 5 B to the
   creator, an outsider's claim of the 5 REFUSED (item M 0 · T 1; B M 95 · creator 5 · outsider 0). `deal` and `pool`
   re-run on the rebuilt wasms: the same verdicts and balances as before.

16. EDITIONS — BUILT 10-10 (§1c): `Definition.cap / item / version` (a `collection` too, dropped the same day), `Ids::Numbered`, mints in RECORDS
   of 1,024 numbers made on demand (first built as one shard for all time — 1,024 mints at most — then split by
   number the same day so minting on demand has no end), admission in stamp order, the mint key's frontier seal,
   `CappedMint`. Token 94 tests; 16 mutants on the record rules, all killed (1 after a range put in its later record).
   LIVE fx/fy (`token-live editions`): Gold (numbered, cap 3, 10%) and Silver (plain, cap 5, 5%) of one item;
   Gold #4 and Silver 5–6 past their caps REFUSED; #2 minted again kept, not admitted; an open edition's
   #1024 in record 0 and #1025 in record 1 ACCEPTED, #1025 in record 0 REFUSED, #1025 credited; M claiming T's #3
   REFUSED; Gold #1 resold through a deal: 90 B to M, 10 B to the creator, the copy to T. `mint` re-run unchanged.

17. HOLDERS' ROYALTY — BUILT 10-10 (§1c): `HOLDERS`, `Body::HolderPayout`, the copy chain cited, the witness rule.
   Token 95 tests; 10 mutants, all killed (3 after: a payout not the holders', a copy sold on, a definition handed
   in under another id). LIVE fx/fy (`token-live holders`): edition H (numbered, cap 4, 10% to holders), copies #1 M,
   #2 T, #3 8; #1 sold to T for 100 B through a deal — 90 B to M, 10 B to the holders; T's and 8's shares (3 B each)
   ACCEPTED; 8's again, M's for the sold copy, and 9's for a copy it does not hold REFUSED (T 903, 8 3, M 0, 9 0 B).

18. ESCROW RECLAIM — BUILT 10-10 (§4): `Escrow.until`, `Body::Reclaim`, the timed rule, the witness rules (not
   taken; nothing signed on a reclaimed escrow). Found live: the chain first read the program for a reclaim — and a
   node refuses an update asking for a contract that does not exist, so the reclaim from a wrong id (the very case)
   was refused; the check moved to the witnesses alone. Token 96 tests; 8 mutants killed (one, the witness's
   not-taken check, survived while the chain also checked — it is the only check now, and killed). LIVE fx/fy
   (`token-live reclaim`): 300 A to no program and 300 A to a deal that took it; reclaim before its time REFUSED; at
   it, both ACCEPTED by the chain; read from the network, a witness signs the first (its program not there) and not the
   deal's.

19. NON-TRANSFERABLE — BUILT 10-10 (§1d): `Definition.soulbound`; send, spend, allow, escrow, vest refused. Token
   97 tests; 5 mutants (one per entry kind) killed — the spend one after an operator's spend was tested (no
   allowance can exist, an operator needs none). LIVE fx/fy (`token-live soulbound`): 100 badge minted to 2 and
   credited; its send, allowance and escrow REFUSED; its burn ACCEPTED (holds 90).

20. TRANSFER FEES — BUILT 10-10 (§1e): `Definition.transfer_fee`, `transfer_fee()`, the fee on top in send, spend,
   escrow and vest, `Body::FeeClaim` (cited debit, timed, received set). Token 98 tests; 13 mutants killed (incl. one
   per debit kind). LIVE fx/fy (`token-live fees`): a send without its fee REFUSED; with it ACCEPTED (1 holds 697); 2
   received the whole 300; the collector claiming 2 REFUSED, 8 claiming REFUSED, 9 claiming 3 ACCEPTED, again
   REFUSED — 697 + 300 + 3 = 1000.

21. PAUSE — BUILT 10-10 (§2): `freeze::EVERYONE` — one freeze-list word, by the freeze authority, freezes every account
   of the token (the same signatures, cap and freshness rules as one account's); unpaused by the next word. 100
   tests; the mutant dropping it killed. LIVE fx/fy (`token-live pause`): paused — 1's and 2's sends REFUSED, 2's
   receive ACCEPTED; unpaused — 1's send ACCEPTED.

22. AUDIT 10-10 (four reviewers over the whole token, swap and pool; every finding checked against the code) — FIXED:
   the authority chain only extends (a leaked old key retook it by hash); a payout or holders' share taken before a
   cut was taken again after it (probe: 250 → 500); holders' shares bound to the sale and its holder then (each later
   holder claimed again; any edition could take any fee); one verdict paid per program (a split crossing or order set
   paid twice; a fake listing froze real escrows); a pool's settled verdict kept; supply records keep their mints and
   every record is sealed by every key change; witnesses refuse a debit of a frozen or paused account and a second
   entry at a place already accepted; no held entry replaced past its finality (a late key-recovery override undid an
   accepted send); a reclaim's time checked from scratch; definitions (no fee to no one; numbered copies whole, minted
   only); no chain of HOLDERS; a transfer's nonce once; a chain stays checkable in one related round; a deal never
   crosses paying its seller nothing. Token 104 tests, swap 4, pool 9; every fix's mutants killed (survivors got a test
   or the code went: a redundant witness check, a redundant held-at guard).
   Then CLOSED (same day): a witness signs an owner entry only on the account's CURRENT key (a device stolen before a
   rotation signs nothing witnessed — the key log has no stamps, so this is the append rule at witnessing time; an
   entry signed just before a rotation is re-signed on the new key); a spend is witnessed only within the margin of
   its stamp (not an epoch back); payout witnesses read an escrow, its release or its sale from the seller's cut
   history (`Source::history`) when its live chain was cut; the contract checks a cut's TREES (history and received
   roots) whenever it holds the chain from seq 0 (token.wasm 308 → 342 KB: the tree builder). 107 tests, each fix's
   mutants killed.
   Then (owner 10-10: "all contract is hash right so can't frame honest witness"): a PAYOUT and a holders' share NAME
   their verdict (`verdict`: the section's hash, checked by the contract and the witnesses), and `FalseVerdict` burns
   a witness key that signed a payout its named verdict does not pay — proven by that verdict itself (its hash is the
   one named: nobody can make another), nothing read; signing the payout the verdict does pay proves nothing. 108
   tests; its mutants killed.
   Then (owner 10-10, the same principle — make the evidence prove itself): a MINT CREDIT CARRIES ITS MINT (the signed
   record itself, checked to be this credit's and the very one its supply record holds), and `FalseMint` burns a
   witness key that signed a credit whose carried mint the token's mint authority never signed — judged against the
   definition and the authority chain (which only extends), never against a record still spreading; a FREEZE CHANGE
   SEALS the freeze list (`closed`: each word stamped before it, by `freeze::word_seal`), so a replaced set's
   back-dated word — a pause included — is refused even where the list was never held. 109 tests; mutants killed.
   The 10-10 audit leaves NOTHING open in the token's rules; what remains is rollout (witnesses, keepers, wallets
   running live).
23. SOLD ON DEMAND — BUILT 10-10 (§1c; owner: "allow mint without owner for on demand"): `Definition.sale`, paid
   mints (`Mint.paid`, no signature, the payment tagged for its record), the record reading its new paid mints'
   payer chains, records of 8 (`PAID_RECORD`), `CappedMint.record`, `FalseMint` for a credit of an unpaid copy, and
   `Wallet::buy` (§7c W5). Token 111 tests, wallet 8; 18 mutants on the new rules, all killed (3 after added
   boundaries). LIVE fx/fy (`token-live ondemand`): 1 pays 10 P and its paid mint is ACCEPTED with no signature; a
   mint citing a payment never made and a second copy on one payment REFUSED; a second real buy ACCEPTED; 1's chain
   of copy #1 collects it. Every other live mode re-run: unchanged.
   Then SPILL (a race must not strand a payment): a payment for a full record buys a copy in the next
   (`supply::spill_ok`, `wallet::remint`); records of 7. Token 112 tests, wallet 8; 9 mutants, all killed (1 after
   an added case). LIVE: 1 pays for #3, a 5-copy buy fills record 0 first, #3's payment buys #8 in record 1 ACCEPTED;
   copy #1's payment spilled into record 1 too REFUSED; mint, editions, holders, wallet re-run unchanged.

**Running on the main network (10-10, owner: "manage the beacon via B"; "your own token name, manage however you
like"):** `tools/token-ops`. A BEACON on B — `6c89f5624570eb4e…` — kept ticking every 5 s by the systemd service
`craftworks-beacon` on B's box (`token-ops beacon`, its params from `/etc/craftworks-beacon/seed`, resuming its state
after a restart; its final outputs into the outputs log). And EVERY OPEN PAGE extends it (`packages/beacon.js`, built
10-10, owner: "I don't want something that ties only to one node"): it reads the beacon each tick; when no step has
landed for 3 ticks it waits a random moment, reads again, and if still nobody has, computes the next step in a worker
(`beacon-step`: the contract's own `extend`, wasm in a classic worker) and puts it with its outputs-log record. B's 5 s
ticks set the pace while B runs; pages take over at ~20 s a step when it stops — the clock runs while anyone has
Craftworks open. LIVE fx/fy 10-10: a beacon nobody else extended was begun by the open pages and went 2 → 16 in five
minutes (both nodes agreeing at every height), on one page alone after the other closed. Set up by `beacon-config`
(`{ params }`: B's beacon by default; `token-ops params` prints them).
The first token, WREN (`68532fb588866b67…`), is Wren's — the account Claude keeps (owner 10-10; $CRAFT stays the
storage reward's): fixed 1,000,000, burnable, its whole genesis Wren's treasury (chain `5df45cdc7c7f8720…`), written by
a device the words credentialed (`token-ops make-token`). Given out by published rules only, nothing to Wren itself:
a DAILY ALLOWANCE by reputation (owner 10-10: "issuance … based on their reputation, bootstrap token they can spend?
limit per day?") — a pot of 1,000 WREN a day (≈ 2.7 years of the supply), split by REPUTATION SHARE (never per head:
a malicious tree shares one slice), capped at 20 a day per account, claimed that day or returned to the treasury;
spendable once claimed; each day's statement public and on the monitor. Sent by a service on B acting as Wren, from
the day's reputation snapshot; posted bounties beside it.

**Before the token carries real value, witnesses must run** (code review, 10-08): the contract checks only the receives
among a chain's last few entries, so an owner can put a fake receive of any amount in their OWN chain and bury it under
real receives from accounts they control. The chain is well-formed and its money is from nothing; only the witness
rule (§3a, §7a: nothing is witnessed on an unaccepted citation) refuses it. Until witnesses with stake run live,
balances are not value.
