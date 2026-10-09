// THE CONTRACTS THE SDK SHIPS ARE THE RELEASED ONES (tools/contracts-released.mjs, build.sh's one check). A
// scratch contracts checkout: a released.toml with two epochs and a build/ of four contracts. The script is run as
// build.sh runs it (a child process, its exit status and its words), so the test sees what a person would.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { childEnv } from "./common/child-env.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const tool = join(root, "tools", "contracts-released.mjs");
const NAMES = ["block", "register", "webapp", "site"];
let failures = 0;
const t = (name, fn) => {
  try { fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.message}\n`); }
};
const sha = b => createHash("sha256").update(b).digest("hex");
const bytes = (name, epoch) => Buffer.from(`${name} code of epoch ${epoch}`);

/** A contracts checkout: released.toml's epochs `upTo` (1..upTo; `site` from 2 on), and build/ holding `built`'s code. */
function checkout(built, { table = true, siteIn2 = true, upTo = 2 } = {}) {
  const d = mkdtempSync(join(tmpdir(), "contracts-released-"));
  mkdirSync(join(d, "build"));
  for (const [n, e] of Object.entries(built)) writeFileSync(join(d, "build", `${n}.wasm`), bytes(n, e));
  const row = (n, e) => `[epoch.contract.${n}]\nlock_sha256 = "sha256:${"0".repeat(64)}"\nsha256      = "sha256:${sha(bytes(n, e))}"\nbytes       = 1\n\n`;
  const epoch = (e, names) => `[[epoch]]\nnumber   = ${e}\nreleased = "2026-09-2${e}"\ntag      = "v0.${e}.0"\n\n[epoch.source]\ncommit = "abc"\n\n${names.map(n => row(n, e)).join("")}`;
  const rows = [epoch(1, ["block", "register", "webapp"]), epoch(2, siteIn2 ? NAMES : ["block", "register", "webapp"])];
  for (let e = 3; e <= upTo; e += 1) rows.push(epoch(e, NAMES));
  if (table) writeFileSync(join(d, "released.toml"), `# header\n\n${rows.slice(0, upTo).join("")}`);
  return d;
}
const run = (dir, env = {}, need = 2) => spawnSync(process.execPath, [tool, dir, String(need), ...NAMES], { encoding: "utf8", env: childEnv({ CRAFTWORKS_CONTRACTS_UNRELEASED: "", ...env }) });

t("**THE INCIDENT: a checkout BEHIND the release is REFUSED** -- its table ends at epoch 2, its build IS epoch 2's released code, and this SDK needs epoch 3 (the architect, sdk#388: \"the latest epoch of that table\" passed it). Mutant \"latest epoch\" -> red", () => {
  const d = checkout({ block: 2, register: 2, webapp: 2, site: 2 }, { upTo: 2 });
  const r = run(d, {}, 3);
  assert.equal(r.status, 1, `a checkout behind the release passed: ${r.stdout}`);
  assert.match(r.stderr, /has epochs 1, 2; this SDK needs epoch 3/, r.stderr);
  assert.match(r.stderr, /fix: build the contracts at a release that has epoch 3/);
  assert.match(r.stderr, /a PR building a NEW epoch keeps CONTRACTS_EPOCH at the released one and sets CRAFTWORKS_CONTRACTS_UNRELEASED=1/, "the refusal does not say how a new epoch's PR builds");
  rmSync(d, { recursive: true, force: true });
});

t("**needing epoch 3 means 3, not the latest**: a table with epochs 1-4 and a build of 3 passes", () => {
  const d = checkout({ block: 3, register: 3, webapp: 3, site: 3 }, { upTo: 4 });
  const r = run(d, {}, 3);
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /are epoch 3's released code \(v0\.3\.0\)/);
  // THE CONTROL: the same checkout's epoch-4 build is not what this SDK needs.
  const e = checkout({ block: 4, register: 4, webapp: 4, site: 4 }, { upTo: 4 });
  assert.equal(run(e, {}, 3).status, 1, "a later epoch's code passed for an SDK that needs epoch 3");
  for (const x of [d, e]) rmSync(x, { recursive: true, force: true });
});

t("**the needed epoch's released code passes**", () => {
  const d = checkout({ block: 2, register: 2, webapp: 2, site: 2 });
  const r = run(d);
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /are epoch 2's released code \(v0\.2\.0\)/);
  rmSync(d, { recursive: true, force: true });
});

t("**a STALE block (an earlier epoch's) is REFUSED by name**: the contract, expected and actual sha, the checkout, and the fix", () => {
  const d = checkout({ block: 1, register: 2, webapp: 2, site: 2 });
  const r = run(d);
  assert.equal(r.status, 1, "a stale block was not refused");
  assert.match(r.stderr, /REFUSED/);
  assert.match(r.stderr, new RegExp(`block\\.wasm: expected ${sha(bytes("block", 2))}, actual ${sha(bytes("block", 1))}`), r.stderr);
  assert.match(r.stderr, /not a git checkout|the checkout at/, "the checkout's HEAD is not named");
  assert.match(r.stderr, /fix: rebuild the contracts at the released tag.*v0\.2\.0/, "the fix is not named");
  assert.doesNotMatch(r.stderr, /register\.wasm/, "a contract that IS released was named as wrong");
  rmSync(d, { recursive: true, force: true });
});

t("**a contract with no row in the needed epoch is refused** (unreleased code), and so is a missing build", () => {
  const d = checkout({ block: 2, register: 2, webapp: 2, site: 2 }, { siteIn2: false });
  const r = run(d);
  assert.equal(r.status, 1);
  assert.match(r.stderr, /site\.wasm: expected \(no row in epoch 2\)/);
  const e = checkout({ block: 2, register: 2, webapp: 2 });
  const r2 = run(e);
  assert.equal(r2.status, 1);
  assert.match(r2.stderr, /site\.wasm: expected [0-9a-f]{64}, actual \(no build\/site\.wasm\)/);
  for (const x of [d, e]) rmSync(x, { recursive: true, force: true });
});

t("**no released.toml: COULD NOT CHECK is a refusal**, never a skip", () => {
  const d = checkout({ block: 2, register: 2, webapp: 2, site: 2 }, { table: false });
  const r = run(d);
  assert.equal(r.status, 1, "a checkout with no released.toml passed");
  assert.match(r.stderr, /released\.toml does not exist/);
  rmSync(d, { recursive: true, force: true });
});

t("**the override** (a PR that builds a NEW epoch) passes, and says LOUDLY what is not released", () => {
  const d = checkout({ block: 1, register: 2, webapp: 2, site: 2 });
  const r = run(d, { CRAFTWORKS_CONTRACTS_UNRELEASED: "1" });
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /OVERRIDE \(CRAFTWORKS_CONTRACTS_UNRELEASED=1\).*NOT epoch 2's released code/);
  assert.match(r.stdout, /block\.wasm: expected/);
  rmSync(d, { recursive: true, force: true });
});

t("**build.sh states CONTRACTS_EPOCH exactly once, passes it to the check, and runs the check BEFORE it copies any contract** (the one home)", () => {
  const src = readFileSync(join(root, "build.sh"), "utf8");
  const sets = [...src.matchAll(/^CONTRACTS_EPOCH=(\d+)$/gm)];
  assert.equal(sets.length, 1, `build.sh sets CONTRACTS_EPOCH ${sets.length} times`);
  assert.equal(src.match(/CONTRACTS_EPOCH=/g).length, 1, "CONTRACTS_EPOCH is assigned somewhere else too");
  const checkAt = src.indexOf(`node tools/contracts-released.mjs "$contracts" "$CONTRACTS_EPOCH" block register webapp site || exit 1`);
  assert.ok(checkAt > 0, "build.sh does not run tools/contracts-released.mjs with CONTRACTS_EPOCH over the contracts it copies");
  const firstCopy = src.search(/\bcp "\$contracts\/build\//);
  assert.ok(firstCopy > 0, "THE CONTROL: build.sh copies no contract (the source read found nothing)");
  assert.ok(checkAt < firstCopy, "build.sh copies a contract before checking it");
  for (const n of NAMES) assert.match(src, new RegExp(`cp [^\\n]*"\\$contracts/build/${n}\\.wasm"`), `build.sh does not copy ${n}.wasm (the checked list and the copied list differ)`);
});

process.stdout.write(failures ? `${failures} FAILED\n` : "contracts released: all ok\n");
process.exit(failures ? 1 : 0);
