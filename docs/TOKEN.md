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
| **Account chain** | `{ definition id, owner key, code hash }` | the owner's entries, in sequence, and its CARRIED STATE (§2) | the owner — and anyone appending what the chain's own rules PROVE (§3) |
| **Registry** | a public bag | pointers to definitions | anyone (discovery only, like Discover) |

- A token's id is its definition's address. Two tokens never share a chain: an owner has one chain per token.
- **One writer key per chain — a DEVICE's, never an account key shared by several devices** (two devices signing the
  same `seq` would be an equivocation, §3b). An account with several devices has a chain per device (its balance is
  the sum), or names one device its writer; a lost device's chain is moved by an entry signed with the account's
  recovery (the identity's words), never by a second live writer.
- **Authorities** (optional, per definition): a MINT key (or none: supply fixed by the rule), a FREEZE key (or none).
  A multisig authority is a k-of-n key set (as the attestation substrate's named quorum).
- **Supply rule** — a fixed menu, never arbitrary code, so every instance stays checkable (§6).

## 1a. Supply, events, authorities

- **Supply bag** (`supply <definition> <epoch>`, one per beacon epoch so none grows without bound): every `mint` and `burn` pointed there by whoever appends it. `totalSupply` =
  genesis + Σ mints − Σ burns, each pointer checked against its chain (a pointer to nothing counts nothing). Anyone
  recomputes it; it is final as the entries it counts are (§3). **A `mint` is valid only if its pointer is ALREADY in
  its epoch's supply bag** (validation reads that one shard: a related contract) — so no token comes into being where nobody can find
  it.
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
- **Authority chain** (params `{ definition id, "authority" }`): the definition names the FIRST mint and freeze key
  sets; each later entry, signed by the current set, names the next (or none: dropped for good). Validation reads it
  (a related contract) to know who may `mint` or `freeze` at that beacon.

## 1b. Many ids: NFTs and multi-tokens

A definition may say `ids: none` (fungible), `ids: unique` (ERC-721: each id's supply is 1) or `ids: many` (ERC-1155).
An account chain then holds an amount PER ID; every entry names its id(s); a `send` may move several ids at once (one
entry: atomic in the sender's chain). For `unique`, the chain of the id's holder is the `ownerOf` answer; a transfer is
the same send/receive. Per-id metadata is a Craftworks item named by the id.

## 2. A chain's entries

An account chain is a sequence `seq 0, 1, 2, …`, each entry naming the one before (its hash) and CARRYING the state
after it — checked by validation (the previous state changed by this entry), so every check is a read of the head,
never a scan of history (which may be flushed): the BALANCE (per id), the open ALLOWANCES and what is left of each, the
open OFFERS and what is left of each, the open LOCKS, and an ACCUMULATOR (a Merkle root) of every send already
received here and every lock already filled — a `receive` or `fill` carries the proof that its name is NOT yet in it,
and the entry adds it. A PRUNE CUT (older entries flushed into a tree, §1a) is an owner-signed entry naming the tree's
root and carrying the state at the cut: the contract takes it as the chain's start; wallets verify across it (§3a). **A debit is valid only if the balance covers it** at its place. An owner may be a k-of-n key set:
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
- **Checked cheaply, proven wrong by anyone.** A step carries its `S` intermediate points (a Merkle root, and the
  sampled ones opened). The contract recomputes `s` segments chosen from the step's own points (cost `K·s/S`: with
  `S = 1024`, `s = 8`, under 1% of the work). That sample is only a SPEED BUMP: chosen from the step's own points, a
  cheater can regenerate fake points until the sampled segments happen to be honest (half faked: ~1 in 256 tries). The
  SAFETY is the FRAUD PROOF: anyone recomputing a step fully and finding a wrong segment publishes it — that step and
  every one after it lose to any chain without it (the merge: the longest chain with no proven-bad step; equal length,
  the lowest output). So **every extender fully verifies the step it builds on** (twice the work of extending alone),
  and wallets may verify the steps their `until`s depend on.
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
- ITS OWN ISSUANCE, still to choose (the Craftworks token's, not the standard's): its supply rule and size (e.g.
  `fixed` + `burnable`, with or without a `keepers` launch phase), who holds its genesis (a sale, early contributors,
  keepers only), and the network share (burned, or kept for upkeep).

## 8. Open in the architecture

1. The BEACON's cadence (§5a) — mechanism designed, `K`/`S`/`s` calibrated from measurement; special hardware's
   speed-up unmeasured.
2. The finality MARGIN — measured on the real network (step 4), set generously.
3. Pooled lending (§0) — after pools work.

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
4. The beacon contract (§5a), then the real network: finality margin, related fetch latency across the ring.
5. `allow`/`spend`, `offer`/`fill`/`lock`/`claim`/`refund`, multisig owners, ids (NFT, multi-token), vesting.
6. The Craftworks token: contributions as locks, payouts as claims.
7. Batch-auction pools.
8. Wallet verification: the citation walk, public checkpoints and vouches, fraud proofs naming maker and vouchers (§3a).
