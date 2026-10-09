// A BUILD SHIPS ONLY WHAT IT JUST BUILT (sdk#263). A pre-#260 build's
// engine_delegate.wasm stayed in pkg/web after the build stopped making it,
// and shipped-artefacts' control then failed on a file nothing had built.
// `tools/pkg-reset.sh` empties pkg/web and pkg/node; `build.sh` runs it before
// its first write. Driven here with /bin/bash (macOS ships 3.2).
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const reset = join(root, "tools", "pkg-reset.sh");
let failures = 0;
const t = (name, fn) => {
  try { fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.message}\n`); }
};

/** A pkg dir as an old build left it: a stale artefact in web and node, and a kept symbols file. */
function planted() {
  const pkg = join(mkdtempSync(join(tmpdir(), "pkg-reset-")), "pkg");
  for (const d of ["web", "node", "web-symbols"]) mkdirSync(join(pkg, d), { recursive: true });
  writeFileSync(join(pkg, "web", "engine_delegate.wasm"), "stale");
  mkdirSync(join(pkg, "web", "old-dir"));
  writeFileSync(join(pkg, "web", "old-dir", "x.js"), "stale");
  writeFileSync(join(pkg, "node", "engine_delegate.wasm"), "stale");
  writeFileSync(join(pkg, "web-symbols", "craftworks_sdk_bg.abc.wasm"), "names");
  return pkg;
}

t("**a planted stale file is gone from pkg/web and pkg/node, and both are left empty**", () => {
  const pkg = planted();
  assert.ok(existsSync(join(pkg, "web", "engine_delegate.wasm")), "THE CONTROL: the stale file was planted");
  const r = spawnSync("/bin/bash", [reset, pkg], { encoding: "utf8" });
  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(readdirSync(join(pkg, "web")), [], "pkg/web still holds files no build wrote");
  assert.deepEqual(readdirSync(join(pkg, "node")), [], "pkg/node still holds files no build wrote");
});

t("the SYMBOLS are kept: a crash report from an older build still needs its names", () => {
  const pkg = planted();
  spawnSync("/bin/bash", [reset, pkg], { encoding: "utf8" });
  assert.equal(readFileSync(join(pkg, "web-symbols", "craftworks_sdk_bg.abc.wasm"), "utf8"), "names");
});

t("a pkg dir that does not exist yet (a fresh clone) is created, not an error", () => {
  const pkg = join(mkdtempSync(join(tmpdir(), "pkg-reset-fresh-")), "pkg");
  const r = spawnSync("/bin/bash", [reset, pkg], { encoding: "utf8" });
  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(readdirSync(join(pkg, "web")), []);
});

t("**build.sh resets pkg BEFORE its first write into it**", () => {
  const lines = readFileSync(join(root, "build.sh"), "utf8").split("\n").filter(l => !/^\s*#/.test(l));
  const reset = lines.findIndex(l => /^\s*tools\/pkg-reset\.sh pkg\s*$/.test(l));
  const firstWrite = lines.findIndex(l => /pkg\/(web|node)/.test(l));
  assert.ok(reset >= 0, "build.sh never runs tools/pkg-reset.sh pkg");
  assert.ok(reset < firstWrite, `build.sh writes into pkg (line: ${lines[firstWrite]}) before resetting it`);
});

if (failures) { process.stdout.write(`${failures} failed\n`); process.exit(1); }
