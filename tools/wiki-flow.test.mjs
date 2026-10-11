// THE WIKI's service paths (`packages/wiki-pages.js` `start`) over an IN-MEMORY items / roles / space — create, edit,
// a conflict and its settling, a suggestion accepted and one declined, history and restore, a whole-page edit, copy
// to your wiki and propose back (written as an editor, suggested as not one), the commons refused. No node, no page:
//   node --test tools/wiki-flow.test.mjs        (`WIKI_PAGES_JS`: another copy of the file, as the pure tests)
// What it does not show: the real `items` (storage, feeds, pointers) — the fakes keep one row per item, no network.
import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";
const src = process.env.WIKI_PAGES_JS ?? new URL("../packages/wiki-pages.js", import.meta.url).href;
const W = await import(src.startsWith("file:") ? src : pathToFileURL(src).href);

let me = "did:craftec:A";
let clock = 1000;
const now = () => ++clock;
const store = new Map(); // ref -> item
const hist = new Map(); // ref -> [old versions]
let n = 0;
const nid = () => `t${String(++n).padStart(12, "0")}`;
const SP = { id: "f".repeat(64), name: "Club", governance: { owner: "did:craftec:A" } };
const placeOf = ref => ref.slice(0, ref.lastIndexOf("/"));
const editors = new Set(["did:craftec:A", "did:craftec:B"]);
const items = {
  spaceIdOf: x => (String(x).startsWith("space:") ? String(x).slice(6, String(x).indexOf("/")) : null),
  async submit({ board, title, body, kind, meta }) {
    const id = nid();
    const ref = board ? `space:${board}/${id}` : `${me}/${id}`;
    store.set(ref, { ref, id, kind, title, body, by: me, at: now(), edited: 0, meta, in: null });
    return ref;
  },
  async attach(post, kind, body, { meta }) {
    const id = nid();
    const inSpace = post.startsWith("space:");
    const ref = inSpace ? `${placeOf(post)}/${id}` : `${me}/${id}`;
    store.set(ref, { ref, id, kind, body, by: me, at: now(), edited: 0, meta, in: inSpace ? post.slice(post.lastIndexOf("/") + 1) : post });
    return ref;
  },
  async editItem(ref, body, { meta, title }) {
    const it = store.get(ref);
    assert.equal(it.by, me, "only its author edits");
    (hist.get(ref) ?? hist.set(ref, []).get(ref)).push({ ...it });
    Object.assign(it, { body, meta: meta ?? it.meta, edited: now(), ...(title !== undefined ? { title } : {}) });
  },
  async inPlaces({ spaces = [], people = [] }, kinds) {
    return [...store.values()].filter(it => kinds.includes(it.kind) && (spaces.some(s => it.ref.startsWith(`space:${s.id}/`)) || people.some(d => it.ref.startsWith(`${d}/`)))).map(x => ({ ...x }));
  },
  async attached(ref, kind) {
    return [...store.values()].filter(it => it.kind === kind && (it.in === ref || `${placeOf(it.ref)}/${it.in}` === ref)).map(x => ({ ...x }));
  },
  async history(ref) {
    return (hist.get(ref) ?? []).map(x => ({ ...x }));
  },
  async publicIn() {
    return false;
  },
};
const space = { account: async () => ({ id: me }), mine: async () => [SP] };
const roles = {
  of: async () => ({ settled: Promise.resolve(), allowsIn: () => true }),
  mayWrite: ({ writer }) => editors.has(writer),
};
const ctx = { require: async name => ({ items, space, roles })[name] };
const S = await W.start(ctx);
const as = who => (me = `did:craftec:${who}`);
const spaceAt = { who: "space", space: SP };

// 1. A creates a page in the space.
as("A");
let wiki = await S.of(spaceAt);
const ref = await wiki.create("Home page", "Lead\n\n# One\na\n\n# Two\nb", {});
let p = await wiki.page("home_page");
assert.deepEqual(p.sections.map(s => s.heading), [null, "One", "Two"]);
assert.equal(p.text, "Lead\n\n# One\na\n\n# Two\nb");
await assert.rejects(wiki.create("HOME  page", "x"), /already/);

// 2. A and B edit section One from the same version: a conflict, both kept.
const snap = p;
const one = snap.sections.find(s => s.heading === "One");
await wiki.apply(snap, W.planSection(snap.sections, one.sid, "# One\nA's"));
as("B");
wiki = await S.of(spaceAt);
await wiki.apply(snap, W.planSection(snap.sections, one.sid, "# One\nB's"));
p = await wiki.page(ref);
const c = p.sections.find(s => s.sid === one.sid);
assert.equal(c.conflict, true);
assert.equal(p.conflicts, 1);
assert.deepEqual(c.heads.map(h => h.body).sort(), ["# One\nA's", "# One\nB's"]);

// 3. Z (not an editor) suggests on Two; A accepts. Z also suggests on One-later; A declines.
as("Z");
wiki = await S.of(spaceAt);
assert.equal(wiki.mayEdit(), false);
const two = p.sections.find(s => s.heading === "Two");
await wiki.apply(p, W.planSection(p.sections, two.sid, "# Two\nZ's better b"));
p = await wiki.page(ref);
assert.equal(p.pending.length, 1);
assert.equal(p.sections.find(s => s.sid === two.sid).body, "# Two\nb");
as("A");
wiki = await S.of(spaceAt);
await wiki.accept(p, p.pending[0]);
p = await wiki.page(ref);
assert.equal(p.pending.length, 0);
assert.equal(p.sections.find(s => s.sid === two.sid).body, "# Two\nZ's better b");
as("Z");
wiki = await S.of(spaceAt);
await wiki.apply(p, W.planSection(p.sections, two.sid, "# Two\nZ again"));
as("A");
wiki = await S.of(spaceAt);
p = await wiki.page(ref);
await wiki.decline(p, p.pending[0]);
p = await wiki.page(ref);
assert.equal(p.pending.length, 0);
assert.equal(p.sections.find(s => s.sid === two.sid).body, "# Two\nZ's better b");

// 4. A settles the conflict by hand.
const cc = p.sections.find(s => s.sid === one.sid);
await wiki.apply(p, W.planSection(p.sections, one.sid, "# One\nA's and B's", { base: cc.heads.at(-1).h, merges: cc.heads.map(h => h.h) }));
p = await wiki.page(ref);
assert.equal(p.conflicts, 0);
assert.equal(p.sections.find(s => s.sid === one.sid).body, "# One\nA's and B's");

// 5. History: every version, newest first; restore an old one.
const hs = await wiki.history(p);
assert.ok(hs.length >= 6, `versions: ${hs.length}`);
assert.ok(hs.every((v, i) => i === 0 || hs[i - 1].at >= v.at));
const old = hs.find(v => v.sid === one.sid && v.body === "# One\na");
await wiki.restore(p, old);
p = await wiki.page(ref);
assert.equal(p.sections.find(s => s.sid === one.sid).body, "# One\na");
assert.equal(p.conflicts, 0);

// 6. A whole-page edit writes only what changed.
const before = store.size;
await wiki.apply(p, W.planWhole(p.sections, p.text.replace("Lead", "New lead")));
p = await wiki.page(ref);
assert.equal(p.text.startsWith("New lead"), true);
assert.equal(store.size, before, "A's own items edited, none added");

// 7. B copies it to their wiki, changes it, proposes back (an editor there: written).
as("B");
const copyRef = await S.copyToMine(p);
let mineW = await S.of({ who: "mine" });
let mine = await mineW.page(copyRef);
assert.equal(mine.text, p.text);
assert.equal(mine.meta.lineage.from, ref);
await mineW.apply(mine, W.planWhole(mine.sections, mine.text + "\n\n# Three\nfrom B's copy"));
mine = await mineW.page(copyRef);
const r1 = await S.propose(mine);
assert.equal(r1.suggest, false);
assert.equal(r1.written, 1);
as("A");
wiki = await S.of(spaceAt);
p = await wiki.page(ref);
assert.equal(p.sections.at(-1).body, "# Three\nfrom B's copy");
// Proposing again changes nothing.
as("B");
mineW = await S.of({ who: "mine" });
mine = await mineW.page(copyRef);
assert.equal((await S.propose(mine)).written, 0);
// 8. Z copies, edits, proposes: a suggestion there.
as("Z");
const zc = await S.copyToMine(p);
const zw = await S.of({ who: "mine" });
let zp = await zw.page(zc);
await zw.apply(zp, W.planSection(zp.sections, zp.sections[0].sid, "Z's lead"));
zp = await zw.page(zc);
const r2 = await S.propose(zp);
assert.deepEqual(r2, { written: 1, suggest: true });
as("A");
wiki = await S.of(spaceAt);
p = await wiki.page(ref);
assert.equal(p.pending.length, 1);
assert.equal(p.pending[0].fresh, true);
// 9. The commons is not built: refused by name.
await assert.rejects(S.of({ discover: true }), /not built yet/);
console.log("flow ok:", store.size, "items,", hs.length, "versions in history");
