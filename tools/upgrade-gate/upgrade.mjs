// ENTER BY ITSELF (FOUNDATION "Members and keys", 10-17) on fx/fy, traced: (helpers from admit.mjs) ADMISSION, one rule (10-13), on fx/fy: A (fx) makes a space and a one-use code; B (fy) joins by it; A's page lets B in.
// THE LOOP CHECK: requests that must NOT be let in — B asking again from the epoch it holds, a request with no key of
// its own, a request naming B but carrying C's key, the same request again — and A's admit run repeatedly over them:
// the space's epoch must not move. Then A's page CLOSED: C (fy) asks by a second code and A's NODE lets it in; A's page
// reopened lets nobody in again (the node's record read).
//   node admit.mjs <fresh> [phase]
const { chromium } = await import(process.env.PLAYWRIGHT ?? `${process.env.HOME}/dev/atelier/node_modules/playwright-core/index.mjs`);
const [fresh, phase = "all"] = process.argv.slice(2);
// A PIN is the node's: each run its own (a PIN registered by an earlier run is taken).
const PIN = { a: `${fresh}1`, b: `${fresh}2`, c: `${fresh}3` };
const site = port => `http://127.0.0.1:${port}/v1/contract/web/EckhKsQWCG6hSJqhRvxXqBEkFMBCm41YHTKnKRmMznpJ/?fresh=${fresh}`;
let SID = null;
const b = await chromium.launch({ headless: true, executablePath: process.env.CHROME });
const say = (...a) => console.log(new Date().toISOString().slice(11, 19), ...a);
const wait = ms => new Promise(r => setTimeout(r, ms));
async function open(port, hash = "#/settings") {
  const ctx = await b.newContext();
  const p = await ctx.newPage();
  // THE NODE's BACKGROUND PROMPT ("run … even with its tab closed"): Allow, as the person would — wake-ups need it.
  const allow = setInterval(() => p.evaluate(() => [...(document.getElementById("__freenet_perm_overlay")?.querySelectorAll("button") ?? [])].find(b => b.innerText.trim() === "Allow")?.click()).catch(() => {}), 2000);
  p.on("close", () => clearInterval(allow));
  await p.goto(site(port) + hash);
  await p.waitForTimeout(25000);
  const fr = () => p.frames().find(x => x !== p.mainFrame() && x.url() !== "about:srcdoc");
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


// MEMBERS AND KEYS on fx/fy (10-16): every step BOUNDED, its stall named; each answer read from the app's own state.
const sub = async (what, p, ms = 120000) => { let t; const r = await Promise.race([p, new Promise((_, no) => (t = setTimeout(() => no(new Error(`STALLED at: ${what} (${ms / 1000} s)`)), ms)))]); clearTimeout(t); return r; };
const until = async (what, test, ms, every = 10000) => { const end = Date.now() + ms; for (;;) { const v = await sub(`${what} (one check)`, test(), 90000).catch(e => (say(what, "check:", e.message), null)); if (v) return v; if (Date.now() > end) throw new Error(`NOT WITHIN ${ms / 1000} s: ${what}`); await wait(every); } };
const leaves = (u, id) => u.ev(async id => { const sp = (await (await app.require("space")).mine()).find(s => s.id === id); if (!sp) return null; const { glue } = await app.require("node"); const st = await (await app.require("keys")).group(sp).ready(); return (st?.members ?? []).filter(m => m.cred).map(m => glue.did_of(new Uint8Array(m.cred.match(/../g).slice(4, 36).map(x => parseInt(x, 16))))); }, id);
const nodeSaid = async u => ((await u.ev(async () => (await (await app.require("auth")).identity.upkeepStatus())?.upkeep ?? null))?.decided ?? []).slice(-4).map(d => `${d.step}: ${d.why}`).join(" / ");
// TELEMETRY BY DEFAULT: after every step, each page's lifecycle moves since the last print (group, member, join) and
// its node's record — printed whether the step passed or not.
const seenAt = new Map();
async function trace(label, u) {
  if (!u) return;
  const since = seenAt.get(u) ?? 0;
  const j = await u.ev(async since => (await (await app.require("lifecycle")).journal()).filter(e => ["group", "member", "join"].includes(e.m) && e.at > since).map(e => [e.at, `${new Date(e.at).toISOString().slice(11, 19)} ${e.m} ${String(e.key).slice(0, 10)} ${e.event} ${e.from}>${e.to} ${e.why ?? e.waits ?? ""} ${e.by ?? ""}`]), since).catch(x => [[0, `journal not read: ${x.message}`]]);
  if (j.length) seenAt.set(u, Math.max(since, ...j.map(([at]) => at)));
  const st = await u.ev(async () => (await (await app.require("auth")).identity.upkeepStatus())?.upkeep ?? null).catch(() => null);
  say(`  [${label}] journal:\n     ` + (j.map(([, l]) => l).join("\n     ") || "(no moves)") + `\n  [${label}] node: wake-ups ${st?.wakeups ?? "?"}, round at ${st?.roundAt ?? "?"}, decided ${JSON.stringify((st?.decided ?? []).slice(-6).map(d => `${d.step}: ${d.why}`))}`);
}
const out = {};
const K3 = {};
let KA = null, KB = null;
let A = null;
const errs = [];
const inSpace = (u, id) => u.ev(async id => !!(await (await app.require("space")).mine()).find(s => s.id === id), id);
const sp = `const sp = (await (await app.require("space")).mine()).find(s => s.id === id);`;
const logout = async u => {
  await u.f.locator("body").evaluate(() => (location.hash = "#/settings"));
  await u.p.waitForTimeout(8000);
  await u.f.getByRole("button", { name: "Log out" }).first().click({ force: true });
  await u.p.waitForTimeout(10000);
  await u.ctx.close();
};

// THE UPGRADE GATE: a lived-in account on the OLD build (UP_PHASE=old), then the NEW build published and the same
// accounts unlocked with their PINs (UP_PHASE=new). State between the phases in $UPGRADE_STATE.
import { readFileSync, writeFileSync } from "node:fs";
const STATE = process.env.UPGRADE_STATE ?? `${process.env.TMPDIR ?? "/tmp"}/craftworks-upgrade.json`;
const upPhase = process.env.UP_PHASE;
const row = (u, id, table, key) => u.ev(async ([id, table, key]) => { const s = (await (await app.require("space")).mine()).find(x => x.id === id); if (!s) return "NO SPACE"; const st = await app.require("storage"); const sp = await app.require("space"); const t = await st.table(sp.tableOf(s, table), s); await t.settled; const dec = await t.decided?.().catch(e => ({ err: e.message })); const r = t.rows().find(r => r.key === key); return r?.value ?? `MISSING (${JSON.stringify(dec ?? t.state)})`; }, [id, table, key]);
const put = (u, id, table, key, value) => u.ev(async ([id, table, key, value]) => { const s = (await (await app.require("space")).mine()).find(x => x.id === id); const st = await app.require("storage"); const sp = await app.require("space"); const t = await st.table(sp.tableOf(s, table), s); await t.put(key, value); return "ok"; }, [id, table, key, value]).catch(e => "ERR " + e.message);
const channels = (u, id) => u.ev(async id => { const s = (await (await app.require("space")).mine()).find(x => x.id === id); if (!s) return "NO SPACE"; const c = await (await app.require("conversation")).channels(s); await c.settled; return c.list().map(x => x.name ?? x.id); }, id).catch(e => "ERR " + e.message);
const reconcile = (u, id) => u.ev(async id => { const s = (await (await app.require("space")).mine()).find(x => x.id === id); return s ? (await app.require("members")).group(s).reconcile() : "NO SPACE"; }, id).catch(e => ({ err: e.message }));
const deep = (u, id) => u.ev(async id => { const s = (await (await app.require("space")).mine()).find(x => x.id === id); if (!s) return "NO SPACE"; const r = await (await app.require("roles")).of(s); await r.settled; const st = await app.require("storage"); const sp = await app.require("space"); const t = await st.table(sp.board(s).messages, s); await t.settled; const ep = ((await (await app.require("auth")).identity.epochSecrets(s.idBytes, 2 ** 24).catch(() => [])) ?? []).map(([e]) => e); return { acts: r.acts().length, board: t.rows().filter(x => x.value).length, epochs: ep, problems: st.problemsIn(s.id).length }; }, id).catch(e => "ERR " + e.message);
const mine = u => u.ev(async () => (await (await app.require("space")).mine()).map(s => `${s.kind}:${s.id.slice(0, 8)}`)).catch(e => "ERR " + e.message);
try {
  if (upPhase === "old") {
    KA = await open(17691, "#/settings");
    const aDid = await sub("A registers", register(KA, PIN.a, "anna"), 300000);
    KB = await open(17692, "#/settings");
    const bDid = await sub("B registers", register(KB, PIN.b, "bob"), 300000);
    await KB.ev(async () => (await app.require("members")).me());
    const sid = await KA.ev(async () => (await (await app.require("space")).create("server", "Upgrade space")).id);
    await KA.ev(async ([id, d]) => { const s = (await (await app.require("space")).mine()).find(x => x.id === id); return (await app.require("conversation")).invite(s, d); }, [sid, bDid]);
    await until("A reconciles B in", async () => { const r = await reconcile(KA, sid); return r?.added?.length || (r?.placed !== false && (await KA.ev(async id => { const s = (await (await app.require("space")).mine()).find(x => x.id === id); return (await app.require("members")).group(s).members().length; }, sid)) >= 2); }, 300000, 20000);
    await until("B in the space", async () => (await acceptRun(KB), await KB.ev(async id => !!(await (await app.require("space")).mine()).find(x => x.id === id), sid)), 300000);
    await until("B placed", async () => { const r = await reconcile(KB, sid); return KB.ev(async id => { const s = (await (await app.require("space")).mine()).find(x => x.id === id); return (await app.require("members")).group(s).placed(); }, sid); }, 400000, 20000);
    say("1 put before:", await put(KA, sid, "up", "before", "written on the old build"));
    const dm = await KA.ev(async d => (await (await app.require("conversation")).direct(d)).id, bDid).catch(e => "ERR " + e.message);
    say("2 direct conversation:", dm);
    say("   dm put:", await put(KA, dm, "up", "dm", "a direct message on the old build"));
    await until("B sees the channel list", async () => { const c = await channels(KB, sid); return Array.isArray(c) && c.length ? c : null; }, 400000, 20000).catch(e => say("   (no channel list on the old build:", e.message, ")"));
    const before = { channels: await channels(KB, sid), aChannels: await channels(KA, sid), aDeep: await deep(KA, sid), bDeep: await deep(KB, sid), row: await until("B reads 'before'", async () => { const v = await row(KB, sid, "up", "before"); return String(v).startsWith("MISSING") ? null : v; }, 400000, 20000).catch(e => e.message), aSpaces: await mine(KA), bSpaces: await mine(KB) };
    say("3 OLD BUILD, B sees:", JSON.stringify(before));
    writeFileSync(STATE, JSON.stringify({ sid, dm, aDid, bDid, pins: PIN, before }));
  } else {
    const st = JSON.parse(readFileSync(STATE, "utf8"));
    Object.assign(PIN, st.pins);
    KA = await open(17691, "#/settings");
    KB = await open(17692, "#/settings");
    // (LOGIN IS THE NODE'S: whoever is logged in there from an earlier run logs out first — else the unlock below
    // tests that account, not this one.)
    for (const [n, u, pin, did] of [["A", KA, st.pins.a, st.aDid], ["B", KB, st.pins.b, st.bDid]]) {
      if (await u.f.getByRole("button", { name: "Log out" }).count()) {
        await u.f.getByRole("button", { name: "Log out" }).first().click({ force: true });
        await u.p.waitForTimeout(10000);
        await u.p.goto("about:blank");
        await u.p.goto(site(u.port) + "#/settings");
        await u.p.waitForTimeout(25000);
      }
      const r = await sub(`${n} unlocks`, u.ev(async pin => (await app.require("auth")).unlock(pin).then(x => JSON.stringify(x).slice(0, 200), e => "ERR " + e.message), pin), 300000).catch(e => e.message);
      // (A session opened: the page loaded again, as a login does.)
      await u.p.goto("about:blank");
      await u.p.goto(site(u.port) + "#/settings");
      await u.p.waitForTimeout(40000);
      const who = await u.ev(async () => (await (await app.require("space")).account())?.id ?? null).catch(e => "ERR " + e.message);
      say(`4 ${n} unlocks on the NEW build: ${r}; account ${who === did ? "is " + n + "'s" : "WRONG: " + who}`);
      if (who !== did) throw new Error(`${n}'s unlock did not open ${n}'s account`);
    }
    // THE RECOVERY (in the background after a login): watched until each holds epoch 0 again, its log printed.
    // The login's work on the earlier builds: its LIFECYCLE moves (`build`: retired / silent / stopped / keys / epochs),
    // the not-here ones counted — and the member move from the trace.
    const logOf = async u => {
      const moves = await u.ev(async () => (await (await app.require("lifecycle")).journal()).filter(e => e.m === "build")).catch(e => [{ event: "ERR " + e.message }]);
      const notHere = moves.filter(e => e.event === "not-here").length;
      const lines = moves.filter(e => e.event !== "not-here").map(e => `build ${String(e.key).slice(0, 10)} ${e.event} ${e.from}>${e.to} ${JSON.stringify(Object.fromEntries(Object.entries(e).filter(([k]) => !["m", "key", "from", "to", "event", "at", "v"].includes(k))))}`);
      const moved = await u.ev(() => new Promise(r => { const o = []; app.on(e => /member moved/i.test(`${e.step}`) && o.push(`${e.step}: ${String(e.what ?? "").slice(0, 220)}`)); setTimeout(() => r(o.slice(-4)), 50); })).catch(e => ["ERR " + e.message]);
      return [...moved, ...lines, `(${notHere} build(s) not on this node)`, moves.some(e => e.event === "retired") ? "RETIRED: yes" : "RETIRED: NONE"];
    };
    for (const [n, u] of [["A", KA], ["B", KB]]) {
      const got = await until(`${n} holds epoch 0 again`, async () => { const d = await deep(u, st.sid); return Array.isArray(d?.epochs) && d.epochs.includes(0) ? d : null; }, 300000, 20000).catch(e => e.message);
      say(`4b ${n} recovery: ${JSON.stringify(got).slice(0, 200)}\n     ` + (await logOf(u)).join("\n     "));
    }
    const t0 = Date.now();
    const check = async () => ({
      aSpaces: await mine(KA), bSpaces: await mine(KB),
      aChannels: await channels(KA, st.sid), bChannels: await channels(KB, st.sid),
      aBefore: await row(KA, st.sid, "up", "before"), bBefore: await row(KB, st.sid, "up", "before"),
      bDm: await row(KB, st.dm, "up", "dm"),
      aDeep: await deep(KA, st.sid), bDeep: await deep(KB, st.sid),
    });
    say("5 NEW BUILD at once:", JSON.stringify(await check()));
    say("   A reconciles:", JSON.stringify(await reconcile(KA, st.sid)).slice(0, 300));
    say("   B reconciles:", JSON.stringify(await reconcile(KB, st.sid)).slice(0, 300));
    say("6 after reconcile:", JSON.stringify(await check()));
    say("7 A puts after:", await put(KA, st.sid, "up", "after", "written on the new build"));
    const after = await until("B reads 'after'", async () => { const v = await row(KB, st.sid, "up", "after"); return String(v).startsWith("MISSING") ? null : v; }, 300000, 20000).catch(e => e.message);
    say("8 B reads after:", after, `(${Math.round((Date.now() - t0) / 1000)} s)`);
    K3.final = await check();
    K3.after = after;
    for (const [n, u] of [["A", KA], ["B", KB]]) {
      const j = await u.ev(async () => (await (await app.require("lifecycle")).journal()).filter(e => e.m === "group" && /^(keys|follow-keys)/.test(e.event)).map(e => `${e.event}: ${String(e.why ?? "").slice(0, 120)}`)).catch(e => ["ERR " + e.message]);
      say(`9 ${n} key moves:\n     ` + j.join("\n     "));
    }
  }
} catch (e) {
  say("FAILED:", e.message);
  K3.failed = e.message;
}
say("RESULT", JSON.stringify(K3));
await b.close();
