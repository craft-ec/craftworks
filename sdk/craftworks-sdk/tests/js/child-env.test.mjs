// NO TEST SPREADS THE OUTER ENVIRONMENT INTO A CHILD (sdk#469, the architect): the class, not the instance. Twice an
// outer run's GATE_* reached a child the suite started (#452's GATE_OWNERS_PER_PR, #469's GATE_PR_BASE) through a
// literal that spreads the process environment. So every test file builds a child's environment through the ONE helper,
// tests/js/common/child-env.mjs, and this scan holds it: no file under tests/js spreads `process.env` itself, except
// the helper and a line marked `// LEAK CONTROL` (a control that proves a leak is real). Read at test time, so a new
// file is covered without being listed.
import assert from "node:assert/strict";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.message}\n`); }
};

const dir = new URL(".", import.meta.url).pathname;
const HELPER = "common/child-env.mjs";

/** Every JS file under tests/js, relative to it. */
const files = (at = dir, rel = "") =>
  readdirSync(at).flatMap(n => {
    const p = join(at, n);
    if (statSync(p).isDirectory()) return files(p, `${rel}${n}/`);
    return /\.(mjs|cjs|js)$/.test(n) ? [`${rel}${n}`] : [];
  });

/** `(file:line)` for each line that spreads process.env outside the helper and a marked control. */
const breaches = (file, src) =>
  file === HELPER
    ? []
    : src.split("\n").flatMap((l, i) => {
        // Code only: a comment naming the form spreads nothing.
        const code = l.trimStart().startsWith("//") ? "" : l;
        return /\.\.\.\s*process\.env\b/.test(code) && !l.includes("// LEAK CONTROL") ? [`${file}:${i + 1}: ${l.trim()}`] : [];
      });

await t("**no test spreads process.env into a child: every one goes through common/child-env.mjs**", async () => {
  const all = files();
  assert.ok(all.length > 40 && all.includes(HELPER), `THE CONTROL: the scan read ${all.length} files -- not where the tests are`);
  const bad = all.flatMap(f => breaches(f, readFileSync(join(dir, f), "utf8")));
  assert.deepEqual(bad, [], `a test spreads the outer environment into a child:\n${bad.join("\n")}`);
  const uses = all.filter(f => readFileSync(join(dir, f), "utf8").includes('from "./common/child-env.mjs"')).length;
  assert.ok(uses >= 10, `THE CONTROL: only ${uses} files use the helper`);
});

// The forbidden form, built so it never appears literally in this file.
const SPREAD = "..." + "process.env";

await t("THE CONTROLS: an unmarked spread is flagged; a marked control, the helper, a comment and a plain read are not", async () => {
  assert.equal(breaches("x.test.mjs", `spawnSync(g, [], { env: { ${SPREAD}, A: 1 } });`).length, 1, "an unmarked spread passed");
  assert.equal(breaches("x.test.mjs", `const e = { ${SPREAD.replace("...", "... ")} };`).length, 1, "a spaced spread passed");
  assert.equal(breaches("x.test.mjs", `spawnSync(g, [], { env: { ${SPREAD} } }); // LEAK CONTROL`).length, 0, "the marked control was flagged");
  assert.equal(breaches(HELPER, `export const childEnv = () => ({ ${SPREAD} });`).length, 0, "the helper was flagged");
  assert.equal(breaches("x.test.mjs", `// never write { ${SPREAD} } here`).length, 0, "a comment was flagged");
  assert.equal(breaches("x.test.mjs", "const c = process.env.CRAFTWORKS_CONTRACTS;").length, 0, "a plain read was flagged");
});

await t("**the helper drops the outer run's GATE_* and DISK_GUARD* and keeps the rest; a test's own values win**", async () => {
  const { childEnv } = await import("./common/child-env.mjs");
  const saved = Object.fromEntries(Object.entries(process.env));
  try {
    Object.assign(process.env, { GATE_PR_BASE: "FETCH_HEAD", DISK_GUARD: "/refuse", DISK_GUARD_JOBS: "/x", CARGO_TARGET_DIR: "/t" });
    const env = childEnv({ DISK_GUARD: "/admit" });
    assert.equal(env.GATE_PR_BASE, undefined, "the outer GATE_PR_BASE reached the child");
    assert.equal(env.DISK_GUARD_JOBS, undefined, "the outer DISK_GUARD_JOBS reached the child");
    assert.equal(env.DISK_GUARD, "/admit", "the test's own DISK_GUARD did not win");
    assert.equal(env.CARGO_TARGET_DIR, "/t", "THE CONTROL: a variable the helper must keep was dropped");
  } finally {
    for (const k of Object.keys(process.env)) if (!(k in saved)) delete process.env[k];
    Object.assign(process.env, saved);
  }
});

if (failures) { process.stdout.write(`child-env: ${failures} FAILED\n`); process.exit(1); }
process.stdout.write("child-env: all ok\n");
