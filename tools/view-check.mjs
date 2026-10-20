// VIEW CHECK (the build's): EVERY SCREEN HAS ONE BEHAVIOUR (FOUNDATION.md §3 `view`; owner 10-15: "the behavior is what
// we want standardize"). A screen (a page module, `mount(ctx, el)`) draws through `ctx.view` — loading, empty, still
// arriving, a failure, redrawn when what it read changes, never under the typing — with its own template. A screen
// that only hands its element to another that does (`DELEGATES`) is one already. The NOT-YET list only shrinks: a screen
// moved is taken off it in the same change. A new screen drawing on its own fails the build.
//
//   node tools/view-check.mjs [repo dir]
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const root = process.argv[2] ?? new URL("..", import.meta.url).pathname;
const pkgs = join(root, "packages");
// Screens that hand their element to a view elsewhere (`places.side`, the Settings page, a space's home; the office apps
// to `office-app`, whose one view draws Docs, Sheets and Slides).
const DELEGATES = new Set(["places-side", "settings", "space-home", "docs", "sheets", "slides"]);
// SCREENS NOT YET ON A VIEW (10-15): only ever shorter.
const NOT_YET = new Set([]);
const bad = [];
for (const f of readdirSync(pkgs).filter(f => f.endsWith(".js"))) {
  const name = f.slice(0, -3);
  const src = readFileSync(join(pkgs, f), "utf8");
  // ONE NAVIGATION (owner 10-15: "is it still many duplicate implementation?"): an app's entries are written only by
  // `where` (`tabs`, `menu`) — the side panel places them by their roles.
  if (f !== "where.js") for (const m of src.matchAll(/ctx\.actions\[[^\]]+\]\s*=[^=]/g)) bad.push(`packages/${f}:${src.slice(0, m.index).split("\n").length}: an app's menu written by hand — where.tabs / where.menu`);
  // ONE WAY TO ASK (10-16): `theme.ask`, by the control — never the browser's prompt / confirm / alert (an app's frame
  // has none, and a dialog an app could fill with anything is no system's).
  const prose = src.replace(/(^|[^:"'`\\])\/\/[^\n]*/g, (m, a) => a + " ".repeat(m.length - a.length));
  for (const m of prose.matchAll(/(?<![\w.$])(?:window\.)?(prompt|confirm|alert)\(/g)) bad.push(`packages/${f}:${src.slice(0, m.index).split("\n").length}: the browser's ${m[1]}() — theme.ask(anchor, { text, value?, ok? })`);
  if (!/export\s+async\s+function\s+mount\s*\(\s*ctx\b/.test(src)) continue;
  const viewed = /\bctx\.view\(/.test(src);
  if (viewed && NOT_YET.has(name)) bad.push(`${name}: on a view now — take it off view-check's NOT-YET list`);
  if (!viewed && !DELEGATES.has(name) && !NOT_YET.has(name)) bad.push(`packages/${f}: a screen drawing on its own — draw it through ctx.view (its template, the one behaviour)`);
}
if (bad.length) {
  console.error(`view-check: ${bad.length} site(s)\n${bad.join("\n")}`);
  process.exit(1);
}
console.log(`view-check: ok (${NOT_YET.size} screen(s) not yet on a view)`);
