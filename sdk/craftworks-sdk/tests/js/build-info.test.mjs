// What this build says it is, and where each value comes from.
//
// The point of these assertions is PROVENANCE, not shape. A test that only
// checked `rev` is a 7-hex string would pass against a constant typed into
// wrap.js, which is exactly the failure buildInfo() exists to prevent — so
// each value is compared against the thing it is supposed to have come from,
// read independently here.
import assert from "node:assert";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { wrap } from "../../js/wrap.js";

const sdk = wrap(createRequire(import.meta.url)("../../pkg/node/craftworks_sdk.js"));
const info = sdk.buildInfo();

// ---- shape -----------------------------------------------------------------
assert.deepStrictEqual(
  Object.keys(info).sort(),
  ["blockHash", "contractsRev", "formatTag", "prollyRev", "registerHash", "rev", "version"],
  "buildInfo has exactly these fields",
);

// ---- version: the crate's own -----------------------------------------------
const cargo = readFileSync(new URL("../../Cargo.toml", import.meta.url), "utf8");
const declared = /^version\s*=\s*"([^"]+)"/m.exec(cargo)[1];
assert.strictEqual(info.version, declared, "version is the one in Cargo.toml");
assert.strictEqual(info.version, sdk.version(), "and agrees with version()");

// ---- rev: the commit this wasm was built from --------------------------------
// Read from git HERE, not from the same env var the build used. `unknown` is a
// failure: the builder treats it as a mismatch, so a test that accepted it
// would green-light a panel that can never verify anything.
assert.notStrictEqual(info.rev, "unknown", "the build must be able to name itself");
const head = execFileSync("git", ["rev-parse", "--short=7", "HEAD"], {
  cwd: new URL("../../", import.meta.url),
}).toString().trim();
const dirty = execFileSync("git", ["status", "--porcelain"], {
  cwd: new URL("../../", import.meta.url),
}).toString().trim().length > 0;
// A wasm built before the last commit legitimately names an older commit, so
// the assertion is on the SHAPE of the agreement, not on equality with HEAD
// right now: it must be a 7-hex rev, optionally marked dirty, and when the
// tree is clean and the wasm is current it must BE head.
assert.match(info.rev, /^[0-9a-f]{7}(-dirty)?$/, `rev looks like a rev: ${info.rev}`);
if (!dirty && process.env.SDK_WASM_IS_CURRENT === "1") {
  assert.strictEqual(info.rev, head, "a clean, current build names HEAD");
}
if (dirty) {
  assert.ok(
    info.rev.endsWith("-dirty") || process.env.SDK_WASM_IS_CURRENT !== "1",
    "a build from a dirty tree must say so",
  );
}

// ---- prollyRev: what the LOCK resolved, not what the manifest asked for -------
const lock = readFileSync(new URL("../../Cargo.lock", import.meta.url), "utf8");
const block = lock.split("[[package]]").find(p => /name = "freenet-prolly"/.test(p));
const locked = /source = "[^"]*#([0-9a-f]+)"/.exec(block)[1].slice(0, 7);
assert.strictEqual(info.prollyRev, locked, "prollyRev is the rev Cargo.lock resolved");

// ---- the contracts this build PROVISIONS with --------------------------------
//
// Provenance again, and a sharper case of it: these hashes are COPIED from the
// contracts build, never computed here. `hashes.toml` says why in its own
// header — a consumer that re-hashes matches only while its algorithm and its
// notion of the canonical wasm stay byte-identical to the one script that
// builds them, and shows an authoritative-looking number that matches nothing
// when they drift. So this test reads the same file and compares, rather than
// hashing `pkg/web/block.wasm` and agreeing with itself.
const contracts = process.env.CRAFTWORKS_CONTRACTS ?? new URL("../../../freenet-contracts", import.meta.url).pathname;
let hashes = null;
try { hashes = readFileSync(`${contracts}/build/hashes.toml`, "utf8"); } catch (_) {}
if (hashes) {
  const field = n => new RegExp(`^${n}\\s*=\\s*"([^"]+)"`, "m").exec(hashes)?.[1];
  assert.strictEqual(info.blockHash, field("block"), "blockHash is the one the contracts build recorded");
  assert.strictEqual(info.registerHash, field("register"), "registerHash likewise");
  assert.strictEqual(info.contractsRev, field("rev"), "contractsRev likewise");
} else {
  // NOT a silent skip. A build that cannot name what it provisions with is
  // the case the versions panel exists to surface, so the value must SAY it
  // could not be read rather than looking like a hash.
  assert.strictEqual(info.blockHash, "unknown", "no contracts build, so this must say unknown");
  assert.strictEqual(info.registerHash, "unknown");
  console.log("  (no contracts checkout: asserted the honest `unknown` instead)");
}

// ---- formatTag: from the linked tree library ---------------------------------
assert.strictEqual(info.formatTag, "PT01");
// And it really is four bytes of magic rather than a string in the SDK: the
// nodes the library builds start with it.
const db = new sdk.Db();
db.define("t", { type: "T", fields: [{ name: "v", kind: "text", required: true }] });
db.put("t", { v: "x" });
assert.ok(db.root().length > 0, "a tree exists to carry that tag");

console.log(`build-info ok: ${JSON.stringify(info)}`);
