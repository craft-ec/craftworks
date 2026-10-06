# Rewards: paid demand

Owner, 2026-10-06: people pay a monthly subscription; the money is shared among the people whose content they use and
the people whose machines carry it. Nothing is paid out of thin air: every reward is someone's subscription.

## 1. The rule

**A reward is a share of a real payment, never a share of a pool.** Each subscriber's monthly fee is split by what
*that subscriber* used. A subscriber can only direct their own money, so paying yourself (watching your own videos,
fetching your own pieces) moves your money back to you and earns nothing. There is no "points" pool to inflate.

## 2. The split of one subscriber's month

| Share | To | Measured by |
|---|---|---|
| Creators (default 60%) | the owners of the items this subscriber read, watched or listened to | the subscriber's own usage record (§3) |
| Carriers (default 30%) | the keepers who held and served what this subscriber read (§4) | keepers' possession proofs for those files |
| Network (default 10%) | upkeep of what nobody pays for yet: the free tier, the demo, shared infrastructure | — |

The percentages are a setting of the Craftworks treasury (governed, published), not hard-coded.

## 3. Creators: the subscriber's own record

Each subscriber's pages keep a **usage record** in the subscriber's own table (`usage`, sealed like any table):
item → time watched / listened (media), opened (text), downloaded (files), per month. At month end the record is
summed into weights, signed by the subscriber's account, and published as a **statement** (one per subscriber per
month). The creators' share is split by those weights.

- Unfakeable by others: only the subscriber's account signs its statement; nobody can add usage to someone else's.
- Self-dealing is zero-sum (§1). A creator buying subscriptions to watch their own work gets back less than they paid
  (the carriers' and network shares).
- Privacy: a statement names items and weights only to the treasury that settles it (sealed to the treasury key),
  not to the public. A subscriber may opt to publish theirs.

## 4. Carriers: keepers who prove they hold

Freenet does not tell a reader which node answered (a get may be answered by any node that cached it on the way), so
bandwidth cannot be paid per answer. Owner, 2026-10-06: **pay the holders, weighted by demand** — and **the users are
the keepers**: a subscriber's node already received every piece of what it watched, read or downloaded, so it keeps
what it used (up to a storage limit its owner sets, oldest dropped first; the `keep` capability re-reads and repairs
it) and registers in the **keepers bag** as its keeper. Supply follows demand by itself (a hit has as many keepers as
past viewers, as torrent seeding), and the money moves between subscribers: a subscriber's carrier share goes to the
earlier users keeping what they used, and comes back to them from the later users of what they keep — **never to the
subscriber's own nodes** (§1). Anyone else may opt in as a keeper too (a creator's own nodes, a space's, paid pinning
of cold files, §6).

- **The keeper's copy is the node's own, held by a SUBSCRIPTION** (no second store, no node change). Measured
  2026-10-06 on freenet 0.2.139, a gateway plus a keeper node with `--max-hosting-storage` 1 MiB and 64 KiB contracts:
  the keeper subscribed to A and only got B, then fetched 60 others; 84 evictions followed, B among them (got
  10:48:00, evicted 10:48:02), A not. With the gateway stopped, the keeper answered A from its own store in 0.3 ms;
  B did not answer in 30 s. Freenet orders eviction by subscriber count (`ring/hosting/cache.rs`): a subscribed
  contract goes last, and still goes as a last resort if the node stays over its limit with nothing else to drop —
  a keeper keeps its claim inside its node's storage limit. A copy stays as long as it is not evicted; while the
  keeper's page is open its subscription puts the copy last in line, and its proofs show it is still there.
- **Proof of holding — a continuous, deterministic random queue** (as ZephCraft's HealthScan: a small slice every
  cycle, never one big daily round; `craftec/docs/CRAFTOBJ_DESIGN.md` §HealthScan, rendezvous by
  `BLAKE3(node_id ‖ cid ‖ epoch)`). At a cadence still to be tuned (§6), a public **beacon** turns over (the newest Discover head — known
  to nobody beforehand); each keeper's queue is its claimed pieces ranked by `BLAKE3(beacon ‖ node id ‖ piece)`, and it
  answers the head of that queue — a slice sized so the queue covers a set share of its claim over time (the share and pace to be tuned, §6) — with
  `hash(piece ‖ beacon ‖ node id)`, within that beacon's window. Deterministic: anyone recomputes which pieces were due
  and checks a sample against the public pieces. Random: nobody knows the next slice before its beacon. The node id
  makes every keeper's answer its own (none can copy another's); a slice not answered in its window earns nothing.
- **Why holding beats fetching on demand:** the challenge grows with the claim, so a keeper that deletes and re-fetches
  pieces only when challenged ends up re-fetching the whole file once per pass of its queue (at, say, 1% a day: every 100 days), forever — bandwidth costs more than
  disk, so receive-and-delete costs more than keeping. Bandwidth is the overhead of carrying, never the thing paid;
  honest keeping is the cheapest way to answer. (Stronger, if ever needed: a copy unique per keeper, slow to make.)
- **What it pays for is durability:** Freenet routes a get by ring position, not to keepers, so a keeper answers only
  the gets that pass through it. Carrier pay is pay for keeping copies alive, weighted by the demand on them.
- **Share:** a subscriber's carrier share goes to the keepers of the files *that subscriber* used, each in proportion
  to `pieces of those files it proved held × that subscriber's data for those files` (the usage record's bytes, §3).
  A keeper of files nobody pays to use earns nothing; a keeper of a hit earns most.
- **Seen beside the usage:** next to an item's watch time, opens and data, the usage record shows **who hosts it**
  (the keepers proving its files) — the viewer sees where their carrier share goes, the owner sees who carries their
  work.
- **Everyone gets their turn:** on the global network every node eventually takes part in hosting (Freenet places data
  across the ring), so carrying pay spreads across participants over time rather than pooling at a few servers.
- Readers repair and keep (files that heal on read) work whether or not anyone is paid; keepers make them reliable.
- Exact per-answer bandwidth pay would need the node to name the peer that delivered each piece (a node change, and
  even then only the last hop): not planned.

## 5. Paying and settling

- **Subscription:** paid on an external rail (card or stablecoin) to the Craftworks treasury, which signs a monthly
  **pass** to the account (a register entry: account → month). Pages show subscriber features to a pass holder.
- **Settlement:** at month end the treasury sums the statements and the keepers' proofs, publishes the monthly
  ledger (who earns what, from which statements, all checkable), and pays out on the same rail. Earnings below a
  minimum carry over.
- **Free tier:** no pass, no statement; usage is carried by the network share and by keepers' goodwill.

## 6. What is open, and the proposal for each

- *Treasury trust:* one key settles payouts. Proposal: a k-of-n signer set (the owner plus independent members), and
  a public ledger anyone can recompute from statements and proofs.
- *Sybil subscribers:* cannot mint money (every reward is a real fee); they only cost the network share, which is
  bounded by the free tier's limits.
- *A creator who is also a keeper:* allowed; each share is earned separately and both are bounded by real fees.
- *Paid pinning (cold files nobody watches):* the owner pays keepers directly to hold them, by the same proofs (§4).

- *Proof tuning (concept only — to discuss and detail):* the queue's shape is decided (continuous, deterministic,
  random per beacon, bound to the node id); its numbers are not. To settle: the beacon's source and cadence (minutes
  is likely too often); the share of a claim covered per day and so the length of a pass; the answer window; the cost
  to a keeper (reads, hashes, writes of answers to the network) and to checkers at network scale; who checks and how
  many samples; what a missed slice costs (that window's pay only, or the claim's month); behaviour for nodes that
  sleep (home machines: offline is not lost); and where answers are written (one contract per keeper, the bag, a
  log).

## 7. Build order

1. Usage record in the subscriber's table (no money: shows "your month" to the user). 2. Keepers bag + daily proofs
(no money: shows who holds what beside the usage record — useful for durability now). 3. Treasury pass + statements + monthly ledger in
shadow mode (computed, not paid) for a few months to calibrate the split. 4. Payouts.
