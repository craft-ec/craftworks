// `./gate.sh --pr`, the one command every PR runs: every structural control whatever the PR touches; the changed
// members PLUS their reverse dependents (tools/pr-scope.mjs, from cargo metadata); a count line per member, a DROP
// failing; and a target shared with another worktree refused.
import assert from "node:assert/strict";
import { spawnSync, execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { count } from "../../tools/pr-scope.mjs";
import { childEnv } from "./common/child-env.mjs";

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.message}\n`); }
};
const root = new URL("../../", import.meta.url).pathname;
// The outer run's GATE_* never reaches a child gate (sdk#469): tests/js/common/child-env.mjs.
const gate = (cwd, args, env = {}) => spawnSync("./gate.sh", args, { cwd, encoding: "utf8", env: childEnv(env) });
// THE PLAN'S INPUT IS `GATE_PR_CHANGED` ALONE (sdk#461): a `--pr --dry-run` plan must not depend on the machine it runs
// on. The one other thing a dry run reads is the disk guard, and a gate run near the floor (engineer3's, 26 GiB
// against 25 before build.sh wrote more) had this suite's child gate refuse the disk and fail "ENGINE only", and npm's
// chain lost 42 tests behind it. So the plan tests pass a guard that always admits: the disk decides whether a gate
// may BUILD, never what a plan says. (The guard's own behaviour is disk-guard.test.mjs's.)
const ADMIT_DIR = mkdtempSync(join(tmpdir(), "gate-pr-admit-"));
const ADMIT = join(ADMIT_DIR, "admit.sh");
writeFileSync(ADMIT, "#!/bin/sh\nexit 0\n", { mode: 0o755 });
const plan = (cwd, changed, env = {}) => gate(cwd, ["--pr", "--dry-run"], { DISK_GUARD: ADMIT, GATE_PR_CHANGED: changed, ...env });
const planLine = (out, what) => (out.split("\n").find(l => l.startsWith(`gate --pr: ${what}:`)) ?? "").split(": ").slice(2).join(": ").trim();

await t("**a change to ENGINE tests ENGINE only** (the owner: dependents run at the batch gate), and npm, since engine is in the wasm package (sdk#537)", async () => {
  const r = plan(root, "engine/src/lib.rs");
  assert.equal(r.status, 0, r.stderr);
  assert.equal(planLine(r.stdout, "members tested (changed only; dependents run at the batch gate)"), "engine");
  assert.equal(planLine(r.stdout, "npm"), "true", "engine compiles into the package the JS tests load, yet npm was skipped");
  const two = plan(root, "page-io/src/lib.rs page/src/lib.rs");
  assert.equal(planLine(two.stdout, "members tested (changed only; dependents run at the batch gate)"), "page page-io",
    "page-io/x was taken for page's, or a member was missed");
});

await t("**THE PLAN IS A PURE FUNCTION OF GATE_PR_CHANGED** (sdk#461): a tree with a stray untracked file and modified files plans exactly what a clean one does; the disk is not its input either", async () => {
  // A killed earlier run leaves its scratch worktree registered: drop the stale entries first.
  execFileSync("git", ["-C", root, "worktree", "prune"], { stdio: "ignore" });
  const scratch = mkdtempSync(join(tmpdir(), "gate-pr-dirty-"));
  const wt = join(scratch, "wt");
  execFileSync("git", ["-C", root, "worktree", "add", "--detach", wt, "HEAD"], { stdio: "ignore" });
  try {
    // The gate AS IT IS in this tree (a PR runs the gate on its own branch, committed or not).
    for (const f of ["gate.sh", "tools/pr-scope.mjs"]) writeFileSync(join(wt, f), readFileSync(join(root, f)));
    // probe: a member OUTSIDE the wasm package, so npm is false unless the dirt leaks in (sdk#537 made engine ship).
    const clean = plan(wt, "probe/src/lib.rs");
    assert.equal(clean.status, 0, `THE CONTROL: the clean tree failed:\n${clean.stderr}`);
    // Dirt a git-status reader WOULD see: an untracked file in page/, a modified file in JS (npm), in page-io.
    writeFileSync(join(wt, "page/src/stray.rs"), "// not committed\n");
    writeFileSync(join(wt, "js/served.js"), `${readFileSync(join(wt, "js/served.js"), "utf8")}\n// modified\n`);
    writeFileSync(join(wt, "page-io/src/lib.rs"), `${readFileSync(join(wt, "page-io/src/lib.rs"), "utf8")}\n// modified\n`);
    const st = execFileSync("git", ["-C", wt, "status", "--porcelain"], { encoding: "utf8" });
    assert.ok(st.includes("page/src/stray.rs") && st.includes("js/served.js"), `THE CONTROL: the tree is not dirty:\n${st}`);
    const dirty = plan(wt, "probe/src/lib.rs");
    assert.equal(dirty.status, 0, `the dirty tree's plan failed:\n${dirty.stderr}`);
    for (const what of ["changed members", "members tested (changed only; dependents run at the batch gate)", "npm"]) {
      assert.equal(planLine(dirty.stdout, what), planLine(clean.stdout, what), `\`${what}\` read the working tree`);
    }
    assert.equal(planLine(dirty.stdout, "npm"), "false", "a modified JS file outside GATE_PR_CHANGED ran npm");
    // The disk: a guard that REFUSES stops an unstubbed dry run (so the guard is really consulted, and ADMIT is what
    // keeps the plan tests off the disk), and never changes a plan that runs.
    const refuse = join(scratch, "refuse.sh");
    writeFileSync(refuse, "#!/bin/sh\necho 'disk-guard: under the floor' >&2\nexit 1\n", { mode: 0o755 });
    const atFloor = gate(wt, ["--pr", "--dry-run"], { DISK_GUARD: refuse, GATE_PR_CHANGED: "probe/src/lib.rs" });
    assert.notEqual(atFloor.status, 0, "THE CONTROL: a refusing guard did not stop the gate: ADMIT isolates nothing");
  } finally {
    execFileSync("git", ["-C", root, "worktree", "remove", "--force", wt], { stdio: "ignore" });
    rmSync(scratch, { recursive: true, force: true });
  }
});

await t("**the OUTER run's GATE_* never reaches a child gate** (sdk#469): a stray GATE_PR_BASE=FETCH_HEAD leaves a scratch worktree's plan as it is", async () => {
  execFileSync("git", ["-C", root, "worktree", "prune"], { stdio: "ignore" });
  const scratch = mkdtempSync(join(tmpdir(), "gate-pr-env-"));
  const wt = join(scratch, "wt");
  execFileSync("git", ["-C", root, "worktree", "add", "--detach", wt, "HEAD"], { stdio: "ignore" });
  const saved = process.env.GATE_PR_BASE;
  try {
    for (const f of ["gate.sh", "tools/pr-scope.mjs"]) writeFileSync(join(wt, f), readFileSync(join(root, f)));
    process.env.GATE_PR_BASE = "FETCH_HEAD";
    // THE CONTROL: the stray value, passed through, breaks the child (the leak is real, not assumed).
    const leaked = spawnSync("./gate.sh", ["--pr", "--dry-run"], { cwd: wt, encoding: "utf8", env: { ...process.env, DISK_GUARD: ADMIT, GATE_PR_CHANGED: "engine/src/lib.rs" } }); // LEAK CONTROL
    assert.notEqual(leaked.status, 0, "THE CONTROL: a stray GATE_PR_BASE=FETCH_HEAD did not break a scratch worktree's gate");
    const r = plan(wt, "engine/src/lib.rs");
    assert.equal(r.status, 0, `the outer run's GATE_PR_BASE reached the child:\n${r.stderr}`);
    assert.equal(planLine(r.stdout, "members tested (changed only; dependents run at the batch gate)"), "engine");
  } finally {
    if (saved === undefined) delete process.env.GATE_PR_BASE;
    else process.env.GATE_PR_BASE = saved;
    execFileSync("git", ["-C", root, "worktree", "remove", "--force", wt], { stdio: "ignore" });
    rmSync(scratch, { recursive: true, force: true });
  }
});

await t("**the MODEL targets are DERIVED (never listed): both gates run them in --profile model, --pr at a share of the seeds, the batch at the full count**", async () => {
  const { testArgs, modelTargets } = await import("../../tools/pr-scope.mjs");
  const meta = JSON.parse(execFileSync("cargo", ["metadata", "--format-version", "1", "--no-deps"], { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }));
  // Every seeded model main has, found by its name, with nobody writing a list.
  assert.deepEqual(modelTargets(meta, "page"), ["model"]);
  assert.deepEqual(modelTargets(meta, "testkit"), ["read_state_model"]);
  assert.deepEqual(modelTargets(meta, "craftworks-sdk"), ["write_path_model"]);
  assert.deepEqual(modelTargets(meta, "signer"), ["provision_model"]);
  assert.deepEqual(modelTargets(meta, "engine"), [], "engine has no model target, yet one was found");
  const page = testArgs(meta, "page", modelTargets(meta, "page"));
  assert.ok(!page.args.includes("model"), `model ran in DEBUG beside --profile model: ${page.args}`);
  assert.ok(page.args.includes("--lib") && page.args.includes("race_get"), `page's other tests were dropped: ${page.args}`);
  assert.equal(page.doc, true, "page's doc-tests would not be counted");
  // The root crate's library is an `rlib`: its unit tests and doc-tests run beside its model target too.
  const sdk = testArgs(meta, "craftworks-sdk", modelTargets(meta, "craftworks-sdk"));
  assert.ok(sdk.args.includes("--lib") && !sdk.args.includes("write_path_model"), `craftworks-sdk's lib tests were dropped: ${sdk.args}`);
  assert.equal(sdk.doc, true, "craftworks-sdk's doc-tests would not be counted");
  const pr = plan(root, "page/src/lib.rs");
  assert.equal(planLine(pr.stdout, "model seeds"), "4 of 40 (GATE_PR_MODEL_SEEDS), --profile model", "--pr does not state its model seeds");
  const batch = gate(root, ["--dry-run"], { CRAFTWORKS_MODEL_SEEDS: "" });
  assert.match(batch.stdout, /gate: model seeds 40 \(CRAFTWORKS_MODEL_SEEDS\)/, "the batch gate does not state the full seed count");
  assert.match(batch.stdout, /gate: model targets, --profile model, derived per member/);
});

await t("**a SEEDED model not named a model FAILS the model_targets control (it would run in DEBUG on every PR)**", async () => {
  const { misnamedModels } = await import("../../tools/pr-scope.mjs");
  const meta = { packages: [{ name: "page", targets: [
    { kind: ["test"], name: "model", src_path: "page/tests/model.rs" },
    { kind: ["test"], name: "race_get", src_path: "page/tests/race_get.rs" },
    { kind: ["test"], name: "sweep", src_path: "page/tests/sweep.rs" },
    { kind: ["lib"], name: "page", src_path: "page/src/lib.rs" },
  ] }] };
  const src = { "page/tests/model.rs": "use testkit::model;", "page/tests/race_get.rs": "fn plain() {}", "page/tests/sweep.rs": "", "page/src/lib.rs": "CRAFTWORKS_MODEL_SEEDS" };
  assert.deepEqual(misnamedModels(meta, f => src[f]), [], "a clean tree was flagged");
  // PLANTED, three ways a model hides under another name: the runner, a share, the knob read by hand.
  for (const planted of ["let r = testkit::model::run(0..9, f);", "let n = model::share(200);", "std::env::var(\"CRAFTWORKS_MODEL_SEEDS\")"]) {
    const bad = misnamedModels(meta, f => (f === "page/tests/sweep.rs" ? planted : src[f]));
    assert.equal(bad.length, 1, `not caught: ${planted}`);
    assert.match(bad[0], /page: test target `sweep`/);
  }
  const r = spawnSync("node", ["tools/pr-scope.mjs", "model-check"], { cwd: root, encoding: "utf8" });
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /4 model target\(s\)/, r.stdout);
});

await t("**npm runs when a changed member is IN the wasm package the JS tests load (sdk#537): page-io yes, probe no**", async () => {
  const { wasmRoots, closure } = await import("../../tools/pr-scope.mjs");
  // The roots are READ from build.sh: its wasm builds, whatever else it compiles.
  assert.deepEqual(wasmRoots("cargo build --release -p web --target wasm32-unknown-unknown\ncargo build -q --release -p wire --bin x\ncargo build --profile decoder -p decoder --target wasm32-unknown-unknown\n"), ["web", "decoder"]);
  assert.deepEqual(wasmRoots(readFileSync(join(root, "build.sh"), "utf8")).sort(), ["decoder", "web"]);
  const meta = JSON.parse(execFileSync("cargo", ["metadata", "--format-version", "1", "--no-deps"], { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }));
  const shipped = closure(meta, ["web"]);
  assert.ok(shipped.has("page-io") && shipped.has("engine") && shipped.has("web"), `web's closure misses what it links: ${[...shipped]}`);
  assert.ok(!shipped.has("probe") && !shipped.has("testkit"), `a crate web does not link counts as shipped: ${[...shipped]}`);
  // The plan, end to end through gate.sh: a Rust-only change to a shipped member runs npm, and says why.
  const pio = plan(root, "page-io/src/lib.rs");
  assert.equal(planLine(pio.stdout, "npm"), "true", "a page-io-only PR skipped npm: #522's red JS tests went unseen that way");
  // THE CONTROL: a member outside the package does not.
  assert.equal(planLine(plan(root, "probe/src/lib.rs").stdout, "npm"), "false", "a probe-only PR ran npm");
});

await t("**EVERY control is in the plan whatever the PR touches; a README-only PR tests no member and runs no npm**", async () => {
  const r = plan(root, "README.md");
  const gateSrc = readFileSync(join(root, "gate.sh"), "utf8");
  const listed = [...gateSrc.matchAll(/^ {2}"([a-z0-9_-]+)\|/gm)].map(m => m[1]);
  assert.ok(listed.length >= 7, `the CONTROLS list read nothing: ${listed}`);
  assert.deepEqual(planLine(r.stdout, "controls").split(" "), listed);
  assert.equal(planLine(r.stdout, "members tested (changed only; dependents run at the batch gate)"), "(none)");
  assert.equal(planLine(r.stdout, "npm"), "false");
  const js = plan(root, "js/session.js");
  assert.equal(planLine(js.stdout, "npm"), "true", "a JS change did not run npm");
});

await t("**a DELETED test shows as a count DROP and fails; named with --accept-loss it is shown, not failed**", async () => {
  assert.deepEqual(count("engine", "120", "119"), { line: "engine: 120 -> 119 (-1) DROP: a test stopped running", drop: true });
  assert.equal(count("engine", "120", "119", true).drop, false);
  assert.match(count("engine", "120", "119", true).line, /DROP, named with --accept-loss/);
  assert.deepEqual(count("engine", "120", "121"), { line: "engine: 120 -> 121 (+1)", drop: false });
  assert.deepEqual(count("pieces", "-", "5"), { line: "pieces: NEW -> 5", drop: false });
  const r = spawnSync("node", [join(root, "tools/pr-scope.mjs"), "count", "web", "30", "29"], { encoding: "utf8" });
  assert.equal(r.status, 1, "a drop did not fail the command");
});

await t("**a target claimed by ANOTHER worktree is refused before anything builds**", async () => {
  const target = mkdtempSync(join(tmpdir(), "gate-pr-target-"));
  writeFileSync(join(target, ".craftworks-worktree"), "/some/other/worktree\n");
  const r = gate(root, ["--controls"], { CARGO_TARGET_DIR: target });
  rmSync(target, { recursive: true, force: true });
  assert.notEqual(r.status, 0, "a shared target was used");
  assert.match(r.stderr, /belongs to \/some\/other\/worktree, another worktree/);
});

await t("**a control violation PLANTED in a crate the PR does not touch fails the PR** (its own worktree and target)", async () => {
  const scratch = mkdtempSync(join(tmpdir(), "gate-pr-plant-"));
  const wt = join(scratch, "wt");
  const target = join(scratch, "target");
  execFileSync("git", ["-C", root, "worktree", "add", "--detach", wt, "HEAD"], { stdio: "ignore" });
  try {
    // The gate AS IT IS in this tree (a PR runs the gate on its own branch, committed or not).
    for (const f of ["gate.sh", "tools/pr-scope.mjs", "tools/target-dir.sh"]) writeFileSync(join(wt, f), readFileSync(join(root, f)));
    const env = { GATE_PR_CHANGED: "README.md", GATE_CONTROLS_ONLY: "one_home", CARGO_TARGET_DIR: target };
    const clean = gate(wt, ["--pr"], env);
    assert.equal(clean.status, 0, `THE CONTROL: the clean tree failed:\n${clean.stdout}\n${clean.stderr}`);
    assert.match(clean.stdout, /control one_home: ok \(\d+ test/);
    const f = join(wt, "wire/src/lib.rs");
    writeFileSync(f, `${readFileSync(f, "utf8")}\npub fn planted(b: u8) -> String { format!("{b:02x}") }\n`);
    const planted = gate(wt, ["--pr"], env);
    assert.notEqual(planted.status, 0, "a hand-written hex in wire/ (untouched by the PR) passed");
    assert.match(planted.stderr, /control one_home FAILED/);
  } finally {
    execFileSync("git", ["-C", root, "worktree", "remove", "--force", wt], { stdio: "ignore" });
    rmSync(scratch, { recursive: true, force: true });
  }
});

await t("**GATE_OWNERS_PER_PR is the batch gate's alone** (sdk#420): refused in --pr and a bare full run (both as --dry-run, so a mutant that HONOURS the flag ends fast instead of running a gate), before any work, naming the flag; a non-count refused even with --accept", async () => {
  for (const args of [["--pr", "--dry-run"], ["--dry-run"]]) {
    const r = gate(root, args, { GATE_OWNERS_PER_PR: "1" });
    assert.notEqual(r.status, 0, `honoured with ${JSON.stringify(args)}: a stray export would switch the owners control off`);
    assert.match(r.stderr, /GATE_OWNERS_PER_PR is only for the batch gate/);
  }
  const bad = gate(root, ["--accept"], { GATE_OWNERS_PER_PR: "x" });
  assert.notEqual(bad.status, 0);
  assert.match(bad.stderr, /must be a PR count, got 'x'/);
});

await t("**GATE_OWNERS_PER_PR is read once and UNSET before the gate starts anything** (batch 3: an inner gate.sh inherited it, refused, and npm lost 39 tests)", async () => {
  const src = readFileSync(new URL("../../gate.sh", import.meta.url), "utf8");
  const read = src.indexOf("OWNERS_PER_PR=${GATE_OWNERS_PER_PR:-}");
  const unset = src.indexOf("unset GATE_OWNERS_PER_PR");
  const firstChild = Math.min(...["cargo ", "npm ", "node "].map(w => { const i = src.indexOf("\n" + w); return i < 0 ? Infinity : i; }), src.indexOf("$guard"));
  assert.ok(read > 0, "the flag is not read into a local");
  assert.ok(unset > read, "the flag is not unset after it is read: a child gate.sh would inherit it");
  assert.ok(unset < firstChild, "the flag is unset only after the gate has started a child");
  const uses = [...src.matchAll(/GATE_OWNERS_PER_PR/g)].length;
  assert.ok(!/\$\{?GATE_OWNERS_PER_PR/.test(src.slice(unset)), "the environment variable is read again after it was unset");
  assert.ok(uses >= 3, `the scan found ${uses} mentions: it read nothing`);
});

rmSync(ADMIT_DIR, { recursive: true, force: true });
if (failures) { process.stdout.write(`gate-pr: ${failures} FAILED\n`); process.exit(1); }
process.stdout.write("gate-pr: all ok\n");
