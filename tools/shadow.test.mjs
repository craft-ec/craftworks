// NO PACKAGE SHADOWS ITS OWN EXPORT: a name a package exports at the top level is never declared again inside it (a
// `const`/`let`, a destructured name, a function). v399: `items` exported `refOf(by, id)` and its `start` destructured a
// link reader of the same name — every profile item's reference came out wrong, and Discover and the media feeds were
// empty. Read from the sources (comments and strings left out).
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";

const strip = src => src.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/[^\n]*/g, "").replace(/`(?:\\.|[^`\\])*`|"(?:\\.|[^"\\\n])*"|'(?:\\.|[^'\\\n])*'/g, '""');

export function shadows(src) {
  const s = strip(src);
  const exported = new Set([...s.matchAll(/^export\s+(?:async\s+)?(?:const|let|function)\s+([A-Za-z_$][\w$]*)/gm)].map(m => m[1]));
  const found = [];
  // (The LOADER's entry points — `start`, `frame` — are called by the loader, never from inside the package: a local
  // helper of that name inside them reads nothing of the export.)
  for (const name of [...exported].filter(n => n !== "start" && n !== "frame")) {
    const n = name.replace(/\$/g, "\\$");
    // Declared again, not at the top level (indented): a const/let/function of that name, or a destructured one.
    const again = new RegExp(`^\\s+(?:(?:const|let|var)\\s+${n}\\b|(?:async\\s+)?function\\s+${n}\\s*\\(|(?:const|let|var)\\s*\\{[^}]*\\b${n}\\b(?!\\s*:)[^}]*\\}\\s*=)`, "m");
    if (again.test(s)) found.push(name);
  }
  return found;
}

test("no package re-declares a name it exports", () => {
  const dir = new URL("../packages/", import.meta.url);
  const bad = readdirSync(dir).filter(f => f.endsWith(".js")).flatMap(f => shadows(readFileSync(new URL(f, dir), "utf8")).map(n => `${f}: ${n}`));
  assert.deepEqual(bad, []);
});

test("control: the v399 shape is caught", () => {
  const v399 = 'export const refOf = (by, id) => `${by}/${id}`;\nexport async function start(ctx) {\n  const { appOf, pageOf, linkOf, refOf } = pure(kinds);\n  return { refOf };\n}\n';
  assert.deepEqual(shadows(v399), ["refOf"]);
  assert.deepEqual(shadows(v399.replace("refOf } = pure", "linkRef } = pure")), [], "renamed: nothing");
  assert.deepEqual(shadows('export const a = 1;\nfunction f() {\n  const { a: b } = x;\n}\n'), [], "a renamed binding is not a shadow");
});
