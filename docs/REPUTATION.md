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
  - **Now (no paid demand, no token):** the record IS the keeper's reputation — non-transferable, burned on a proven
    misdeed. It is the witness weight (TOKEN §7a).
  - **When $CRAFT launches:** issuance pays the SAME record — retroactively for what was kept during the bootstrap —
    and lands LOCKED as witness stake (TOKEN §7a: "earned stake"). Paid demand takes over from issuance as it grows.

## 5. Witness weight

- Lot weight = the keeping record, counted only for vouched accounts; once $CRAFT exists, the locked stake it earned.
- The committee's rule stays the token's: a witness signs only by induction; a misdeed is self-proving; the burn reaches
  the record (and its stake), and through the vouch graph the account's vouchers.

## 6. Decided (owner 10-10)

- **ROOTS: a short public list, the owner's account first** ("my key can start the vouch root"). Anyone may vouch, but
  a vouch passes on only the voucher's own reputation — an unvouched account passes nothing until someone reputable
  vouches for it (then its earlier vouches count, from that moment). A self-declared root is just an unvouched account.
- **Roots grow** (owner 10-10: "single root doesn't grow"): (1) at the start, the owner plus a few people from different
  circles the owner names — separate trees from day one; (2) PROMOTION: an account that, over some epochs, holds solid
  reputation AND is vouched for from at least three different trees becomes a root; its descendants a new tree. A pure
  function of the vouches like the rest: every node computes the same list.
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
3. Roots: the starting list (the owner's + those named), and promotion (reputation held + vouched from ≥3 trees).
4. Burns reaching reputation: a misdeed proof zeroes the account and charges its vouchers.
5. The keeping record per vouched account (standing answers → a tapering per-account rate), as reputation.
6. The lot weighted by that record (`wallet::lot_of`, the claims registry: a claim of record instead of stake).
7. At $CRAFT launch: issuance paying the record (retroactively), locked as stake.
