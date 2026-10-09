// THE DISK GUARD (the owner's control, scripts/disk-guard.sh): a gate or realnet run does not START with the
// data volume under the floor, and a refusal NAMES who holds the space -- job dirs, the workspace's target/
// and worktree dirs, the shared build caches -- largest first. Could-not-check is a failure, never a pass.
// The table is read from a FIXTURE (DISK_GUARD_JOBS / _WORKSPACE / _CACHES), so this never du's the real disk.
import assert from "node:assert/strict";
import { spawnSync, execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir, homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { childEnv } from "./common/child-env.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const guard = process.env.DISK_GUARD_UNDER_TEST ?? join(root, "scripts", "disk-guard.sh");
let failures = 0;
const t = (name, fn) => {
  try { fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.message}\n`); }
};

const fx = mkdtempSync(join(tmpdir(), "disk-guard-test-"));
const put = (dir, kb) => { mkdirSync(dir, { recursive: true }); writeFileSync(join(dir, "f"), Buffer.alloc(kb * 1024)); };
put(join(fx, "jobs", "aaaa1111", "tmp"), 64);
writeFileSync(join(fx, "jobs", "aaaa1111", "state.json"), JSON.stringify({ name: "small-session" }));
put(join(fx, "jobs", "bbbb2222", "tmp"), 1024);
writeFileSync(join(fx, "jobs", "bbbb2222", "state.json"), JSON.stringify({ name: "big-session" }));
put(join(fx, "ws", "some-repo", "target"), 3072);             // the largest: a repo's target/
put(join(fx, "ws", "other-repo", ".claude", "worktrees"), 512);
put(join(fx, "caches", "craftworks-sdk-x"), 2048);
const env = extra => childEnv({ DISK_GUARD_JOBS: join(fx, "jobs"), DISK_GUARD_WORKSPACE: join(fx, "ws"), DISK_GUARD_CACHES: join(fx, "caches"), ...extra });
const run = (args, extra) => spawnSync("/bin/bash", [guard, ...args], { encoding: "utf8", env: env(extra) });
const freeGb = Math.floor(Number(execFileSync("/bin/sh", ["-c", `df -Pk "${homedir()}" | awk 'NR == 2 { print $4 }'`], { encoding: "utf8" })) / 1048576);
const above = String(freeGb + 1000);

t(`**a floor ABOVE the free space (${above} GiB > ${freeGb} GiB) REFUSES: exit 1, and says what did not run**`, () => {
  const r = run(["the test's gate"], { DISK_GUARD_MIN_GB: above });
  assert.equal(r.status, 1, r.stdout + r.stderr);
  assert.match(r.stdout, /REFUSED/);
  assert.match(r.stdout, /the test's gate does not run/);
  assert.match(r.stdout, new RegExp(`${freeGb} GiB free`), "the refusal does not print the free space");
});

t("**the refusal NAMES who holds the space, LARGEST FIRST, across jobs, workspace targets, worktrees and caches**", () => {
  const out = run(["x"], { DISK_GUARD_MIN_GB: above }).stdout;
  const at = s => out.indexOf(s);
  const order = ["some-repo/target", "craftworks-sdk-x", "big-session", "other-repo/.claude/worktrees", "small-session"];
  for (const s of order) assert.ok(at(s) >= 0, `the table does not list ${s}:\n${out}`);
  for (let i = 1; i < order.length; i++) assert.ok(at(order[i - 1]) < at(order[i]), `${order[i - 1]} is not listed before ${order[i]}:\n${out}`);
  assert.match(out, /job bbbb2222 \(big-session\)/, "a job is not named by its session");
});

t("a floor BELOW the free space (0 GiB) passes: exit 0, the free space printed, and NO table (the du is for a refusal)", () => {
  const r = run(["the test's gate"], { DISK_GUARD_MIN_GB: "0" });
  assert.equal(r.status, 0, r.stdout + r.stderr);
  assert.match(r.stdout, /may run/);
  assert.doesNotMatch(r.stdout, /big-session|some-repo/);
});

t("**could not read the free space: a FAILURE (exit 2), never a pass**", () => {
  const r = run(["x"], { DISK_GUARD_PATH: "/no/such/volume", DISK_GUARD_MIN_GB: "0" });
  assert.equal(r.status, 2, r.stdout + r.stderr);
});

t("a floor that is not a number is refused (exit 2), not read as 0", () => {
  assert.equal(run(["x"], { DISK_GUARD_MIN_GB: "lots" }).status, 2);
});

t("--report prints the table and no verdict: exit 0", () => {
  const r = run(["--report"], { DISK_GUARD_MIN_GB: above });
  assert.equal(r.status, 0);
  assert.match(r.stdout, /some-repo\/target/);
});

rmSync(fx, { recursive: true, force: true });
console.log(`disk guard: ${failures ? `${failures} FAILED` : "all ok"}`);
process.exit(failures ? 1 : 0);
