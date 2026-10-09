// sdk#228: the SHIPPED wasm has no function names, and a frame from a real
// abort inside it is symbolised from the names the build KEPT for exactly
// those bytes (keyed by their sha256).
import assert from "node:assert";
import { readFileSync, readdirSync } from "node:fs";
import { createHash } from "node:crypto";
import { functionNames, symbolise } from "../../tools/symbolise.mjs";

const shipped = readFileSync(new URL("../../pkg/web/craftworks_sdk_bg.wasm", import.meta.url));
const hash = createHash("sha256").update(shipped).digest("hex");
const kept = new URL(`../../pkg/web-symbols/craftworks_sdk_bg.${hash}.wasm`, import.meta.url);

// 1. Stripped: the shipped module carries no name section at all.
assert.equal(WebAssembly.Module.customSections(new WebAssembly.Module(shipped), "name").length, 0, "the shipped wasm still carries its name section");
console.log("  ok the shipped wasm carries no name section");

// 2. The names are kept, keyed by the SHIPPED bytes' hash — and only one set.
const keptFiles = readdirSync(new URL("../../pkg/web-symbols/", import.meta.url));
assert.deepEqual(keptFiles, [`craftworks_sdk_bg.${hash}.wasm`], `no names kept for these exact bytes: ${keptFiles}`);
const names = functionNames(readFileSync(kept));
assert.ok(names.size > 1000, `the kept names are too few to be a build's: ${names.size}`);
console.log(`  ok the names are kept for these exact bytes (${names.size} functions)`);

// 3. A REAL abort inside the shipped module (the build is panic = abort): an
// allocation no layout can hold. Every import answers with a no-op, which is
// enough to instantiate and never reached before the trap.
const imports = {};
for (const imp of WebAssembly.Module.imports(new WebAssembly.Module(shipped))) {
  imports[imp.module] ??= {};
  imports[imp.module][imp.name] = imp.kind === "function" ? () => 0 : undefined;
}
const { instance } = await WebAssembly.instantiate(shipped, imports);
let trace = null;
try {
  instance.exports.__wbindgen_malloc(0xfffffff0, 1 << 20);
} catch (e) {
  assert.ok(e instanceof WebAssembly.RuntimeError, `not a wasm trap: ${e}`);
  trace = e.stack;
}
assert.ok(trace, "the impossible allocation did not trap: the test drove nothing");
console.log("  ok a real abort inside the shipped wasm traps");
const frames = symbolise(trace, names);
assert.ok(frames.length > 0, `the trap's stack carries no wasm frames: ${trace}`);
assert.ok(frames.every(f => f.name !== null), `a frame was not symbolised: ${JSON.stringify(frames)}`);
assert.ok(frames.some(f => /malloc|alloc/.test(f.name)), `the frames do not name the allocation that aborted: ${JSON.stringify(frames)}`);
console.log("  ok its frames are symbolised from the kept names:", frames.slice(0, 3).map(f => `[${f.index}] ${f.name}`).join(" | "));
