// K90 KICK WITH EVERY PAGE CLOSED, on fx/fy/fz: A (fz — a node under freenet's 64-per-app background cap) owner, B (fy)
// and C (fx) members; A uploads a file; A KICKS B and
// every A and C page closes at once. With no A or C page open, a NODE takes B out of the space's tree and re-keys its
// files: their feeds change while closed (raw GETs from B's page), then C, reopened, reads (no reconcile) B out and the
// file under the new salt.   node k90-kick-closed.mjs <fresh>
const { chromium } = await import(process.env.PLAYWRIGHT ?? `${process.env.HOME}/dev/atelier/node_modules/playwright-core/index.mjs`);
const { traced, traceDir, nodeTrace } = await import(new URL("./trace.mjs", import.meta.url));
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
  // (NOT logged in after it: what the dialog says and the page's own auth trace — 10-29, two registrations failed.)
  if (!(await u.ev(async () => !!(await (await app.require("space")).account())).catch(() => false))) {
    say(handle, "registration did not finish — the dialog:", JSON.stringify((await u.f.locator(".auth-dialog").innerText().catch(() => "?")).slice(0, 500)));
    say(handle, "its trace:\n  " + (await u.ev(() => new Promise(r => { const o = []; app.on(e => /auth|register|identity|provision|signer|account|login|session|failed|refus/i.test(`${e.step} ${e.what ?? ""}`) && o.push(`${e.t ?? ""} ${e.step}: ${String(e.what ?? "").slice(0, 220)}`)); setTimeout(() => r(o.slice(-40)), 50); })).catch(e => ["ERR " + e.message])).join("\n  "));
  }
  await u.ev(async () => (await app.require("directory")).publish()).catch(e => say(handle, "card publish:", e.message));
  return u.ev(async () => (await (await app.require("space")).account()).id);
}
const epochOf = (u, id) => u.ev(async id => { const sp = (await (await app.require("space")).mine()).find(s => s.id === id); return sp ? (await (await app.require("keys")).group(sp).ready())?.epoch ?? null : null; }, id);
const admitRun = (u, id) => u.ev(async id => { const sp = (await (await app.require("space")).mine()).find(s => s.id === id); return (await app.require("conversation")).admit(sp); }, id);
const acceptRun = u => u.ev(async () => (await (await app.require("conversation")).accept()).map(s => s.id));

const step = async (what, p, ms = 180000) => { say("…", what); let t; try { return await Promise.race([p, new Promise((_, no) => (t = setTimeout(() => no(new Error(`STALLED at: ${what}`)), ms)))]); } finally { clearTimeout(t); } };
const fails = [];
const check = (what, ok, got) => (say(ok ? "PASS" : "FAIL", what, JSON.stringify(got)?.slice(0, 500)), ok || fails.push(what));
const A = await open(17693);
const aDid = await step("A registers on fz", register(A, PIN.a, "alice"), 300000);
const B = await open(17692);
const bDid = await step("B registers on fy", register(B, PIN.b, "bob"), 300000);
const C = await open(17691);
const cDid = await step("C registers on fx", register(C, PIN.c, "carol"), 300000);
say("A", aDid.slice(12, 20), "B", bDid.slice(12, 20), "C", cDid.slice(12, 20));
const sid = await step("A makes a space", A.ev(async () => (await (await app.require("space")).create("server", "Kick space")).id), 120000).catch(e => say("space:", e.message));
const spOf = `(await (await app.require("space")).mine()).find(s => s.id === id)`;
const invite = () => A.ev(async id => { const sp = (await (await app.require("space")).mine()).find(s => s.id === id); return (await app.require("conversation")).createInvite(sp, { uses: 1 }); }, sid);
await step("B joins", B.ev(async c => (await app.require("conversation")).join(c), await invite()), 180000).catch(e => say("B join:", e.message));
await step("C joins", C.ev(async c => (await app.require("conversation")).join(c), await invite()), 180000).catch(e => say("C join:", e.message));
const treeOf = (u, reconcile = true) => u.ev(async ([id, reconcile]) => {
  const sp = (await (await app.require("space")).mine()).find(x => x.id === id);
  if (!sp) return { placed: false, members: [], ERR: "not in my spaces" };
  const g = (await app.require("members")).group(sp);
  if (reconcile) await g.reconcile(); else await g.sync();
  const me = await (await app.require("members")).me();
  return { placed: g.placed(), members: g.members().map(x => String(x).slice(0, 8)), me: String(me.id).slice(0, 8) };
}, [sid, reconcile]).catch(e => ({ ERR: e.message }));
let tA = null, tB = null, tC = null;
for (let i = 0; i < 18; i++) {
  [tA, tB, tC] = [await treeOf(A), await treeOf(B), await treeOf(C)];
  if (tA?.members?.length === 3 && tB?.placed && tC?.placed) break;
  await wait(10000);
}
check("A's tree places A, B and C", tA?.members?.length === 3 && [tB?.me, tC?.me].every(m => tA.members.includes(m)), { tA, tB, tC });
// A FILE in the space (not inline: its pieces are what a re-key moves).
const fileRef = await step("A uploads a file", A.ev(async id => { const sp = (await (await app.require("space")).mine()).find(s => s.id === id); const body = new Uint8Array(200000).map((_, i) => (i * 31) & 255); return (await app.require("files")).put(new Blob([body]), { space: sp, app: "probe", inline: false }); }, sid), 300000).catch(e => ({ ERR: e.message }));
const filesOf = u => u.ev(async id => {
  const sp = (await (await app.require("space")).mine()).find(s => s.id === id);
  if (!sp) return "NO SPACE";
  const S = await app.require("storage"), sn = (await app.require("space")).tableOf(sp, "files");
  const t = await S.table(sn, sp); await t.settled;
  const salt = t.rows().find(r => r.key === "salt" && r.value);
  const files = t.rows().filter(r => r.value && !r.key.startsWith("salt")).map(r => { try { const v = JSON.parse(r.value); return { key: r.key.slice(0, 10), n: v.n }; } catch { return { key: r.key.slice(0, 10) }; } });
  return { salt: salt ? JSON.parse(salt.value).n : null, files };
}, sid).catch(e => "ERR " + e.message);
let f0 = null;
for (let i = 0; i < 12; i++) { f0 = await filesOf(C); if (f0?.files?.length) break; await wait(10000); }
check("C sees the file under the space's salt", f0?.files?.length >= 1 && f0.files[0].n === f0.salt, { f0, fileRef: JSON.stringify(fileRef).slice(0, 120) });
// THE FEEDS WATCHED: A's and C's own feeds of the space's key operations and files.
const feedsOf = u => u.ev(async id => {
  const sp = (await (await app.require("space")).mine()).find(s => s.id === id);
  const n = await app.require("node"), S = await app.require("storage"), sx = (await app.require("space")).tableOf(sp, "files");
  return (await S.own()).filter(x => x.app === sx || /keyops$/.test(x.app)).map(x => ({ app: x.app.slice(-8), id: n.hex(x.id) }));
}, sid).catch(e => [{ ERR: e.message }]);
const watched = [...(await feedsOf(A)).map(f => ({ ...f, who: "A" })), ...(await feedsOf(C)).map(f => ({ ...f, who: "C" }))];
say("watched feeds:", JSON.stringify(watched.map(w => `${w.who}:${w.app}:${String(w.id).slice(0, 8)}`)));
const fp = () => B.ev(async ids => { const n = await app.require("node"); const out = {}; for (const c of ids) { const j = JSON.stringify(await n.ask(n.core.frames_get(n.bytes(c))[1], x => x.id === c, "raw get", 10000).catch(() => null)); out[c] = j ? Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(j)))).slice(0, 4).map(x => x.toString(16).padStart(2, "0")).join("") : null; } return out; }, watched.map(w => w.id)).catch(e => ({ ERR: e.message }));
// A BASELINE ONCE THE FEEDS ARE STILL (10-29: A's upload, still spreading, counted as the node's work): two equal
// reads in a row, before the kick.
let base = await fp();
for (let end = Date.now() + 180000; Date.now() < end; ) {
  await wait(15000);
  const again = await fp();
  if (JSON.stringify(again) === JSON.stringify(base)) break;
  base = again;
}
// THE KICK — then every A and C page closed.
const tKick = Date.now();
await step("A kicks B", A.ev(async ([id, did]) => { const sp = (await (await app.require("space")).mine()).find(s => s.id === id); return (await (await app.require("moderation")).of(sp)).remove(did); }, [sid, bDid]), 180000).catch(e => say("kick:", e.message));
await A.p.goto("about:blank");
await C.p.goto("about:blank");
const tClosed = Date.now();
say("closed", Math.round((tClosed - tKick) / 1000), "s after the kick began");
// WAITED ON THE FILES FEED MOVING from the still baseline, then STILL again (the node's new salt and the row it
// re-keyed — one wake-up, one or two updates; 10-29: waiting for two sat out the deadline). Deadline 15 min.
const changes = {}, last = { ...base };
let filesMoved = false;
for (let end = Date.now() + 900000; Date.now() < end; ) {
  await wait(10000);
  const now = await fp();
  let movedNow = false;
  for (const w of watched) if (now[w.id] && last[w.id] && now[w.id] !== last[w.id]) { changes[w.id] = (changes[w.id] ?? 0) + 1; last[w.id] = now[w.id]; movedNow = true; say("moved:", `${w.who}:${w.app}`, Math.round((Date.now() - tKick) / 1000), "s after the kick"); }
  if (filesMoved && !movedNow) break;
  filesMoved ||= watched.some(w => !/keyops$/.test(w.app) && changes[w.id]);
}
// C REOPENED: what the nodes wrote, read (merged, no reconcile of its own first).
await C.p.goto(site(17691) + "#/settings");
await wait(25000);
const tK = await treeOf(C, false);
check("B is out of the space's tree (read by C, no reconcile)", !!tK?.members && !tK.members.includes(tB?.me) && tK.members.includes(tA?.me) && tK.members.includes(tC?.me), { tK, b: tB?.me });
const f1 = await filesOf(C);
check("the file is re-keyed under a new salt", f1?.salt > f0?.salt && f1.files.every(f => f.n === f1.salt), { f0, f1 });
// C — on another node — READS the file as it is now (its re-keyed pieces fetched over the network): the bytes A put.
const readBack = await C.ev(async ref => {
  const F = await app.require("files");
  const got = new Uint8Array(await (await F.get(await F.current(ref))).arrayBuffer());
  return { size: got.length, same: got.length === 200000 && got.every((x, i) => x === ((i * 31) & 255)), ref: JSON.stringify(await F.current(ref)).slice(0, 120) };
}, fileRef).catch(e => ({ ERR: e.message }));
check("C reads the file's bytes after the re-key (from another node)", readBack?.same === true, readBack);
// THE NODES' OWN RECORD (always): what each node's rounds decided — the judge of who did it.
const nodeOf = u => u.ev(async () => (await (await app.require("auth")).identity.upkeepStatus())?.upkeep ?? null).catch(e => ({ ERR: e.message }));
const cNode = await nodeOf(C);
say("C's node:", JSON.stringify(cNode));
const tReopenA = Date.now();
await A.p.goto(site(17693) + "#/settings");
await wait(25000);
const aNode = await nodeOf(A);
say("A's node:", JSON.stringify(aNode));
say("node traces:", "A (fz)", await nodeTrace(A, "A-fz", TRACE), "line(s); C (fx)", await nodeTrace(C, "C-fx", TRACE), "line(s)");
const spHex = await C.ev(async id => (await (await app.require("space")).mine()).find(s => s.id === id)?.id ?? null, sid).catch(() => null);
const after = d => (d?.decided ?? []).filter(x => x.at >= tKick && (!spHex || x.space === spHex || x.space?.startsWith?.(String(spHex).slice(0, 12))));
const decided = [...after(aNode).map(x => ({ node: "A", ...x })), ...after(cNode).map(x => ({ node: "C", ...x }))];
// (Who took B out is reported, not judged: B's own page — open, for the raw reads — takes its member out when kicked;
// otherwise a node does. That B is out is checked above.)
say("B taken out by:", decided.some(x => x.step === "keys-reconciled" && /1 removed/.test(x.why ?? "")) ? "a node" : "B's own page (kicked, it leaves)");
check("a NODE re-keyed the space's files (files-rekeyed, after the kick)", decided.some(x => x.step === "files-rekeyed"), decided.map(x => `${x.node} ${x.step}: ${String(x.why).slice(0, 90)}`));
// A's PAGE did no key change after the kick (its whole trace, on file).
const fsm = await import("node:fs");
const aTrace = (() => { try { return fsm.readFileSync(`${TRACE}/17693-1.trace`, "utf8"); } catch { return ""; } })();
const tag = String(spHex ?? "").slice(0, 12), hhmmss = new Date(tKick).toISOString().slice(11, 19);
const reopened = new Date(tReopenA).toISOString().slice(11, 19);
const pageAfter = aTrace.split("\n").filter(l => l.slice(0, 8) >= hhmmss && l.slice(0, 8) < reopened && l.includes(tag) && /keys-reconciled|keys-tree-entered/.test(l));
check("A's page made no key change after the kick (its trace)", pageAfter.length === 0, pageAfter.slice(0, 3));
say("RESULT", fails.length ? "FAIL " + fails.join("; ") : "PASS");
await b.close();
process.exit(0);
