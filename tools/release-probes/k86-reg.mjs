const { chromium } = await import(process.env.PLAYWRIGHT ?? `${process.env.HOME}/dev/atelier/node_modules/playwright-core/index.mjs`);
const [fresh] = process.argv.slice(2);
const site = port => `http://127.0.0.1:${port}/v1/contract/web/EckhKsQWCG6hSJqhRvxXqBEkFMBCm41YHTKnKRmMznpJ/?fresh=${fresh}`;
const b = await chromium.launch({ headless: true, executablePath: process.env.CHROME });
const say = (...a) => console.log(new Date().toISOString().slice(11, 19), ...a);
const fails = [];
const check = (what, ok, got) => (say(ok ? "PASS" : "FAIL", what, JSON.stringify(got)?.slice(0, 300)), ok || fails.push(what));
const wants = new Map();
// (PER NODE: each page must run what ITS node serves — 10-28, nodes served their own copies of the site.)
const wantV = port => { if (!wants.has(port)) wants.set(port, fetch(`http://127.0.0.1:${port}/v1/contract/web/EckhKsQWCG6hSJqhRvxXqBEkFMBCm41YHTKnKRmMznpJ/manifest.json`).then(r => r.arrayBuffer()).then(b => crypto.subtle.digest("SHA-256", b)).then(h => Buffer.from(h).toString("hex").slice(0, 12))); return wants.get(port); };
async function open(port, hash = "#/settings") {
  const ctx = await b.newContext();
  const p = await ctx.newPage();
  const allow = setInterval(() => p.evaluate(() => [...(document.getElementById("__freenet_perm_overlay")?.querySelectorAll("button") ?? [])].find(x => x.innerText.trim() === "Allow")?.click()).catch(() => {}), 2000);
  p.on("close", () => clearInterval(allow));
  await p.goto(site(port) + hash);
  await p.waitForTimeout(25000);
  const fr = () => p.frames().find(x => x !== p.mainFrame() && x.url() !== "about:srcdoc");
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
  return { p, port, f: p.frameLocator("#app"), ev: (fn, arg) => fr().evaluate(fn, arg) };
}
async function register(u, pin) {
  if (await u.ev(async () => !!(await (await app.require("space")).account())).catch(() => false)) {
    // (Logging out reloads the page: the call loses its page, as expected.)
    await u.ev(async () => (await app.require("auth")).logout()).catch(e => say("logout:", e.message.slice(0, 80)));
    await u.p.waitForTimeout(8000);
    await u.p.goto("about:blank");
    await u.p.goto(site(u.port) + "#/settings");
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
  for (let i = 0; i < 48; i++) {
    await u.p.waitForTimeout(5000);
    if (await u.ev(async () => !!(await (await app.require("space")).account())).catch(() => false)) break;
  }
  await u.p.waitForTimeout(10000);
  await u.ev(async () => (await app.require("directory")).publish()).catch(e => say("card publish:", e.message));
  return u.ev(async () => (await (await app.require("space")).account()).id);
}
const bounded = (what, p, ms = 180000) => Promise.race([p, new Promise((_, no) => setTimeout(() => no(new Error(`STALLED: ${what} (${ms / 1000} s)`)), ms))]);
const until = async (what, test, ms) => {
  const end = Date.now() + ms;
  for (;;) {
    const v = await bounded(what, test(), 120000).catch(e => (say(what, "check:", e.message), null));
    if (v) return v;
    if (Date.now() > end) return null;
    await new Promise(r => setTimeout(r, 15000));
  }
};


// K8.5 PHASE 2 on fx: a fresh account's TREE made beside MLS — its root, its signed leaf and its generation's escrow in
// the channel; the generation kept as the account's newest epoch; a write sealed with it; a reload reads it.
const A = await open(17691);
const did = await register(A, `${fresh}1`);
say("registered", did?.slice(0, 24));
const r1 = await bounded("account tree reconcile", A.ev(async () => {
  const m = await app.require("members");
  const t = await m.account();
  const said = await t.reconcile();
  const ch = await (await app.require("storage")).table((await app.require("space")).tables.channel);
  const rows = ch.rows().map(r => r.key.slice(0, 2));
  return { said: JSON.stringify(said).slice(0, 300), placed: t.placed(), gens: t.generationNumbers(), o: rows.filter(k => k === "o/").length, d: rows.filter(k => k === "d/").length, g: rows.filter(k => k === "g/").length };
}), 300000).catch(e => ({ ERR: e.message }));
const k6 = await bounded("6a facts", A.ev(async () => {
  const keys = await app.require("keys");
  const st = await keys.ready();
  const ch = await (await app.require("storage")).table((await app.require("space")).tables.channel);
  const life = await app.require("lifecycle");
  return { members: st?.members?.length ?? null, me: st?.me ?? null, writable: await keys.writable(), account: life.state("account", "me").s, session: life.state("session", "me").s, mlsRows: ch.rows().filter(r => r.key === "info" || r.key.startsWith("e/") || r.key === "start").map(r => r.key) };
}), 120000).catch(e => ({ ERR: e.message }));
check("6a: one device credentialed, MEMBER, OPEN, writable, no MLS rows in the channel", k6.members === 1 && k6.account === "MEMBER" && k6.session === "OPEN" && k6.writable === true && k6.mlsRows?.length === 0, k6);
check("the account's tree made: placed, a generation, root/leaf/escrow in the channel", r1.placed && r1.gens?.length >= 1 && r1.o >= 1 && r1.d >= 1 && r1.g >= 1, r1);
const gen = Number(r1.gens?.[0]);
const w = await bounded("write after", A.ev(async () => {
  const t = await (await app.require("storage")).table("pins");
  await t.put("k85", "written with the generation");
  const acc = await (await app.require("access")).keyAt("pins", -1, {});
  return { epoch: acc.epoch };
}), 120000).catch(e => ({ ERR: e.message }));
check("the newest epoch is the generation", w.epoch === gen, { w, gen });
await A.p.goto("about:blank");
await A.p.goto(site(A.port) + "#/settings");
await A.p.waitForTimeout(40000);
const back = await until("read after reload", () => A.ev(async () => (await (await app.require("storage")).table("pins")).rows().find(r => r.key === "k85")?.value ?? null), 240000);
check("read after a reload", back === "written with the generation", back);
say("RESULT", fails.length ? `FAIL ${fails.join("; ")}` : "PASS");
await b.close();
