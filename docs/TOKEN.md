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

## 2. A chain's entries

An account chain is a sequence `seq 0, 1, 2, …`, each entry naming the one before (its hash). The BALANCE after an entry
is the sum of the credits before it minus the debits; **a debit is valid only if the balance covers it** at its place.

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
3. The contract: definition + account chain with `send`/`receive`/`burn`, the lowest-hash merge, unit tests with
   mutations for every rule (a debit over balance, a double receive, a fork).
4. The real network: finality margin, related fetch latency across the ring.
5. `allow`/`spend`, `offer`/`fill`/`lock`/`claim`/`refund`.
6. The Craftworks token: contributions as locks, payouts as claims.
7. Batch-auction pools.
