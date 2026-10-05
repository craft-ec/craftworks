// STYLE CHECK (the build's): ONE LOOK PER COMPONENT. A shared class (`cw-…`) gets its look only in the package that
// makes it (puts it in a `className`) — the theme (`theme.js`) is everyone's base. Another package may only PLACE it
// (position, size, flex/grid, margins). And no two packages that both make a class may both style it (a name clash:
// `cw-panel` was the theme's and the spaces panel's). Fails the build, naming each rule.
//
//   node tools/style-check.mjs [packages dir]
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const dir = process.argv[2] ?? new URL("../packages", import.meta.url).pathname;
const BASE = "theme.js";
// PLACING a component, not restyling it.
const LAYOUT = /^(position|top|right|bottom|left|inset|z-index|flex|flex-[a-z-]+|order|align-self|justify-self|place-self|grid-[a-z-]+|margin(-[a-z-]+)?|width|height|min-width|min-height|max-width|max-height|box-sizing|display|visibility)$/;

const files = readdirSync(dir).filter(f => f.endsWith(".js"));
const made = new Map(); // class → Set(package)
const rules = []; // { file, selector, props }
for (const f of files) {
  const src = readFileSync(join(dir, f), "utf8");
  for (const m of src.matchAll(/className[^:=]*[:=]\s*(`[^`]*`|"[^"]*"|'[^']*')/g)) for (const c of m[1].match(/\bcw-[a-z0-9-]+/g) ?? []) (made.get(c) ?? made.set(c, new Set()).get(c)).add(f);
  // Its CSS: what a style element or `<style>` is given (template literals).
  for (const m of src.matchAll(/(?:textContent\s*\+?=\s*|<style>)`?([\s\S]*?)(?:`;|<\/style>)/g)) {
    const css = m[1].replace(/\/\*[\s\S]*?\*\//g, "");
    // Rules at any depth (an @media's too): selector { declarations }.
    for (const r of css.matchAll(/([^{}@;]+)\{([^{}]*)\}/g)) {
      const selector = r[1].trim();
      if (!selector || /^(from|to|\d+%)$/.test(selector)) continue;
      const props = [...r[2].matchAll(/([a-z-]+)\s*:/g)].map(x => x[1]);
      rules.push({ file: f, selector, props });
    }
  }
}
// The SUBJECT of a selector: its last compound (what is styled), without what :not()/:has()/:is() only test.
const subjects = sel =>
  sel
    .split(",")
    .map(part => part.trim().split(/\s*[>+~]\s*|\s+/).at(-1) ?? "")
    .map(c => c.replace(/:(not|has|is|where)\((?:[^()]|\([^()]*\))*\)/g, ""))
    .flatMap(c => c.match(/\.cw-[a-z0-9-]+/g) ?? [])
    .map(c => c.slice(1));

const bad = [];
const lookIn = new Map(); // class → Set(package giving it a look)
for (const r of rules) {
  const look = r.props.some(p => !LAYOUT.test(p));
  for (const c of new Set(subjects(r.selector))) {
    const makers = made.get(c);
    if (!makers) continue; // a class no package makes (a theme part, a base class)
    if (look) (lookIn.get(c) ?? lookIn.set(c, new Set()).get(c)).add(r.file);
    if (r.file === BASE || makers.has(r.file) || !look) continue;
    bad.push(`${r.file}: restyles .${c} (made in ${[...makers].join(", ")}) — "${r.selector.slice(0, 90)}" sets ${r.props.filter(p => !LAYOUT.test(p)).join(", ")}; give the component an option, or only place it`);
  }
}
for (const [c, fs] of lookIn) {
  // Its look from two packages — the theme's part and a component's of the same name, or two components'.
  const own = [...fs].filter(f => f === BASE || made.get(c)?.has(f));
  if (own.length > 1) bad.push(`.${c}: one name, two looks — styled by ${own.join(" and ")}: rename one`);
}
if (bad.length) {
  console.error(`style check: ${bad.length} problem(s)\n  ${bad.join("\n  ")}`);
  process.exit(1);
}
console.log(`style check: ${rules.length} rules in ${files.length} packages — every shared class styled only by its component`);
