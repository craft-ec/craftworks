// THE RATCHET REFUSES what a partial or shrinking run would write (sdk#159).
//
// `./gate.sh --accept` wrote `npm=14` over 146 after a suite stopped partway —
// the same run printed `npm LOST 92` — and the next honest run then read as
// "counts moved UP". `tools/gate-accept.sh` is now the baseline's one writer;
// this drives it with /bin/bash (macOS ships 3.2, which the gate must run on)
// and checks the gate still hands it the step-failure flag.
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { childEnv } from "./common/child-env.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const helper = join(root, "tools", "gate-accept.sh");
let failures = 0;
const t = (name, fn) => {
  try { fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.message}\n`); }
};
const sha = p => createHash("sha256").update(readFileSync(p)).digest("hex");

/** A baseline and a run's counts in a fresh dir; run the helper; report. */
function accept(baseline, counts, failed = 0, extra = []) {
  const dir = mkdtempSync(join(tmpdir(), "gate-accept-"));
  const b = join(dir, "gate.baseline"), c = join(dir, "counts");
  writeFileSync(b, baseline);
  writeFileSync(c, counts);
  const before = sha(b);
  const r = spawnSync("/bin/bash", [helper, b, String(failed), c, ...extra], { encoding: "utf8" });
  return { rc: r.status, err: r.stderr, unchanged: sha(b) === before, now: readFileSync(b, "utf8") };
}
const BASE = "craftworks-sdk=221\nengine=78\nnpm=146\n";

t("**a baseline that CANNOT be written is a failure naming it — never 'recorded' (sdk#252)**", () => {
  const dir = mkdtempSync(join(tmpdir(), "gate-accept-ro-"));
  const b = join(dir, "gate.baseline"), c = join(tmpdir(), `gate-accept-counts-${process.pid}`);
  writeFileSync(b, BASE); writeFileSync(c, "craftworks-sdk=222\nengine=78\nnpm=146\n");
  chmodSync(dir, 0o555); // the directory refuses the rename, as a full disk refuses the write
  try {
    const r = spawnSync("/bin/bash", [helper, b, "0", c], { encoding: "utf8" });
    assert.notEqual(r.status, 0, `a write that failed exited 0: ${r.stdout}${r.stderr}`);
    assert.match(r.stderr, /FAILED to write .*gate\.baseline/);
    assert.doesNotMatch(r.stdout, /recorded/, "it said 'recorded' about counts it did not record");
    assert.equal(readFileSync(b, "utf8"), BASE, "the baseline changed");
  } finally { chmodSync(dir, 0o755); }
});

t("**free space is checked again RIGHT BEFORE writing: under the floor, refused, byte-identical**", () => {
  const dir = mkdtempSync(join(tmpdir(), "gate-accept-"));
  const b = join(dir, "gate.baseline"), c = join(dir, "counts");
  writeFileSync(b, BASE); writeFileSync(c, "craftworks-sdk=222\nengine=78\nnpm=146\n");
  const r = spawnSync("/bin/bash", [helper, b, "0", c], { encoding: "utf8", env: childEnv({ GATE_MIN_GIB: "5", GATE_FREE_GIB_FOR_TEST: "2" }) });
  assert.notEqual(r.status, 0);
  assert.match(r.stderr, /2 GiB free, under 5 GiB, right before writing/);
  assert.equal(readFileSync(b, "utf8"), BASE);
});

t("THE CONTROL: a writable baseline with room is recorded and reads back identical", () => {
  const r = accept(BASE, "craftworks-sdk=222\nengine=78\nnpm=146\n");
  assert.equal(r.rc, 0, r.err);
  assert.equal(r.now, "craftworks-sdk=222\nengine=78\nnpm=146\n");
});

t("**a run in which a STEP FAILED is refused, and the baseline is byte-identical**", () => {
  // Counts that only RISE, so the loss check cannot be what refuses it: the
  // step-failure guard alone has to. (With npm=14 here, removing that guard
  // stayed refused — two guards on one property hide each other.)
  const r = accept(BASE, "craftworks-sdk=230\nengine=80\nnpm=150\n", 1);
  assert.notEqual(r.rc, 0);
  assert.ok(r.unchanged, "the baseline was rewritten by a failed run");
  assert.match(r.err, /a step of this run FAILED/);
});

t("THE CONTROL: a green run that RAISES a count is recorded", () => {
  const r = accept(BASE, "craftworks-sdk=224\nengine=78\nnpm=146\n");
  assert.equal(r.rc, 0, r.err);
  assert.equal(r.now, "craftworks-sdk=224\nengine=78\nnpm=146\n");
});

t("**a green run that LOWERS a count is refused without an override, byte-identical**", () => {
  const r = accept(BASE, "craftworks-sdk=221\nengine=78\nnpm=140\n");
  assert.notEqual(r.rc, 0);
  assert.ok(r.unchanged);
  assert.match(r.err, /npm would fall 146 -> 140 \(6 test\(s\) lost\)/);
});

t("a lowered count NAMED with --accept-loss is recorded, and what was lost is printed", () => {
  const r = accept(BASE, "craftworks-sdk=221\nengine=78\nnpm=140\n", 0, ["--accept-loss", "npm"]);
  assert.equal(r.rc, 0, r.err);
  assert.match(r.err, /npm LOWERED 146 -> 140: 6 test\(s\) given up/);
  assert.match(r.now, /^npm=140$/m);
});

t("an --accept-loss naming a member that did NOT fall is refused: it would license the next loss", () => {
  const r = accept(BASE, "craftworks-sdk=222\nengine=78\nnpm=146\n", 0, ["--accept-loss", "engine"]);
  assert.notEqual(r.rc, 0);
  assert.ok(r.unchanged);
});

t("a run with NO counts is refused, not recorded as an empty baseline", () => {
  const r = accept(BASE, "");
  assert.notEqual(r.rc, 0);
  assert.ok(r.unchanged);
});

t("a NEW member is recorded; one no longer in the run is dropped", () => {
  const r = accept(BASE, "craftworks-sdk=221\nnpm=146\nwire=33\n", 0, []);
  assert.equal(r.rc, 0, r.err);
  assert.equal(r.now, "craftworks-sdk=221\nnpm=146\nwire=33\n");
});

// The call site still ASKS (a helper nothing calls has cost this repo more
// than any other shape): gate.sh routes --accept through the helper, with the
// step-failure flag, and every step's failure sets it.
// THE DISK GUARD (the owner's control): gate.sh asks the ONE workspace guard before anything
// else, and does not start when it refuses -- nor when there is no guard to ask. Run for real,
// with stub guards, so no cargo or npm is reached either way (a refused gate costs nothing).
t("**gate.sh refuses to START when the disk guard refuses, and when there is no guard at all**", () => {
  const d = mkdtempSync(join(tmpdir(), "gate-guard-"));
  const refuse = join(d, "refuse.sh"), pass = join(d, "pass.sh");
  writeFileSync(refuse, "#!/bin/sh\necho 'stub guard: REFUSED'\nexit 1\n"); chmodSync(refuse, 0o755);
  writeFileSync(pass, "#!/bin/sh\necho \"stub guard: $1 may run\"\nexit 0\n"); chmodSync(pass, 0o755);
  // PATH without cargo: a gate the guard lets through stops at its own "no cargo" step, so no
  // build or suite runs in any case here -- and reaching that step is the proof it got past the guard.
  const run = guard => spawnSync("/bin/bash", ["./gate.sh"], { cwd: root, encoding: "utf8", env: childEnv({ PATH: "/usr/bin:/bin", DISK_GUARD: guard }), timeout: 60000 });
  const refused = run(refuse);
  assert.notEqual(refused.status, 0, "the gate started although the disk guard refused");
  assert.match(refused.stdout + refused.stderr, /stub guard: REFUSED/, "the guard was not asked");
  assert.doesNotMatch(refused.stdout + refused.stderr, /no cargo/, "the gate went on past a refusing guard");
  const missing = run(join(d, "no-such-guard.sh"));
  assert.notEqual(missing.status, 0, "a gate with NO disk guard started (could not check is a failure)");
  assert.match(missing.stderr, /no disk guard at .*no-such-guard\.sh/, "the missing guard was not named");
  // THE CONTROL: a passing guard lets the gate past this point: it reaches the next step (no cargo on this PATH).
  const passed = run(pass);
  assert.match(passed.stdout, /stub guard: the SDK gate may run/, "a passing guard was not asked, or not named what runs");
  assert.match(passed.stderr, /no cargo/, "a passing guard did not let the gate go on to its next step");
  rmSync(d, { recursive: true, force: true });
});

t("**gate.sh hands --accept to the helper WITH the step-failure flag, and writes the baseline nowhere else**", () => {
  const src = readFileSync(join(root, "gate.sh"), "utf8");
  assert.match(src, /\.\/tools\/gate-accept\.sh "\$BASELINE" "\$STEP_FAILED" "\$counts_file"/);
  assert.doesNotMatch(src, />>?\s*"\$BASELINE"/, "gate.sh writes the baseline directly again");
  const stepFails = (src.match(/^\s*.*\bstep_fail "/gm) ?? []).length;
  assert.ok(stepFails >= 8, `only ${stepFails} step failures set STEP_FAILED`);
  // THE CONTROL: the reader does find the other kind of failure — a count
  // that moved is a `fail`, not a `step_fail`, or --accept could never raise.
  assert.match(src, /\n\s+fail "\$m LOST/);
});

t("THE CONTROL: the helper really runs under /bin/bash (3.2 on macOS)", () => {
  const v = execFileSync("/bin/bash", ["-c", "echo $BASH_VERSION"], { encoding: "utf8" }).trim();
  assert.ok(v.length > 0);
});

// ── ONE FILE PER MEMBER (sdk#279) ─────────────────────────────────────────
// `gate.baseline.d/<member>` holds one count. The same rules as the file:
// a failed step or a fall is refused byte-identically, the write is checked,
// and what is written reads back — per member.

/** A baseline DIRECTORY (`{member: count}`) and a run's counts; run the helper. */
function acceptDir(members, counts, failed = 0, extra = []) {
  const root = mkdtempSync(join(tmpdir(), "gate-accept-d-"));
  const d = join(root, "gate.baseline.d"), c = join(root, "counts");
  mkdirSync(d);
  for (const [k, v] of Object.entries(members)) writeFileSync(join(d, k), `${v}\n`);
  writeFileSync(c, counts);
  const snap = () => readdirSync(d).sort().map(f => `${f}=${readFileSync(join(d, f), "utf8")}`).join("|");
  const before = snap();
  const r = spawnSync("/bin/bash", [helper, d, String(failed), c, ...extra], { encoding: "utf8", env: childEnv({ GATE_FREE_GIB_FOR_TEST: "100" }) });
  const now = Object.fromEntries(readdirSync(d).sort().map(f => [f, readFileSync(join(d, f), "utf8").trim()]));
  return { rc: r.status, out: r.stdout, err: r.stderr, unchanged: snap() === before, now };
}

t("**a green run that raises a count records ONE FILE PER MEMBER, and reads it back**", () => {
  const r = acceptDir({ "craftworks-sdk": 221, engine: 78, npm: 146 }, "craftworks-sdk=222\nengine=78\nnpm=146\n");
  assert.equal(r.rc, 0, r.err);
  assert.deepEqual(r.now, { "craftworks-sdk": "222", engine: "78", npm: "146" });
  assert.match(r.out, /recorded 3 counts in .*gate\.baseline\.d/);
});

t("**a count that FALLS is refused, and the directory is byte-identical**", () => {
  const r = acceptDir({ "craftworks-sdk": 221, engine: 78 }, "craftworks-sdk=220\nengine=79\n");
  assert.notEqual(r.rc, 0, "a fall was recorded");
  assert.ok(r.unchanged, "a refused run changed the directory");
  assert.match(r.err, /craftworks-sdk would fall 221 -> 220/);
});

t("a run whose STEP FAILED is refused, and the directory is byte-identical", () => {
  const r = acceptDir({ engine: 78 }, "engine=90\n", 1);
  assert.notEqual(r.rc, 0);
  assert.ok(r.unchanged);
});

t("a member no longer counted LOSES its file — the directory says what the tree holds", () => {
  const r = acceptDir({ engine: 78, gone: 5 }, "engine=78\n");
  assert.equal(r.rc, 0, r.err);
  assert.deepEqual(Object.keys(r.now), ["engine"], "a gone member's file outlived it");
});

/** A throwaway repository with `gate.baseline.d`, and a git that needs no config. */
function repo(members) {
  const dir = mkdtempSync(join(tmpdir(), "gate-merge-"));
  const git = (...a) => spawnSync("git", ["-c", "user.name=t", "-c", "user.email=t@invalid", "-c", "commit.gpgsign=false", ...a], { cwd: dir, encoding: "utf8" });
  git("init", "-q", "-b", "main");
  mkdirSync(join(dir, "gate.baseline.d"));
  for (const [k, v] of Object.entries(members)) writeFileSync(join(dir, "gate.baseline.d", k), `${v}\n`);
  git("add", "-A"); git("commit", "-q", "-m", "base");
  const branch = (name, member, value) => {
    git("checkout", "-q", "-b", name, "main");
    writeFileSync(join(dir, "gate.baseline.d", member), `${value}\n`);
    git("commit", "-q", "-am", name);
    git("checkout", "-q", "main");
  };
  return { dir, git, branch };
}

t("**two PRs that move DIFFERENT members merge with no conflict** — the reason for the split", () => {
  const { git, branch, dir } = repo({ "craftworks-sdk": 349, engine: 125, npm: 229 });
  branch("a", "engine", 129);
  branch("b", "npm", 231);
  assert.equal(git("merge", "-q", "--no-edit", "a").status, 0);
  const m = git("merge", "-q", "--no-edit", "b");
  assert.equal(m.status, 0, `independent count changes conflicted: ${m.stdout}${m.stderr}`);
  assert.equal(readFileSync(join(dir, "gate.baseline.d", "engine"), "utf8").trim(), "129");
  assert.equal(readFileSync(join(dir, "gate.baseline.d", "npm"), "utf8").trim(), "231");
});

t("THE CONTROL: two PRs that move the SAME member DO conflict — on that one file, as they should", () => {
  const { git, branch } = repo({ engine: 125, npm: 229 });
  branch("a", "engine", 129);
  branch("b", "engine", 127);
  assert.equal(git("merge", "-q", "--no-edit", "a").status, 0);
  const m = git("merge", "-q", "--no-edit", "b");
  assert.notEqual(m.status, 0, "two changes to one member's count merged silently");
  assert.match(git("diff", "--name-only", "--diff-filter=U").stdout.trim(), /^gate\.baseline\.d\/engine$/);
});

t("**gate.sh reads the counts ONE FILE PER MEMBER, and a missing file is a FAILURE, not zero**", () => {
  const src = readFileSync(join(root, "gate.sh"), "utf8");
  assert.match(src, /^BASELINE=gate\.baseline\.d$/m);
  assert.match(src, /base_for\(\) \{ \[ -f "\$BASELINE\/\$1" \]/, "the lookup is not per member");
  // A member with no recorded count fails the gate by name (its file deleted
  // reads exactly as a NEW member): the negative control is run live and its
  // line is in the PR; this pins the branch that makes it fail.
  assert.match(src, /fail "\$m is not in \$BASELINE/);
  assert.match(src, /if \[ -e gate\.baseline \]; then/, "the old single file is not refused");
});

if (failures) { console.log(`\ngate accept: ${failures} FAILED`); process.exit(1); }
console.log("\ngate accept: all ok");
