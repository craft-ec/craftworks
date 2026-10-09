// TESTS THAT NEVER RAN (sdk#475): tools/test-targets.mjs over REAL cargo output shapes and real files in a scratch
// member, nothing compiled. Each refusal has its control: the same input with the test running passes.
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { addedTests, addedWasmTests, ignoredTests, silentTargets, targetsRun, unranTests } from "../../tools/test-targets.mjs";

let failures = 0;
const t = (name, fn) => {
  try { fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.message}\n`); }
};

// `cargo test -p web` on 200ce38, as printed (paths shortened): the wasm32-only lib runs 0, its tests/ run.
const WEB_OUT = `
     Running unittests src/lib.rs (target/debug/deps/web-4e82a51c0b64fc48)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/node_path_rules.rs (target/debug/deps/node_path_rules-9b2ac4aca15b3d3c)

running 4 tests
test every_node_path_is_built_in_one_place ... ok
test the_rules_hold ... ok
test inner::nested_case ... ignored
test last ... FAILED

test result: FAILED. 2 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.02s

     Running tests/empty_file.rs (target/debug/deps/empty_file-1)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests web

running 0 tests
`;

const member = (files) => {
  const d = mkdtempSync(join(tmpdir(), "test-targets-"));
  for (const [f, body] of Object.entries(files)) {
    mkdirSync(join(d, f, ".."), { recursive: true });
    writeFileSync(join(d, f), body);
  }
  return d;
};

t("targetsRun reads each target and its count, `unittests` prefix and Doc-tests included", () => {
  assert.deepEqual(targetsRun(WEB_OUT), [
    { src: "src/lib.rs", ran: 0 }, { src: "tests/node_path_rules.rs", ran: 4 }, { src: "tests/empty_file.rs", ran: 0 },
  ]);
});

t("**a lib whose src/ declares a #[test] and ran 0 is SILENT** (web's case); control: the same lib with no test is not", () => {
  const bad = member({ "src/lib.rs": "#![cfg(target_arch = \"wasm32\")]\nmod keep;\n", "src/keep.rs": "#[cfg(test)]\nmod t {\n  #[test]\n  fn shapes() {}\n}\n", "tests/empty_file.rs": "" });
  const ok = member({ "src/lib.rs": "#![cfg(target_arch = \"wasm32\")]\n#[cfg(test)]\nmod t {}\n", "tests/empty_file.rs": "// no tests\n" });
  try {
    const s = silentTargets(WEB_OUT, bad);
    assert.equal(s.length, 1, JSON.stringify(s));
    assert.equal(s[0].src, "src/lib.rs");
    assert.ok(s[0].file.endsWith("src/keep.rs"), s[0].file);
    assert.deepEqual(silentTargets(WEB_OUT, ok), [], "THE CONTROL: #[cfg(test)] alone is not a test");
  } finally { rmSync(bad, { recursive: true }); rmSync(ok, { recursive: true }); }
});

t("**a tests/x.rs (or tests/x/) that declares a test and ran 0 is silent**; a target that RAN is never silent", () => {
  const d = member({ "src/lib.rs": "", "tests/empty_file/helper.rs": "#[tokio::test(flavor = \"multi_thread\")]\nasync fn a() {}\n",
    "tests/node_path_rules.rs": "#[test]\nfn the_rules_hold() {}\n" });
  try {
    const s = silentTargets(WEB_OUT, d);
    assert.deepEqual(s.map(x => x.src), ["tests/empty_file.rs"], JSON.stringify(s));
  } finally { rmSync(d, { recursive: true }); }
});

t("**a bin target is its own file, not the whole src/** (probe's case: 14 bins wrongly blamed for one bin's test); the lib excludes src/bin/", () => {
  const out = [
    "     Running unittests src/lib.rs (target/debug/deps/probe-1)", "running 3 tests",
    "     Running unittests src/bin/live-reregister.rs (target/debug/deps/live_reregister-1)", "running 1 test",
    "     Running unittests src/bin/put-acks.rs (target/debug/deps/put_acks-1)", "running 0 tests",
    "     Running unittests src/bin/silent-bin.rs (target/debug/deps/silent_bin-1)", "running 0 tests",
  ].join("\n");
  const d = member({ "src/lib.rs": "#[test]\nfn a() {}\n", "src/bin/live-reregister.rs": "#[test]\nfn b() {}\n",
    "src/bin/put-acks.rs": "fn main() {}\n", "src/bin/silent-bin.rs": "fn main() {}\n", "src/bin/silent-bin/m.rs": "#[test]\nfn c() {}\n" });
  try {
    assert.deepEqual(silentTargets(out, d).map(x => x.src), ["src/bin/silent-bin.rs"], "only the bin whose OWN source declares a test");
  } finally { rmSync(d, { recursive: true }); }
  // The lib does not own src/bin/: a lib that ran 0 is not blamed for a bin's test.
  const lib = member({ "src/lib.rs": "pub fn x() {}\n", "src/bin/t.rs": "#[test]\nfn b() {}\n" });
  try {
    assert.deepEqual(silentTargets("     Running unittests src/lib.rs (x)\nrunning 0 tests\n", lib), []);
  } finally { rmSync(lib, { recursive: true }); }
});

t("what counts as a test attribute: #[test], #[tokio::test(...)]; NOT #[cfg(test)], #[test_case], wasm_bindgen_test", () => {
  const diff = (attr) => `+++ b/x/src/a.rs\n@@\n+${attr}\n+fn f() {}\n`;
  assert.equal(addedTests(diff("#[test]")).length, 1);
  assert.equal(addedTests(diff("#[tokio::test(flavor = \"current_thread\")]")).length, 1);
  assert.equal(addedTests(diff("#[cfg(test)]")).length, 0);
  assert.equal(addedTests(diff("#[test_case(1)]")).length, 0);
  assert.equal(addedTests(diff("#[wasm_bindgen_test]")).length, 0);
});

t("addedTests: added fns under a test attribute (other attributes between), an attribute added to an EXISTING fn; not removed ones", () => {
  const diff = [
    "diff --git a/page/src/a.rs b/page/src/a.rs", "--- a/page/src/a.rs", "+++ b/page/src/a.rs", "@@ -1,3 +1,12 @@",
    "+#[test]", "+#[ignore = \"slow\"]", "+pub fn one() {}",
    "+#[test]", " fn existing_now_a_test() {}",
    "-#[test]", "-fn removed() {}",
    "+#[test]", "+", "+async fn spaced() {}",
    "+#[test]", "+let x = 1;", "+fn not_after_code() {}",
  ].join("\n");
  assert.deepEqual(addedTests(diff).map(x => x.name), ["one", "existing_now_a_test", "spaced"]);
  assert.equal(addedTests(diff)[0].file, "page/src/a.rs");
});

t("**an added test cargo never listed is UNRAN**; ok / FAILED / ignored and path-qualified names all count as run", () => {
  const diff = ["+++ b/web/tests/node_path_rules.rs", "@@", "+#[test]", "+fn the_rules_hold() {}", "+#[test]", "+fn nested_case() {}",
    "+#[test]", "+fn last() {}", "+++ b/web/src/keep.rs", "@@", "+#[test]", "+fn shapes() {}"].join("\n");
  const u = unranTests(diff, WEB_OUT);
  assert.deepEqual(u, [{ file: "web/src/keep.rs", name: "shapes" }]);
  // THE CONTROL: the same test listed as run is not flagged.
  assert.deepEqual(unranTests(diff, `${WEB_OUT}\ntest keep::t::shapes ... ok\n`), []);
});

t("**an added test cargo lists as IGNORED is named** (it executes nothing); control: one that ran is not", () => {
  const diff = ["+++ b/web/tests/node_path_rules.rs", "@@", "+#[test]", "+#[ignore]", "+fn nested_case() {}", "+#[test]", "+fn the_rules_hold() {}"].join("\n");
  assert.deepEqual(unranTests(diff, WEB_OUT), [], "an ignored test is listed, so it is not unran");
  assert.deepEqual(ignoredTests(diff, WEB_OUT), [{ file: "web/tests/node_path_rules.rs", name: "nested_case" }]);
});

t("**an added #[wasm_bindgen_test] fails: no runner** (#475's class); control: a native #[test] is not one", () => {
  const diff = (attr) => `+++ b/web/src/a.rs\n@@\n+${attr}\n+fn in_browser() {}\n`;
  assert.deepEqual(addedWasmTests(diff("#[wasm_bindgen_test]")), [{ file: "web/src/a.rs", name: "in_browser" }]);
  assert.equal(addedWasmTests(diff("#[wasm_bindgen_test::wasm_bindgen_test]")).length, 1);
  assert.deepEqual(addedWasmTests(diff("#[test]")), [], "THE CONTROL");
});

if (failures) { process.stdout.write(`test-targets: ${failures} FAILED\n`); process.exit(1); }
