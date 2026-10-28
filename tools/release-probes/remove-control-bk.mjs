// SHARED ITEMS (S1, 10-20) on fx/fy: A (fx) makes a private note and shares it with B (fy) as EDITOR; B finds it under
// Shared and edits it, and A reads B's edit; A makes B a VIEWER: B's next edit is refused and nothing changes for A;
// A removes B. Every step bounded; each answer read from the app's own state.
//   node share.mjs <fresh>
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
  await dlg.locator("input[name=kept]").check();
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

// UNFRIENDED (C2): a circle's files re-key when someone leaves it; what is posted after is not theirs to read.
// CONTROL for C2: removal from an ORDINARY space — does the key commit follow the remove act?
const A = await open(17691);
const B = await open(17692);
await register(A, `${fresh}1`);
const bDid = await register(B, `${fresh}2`);
const sid = await A.ev(async d => { const S = await app.require("space"); const sp = await S.create("server", "Removal control"); await (await app.require("conversation")).invite(sp, d); return sp.id; }, bDid);
// (K8.3: keys are the BeeKEM tree — B placed in it before; A's tree no longer holding B after the removal.)
const placedB = () => B.ev(async id => { const sp = (await (await app.require("space")).mine()).find(s => s.id === id); return sp ? (await app.require("members")).group(sp).placed() : null; }, sid);
const holdsB = () => A.ev(async ([id, d]) => { const sp = (await (await app.require("space")).mine()).find(s => s.id === id); return (await app.require("members")).group(sp).holds(d); }, [sid, bDid]);
const inB = await until("B enters and is placed", async () => (await B.ev(async () => (await app.require("conversation")).accept()).catch(() => null), (await placedB()) ? true : null), 600000);
check("control: B is in (placed in the tree)", inB === true, inB);
await A.ev(async ([id, d]) => { const sp = (await (await app.require("space")).mine()).find(s => s.id === id); return (await (await app.require("moderation")).of(sp)).remove(d); }, [sid, bDid]);
const t0 = Date.now();
const out = await until("A's tree no longer holds B", async () => ((await holdsB()) === false ? true : null), 300000);
check("the keys followed the remove act (the tree no longer holds B)", out === true, { out, s: Math.round((Date.now() - t0) / 1000) });
// THE LEAK ITSELF: A posts in the space after the removal; does B read it?
const after = await A.ev(async id => (await app.require("items")).submit({ board: id, kind: "post", title: "after removal", body: "x", audience: "members" }), sid);
await new Promise(r => setTimeout(r, 45000));
const bReads = await B.ev(async r => !!(await (await app.require("items")).get(r).catch(() => null)), after);
check("B does not read what is posted after its removal", bReads === false, bReads);
say(fails.length ? `RESULT FAIL: ${fails.join(" · ")}` : "RESULT PASS");
await b.close();
process.exit(fails.length ? 1 : 0);
