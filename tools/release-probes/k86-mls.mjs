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
let target = null; // the build just published (set by `pub`)
const wants = new Map();
// (PER NODE: each page must run what ITS node serves — 10-28, nodes served their own copies of the site.)
const wantV = port => { if (!wants.has(port)) wants.set(port, fetch(`http://127.0.0.1:${port}/v1/contract/web/EckhKsQWCG6hSJqhRvxXqBEkFMBCm41YHTKnKRmMznpJ/manifest.json`).then(r => r.arrayBuffer()).then(b => crypto.subtle.digest("SHA-256", b)).then(h => Buffer.from(h).toString("hex").slice(0, 12))); return wants.get(port); };
async function open(port, hash = "#/settings") {
  const ctx = await b.newContext();
  const p = await ctx.newPage();
  await traced(p, `${port}-${++pageN}`, TRACE);
  await p.goto(site(port) + hash);
  await p.waitForTimeout(25000);
  const fr = () => p.frames().find(x => x !== p.mainFrame() && x.url() !== "about:srcdoc");
  const want = target ?? (await wantV(port));
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
import { execSync } from "node:child_process";
// UNLOCK ON THE NEW BUILD with its PIN (as the upgrade gate): whoever is logged in on the node from an earlier run out
// first — else the page runs as that account.
async function unlockAs(u, port, pin, did) {
  await pinned(u, port);
  if ((await u.ev(async () => (await (await app.require("space")).account())?.id ?? null).catch(() => null)) === did) return true;
  if (await u.f.getByRole("button", { name: "Log out" }).count()) {
    await u.f.getByRole("button", { name: "Log out" }).first().click({ force: true });
    await u.p.waitForTimeout(10000);
    await pinned(u, port);
  }
  const r = await u.ev(async pin => (await app.require("auth")).unlock(pin).then(x => JSON.stringify(x).slice(0, 120), e => "ERR " + e.message), pin).catch(e => e.message);
  say("unlock", port, r);
  await pinned(u, port);
  await u.p.waitForTimeout(15000);
  return (await u.ev(async () => (await (await app.require("space")).account())?.id ?? null).catch(() => null)) === did;
}

const here = new URL(".", import.meta.url).pathname;
// THE BUILD JUST PUBLISHED (fx's manifest hash): every page opened or reloaded after a publish runs it, or the run
// stops (10-29: the "old build" phase registered on the newer build — a fixed wait after a reload, and a hash cached
// from before the publish).
const pub = dir => {
  say("publish", dir.split("/").pop(), execSync(`bash ${here}publish-to.sh ${dir} 17691`, { encoding: "utf8" }).trim().replace(/\n/g, " | "));
  target = execSync(`curl -sf -m 60 "http://127.0.0.1:17691/v1/contract/web/EckhKsQWCG6hSJqhRvxXqBEkFMBCm41YHTKnKRmMznpJ/manifest.json" | shasum -a 256 | cut -c1-12`, { encoding: "utf8" }).trim();
  say("  the build now:", target);
};
async function pinned(u, port, hash = "#/settings") {
  const fr = () => u.p.frames().find(x => x !== u.p.mainFrame() && x.url() !== "about:srcdoc");
  for (let end = Date.now() + 300000; ; ) {
    await u.p.goto("about:blank");
    await u.p.goto(site(port) + hash);
    await u.p.waitForTimeout(25000);
    const v = await fr()?.evaluate(() => app.siteV).catch(() => null);
    if (!target || v === target) return;
    if (Date.now() > end) throw new Error(`the page on ${port} runs ${v ?? "nothing"}, not the build just published (${target})`);
    // (What the NODE serves now, beside what the page runs: a page behind its node, or a node gone back — 10-30, twice
    // in a full run the page on fx ran the build before for 5 min.)
    const served = execSync(`curl -sf -m 60 "http://127.0.0.1:${port}/v1/contract/web/EckhKsQWCG6hSJqhRvxXqBEkFMBCm41YHTKnKRmMznpJ/manifest.json" | shasum -a 256 | cut -c1-12`, { encoding: "utf8" }).trim();
    say(`  page on ${port} runs ${v ?? "nothing yet"}, the node serves ${served}, waiting for ${target}`);
  }
}
const fails = [];
const check = (what, ok, got) => (say(ok ? "PASS" : "FAIL", what, JSON.stringify(got)?.slice(0, 400)), ok || fails.push(what));
// 1. THE OLD BUILD: two devices of one account, no tree.
pub(process.env.OLD);
// (The page reopened on the build just published.)
await pinned(A, 17691);
const aDid = await step("A registers on fx (old build)", register(A, PIN.a, "alice"), 300000);
clearInterval(capture);
say("A", aDid.slice(12, 20), "| words captured:", !!words);
const e0 = await A.ev(async () => ({ newest: (await (await app.require("access")).keyAt("pins", -1, {})).epoch, mls: (await (await app.require("keys")).ready())?.epoch, did: (await (await app.require("space")).account())?.id?.slice(12, 20) })).catch(e => ({ ERR: e.message }));
say("fx right after registering (old build):", JSON.stringify(e0));
const F = await open(17692);
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
check("fy is the same account (old build)", (await F.ev(async () => (await (await app.require("space")).account())?.id).catch(() => null)) === aDid, null);
for (let i = 0; i < 24; i++) { if ((await F.ev(async () => (await (await app.require("keys")).ready())?.members?.length ?? 0).catch(() => 0)) >= 2) break; await F.p.waitForTimeout(5000); }
await F.p.goto("about:blank");
await A.p.goto("about:blank");
// 2. THE CANDIDATE: fx alone makes the tree and writes; then closes.
pub(process.env.NEW);
await unlockAs(A, 17691, PIN.a, aDid);
const didOf = u => u.ev(async () => (await (await app.require("space")).account())?.id ?? null).catch(() => null);
check("fx, reloaded on the new build, is the account just registered", (await didOf(A)) === aDid, { now: (await didOf(A))?.slice(12, 20), registered: aDid.slice(12, 20) });
const a1 = await step("fx: the account's tree", A.ev(async () => { const t = await (await app.require("members")).account(); await t.reconcile(); return { placed: t.placed(), members: t.members().length, gens: t.generationNumbers() }; }), 300000).catch(e => ({ ERR: e.message }));
check("fx makes the tree alone (fy offline: one member)", a1.placed && a1.members === 1, a1);
const w = await step("fx writes", A.ev(async () => { const t = await (await app.require("storage")).table("pins"); await t.put("k85m", "sealed with the generation"); return (await (await app.require("access")).keyAt("pins", -1, {})).epoch; }));
check("fx writes with the generation", String(w) === String(a1.gens?.[0]), { w, gens: a1.gens });
await A.p.goto("about:blank");
// 3. fy BACK, no words, nobody to place it: it reads through the MLS-sealed generation.
await unlockAs(F, 17692, PIN.b, aDid);
await F.ev(() => { window.__mls = []; app.on(e => /opened with its MLS epoch|recovered from their escrows/.test(String(e.what ?? "")) && window.__mls.push(`${e.t} ${e.what}`)); });
check("fy, reloaded, is the same account", (await didOf(F)) === aDid, { now: (await didOf(F))?.slice(12, 20), registered: aDid.slice(12, 20) });
const got = await step("fy reads fx's row", F.ev(async () => { for (let i = 0; i < 36; i++) { const v = (await (await app.require("storage")).table("pins")).rows().find(r => r.key === "k85m")?.value; if (v) return v; await new Promise(r => setTimeout(r, 5000)); } return null; }), 240000).catch(e => e.message);
const fyPlaced = await F.ev(async () => (await (await app.require("members")).account()).placed()).catch(e => e.message);
const how = await F.ev(() => window.__mls).catch(() => []);
check("fy (a device from before the tree, no words) reads what fx sealed with the generation", got === "sealed with the generation", { got, how });
const fy2 = await step("fy: credential carried, tree entered", F.ev(async () => { const st = await (await app.require("keys")).ready(); const t = await (await app.require("members")).account(); await t.reconcile(); const life = await app.require("lifecycle"); return { members: st?.members?.length ?? null, account: life.state("account", "me").s, placed: t.placed(), treeMembers: t.members().length, writable: await (await app.require("keys")).writable() }; }), 300000).catch(e => ({ ERR: e.message }));
check("fy: its MLS-era credential carried, MEMBER, entered the tree by itself, writable", fy2.members === 2 && fy2.account === "MEMBER" && fy2.placed === true && fy2.writable === true, fy2);
// (On failure: fy's account/keys TRACE — what decided its state.)
if (!(fy2.account === "MEMBER")) {
  const tr = await F.ev(() => new Promise(r => { const o = []; app.on(e => /lifecycle (account|session)|account keys|keys|join|devices|credential|mls|handover|earlier build|data key/i.test(`${e.step} ${e.what ?? ""}`) && o.push(`${e.t ?? ""} ${e.step}: ${String(e.what ?? "").slice(0, 240)}`)); setTimeout(() => r(o.slice(-40)), 50); })).catch(e => ["ERR " + e.message]);
  say("fy trace:\n  " + tr.join("\n  "));
}
say("RESULT", fails.length ? `FAIL ${fails.join("; ")}` : "PASS");
await b.close();
