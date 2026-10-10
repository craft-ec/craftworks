# Reputation: a weight earned from others, lost on proof

Owner 10-10: "while there is no token stake, perhaps we can include reputation stake? reputation is something that we can
design and implement for our whole ecosystem." This is that design. Status: DESIGN — nothing built.

## 1. What it is for

- **Witness weight before $CRAFT stake exists** (TOKEN §7a): the lot is drawn weighted by something a sybil cannot
  multiply. Today that something does not exist; earned stake comes only with storage rewards.
- **The whole ecosystem afterwards:** who may post where without review, whose flags count, whose invites admit, how a
  feed ranks strangers — every place a space today trusts only its members, or nobody.

## 2. What it cannot be (each measured or argued already)

- **Time alone.** An attacker makes N identities on day one and ages them all at once: time costs nothing per identity.
- **Presence or storage proofs** (standing, TOKEN §7a): a node re-seals the few challenged pieces on demand (243 ms a
  piece) — CPU stands in for disk; peers' attestations of ring place are signed by any keys. Both are sybil-cheap.
- **Votes from anyone.** Fake accounts upvote each other.
- **A minted score.** Someone would have to mint it: an authority. Reputation is COMPUTED, never issued.

## 3. The design: vouches, weighed from roots, burned with the vouchee

- **A VOUCH** is a public signed claim by an account: "I vouch for B" (with a weight from a small fixed budget, e.g. at
  most 10 vouches outstanding). It sits in the voucher's own public table — written by its owner only, like a card.
- **REPUTATION is a pure function of all vouches**, recomputed by anyone (as statements are, REWARDS): trust flows from
  a published set of ROOTS along vouches (EigenTrust / personalised PageRank: each account passes on a damped share of
  what reaches it, split over its vouches). An account nobody reputable vouched for has none, however old or active.
  A sybil ring vouching only for itself has only what flows into it — what it fooled real accounts into giving.
- **THE FAMILY TREE (owner 10-10: "vouch coming from the same tree worth less than those coming from different
  trees").** An account's PARENT is its first voucher (earliest by beacon height); parents up to a root make its LINE,
  and each root's descendants one TREE. Two vouchers' KINSHIP is how far back their lines meet (siblings at their
  parent; cousins higher; different trees never). A vouch counts by the voucher's reputation × its DIVERSITY: in full
  from another tree, most from a distant branch of the same tree, little from close family — and vouches from one
  branch add up with diminishing returns. A sybil ring hung off one fooled account is ONE branch: it gets that branch's
  worth however many fakes it makes; to count it needs vouches from other trees — real people in other communities.
  It also makes many independent roots a strength: a new tree gains standing as other trees' members vouch into it.
- **TIME as a multiplier, not a source:** a vouch counts in full only after it has stood for some epochs (beacon
  heights), so reputation cannot be borrowed for one draw and handed back.
- **STAKED ON THE VOUCHEE.** A proven misdeed of B (the token's: two signatures at one place, a fake credit, a false
  verdict, a false mint — each self-proving, TOKEN §7a) zeroes B and costs every voucher of B a share of its own
  reputation for a period. Vouching is a bet; whoever vouches for sybils pays when one cheats.
- **Decay:** an account that stops taking part (no standing record for some epochs) passes on less; nothing is
  permanent without presence.

## 4. Keeping earns; vouches gate

Owner 10-10: "keeper get reputation + token in the future but for now, it get reputation first".

Two parts, one path from bootstrap to the token:

- **THE GATE — vouch reputation (§3):** does this account count at all. Unvouched accounts may keep data; they earn
  nothing for it. This is what stops one disk posing as many keepers (keeping proofs are sybil-cheap, §2).
- **THE RECORD — keeping:** what a vouched account EARNS — its proven keeping (standing answers, REWARDS §4: pieces
  the beacon-ranked assignment gave it, held, answered), at a fixed rate per account that tapers by height (the
  `keepers(rate, half-life)` rule, TOKEN §6), so splitting one disk over many identities gains nothing.
  - **Paid in $CRAFT from the start** (owner 10-10: "pair it with $craft issuance to keeper"; TOKEN §7d): each finished
    epoch proven kept mints a flat amount LOCKED as witness stake. Two steps for a user: get VOUCHED (past the gate),
    then KEEP (stake grows). Keeping does NOT raise reputation — reputation is social (vouches) and stake is earned;
    if keeping fed reputation, earnings would grow the very thing that gates them (the compounding we ruled out).
    (Replaces the earlier interim "keeping earns reputation until $CRAFT exists".) Paid demand takes over from
    issuance as it grows.

## 5. Witness weight

- Lot weight = the locked $CRAFT stake keeping earned (only vouched accounts earn it).
- The committee's rule stays the token's: a witness signs only by induction; a misdeed is self-proving; the burn reaches
  the record (and its stake), and through the vouch graph the account's vouchers.

## 6. Decided (owner 10-10)

- **THE ROOT IS WREN** (owner 10-10: "wren hold the key and root reputation. i'll be the first layer member as anyone
  else"; earlier "my key can start the vouch root", then moved): `did:craftec:BZ6QnLTT3pnaWtUvfWr9bjtc8nCeRnC2Q7q44XMNUt5h`, the account Claude keeps.
  Safeguards: the owner holds Wren's recovery words OFFLINE too (saved 10-10; to rotate the key and take the root back);
  the root NEVER earns, and witnesses only as the bootstrap baseline until 6 other live witnesses exist (TOKEN §7d) (it holds the most reputation by construction — it would take the cap every epoch and a
  stolen key would be worth the most); its vouches ramp in like anyone's. WREN'S VOUCH POLICY, public: the owner's
  everyday account first; then, over time, up to 9 more people chosen for coming from DIFFERENT communities (each starts
  a lineage — diversity is the system's strength); every root vouch logged with its reason. The owner, a layer-1 member,
  grows a branch of their own below at full speed.
- **ROOTS: a short public list** (now Wren alone). Anyone may vouch, but
  a vouch passes on only the voucher's own reputation — an unvouched account passes nothing until someone reputable
  vouches for it (then its earlier vouches count, from that moment). A self-declared root is just an unvouched account.
- **Roots grow** (owner 10-10: "single root doesn't grow"): (1) at the start, the owner plus a few people from different
  circles the owner names (owner 10-10: none needed — "every account start the same", relationships build the rest: the
  owner's account is the ONLY starting root); (2) PROMOTION: an account that, over some epochs, holds solid reputation
  AND is vouched for from at least three different LINEAGES becomes a root; its descendants a new tree. A LINEAGE is a
  starting root's direct vouchee and everything below it (with one starting root: up to 10 lineages); a promoted tree
  keeps the lineages it came from. A pure function of the vouches like the rest: every node computes the same list.
- **Promotion makes a FAMILY, never a SOURCE** (owner 10-10: "sybil can still create multiple root and vouch for each
  other?"). Trust enters only at the starting roots; a promoted root is a kinship boundary (its descendants count as
  another tree for the diversity discount) and adds no trust. Promoted trees inherit their founders' starting-root
  lineage, so a sybil that got one fake promoted cannot use it to promote more: promotion still needs three distinct
  lineages. A sybil can only fool real people — and then shares only what they gave, split over their 10
  vouches, discounted as close kin, and burned with them on a cheat.
- **One network-wide reputation** (witnesses and keepers); a space may later weigh it with roots of its own for its
  moderation.
- The numbers (vouch budget 10, damping, kinship discount, how long a vouch takes to count, a voucher's loss) are set
  and tuned by measurement.

## 7. Order of building

1. BUILT 10-10 — vouches: `packages/vouch.js` (the account's public tail `vouches`: a row per person, stamped with the
   beacon height; 10 at most; withdrawable), "Vouch" beside Follow on a person's panel (confirmed: public, a bet).
   LIVE fx: A vouched for B; the panel then read "Vouched ✓ · withdraw", also after a fresh load.
2. BUILT 10-10 — `reputation/` (pure, integers): the TRUST tree (an account's parent is the voucher through whom trust
   first reached it from the roots — not merely its first voucher, which a sybil games by having every fake vouch
   before being vouched for, each then its own root), kinship by lowest common ancestor, diversity Π(1 − closeness),
   personalised PageRank (damping 0.85, 40 rounds, budget 10). Measured: a ring of 10 fakes off one fooled account
   takes 6.8 (×10⁻³), a ring of 1,000 takes 7.2 — flat; the same 1,000 with no discount 24.6; cousins' vouches 6.14 vs
   siblings' 4.91.
3. BUILT 10-10 — roots and promotion: `reputation::ROOTS` = Wren (moved from the owner's account the same day); a LINEAGE is
   a starting root's direct vouchee and below; promoted = vouched from ≥3 lineages and holding at least the MEDIAN
   reputation (relative: a fixed bar does not scale as reputation thins with growth) — a family boundary, never a
   source. Measured: promotion moved the network's total reputation by 0.2% (rounding), a ring off one fooled account
   promotes nobody, a ring off three fooled lineages took 15.9 against 73.7 with no discount. The page: `reputation`
   crawls vouches outward from the roots (≤ 400 accounts, kept 10 min) and the person panel shows "Reputation N ·
   vouched by K · heads a family". LIVE fx: B's panel reads "none yet" — A vouched for B, but nothing links A to the
   owner's root (as designed).
4. Burns reaching reputation: a misdeed proof zeroes the account and charges its vouchers.
5. Keeper issuance (TOKEN §7d): `KeeperMint` — the token checks the standing proof and the amount; witnesses the gate.
6. The lot weighted by that stake (already: §7a's earned stake).
