// THE FILES THE JAVASCRIPT FETCHES ARE THE FILES THE BUILD SHIPS.
//
// `SHIPPED_ARTEFACTS` (js/session.js) names each artefact by a URL beside
// session.js; `build.sh` ships the files and writes their hashes to
// artefacts.json. Nothing tied the two together: renaming an entry to a file
// the build no longer ships (`./engine_delegate.wasm`, after the switch-over)
// left every other test green, and a page would have fetched a 404 and
// handed an empty array to provisioning (main's mutant M1 on sdk#260).
//
// So, from the BUILT pkg/web (the session.js a page loads, and the manifest
// beside it): for EVERY entry of SHIPPED_ARTEFACTS, the file it names exists
// in pkg/web, it is the manifest's file for that name, and its sha256 is the
// manifest's hash. Not a silent skip when the build is missing: without a
// build there is nothing to check, and that is a failure.

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.stack}\n`); }
};

const web = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "pkg", "web");
const sha = path => createHash("sha256").update(readFileSync(path)).digest("hex");

/**
 * Every way `shipped` (name → URL) disagrees with the build in `dir` and its
 * `manifest`: a file that is not shipped, a file that is not the manifest's
 * for that name, a hash that is not the manifest's. Empty means they agree.
 */
function disagreements(shipped, manifest, dir) {
  const out = [];
  for (const [name, url] of Object.entries(shipped)) {
    const file = basename(new URL(url).pathname);
    const entry = manifest[name];
    if (!entry) { out.push(`${name}: the manifest has no entry`); continue; }
    if (file !== entry.file) out.push(`${name}: fetches ${file}, the manifest ships ${entry.file}`);
    const path = join(dir, file);
    if (!existsSync(path)) { out.push(`${name}: ${file} is not in the build`); continue; }
    if (sha(path) !== entry.sha256) out.push(`${name}: ${file} does not hash to the manifest's ${entry.sha256}`);
  }
  return out;
}

const manifest = JSON.parse(readFileSync(join(web, "artefacts.json"), "utf8"));
const { SHIPPED_ARTEFACTS } = await import(join(web, "session.js"));

await t("**every artefact the JavaScript fetches is a file the build ships, with the manifest's hash**", async () => {
  const names = Object.keys(SHIPPED_ARTEFACTS).sort();
  assert.deepEqual(names, ["block", "register", "signer"], `SHIPPED_ARTEFACTS names ${names.join(", ")}`);
  assert.deepEqual(disagreements(SHIPPED_ARTEFACTS, manifest, web), []);
});

await t("THE CONTROL: a name the build no longer ships is refused, naming it", async () => {
  const wrong = { ...SHIPPED_ARTEFACTS, signer: new URL("./engine_delegate.wasm", SHIPPED_ARTEFACTS.signer).href };
  const found = disagreements(wrong, manifest, web);
  assert.ok(found.some(d => /signer: fetches engine_delegate\.wasm/.test(d)), `not refused: ${found}`);
  assert.ok(found.some(d => /engine_delegate\.wasm is not in the build/.test(d)), `the missing file was not named: ${found}`);
});

await t("THE CONTROL: a hash that is not the file's is refused", async () => {
  const lied = { ...manifest, block: { ...manifest.block, sha256: "0".repeat(64) } };
  const found = disagreements(SHIPPED_ARTEFACTS, lied, web);
  assert.ok(found.some(d => /block: block\.wasm does not hash/.test(d)), `not refused: ${found}`);
});

await t("**`modules` names EVERY module the package needs, each present (sdk#312)**", async () => {
  // A consumer that ships the SDK's JS (the builder's app container) takes
  // this list instead of its own; a hand list missed rto.js the day it came.
  const { reachable } = await import(join(dirname(fileURLToPath(import.meta.url)), "..", "..", "tools", "reachable.mjs"));
  const want = [...reachable(join(web, "index.js"), web)].sort();
  assert.ok(Array.isArray(manifest.modules), "artefacts.json has no `modules` list");
  assert.deepEqual([...manifest.modules].sort(), want, "`modules` is not what reachable.mjs computes");
  assert.ok(manifest.modules.includes("rto.js"), "THE CONTROL: rto.js (sdk#312) is not named");
  for (const m of manifest.modules) assert.ok(existsSync(join(web, m)), `${m} is named but not in pkg/web`);
});

await t("**`starter` names the SDK's pre-SDK modules: what its starter ENTRIES reach, computed (the one owner of the list)**", async () => {
  // The builder's starter container and its loader's `external` read this list; nobody keeps a copy (the architect:
  // it lived in three places, and adding a module was three edits). Its ENTRIES are the SDK's own fact: the modules
  // that declare themselves the starter's.
  const { reachable } = await import(join(dirname(fileURLToPath(import.meta.url)), "..", "..", "tools", "reachable.mjs"));
  // The entries are written HERE AGAIN on purpose: an INDEPENDENT oracle of build.sh's STARTER_ENTRIES. Read from
  // build.sh instead, a wrong entry there would agree with itself; stated twice, a change to one is a red test.
  const want = [...new Set(["served.js", "pieces.js"].flatMap(e => [...reachable(join(web, e), web)]))].sort();
  assert.ok(Array.isArray(manifest.starter), "artefacts.json has no `starter` list");
  assert.deepEqual([...manifest.starter].sort(), want, "`starter` is not what the starter entries reach");
  assert.deepEqual([...manifest.starter].sort(), ["instrument-vocab.js", "pieces.js", "rto.js", "served.js"], "THE CONTROL: the starter is not the four modules it is today");
  for (const m of manifest.starter) assert.ok(existsSync(join(web, m)), `${m} is named in starter but not in pkg/web`);
});

await t("**every module the build EMITS passes the one module rule its consumers check (core_types::name::module_ok)**", async () => {
  // The rule binds BOTH ends: the SDK never writes a module name its own
  // `sdk.ids.module` refuses, so a consumer never has to reject the SDK's manifest.
  const require = createRequire(import.meta.url);
  const raw = require(join(dirname(fileURLToPath(import.meta.url)), "..", "..", "pkg", "node", "craftworks_sdk.js"));
  const refused = manifest.modules.filter(m => !raw.module_name_ok(m));
  assert.deepEqual(refused, [], `the build emitted module names its own rule refuses: ${refused.join(", ")}`);
  assert.equal(raw.module_name_ok("../escape.js"), false, "THE CONTROL: the rule refuses a path");
});

if (failures) { process.stdout.write(`${failures} failed\n`); process.exit(1); }
