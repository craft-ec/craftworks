// AN APP'S WEB CONTAINER, BUILT IN THE PAGE (builder#104), through the real
// wasm `AppContainer` — and read back by the REAL `xz` and `tar`, since a
// reader of our own would only agree with our own writer.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const { AppContainer, webapp_params } = createRequire(import.meta.url)("../../pkg/node/craftworks_sdk.js");

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.stack}\n`); }
};
const enc = s => new TextEncoder().encode(s);
const FILES = [["index.html", enc("<!doctype html><script type=module src=loader.js></script>")], ["loader.js", enc("export {}")], ["app.json", enc('{"name":"notes"}')]];
const build = files => { const c = new AppContainer(); for (const [p, b] of files) c.add(p, b); return c.finish(); };

await t("**the real xz and tar unpack it to exactly its files**", async () => {
  const c = build(FILES);
  const view = new DataView(c.buffer, c.byteOffset);
  assert.equal(view.getBigUint64(0), 0n, "metadata is not empty");
  const webLen = Number(view.getBigUint64(8));
  assert.equal(c.length, 16 + webLen, "the node's framing does not add up");
  const dir = mkdtempSync(join(tmpdir(), "app-container-"));
  try {
    const x = spawnSync("sh", ["-c", `xz -dc | tar -xf - -C ${dir}`], { input: c.subarray(16) });
    assert.equal(x.status, 0, `xz/tar refused it: ${x.stderr}`);
    for (const [p, b] of FILES) assert.deepEqual(new Uint8Array(readFileSync(join(dir, p))), b, p);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

await t("**deterministic, whatever order the files are added in** — so one app is one address", async () => {
  const a = build(FILES), b = build([...FILES].reverse());
  assert.deepEqual(a, b);
  assert.deepEqual(webapp_params(a), webapp_params(b));
  assert.equal(webapp_params(a).length, 32);
  const changed = build([...FILES.slice(0, 2), ["app.json", enc('{"name":"notes2"}')]]);
  assert.notDeepEqual(webapp_params(changed), webapp_params(a), "THE CONTROL: a different app got the same params");
});

await t("a path added twice, an unsafe path, and an empty container are refused", async () => {
  const c = new AppContainer();
  c.add("a.js", enc("1"));
  assert.throws(() => c.add("a.js", enc("2")), /twice/);
  const bad = new AppContainer();
  bad.add("../escape.js", enc("x"));
  assert.throws(() => bad.finish(), /plain relative path/);
  assert.throws(() => new AppContainer().finish(), /no files/);
});

await t("the SDK ENTRY hands a page the same three (wrap), not only the raw module", async () => {
  const { wrap } = await import("../../js/wrap.js");
  const raw = createRequire(import.meta.url)("../../pkg/node/craftworks_sdk.js");
  const { webapp } = wrap(raw);
  for (const k of ["params", "address", "AppContainer"]) assert.equal(typeof webapp[k], "function", `sdk.webapp.${k}`);
  const c = new webapp.AppContainer();
  for (const [p, b] of FILES) c.add(p, b);
  assert.deepEqual(c.finish(), build(FILES), "the entry's AppContainer is not the module's");
  // Its WEB part (builder#117: what a site publishes) is exactly what `finish` frames: [0 u64][][len u64][web].
  const web = c.web(), whole = c.finish();
  const head = new DataView(whole.buffer, whole.byteOffset);
  assert.equal(head.getBigUint64(0), 0n, "an app container carries no metadata");
  assert.equal(head.getBigUint64(8), BigInt(web.length), "finish frames a web part of another length");
  assert.deepEqual(whole.slice(16), web, "web() is not the web part finish frames");
  assert.deepEqual(webapp.params(build(FILES)), webapp_params(build(FILES)));
});

if (failures) { process.stdout.write(`${failures} failed\n`); process.exit(1); }
