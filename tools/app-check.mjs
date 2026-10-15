// APP CHECK (the build's): AN APP RUNS IN ITS OWN FRAME (FOUNDATION.md §3, "Apps are installed"; ARCHITECTURE §1). A
// page module that mounts with the page's `ctx` (`mount(ctx, el)`) is the OS's — or an app NOT YET moved into its frame.
// The OS list is fixed; the NOT-YET list only shrinks: an app moved (`mount(api, el)`, published as bytes) is taken off
// it in the same change. A new page module on neither list fails the build.
//
//   node tools/app-check.mjs [repo dir]
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const root = process.argv.slice(2).find(a => !a.startsWith("--")) ?? new URL("..", import.meta.url).pathname;
const pkgs = join(root, "packages");
const bad = [];
const bad0 = s => bad.push(s);
// THE OS: what every app stands on (FOUNDATION §3) — stays in the page.
const OS = new Set(["places-side", "home", "settings", "account", "spaces-app", "space-home", "store", "tag", "contact", "chat", "mail", "wallet-app", "nodes-app", "explorer-app"]);
// APPS NOT YET IN THEIR FRAMES (10-15): only ever shorter.
const NOT_YET = new Set([]);
// FRAMED BY THE FRAME RUNTIME (FOUNDATION §3): the manifest's `framed` (the publisher's) — the app keeps its
// `mount(ctx, el)`, run in its frame with its grants.
const pub = readFileSync(join(root, "publish/src/main.rs"), "utf8");
const framedSrc = pub.match(/\\"framed\\": \{\{(.*?)\}\}/)?.[1] ?? "";
const FRAMED = new Set([...framedSrc.matchAll(/\\"([a-z0-9-]+)\\": \[/g)].map(m => m[1]));
if (!FRAMED.size) bad0("publish/src/main.rs: no `framed` map found (the check must see one)");
for (const f of readdirSync(pkgs).filter(f => f.endsWith(".js"))) {
  const name = f.slice(0, -3);
  const src = readFileSync(join(pkgs, f), "utf8");
  if (!/export\s+async\s+function\s+mount\s*\(\s*ctx\b/.test(src)) continue;
  if (OS.has(name) || NOT_YET.has(name) || FRAMED.has(name)) continue;
  bad.push(`packages/${f}: a page mounted with the page's ctx — an app runs in its own frame (\`mount(api, el)\`), or it is the OS's`);
}
// A name on the NOT-YET list that is framed now (or gone): take it off.
for (const name of NOT_YET) {
  let src = "";
  try {
    src = readFileSync(join(pkgs, `${name}.js`), "utf8");
  } catch {}
  if (!/export\s+async\s+function\s+mount\s*\(\s*ctx\b/.test(src)) bad.push(`${name}: framed (or gone) — take it off app-check's NOT-YET list`);
}
// EACH FRAMED APP's GRANTS ARE WHAT ITS CODE REACHES (10-16: Blog's hand list missed `blog-press` once it became the
// host's): the host capabilities required by the app and every package it loads into its frame (the trust core —
// `apphost.js`'s HOST, its one owner — is where the walk stops). Not one fewer (the app fails), not one more (least
// privilege). `--fix` writes the publisher's map from this.
const hostSrc = readFileSync(join(pkgs, "apphost.js"), "utf8");
const HOST = new Set([...(hostSrc.match(/const HOST = new Set\(\[([^\]]*)\]\)/)?.[1] ?? "").matchAll(/"([a-z0-9-]+)"/g)].map(m => m[1]));
if (HOST.size < 10) bad.push("packages/apphost.js: its HOST set not found (the check must see it)");
const srcOf = name => {
  try {
    return readFileSync(join(pkgs, `${name}.js`), "utf8");
  } catch {
    return "";
  }
};
const reqs = src => [...src.matchAll(/require\(\s*"([a-z0-9-]+)"\s*\)/g)].map(m => m[1]).concat([...src.matchAll(/\[([^\[\]]*)\]\.map\(\w+ => (?:ctx|app)\.require\(\w+\)\)/g)].flatMap(m => [...m[1].matchAll(/"([a-z0-9-]+)"/g)].map(x => x[1])));
const grantsOf = app => {
  const seen = new Set();
  const host = new Set();
  const walk = [app];
  while (walk.length) {
    const n = walk.pop();
    if (seen.has(n)) continue;
    seen.add(n);
    for (const r of reqs(srcOf(n))) HOST.has(r) ? host.add(r) : walk.push(r);
  }
  return [...host].sort();
};
const framedMap = Object.fromEntries([...framedSrc.matchAll(/\\"([a-z0-9-]+)\\": \[([^\]]*)\]/g)].map(m => [m[1], [...m[2].matchAll(/\\"([a-z0-9-]+)\\"/g)].map(x => x[1]).sort()]));
const want = Object.fromEntries(Object.keys(framedMap).map(a => [a, grantsOf(a)]));
if (process.argv.includes("--fix")) {
  const inner = Object.entries(want).map(([a, g]) => `\\"${a}\\": [${g.map(x => `\\"${x}\\"`).join(", ")}]`).join(", ");
  writeFileSync(join(root, "publish/src/main.rs"), pub.replace(/\\"framed\\": \{\{.*?\}\}/, `\\"framed\\": {{ ${inner} }}`));
  console.log("app-check: the publisher's framed grants written");
  process.exit(0);
}
for (const [a, g] of Object.entries(want)) {
  const has = framedMap[a];
  const missing = g.filter(x => !has.includes(x));
  const extra = has.filter(x => !g.includes(x));
  if (missing.length || extra.length) bad.push(`${a}: its grants ≠ what its code reaches${missing.length ? ` — missing ${missing.join(", ")}` : ""}${extra.length ? ` — not needed ${extra.join(", ")}` : ""} (node tools/app-check.mjs --fix)`);
}
if (bad.length) {
  console.error(`app-check: ${bad.length} site(s)\n${bad.join("\n")}`);
  process.exit(1);
}
console.log(`app-check: ok (${FRAMED.size} framed: ${[...FRAMED].join(", ")}; ${NOT_YET.size} not yet)`);
