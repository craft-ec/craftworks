// EVERY CALL THE PAGE MAKES MUST EXIST ON THE REAL SESSION.
//
// Every other JavaScript test in this repo drives a FAKE `Session`, and that
// is deliberate: a test that needed a node and a browser would not run. But
// it means the whole JS suite is green over a `session.whatever()` that the
// wasm does not have — the fake answers it, the real object throws
// `is not a function`, and the first thing to find out is an app.
//
// That is not hypothetical. This file was written because two new calls
// (`tick_ms`, `flush`) went into `js/session.js` in the same change that
// added them to the wasm; had the `#[wasm_bindgen]` attribute been missing
// from either, every gate here would still have passed.
//
// So this one reads the BUILT bindings — `pkg/web/craftworks_sdk.d.ts`,
// which wasm-bindgen generates from what is actually exported — and checks
// the page against them.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");

let failures = 0;
const t = (name, fn) => {
  try { fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.message}\n`); }
};

/** The methods a class declares in the generated .d.ts. */
function methodsOf(dts, className) {
  const open = dts.indexOf(`export class ${className} {`);
  assert.notEqual(open, -1,
    `the built bindings declare no class ${className}. Either the build is ` +
    `stale — run build.sh — or the wasm stopped exporting it`);
  // Line by line to the first line that is exactly "}": a doc comment in the
  // body can contain anything, including a brace at the start of a line.
  const lines = dts.slice(open).split("\n");
  const end = lines.findIndex((l, i) => i > 0 && l === "}");
  assert.notEqual(end, -1, `could not find the end of class ${className}`);
  const names = new Set();
  for (const line of lines.slice(1, end)) {
    const m = /^\s+(?:static\s+|readonly\s+)?([A-Za-z_]\w*)\s*\(/.exec(line);
    if (m) names.add(m[1]);
  }
  assert.ok(names.size > 5,
    `only ${names.size} methods parsed out of ${className}; the reader is ` +
    `broken, not the class — a gate that finds nothing passes everything`);
  return names;
}

/** Every `session.NAME(` in a page file. */
function callsIn(file) {
  const src = readFileSync(join(root, file), "utf8");
  return new Set([...src.matchAll(/\bsession\.([A-Za-z_]\w*)\s*\(/g)].map(m => m[1]));
}

const dts = (() => {
  try {
    return readFileSync(join(root, "pkg/web/craftworks_sdk.d.ts"), "utf8");
  } catch (e) {
    // NOT A SKIP. An absent build is the state in which every check here
    // would silently pass, so it is the state that must fail.
    process.stdout.write(
      `  FAIL the built bindings are not there (${e.code}).\n` +
      `    Run build.sh first — this gate reads what the wasm actually\n` +
      `    exports, and it has nothing to read.\n`);
    process.exit(1);
  }
})();

t("**every session call the page makes exists on the real wasm Session**", () => {
  const declared = methodsOf(dts, "Session");
  const called = callsIn("js/session.js");
  const missing = [...called].filter(n => !declared.has(n));
  assert.deepEqual(missing, [],
    `js/session.js calls ${JSON.stringify(missing)} on the wasm Session, and ` +
    `the built bindings declare no such method. Every test in this suite uses ` +
    `a fake that answers it; a real page throws.`);
  process.stdout.write(`      ${called.size} calls checked against ${declared.size} exported methods\n`);
});

t("THE CONTROL: a call that does not exist is caught", () => {
  // Without this, a reader that found no calls — a changed variable name, a
  // regex that stopped matching — would report "0 missing" for ever.
  const declared = methodsOf(dts, "Session");
  assert.ok(!declared.has("no_such_method_on_the_session"),
    "the control names something that exists, so it proves nothing");
  const pretend = new Set(["tick", "no_such_method_on_the_session"]);
  const missing = [...pretend].filter(n => !declared.has(n));
  assert.deepEqual(missing, ["no_such_method_on_the_session"],
    "the comparison does not notice a method the wasm does not have");
});

t("and the two calls this gate was written for are among them", () => {
  const called = callsIn("js/session.js");
  for (const name of ["tick", "tick_ms", "flush"]) {
    assert.ok(called.has(name),
      `js/session.js no longer calls ${name}(). Either the page stopped ` +
      `telling the delegate the time, or this gate stopped reading the page`);
  }
});

/** Session methods whose parameters include a `bigint`, by name. */
function bigintMethods(text, className) {
  const open = text.indexOf(`export class ${className} {`);
  const lines = text.slice(open).split("\n");
  const end = lines.findIndex((l, i) => i > 0 && l === "}");
  return new Set(lines.slice(1, end).map(l => /^\s+([A-Za-z_]\w*)\s*\(([^)]*)\)/.exec(l)).filter(m => m && /bigint/.test(m[2])).map(m => m[1]));
}

t("**no session call the page makes takes a BigInt** — a JSON number would throw", () => {
  // `resume(ticket: u64)` crossed as `bigint`; the page passed the number
  // `take_loads` gave it, the call threw, and every read that waited on a
  // ticket reported UNAVAILABLE. Every fake in this suite accepted it.
  const big = bigintMethods(dts, "Session");
  const called = new Set([...callsIn("js/session.js"), ...callsIn("js/engine-db.js")]);
  const both = [...called].filter(n => big.has(n));
  assert.deepEqual(both, [], `the page calls ${JSON.stringify(both)} with numbers, and the wasm declares a bigint parameter`);
  assert.ok(called.has("resume"), "the reader no longer finds engine-db.js's resume() call, so it checks nothing");
});

t("THE CONTROL: a bigint parameter is found", () => {
  const fake = "export class Session {\n  free(): void;\n  resume(ticket: bigint): void;\n}\n";
  assert.deepEqual([...bigintMethods(fake, "Session")], ["resume"]);
});

process.stdout.write(failures ? `\n${failures} failing\n` : "\nall passing\n");
process.exit(failures ? 1 : 0);
