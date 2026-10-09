// THE DECODER'S wasm-opt IS PINNED (sdk#347): the build FAILS by name when binaryen 125's wasm-opt is not there,
// never shipping an unoptimised decoder in silence (tools/optimise-decoder.sh, which build.sh calls).
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { childEnv } from "./common/child-env.mjs";

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.message}\n`); }
};
const root = new URL("../../", import.meta.url).pathname;
const script = join(root, "tools/optimise-decoder.sh");
const input = join(root, "pkg/web/decoder.wasm");
const run = env => {
  const out = join(mkdtempSync(join(tmpdir(), "decoder-pin-")), "d.wasm");
  const r = spawnSync("/bin/bash", [script, input, out], { encoding: "utf8", env: childEnv(env) });
  return { ...r, out };
};

await t("**a PATH with no wasm-opt is REFUSED by name, and nothing is written**", async () => {
  const r = run({ PATH: "/usr/bin:/bin" });
  assert.notEqual(r.status, 0, "the decoder was shipped with no wasm-opt");
  assert.match(r.stderr, /wasm-opt \(not found on PATH\), expected binaryen 125/);
  assert.ok(!existsSync(r.out), "a decoder was written anyway");
});

await t("a DIFFERENT binaryen is refused by name", async () => {
  const r = run({ WANT_WASM_OPT_OVERRIDE_FOR_TEST: "999" });
  assert.notEqual(r.status, 0);
  assert.match(r.stderr, /expected binaryen 999/);
});

await t("THE CONTROL: the pinned wasm-opt optimises", async () => {
  const r = run({});
  assert.equal(r.status, 0, r.stderr);
  assert.ok(existsSync(r.out), "no decoder written");
});

if (failures) { process.stdout.write(`decoder-pin: ${failures} FAILED\n`); process.exit(1); }
process.stdout.write("decoder-pin: all ok\n");
