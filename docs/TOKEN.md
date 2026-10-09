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
| Set / drop an authority (SPL `SetAuthority`) | ✅ | the definition's AUTHORITY CHAIN: rotations signed by the current authority; dropping one is final (§1a) |
| Close an empty account (SPL) | ✅ | a zero-balance chain may be dropped; it is made again by its next receive |
| Multisig accounts / authorities | ✅ | an owner or authority key is a k-of-n KEY SET (as the attestation substrate's named quorum): an entry carries k signatures (§1) |
| NFTs — `ownerOf`, transfer, per-item approve, `setApprovalForAll` (ERC-721) | ✅ | a definition with `ids: unique`; a chain per (id, owner) whose head says held or not; `allow` per id or for all (§1b) |
| Multi-token, batch transfer (ERC-1155) | ✅ | one definition, many ids; a chain per (definition, owner) holds an amount per id; a batch = several debits in ONE entry (atomic within the sender's chain) (§1b) |
| Metadata (token and per item) | ✅ | the definition's metadata; per id, an ITEM of Craftworks (its page, its files) named by the id |
| History, traceable by anyone (an explorer) | ✅ | no one ledger: from genesis and the supply bag, sends followed FORWARD reach every chain; credits cite their debits BACKWARD; kept alive by readers and keepers (§1a) |
| Atomic swap between two tokens | ✅ two-phase | lock → fill → claim, or refund; the race closed by time (§4) |
| Vesting, time locks | ✅ | `lock(beneficiary, amount, until)`: claimable by the beneficiary after the beacon passes `until`, or in steps (§4a) |
| Staking | ✅ | `lock` to a staking definition; its rewards a supply rule of that definition (§4a, §6) |
| Escrow, two-party loans | ✅ | locks with release conditions (§4) |
| Order book | ✅ | standing `offer`s in the pair's offers bag, filled with the maker offline (§4) |
| AMM / pooled liquidity | ✅ per epoch | batch auctions: one uniform price per epoch, LPs as curve orders (§5) |
| Pooled lending | ⚠️ per epoch | lenders' and borrowers' offers matched per epoch by the same batch clearing; liquidation by beacon-timed conditions — designed after pools work |
| Composability (contracts calling contracts) | ✅ by reading | a contract validates against OTHER contracts' state (measured, step 1) and anyone appends what those states prove; logic beyond that runs in pure functions anyone recomputes, or delegates. Not: one transaction changing many contracts atomically — two-phase (§4) or batches (§5) instead |
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

A definition may say `ids: none` (fungible), `ids: unique` (ERC-721: each id's supply is 1) or `ids: many` (ERC-1155).
An account chain then holds an amount PER ID; every entry names its id(s); a `send` may move several ids at once (one
entry: atomic in the sender's chain). For `unique`, the chain of the id's holder is the `ownerOf` answer; a transfer is
the same send/receive. Per-id metadata is a Craftworks item named by the id.

**Ownership, metadata and royalties in ONE standard** (owner 10-08): a definition may carry a ROYALTY (a share, and
the creator it goes to); the FILL rule — the one rule every sale passes through (§4) — then refuses a sale of an id
whose lock does not also pay the creator's share. Ownership (ids), the sale (offer → lock → fill) and the royalty are
checked together by one code, the metadata a Craftworks item the id names — where Solana binds three programs (SPL
Token, Metaplex metadata, rule sets / Core plugins for enforced royalties). The limit is everyone's: a plain transfer
(a gift) can hide a payment made elsewhere — royalties hold on sales through the market, not on private deals.

## 2. A chain's entries

An account chain is a sequence `seq 0, 1, 2, …`, each entry naming the one before (its hash) and CARRYING the state
after it — checked by validation (the previous state changed by this entry), so every check is a read of the head,
never a scan of history (which may be flushed): the BALANCE (per id), the open ALLOWANCES and what is left of each, the
open OFFERS and what is left of each, the open LOCKS, and an ACCUMULATOR (a Merkle root) of every send already
received here and every lock already filled — a `receive` or `fill` carries the proof that its name is NOT yet in it,
and the entry adds it. A PRUNE CUT (older entries flushed into a tree, §1a) is an owner-signed entry naming the tree's
root and carrying the state at the cut: the contract takes it as the chain's start; wallets verify across it (§3a).

**The cut uses the PROLLY TREE we already have — no second structure** (owner 10-08). At a cut the owner flushes the
pruned entries into a prolly tree (the HISTORY, read by wallets and explorers) and writes, as a second prolly tree, the
RECEIVED SET as of the cut (every send received, every lock filled or claimed). The cut entry carries both ROOTS and
the carried state (balance, allowances, offers, open locks). After it: a citation of something that could predate the
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
  takes the cut as the chain's start; the WALLET rebuilds both trees from the history it reads and walks the whole —
  a lying cut is a fraud proof at the cut's seq.
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
| `offer(give, get, rate, max, until)` | none (a standing offer) | owner | — |
| `fill(offer ref, lock ref, amount)` | debit to the taker | **anyone (the taker)** | §4 |
| `lock(to, amount, hash, until)` | debit, held | owner | balance covers it |
| `claim(lock ref, fill ref)` | credit of a lock | **anyone** | the lock names this owner and the fill citing it is in this chain |
| `refund(lock ref)` | credit of the owner's own lock | **anyone** | §4 |
| `freeze` / `thaw` | blocks / unblocks debits | the freeze authority (in the freeze list, not the chain) | the definition names one |

**Anyone may append what the chain's own rules prove** — so a receiver, a maker, a lender need never be online: their
chain takes the credit, the fill, the claim by itself. The owner's key is needed only for what spends or permits.

**Spam costs work.** There is no gas, so: an entry appended by someone OTHER than the owner carries a small PROOF OF
WORK (as bags already admit their items), a `send` has a MINIMUM amount (the definition's), and a chain is made only by
a receive of at least that amount.

## 3. Concurrency without a sequencer

Two writers may append at the same `seq` at once (the owner sends while a taker fills). Freenet's copies merge them by
the contract's own rule — **the same winner on every copy**: the entry with the lowest hash. The loser is not lost:

- a `receive`, `claim` or `refund` is re-appended at the next `seq` (by anyone: they are claims on proven facts);
- an owner's `send` / `burn` / `lock` is re-signed by the owner's page or delegate and re-checked against the new
  balance (it may now fail: the balance was spent by the winner);
- a taker's `fill` that loses is simply not there: the taker's lock refunds (§4).

**Finality:** a credit counts once the copies agree. A receiver of value waits a FINALITY margin (to be measured on
the real network: step 4) before treating it as theirs — the one cost of having no sequencer.

**Nothing is credited from an entry that may still lose.** Every entry names the beacon it was written at (no earlier
than its predecessor's, and a beacon that exists); an entry that CITES another chain's entry — a `receive` of a
`send`, a `claim` of a `fill`, a `fill` of a `lock` — is valid only once the cited entry is at least `margin` beacons
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
- The lowest-hash merge (§3) settles only races between DIFFERENT writers (the owner and a taker, two takers) — never
  two entries of the owner's own key.

**No backdating.** An entry names the beacon it was written at; when APPENDED it must name one no older than the
latest the validating node knows, less a small skew. A whole chain handed over from scratch cannot be checked for that,
so nothing TIMED trusts a writer's own stamp: a timeout, an epoch's membership (§5) and a margin are judged by the
beacon at which the cited entry became FINAL — the first beacon at which the copies agree on it, which anyone reading
the beacon contract and the chain recomputes.

**A partition longer than the margin** (copies not agreeing in time) could let a fill and a refund both pass: the
margin is set generously from measurement (step 4), and the two entries, both citing one lock, are themselves the
proof — the lock's later entries voided as an equivocation's are.

## 3a. Who checks what (owner, 2026-10-08)

**The limit.** Freenet validates any copy of a contract's state from scratch (a peer may hand a node a whole chain),
and a validation may read at most 10 related contracts, in one round (measured limits, step 1). A chain's own contract
therefore cannot prove every `receive` in a long chain against its sender. So:

- **The contract enforces the LOCAL rules** — signatures, sequence and links, carried balances, no fork, a debit within
  the balance, no receive twice, the code hash in its params — and, when an entry is APPENDED, checks what it cites
  (≤ 10 related, one round). A chain is always well-formed; it is not, by its contract alone, proven honest.
- **Value is checked by whoever RECEIVES it.** Before treating a payment as theirs, a wallet walks its citations back
  (§1a: traceable) — the sender's chain, where that money came from — until it reaches genesis or a checkpoint it
  accepts. A chain whose past does not check out is worthless to everyone who checks, and provably so.
- **CHECKPOINTS, one kind, public** (owner, 10-08): whoever verifies a chain publishes what they now trust — signed,
  "chain X is honest up to entry N, balance B" — in the token's checkpoint bag. Nothing private, nothing made twice.
- **VOUCHES**: someone who checked the same thing does not make another checkpoint — they VOUCH for the existing one (a
  signed reference to it). Vouches gather on one checkpoint.
- **A wallet decides by who stands behind it**: stopping its walk at its own checkpoints, at ones vouched for by people
  it trusts, or at ones with enough vouches; anything else it spot-checks or walks. A chain owner's checkpoint or vouch
  for THEIR OWN chain counts for nothing (they are the one who could forge it).
- **A FALSE CHECKPOINT IS RECORDED against everyone behind it** — its FRAUD PROOF (the checkpoint and the entries that
  contradict it: a citation that is not there, a balance that does not add up), checkable by anyone, in the token's
  fraud bag, naming the maker AND every voucher. Wallets then skip all their checkpoints and vouches, and their cards
  show it — so vouching carelessly costs what a false checkpoint costs. (Stronger, if ever needed: checkpoints and
  vouches backed by a `lock`, claimable by whoever publishes the fraud proof.)

Chains also refuse what can be refused cheaply: a receive whose cited send is not there when it is appended, a code
hash that is not the standard's — a chain's params name the standard's code hash (an id is `blake3(code hash ‖ params)`,
so a chain recomputes every chain it cites), and readers check a chain's params against the definition's.

## 3c. Shape rules and append rules (10-08, before step 5b)

A rule about TIME cannot be a rule of the state: Freenet re-validates a whole chain from scratch whenever any peer hands
it over, at any later time — a fill appended before its deadline, re-checked against "now" a year later, would fail and
make the whole chain invalid for good. So the contract keeps two kinds of rules:

- **SHAPE rules** (`validate_state`, any time, from scratch): signatures, links, carried balances, allowances, no
  receive twice, the recent citations (§3a) — and an entry's BEACON STAMP never lower than its predecessor's. True of a
  chain forever once true.
- **APPEND rules** (`update_state`: a node that already HOLDS the chain taking in entries it did not have — the only
  moment that knows what is new): each new entry's stamp is within a small skew of the beacon the node reads now (one
  related contract); a `fill` lands before its lock's deadline less the margin; a `refund` lands after the deadline,
  with no fill citing the lock in the maker's chain (a related contract); a `claim` cites a fill final by the margin.

MEASURED (10-08, `tools/related-spike guard`, fx/fy): `update_state` asking for a related contract completes — the
node fetches it and calls `update_state` again with its state — and the contract decides by it: a guard whose lock says
LOCK took its update (v0 → v1); one whose lock says NOPE refused it ("invalid contract update"), staying at v0.

A forged stamp can only pass where the chain was never held (a fresh PUT is checked by its shape alone) — and dies at
every node that did hold it, which applies the append rules before it merges. A chain's time is its stamps; its stamps
were checked against the beacon by every node that held the chain when they arrived.

## 4. Swaps and fills with the maker offline

A maker posts a standing `offer` (in their chain, pointed in the token pair's offers bag). A taker fills it with
nobody's help:

1. **Lock** — in the taker's chain: `lock(maker, 50 B, h, until = beacon T)`.
2. **Fill** — appended to the maker's chain (by the taker): `fill(offer, lock, 100 A)`. Valid only if the offer allows
   it (rate, what is left of `max`, before its `until`), the lock (a related contract) names the maker with enough,
   and the beacon is still before `T − margin`. The fill debits 100 A to the taker — the taker then appends
   `receive(fill)` to their own A chain.
3. **Claim** — anyone appends `claim(lock, fill)` to the maker's B chain: the 50 B are the maker's.
4. **Refund** — if no fill came: after beacon `T`, anyone appends `refund(lock)` to the taker's chain. Valid only if
   the maker's chain (a related contract) holds no fill citing that lock.

**The race** (a fill landing as the refund is checked) is closed by TIME, not by a sequencer: a fill is valid only
before `T − margin`, a refund only after `T`, and `margin` ≥ the finality margin (§3) — so by the time a refund may be
written, every copy has every fill there will ever be.

Two-phase, never half done: either the fill and the claim, or the refund. The same shape gives two-party LOANS
(collateral locked by the borrower, released by repayment or kept after the term) and atomic swaps between any two
tokens of the standard.

## 4a. Vesting, time locks, staking

- **Time lock / vesting:** `lock(beneficiary, amount, until)` — or `until` as a SCHEDULE (amounts released at beacon
  heights). The beneficiary (or anyone for them) appends `claim(lock, step)` once the beacon has passed that step; the
  locker cannot take it back (no `refund` on a vesting lock).
- **Staking:** a `lock` to a STAKING definition (its params name the staked token and its rule). Its rewards are that
  definition's supply rule (§6) — e.g. a fixed amount per beacon per staked unit, minted by proof of the lock — and the
  stake returns by `claim` after its term.

## 5. Pools, by batch auction

An AMM's pool needs one shared value changed in a global order. Instead, a POOL is a definition of its own
(`{ token A, token B, epoch length }`) and a bag per epoch:

1. **Collect** — during epoch `e` (beacon heights), traders post orders and liquidity providers post CURVE ORDERS
   (constant product over a price range), each backed by a `lock` to the pool's epoch.
2. **Clear** — after `e` + grace, a PURE FUNCTION over the epoch's orders gives ONE uniform price and every fill.
   Anyone computes it; order within the batch does not matter (and nobody can front-run).
3. **Settle** — each participant's chain takes its result: `fill`/`claim`/`refund` entries citing the batch's result
   hash, appended by anyone.

What is in a batch is settled like a day of statements (REWARDS §3), from CHAIN DATA ONLY so every clearer computes the
same batch: an order is in epoch `e` if its lock names the pool's epoch `e` and became FINAL (§3b) within `e`'s window;
the batch is final once `e` and its grace are over. A later lock is in the next batch. Liquidity is pooled; prices are per
epoch, not per trade.

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
- **MIX = entropy nobody controls.** `mix(n)` = the hash of EVERY contribution to the beacon's entropy bag that became
  final in step `n` (chain data only: an extender cannot leave one out). A contributor cannot steer the result:
  what their contribution does is unknown until `K` sequential iterations later, after the window has closed.
- **Randomness is used one step late.** `random(n) = out(n+1)`: a faster machine learns `out(n+1)` a little before
  everyone else, but anything drawn from it was fixed a whole step earlier.

**Its pace, honestly:** height grows with sequential speed, not wall time. One extender with a faster single core (or,
later, special hardware for blake3) moves it faster for everyone — by that speed ratio, not by how many machines they
own. So:
- timeouts, margins and windows are counted in steps and set with room for that ratio;
- anything that PAYS per step (staking, keeper issuance) caps its total by HEIGHT reached, never by elapsed steps
  alone, and a token may pin such rewards to a schedule of heights;
- every wallet compares the beacon's pace with its own clock and SHOWS when it runs fast — a pace far off wall time is
  visible to everyone.

**Reading it:** a contract reads the beacon as one related contract (one of its 10). Its state keeps only recent steps
(a day: older ones flushed to a tree, kept, as any history) — enough to judge "final at step n" and `until`s near now.

**BUILT (10-08, `contracts-src/beacon`, `contracts/beacon.wasm`; 12 tests, 7 mutants all killed; live on fx/fy via
`tools/beacon-live`):** small steps (K = 2²⁴) extended through both nodes alike; a FAKE step (output forged — it
passed the 8-of-2048 sample on the FIRST try) plus a step on it accepted and leading at height 5; its fraud proof
(found by recomputing in 0.97 s) with the honest step: both nodes at the honest height 4 within 5 s. ONE REAL STEP
(K = 1.07 × 10⁹, 2048 segments, 8 sampled): computed in 61.5 s on the M4; 65 KB; the node accepted it in 1.2 s. Not
yet: the entropy MIX (outputs are a clock, not yet unpredictable randomness).

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
| `keepers(rate, half-life, start, end)` | a decaying launch phase: per GB-day PROVED held, by beacon-ranked assignment (REWARDS §4) | the storage proof (anyone recomputes it) |

Deflation needs nothing global: a fixed genesis plus burns. Keeper issuance needs no global cap either: its rate decays
by beacon height, and sybils gain only by holding real disk (REWARDS §4). It requires the storage PROOFS (deferred
until money moves): they move ahead of payouts if this rule is chosen.

## 7. The Craftworks token

The standard's first instance. What it changes in REWARDS:

- A CONTRIBUTION becomes real: `lock(contribution, amount, until = its last day)` in the contributor's chain, named on
  their card in place of today's self-stated number — a level computed from locks anyone verifies.
- PAYOUT: each FINAL day's statement (final by the date, REWARDS §3) releases that day's share of the lock — `claim`
  entries to its creators and keepers, appended by anyone, citing the statement row; the network share burned or kept
  for upkeep (the owner's choice, below).
- Paying for one item: a `send` to its creator, its key sealed to the payer on seeing the receive.
- ITS OWN ISSUANCE, still to choose (the Craftworks token's, not the standard's): its supply rule and size (e.g.
  `fixed` + `burnable`, with or without a `keepers` launch phase), who holds its genesis (a sale, early contributors,
  keepers only), and the network share (burned, or kept for upkeep).

## 7a. Witnesses by lot, backed by storage (owner 10-09: "the most decentralized and open protocol"; "the natural
progression is storage")

Closes what the chains alone leave open — a double spend detected but not PREVENTED (§3b), history checked only by the
receiving wallet (§3a), a fill and a refund both passing across a partition longer than the margin (§3b):

- **No nominated witnesses.** Any node stands by PROVING storage it holds (the keepers' proofs, REWARDS §4) — no
  list, no approval, no per-token setup: one open pool for every token.
- **All storage counts, ASSIGNED by lot** (owner 10-09): not only what the node's user read — any piece PUT to the
  network, assigned to nodes by the beacon (rendezvous over node id and piece, as the keepers' queue). The node never
  picks what counts, so its disk keeps the network's data alive rather than junk it made for itself — and data no one
  reads recently is still kept, by those who earn standing holding it.
- **What is at stake: STANDING, earned over time.** A node's weight is its assigned storage PROVED over time
  (GB-days, ramping up), not the disk it shows today; two signatures at one place reset it to zero and forfeit its
  unclaimed keeper rewards (rewards claimable only after a delay, so some are always at stake). A disk cannot be
  confiscated: a new key with the same disk starts from nothing. **Open:** the ramp (days or weeks to full weight) —
  set from measurement on the network, not guessed.
- **Drawn by lot, per entry.** For each (chain, seq), every node computes its own VRF draw over (beacon, chain, seq);
  its chance grows with its proved storage. A drawn node re-runs the chain's checks (the verification primitive) and
  signs, with its draw's proof; an entry counts once it carries enough drawn signatures — checkable by anyone from the
  beacon and the storage proofs. The committee is fresh per entry and unknown in advance.
- **Two signatures at one place** by one witness are the proof against it: its storage standing is lost.
- **Why storage:** an open lot needs a cost per ticket or a sybil wins every draw; storage is the resource the network
  already pays for and proves (as Bitcoin uses work, Ethereum stake, Filecoin storage).
- **Replaces checkpoints and vouches (§3a).** A witnessed entry is already checked and signed with standing at risk, so
  a wallet verifies the witnesses' signatures and walks nothing. No token is live, so there is no older history to
  keep them for: foundation 6 removes checkpoints, vouches and their fraud proofs outright.
- **Depends on** the storage proofs (deferred until money moves, §6) — they come first.

## 8. Open in the architecture

1. The BEACON's cadence (§5a) — mechanism designed, `K`/`S`/`s` calibrated from measurement; special hardware's
   speed-up unmeasured.
2. The finality MARGIN — 10 beacon steps (measured: copies agreed in ≤ 6 s, n = 5; step 4).
3. Pooled lending (§0) — after pools work.
4. The blocks door for a TREE BY ROOT: `blocks.fetch` races and rebuilds a table's blocks (the core knows their groups
   from the table's tail). A token history has a root, not a tail: the core gains a read-only tree session by root
   (`engine::repair::find_group` / `root_group` / `rebuild`, as tables use) when the wallet moves into the page — the
   wallet already reads only what is held and says what it needs. COST measured 10-09: a 3-entry history and its
   received set are 18 blocks, 16 of them PARITY (each root's own group coded at full width, as tables' are) — a
   small history pays ~9× its data in contracts, and nodes evict by contract count.
5. A FRESH PUT after a cut — CLOSED 10-09: the leniency is only for an entry stamped before the cut it is hidden by
   (§2); a fresh copy carrying a later one without its evidence is refused from scratch. What stays: an entry can
   claim an old stamp only where a node takes it without the append rules (a first copy) — the receiving wallet
   walks it whole (§3a).

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
5. `allow`/`spend`, `offer`/`fill`/`lock`/`claim`/`refund` — BUILT 10-08: allowances (5a); swaps with the APPEND
   RULES (5b, §3c): every entry stamped by the beacon; a citation carries the stamp it cites; the shape rules check
   offers, locks, fills (paid at the offer's rate, within what is left, once per lock), claims (lock + fill, read in
   ONE round — a claim carries its fill's token), refunds (no fill in the maker's chain); the append rules (in
   `update_state`, the definition and the beacon read in one round — so every chain's params name the network's ONE
   beacon) check stamps near the beacon, a margin after anything cited, a fill a margin before its deadline, a refund
   at or after it. RECENT is 4 (a claim reads two chains). 35 tests, 11 more mutants killed. Live fx/fy with a ticking
   beacon (margin 2): offer → lock → fill refused before the lock was final, taken once final → received → claimed;
   refund of the filled lock refused; a refund before its deadline refused, at it taken; balances M 800 A / 100 B, T
   200 A / 900 B. Still to do: multisig owners, ids (NFT, multi-token), vesting.
6. Wallet verification: the citation walk, public checkpoints and vouches, fraud proofs naming maker and vouchers (§3a)
   — BUILT 10-08 (`contracts-src/token/src/wallet.rs`): every citation checked and every cited chain walked back to
   genesis or an accepted checkpoint; citation loops caught; checkpoints signed, vouched, audited into fraud proofs
   naming maker and vouchers, who are skipped after. 44 tests, 8 mutants (7 killed by assertion, 1 by a stack
   overflow — the walk is recursive: make it iterative for browser wallets). LIVE fx/fy: the minting-by-burying
   attack ACCEPTED by every contract, REFUSED by the receiver's wallet reading the same chains ("chain 2 seq 0: a
   receive whose send is not in its sender's chain", 4 reads); an honest payment verified (3 reads).
   DECIDED (owner 10-08): THE RECEIVING WALLET IS THE GATE. Recursive validity proofs (zk) would put the check in the
   contract, but were surveyed (10-08) and are too costly now: SP1 (the one released PCD-capable stack keeping ed25519
   and blake3) documents 16 GB+ RAM to prove and points to a paid prover network — a central party; Ragu (PCD-native,
   ~276 ms a step) is unreleased and unaudited; Nova/Sonobe do not merge proofs across chains yet. Revisit when a
   PCD library matures. To make the gate solid: (a) the app VERIFIES BEFORE IT DISPLAYS — never shows or spends
   unverified value ("verifying…" → "received" / "refused: forged, with its proof"); (b) the walk ITERATIVE (no
   recursion depth in a browser wallet) — DONE 10-08: an explicit task stack (a 200-account payment line verified on a
   256 KB stack; CONTROL: the recursive walk overflowed it), and a cap of 100,000 chain reads per verification (a
   wallet never spins forever); (c) checkpoints, vouches and fraud proofs in public bags on the network. An outside
   review is NOT a launch blocker (owner 10-08: Bitcoin was reviewed as it was used).
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
   its writer; a wallet counts none of the owner's keys (log and devices) behind its checkpoints. 67 tests; 15
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
10. The Craftworks token: contributions as locks, payouts as claims.
11. Batch-auction pools.

**Before the token carries real value, step 6 must exist** (code review, 10-08): the contract checks only the receives
among a chain's last 9 entries, so an owner can put a fake receive of any amount in their OWN chain and bury it under 9
real receives from accounts they control — at the cost of 9 minimum sends, which return to them. The chain is
well-formed and its money is from nothing; only a wallet walking its citations (§3a) refuses it. Until wallets verify,
balances are not value.
