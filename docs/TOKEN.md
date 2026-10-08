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
| **Definition** | `{ name, symbol, decimals, supply rule, authorities, nonce }` | none beyond its params (it is its id) | made once; never changes |
| **Account chain** | `{ definition id, owner key }` | the owner's entries, in sequence | the owner — and anyone appending what the chain's own rules PROVE (§3) |
| **Registry** | a public bag | pointers to definitions | anyone (discovery only, like Discover) |

- A token's id is its definition's address. Two tokens never share a chain: an owner has one chain per token.
- **Authorities** (optional, per definition): a MINT key (or none: supply fixed by the rule), a FREEZE key (or none).
  A multisig authority is a k-of-n key set (as the attestation substrate's named quorum).
- **Supply rule** — a fixed menu, never arbitrary code, so every instance stays checkable (§6).

## 1a. Supply, events, authorities

- **Supply bag** (`supply <definition>`): every `mint` and `burn` pointed there by whoever appends it. `totalSupply` =
  genesis + Σ mints − Σ burns, each pointer checked against its chain (a pointer to nothing counts nothing). Anyone
  recomputes it; it is final as the entries it counts are (§3). **A `mint` is valid only if its pointer is ALREADY in
  the supply bag** (validation reads the bag: a related contract) — so no token comes into being where nobody can find
  it.
- **Traceable without one ledger.** Every token starts at the definition's GENESIS entries or at a `mint` (all in the
  supply bag); every `send` NAMES its recipient and every credit CITES its debit. So, from genesis and the supply bag,
  anyone follows sends FORWARD to every chain that ever held the token, and any credit BACKWARD to where it came from
  — the whole history, each entry signed and checked, readable by anyone: an explorer's walk over many contracts
  instead of one. The bags only make it faster (an entry missing from the activity bag is still reached by the walk).
- **History kept.** A long chain's older entries are flushed into a tree (as tables are: carried balance at the cut,
  the entries readable); every chain and tree is kept alive by its readers and keepers (`keep`), as files and tables
  are — Freenet drops what nobody reads, where a blockchain's full nodes keep everything.
- **Activity bag** (`activity <definition> <month>`): every entry pointed there — the EVENT log a wallet or explorer
  reads; a holder's own chain, subscribed, pushes its own changes.
- **Authority chain** (params `{ definition id, "authority" }`): the definition names the FIRST mint and freeze key
  sets; each later entry, signed by the current set, names the next (or none: dropped for good). Validation reads it
  (a related contract) to know who may `mint` or `freeze` at that beacon.

## 1b. Many ids: NFTs and multi-tokens

A definition may say `ids: none` (fungible), `ids: unique` (ERC-721: each id's supply is 1) or `ids: many` (ERC-1155).
An account chain then holds an amount PER ID; every entry names its id(s); a `send` may move several ids at once (one
entry: atomic in the sender's chain). For `unique`, the chain of the id's holder is the `ownerOf` answer; a transfer is
the same send/receive. Per-id metadata is a Craftworks item named by the id.

## 2. A chain's entries

An account chain is a sequence `seq 0, 1, 2, …`, each entry naming the one before (its hash) and CARRYING the balance
after it (per id, for a multi-token) — checked by validation (the previous balance ± this entry), so a balance is one
read of the head. **A debit is valid only if the balance covers it** at its place. An owner may be a k-of-n key set:
an entry it signs carries k signatures.

| Entry | Effect | Written by | Valid when |
|---|---|---|---|
| `send(to, amount, memo)` | debit | owner | balance covers it |
| `receive(send ref)` | credit | **anyone** | the cited send names this owner and is in its chain (related contract) and not received here before |
| `burn(amount)` | debit, no credit anywhere | owner | balance covers it |
| `mint(amount, proof)` | credit | per the supply rule | the rule accepts the proof (§6) |
| `allow(spender, limit, until)` | none (a permission) | owner | — |
| `spend(allow ref, to, amount)` | debit | the spender | within the allowance's limit and time, balance covers it |
| `offer(give, get, rate, max, until)` | none (a standing offer) | owner | — |
| `fill(offer ref, lock ref, amount)` | debit to the taker | **anyone (the taker)** | §4 |
| `lock(to, amount, hash, until)` | debit, held | owner | balance covers it |
| `claim(lock ref, fill ref)` | credit of a lock | **anyone** | the lock names this owner and the fill citing it is in this chain |
| `refund(lock ref)` | credit of the owner's own lock | **anyone** | §4 |
| `freeze` / `thaw` | blocks / unblocks debits | the freeze authority | the definition names one |

**Anyone may append what the chain's own rules prove** — so a receiver, a maker, a lender need never be online: their
chain takes the credit, the fill, the claim by itself. The owner's key is needed only for what spends or permits.

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

What is in a batch is settled like a day of statements (REWARDS §3): FINAL once the epoch and its grace are over, the
orders whose locks every copy holds by then. A late order is in the next batch. Liquidity is pooled; prices are per
epoch, not per trade.

## 6. Supply rules (the menu)

| Rule | Supply | Mint checked by |
|---|---|---|
| `fixed(genesis)` | issued once, in the definition's genesis entries; never more | — |
| `authority` | the mint key's `mint` entries | the mint key's signature |
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

## 8. Open for the owner

1. The supply rule and size (`fixed` + `burnable`, with or without a `keepers` launch phase).
2. Who holds the genesis (a sale, a distribution to early contributors, keepers only).
3. The network share: burned (deflation) or kept for upkeep.
4. The beacon's cadence — it is the clock of every `until`, epoch and grace.

## 9. Build order

1. Related-contract validation — MEASURED (step 1, above).
2. This design, reviewed.
3. The contract (§3a's local rules): definition + account chain with `send`/`receive`/`burn`, carried balances, the
   lowest-hash merge, cited entries checked on append,
   unit tests with mutations for every rule (a debit over balance, a double receive, a fork, a wrong carried balance);
   the supply and activity bags; the authority chain.
4. The real network: finality margin, related fetch latency across the ring.
5. `allow`/`spend`, `offer`/`fill`/`lock`/`claim`/`refund`, multisig owners, ids (NFT, multi-token), vesting.
6. The Craftworks token: contributions as locks, payouts as claims.
7. Batch-auction pools.
8. Wallet verification: the citation walk, public checkpoints and vouches, fraud proofs naming maker and vouchers (§3a).
