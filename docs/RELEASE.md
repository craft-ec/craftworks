# Releasing the token build to the live network

The token build re-keys token, allowance, standing and bag (no migration, owner 10-11: nothing on them is in use).
WREN and every chain start fresh; identity, devices and the signer must NOT change (`contracts/signer.wasm` is frozen).

## 0. Gate (all on fx/fy first, CLAUDE.md rule 2)

- [ ] Statements agree across fx and fy, and are accepted (`token-ops accepted`).
- [ ] Keeper listing and claims made with the page closed (`token-ops keepers`, `token-ops chains`).
- [ ] A delegate signs a witness entry with its page closed (`token-ops witnessed`).
- [ ] The allowance pays one epoch: escrow → verdict → release → payout.
- [ ] `cargo test` for each crate touched since the last live publish; wasm built without `testlot` (no `BASELINE_SEED`
      in `contracts/*.wasm`).

## 1. Build

```sh
./build.sh                                   # contracts rebuilt, contracts/SHA256SUMS updated
git diff --stat contracts/SHA256SUMS         # expected: token, allowance, standing, bag (+ core/delegate wasm)
```

Stop if `signer.wasm`, `idlog.wasm` or `devices.wasm` moved: that changes identity (everyone re-enters a PIN) or
re-keys devices. Find the cause before going on.

## 2. Tokens on the live network (through B, under the lock)

Beacon: `packages/beacon-config.json` (the live beacon) stays. `$CRAFT` needs no step: its id is derived
(`wallet::craft::definition`).

```sh
# WREN: Wren's account (DID in ~/.config/craftworks/debug-account.txt), 1,000,000, through the B tunnel.
DID=<wren did> tools/token-ops/target/release/token-ops make-token contracts/token.wasm contracts/idlog.wasm \
  "$WS_B" <beacon hex> <words file> ~/.config/craftworks/wren-device.seed WREN 1000000
# Fund the allowance for the coming epochs (each epoch once; rerun to extend).
DID=<wren did> tools/token-ops/target/release/token-ops fund-allowance contracts/token.wasm contracts/idlog.wasm \
  "$WS_B" <beacon hex> <words file> ~/.config/craftworks/wren-device.seed <WREN id> 64
```

Write `packages/tokens-config.json`: `{ "pay": "<edition pay token>", "allowance": "<WREN id>" }`.

## 3. Publish

`./publish.sh` (builds, takes the lock, tunnels to B). Check the trace's "beacon extending <live beacon>" line: a
height-0 beacon means the wrong `beacon-config` went out.

## 4. After

- Wren's witness as a service on B (`token-ops witness … craft <key seed>`), restarted on every new binary: a stale
  witness computes different statement roots.
- Health: `tools/health-watch/check-live.sh`; reports are sealed to `packages/support-config.json`'s key, read with
  `HEALTH_KEY` (`~/.config/craftworks/health.key`).
- Re-run `fund-allowance` before the funded epochs run out.
- Record the release (version, manifest, token ids) in `.claude/feature-progress.md`.
