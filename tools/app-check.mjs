// APP CHECK (the build's): AN APP RUNS IN ITS OWN FRAME (FOUNDATION.md §3, "Apps are installed"; ARCHITECTURE §1). A
// page module that mounts with the page's `ctx` (`mount(ctx, el)`) is the OS's — or an app NOT YET moved into its frame.
// The OS list is fixed; the NOT-YET list only shrinks: an app moved (`mount(api, el)`, published as bytes) is taken off
// it in the same change. A new page module on neither list fails the build.
//
//   node tools/app-check.mjs [repo dir]
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const root = process.argv[2] ?? new URL("..", import.meta.url).pathname;
const pkgs = join(root, "packages");
// THE OS: what every app stands on (FOUNDATION §3) — stays in the page.
const OS = new Set(["places-side", "home", "settings", "account", "spaces-app", "space-home", "store", "tag", "contact", "chat", "mail", "wallet-app", "nodes-app", "explorer-app"]);
// APPS NOT YET IN THEIR FRAMES (10-15): only ever shorter.
const NOT_YET = new Set(["wiki", "vote", "form", "note", "board", "blog", "media", "caption", "drive", "bundle-app"]);
const bad = [];
for (const f of readdirSync(pkgs).filter(f => f.endsWith(".js"))) {
  const name = f.slice(0, -3);
  const src = readFileSync(join(pkgs, f), "utf8");
  if (!/export\s+async\s+function\s+mount\s*\(\s*ctx\b/.test(src)) continue;
  if (OS.has(name) || NOT_YET.has(name)) continue;
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
if (bad.length) {
  console.error(`app-check: ${bad.length} site(s)\n${bad.join("\n")}`);
  process.exit(1);
}
console.log(`app-check: ok (${NOT_YET.size} app(s) not yet framed)`);
