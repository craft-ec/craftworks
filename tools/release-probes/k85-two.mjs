// ADMISSION, one rule (10-13), on fx/fy: A (fx) makes a space and a one-use code; B (fy) joins by it; A's page lets B in.
// THE LOOP CHECK: requests that must NOT be let in — B asking again from the epoch it holds, a request with no key of
// its own, a request naming B but carrying C's key, the same request again — and A's admit run repeatedly over them:
// the space's epoch must not move. Then A's page CLOSED: C (fy) asks by a second code and A's NODE lets it in; A's page
// reopened lets nobody in again (the node's record read).
//   node admit.mjs <fresh> [phase]
const { chromium } = await import(process.env.PLAYWRIGHT ?? `${process.env.HOME}/dev/atelier/node_modules/playwright-core/index.mjs`);
const { traced, traceDir } = await import(new URL("./trace.mjs", import.meta.url));
const TRACE = traceDir(import.meta.url);
let pageN = 0;
const [fresh, phase = "all"] = process.argv.slice(2);
// A PIN is the node's: each run its own (a PIN registered by an earlier run is taken).
const PIN = { a: `${fresh}1`, b: `${fresh}2`, c: `${fresh}3` };
const site = port => `http://127.0.0.1:${port}/v1/contract/web/EckhKsQWCG6hSJqhRvxXqBEkFMBCm41YHTKnKRmMznpJ/?fresh=${fresh}`;
const b = await chromium.launch({ headless: true, executablePath: process.env.CHROME });
const say = (...a) => console.log(new Date().toISOString().slice(11, 19), ...a);
const wait = ms => new Promise(r => setTimeout(r, ms));
const wants = new Map();
// (PER NODE: each node serves its own last copy of the site — 10-28, fx's update never reached fy or fz — so a page must
// run what ITS node serves, published there too.)
const wantV = port => { if (!wants.has(port)) wants.set(port, fetch(`http://127.0.0.1:${port}/v1/contract/web/EckhKsQWCG6hSJqhRvxXqBEkFMBCm41YHTKnKRmMznpJ/manifest.json`).then(r => r.arrayBuffer()).then(b => crypto.subtle.digest("SHA-256", b)).then(h => Buffer.from(h).toString("hex").slice(0, 12))); return wants.get(port); };
async function open(port, hash = "#/settings") {
  const ctx = await b.newContext();
  const p = await ctx.newPage();
  await traced(p, `${port}-${++pageN}`, TRACE);
  await p.goto(site(port) + hash);
  await p.waitForTimeout(25000);
  const fr = () => p.frames().find(x => x !== p.mainFrame() && x.url() !== "about:srcdoc");
  // THE VERSION JUST PUBLISHED, on every page (as the upgrade gate): fx's manifest's hash — a node serves the version
  // before until the new one's pieces arrive (10-28: fy ran an older groups package than fx in one run).
  const want = await wantV(port);
  for (let end = Date.now() + 300000; ; ) {
    const v = await fr()?.evaluate(() => app.siteV).catch(() => null);
    if (v === want) break;
    if (Date.now() > end) throw new Error(`the page on ${port} runs ${v ?? "nothing"}, not the version just published (${want})`);
    say(`  page on ${port} runs ${v ?? "nothing yet"}, waiting for ${want}`);
    await p.goto("about:blank");
    await p.goto(site(port) + hash);
    await p.waitForTimeout(20000);
  }
  return { ctx, p, port, f: p.frameLocator("#app"), ev: (fn, arg) => fr().evaluate(fn, arg) };
}
async function register(u, pin, handle) {
  // LOGIN IS THE NODE'S: whoever is logged in there from before logs out first.
  if (await u.f.getByRole("button", { name: "Log out" }).count()) {
    await u.f.getByRole("button", { name: "Log out" }).first().click({ force: true });
    await u.p.waitForTimeout(10000);
    await u.p.goto("about:blank");
    await u.p.goto(site(u.port) + "#/chat");
    await u.p.waitForTimeout(25000);
  }
  const dlg = u.f.locator(".auth-dialog");
  await dlg.locator("[data-tab=register]").click();
  await dlg.locator("[data-panel=register] input[name=pin]").fill(pin);
  await dlg.locator("[data-panel=register] input[name=again]").fill(pin);
  await dlg.locator('[data-panel=register] button:has-text("Next")').click();
  await u.p.waitForTimeout(5000);
  await dlg.locator("input[name=kept]").check();
  await dlg.locator('button:has-text("Create my account")').click();
  // Logged in (the account known), not a fixed wait: registration waits on the identity build up to 180 s.
  for (let i = 0; i < 48; i++) {
    await u.p.waitForTimeout(5000);
    if (await u.ev(async () => !!(await (await app.require("space")).account())).catch(() => false)) break;
  }
  await u.p.waitForTimeout(10000);
  await u.ev(async () => (await app.require("directory")).publish()).catch(e => say(handle, "card publish:", e.message));
  return u.ev(async () => (await (await app.require("space")).account()).id);
}
const epochOf = (u, id) => u.ev(async id => { const sp = (await (await app.require("space")).mine()).find(s => s.id === id); return sp ? (await (await app.require("keys")).group(sp).ready())?.epoch ?? null : null; }, id);
const admitRun = (u, id) => u.ev(async id => { const sp = (await (await app.require("space")).mine()).find(s => s.id === id); return (await app.require("conversation")).admit(sp); }, id);
const acceptRun = u => u.ev(async () => (await (await app.require("conversation")).accept()).map(s => s.id));

const step = async (what, p, ms = 180000) => { say("…", what); let t; try { return await Promise.race([p, new Promise((_, no) => (t = setTimeout(() => no(new Error(`STALLED at: ${what}`)), ms)))]); } finally { clearTimeout(t); } };
// A on fx: registered — its WORDS captured as the dialog shows them once.
const A = await open(17691);
let words = null;
A.p.on("framenavigated", () => {});
const capture = setInterval(async () => { if (words) return; const w = await A.f.locator("form.words-once .shown li").allTextContents().catch(() => []); if (w.length >= 12) words = w.join(" "); }, 300);
const aDid = await step("A registers on fx", register(A, PIN.a, "alice"), 300000);
await A.ev(() => { window.__ev = []; app.on(e => /writers|reopen|table open|pins|directory|gather|sealing/i.test(`${e.step} ${e.what ?? ""}`) && window.__ev.push(`${e.t} ${e.step} ${String(e.what ?? "").slice(0, 140)}`)); });
clearInterval(capture);
say("A", aDid.slice(12, 20), "| words captured:", !!words);
const fails = [];
const check = (what, ok, got) => (say(ok ? "PASS" : "FAIL", what, JSON.stringify(got)?.slice(0, 400)), ok || fails.push(what));
const tree = process.env.NO_TREE ? async () => ({ placed: true, members: 2, gens: ["-"], said: "no tree (control)" }) : u => u.ev(async () => {
  const t = await (await app.require("members")).account();
  const said = await t.reconcile();
  const ch = await (await app.require("storage")).table((await app.require("space")).tables.channel);
  const keys = ch.rows().map(r => r.key.slice(0, 2));
  return { said: JSON.stringify(said).slice(0, 240), placed: t.placed(), members: t.members().length, gens: t.generationNumbers(), o: keys.filter(k => k === "o/").length, d: keys.filter(k => k === "d/").length, g: keys.filter(k => k === "g/").length };
});
const a1 = await step("fx: the account's tree", tree(A), 300000);
check("fx makes the tree", a1.placed && a1.gens.length >= 1, a1);
await step("fx writes", A.ev(async () => (await (await app.require("storage")).table("pins")).put("k85a", "from fx")));
// fy: the same account by its WORDS.
const F = await open(17692);
const logs = { A: [], F: [] };
A.p.on("console", m => /pins|not saved|sealed|tables|directory|k85/i.test(m.text()) && logs.A.push(m.text().slice(0, 200)));
F.p.on("console", m => /pins|not saved|sealed|tables|directory|k85/i.test(m.text()) && logs.F.push(m.text().slice(0, 200)));
if (await F.f.getByRole("button", { name: "Log out" }).count()) {
  await F.f.getByRole("button", { name: "Log out" }).first().click({ force: true });
  await F.p.waitForTimeout(10000);
  await F.p.goto("about:blank");
  await F.p.goto(site(17692) + "#/settings");
  await F.p.waitForTimeout(25000);
}
const dlg = F.f.locator(".auth-dialog");
await dlg.locator("[data-tab=login]").click();
await dlg.locator("button.swap[data-to=words]").click();
await dlg.locator("form.words textarea[name=words]").fill(words);
await dlg.locator("form.words input[name=pin]").fill(PIN.b);
await dlg.locator("form.words input[name=again]").fill(PIN.b);
await dlg.locator("form.words button[type=submit], form.words button:not([type])").first().click();
for (let i = 0; i < 60; i++) { await F.p.waitForTimeout(5000); if (await F.ev(async () => (await (await app.require("space")).account())?.id).catch(() => null)) break; }
check("fy is the same account", (await F.ev(async () => (await (await app.require("space")).account())?.id).catch(() => null)) === aDid, null);
for (let i = 0; i < 24; i++) { if (await F.ev(async () => !!(await (await app.require("keys")).ready())).catch(() => false)) break; await F.p.waitForTimeout(5000); }
// fy writes its leaf (not placed yet); fx adds it; fy is placed.
const f1 = await step("fy: first reconcile", tree(F), 300000).catch(e => ({ ERR: e.message }));
say("fy first:", JSON.stringify(f1).slice(0, 300));
let a2 = null, f2 = null;
for (let i = 0; i < 12 && !(f2?.placed && a2?.members === 2); i++) {
  a2 = await step("fx reconciles", tree(A), 300000).catch(e => ({ ERR: e.message }));
  await F.p.waitForTimeout(8000);
  f2 = await step("fy reconciles", tree(F), 300000).catch(e => ({ ERR: e.message }));
}
check("fx adds fy: two members", a2?.members === 2, a2);
check("fy placed in the account's tree", !!f2?.placed, f2);
const read = await step("fy reads fx's row", F.ev(async () => { for (let i = 0; i < 24; i++) { const v = (await (await app.require("storage")).table("pins")).rows().find(r => r.key === "k85a")?.value; if (v) return v; await new Promise(r => setTimeout(r, 5000)); } return null; }), 200000).catch(e => e.message);
check("fy reads what fx wrote", read === "from fx", read);
// (fy's own steps from its write on: when it lists itself — directory, catalog, credential — against the write.)
await F.ev(() => { window.__fyAll = []; app.on(e => window.__fyAll.push([Date.now(), e])); window.__fyFrom = window.__fyAll.length; window.__fyT0 = Date.now(); });
const fyCatsBefore = await F.ev(async () => { const S = await app.require("storage"); const n = await app.require("node"); return (await S.own()).filter(x => /tables/.test(x.app)).map(x => n.hex(x.id)); }).catch(() => []);
const fp = (U, ids) => U.ev(async ids => { const n = await app.require("node"); const out = {}; for (const c of ids) { const j = JSON.stringify(await n.ask(n.core.frames_get(n.bytes(c))[1], x => x.id === c, "raw get", 10000).catch(() => null)); out[c.slice(0, 8)] = j ? Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(j)))).slice(0, 4).map(x => x.toString(16).padStart(2, "0")).join("") : null; } return out; }, ids).catch(e => ({ ERR: e.message }));
say("fy catalogs before its write — on fy:", JSON.stringify(await fp(F, fyCatsBefore)), "on fx:", JSON.stringify(await fp(A, fyCatsBefore)));
await step("fy writes", F.ev(async () => (await (await app.require("storage")).table("pins")).put("k85f", "from fy")));
say("fy catalogs after its write — on fy:", JSON.stringify(await fp(F, fyCatsBefore)), "on fx:", JSON.stringify(await fp(A, fyCatsBefore)));
// THE TIMELINE (node or ours): fy's pins table id; then every 2 s, does fx's NODE hold the row (raw get, our read code
// bypassed) and does fx's PAGE show it — the first time each is true.
const fyPins = await F.ev(async () => { const S = await app.require("storage"); const n = await app.require("node"); const t = (await S.own()).find(x => x.app === "pins"); return t ? n.hex(t.id) : null; }).catch(() => null);
// (fy's OWN node, the same raw get, polled alongside: when the writer's node holds the row, epoch ms.)
const fyNodeP = F.ev(async id => { const n = await app.require("node"); for (let i = 0; i < 120; i++) { const said = await n.ask(n.core.frames_get(n.bytes(id))[1], x => x.id === id, "raw get", 10000).catch(e => ({ ERR: e.message })); if (JSON.stringify(said).includes("k85f")) return Date.now(); await new Promise(r => setTimeout(r, 500)); } return null; }, fyPins).catch(e => "ERR " + e.message);
const fyCatIds = await F.ev(async () => { const S = await app.require("storage"); const n = await app.require("node"); return (await S.own()).filter(x => /tables/.test(x.app)).map(x => n.hex(x.id)); }).catch(() => []);
await A.ev(ids => (window.__fyCats = ids), fyCatIds);
const back = await step("fx reads fy's row", A.ev(async id => {
  const n = await app.require("node");
  const t0 = Date.now(), line = [], ev = []; window.__fxT0 = t0;
  // (fx's own discovery steps as they happen: writers checked, catalogs read, tables opened.)
  // (Only what happens from now: `app.on` replays its history as it subscribes — those dropped.)
  const all = [];
  app.on(e => all.push([Date.now() - t0, e]));
  const from = all.length;
  const evNow = () => all.slice(from).filter(([, e]) => /feed opened|writers checked|table open|pins|catalog|tables|directory|feed|gather|devices|members|node:/i.test(`${e.step} ${e.what ?? ""}`)).map(([ms, e]) => `${ms}ms ${e.step}: ${String(e.what ?? "").slice(0, 170)}`);
  let nodeAt = null, pageAt = null, v = null;
  // (fy's catalogs on fx's NODE, every second: when their root moves — the node's arrival, apart from the page's push.)
  const cats = window.__fyCats ?? [], catAt = {};
  // (The whole answer's fingerprint — a tail delta moves no root, 10-28: only a flush does.)
  const catRoot = async c => { const j = JSON.stringify(await n.ask(n.core.frames_get(n.bytes(c))[1], x => x.id === c, "raw get", 10000).catch(() => null)); return j ? Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(j)))).slice(0, 4).map(x => x.toString(16).padStart(2, "0")).join("") : null; };
  const roots0 = Object.fromEntries(await Promise.all(cats.map(async c => [c, await catRoot(c)])));
  const catPoll = setInterval(() => cats.forEach(async c => { const r = await catRoot(c); if (r && r !== roots0[c] && !catAt[c]) catAt[c] = `${Date.now() - t0}ms ${roots0[c]}→${r}`; }), 1000);
  for (let i = 0; i < 60 && pageAt === null; i++) {
    if (nodeAt === null && id) {
      const said = await n.ask(n.core.frames_get(n.bytes(id))[1], x => x.id === id, "raw get", 10000).catch(e => ({ ERR: e.message }));
      if (JSON.stringify(said).includes("k85f")) nodeAt = Date.now() - t0;
      else line.push(`${Date.now() - t0}ms node: ${JSON.stringify(said).slice(0, 80)}`);
    }
    v = (await (await app.require("storage")).table("pins")).rows().find(r => r.key === "k85f")?.value;
    if (v) pageAt = Date.now() - t0;
    else await new Promise(r => setTimeout(r, 2000));
  }
  clearInterval(catPoll);
  return { v, nodeAt, pageAt, catAt, roots0, misses: line.slice(0, 2), replayed: from, ev: evNow().slice(0, 50) };
}, fyPins), 200000).catch(e => ({ ERR: e.message }));
const fyNodeAt = await fyNodeP; const fxT0 = await A.ev(() => window.__fxT0).catch(() => null);
say("fy node holds row at (ms from fx t0):", typeof fyNodeAt === "number" && fxT0 ? fyNodeAt - fxT0 : fyNodeAt);
say("fx read timeline:", JSON.stringify({ ...back, ev: undefined }));
say("fx steps:\n  " + (back?.ev ?? []).join("\n  "));
const fySteps = await F.ev(() => window.__fyAll.slice(window.__fyFrom).filter(([, e]) => /written|directory|tables|node:|listed|catalog|devices|credential|members|pins/i.test(`${e.step} ${e.what ?? ""}`)).map(([t, e]) => `${t - window.__fyT0}ms ${e.step}: ${String(e.what ?? "").slice(0, 170)}`).slice(0, 40)).catch(e => ["ERR " + e.message]);
say("fy steps:\n  " + fySteps.join("\n  "));
// fx: is the account's directory FOLLOWED (pushed) or only READ (seen when read again)?
const follow = await A.ev(async () => { const n = await app.require("node"); return [...n.gets()].filter(([, r]) => /tables|directory|devices|members/i.test(r.what ?? "")).map(([k, r]) => `${k.slice(0, 8)} ${r.s} gets=${r.gets} ${r.what}`).slice(0, 12); }).catch(e => ["ERR " + e.message]);
say("fx follows:\n  " + follow.join("\n  "));
// FY's CATALOGS (its own "tables" contracts), raw on fy's node and on fx's node: the same state on both, or not.
const fyCats = await F.ev(async () => { const S = await app.require("storage"); const n = await app.require("node"); return (await S.own()).filter(x => /tables/.test(x.app)).map(x => ({ app: x.app, id: n.hex(x.id) })); }).catch(e => [{ ERR: e.message }]);
const rawOn = (U, id) => U.ev(async id => { const n = await app.require("node"); const s = await n.ask(n.core.frames_get(n.bytes(id))[1], x => x.id === id, "raw get", 10000).catch(e => ({ ERR: e.message })); return JSON.stringify(s).slice(0, 200); }, id).catch(e => "ERR " + e.message);
for (const c of fyCats) say(`fy catalog ${c.app} ${String(c.id).slice(0, 12)}: on fy ${await rawOn(F, c.id)} | on fx ${await rawOn(A, c.id)}`);
say("fy pins on fx:", fyPins, JSON.stringify(await A.ev(async id => { const n = await app.require("node"); const r = n.gets().get(id); return r ? { s: r.s, gets: r.gets, followed: r.followed, what: r.what } : "never read"; }, fyPins).catch(e => "ERR " + e.message)));
// WHERE DISCOVERY STANDS on fx (whether or not it showed): the devices it reads, the pins feeds found for each,
// fy's own catalog's listing of pins.
const disc = await A.ev(async () => {
  const S = await app.require("storage"), n = await app.require("node");
  const nodes = await S.nodes().catch(e => "ERR " + e.message);
  const feeds = Array.isArray(nodes) ? await S.feedsOf("pins", nodes).catch(e => "ERR " + e.message) : null;
  const t = await S.table("pins");
  return { nodes: Array.isArray(nodes) ? nodes.map(x => String(x).slice(0, 12)) : nodes, feeds: JSON.stringify(feeds).slice(0, 700), built: (t.built ?? []).map(x => x.slice(0, 12)), info: t.info };
}).catch(e => ({ ERR: e.message }));
say("fx discovery:", JSON.stringify(disc).slice(0, 1500));
const fyCat = await F.ev(async () => { const S = await app.require("storage"); const own = await S.own(); return own.map(x => `${x.app}`).join(","); }).catch(e => "ERR " + e.message);
say("fy's own tables:", fyCat);
check("fx reads what fy wrote", back?.v === "from fy", back);

// WHERE IT STOPS: each side's view of the account's devices, fy's own pins feed, fx's view of fy's.
const view = u => u.ev(async () => {
  const S = await app.require("storage");
  const nodes = await S.nodes().catch(e => "ERR " + e.message);
  const me = (await (await app.require("space")).account())?.self;
  const feeds = Array.isArray(nodes) ? await S.feedsOf("pins", nodes).catch(e => "ERR " + e.message) : null;
  const pins = await S.table("pins");
  const trace = (window.__trace ?? []).slice(-0);
  const mw = await (await app.require("membership")).writers().catch(e => "ERR " + e.message);
  const kr = await (await app.require("keys")).ready().catch(e => ({ ERR: e.message }));
  return { mlsWriters: Array.isArray(mw) ? mw.map(n => String(n).slice(0, 10)) : mw, mlsEpoch: kr?.epoch, mlsMembers: kr?.members?.length, me: String(me).slice(0, 10), nodes: Array.isArray(nodes) ? nodes.map(n => String(n).slice(0, 10)) : nodes, feeds: Array.isArray(feeds) ? feeds.map(f => ({ owner: String(f.owner).slice(0, 10), keys: f.rows.map(r => r.key) })) : feeds, merged: pins.rows().map(r => r.key), state: pins.state ?? null };
});
const gOf = u => u.ev(async () => (await (await app.require("storage")).table((await app.require("space")).tables.channel)).rows().filter(r => r.key.startsWith("g/")).length);
const g1 = await gOf(A).catch(() => -1);
await A.p.waitForTimeout(60000);
const g2 = await gOf(A).catch(() => -2);
check("no more key changes once both are placed (escrows over a minute)", g1 === g2, { g1, g2 });
// REMOVAL: fy removes the fx device with the words (the key log rotated, fy credentialed again) — fy's reconcile takes
// fx out with a new generation; fx reads nothing fy writes after.
const fxNode = await A.ev(async () => (await (await app.require("membership")).nodes())?.find(n => n.me)?.key);
const fxIndex = await F.ev(async k => (await (await app.require("membership")).nodes())?.find(n => n.key === k)?.index, fxNode);
await step("fy removes fx", F.ev(async ([i, w]) => { const W = (await app.require("node")).glue.CraftworksCore.entropy_of(w); await (await app.require("membership")).remove(i, W); return true; }, [fxIndex, words]), 300000).catch(e => say("remove:", e.message));
const devs = u => u.ev(async () => {
  const node = await app.require("node"), auth = await app.require("auth");
  const didB = node.glue.did_bytes((await (await app.require("space")).account()).id);
  const log = await auth.identity.readKeyLog(didB), dev = await auth.identity.readDevices(didB);
  const [dc, ic] = await Promise.all(["devices-wasm", "idlog-wasm"].map(n => app.require(n)));
  return { log, dev, current: [...node.core.account_device_members(dc, ic, didB)].map(x => String(x).slice(0, 8)), ever: [...node.core.account_device_members_ever(dc, ic, didB)].map(x => String(x).slice(0, 8)) };
}).catch(e => ({ ERR: e.message }));
say("fy devices after the removal:", JSON.stringify(await devs(F)));
let fr = null;
for (let i = 0; i < 12; i++) { fr = await step("fy reconciles after the removal", tree(F), 300000).catch(e => ({ ERR: e.message })); if (fr?.members === 1) break; await F.p.waitForTimeout(10000); }
say("fy devices at the end:", JSON.stringify(await devs(F)));
check("fy takes fx out of the account's tree", fr?.members === 1 && fr?.placed, fr);
const sealedWith = await step("fy writes after", F.ev(async () => { const t = await (await app.require("storage")).table("pins"); await t.put("k85after", "after the removal"); const ev = []; app.on(e => /table sealing pins/.test(`${e.step} ${e.what ?? ""}`) && ev.push(e.what)); await new Promise(r => setTimeout(r, 50)); return { newest: (await (await app.require("access")).keyAt("pins", -1, {})).epoch, tail: ev.at(-1) ?? null }; }));
const fxHolds = await step("fx: does it hold that epoch", A.ev(async e => { const k = await (await app.require("access")).keyAt("pins", e, {}); return { key: !!k.key, why: k.why ?? null }; }, sealedWith.newest), 60000).catch(e => ({ ERR: e.message }));
await step("fx: listen", A.ev(() => { window.__mls = []; app.on(e => /opened with its MLS epoch|recovered from their escrows/.test(String(e.what ?? "")) && window.__mls.push(`${e.t} ${e.what}`)); }), 30000).catch(() => {});
const mlsOf = u => step("MLS epoch", u.ev(async () => (await (await app.require("keys")).mlsNow())?.epoch ?? null), 60000).catch(e => "ERR " + e.message);
const mrows = await F.ev(async () => (await (await app.require("storage")).table((await app.require("space")).tables.channel)).rows().filter(r => r.key.startsWith("m/")).map(r => r.key)).catch(e => e.message);
say("MLS epochs — fy:", await mlsOf(F), "fx:", await mlsOf(A), "| m/ rows:", JSON.stringify(mrows));
say("fy sealed its write with", sealedWith, "| fx holds that epoch:", JSON.stringify(fxHolds), "| fy's tree gens:", JSON.stringify(fr?.gens));
await A.p.waitForTimeout(60000);
const leak = await step("fx reads after", A.ev(async () => (await (await app.require("storage")).table("pins")).rows().find(r => r.key === "k85after")?.value ?? null), 60000).catch(e => "ERR " + e.message);
say("fx after:", JSON.stringify(await step("fx after", A.ev(async () => ({ gens: (await (await app.require("members")).account()).generationNumbers(), opened: window.__mls ?? [] })), 60000).catch(e => e.message)));
check("the removed fx reads nothing written after", leak === null || String(leak).startsWith("ERR"), leak);
say("fx events:", JSON.stringify(await step("fx events", A.ev(() => window.__ev.filter(x => /pins|reopen/.test(x)).slice(-30)), 30000).catch(e => e.message)));
say("fx view:", JSON.stringify(await step("fx view", view(A), 60000).catch(e => e.message)).slice(0, 600));
say("fy view:", JSON.stringify(await step("fy view", view(F), 60000).catch(e => e.message)).slice(0, 600));
say("fx log:", JSON.stringify(logs.A.slice(-12)));
say("fy log:", JSON.stringify(logs.F.slice(-12)));
say("RESULT", fails.length ? `FAIL ${fails.join("; ")}` : "PASS");
await b.close();
