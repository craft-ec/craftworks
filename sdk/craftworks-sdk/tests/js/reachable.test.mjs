// The module-reachability walker, and the gate built on it.
//
// A hand-written list of files to ship is correct on the day it is written
// and silently wrong on the day someone adds a module. That has happened
// twice here already: `wrap.js` gained `session.js` and `engine-db.js`, and
// both this package and the builder's copy of it went on shipping the older
// list. Neither build failed. The symptom was ERR_MODULE_NOT_FOUND in a
// browser, at run time, in another repository.

import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync, rmSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { reachable } from "../../tools/reachable.mjs";

let failures = 0;
const t = (name, fn) => {
  try { fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.message}\n`); }
};

const fixture = files => {
  const dir = mkdtempSync(join(tmpdir(), "reach-"));
  for (const [name, body] of Object.entries(files)) {
    mkdirSync(join(dir, name, ".."), { recursive: true });
    writeFileSync(join(dir, name), body);
  }
  return dir;
};

t("it follows imports transitively", () => {
  const d = fixture({
    "index.js": 'export { wrap } from "./wrap.js";',
    "wrap.js": 'import { openSession } from "./session.js";\nimport { engineDb } from "./engine-db.js";\nexport const wrap = 1;',
    "session.js": 'import { connect } from "./connection.js";\nexport const openSession = 1;',
    "engine-db.js": "export const engineDb = 1;",
    "connection.js": "export const connect = 1;",
  });
  assert.deepEqual(reachable(join(d, "index.js"), d),
    ["connection.js", "engine-db.js", "index.js", "session.js", "wrap.js"]);
  rmSync(d, { recursive: true });
});

t("a bare side-effect import is followed too", () => {
  const d = fixture({ "index.js": 'import "./side.js";', "side.js": "globalThis.x = 1;" });
  assert.deepEqual(reachable(join(d, "index.js"), d), ["index.js", "side.js"]);
  rmSync(d, { recursive: true });
});

t("a file NOT imported is not reported reachable", () => {
  // The control for the test above: the walker must report a CLOSURE, not
  // every file in the directory. A walker that returned everything would
  // make the shipping gate pass on any package at all.
  const d = fixture({ "index.js": 'import "./a.js";', "a.js": "", "orphan.js": "" });
  const r = reachable(join(d, "index.js"), d);
  assert.ok(!r.includes("orphan.js"), `it claimed an unimported file: ${r}`);
  rmSync(d, { recursive: true });
});

t("**THE CASE THIS EXISTS FOR**: a missing module is an error, not a silence", () => {
  const d = fixture({ "index.js": 'import "./gone.js";' });
  assert.throws(() => reachable(join(d, "index.js"), d), e => {
    assert.match(e.message, /gone\.js/);
    assert.match(e.message, /not there/);
    return true;
  }, "a package shipping a broken import was reported as fine");
  rmSync(d, { recursive: true });
});

t("a dynamic import is REFUSED, not quietly skipped", () => {
  // The walker cannot follow one, so it must not report a closure it did not
  // compute — that would be a gate that checks less than it says it does.
  const d = fixture({ "index.js": 'const m = await import("./x.js");', "x.js": "" });
  assert.throws(() => reachable(join(d, "index.js"), d), /dynamic import/);
  rmSync(d, { recursive: true });
});

t("a cycle terminates", () => {
  const d = fixture({ "a.js": 'import "./b.js";', "b.js": 'import "./a.js";' });
  assert.deepEqual(reachable(join(d, "a.js"), d), ["a.js", "b.js"]);
  rmSync(d, { recursive: true });
});

process.stdout.write(failures ? `\n${failures} failing\n` : "\nall passing\n");
process.exit(failures ? 1 : 0);
