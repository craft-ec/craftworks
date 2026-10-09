// A BRANCH THAT CROSSES OWNERS SAYS SO (craftworks-sdk#320). Driven on a real
// git repository in a temp dir: the check reads what the branch changed.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync, mkdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { ownerOf, parseOwners } from "../../tools/owners.mjs";
import { childEnv } from "./common/child-env.mjs";

const CHECK = fileURLToPath(new URL("../../tools/owners.mjs", import.meta.url));
let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${String(e.message).split("\n").join("\n    ")}\n`); }
};
const OWNERS = "*        core\nengine/  e3\nsigner/  e4\njs/artefacts.js e4\n";

function repo() {
  const root = mkdtempSync(join(tmpdir(), "owners-test-"));
  const g = (...a) => execFileSync("git", ["-C", root, ...a], { encoding: "utf8" });
  g("init", "-q", "-b", "base");
  g("config", "user.email", "t@t"); g("config", "user.name", "t");
  for (const d of ["engine", "signer", "js"]) mkdirSync(join(root, d));
  writeFileSync(join(root, "OWNERS"), OWNERS);
  for (const f of ["engine/a.rs", "signer/b.rs", "js/artefacts.js", "README"]) writeFileSync(join(root, f), "x\n");
  g("add", "."); g("commit", "-qm", "base");
  g("checkout", "-qb", "work");
  const change = (f, msg) => { writeFileSync(join(root, f), `${msg}\n`); g("commit", "-qam", msg); };
  return { root, change, done: () => rmSync(root, { recursive: true, force: true }) };
}
const check = root => {
  try { return { code: 0, out: execFileSync("node", [CHECK, "--root", root, "--base", "base"], { encoding: "utf8", env: childEnv({ PR_BODY: "" }) }) }; }
  catch (e) { return { code: e.status, out: String(e.stdout) }; }
};

await t("longest prefix wins; a path without `/` owns exactly that file; `*` is the rest", async () => {
  const r = parseOwners(OWNERS);
  assert.deepEqual(["engine/x.rs", "signer/y.rs", "js/artefacts.js", "js/session.js", "enginex/z"].map(f => ownerOf(r, f)), ["e3", "e4", "e4", "core", "core"]);
});

await t("THE CONTROL: a branch in ONE owner's files passes, and prints the owner", async () => {
  const { root, change, done } = repo();
  change("engine/a.rs", "engine only");
  const r = check(root); done();
  assert.equal(r.code, 0, r.out);
  assert.match(r.out, /e3: engine\/a\.rs/);
});

await t("**a branch crossing two owners with no `shared:` line is red, and names both**", async () => {
  const { root, change, done } = repo();
  change("engine/a.rs", "engine");
  change("signer/b.rs", "and the signer");
  const r = check(root); done();
  assert.equal(r.code, 1, `a crossing passed:\n${r.out}`);
  assert.match(r.out, /CROSSES OWNERS/);
  assert.match(r.out, /e3: engine\/a\.rs/);
  assert.match(r.out, /e4: signer\/b\.rs/);
});

await t("…and passes once a commit on the branch says `shared:`", async () => {
  const { root, change, done } = repo();
  change("engine/a.rs", "engine");
  change("signer/b.rs", "and the signer\n\nshared: e3 reviewed the engine half");
  const r = check(root); done();
  assert.equal(r.code, 0, r.out);
});

await t("an UNCOMMITTED change counts: the gate runs on the tree", async () => {
  const { root, change, done } = repo();
  change("engine/a.rs", "engine");
  writeFileSync(join(root, "signer/b.rs"), "edited, not committed\n");
  const r = check(root); done();
  assert.equal(r.code, 1, r.out);
});

await t("no OWNERS file could not check: exit 2, never a pass", async () => {
  const { root, change, done } = repo();
  change("engine/a.rs", "engine");
  rmSync(join(root, "OWNERS"));
  const r = check(root); done();
  assert.equal(r.code, 2, r.out);
});

await t("run from a COPY in a temp dir, it still RUNS (never a silent exit 0)", async () => {
  const { copyFileSync } = await import("node:fs");
  const dir = mkdtempSync(join(tmpdir(), "owners-copy-"));
  copyFileSync(CHECK, join(dir, "owners.mjs"));
  const { root, change, done } = repo();
  change("engine/a.rs", "engine");
  change("signer/b.rs", "and the signer");
  let r;
  try { r = { code: 0, out: execFileSync("node", [join(dir, "owners.mjs"), "--root", root, "--base", "base"], { encoding: "utf8", env: childEnv({ PR_BODY: "" }) }) }; }
  catch (e) { r = { code: e.status, out: String(e.stdout) }; }
  done(); rmSync(dir, { recursive: true, force: true });
  assert.equal(r.code, 1, `the copied check did not run: exit ${r.code}\n${r.out}`);
});

if (failures) { console.log(`${failures} failing`); process.exit(1); }
