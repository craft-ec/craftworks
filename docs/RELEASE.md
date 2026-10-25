# Releasing to the live network

A release needs the owner's say. Every step below is required; a step that could not run is a failed step.
(10-18: v403 lost the owner's channels because nothing upgraded a lived-in account before release; v404 ran for nobody
for hours because three of its packages never reached the owner's node — both found only by the owner.)

## 0. Gate (fx/fy, CLAUDE.md rule 2)

- [ ] `cargo test` for each crate touched since the last live publish; `node --test tools/*.test.mjs`.
- [ ] **THE UPGRADE TEST** (`tools/upgrade-gate/run.sh`): the LIVE tree published on fx/fy, two lived-in accounts
      made there (a space, a channel, a row, a DM); the CANDIDATE published over it; each account logs in with its PIN.
      Must hold, read from the run's output:
      - each unlock opens its own account; recovery ends with every epoch it had (`epochs` includes 0);
      - channels, the row written before, the DM, acts, board and catalog read on the candidate;
      - a row written after the upgrade is read by the other account;
      - the login's `build` lifecycle moves show the old build RETIRED (`RETIRED: yes`);
      - 0 `Delegate operation failed` on fx/fy.

## 1. Build

```sh
cp ~/dev/craftworks/contracts/identity-history <release tree>/contracts/   # main's WORKING copy: every build ever run
./build.sh                                                                 # contracts rebuilt, SHA256SUMS updated
```

- `contracts/signer.wasm` must not move (CLAUDE.md rule 6). A new `identity` hash means every member enters their PIN
  once: say so to the owner before publishing.
- The publisher refuses a build list that drops a build the live version knows (`identity_prior`).

## 2. Publish (through B, under the lock)

`PREVIOUS_SITE` = the live site (`tools/previous-site.sh`), so the identity builds the live version knows are carried
(a page always runs the version served — never an older one: owner 10-25); `FIND_WS` = the owner node's tunnel, so every piece is checked from a node other than B and placed
again where it is not found (up to 8 rounds).

## 3. After — the release reached people

- [ ] **Delivery** (`tools/find-check.sh <dir>`): every live piece fetched through the owner's node; every package must
      have at least `k` pieces. Compare a failure against a FRESH live node (one started for the check): a piece only
      the owner's node cannot get is the node (10-18: restart fixed it), not the placement.
- [ ] **Wren on the live network** (B): Wren logs in on the new version with its PIN, reads its spaces, channels and
      DMs, writes one row; its report names the new version.
- [ ] **Health** (`tools/health-watch/check-live.sh`): versions per account; `lifecycle-build` moves show which earlier
      builds a login retired.
- [ ] Record the release (version, manifest hash, identity build) in `.claude/feature-progress.md`.

## The token build (once, when it goes live)

The token build re-keys token, allowance, standing and bag (no migration, owner 10-11: nothing on them is in use);
releases until then publish with `CRAFTWORKS_TOKEN=off`. Its own gate on fx/fy: statements agree across fx and fy and
are accepted (`token-ops accepted`); keeper listing and claims with the page closed (`token-ops keepers`, `chains`); a
delegate signs a witness entry with its page closed (`token-ops witnessed`); the allowance pays one epoch (escrow →
verdict → release → payout); wasm built without `testlot` (no `BASELINE_SEED` in `contracts/*.wasm`). `signer.wasm`,
`idlog.wasm` and `devices.wasm` must not move.

```sh
# WREN: Wren's account (DID in ~/.config/craftworks/debug-account.txt), 1,000,000, through the B tunnel.
DID=<wren did> tools/token-ops/target/release/token-ops make-token contracts/token.wasm contracts/idlog.wasm \
  "$WS_B" <beacon hex> <words file> ~/.config/craftworks/wren-device.seed WREN 1000000
# Fund the allowance for the coming epochs (each epoch once; rerun to extend).
DID=<wren did> tools/token-ops/target/release/token-ops fund-allowance contracts/token.wasm contracts/idlog.wasm \
  "$WS_B" <beacon hex> <words file> ~/.config/craftworks/wren-device.seed <WREN id> 64
```

Write `packages/tokens-config.json`: `{ "pay": "<edition pay token>", "allowance": "<WREN id>" }`. Beacon:
`packages/beacon-config.json` stays; check the publish trace's "beacon extending <live beacon>" (height 0 = the wrong
config). After: Wren's witness as a service on B (`token-ops witness … craft <key seed>`), restarted on every new
binary; re-run `fund-allowance` before the funded epochs run out. Health reports are sealed to
`packages/support-config.json`'s key, read with `HEALTH_KEY` (`~/.config/craftworks/health.key`).
