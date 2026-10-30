// K8 JOIN AND KICK, on fx/fy: A (fx) and B (fy) are TWO accounts. A makes a space and a one-use code; B joins by it;
// each reads what the other writes after the join. A KICKS B: the space's tree takes B out, A writes on, B reads
// nothing A writes after, and what B writes after is not read by A.   node k88-join-kick.mjs <fresh>
const { chromium } = await import(process.env.PLAYWRIGHT ?? `${process.env.HOME}/dev/atelier/node_modules/playwright-core/index.mjs`);
const { traced, traceDir } = await import(new URL("./trace.mjs", import.meta.url));
const TRACE = traceDir(import.meta.url);
let pageN = 0;
const [fresh, phase = "all"] = process.argv.slice(2);
// A PIN is the node's: each run its own (a PIN registered by an earlier run is taken).
const PIN = { a: `${fresh}1`, b: `${fresh}2`, c: `${fresh}3`, z: `${fresh}4` };
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
  // (The node's permission prompt — Background, for wake-ups — allowed.)
  const allow = setInterval(() => p.evaluate(() => [...(document.getElementById("__freenet_perm_overlay")?.querySelectorAll("button") ?? [])].find(x => x.innerText.trim() === "Allow")?.click()).catch(() => {}), 2000);
  p.on("close", () => clearInterval(allow));
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
const fails = [];
const check = (what, ok, got) => (say(ok ? "PASS" : "FAIL", what, JSON.stringify(got)?.slice(0, 400)), ok || fails.push(what));
const A = await open(17691);
const aDid = await step("A registers on fx", register(A, PIN.a, "alice"), 300000);
const B = await open(17692);
const bDid = await step("B registers on fy", register(B, PIN.b, "bob"), 300000);
say("A", aDid.slice(12, 20), "B", bDid.slice(12, 20));
const sid = await step("A makes a space", A.ev(async () => (await (await app.require("space")).create("server", "Join space")).id), 120000).catch(e => say("space:", e.message));
const board = (u, put) => u.ev(async ([id, put]) => {
  // (The space as this page last held it: a KICKED member writes anyway — what a client ignoring the kick does — so the
  // check is of A's reading, never of whether B's page had dropped the space yet; 10-30: two runs passed only because
  // B's page had, "NO SPACE", and wrote nothing.)
  const sp = (await (await app.require("space")).mine()).find(x => x.id === id) ?? window.__probeSpace;
  if (!sp) return "NO SPACE";
  window.__probeSpace = sp;
  const t = await (await app.require("storage")).table((await app.require("space")).board(sp).messages, sp);
  await t.settled;
  if (put) await t.put(put[0], put[1]);
  return t.rows().filter(r => r.value).map(r => r.key);
}, [sid, put]).catch(e => "ERR " + e.message);
const treeOf = u => u.ev(async id => {
  const sp = (await (await app.require("space")).mine()).find(x => x.id === id);
  if (!sp) return { placed: false, members: [], ERR: "not in my spaces" };
  const g = (await app.require("members")).group(sp);
  await g.reconcile();
  const me = await (await app.require("members")).me();
  return { placed: g.placed(), members: g.members().map(x => String(x).slice(0, 8)), me: String(me.id).slice(0, 8) };
}, sid).catch(e => ({ ERR: e.message }));
// A row seen by `u` within `ms`: polled, the first time it shows (or null).
const seen = async (u, key, ms) => { for (let end = Date.now() + ms; Date.now() < end; ) { const r = await board(u); if (Array.isArray(r) && r.includes(key)) return true; await wait(5000); } return false; };
say("A rows before the join:", JSON.stringify(await board(A, ["a-before", "written before B joined"])));
// THE JOIN: A's one-use code, B joins by it; both trees place B.
const code = await A.ev(async id => { const sp = (await (await app.require("space")).mine()).find(s => s.id === id); return (await app.require("conversation")).createInvite(sp, { uses: 1 }); }, sid);
await step("B joins", B.ev(async c => (await app.require("conversation")).join(c), code), 180000).catch(e => say("join:", e.message));
let tA = null, tB = null;
for (let i = 0; i < 18; i++) {
  tA = await step("A's space tree", treeOf(A), 120000).catch(e => ({ ERR: e.message }));
  tB = await step("B's space tree", treeOf(B), 120000).catch(e => ({ ERR: e.message }));
  if (tA?.members?.length === 2 && tB?.placed) break;
  await wait(10000);
}
check("A's tree places B", tA?.members?.length === 2 && tA.members.includes(tB?.me), { tA, tB });
check("B is placed in the space's tree", !!tB?.placed, tB);
// AFTER THE JOIN: each reads what the other writes.
await board(A, ["a-joined", "A after B joined"]);
check("B reads what A writes after the join", await seen(B, "a-joined", 120000), await board(B));
await board(B, ["b-member", "B as a member"]);
check("A reads what B writes", await seen(A, "b-member", 120000), await board(A));
say("B reads A's row from before it joined:", await seen(B, "a-before", 5000));
// THE KICK: A removes B; A's tree takes B's member out.
await step("A kicks B", A.ev(async ([id, did]) => { const sp = (await (await app.require("space")).mine()).find(s => s.id === id); return (await (await app.require("moderation")).of(sp)).remove(did); }, [sid, bDid]), 180000).catch(e => say("kick:", e.message));
let tK = null;
for (let i = 0; i < 18; i++) {
  tK = await step("A's space tree after the kick", treeOf(A), 120000).catch(e => ({ ERR: e.message }));
  if (tK?.members && !tK.members.includes(tB?.me)) break;
  await wait(10000);
}
check("A's tree takes B out", !!tK?.members && !tK.members.includes(tB?.me) && tK.members.includes(tA?.me), { tK, b: tB?.me });
// A WRITES ON (its write not refused, read back).
const after = await board(A, ["a-after-kick", "A after B was kicked"]);
check("A writes after the kick", Array.isArray(after) && after.includes("a-after-kick"), after);
// B READS NOTHING A WRITES AFTER (a control: B still reads what it read before).
const bNow = await board(B);
check("B does not read what A writes after the kick", !(await seen(B, "a-after-kick", 90000)), await board(B));
say("B still lists (control):", JSON.stringify(bNow));
// WHAT B WRITES AFTER is not read by A.
say("B's write after the kick:", JSON.stringify(await board(B, ["b-after-kick", "B after it was kicked"])));
check("A does not read what B writes after the kick", !(await seen(A, "b-after-kick", 90000)), await board(A));
// (WHY, always: A's departed rows, the writer caps it derives, and B's feeds' heads now — read from A's page.)
const why = await A.ev(async ([id, bDid]) => {
  const sp = (await (await app.require("space")).mine()).find(s => s.id === id);
  const S = await app.require("storage"), dir = await app.require("directory");
  const t = await S.table((await app.require("space")).tableOf(sp, "departed"), sp); await t.settled;
  const w = await S.writersOf(sp);
  const bNodes = await dir.devices.strict(bDid);
  return { departed: t.rows().map(r => [r.key.slice(0, 10), r.value]), caps: [...w.byNode].filter(([n]) => bNodes.includes(n)).map(([n, x]) => [n.slice(0, 10), x]), bNodes: bNodes.map(n => n.slice(0, 10)), bHeads: await S.headsOf(sp, bNodes) };
}, [sid, bDid]).catch(e => ({ ERR: e.message }));
say("A's view of B (departed, caps, heads):", JSON.stringify(why).slice(0, 2500));
check("A still reads what B wrote as a member", (await board(A)).includes?.("b-member"), await board(A));
say("RESULT", fails.length ? "FAIL " + fails.join("; ") : "PASS");
await b.close();
process.exit(0);
