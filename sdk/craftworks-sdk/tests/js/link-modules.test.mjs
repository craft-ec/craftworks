// LINKING THE BUNDLE'S MODULES (sdk#347): relative imports rewritten to per-path URLs, in dependency order, one URL
// per path (one instance), starter paths to the starter's URLs, a cycle refused. In node, data: URLs stand in for
// the page's blob: URLs (node imports data: modules; the blob: path was measured in a sandboxed frame, #347).
import assert from "node:assert/strict";
import { linkModules } from "../../js/pieces.js";

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.message}\n`); }
};
const enc = new TextEncoder();
const files = o => new Map(Object.entries(o).map(([p, s]) => [p, enc.encode(s)]));
// UNIQUE per call, like the page's blob: URLs (a data: URL of the same text is the same URL, and would hide a
// module linked twice behind one instance).
let minted = 0;
const dataUrl = text => `data:text/javascript;base64,${Buffer.from(`${text}\n// url ${(minted += 1)}`).toString("base64")}`;
globalThis.__linkCount = 0;

await t("a bundle shaped like the real one links, and the glue is ONE instance for every importer", async () => {
  const f = files({
    "sdk/craftworks_sdk.js": `globalThis.__linkCount += 1; export const id = Symbol("glue");`,
    "sdk/wrap.js": `import { id } from "./craftworks_sdk.js"; export const wrapGlue = id;`,
    "sdk/index.js": `// Web entry point: import { load } from "./sdk/index.js" (a comment, not an import)
import { id } from "./craftworks_sdk.js";
import { wrapGlue } from "./wrap.js";
import { rto } from "./rto.js";
export const same = id === wrapGlue; export const fromStarter = rto;`,
    "handoff.js": `import { id } from "./sdk/craftworks_sdk.js"; export const handoffGlue = id;`,
    "runtime.js": `import { handoffGlue } from "./handoff.js";
import { id } from "./sdk/craftworks_sdk.js";
export const runtimeSame = handoffGlue === id;`,
    "sdk/craftworks_sdk_bg.wasm": "not a module",
  });
  const starter = { "sdk/rto.js": dataUrl(`export const rto = "from the starter";`) };
  const urls = linkModules(f, { external: p => starter[p] ?? null, toUrl: dataUrl });
  assert.deepEqual([...urls.keys()].sort(), ["handoff.js", "runtime.js", "sdk/craftworks_sdk.js", "sdk/index.js", "sdk/wrap.js"]);
  const index = await import(urls.get("sdk/index.js"));
  const runtime = await import(urls.get("runtime.js"));
  assert.equal(index.same, true, "index and wrap saw two different glue modules");
  assert.equal(runtime.runtimeSame, true, "runtime and handoff saw two different glue modules");
  assert.equal(index.fromStarter, "from the starter");
  assert.equal(globalThis.__linkCount, 1, `the glue module ran ${globalThis.__linkCount} times`);
});

await t("a cycle is refused by name", async () => {
  const f = files({ "a.js": `import "./b.js";`, "b.js": `import "./a.js";` });
  assert.throws(() => linkModules(f, { toUrl: dataUrl }), /cycle: a\.js -> b\.js -> a\.js/);
});

await t("../ resolves against the importer's directory", async () => {
  const f = files({ "sdk/x.js": `import { v } from "../top.js"; export const w = v;`, "top.js": `export const v = 7;` });
  const urls = linkModules(f, { toUrl: dataUrl });
  assert.equal((await import(urls.get("sdk/x.js"))).w, 7);
});

await t("the SDK's REAL modules (pkg/web), linked from a bundle, import and load its wasm from bytes", async () => {
  // What a published app does (#347): every module has a URL nothing resolves a relative path against, so a module
  // that computes one at its top level (`new URL("./signer.wasm", import.meta.url)`) throws before the app runs.
  const { readFileSync } = await import("node:fs");
  const web = new URL("../../pkg/web/", import.meta.url);
  // The starter's modules, from the SDK's own manifest: the one owner of the list (never a copy here).
  const manifest = JSON.parse(readFileSync(new URL("artefacts.json", web), "utf8"));
  const starter = manifest.starter;
  assert.ok(Array.isArray(starter) && starter.length > 0, "artefacts.json names no `starter`");
  // The bundle is every module the SDK's entry reaches that the starter does not carry: both from the manifest.
  const inBundle = manifest.modules.filter(m => !starter.includes(m));
  const f = new Map(inBundle.map(m => [`sdk/${m}`, readFileSync(new URL(m, web))]));
  const ext = Object.fromEntries(starter.map(m => [`sdk/${m}`, new URL(m, web).href]));
  const urls = linkModules(f, { external: p => ext[p] ?? null, toUrl: dataUrl });
  // EVERY module, through the URL linkModules produced for it (the architect, #355): each resolves, so no specifier
  // was mis-rewritten (a regex does the rewriting; this is its evidence).
  assert.deepEqual([...urls.keys()].sort(), inBundle.map(m => `sdk/${m}`).sort());
  for (const [p, u] of urls) await assert.doesNotReject(() => import(u), `${p} did not import through its linked URL`);
  const { load } = await import(urls.get("sdk/index.js"));
  const sdk = await load(readFileSync(new URL("craftworks_sdk_bg.wasm", web)));
  assert.equal(typeof sdk.openAsked, "function");
  // Nothing is beside a linked module: the shipped artefacts are absent, and a caller must name its own.
  assert.equal(sdk.SHIPPED_ARTEFACTS, null);
  await assert.rejects(() => sdk.openAsked({ port: 1 }), /needs the artefacts/);
});

await t("the bundle build REFUSES a dynamic import of a non-literal (linkModules can rewrite only literals)", async () => {
  const { execFileSync, spawnSync } = await import("node:child_process");
  const { mkdtempSync, writeFileSync } = await import("node:fs");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  const root = new URL("../../", import.meta.url).pathname;
  const tool = join(root, "pkg/tools/load-pieces");
  const dir = mkdtempSync(join(tmpdir(), "dynamic-import-"));
  const put = (n, text) => { writeFileSync(join(dir, n), text); return `sdk/${n}=${join(dir, n)}`; };
  const code = join(root, "pkg/web/webapp.wasm");
  const bad = spawnSync(tool, [code, "2048", "2", join(dir, "out"), put("x.js", `export const x = 1;\nconst m = await import(name);\n`)], { encoding: "utf8" });
  assert.notEqual(bad.status, 0, "a dynamic import(name) was bundled");
  assert.match(bad.stderr, /sdk\/x\.js:2: a dynamic import/);
  // THE CONTROL: a literal dynamic import, and the word in a comment, bundle.
  execFileSync(tool, [code, "2048", "2", join(dir, "ok"), put("y.js", `// import(anything) in a comment\nconst m = await import("./x.js");\n`)]);
});

if (failures) { process.stdout.write(`link-modules: ${failures} FAILED\n`); process.exit(1); }
process.stdout.write("link-modules: all ok\n");
