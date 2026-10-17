# Members and keys on BeeKEM (proposal, 10-17)

Status: PROPOSAL, approved 10-17 — not built. Replaces the MLS group layer of FOUNDATION "Members and keys" once the owner approves it
and the simulation gate (below) passes for every flow. Nothing in the product changes until then.

## Why

- **MLS forks without a server.** MLS assumes a Delivery Service that orders every commit (RFC 9420). We have none (no
  authority, nobody must stay online), so two members committing at once split the group; every rule layered on top
  (turns, healing, anchors — ~60 fixes 10-03 → 10-17) only narrowed the window.
- **Decentralized MLS does not fix it.** The DMLS draft (Kohbrok, expired) accepts forks and leaves resolution open.
- **Voting on one commit per epoch stalls.** Our own simulation (`mls/tests/witnessed.rs`, uncommitted): witness quorums
  never forked, but split votes left positions undecided for good — consensus (Tendermint-style rounds) would be needed,
  which conflicts with "behaviour, not blockchain" (TOKEN).
- **BeeKEM** (Ink & Switch, Keyhive) is a TreeKEM variant built for local-first, peer-to-peer groups: it needs only
  CAUSAL order, and concurrent changes MERGE (both kept as "conflict keys" until a later update resolves them) — nothing
  to fork, nothing to heal. One shared group key, a tree like MLS's.

## What was measured (scratch evaluation, keyhive @ 585e6fc, 2026-10-13)

| Check | Result |
|---|---|
| Our situation, randomized: list changes, every member reconciling at once on stale views, rotations, per-replica delays, offline periods (100 runs, 3,748 ops) | 0 failures: every tick, replicas with the same operations hold the same replayed tree and root key; at the end every listed member derives the same root key, no removed member derives one, the group places exactly the list |
| Control: one operation withheld from one member | caught 30 of 30 |
| Root key from the LIVE (not replayed) replica, same operations | differed in 373 pairs (30 runs) — keys must come from a replay (below) |
| One add into a full group | 0.1 ms (1k), 0.8 ms (10k), 3.0 ms (30k) |
| Rotation, worst case (only one member ever rotated) | 21 ms / 119 KB (1k), 207 ms / 1.2 MB (10k), 625 ms / 3.5 MB (30k) — ~120 B per member |
| Replay (fork + merge) | 3 ms (1k), 36 ms (10k), 116 ms (30k) |
| Rotation size once members have rotated (1k) | 122 KB at the first, 64 KB at the 500th, **3.8 KB** once all had — long-lived groups stay small |
| Rotations with each member KEEPING its replica (300 members, 900 ops) | slowest 6.3 ms, the last 0.35 ms, 0 unrecoverable updates |
| Rotations by a replica REBUILT from scratch over a long history (every earlier update unrecoverable) | 3.9 s (300 members), 131 s (1,000) — every rotation retries rebuilding each; the case of a device that lost its state, which the account's shared leaf (open question 1) avoids |

(Native release builds; a browser's wasm is expected ~3× slower.)

## The model

1. **The member list stays the acts** (`gov`), unchanged. BeeKEM holds the KEYS; the list decides who should hold them.
2. **One BeeKEM tree per group** (a space, a chat, an audience, a circle, a PRO/VIP group; the account's devices — open
   question 1).
3. **Operations travel as each member's own feed** (one writer per feed: the token's owner-chain shape). An operation
   names its predecessors; a reader merges another's operation once it holds the predecessors (causal order) — the same
   way our acts already name their `deps`.
4. **Reconcile, by anyone, at any time:** a member whose view of the tree differs from the list ADDS who is listed and not
   placed, REMOVES who is placed and not listed (a removal only from a whole read of the list — I6). Several members doing
   it at once is safe: concurrent adds of one person fill one leaf; a removal wins over the removed member's concurrent
   update. A member's NODE does it with the page closed. No turns, no takeover.
5. **Keys come from a REPLAY** of the tree's operations (measured above: the live replica's key can differ by arrival
   order). Replay when new operations arrive; the content keys derive from the replayed root (BeeKEM's app secrets).
6. **A newcomer reads at once:** an add carries an INVITATION — the root secrets the adder could reach, sealed to the
   newcomer's key. HISTORY: each update carries its predecessors' root secrets sealed under its own; a newcomer works back.
7. **After concurrent changes the root may be unresolved** until some member updates through the conflict: the member
   that adds or removes also rotates; any member that cannot derive the root rotates.
8. **Large operations** (a worst-case rotation over ~1 MB, from ~8,000 members) travel as our erasure-coded pieces, not
   one contract.

## Flows (walked on paper before any code; each becomes a simulation case)

| # | Flow | Steps | Who must be online |
|---|---|---|---|
| 1 | Join by code / open door / level | the newcomer's join act (proof) → ANY member's page or node sees it listed → adds it (invitation) + rotates → newcomer reads | any one member, eventually (a node) |
| 2 | Manual approval | request → let-in act → as 1 | the approver (writing the act), then any member |
| 3 | Every page closed | reconcile runs on members' nodes | any member's node |
| 4 | Device lost its state | its account is still placed; the device rebuilds from the operations and its account's leaf key (open question 1) | nobody |
| 5 | Member offline for long | merges every operation it missed, in causal order, then replays | nobody |
| 6 | Removed / banned | act → any member removes + rotates → the removed one derives nothing new | any member |
| 7 | Two members change it at once | both operations kept; merged alike everywhere; the next update resolves | nobody |
| 8 | A member removed while it rotates | the removal wins (its update does not survive) | nobody |
| 9 | Ivvor's space | migration: a new tree from the current list (open question 3) | its owner once |
| 10 | Outsider posts operations | an operation is signed by a placed member — an outsider's is not merged (BeeKEM checks the signer) | nobody |

## Messages (the content API — what the product uses)

The simulation with members WRITING messages at random (BeeKEM's `new_app_secret_for` / `decryption_key_for`: content
names the update whose secret sealed it, and its hash): 100 runs, 5,347 operations, 2,405 messages, 0 failures — every
listed member decrypts EVERY message (history included), no removed member decrypts the one written after the list is
final. (The tree's live root can differ by arrival order; content keys never use it.)

## Flows proven (scratch evaluation, each with its control)

| Flow | Result |
|---|---|
| 1 + history: a newcomer | added after 3 updates, the adder then rotating: derives the group's root key, and reaches all 3 earlier updates' secrets |
| 10: an outsider's operation | BeeKEM ALONE takes it (it merges what it is given — authorization is the layer above); our check refuses it |
| Two devices of one account | without the account's new leaf secret the second cannot read; handed it, it reads |
| 4: a device that lost its state | restored from the account's leaf secrets + the operations: reads; its rotation 0.22 ms over 160 operations |
| 6: removed | reads nothing new; an operation it writes from its copy before the removal is refused by our check |
| 7, 8: concurrent changes, a removal racing an update | the randomized simulation (100 runs, 0 failures) |

REQUIRED BY THEM (part of the model):
- **Our check before BeeKEM:** an operation is merged only when its signer is a member the list names (a member id is
  its signing key — the account's), and an add or remove names, as its authorization, the hash of the act that lets
  that person in or puts them out (`CgkaAuthorization`'s 32 bytes).
- **The account hands a new leaf secret to its other devices** (one slot per account).
- **The adder rotates right after an add** (the add blanks the root).

## Decided (owner 10-17: "if that fit the design, go on")

- One slot per ACCOUNT, its devices sharing the leaf secret through the account.
- Each existing space gets a BeeKEM tree made from its current list; history before stays readable with the keys each
  device holds.
- The `beekem` crate, PINNED and VENDORED in the repo (keyhive @ 585e6fc), with the simulation and flow tests beside it.
- (Not decided: reporting the live-replica finding upstream — an outward post, the owner's say.)

## Open questions (decided before code)

1. **A member's leaf belongs to an ACCOUNT, used from several devices.** Share the account's leaf key across its
   devices (as the MLS credential is today), or one leaf per device? (Simpler: per account; devices share the leaf
   secret through the account.)
2. **Content keys and our sealing.** Today tables seal with per-epoch keys from MLS epoch secrets. BeeKEM gives a root
   secret per update; map "epoch" → "update" (each update a key generation) — the storage layer's key walk changes.
3. **Migration.** Existing spaces' MLS groups end; each space gets a BeeKEM tree made from its list; history before the
   switch stays readable with the keys each device already holds.
4. **Maturity.** BeeKEM is PRE-ALPHA and unaudited (its README: "DO NOT use this release in production"). Use the crate
   pinned and vendored, or implement from its design — and report the live-replica finding upstream (owner's say: it is
   an outward post).
5. **No forward secrecy** (BeeKEM keeps old secrets so history stays readable) — the same as our product's choice that
   members read history.

## The gate

Every flow above, and their combinations, in the simulation with the real crate, pass/fail per run, controls proving the
checks fire — before any product code. Then one flow at a time into the product, each proven on fx.

## K3 — where operations live and how they travel (design before code)

1. **One feed per DEVICE, per group: `keyops`** — the device's own tail (its node key: one writer, as every space feed
   today), so two devices never write one tail. The OPERATION inside is signed by the ACCOUNT's member key (BeeKEM's
   signer); our check reads that signer, never the feed's owner.
2. **Sealed with the space's OPEN key** (`space_table_key` from the space id — as the writers list and the door): the
   operations carry only public keys and secrets already encrypted to members (Keyhive syncs them openly), and a
   NEWCOMER must read them before it holds any group key.
3. **Which feeds:** every device of every account the space's credentials name (`storage.writersOf` — the one map of a
   space's writers), the newcomer's own included.
4. **Large operations** (a worst-case rotation over ~200 KB) go as a content-addressed block of the space's (erasure-
   coded, as files); the row holds its hash. Readers fetch the block, check the hash, then merge.
5. **Merging:** every operation read, from every feed, offered to the group in any order; `Waiting` ones offered again
   when another arrives (causal order without a global log). The group is saved after each pass.
6. **Who writes when:** the page while open; the node (K5) when no page ticks — upkeep's existing rule ("a page that
   ticks keeps upkeep out of its way"), so a device's feed has one writer at a time.
7. **Nothing else changes in K3:** content keys still come from MLS until K4 switches the space's tables.

Walked flows: a newcomer (reads feeds by credentials, gets the add with its invitation, merges, reads); a member offline
long (reads every feed's new rows, merges in any order); two devices of one account (two feeds, one member — both
signed by the account key; our check is per signer); a removed member's later rows (still in its feed; refused by our
check); a device that lost its state (reads every feed from the start; the account hands its leaf secrets — K6).

## K4 — a space's content keys from BeeKEM (design before code)

The storage's EPOCH-KEY machinery stays as it is (table keys from an epoch secret — `epoch_table_key`; the identity
keeping each epoch's secret — `keep_epoch` / `tableKeyAt`; the node reading with them). What changes is WHERE an epoch
secret comes from:

1. **A GENERATION is an epoch.** Each BeeKEM update is one: its number the update's DEPTH in the operations × 2^24
   plus 3 bytes of its hash (a later generation is always higher, so the storage's "newest" is the current one), its
   secret the update's root secret (`PcsKey`).
2. **Writing:** with the CURRENT generation — BeeKEM's single-head root (a key change made first when there are
   concurrent heads, as its content API does).
3. **Reading:** every generation this member reaches — its own, those its tree decrypts, and earlier ones through the
   predecessor-secret chain (history) — handed to the identity (`epochKeep`) as epochs of the space; the storage reads a
   row with the epoch it names.
4. **One switch per space:** a space whose BeeKEM tree exists uses it; every other (made before) keeps MLS until K7
   moves it. Nothing in the MLS path changes in K4.
5. **The vendored crate** gains two read-only accessors (a CRAFTWORKS PATCH, as mls-rs's): the current generation
   (`record_tree_root_secret`) and every reachable one (`find_or_derive_root_secret` over the updates) — BeeKEM keeps
   them private.

Walked flows: a member writes (current generation, or a key change first); a newcomer reads history (the chain gives the
earlier generations; the identity keeps them); a removed member (no generation after its removal reachable: rows sealed
after are unreadable to it); two devices of one account (each derives the same generations; the account's leaf secret
handed on — K6); concurrent changes (writing makes the key change that resolves them, readers reach both branches'
generations).

## K6 — the account's devices share its leaf by DERIVATION (built before K5)

A key change's leaf secret is `derive_key("craftworks 2026-10-17 beekem rotation", member seed ‖ the heads it follows)`
— never drawn. A device (or the account's node) merging an update signed by its own account derives the leaf secret
from the update's predecessors and keeps it when its public key matches the update's. Nothing is handed between
devices; a device that lost its state derives every leaf from the operations. (The path secrets above the leaf stay
random; the leaf secret was always the account's — its seed is the account's identity — so no forward secrecy is lost
that the model had: open question 5.)

## K5 — the node keeps the tree to the list with every page closed (design before code)

The follower round (`upkeep`) already reads a space's acts from every member's feeds with no page open. For a space
with a BeeKEM tree it also reads `pub-keyops`, and instead of the MLS steps:

1. **The tree, rebuilt from the operations** each round (no state kept: every leaf secret of the account derives — K6).
2. **Our check** from the replay (`gov`) and the members' CARDS (the `beekem` row: credential checked against the key
   log — the reading already reads each member's card).
3. **Generations kept**: each reached handed to the identity (`keep_epoch`) — the node reads the space's new rows too.
4. **Reconcile** (only from a WHOLE read — I6): ADD each listed account whose card names a member and that the tree does
   not place, authorized by the latest act letting it in; REMOVE each placed member whose account the list no longer
   names, by the act putting it out; then ONE key change. No turns: concurrent adds of one person by several nodes fill
   one leaf; concurrent removals of one member are one removal.
5. **Written** into this member's own feed of `pub-keyops` (`write::rows`, as the re-key round writes `files`); a key
   change needs randomness (the pool a page stirs): none — the round waits, said.

## K7 — every space onto BeeKEM (design before code)

Every group kind is a space (`space.create`: a server, a direct or group chat, a circle, a level group — `groups.js`),
so one move covers them all. (The account's own space is not a group of members: it stays as it is.)

1. **A tree for every space.** A new space: made with it (`space.create`). An existing one (no operation in its
   `pub-keyops`): made by the first LISTED member whose page opens it — its own placement. Several at once: each places
   itself; the adds merge (no fork).
2. **Our check's add rule:** an add is allowed when the account added is LISTED now, or the act it names lets it in;
   reconciling always NAMES that act (a replay after a later removal judges the add by it — "listed now" alone would
   refuse every past add of someone since put out, and the history would not replay: the K5 removal test caught it). A
   removal names the act that puts them out. (Any listed member may place any listed account, itself included.)
3. **One plan, page and node:** whom to add and remove (`groups::plan`: listed and card-named but not placed; placed
   but not listed, by the act that put them out) — the node's round and the page's `members.reconcile` both call it.
   The page reconciles when it opens a space and when its acts or cards change; the node with every page closed (K5).
4. **History across the move — the BRIDGE:** the member that makes the tree writes every MLS epoch secret it holds of
   the space into the space's table `keys-history` (sealed with the newest epoch — the first generation): anyone the
   tree places reads it and keeps those epochs (`epochKeep`), so what was written before the move reads for a
   newcomer too; later members add epochs they hold that are not there.
5. **Writes move at once:** a generation's number (≥ 2^24) is above every MLS epoch, so the storage writes with it
   as soon as it is reached (K4); MLS keeps running beside it until K8 removes it.

Walked flows: Ivvor's space (owner's page opens → tree, bridge, reconcile adds A, B, C → each reads history through
the bridge); a member whose page opens first (places itself; the owner's later placement merges); two at once; a
member removed before the move (not listed: never placed); a newcomer after the move (reconcile adds; the bridge gives
the MLS history); a direct chat (two members, same path).
