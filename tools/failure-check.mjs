// FAILURE CHECK (the build's): A FAILURE IS HANDLED IN ONE PLACE (FOUNDATION.md §4). A package never turns a failure into
// a value of its own: a capability's read is TOTAL (`ctx.total`, its empty value and read state), a best-effort write is
// `ctx.background(what, p)`, an optional feature `ctx.optional(what, p, fallback)` — each reported by its fault's kind
// in the loader. So no package may write its own fallback catch (`.catch(() => v)`, `.then(f, () => v)`), nor the
// retired `ctx.quiet`. (A catch that USES the error — `ctx.failed`, a message shown — is a decision, not a fallback.)
// Exempt: the loader (the machinery itself) and framed apps (`mount(api, …)`: no `ctx`; the frame's API is the
// boundary). Fails the build, naming each site.
//
//   node tools/failure-check.mjs [packages dir]
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const dir = process.argv[2] ?? new URL("../packages", import.meta.url).pathname;
const MACHINERY = new Set(["loader.js"]);
const RULES = [
  [/\bctx\.quiet\b/, "ctx.quiet is retired: ctx.total / ctx.background / ctx.optional"],
  [/\.catch\(\s*(?:\(\s*\)|_\w*)\s*=>/, "a fallback catch: make the read total, or ctx.background / ctx.optional"],
  // A FAULT SHOWN TO A PERSON is said in its kind's words (`ctx.say`, one wording per kind — FOUNDATION §4), never its
  // raw message (a stack's text, a node's words).
  [/(textContent\s*[:=]|\.append\(|\bsay\()[^;]*\b(e|err|error)\??\.message\b/, "a fault's raw message shown: ctx.say(e) (its kind's words)"],
];
// `.then(f, () => v)`: a second argument at the call's own depth that is a fallback (brackets matched; strings skipped).
function thenFallbacks(src) {
  const out = [];
  for (const m of src.matchAll(/\.then\(/g)) {
    let depth = 0;
    let q = null;
    for (let i = m.index + 5; i < src.length; i++) {
      const c = src[i];
      if (q) {
        if (c === "\\") i++;
        else if (c === q) q = null;
        continue;
      }
      if (c === '"' || c === "'" || c === "`") q = c;
      else if ("([{".includes(c)) depth++;
      else if (")]}".includes(c) && --depth === 0) break;
      else if (c === "," && depth === 1 && /^,\s*\(\s*\)\s*=>/.test(src.slice(i, i + 16))) {
        out.push(src.slice(0, m.index).split("\n").length);
        break;
      }
    }
  }
  return out;
}

const bad = [];
for (const f of readdirSync(dir).filter(f => f.endsWith(".js")).sort()) {
  if (MACHINERY.has(f)) continue;
  const src = readFileSync(join(dir, f), "utf8");
  if (/export\s+(async\s+)?function\s+mount\s*\(\s*api\b/.test(src)) continue;
  const lines = src.split("\n");
  for (const n of thenFallbacks(src)) bad.push(`${f}:${n}: a fallback in .then(f, () => v): ctx.background / ctx.optional\n    ${lines[n - 1].trim().slice(0, 160)}`);
  lines.forEach((line, i) => {
    if (/^\s*\/\//.test(line)) return;
    for (const [re, why] of RULES) if (re.test(line)) bad.push(`${f}:${i + 1}: ${why}\n    ${line.trim().slice(0, 160)}`);
  });
}
if (bad.length) {
  console.error(`failure-check: ${bad.length} site(s) decide a failure themselves\n${bad.join("\n")}`);
  process.exit(1);
}
console.log("failure-check: ok");
