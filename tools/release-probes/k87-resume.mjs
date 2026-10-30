// K8.7 A REMOVAL TAKES BACK EVERYTHING, on fx/fy (built on k85-two): fx registers, fy logs in with the words; fy
// makes a space; fy REMOVES fx — the key log's data key rotates, fy keeps the new seed, the account's tables move, the
// page loads again; the space swaps the account's member. (Header kept from admit.mjs:)
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
  await dlg.locator("input[name=kept]").check({ timeout: 60000 }).catch(async e => { say("register: the words step did not show — the dialog says:", JSON.stringify((await dlg.innerText().catch(() => "?")).slice(0, 600))); throw e; });
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
await step("fy writes", F.ev(async () => (await (await app.require("storage")).table("pins")).put("k85f", "from fy")));
const back = await step("fx reads fy's row", A.ev(async () => { for (let i = 0; i < 24; i++) { const v = (await (await app.require("storage")).table("pins")).rows().find(r => r.key === "k85f")?.value; if (v) return v; await new Promise(r => setTimeout(r, 5000)); } return null; }), 200000).catch(e => e.message);
check("fx reads what fy wrote", back === "from fy", back);
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
// A SPACE fy makes before the removal, a row in it.
const sid = await step("fy makes a space", F.ev(async () => (await (await app.require("space")).create("server", "Rotation space")).id), 120000).catch(e => say("space:", e.message));
const board = (u, put) => u.ev(async ([id, put]) => {
  const sp = (await (await app.require("space")).mine()).find(x => x.id === id);
  if (!sp) return "NO SPACE";
  const t = await (await app.require("storage")).table((await app.require("space")).board(sp).messages, sp);
  await t.settled;
  if (put) await t.put(put[0], put[1]);
  return t.rows().filter(r => r.value).map(r => r.key);
}, [sid, put]).catch(e => "ERR " + e.message);
say("fy space rows before:", JSON.stringify(await board(F, ["rot-before", "written before the rotation"])));
const treeOf = u => u.ev(async id => {
  const sp = (await (await app.require("space")).mine()).find(x => x.id === id);
  const g = (await app.require("members")).group(sp);
  await g.reconcile();
  const me = await (await app.require("members")).me();
  return { placed: g.placed(), members: g.members().map(x => String(x).slice(0, 8)), me: String(me.id).slice(0, 8) };
}, sid).catch(e => ({ ERR: e.message }));
const t0 = await step("fy's space tree before", treeOf(F), 120000);
say("fy's space tree before:", JSON.stringify(t0));
const facts = u => u.ev(async () => {
  const node = await app.require("node"), auth = await app.require("auth");
  const s = await auth.check();
  const [dc, ic] = await Promise.all(["devices-wasm", "idlog-wasm"].map(n => app.require(n)));
  await auth.identity.readKeyLog(s.didBytes);
  const d = JSON.parse(node.core.account_devices_now(dc, ic, s.didBytes) ?? "null");
  const card = await (await app.require("directory")).card(s.did, { fresh: true });
  return { session: String(s.data).slice(0, 12), head: String(d?.data).slice(0, 12), seq: d?.seq, devices: d?.devices?.length, cardMember: !!card?.beekem, was: (card?.was ?? []).length };
}).catch(e => ({ ERR: e.message }));
say("fy before:", JSON.stringify(await facts(F)));
// THE REMOVAL: fy removes fx with the words — its page loads again on the new key.
const fxNode = await A.ev(async () => (await (await app.require("membership")).nodes())?.find(n => n.me)?.key);
const fxIndex = await F.ev(async k => (await (await app.require("membership")).nodes())?.find(n => n.key === k)?.index, fxNode);
// THE PAGE STOPS BEFORE THE COMMIT (as a page closed midway): its `dataKeep` throws this once.
await F.ev(async () => { const id = (await app.require("auth")).identity; id.dataKeep = async () => { throw new Error("stopped before the commit (probe)"); }; });
await step("fy removes fx", F.ev(async ([i, w]) => { const W = (await app.require("node")).glue.CraftworksCore.entropy_of(w); await (await app.require("membership")).remove(i, W); return true; }, [fxIndex, words]), 300000).catch(e => say("remove (the page loads again):", e.message.slice(0, 120)));
await F.p.waitForTimeout(20000);
const mid = await facts(F);
say("fy stopped midway:", JSON.stringify(mid));
check("control: stopped before the commit — fy still on the old key, the head on the new", mid.session && mid.head && mid.session !== mid.head, mid);
// THE PAGE LOADS AGAIN: it finds the rotation it began and finishes it.
await F.p.goto("about:blank");
await F.p.goto(site(17692) + "#/settings");
await F.p.waitForTimeout(90000);
const after = await facts(F);
say("fy after:", JSON.stringify(after));
check("RESUMED: the rotation finished on the next load — fy holds the new key", after.head && after.head === after.session && after.seq >= 1, after);
if (after.head !== after.session) {
  const tr = await F.ev(() => new Promise(r => { const o = []; app.on(e => /account keys|rotat|data key|moved|stray|tree|reconcile|devices|ready|failed|logged in|lifecycle/i.test(`${e.step} ${e.what ?? ""}`) && o.push(`${e.t ?? ""} ${e.step}: ${String(e.what ?? "").slice(0, 260)}`)); setTimeout(() => r(o.slice(-60)), 50); })).catch(e => ["ERR " + e.message]);
  say("fy resume trace:\n  " + tr.join("\n  "));
}
check("fy's card at the new address names a member and keeps the replaced one", after.cardMember && after.was >= 1, after);
let t1 = null;
for (let i = 0; i < 12; i++) { t1 = await step("fy's space tree after", treeOf(F), 180000).catch(e => ({ ERR: e.message })); if (t1?.placed && !t1.members.includes(t0?.me)) break; await F.p.waitForTimeout(10000); }
say("fy's space tree after:", JSON.stringify(t1));
check("the space places the account's NEW member, the old one out", t1?.placed && t1.me !== t0?.me && !t1.members.includes(t0?.me), { t0, t1 });
const rows = await board(F, ["rot-after", "written after the rotation"]);
check("fy reads the space's rows before and after", Array.isArray(rows) && rows.includes("rot-before") && rows.includes("rot-after"), rows);
const fxNow = await step("fx status", A.ev(async () => (await (await app.require("keys")).ready())?.removed ?? "no status"), 30000).catch(e => "ERR " + e.message);
say("fx (removed) status:", JSON.stringify(fxNow));
say("RESULT", fails.length ? `FAIL ${fails.join("; ")}` : "PASS");
await b.close();
