// THE MUTANT HELPER (tools/mutant.sh, sdk#391): one place that applies a mutant, runs the test, says
// "mutant killed" or "SURVIVED", and ALWAYS restores the file with a FRESH mtime -- the case that
// bit sdk#389: a restore that brought back an older mtime left cargo's build on the last mutant.
//
// Everything runs against a throwaway crate in a temp dir with its own CARGO_TARGET_DIR, so no
// real source is ever mutated. MUTANT_UNDER_TEST names another copy of the helper (its mutants).
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { childEnv } from "./common/child-env.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const helper = process.env.MUTANT_UNDER_TEST ?? join(root, "tools", "mutant.sh");
let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.message}\n`); }
};

const fx = mkdtempSync(join(tmpdir(), "mutant-test-"));
const crate = join(fx, "mutfix");
const lib = join(crate, "src", "lib.rs");
const ORIGINAL = `pub fn value() -> u32 {
    41 + 1
}

// A line no test reads: editing it is the planted SURVIVOR.
pub const NOTE: &str = "unread";

#[cfg(test)]
mod t {
    #[test]
    fn value_is_forty_two() {
        assert_eq!(super::value(), 42);
    }
}
`;
mkdirSync(join(crate, "src"), { recursive: true });
writeFileSync(join(crate, "Cargo.toml"), `[package]\nname = "mutfix"\nversion = "0.1.0"\nedition = "2021"\n\n[workspace]\n`);
writeFileSync(lib, ORIGINAL);
// Its own target, and never the SDK's toolchain file: a crate in a temp dir of its own.
const env = childEnv({ CARGO_TARGET_DIR: join(fx, "target") });
delete env.MUTANT_CHECK;
const cargoTest = ["cargo", "test", "-q", "--manifest-path", join(crate, "Cargo.toml")];
const mutant = (search, replace, cmd, extra = {}) =>
  spawnSync("/bin/bash", [helper, lib, search, replace, "--", ...cmd], { encoding: "utf8", env: { ...env, ...extra }, cwd: fx });
const unchanged = () => assert.equal(readFileSync(lib, "utf8"), ORIGINAL, "the file is not byte-identical to its original");

// The baseline builds and passes, so every verdict below is about the mutant.
const base = spawnSync(cargoTest[0], cargoTest.slice(1), { encoding: "utf8", env });
assert.equal(base.status, 0, `the fixture crate does not pass unmutated: ${base.stdout}${base.stderr}`);

await t("**a mutant the test catches is `mutant killed` (exit 0), and the file comes back byte-identical with a NEWER mtime**", () => {
  const before = statSync(lib).mtimeMs;
  const r = mutant("41 + 1", "41 + 2", cargoTest);
  assert.equal(r.status, 0, r.stdout + r.stderr);
  assert.match(r.stdout, /mutant killed \(rc=\d+\)$/m);
  assert.doesNotMatch(r.stdout, /COMPILE ERROR/);
  assert.match(r.stdout, /value_is_forty_two/, "the test's own output is not shown");
  assert.match(r.stdout, /mutant: control passed/, "the unmutated control did not run");
  unchanged();
  assert.ok(statSync(lib).mtimeMs > before, "the restore did not give the file a fresh mtime");
});

await t("**a planted SURVIVOR is reported `SURVIVED` (exit 1), never a kill**", () => {
  const r = mutant(`"unread"`, `"still unread"`, cargoTest);
  assert.equal(r.status, 1, r.stdout + r.stderr);
  assert.match(r.stdout, /^SURVIVED: the command passed with the mutant in place$/m);
  unchanged();
});

await t("a kill by the COMPILER is named as one: not an assertion", () => {
  const r = mutant("41 + 1", `"forty-two"`, cargoTest);
  assert.equal(r.status, 0, r.stdout + r.stderr);
  assert.match(r.stdout, /mutant killed \(rc=\d+\) -- by a COMPILE ERROR, not an assertion/);
  unchanged();
});

await t("**search text not there EXACTLY ONCE is REFUSED (exit 2) and the file is not touched: a mutant that changed nothing is not a kill**", () => {
  const mtime = statSync(lib).mtimeMs;
  for (const [search, n] of [["no such text", 0], ["value", 3]]) {
    const r = mutant(search, "x", ["/usr/bin/false"]);
    assert.equal(r.status, 2, `${search}: ${r.stdout}${r.stderr}`);
    assert.match(r.stderr, new RegExp(`occurs ${n} times`));
    assert.doesNotMatch(r.stdout, /mutant killed/, "a mutant that was never applied was reported killed");
  }
  const same = mutant("41 + 1", "41 + 1", ["/usr/bin/false"]);
  assert.equal(same.status, 2, same.stdout + same.stderr);
  assert.match(same.stderr, /changes nothing/);
  unchanged();
  assert.equal(statSync(lib).mtimeMs, mtime, "a refused mutant touched the file");
});

await t("**a command that FAILS ON THE ORIGINAL is `could not judge` (exit 2), never a kill: nothing was measured**", () => {
  const r = mutant("41 + 1", "41 + 2", ["/usr/bin/false"]);
  assert.equal(r.status, 2, r.stdout + r.stderr);
  assert.match(r.stdout, /COULD NOT JUDGE: the command fails WITHOUT the mutant too/);
  assert.doesNotMatch(r.stdout, /mutant killed/, "a command that always fails was reported as a kill");
  unchanged();
});

await t("**INTERRUPTED mid-run (TERM, after the mutant was BUILT): the file comes back byte-identical, and a later plain `cargo test` builds the ORIGINAL** (sdk#389's mtime case)", async () => {
  const built = join(fx, "built");
  rmSync(built, { force: true });
  // Build the mutant's test binary, say so, then wait to be interrupted.
  const cmd = ["/bin/sh", "-c", `cargo test -q --no-run --manifest-path '${join(crate, "Cargo.toml")}' && touch '${built}' && sleep 120`];
  const p = spawn("/bin/bash", [helper, lib, "41 + 1", "41 + 2", "--", ...cmd], { env, cwd: fx });
  let err = "";
  p.stderr.on("data", d => (err += d));
  const exited = new Promise(res => p.on("exit", (code, sig) => res({ code, sig })));
  const deadline = Date.now() + 120_000;
  while (!existsSync(built)) {
    assert.ok(Date.now() < deadline, "the mutant was never built");
    await new Promise(r => setTimeout(r, 100));
  }
  p.kill("SIGTERM");
  const { code } = await exited;
  assert.equal(code, 143, `the helper did not exit as interrupted (${code}): ${err}`);
  assert.match(err, /interrupted \(TERM\); ending the command's process group, then restoring/);
  unchanged();
  // The case that bit: the mutant's artefacts are newer than the source unless the restore
  // wrote a fresh mtime. A plain run must rebuild and pass on the ORIGINAL.
  const after = spawnSync(cargoTest[0], cargoTest.slice(1), { encoding: "utf8", env });
  assert.equal(after.status, 0, `a later cargo test ran the MUTANT, not the original: ${after.stdout}${after.stderr}`);
});

// A crate SLOW TO COMPILE (a const the compiler spins on), so a TERM can land while rustc is
// still building the mutant: the grandchild the helper must end before it restores.
const slow = join(fx, "slowfix");
const slowLib = join(slow, "src", "lib.rs");
const SLOW = `#![allow(long_running_const_eval)]
pub const SPIN: u64 = {
    let mut i = 0u64;
    let mut a = 0u64;
    while i < 1_500_000 {
        a = a.wrapping_mul(6364136223846793005).wrapping_add(i);
        i += 1;
    }
    a
};
pub fn value() -> u32 {
    41 + 1
}
#[cfg(test)]
mod t {
    #[test]
    fn value_is_forty_two() {
        let _ = super::SPIN;
        assert_eq!(super::value(), 42);
    }
}
`;
mkdirSync(join(slow, "src"), { recursive: true });
writeFileSync(join(slow, "Cargo.toml"), `[package]\nname = "slowfix"\nversion = "0.1.0"\nedition = "2021"\n\n[workspace]\n`);
writeFileSync(slowLib, SLOW);
const slowTest = ["cargo", "test", "-q", "--manifest-path", join(slow, "Cargo.toml")];
// This fixture's own rustc processes: its crate name, writing into this test's own target dir
// (a unique temp path; cargo hands rustc a RELATIVE source path).
const compiling = () =>
  spawnSync("/bin/ps", ["-Ao", "command"], { encoding: "utf8" }).stdout.split("\n")
    .filter(l => /\brustc\b/.test(l) && l.includes("--crate-name slowfix") && l.includes(env.CARGO_TARGET_DIR)).length;

await t("**TERM MID-COMPILE: while rustc is still building the mutant, the helper ends the WHOLE process group before it restores, and a later plain `cargo test` builds the ORIGINAL**", async () => {
  const p = spawn("/bin/bash", [helper, slowLib, "41 + 1", "41 + 2", "--", ...slowTest], { env, cwd: fx });
  let err = "";
  p.stderr.on("data", d => (err += d));
  const exited = new Promise(res => p.on("exit", code => res(code)));
  const deadline = Date.now() + 120_000;
  while (compiling() === 0) {
    assert.ok(Date.now() < deadline, "rustc never started on the mutant");
    await new Promise(r => setTimeout(r, 50));
  }
  p.kill("SIGTERM");
  const code = await exited;
  assert.equal(code, 143, `the helper did not exit as interrupted (${code}): ${err}`);
  assert.equal(readFileSync(slowLib, "utf8"), SLOW, "the file is not byte-identical to its original");
  const left = compiling();
  // An orphan rustc (a helper that ended only its direct child) is let finish, so the next
  // run meets whatever it wrote -- the case this test is for.
  while (compiling() > 0) {
    assert.ok(Date.now() < deadline + 120_000, "a rustc on the mutant never ended");
    await new Promise(r => setTimeout(r, 100));
  }
  assert.equal(left, 0, `${left} rustc process(es) on the mutant outlived the helper`);
  const after = spawnSync(slowTest[0], slowTest.slice(1), { encoding: "utf8", env });
  assert.equal(after.status, 0, `a later cargo test ran the MUTANT, not the original: ${after.stdout}${after.stderr}`);
});

rmSync(fx, { recursive: true, force: true });
process.stdout.write(failures ? `\n${failures} failing\n` : "\nall ok\n");
process.exit(failures ? 1 : 0);
