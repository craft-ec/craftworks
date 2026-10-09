// `npm test` RUNS EVERY TEST FILE THAT EXISTS (tools/run-tests.mjs): found by walking the directory at any depth, run
// whatever failed before, the verdict at the end. Each claim with its control, over a scratch directory.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { testFiles } from "../../tools/run-tests.mjs";

const runner = fileURLToPath(new URL("../../tools/run-tests.mjs", import.meta.url));
let failures = 0;
const t = (name, fn) => {
  try { fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.message}\n`); }
};
const scratch = (files) => {
  const d = mkdtempSync(join(tmpdir(), "run-tests-"));
  for (const [f, body] of Object.entries(files)) { mkdirSync(join(d, f, ".."), { recursive: true }); writeFileSync(join(d, f), body); }
  return d;
};
const run = (d) => spawnSync(process.execPath, [runner, d], { encoding: "utf8" });

t("**a test file at ANY depth is found** (a subdirectory's too); control: vectors and helpers are not", () => {
  const d = scratch({ "a.test.mjs": "", "b.test.cjs": "", "sub/deep/c.test.mjs": "", "vectors.json": "", "helper.mjs": "" });
  try {
    assert.deepEqual(testFiles(d).map(f => f.slice(d.length + 1)), ["a.test.mjs", "b.test.cjs", "sub/deep/c.test.mjs"]);
  } finally { rmSync(d, { recursive: true }); }
});

t("**a red file does not hide the files after it**: all run, exit 1, each named; control: all green exits 0", () => {
  const d = scratch({ "1.test.mjs": "process.exit(1)", "2.test.mjs": "console.log('second ran')", "z/3.test.mjs": "console.log('third ran')" });
  const ok = scratch({ "1.test.mjs": "", "2.test.mjs": "" });
  try {
    const r = run(d);
    assert.equal(r.status, 1);
    assert.ok(r.stdout.includes("second ran") && r.stdout.includes("third ran"), r.stdout);
    assert.match(r.stdout, /3 test files: 2 passed, 1 failed/);
    assert.match(r.stdout, /^FAILED .*1\.test\.mjs/m, "a red file's line must match the review grep FAILED|panicked|REFUSED");
    assert.equal(run(ok).status, 0, "THE CONTROL");
  } finally { rmSync(d, { recursive: true }); rmSync(ok, { recursive: true }); }
});

t("**no test files is a refusal**, never a pass; the tally never prints gate.sh's `  ok ` count marker", () => {
  const d = scratch({ "readme.md": "" });
  try {
    const r = run(d);
    assert.equal(r.status, 1);
    assert.match(r.stderr, /no test files/);
  } finally { rmSync(d, { recursive: true }); }
  const one = scratch({ "x.test.mjs": "" });
  try { assert.doesNotMatch(run(one).stdout, /^ {2}ok /m, "the tally would inflate gate.sh's npm count"); }
  finally { rmSync(one, { recursive: true }); }
});

if (failures) { process.stdout.write(`run-tests: ${failures} FAILED\n`); process.exit(1); }
