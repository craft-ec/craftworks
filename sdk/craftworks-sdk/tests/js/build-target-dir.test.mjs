// WHERE THE BUILD OUTPUT IS has ONE home: cargo's `target_directory` (tools/target-dir.sh), so CARGO_TARGET_DIR is
// honoured. A literal `target/` in a build script (or `env -u CARGO_TARGET_DIR`) made every checkout grow its own
// multi-GB build cache beside the one its session named. This scans every shell script the build runs.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { childEnv } from "./common/child-env.mjs";

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.message}\n`); }
};
const root = new URL("../../", import.meta.url).pathname;
const SKIP = new Set(["node_modules", "target", "pkg", ".git"]);

/** Lines of a shell script that name the build output by a path of their own, or unset the caller's. */
function ownTargetPaths(text) {
  return text.split("\n").filter(l => !l.trim().startsWith("#") && (/(^|[^$\w{])target\/(release|debug|wasm32)/.test(l) || /env -u CARGO_TARGET_DIR/.test(l)));
}

function scripts(dir, out = []) {
  for (const n of readdirSync(dir)) {
    const p = join(dir, n);
    if (statSync(p).isDirectory()) { if (!SKIP.has(n) && !n.startsWith(".")) scripts(p, out); }
    else if (n.endsWith(".sh")) out.push(p);
  }
  return out;
}

await t("**no build script names the build output itself: every one asks cargo (tools/target-dir.sh)**", async () => {
  const found = scripts(root);
  assert.ok(found.some(f => f.endsWith("/build.sh")) && found.some(f => f.endsWith("signer/build.sh")), `the scan read nothing: ${found}`);
  const bad = found.flatMap(f => ownTargetPaths(readFileSync(f, "utf8")).map(l => `${relative(root, f)}: ${l.trim()}`));
  assert.deepEqual(bad, [], `build output named by a path of its own:\n  ${bad.join("\n  ")}`);
});

await t("THE CONTROL: a hard-coded target/ path, and an unset CARGO_TARGET_DIR, are each found", async () => {
  assert.equal(ownTargetPaths("wasm=target/wasm32-unknown-unknown/release/web.wasm").length, 1);
  assert.equal(ownTargetPaths('cp "target/release/x" y').length, 1);
  assert.equal(ownTargetPaths("env -u CARGO_TARGET_DIR cargo build").length, 1);
  assert.equal(ownTargetPaths('wasm=$target/wasm32-unknown-unknown/release/web.wasm').length, 0);
  assert.equal(ownTargetPaths('"${target}/release/x"').length, 0);
  assert.equal(ownTargetPaths("# a comment naming target/release").length, 0);
});

await t("**tools/target-dir.sh answers CARGO_TARGET_DIR when it is set, and the workspace's own target when not**", async () => {
  const set = execFileSync(join(root, "tools/target-dir.sh"), { encoding: "utf8", env: childEnv({ CARGO_TARGET_DIR: "/tmp/somewhere-else" }) }).trim();
  assert.equal(set, "/tmp/somewhere-else");
  const env = childEnv();
  delete env.CARGO_TARGET_DIR;
  const unset = execFileSync(join(root, "tools/target-dir.sh"), { encoding: "utf8", env }).trim();
  assert.equal(unset, join(root, "target").replace(/\/$/, ""));
});

if (failures) { process.stdout.write(`build-target-dir: ${failures} FAILED\n`); process.exit(1); }
process.stdout.write("build-target-dir: all ok\n");
