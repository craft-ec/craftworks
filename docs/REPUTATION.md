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
- **TIME as a multiplier, not a source:** a vouch counts in full only after it has stood for some epochs (beacon
  heights), so reputation cannot be borrowed for one draw and handed back.
- **STAKED ON THE VOUCHEE.** A proven misdeed of B (the token's: two signatures at one place, a fake credit, a false
  verdict, a false mint — each self-proving, TOKEN §7a) zeroes B and costs every voucher of B a share of its own
  reputation for a period. Vouching is a bet; whoever vouches for sybils pays when one cheats.
- **Decay:** an account that stops taking part (no standing record for some epochs) passes on less; nothing is
  permanent without presence.

## 4. Witness weight

- Lot weight = reputation now; later the lesser of (reputation, stake) or a blend — chosen when $CRAFT stake exists.
  Reputation keeps sybils out of the lot; stake makes a cheat cost money.
- The committee's rule stays the token's: a witness signs only by induction; a misdeed is self-proving; the burn now
  also reaches reputation (and through it the vouchers).

## 5. Open — the owner's

- **The ROOTS.** Someone starts the flow. Candidates: the owner's account; the first N accounts that made public spaces;
  each space choosing its own roots for its own uses (a per-space reputation — no global root at all). The roots are
  public and replaceable; every reader may also compute with its OWN roots (personalised: "people I trust, and whom they
  vouch for").
- **Whether reputation is one network-wide number or per space** (or both: a global one for witnesses, per-space ones
  for moderation).
- The numbers: vouch budget, damping, how long a vouch takes to count, how much a voucher loses.

## 6. Order of building (once the open points are settled)

1. Vouches: the public table, written from a person's page ("Vouch" beside Follow) — page only.
2. The function: reputation from vouches and roots, in the core, tested against sybil rings (a ring of N fakes with one
   fooled real vouch gets no more than that vouch carries).
3. Burns reaching reputation: a misdeed proof zeroes the account and charges its vouchers.
4. The lot weighted by reputation (`wallet::lot_of`, the claims registry: a claim of reputation instead of stake).
