// WIKI PAGES, a service: a wiki page as SECTIONS over the one item store (`items`) — no store of its own (ARCHITECTURE
// §7, Wiki). A PAGE is an item of kind `wiki` (its title; `meta.key` the title normalized: unique per wiki). Its TEXT
// is SECTIONS, one per heading: each a `wiki-section` item attached to the page (`items.attach`), one item PER WRITER
// PER SECTION — the writer's latest VERSION of it, edited in place (`items.editItem`: storage keeps the earlier ones,
// signed by their writer, as the item's history). A version names its section (`sid`), its place (`pos`, a number:
// the page reads in that order), the version it was written over (`base`: a hash) and, a conflict resolved, the
// others it settles (`merges`). Its hash is computed by every reader from what it holds — never trusted as written.
//
// THE MERGE, the same for every reader: a section's CURRENT version is its HEAD — an editor's version no other editor's
// version was written over (`base`) or settled (`merges`). Two editors of different sections never meet; two editing
// one section from the same version make TWO heads: a CONFLICT, both kept and shown until an editor settles it
// (nothing is lost, nothing is chosen silently). A version by someone who may not edit (as the place's rule was when
// they wrote it) is a SUGGESTION: pending until an editor ACCEPTS it (writes it, settling it) or DECLINES it (their
// review item names it).
//
// WHO EDITS: a PERSONAL wiki (your profile, your private items, your friends' or followers' circle): only its owner.
// A SPACE's wiki: whoever the space's `edit` policy on the `wiki` domain allows (`roles.mayWrite`, inherited as every
// policy); the rest suggest. THE GLOBAL COMMONS (one ownerless wiki: anyone edits behind a reputation gate, disputes by
// Vote) is NOT BUILT: `of()` refuses that scope by name — it comes after Vote, as its own `scope` here.
//
//   const W = await ctx.require("wiki-pages");
//   const wiki = await W.of(at)          // `at`: `where.of(…)` — yours, a person's, a space's
//   await wiki.pages()                   // [{ ref, title, key, by, at, sections, pending, conflicts, text, … }]
//   await wiki.page(key | ref)           // one page, its versions read fresh (suggestions from outside included)
//   wiki.mayEdit()  wiki.mayWrite()      // this person edits here / may write at all (else read only)
//   await wiki.create(title, text, { audience, write })        // its ref
//   await wiki.apply(page, ops)          // ops from planWhole / planSection (an editor's edits; else suggestions)
//   await wiki.accept(page, s)  await wiki.decline(page, s)    // a pending suggestion
//   await wiki.history(page)             // every version of every section, newest first
//   await wiki.copyToMine(page)          // a copy in your personal wiki, its lineage kept: its ref
//   await wiki.propose(myPage)           // your copy's changes back where it came from
// Pure parts (exported, no page needed: `tools/wiki-pages.test.mjs`): normTitle, titleProblem, parseLinks,
// unescapeLinks, linkify, splitSections, joinSections, versionHash, resolve, planWhole, planSection, planPropose,
// backlinks, tree.

export const TITLE_MAX = 200;
export const KIND = "wiki";
export const SECTION = "wiki-section";

// ── TITLES ──────────────────────────────────────────────────────────────────────────────────────────────────────────
// A title as shown: its spaces (and underscores, as in a link) made single, its edges trimmed.
export const cleanTitle = t => String(t ?? "").normalize("NFC").replace(/[_\s]+/g, " ").trim();
// A title as a KEY (unique per wiki): cleaned, compatibility-folded, lower case — "Foo_bar", "foo  BAR" are one page.
export const normTitle = t => cleanTitle(String(t ?? "").normalize("NFKC")).toLowerCase();
// Why a title cannot be one (null: it can).
export function titleProblem(t) {
  const c = cleanTitle(t);
  if (!c) return "a page needs a title";
  if (c.length > TITLE_MAX) return `a title of at most ${TITLE_MAX} characters`;
  if (/[[\]|#{}<>\\]/.test(c)) return "a title has none of [ ] | # { } < > \\";
  if (/^\/|\/$|\/\s*\//.test(c)) return "a title's “/” parts are not empty";
  return null;
}

// ── LINKS ───────────────────────────────────────────────────────────────────────────────────────────────────────────
// Where CODE is (fenced blocks, inline `spans`): no link is read there.
function codeMask(text) {
  const mask = new Uint8Array(text.length);
  let i = 0;
  let fence = null;
  for (const line of text.split("\n")) {
    const f = /^\s{0,3}(`{3,}|~{3,})/.exec(line);
    if (fence) {
      mask.fill(1, i, i + line.length);
      if (f && f[1][0] === fence[0] && f[1].length >= fence.length) fence = null;
    } else if (f) {
      fence = f[1];
      mask.fill(1, i, i + line.length);
    }
    i += line.length + 1;
  }
  for (const m of text.matchAll(/(`+)[^`\n]*?\1/g)) mask.fill(1, m.index, m.index + m[0].length);
  return mask;
}
const LINK = /\[\[([^[\]\n]+?)\]\]/g;
// The [[Title]] and [[Title|label]] links in a text, outside code: [{ title, label, key, index, raw }].
export function parseLinks(text) {
  text = String(text ?? "");
  const mask = codeMask(text);
  const out = [];
  for (const m of text.matchAll(LINK)) {
    if (mask[m.index]) continue;
    const bar = m[1].indexOf("|");
    const title = cleanTitle(bar < 0 ? m[1] : m[1].slice(0, bar));
    if (titleProblem(title)) continue;
    const label = bar < 0 ? title : m[1].slice(bar + 1).trim() || title;
    out.push({ title, label, key: normTitle(title), index: m.index, raw: m[0] });
  }
  return out;
}
// A link written in RICH text comes back escaped (`\[\[Title\]\]`: `md-editor` keeps brackets as text): made a link again.
export const unescapeLinks = text =>
  String(text ?? "").replace(/\\\[\\\[((?:\\.|[^\\\n])+?)\\\]\\\]/g, (_, inner) => `[[${inner.replace(/\\(.)/g, "$1")}]]`);
// The text with each link written as Markdown (`markdown` draws it): `href(title, key)` its address.
export function linkify(text, href) {
  text = String(text ?? "");
  let out = "";
  let at = 0;
  for (const l of parseLinks(text)) {
    const label = l.label.replace(/[[\]]/g, "");
    // Nothing in the address that Markdown would read as markup (emphasis, strike, parentheses, spaces).
    const to = String(href(l.title, l.key)).replace(/[()\s*_~!'^]/g, c => `%${c.charCodeAt(0).toString(16).toUpperCase().padStart(2, "0")}`);
    out += text.slice(at, l.index) + `[${label}](${to})`;
    at = l.index + l.raw.length;
  }
  return out + text.slice(at);
}

// ── SECTIONS ────────────────────────────────────────────────────────────────────────────────────────────────────────
const HEADING = /^\s{0,3}(#{1,6})[ \t]+(.+?)[ \t]*#*[ \t]*$/;
// A text cut at its headings (outside code): [{ heading, level, body }] — the text before the first heading its LEAD
// (heading null), kept only when it holds something. `body` is the section's text whole, its heading line included.
export function splitSections(text) {
  const lines = String(text ?? "").replace(/\r\n?/g, "\n").split("\n");
  const out = [];
  let cur = { heading: null, level: 0, lines: [] };
  let fence = null;
  for (const line of lines) {
    const f = /^\s{0,3}(`{3,}|~{3,})/.exec(line);
    if (fence) {
      if (f && f[1][0] === fence[0] && f[1].length >= fence.length) fence = null;
    } else if (f) fence = f[1];
    else {
      const h = HEADING.exec(line);
      if (h) {
        out.push(cur);
        cur = { heading: h[2].trim(), level: h[1].length, lines: [] };
      }
    }
    cur.lines.push(line);
  }
  out.push(cur);
  return out
    .map(s => ({ heading: s.heading, level: s.level, body: s.lines.join("\n").replace(/^\s*\n/, "").trimEnd() }))
    .filter(s => s.heading !== null || s.body.trim());
}
export const joinSections = list => list.map(s => String(s.body ?? "").trim()).filter(Boolean).join("\n\n");
const headingKey = h => (h == null ? null : normTitle(h));

// ── VERSIONS ────────────────────────────────────────────────────────────────────────────────────────────────────────
// A small, stable hash (cyrb53, twice): names a version by what it holds — not a signature (storage signs the row).
function cyrb53(str, seed) {
  let h1 = 0xdeadbeef ^ seed;
  let h2 = 0x41c6ce57 ^ seed;
  for (let i = 0; i < str.length; i++) {
    const c = str.charCodeAt(i);
    h1 = Math.imul(h1 ^ c, 2654435761);
    h2 = Math.imul(h2 ^ c, 1597334677);
  }
  h1 = Math.imul(h1 ^ (h1 >>> 16), 2246822507) ^ Math.imul(h2 ^ (h2 >>> 13), 3266489909);
  h2 = Math.imul(h2 ^ (h2 >>> 16), 2246822507) ^ Math.imul(h1 ^ (h1 >>> 13), 3266489909);
  return (4294967296 * (2097151 & h2) + (h1 >>> 0)).toString(16).padStart(14, "0");
}
// A version's hash: its section, what it was written over (so the same text written again later is a NEW version, and
// no version can name one written after it), its place, deleted or not, its text.
export function versionHash(v) {
  const s = JSON.stringify([String(v.sid ?? ""), v.base ?? null, [...(v.merges ?? [])].map(String).sort(), Number(v.pos) || 0, !!v.gone, String(v.body ?? "")]);
  return cyrb53(s, 0) + cyrb53(s, 1);
}
const SID = /^[0-9a-z]{4,32}$/;
const HASH = /^[0-9a-f]{28}$/;
// A section item (`items`' shape) read as a VERSION, or null (not one). `at`: when this version was written.
export function versionOf(it) {
  const m = it?.meta ?? {};
  const review = Array.isArray(m.declined);
  if (!review && !SID.test(String(m.sid ?? ""))) return null;
  const v = {
    ref: it.ref ?? null,
    by: it.editor ?? it.by ?? null,
    at: Number(it.edited) || Number(it.at) || 0,
    sid: review ? null : String(m.sid),
    base: HASH.test(String(m.base ?? "")) ? String(m.base) : null,
    merges: Array.isArray(m.merges) ? m.merges.filter(x => HASH.test(String(x))).map(String).slice(0, 32) : [],
    pos: Number.isFinite(Number(m.pos)) ? Number(m.pos) : 0,
    gone: !!m.gone,
    suggest: !!m.suggest,
    review,
    declined: review ? m.declined.filter(x => HASH.test(String(x))).map(String) : [],
    body: String(it.body ?? ""),
  };
  v.h = review ? null : versionHash(v);
  v.heading = review ? null : (splitSections(v.body)[0]?.heading ?? null);
  return v;
}

// THE PAGE NOW, from every version read: its sections in order — each its HEAD, or, two or more heads, a CONFLICT —
// and the SUGGESTIONS pending. `mayEdit(v)`: whether the writer of version v could edit when they wrote it.
//   → { sections: [{ sid, pos, heads, conflict, gone, h, body, heading, by, at }], pending: [v + { fresh }], edits }
export function resolve(versions, mayEdit = () => true) {
  const all = versions.filter(Boolean);
  const edits = all.filter(v => !v.review && !v.suggest && mayEdit(v));
  const suggestions = all.filter(v => !v.review && (v.suggest || !mayEdit(v)));
  // What an editor wrote over, or settled: no longer a head.
  const over = new Set(edits.flatMap(v => [v.base, ...v.merges]).filter(Boolean));
  const bySid = new Map();
  for (const v of edits) (bySid.get(v.sid) ?? bySid.set(v.sid, []).get(v.sid)).push(v);
  const sections = [];
  for (const [sid, vs] of bySid) {
    const seen = new Set();
    let heads = vs
      .filter(v => !over.has(v.h))
      .sort((a, b) => a.at - b.at || (a.h < b.h ? -1 : 1))
      .filter(v => !seen.has(v.h) && seen.add(v.h));
    // Every version named by another (only a cycle does that): the newest stands — never a section lost.
    if (!heads.length) heads = [vs.reduce((a, b) => (b.at > a.at ? b : a))];
    const top = heads.at(-1);
    sections.push({ sid, pos: top.pos, heads, conflict: heads.length > 1, gone: heads.every(v => v.gone), h: top.h, body: top.body, heading: top.heading, by: top.by, at: top.at });
  }
  sections.sort((a, b) => a.pos - b.pos || (a.sid < b.sid ? -1 : 1));
  const declined = new Set(all.filter(v => v.review && mayEdit(v)).flatMap(v => v.declined));
  const headOf = new Map(sections.map(s => [s.sid, s]));
  const pending = suggestions
    .filter(v => !over.has(v.h) && !declined.has(v.h))
    // FRESH: written over the section as it is now (or a new section); else over an older version of it.
    .map(v => ({ ...v, fresh: v.base ? !!headOf.get(v.sid)?.heads.some(x => x.h === v.base) : !headOf.has(v.sid) }))
    .sort((a, b) => a.at - b.at);
  return { sections, pending, edits };
}
// The page's text now (deleted sections and the unsettled heads beyond the newest left out).
export const textOf = sections => joinSections(sections.filter(s => !s.gone));

// WHAT A WRITER'S NEXT VERSION SETTLES: its op's own, and what the writer's previous version of the section settled
// or was written over (`prev`) — that version leaves their item when the new one is written, and with it the record
// that it settled them; carried forward, a settled conflict stays settled. Kept: every one still held by another
// writer's version (`present`), and the newest few beyond (one not read yet).
export function carryMerges(prev, op, present = new Set(), keep = 32) {
  const carried = prev ? [prev.base, ...(prev.merges ?? [])].filter(Boolean) : [];
  const out = [...new Set([...(op.merges ?? []), ...carried.filter((h, i) => present.has(h) || i < keep)])].filter(h => h !== op.base);
  return out.slice(0, 64);
}

// ── EDITS AS OPS ────────────────────────────────────────────────────────────────────────────────────────────────────
// An op: { op: "edit" | "new" | "gone", sid, base, merges?, body, pos } — one version to write.
const between = (a, b) => (a == null && b == null ? 1 : a == null ? b - 1 : b == null ? a + 1 : (a + b) / 2);
const randomSid = () => [...crypto.getRandomValues(new Uint8Array(8))].map(x => x.toString(36).padStart(2, "0")).join("").slice(0, 12);
// The longest run of positions kept in order (indices into `seq` whose values only rise).
function increasing(seq) {
  const idx = seq.map((v, i) => i).filter(i => seq[i] != null);
  const best = new Map();
  const prev = new Map();
  let end = null;
  for (const i of idx) {
    best.set(i, 1);
    for (const j of idx) {
      if (j >= i) break;
      if (seq[j] < seq[i] && best.get(j) + 1 > best.get(i)) (best.set(i, best.get(j) + 1), prev.set(i, j));
    }
    if (end == null || best.get(i) > best.get(end)) end = i;
  }
  const keep = new Set();
  for (let i = end; i != null; i = prev.get(i)) keep.add(i);
  return keep;
}
// THE WHOLE PAGE EDITED: the new text's sections matched to the page's (`sections`: as `resolve` gives them, none in
// conflict) — the same text first, then the same heading, then what stands in the same place — and only what changed
// written: a changed section a new version of it, a new one a new section, a missing one deleted. Sections keep their
// places unless moved, so an edit here never touches a section another editor is changing.
export function planWhole(sections, text, { newSid = randomSid } = {}) {
  const old = sections.filter(s => !s.gone);
  const neu = splitSections(text);
  const match = new Array(neu.length).fill(-1);
  const used = new Set();
  const take = (i, j) => ((match[i] = j), used.add(j));
  neu.forEach((n, i) => {
    const j = old.findIndex((o, k) => !used.has(k) && o.body.trim() === n.body.trim());
    if (j >= 0) take(i, j);
  });
  neu.forEach((n, i) => {
    if (match[i] >= 0 || n.heading == null) return;
    const j = old.findIndex((o, k) => !used.has(k) && o.heading != null && headingKey(o.heading) === headingKey(n.heading));
    if (j >= 0) take(i, j);
  });
  // What is left, GAP by gap (between two sections matched above): as many new as old there, paired in order; else
  // each paired with the old one it most resembles (its heading's or its text's words), never one with nothing alike.
  const words = t => new Set(String(t ?? "").toLowerCase().match(/[\p{L}\p{N}]+/gu) ?? []);
  const alike = (a, b) => {
    const j = (x, y) => (x.size || y.size ? [...x].filter(w => y.has(w)).length / new Set([...x, ...y]).size : 0);
    return Math.max(j(words(a.heading), words(b.heading)), j(words(a.body), words(b.body)));
  };
  for (let i = 0; i < neu.length; ) {
    if (match[i] >= 0) {
      i++;
      continue;
    }
    let end = i;
    while (end < neu.length && match[end] < 0) end++;
    const lo = Math.max(-1, ...match.slice(0, i).filter(j => j >= 0));
    const after = match.slice(end).filter(j => j >= 0);
    const hi = after.length ? Math.min(...after) : old.length;
    const N = [...Array(end - i).keys()].map(k => i + k);
    const O = [...Array(Math.max(0, hi - lo - 1)).keys()].map(k => lo + 1 + k).filter(j => !used.has(j));
    if (N.length === O.length) N.forEach((ni, k) => take(ni, O[k]));
    else {
      const pairs = N.flatMap(ni => O.map(j => [alike(neu[ni], old[j]), ni, j])).filter(([a]) => a >= 0.3).sort((x, y) => y[0] - x[0]);
      for (const [, ni, j] of pairs) if (match[ni] < 0 && !used.has(j)) take(ni, j);
    }
    i = end;
  }
  const keep = increasing(match.map(j => (j >= 0 ? j : null)));
  const ops = [];
  let prev = null;
  neu.forEach((n, i) => {
    const o = match[i] >= 0 ? old[match[i]] : null;
    let pos;
    if (o && keep.has(i)) pos = o.pos;
    else {
      let next = null;
      for (let k = i + 1; k < neu.length; k++) if (keep.has(k)) ((next = old[match[k]].pos), (k = neu.length));
      pos = between(prev, next);
    }
    prev = pos;
    if (!o) ops.push({ op: "new", sid: newSid(), base: null, body: n.body, pos });
    else if (o.body.trim() !== n.body.trim() || o.pos !== pos) ops.push({ op: "edit", sid: o.sid, base: o.h, body: n.body, pos });
  });
  old.forEach((o, j) => !used.has(j) && ops.push({ op: "gone", sid: o.sid, base: o.h, body: "", pos: o.pos }));
  return ops;
}
// ONE SECTION edited (or a conflict settled: `heads` the versions it settles, `base` the one it is written over): its
// text may grow headings — the first part is this section, the rest new sections right after it; empty, it is deleted.
export function planSection(sections, sid, text, { newSid = randomSid, base = null, merges = [] } = {}) {
  const live = sections.filter(s => !s.gone || s.sid === sid);
  const i = live.findIndex(s => s.sid === sid);
  const cur = live[i] ?? null;
  const b = base ?? cur?.h ?? null;
  const pos = cur?.pos ?? between(live.at(-1)?.pos ?? null, null);
  const ms = merges.filter(h => h && h !== b);
  const parts = splitSections(text);
  if (!parts.length) return cur ? [{ op: "gone", sid, base: b, ...(ms.length ? { merges: ms } : {}), body: "", pos }] : [];
  const ops = [];
  const same = cur && !cur.gone && !cur.conflict && cur.body.trim() === parts[0].body.trim();
  if (!same || ms.length) ops.push({ op: cur ? "edit" : "new", sid, base: b, ...(ms.length ? { merges: ms } : {}), body: parts[0].body, pos });
  let prev = pos;
  const next = live[i + 1]?.pos ?? null;
  for (const p of parts.slice(1)) {
    prev = between(prev, next);
    ops.push({ op: "new", sid: newSid(), base: null, body: p.body, pos: prev });
  }
  return ops;
}
// A COPY's changes, as ops on where it came from (`origin`: its sections now; `heads`: sid → the version copied):
// each changed section written over the version that was copied — so a section changed there since is a conflict (or
// a suggestion on an older version), never overwritten; a section added here, new there; one deleted here, deleted.
export function planPropose(mine, origin, heads = {}) {
  const there = new Map(origin.map(s => [s.sid, s]));
  const here = new Set(mine.filter(s => !s.gone).map(s => s.sid));
  const ops = [];
  for (const s of mine) {
    if (s.gone) continue;
    const o = there.get(s.sid);
    const base = heads[s.sid] ?? o?.h ?? null;
    if (o && !o.gone && !o.conflict && o.body.trim() === s.body.trim()) continue;
    ops.push(o || base ? { op: "edit", sid: s.sid, base, body: s.body, pos: s.pos } : { op: "new", sid: s.sid, base: null, body: s.body, pos: s.pos });
  }
  for (const sid of Object.keys(heads)) {
    const o = there.get(sid);
    if (!here.has(sid) && o && !o.gone) ops.push({ op: "gone", sid, base: heads[sid], body: "", pos: o.pos });
  }
  return ops;
}

// ── AROUND A PAGE ───────────────────────────────────────────────────────────────────────────────────────────────────
// WHAT LINKS HERE: the pages (`[{ key, text, … }]`) whose text links to `key`.
export const backlinks = (pages, key) => pages.filter(p => p.key !== key && parseLinks(p.text).some(l => l.key === key));
// THE INDEX as a TREE: titles cut at "/" (`Recipes/Bread`) — [{ name, path, page, children }], each level by name.
export function tree(pages) {
  const root = { children: new Map() };
  for (const p of pages) {
    let at = root;
    const parts = cleanTitle(p.title).split("/").map(x => x.trim());
    parts.forEach((name, i) => {
      const k = normTitle(name);
      if (!at.children.has(k)) at.children.set(k, { name, path: parts.slice(0, i + 1).join("/"), page: null, children: new Map() });
      at = at.children.get(k);
      if (i === parts.length - 1) at.page = p;
    });
  }
  const out = node => [...node.children.values()].sort((a, b) => a.name.localeCompare(b.name)).map(n => ({ name: n.name, path: n.path, page: n.page, children: out(n) }));
  return out(root);
}

// ── THE SERVICE ─────────────────────────────────────────────────────────────────────────────────────────────────────
// IN AN APP's FRAME (FOUNDATION §3, "THE FRAME RUNTIME"): the pure parts beside the host's proxy — the pages, their
// versions and who may edit them are read and decided in the host.
export function frame() {
  return { TITLE_MAX, KIND, SECTION, cleanTitle, normTitle, titleProblem, parseLinks, unescapeLinks, linkify, splitSections, joinSections, versionHash, versionOf, resolve, textOf, carryMerges, planWhole, planSection, planPropose, backlinks, tree };
}

export async function start(ctx) {
  const [items, space, roles] = await Promise.all(["items", "space", "roles"].map(n => ctx.require(n)));
  const me = async () => space.me.strict();
  const placeOf = ref => String(ref).slice(0, String(ref).lastIndexOf("/"));
  // The page a section item is attached to (a space's: its id there; a person's: its whole reference).
  const pageRefOf = it => (String(it.in ?? "").includes("/") ? it.in : it.in ? `${placeOf(it.ref)}/${it.in}` : null);
  // A person's CIRCLES this reader is in (theirs: yours, your friends' and followers'): part of their personal wiki.
  const circlesOf = async did => (await space.mine.strict()).filter(s => s.circle && space.madeBy(s, did));

  // THE SCOPE of a place (`where`'s): a person's own wiki, or a space's. The COMMONS (the global, ownerless wiki) is
  // its own scope, later (after Vote): refused here by name.
  async function of(at) {
    if (at?.discover || at?.who === "commons") throw new Error("the global wiki (an ownerless commons) is not built yet: it comes after Vote");
    const self = await me();
    const sp = at?.space ?? null;
    // A space that is one of a person's CIRCLES is their personal wiki, not a space's.
    const circle = sp?.circle ? sp : null;
    const owner = sp && !circle ? null : (circle?.governance?.owner ?? at?.person ?? self);
    const r = sp && !circle ? await roles.of.strict(sp) : null;
    if (r) await r.settled;
    const rule = { kind: KIND, by: null, meta: {} };
    // May the writer of version v edit (as the rule was when they wrote it)?
    const editorAt = (by, at_) => !!by && (owner ? by === owner : roles.mayWrite({ action: "edit", item: rule, writer: by, r, at: at_ || Infinity }) === true);
    const mayEdit = () => editorAt(self, Infinity);
    // May write at all: the owner (yours), a space's member who may post or comment on its wiki, another person's
    // reader where their page lets them comment (a suggestion is kept as a comment is).
    const mayWrite = () => mayEdit() || (r ? r.allowsIn("comment", self, KIND) : owner !== self);
    const read = async () => {
      const kinds = [KIND, SECTION];
      if (!owner) return items.inPlaces.strict({ spaces: [sp] }, kinds, { withVotes: false });
      return items.inPlaces.strict({ spaces: await circlesOf(owner), people: [owner] }, kinds, { withVotes: false });
    };
    // A page's ONE container (the same title in two places of one wiki — your public and your private — stays two
    // pages: their text is never merged across who reads it).
    const group = list => {
      const pages = new Map();
      for (const it of list.filter(x => x.kind === KIND).sort((a, b) => a.at - b.at || (a.ref < b.ref ? -1 : 1))) {
        const key = it.meta?.key || normTitle(it.title);
        const k = `${placeOf(it.ref)}|${key}`;
        if (!pages.has(k)) pages.set(k, { ...it, key, refs: [it.ref] });
        else pages.get(k).refs.push(it.ref);
      }
      return [...pages.values()];
    };
    const shapePage = (p, sectionItems) => {
      const refs = new Set(p.refs);
      const vs = sectionItems.filter(it => it.kind === SECTION && refs.has(pageRefOf(it))).map(versionOf);
      const { sections, pending } = resolve(vs, v => editorAt(v.by, v.at));
      const live = sections.filter(s => !s.gone);
      return { ...p, title: cleanTitle(p.title), versions: vs.filter(Boolean), sections, pending, conflicts: live.filter(s => s.conflict).length, text: textOf(sections), updated: Math.max(p.at, ...vs.filter(Boolean).map(v => v.at)) };
    };
    // What the place holds (pages and sections), read once per `pages()`.
    let held = null;
    const pages = async ({ fresh = true } = {}) => {
      if (!held || fresh) held = read().then(list => ({ list, pages: group(list).map(p => shapePage(p, list)) }));
      return (await held).pages;
    };
    // ONE page by its key or a reference to it, its versions read fresh where they are (suggestions kept outside it).
    async function page(keyOrRef) {
      const all = await pages();
      const isRef = /^(space:|did:)/.test(String(keyOrRef));
      const p = isRef ? all.find(x => x.refs.includes(keyOrRef)) : all.find(x => x.key === normTitle(keyOrRef));
      if (!p) return null;
      const more = (await Promise.all(p.refs.map(ref => items.attached(ref, SECTION)))).flat();
      const list = [...(await held).list.filter(x => x.kind === SECTION), ...more];
      const seen = new Set();
      return shapePage(p, list.filter(x => !seen.has(x.ref) && seen.add(x.ref)));
    }
    // WRITTEN: each op a version in this person's own item for its section (made the first time) — an editor's edits;
    // anyone else's, suggestions. What its previous version there settled is carried on (`carryMerges`). Returns
    // what was written: [{ sid, h }].
    async function apply(p, ops, { suggest = !mayEdit() } = {}) {
      if (!mayWrite()) throw new Error("you may not write here");
      const self_ = await me();
      // This person's own item per section (its latest version), on this page.
      const mine = new Map(p.versions.filter(v => v.by === self_ && !v.review && v.ref).map(v => [v.sid, v]));
      const present = new Set(p.versions.filter(v => v.by !== self_ && v.h).map(v => v.h));
      const written = [];
      for (const o of ops) {
        const had = mine.get(o.sid);
        const merges = carryMerges(had, o, present);
        const body = o.op === "gone" ? "" : unescapeLinks(o.body);
        const meta = { sid: o.sid, pos: o.pos, ...(o.base ? { base: o.base } : {}), ...(merges.length ? { merges } : {}), ...(o.op === "gone" ? { gone: true } : {}), ...(suggest ? { suggest: true } : {}) };
        const v = { sid: o.sid, base: o.base ?? null, merges, pos: o.pos, gone: o.op === "gone", body };
        let ref = had?.ref;
        if (ref) await items.editItem(ref, body, { meta });
        else ref = await items.attach(p.ref, SECTION, body, { meta });
        mine.set(o.sid, { ...v, ref, by: self_ });
        written.push({ sid: o.sid, h: versionHash(v) });
      }
      return written;
    }
    async function create(title, text, { audience = null, write = null, lineage = null, sections = null } = {}) {
      const why = titleProblem(title);
      if (why) throw new Error(why);
      if (!mayEdit()) throw new Error(owner ? "only its owner writes in this wiki" : "this space's rules do not let you write pages");
      const key = normTitle(title);
      if ((await pages()).some(x => x.key === key)) throw new Error(`there is a page “${cleanTitle(title)}” here already`);
      const aud = audience ?? (sp && !circle ? ((await items.publicIn(sp, KIND)) ? "public" : "members") : "private");
      const ref = await items.submit({ board: sp && !circle ? sp.id : (circle?.id ?? null), title: cleanTitle(title), body: "", kind: KIND, audience: circle ? "members" : aud, meta: { key, ...(lineage ? { lineage } : {}) }, write });
      const p = { ref, refs: [ref], key, title: cleanTitle(title), versions: [], sections: [] };
      await apply(p, sections ?? planWhole([], text), { suggest: false });
      return ref;
    }
    async function accept(p, s) {
      if (!mayEdit()) throw new Error("only an editor accepts a suggestion");
      const cur = p.sections.find(x => x.sid === s.sid);
      const base = cur?.h ?? null;
      // Settled with it: the suggestion itself (and, on a conflict, every head).
      const merges = [s.h, ...(cur?.heads ?? []).map(x => x.h).filter(h => h !== base)];
      await apply(p, [{ op: s.gone ? "gone" : cur ? "edit" : "new", sid: s.sid, base, merges, body: s.body, pos: s.pos }], { suggest: false });
    }
    async function decline(p, s) {
      if (!mayEdit()) throw new Error("only an editor declines a suggestion");
      const self_ = await me();
      const review = p.versions.find(v => v.review && v.by === self_ && v.ref);
      const declined = [...new Set([...(review?.declined ?? []), s.h])].slice(-500);
      if (review) await items.editItem(review.ref, "", { meta: { declined } });
      else await items.attach(p.ref, SECTION, "", { meta: { declined } });
    }
    // HISTORY: every version each section item had (`items.history`: storage's, every writer's feed) and each as it
    // is now — who wrote it, when, which section — newest first.
    async function history(p) {
      const refs = [...new Set(p.versions.filter(v => v.ref && !v.review).map(v => v.ref))];
      const olds = (await Promise.all(refs.map(ref => ctx.optional("wiki-pages: items.history", items.history(ref).then(vs => vs.map(x => versionOf({ ...x, ref }))), () => [])))).flat();
      const seen = new Set();
      return [...p.versions.filter(v => !v.review), ...olds]
        .filter(v => v && !seen.has(`${v.ref}|${v.h}|${v.at}`) && seen.add(`${v.ref}|${v.h}|${v.at}`))
        .map(v => ({ ...v, editor: editorAt(v.by, v.at) && !v.suggest }))
        .sort((a, b) => b.at - a.at);
    }
    // RESTORE one version of a section: written as the section's next version (over its head; a conflict settled).
    const restore = (p, v) => {
      const cur = p.sections.find(x => x.sid === v.sid);
      const heads = cur?.heads ?? [];
      return apply(p, [{ op: v.gone ? "gone" : cur ? "edit" : "new", sid: v.sid, base: cur?.h ?? null, merges: heads.map(x => x.h).filter(h => h !== cur?.h), body: v.body, pos: v.pos }]);
    };
    // (`may`: what this person may do here, decided as the place is read — data, as an app's frame receives it.)
    return { scope: owner ? "personal" : "space", owner, space: sp && !circle ? sp : null, may: { edit: mayEdit(), write: mayWrite() }, mayEdit, mayWrite, pages, page, apply, create, accept, decline, history, restore, editorAt };
  }

  // COPY to my wiki: the page's sections now, under the same section ids (so a proposal back names them), its LINEAGE
  // — where it came from and the version of each section copied.
  async function copyToMine(p, { audience = "private" } = {}) {
    const mine = await of({ who: "mine" });
    const live = p.sections.filter(s => !s.gone);
    const lineage = { from: p.ref, title: p.title, ...(items.spaceIdOf(p.ref) ? { space: items.spaceIdOf(p.ref) } : { by: placeOf(p.ref) }), at: Date.now(), heads: Object.fromEntries(live.map(s => [s.sid, s.h])) };
    const sections = live.map(s => ({ op: "new", sid: s.sid, base: null, body: s.body, pos: s.pos }));
    return mine.create(p.title, "", { audience, lineage, sections });
  }
  // PROPOSE: your copy's changes back to its origin — as edits where you may edit there, else as suggestions. The
  // lineage then names what was written (your copy and its origin agree again).
  async function propose(myPage) {
    const lin = myPage.meta?.lineage;
    if (!lin?.from) throw new Error("this page was not copied from anywhere");
    const sid = items.spaceIdOf(lin.from);
    const sp = sid ? ((await space.mine.strict()).find(s => s.id === sid) ?? null) : null;
    if (sid && !sp) throw new Error("you are not in the space it came from");
    const there = await of(sp ? { who: "space", space: sp } : { who: "person", person: placeOf(lin.from) });
    const origin = await there.page(lin.from);
    if (!origin) throw new Error("its original page is not there (removed, or not readable to you)");
    const ops = planPropose(myPage.sections, origin.sections, lin.heads ?? {});
    const suggest = !there.mayEdit();
    if (!ops.length) return { written: 0, suggest };
    const written = await there.apply(origin, ops);
    // EDITED there: what was written is what each section is now based on (your copy and its origin agree again). A
    // SUGGESTION changes nothing there until accepted: the lineage stays (a later proposal replaces the suggestion).
    const heads = { ...(lin.heads ?? {}) };
    if (!suggest) for (const w of written) heads[w.sid] = w.h;
    await items.editItem(myPage.ref, "", { meta: { ...myPage.meta, lineage: { ...lin, heads, proposed: Date.now() } } });
    return { written: ops.length, suggest };
  }
  return { of, copyToMine, propose, normTitle, cleanTitle, titleProblem, parseLinks, linkify, splitSections, joinSections, planWhole, planSection, backlinks, tree };
}
