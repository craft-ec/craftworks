// THE README'S "Build a site on the SDK" IS THE RUNNING PAGE (sdk#234): the
// code block there is `examples/notes/index.html`'s program, verbatim — so the
// docs cannot drift from a page that is actually run (acceptance.mjs).
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const at = p => readFileSync(new URL(`../../${p}`, import.meta.url), "utf8");
const script = /<script type="module">\n([\s\S]*?)<\/script>/.exec(at("examples/notes/index.html"))?.[1];
assert.ok(script && script.includes("sdk.open("), "THE CONTROL: the page's program was found, and it is the SDK program");
const readme = at("README.md");
const section = readme.slice(readme.indexOf("## Build a site on the SDK"));
const block = /```js\n([\s\S]*?)```/.exec(section)?.[1];
assert.ok(block, "the README has no code block under \"Build a site on the SDK\"");
assert.equal(block, script, "the README's notes program is not the page's — update one from the other");
// And the page imports nothing but the SDK: a site on the SDK ALONE.
const imports = [...script.matchAll(/import .* from "([^"]+)"/g)].map(m => m[1]);
assert.deepEqual(imports, ["../../pkg/web/index.js"], `the page imports ${imports}`);
process.stdout.write("  ok  the README's notes program is the running page's, and it imports only the SDK\n");
