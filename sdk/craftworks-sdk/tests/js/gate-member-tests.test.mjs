// A FAILING MEMBER'S OUTPUT IS KEPT (sdk#360, the architect's tripwire): tools/gate-member-tests.sh runs one
// member's tests for gate.sh and, on a failure, keeps the WHOLE output -- the panic text a one-off failure
// needs -- in a file it names. A stub `cargo` on PATH stands in for the suite, so nothing is built.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { childEnv } from "./common/child-env.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const helper = process.env.GATE_MEMBER_TESTS_UNDER_TEST ?? join(root, "tools", "gate-member-tests.sh");
let failures = 0;
const t = (name, fn) => {
  try { fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.message}\n`); }
};

const d = mkdtempSync(join(tmpdir(), "gate-member-"));
const stub = (body, rc) => {
  const bin = mkdtempSync(join(d, "bin-"));
  writeFileSync(join(bin, "cargo"), `#!/bin/sh\ncat <<'OUT'\n${body}\nOUT\nexit ${rc}\n`);
  chmodSync(join(bin, "cargo"), 0o755);
  return bin;
};
const run = (bin, dir) => spawnSync("/bin/bash", [helper, "page", dir], { encoding: "utf8", env: childEnv({ PATH: `${bin}:/usr/bin:/bin` }) });

t("**a failing member: its WHOLE output is kept in a file the gate names, the count and cargo's status are kept**", () => {
  const marker = "MARKER-a3f9 left == right failed: the panic text a rerun would lose";
  const bin = stub(`running 2 tests\ntest a ... ok\ntest b ... FAILED\n\n---- b stdout ----\nthread 'b' panicked at page/tests/x.rs:1:1:\n${marker}\n\ntest result: FAILED. 1 passed; 1 failed; 0 ignored`, 101);
  const kept = join(d, "kept");
  const r = run(bin, kept);
  assert.equal(r.status, 101, "cargo's failure was not the helper's exit status");
  assert.equal(r.stdout.trim(), "1", "the passed count is not what the gate reads");
  const files = readdirSync(kept);
  assert.equal(files.length, 1, `not one kept log: ${files}`);
  assert.match(files[0], /^page-\d{8}T\d{6}Z\.log$/, "the kept log is not named by member and time");
  assert.ok(readFileSync(join(kept, files[0]), "utf8").includes(marker), "the panic text was not kept");
  assert.match(r.stderr, new RegExp(`kept at .*${files[0]}`), "the gate does not say where the output is kept");
});

t("THE CONTROL: a passing member keeps nothing, and reports its count", () => {
  const bin = stub("running 3 tests\ntest result: ok. 3 passed; 0 failed; 0 ignored", 0);
  const kept = join(d, "kept-ok");
  const r = run(bin, kept);
  assert.equal(r.status, 0);
  assert.equal(r.stdout.trim(), "3");
  let files = [];
  try { files = readdirSync(kept); } catch { /* not made: nothing kept */ }
  assert.deepEqual(files, [], "a passing member's output was kept");
});

t("**--keep: a member gate.sh ran its own way (its model targets in --profile model) is kept by the SAME rule, from stdin**", () => {
  const marker = "MARKER-7c1e the release-run model test's panic";
  const kept = join(d, "kept-stdin");
  const r = spawnSync("/bin/bash", [helper, "--keep", "page", kept, "101"], { encoding: "utf8", input: `test result: FAILED. 4 passed; 1 failed\n---- m stdout ----\n${marker}\n` });
  assert.equal(r.status, 101, "the member's status was not passed through");
  const files = readdirSync(kept);
  assert.equal(files.length, 1, `not one kept log: ${files}`);
  assert.ok(readFileSync(join(kept, files[0]), "utf8").includes(marker), "the panic text was not kept");
  // THE CONTROL: a passing run keeps nothing.
  const ok = join(d, "kept-stdin-ok");
  const p = spawnSync("/bin/bash", [helper, "--keep", "page", ok, "0"], { encoding: "utf8", input: "test result: ok. 5 passed\n" });
  assert.equal(p.status, 0);
  let none = [];
  try { none = readdirSync(ok); } catch { /* not made */ }
  assert.deepEqual(none, [], "a passing member's output was kept");
});

rmSync(d, { recursive: true, force: true });
console.log(`gate member tests: ${failures ? `${failures} FAILED` : "all ok"}`);
process.exit(failures ? 1 : 0);
