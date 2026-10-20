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
// (The page's LAYOUT is the OS's: its header, side and footer — the manifest's `layout`.)
const OS = new Set(["header", "places-side", "footer", "home", "settings", "account", "spaces-app", "space-home", "store", "tag", "contact", "chat", "mail", "wallet-app", "nodes-app", "explorer-app"]);
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
// WHICH VARIABLE holds which host capability in a package: `const [a, b] = await Promise.all(["x", "y"].map(n =>
// ctx.require(n)))` or `([ctx.require("x"), ctx.require("y")])` (over several lines too), `x = await ctx.require("x")`,
// `ctx.require("x").then(x => …)`.
// A require by a KIND's field (`ctx.require(spec.maker)`, `…view`): every name `kinds.js` — the one catalog of makers
// and views — gives that field.
const CATALOG = {};
for (const m of srcOf("kinds").matchAll(/\b(maker|view): "([a-z0-9-]+)"/g)) (CATALOG[m[1]] ??= new Set()).add(m[2]);
const bindingsOf = src => {
  const b = new Map(); // var → cap
  for (const m of src.matchAll(/\[([^\]]+)\]\s*=\s*await Promise\.all\(/g)) {
    let i = m.index + m[0].length;
    for (let d = 1; i < src.length && d > 0; i++) d += src[i] === "(" ? 1 : src[i] === ")" ? -1 : 0;
    const expr = src.slice(m.index + m[0].length, i);
    const lit = /^\s*\[([^\]]*)\]\s*\.map\(/.exec(expr);
    // (A listed array: its elements by position — `ctx.require("x")` or anything else, split at its top level.)
    const elements = () => {
      const body = expr.slice(expr.indexOf("[") + 1, expr.lastIndexOf("]"));
      const out = [];
      let d = 0, at = 0;
      for (let k = 0; k <= body.length; k++) {
        const c = body[k];
        if (c === "(" || c === "[" || c === "{") d++;
        else if (c === ")" || c === "]" || c === "}") d--;
        else if ((c === "," && d === 0) || k === body.length) out.push(body.slice(at, k)), (at = k + 1);
      }
      return out.map(e => /^\s*(?:await\s+)?\(?\s*(?:ctx|app)\.require\(\s*"([a-z0-9-]+)"\s*\)\s*\)?\s*$/.exec(e)?.[1] ?? null);
    };
    const names = lit ? [...lit[1].matchAll(/"([a-z0-9-]+)"/g)].map(x => x[1]) : elements();
    m[1].split(",").map(v => v.trim()).forEach((v, k) => /^\w+$/.test(v) && names[k] && b.set(v, names[k]));
  }
  for (const m of src.matchAll(/(\w+)\s*=\s*await\s+(?:ctx|app)\.require\("([a-z0-9-]+)"\)/g)) b.set(m[1], m[2]);
  for (const m of src.matchAll(/(?:ctx|app)\.require\("([a-z0-9-]+)"\)\.then\(\s*(?:async\s*)?\(?(\w+)/g)) b.set(m[2], m[1]);
  // (By a kind's field: the variable is each name the catalog gives — `var#name`, one binding per name.)
  for (const m of src.matchAll(/(\w+)\s*=\s*await\s+(?:ctx|app)\.require\(\s*[\w.]+\.(maker|view)\s*\)/g)) for (const n of CATALOG[m[2]] ?? []) b.set(`${m[1]}#${n}`, n);
  return b;
};
// AN APP's GRANTS, call by call (10-16): every `capability.method` its code — the app's and each package it loads into
// its frame — calls on the trust core (`HOST`, where the walk stops). A capability required where no call on it can be
// SEEN fails the check (it must be visible to be granted). A handle a granted call returns is the host's to hand out.
const grantsOf = app => {
  const seen = new Set();
  const grants = new Set();
  const walk = [app];
  while (walk.length) {
    const n = walk.pop();
    if (seen.has(n)) continue;
    seen.add(n);
    const src = srcOf(n);
    // IN THE FRAME, A HOST'S ANSWER IS A PROMISE (FOUNDATION §3): a space's roles read as a value there — `r.apps().
    // includes(…)` — throws (10-19 `where`; 10-20 the video page: "(intermediate value)?.apps(...).includes is not a
    // function", from captions). Awaited (`(await r.apps()).includes`) it is the same in the page and in a frame.
    for (const m of src.matchAll(/\b(apps|roster|members|acts|roles|held|lists|invites)\(\)\.(includes|some|filter|map|find|length|every|forEach)\b/g))
      syncReads.add(`packages/${n}.js:${src.slice(0, m.index).split("\n").length}: ${m[1]}() read as a value in an app's frame (${app}) — await it: (await r.${m[1]}()).${m[2]}`);
    const b = bindingsOf(src);
    for (const m of src.matchAll(/(?:ctx|app)\.require\(\s*(?!")([^)]*)\)/g)) {
      const arg = m[1].trim();
      const field = /^[\w.]+\.(maker|view)$/.exec(arg)?.[1];
      if (field) {
        for (const nm of CATALOG[field] ?? []) {
          if (!HOST.has(nm)) {
            walk.push(nm);
            continue;
          }
          // `(await ctx.require(x.maker)).call(` — the call on it.
          const direct = new RegExp(`\\(await (?:ctx|app)\\.require\\(\\s*${arg.replace(/\./g, "\\.")}\\s*\\)\\)\\.(\\w+)`, "g");
          for (const c of src.matchAll(direct)) grants.add(`${nm}.${c[1]}`);
        }
      } else if (!/^(n|name)$/.test(arg)) bad.push(`packages/${n}.js:${src.slice(0, m.index).split("\n").length}: requires by a name the check cannot see (${arg}) — a literal name, or a kind's maker / view`);
    }
    for (const r of new Set(reqs(src))) {
      if (!HOST.has(r)) {
        walk.push(r);
        continue;
      }
      let found = 0;
      for (const m of src.matchAll(new RegExp(`\\(await (?:ctx|app)\\.require\\("${r}"\\)\\)\\.(\\w+)`, "g"))) grants.add(`${r}.${m[1]}`), found++;
      for (const [vn, cap] of b) {
        if (cap !== r) continue;
        const v = vn.split("#")[0];
        for (const m of src.matchAll(new RegExp(`(?<![\\w.$])${v}\\??\\.(\\w+)`, "g"))) grants.add(`${r}.${m[1]}`), found++;
      }
      if (!found) bad.push(`packages/${n}.js: requires "${r}" but no call on it can be seen (bind it as const x = await ctx.require("${r}"), or in a Promise.all list)`);
    }
  }
  // A host module's own FRAME code (its `frame(cap, …)` export, run in the app's frame): its calls on `cap` are the
  // app's too (directory's name element reads `cap.name`, `cap.levelOf`).
  for (const r of new Set([...grants].map(g => g.split(".")[0]))) {
    const src = srcOf(r);
    const m = /export (?:async )?function frame\(\s*(\w+)/.exec(src);
    if (!m) continue;
    let i = src.indexOf("{", m.index);
    const from = i;
    for (let d = 0; i < src.length; i++) if ((d += src[i] === "{" ? 1 : src[i] === "}" ? -1 : 0) === 0) break;
    for (const c of src.slice(from, i).matchAll(new RegExp(`(?<![\\w.$])${m[1]}\\.(\\w+)`, "g"))) grants.add(`${r}.${c[1]}`);
  }
  return [...grants].sort();
};
const syncReads = new Set();
const framedMap = Object.fromEntries([...framedSrc.matchAll(/\\"([a-z0-9-]+)\\": \[([^\]]*)\]/g)].map(m => [m[1], [...m[2].matchAll(/\\"([a-z0-9.A-Z_-]+)\\"/g)].map(x => x[1]).sort()]));
const want = Object.fromEntries(Object.keys(framedMap).map(a => [a, grantsOf(a)]));
for (const x of syncReads) bad.push(x);
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
