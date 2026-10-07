# Rewards: paid demand

Owner, 2026-10-07: CONTRIBUTION, not subscription — anyone gives any amount (a donation), whenever they like, and it
is shared among the people whose content they used and the people whose machines carry it, by what THEY used that
month. Nothing is paid out of thin air: every reward is someone's contribution. (First written 2026-10-06 as a
monthly subscription; the split, statements, ledger and payout are the same — only the amount is the giver's.)

**The token has 8 decimals (owner, 2026-10-07):** 0.00000001 is its smallest amount; every share is rounded to it.

**Split PER DAY (owner, 2026-10-07).** A contribution is spread evenly over a WEEK (7 days) or a MONTH (30
days, the default) — the giver's choice and each day's slice is split by what the giver used THAT day. Contributions that overlap add up on the
days they share (10 over a month and 7 over a week: 1.33 a day for the week, then 0.33). A day with no use sends
its slice to the network. Statements and the ledger are the sum of the days.

## 1. The rule

**A reward is a share of a real payment, never a share of a pool.** Each contributor's contribution is split by what
*that contributor* used. A contributor can only direct their own money, so paying yourself (watching your own videos,
fetching your own pieces) moves your money back to you and earns nothing. There is no "points" pool to inflate.

## 2. The split of one contributor's month

| Share | To | Measured by |
|---|---|---|
| Creators (default 60%) | the owners of the items this contributor read, watched or listened to | the contributor's own usage record (§3) |
| Carriers (default 30%) | the keepers who held and served what this contributor read (§4) — a file with no keeper (but the contributor): its share to its item's CREATOR (owner 2026-10-07) | keepers' claims now, proofs before pay |
| Network (default 10%) | upkeep of what nobody pays for yet: the free tier, the demo, shared infrastructure | — |

The percentages are part of the token contract's published rules (changed only through its own governance), the same
for everyone and not any party's choice.

**Example.** A pays $10. A watched 3 h of Ana's videos and 1 h of Ben's; those files were 9 GB and 3 GB of data for A;
three keepers proved equal shares of Ana's files. Creators' $6: Ana $4.50, Ben $1.50 (by time). Carriers' $3: Ana's
files $2.25, Ben's $0.75 (by data), Ana's split $0.75 to each of her three keepers (by pieces proved). Network $1.
Nothing goes to A's own account or nodes; a file A never touched earns nothing from A.

## 3. Creators: the contributor's own record

Each contributor's pages keep a **usage record** in the contributor's own table (`usage`, sealed like any table):
item → its TIME, per month, in ONE unit for every kind (owner, 2026-10-06): seconds — what played, for a video or
an audio; the time its page was in front of the person (shown, focused, with an input in the last 2 minutes), for
everything else (a note, a paste, a picture, a book). Also kept: opens, and the data its files brought. The
contributor's own pages sum it into weights and SPLIT THEIR OWN CONTRIBUTION by them (the split is user-centric: a contribution follows
only its payer's use, so nobody needs anyone else's record), signed by the contributor's account and published as a
**statement** — one per DAY (owner 2026-10-07: each contribution starts and ends on its own day, so the cutoff is the
day, never a month): today's RUNNING, every earlier day's FINAL (written once, never again); a month or any span is
just the sum of its days.

- Unfakeable by others: only the contributor's account signs its statement; nobody can add usage to someone else's.
- Self-dealing is zero-sum (§1). A creator buying contributions to watch their own work gets back less than they paid
  (the carriers' and network shares).
- Privacy: for now, PUBLIC (owner 2026-10-07: everything public, no central party) — a statement shows what its
  contributor used, as the time counts beside views do. The confidential version is §5's.

## 4. Carriers: keepers who prove they hold

Freenet does not tell a reader which node answered (a get may be answered by any node that cached it on the way), so
bandwidth cannot be paid per answer. Owner, 2026-10-06: **pay the holders, weighted by demand** — and **the users are
the keepers**: a contributor's node already received every piece of what it watched, read or downloaded, so it keeps
what it used (up to a storage limit its owner sets, oldest dropped first; the `keep` capability re-reads it, READ ONLY — owner 2026-10-07: a keeper only keeps, healing is every reader's,
during its read) and registers in the **keepers bag** as its keeper. Supply follows demand by itself (a hit has as many keepers as
past viewers, as torrent seeding), and the money moves between contributors: a contributor's carrier share goes to the
earlier users keeping what they used, and comes back to them from the later users of what they keep — **never to the
contributor's own nodes** (§1). Anyone else may opt in as a keeper too (a creator's own nodes, a space's, paid pinning
of cold files, §6).

- **The keeper's copy is the node's own, held by RE-READING it** (no second store, no node change, no renewal
  traffic). Measured 2026-10-07 on freenet 0.2.142, same set-up as below: the keeper got A and B once, then fetched 60
  others in batches of 10, re-reading A after each batch (answered locally in 2-9 ms); 94 evictions followed — A was
  never evicted after its first get, B (never re-read) was evicted 2 s after its get. Freenet evicts unsubscribed
  contracts least-recently-read first, so a piece re-read more recently than the node's churn stays; a re-read of a
  piece that WAS evicted is an ordinary get, which fetches it back. A SUBSCRIPTION also holds (below) but renews every
  piece over the network every 2 minutes (`SUBSCRIPTION_RENEWAL_INTERVAL`; Freenet's own #3763 "renewal storm") —
  ~330 messages/s for 10 GB of 256 KiB pieces — so keepers re-read instead.
- Earlier measurement, the contribution: Measured
  2026-10-06 on freenet 0.2.139, a gateway plus a keeper node with `--max-hosting-storage` 1 MiB and 64 KiB contracts:
  the keeper subscribed to A and only got B, then fetched 60 others; 84 evictions followed, B among them (got
  10:48:00, evicted 10:48:02), A not. With the gateway stopped, the keeper answered A from its own store in 0.3 ms;
  B did not answer in 30 s. Freenet orders eviction by subscriber count (`ring/hosting/cache.rs`): a subscribed
  contract goes last, and still goes as a last resort if the node stays over its limit with nothing else to drop —
  a keeper keeps its claim inside its node's storage limit. A copy stays as long as it is not evicted; while the
  keeper's page is open its contribution puts the copy last in line, and its proofs show it is still there.
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
- **Share:** a contributor's carrier share goes to the keepers of the files *that contributor* used, each in proportion
  to `pieces of those files it proved held × that contributor's data for those files` (the usage record's bytes, §3).
  A keeper of files nobody pays to use earns nothing; a keeper of a hit earns most.
- **Seen beside the usage:** next to an item's watch time, opens and data, the usage record shows **who hosts it**
  (the keepers proving its files) — the viewer sees where their carrier share goes, the owner sees who carries their
  work.
- **Everyone gets their turn:** on the global network every node eventually takes part in hosting (Freenet places data
  across the ring), so carrying pay spreads across participants over time rather than pooling at a few servers.
- Healing is every reader's (files heal on read: a missing fragment is made again by whoever reads it), paid or not; a keeper
  does not repair — it KEEPS, so that a reader finds enough to read and heal from.
- Exact per-answer bandwidth pay would need the node to name the peer that delivered each piece (a node change, and
  even then only the last hop): not planned.

## 4a. What is signed, what is provable

| Data | Signed by | Provable? | Used to pay |
|---|---|---|---|
| Usage record (time, data, opens) | the contributor's account (its own table) | its author is; its content is the contributor's claim | via the statement |
| Monthly statement (running, then final) | the contributor's account; PUBLIC now, encrypted to its recipients later (§5) | the same: a claim about their own money only | yes |
| Keeper proofs | the keeper's node | yes: anyone recomputes `hash(piece ‖ beacon ‖ node id)` from the public piece | yes |
| Node → account | the node, vouched for by the account's node list | yes | yes |
| Public tally (per-item totals) | anyone may add | no | never: display only |

A contributor's claims only steer their own contribution (§1): misreporting can move it, never mint it or take another's.

## 5. Paying and settling

- **No treasury, no central party (owner 2026-10-07).** Nothing settles but arithmetic anyone can redo:
  - **Contribution:** any amount, in the token, to a token contract, which itself records it (account → month,
    amount) — no one signs it.
  - **Split:** each contributor's own pages split that contributor's contribution (§2, §3) into signed, PUBLIC DAILY
    statements (their account's public tail `statements`, a row per day; listed in the month's bag `statements <month>`):
    today's running, earlier days final.
  - **Ledger:** anyone sums the statements (and the keepers' claims, later proofs) — the same result for everyone; a
    creator or keeper sees their PENDING earnings at any time from the running statements.
  - **Payout:** the token contract pays each contributor's contribution out as their final statement says, IN THE TOKEN — any
    amount, however small: no minimum, nothing carried over. It checks only that a statement's amounts add up to its
    contribution and none is negative (it cannot pay out more than came in).
- **Later, confidential (the real implementation):** the same flow with the amounts hidden. Each contributor encrypts
  each amount under its RECIPIENT's key with additively homomorphic encryption, plus a proof that the amounts add up
  to the contribution and are not negative (as confidential token transfers do); the token contract adds the ciphertexts per
  recipient and checks the proofs, and only the recipient decrypts its own total (its pending view). No key reads a
  statement and no one settles. What stays visible: that a contributor sent something to a recipient, unless entries
  are padded or mixed.
- **What contributing unlocks (owner, 2026-10-06): the private.** Public data is open on Freenet — any client, ours or a
  custom one, reads it free (its pieces are at public addresses, its key travels with the item), and the usage
  record runs only in our pages. So a contribution never gates the public: it unlocks **contributor-only items** (an
  item's key sealed to that month's contributors, as a space's keys are sealed to its members — a client without one gets
  ciphertext), and supports creators. **The public benefits along the way:** a contributor's contribution is split by
  everything they used, public items included, so public creators earn from contributors' viewing, public files are
  kept alive by keepers contributors pay, and the network share carries the free readers.
- **Free tier:** no contribution, no statement; usage is carried by the network share and by keepers' goodwill. A reader who
  bypasses our pages costs no one their pay (no contribution, nothing to split) — only unpaid load, as any free use.

## 5a. Levels, gated items, paying for one item (owner, 2026-10-07)

- **Levels: FREE, PRO, VIP**, by what a person is giving NOW — the sum of their active contributions' daily shares
  (§ Split per day), as a month: PRO from 5 tokens a month, VIP from 20 (placeholders, one place to change). A level
  lasts while its contributions run (a week or a month each), and ends with them: nothing to renew or cancel. Public,
  computed from the contributions' record by anyone — no tier table, no admin (Handcraft's tiers live in a database and
  its pass "signature" anyone can recompute: not repeated here).
- **Contributions are PUBLIC and a level is a BADGE (owner, 2026-10-07):** each person's running contributions are on
  their public card (`directory`: only their account writes it, anyone reads it), so anyone computes their level; every
  name drawn shows it — ⭐ PRO, 💎 VIP. (Self-stated until the token: the token contract's record replaces the card's.)
- **An item's minimum level** (creator's choice: everyone, PRO, VIP): the item ENCRYPTED, its key sealed to the people at
  that level — never one server secret that decrypts everything (Handcraft's `CONTENT_ENCRYPTION_SECRET`).
- **Paying for one item:** in the token, to its creator; the buyer gets the item's key sealed to them.
- **Who hands over a key (owner, 2026-10-07): the existing groups and upkeep — no new mechanism.** Each creator's PRO
  and VIP are GROUPS (as circles: friends, followers), a gated item sealed to the group's key. Joining is the existing
  ASK → ADMIT flow, and admitting needs no page open (the identity delegate, woken every minute, admits askers); the
  group's rule is "members may invite", so ANY current member's node admits a person whose public card shows the level
  — the creator need never be online. A lapsed level: removed by any member (anyone verifies it from the card), the
  group moving to a new key. Honor system, accepted: a malicious member could admit a non-contributor, but honest
  members' nodes remove whoever's card does not show the level on their next tick.
- **Built (2026-10-07; the identity build changed for it once, owner's say):** the level rule is ONE function
  (`craftworks_gov::level_from`: the page through the core, the identity delegate directly). A creator's gated post is
  SEALED in their level group (`circles.level`: join policy `level:pro|vip`) with a public TEASER (`meta.gate`); a
  reader at the level who opens it asks in (bag `level <group>`); any member's page — or the creator's node with no
  page open (the delegate reads the asker's card) — lets them in (`admitted`, code `level`); a member whose card falls
  below is taken out by any member (`lapsed`: not a ban). Posts gated before this are still the page-only gate.

## 6. What is open, and the proposal for each

- *Central trust:* none (§5) — the token contract only checks arithmetic; statements are public now, encrypted to
  their recipients later.
- *Sybil contributors:* cannot mint money (every reward is a real contribution); they only cost the network share, which is
  bounded by the free tier's limits.
- *A creator who is also a keeper:* allowed; each share is earned separately and both are bounded by real contributions.
- *Paid pinning (cold files nobody watches):* the owner pays keepers directly to hold them, by the same proofs (§4).

- *Proof tuning (concept only — to discuss and detail):* the queue's shape is decided (continuous, deterministic,
  random per beacon, bound to the node id); its numbers are not. To settle: the beacon's source and cadence (minutes
  is likely too often); the share of a claim covered per day and so the length of a pass; the answer window; the cost
  to a keeper (reads, hashes, writes of answers to the network) and to checkers at network scale; who checks and how
  many samples; what a missed slice costs (that window's pay only, or the claim's month); behaviour for nodes that
  sleep (home machines: offline is not lost); and where answers are written (one contract per keeper, the bag, a
  log).

## 7. Build order

1. Usage record in the contributor's table (no money: shows "your month" to the user) — BUILT. Per-item TIME and DATA
   shown as public counts beside views (owner 2026-10-06: as a view count for now; a confidential roll-up replaces it).
2. Keep what you watched (re-read daily, read only, within a limit) — BUILT.
3. Keepers bag: claims of who keeps which file — BUILT (claims, unsigned, public).
4. Public DAILY statements (today running, earlier days final), the ledger anyone sums and each one's pending view
   (Usage app: "Your earnings") — BUILT 2026-10-07, in shadow mode (computed, not paid) for a few months to calibrate
   the split; carriers' shares from CLAIMS; a day that does not add up is left out of every sum. No treasury (§5).
5. PROOFS, only before money moves (owner 2026-10-07: skip until rewards pay — a claim is enough to show who keeps
   what; a proof only stops paying a false one). Planned so they cost the network almost nothing: computed during the
   keeper's existing daily re-read (no extra gets), one small write per keeper per day (about 13 KB for 10 GB kept:
   ~1% of its pieces × 32 bytes), the day's Discover head as the beacon (already read by every node), checked for free
   by readers who fetch a challenged piece anyway and by a monthly sample at settlement. A receive-and-delete keeper
   pays by fetching ~1% of its claim every day; an honest one pays nothing on the network.
6. Payouts.
