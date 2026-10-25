// THE APPS DRAW (the upgrade gate's last check): Home, a person's Board and Discover's spaces opened on the candidate —
// each must DRAW its own text (an app's frame: `#cw-app`; Discover: the host's page) with NO page error. A read checked
// from the page is not a screen: 10-24 to 10-25 every app frame threw on its boot answer (`ctx.stated` called before the
// runtime was installed) and drew nothing, while every read the gate checked answered (owner: "no discover home page
// loaded").
//   node frames.mjs <fresh>
const { chromium } = await import(process.env.PLAYWRIGHT ?? `${process.env.HOME}/dev/atelier/node_modules/playwright-core/index.mjs`);
const [fresh] = process.argv.slice(2);
const site = `http://127.0.0.1:17691/v1/contract/web/EckhKsQWCG6hSJqhRvxXqBEkFMBCm41YHTKnKRmMznpJ/?fresh=${fresh}`;
const b = await chromium.launch({ headless: true, executablePath: process.env.CHROME });
const PAGES = [
  ["#/discover/board", p => p.frames().find(f => f.url() === "about:srcdoc"), () => document.getElementById("cw-app")?.innerText ?? ""],
  ["#/board", p => p.frames().find(f => f.url() === "about:srcdoc"), () => document.getElementById("cw-app")?.innerText ?? ""],
  ["#/discover/board/spaces", p => p.frames().find(f => f !== p.mainFrame() && f.url() !== "about:srcdoc"), () => document.querySelector(".cw-discover-spaces")?.innerText ?? ""],
];
let failed = 0;
for (const [hash, frameOf, read] of PAGES) {
  const p = await b.newPage();
  const errs = [];
  p.on("pageerror", e => errs.push(e.message.slice(0, 160)));
  await p.goto(site + hash, { timeout: 180000, waitUntil: "commit" });
  let text = "";
  for (let i = 0; i < 36 && !text && !errs.length; i++) {
    await p.waitForTimeout(5000);
    text = ((await frameOf(p)?.evaluate(read).catch(() => "")) ?? "").replace(/\s+/g, " ").trim();
    // A placeholder is not drawn (the owner's Discover read "Loading…" when first checked).
    if (/^(Loading…|Still arriving…)$/.test(text)) text = "";
  }
  const ok = !!text && !errs.length;
  if (!ok) failed += 1;
  console.log(`${ok ? "DRAWS" : "FAILED"} ${hash}: ${JSON.stringify(text.slice(0, 100))}${errs.length ? ` page errors: ${JSON.stringify(errs)}` : ""}`);
  await p.close();
}
await b.close();
console.log(failed ? `FRAMES: FAILED (${failed})` : "FRAMES: PASS");
process.exit(failed ? 1 : 0);
