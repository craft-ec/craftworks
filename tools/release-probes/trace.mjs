// EVERY PROBE PAGE's WHOLE TRACE, ALWAYS (owner 10-29: "why are you keep on guessing and not really logging the whole
// stack?"): each app event (`app.on` — it replays the page's history as it subscribes), every console message and page
// error, each navigation — appended to <OUT>/<probe>-trace/<page>.trace AS THEY HAPPEN (a crash loses nothing). Hooked
// again in every new frame (a reload makes one). A failure is READ from these files, never re-run behind a filter.
import fs from "node:fs";
import path from "node:path";

export function traceDir(probeUrl) {
  const probe = path.basename(new URL(probeUrl).pathname, ".mjs");
  const d = path.join(process.env.OUT ?? `${process.env.TMPDIR ?? "/tmp"}/craftworks-probes`, `${probe}-trace`);
  fs.mkdirSync(d, { recursive: true });
  return d;
}

export async function traced(page, label, dir) {
  const file = path.join(dir, `${label}.trace`);
  const put = line => fs.appendFileSync(file, `${new Date().toISOString().slice(11, 23)} ${line}\n`);
  page.on("console", m => put(`console.${m.type()}: ${m.text()}`));
  page.on("pageerror", e => put(`pageerror: ${e.stack ?? e.message}`));
  page.on("framenavigated", f => put(`navigated: ${f === page.mainFrame() ? "main" : "frame"} ${f.url().slice(0, 160)}`));
  await page.exposeBinding("__probeTrace", (_src, line) => put(line));
  // (Each frame with an app hooked once; every 2 s, any new frame — a reload's — is hooked too.)
  const hook = setInterval(() => {
    for (const f of page.frames())
      f.evaluate(() => {
        if (window.__traced || !globalThis.app?.on || !window.__probeTrace) return;
        window.__traced = true;
        const where = location.href.slice(0, 24);
        app.on(e => window.__probeTrace(`[${where}] ${e.t ?? ""} ${e.step}: ${typeof e.what === "string" ? e.what : JSON.stringify(e.what ?? e)}`));
      }).catch(() => {});
  }, 2000);
  page.on("close", () => clearInterval(hook));
}

// THE NODE's OWN DEBUG TRACE (its delegate: every request, answer and decision), read through a logged-in page of it and
// written beside the page traces as <dir>/node-<label>.trace.
export async function nodeTrace(u, label, dir) {
  const lines = await u.ev(async () => (await (await app.require("auth")).identity.upkeepTrace())?.trace?.lines ?? null).catch(e => [`ERR ${e.message}`]);
  fs.writeFileSync(path.join(dir, `node-${label}.trace`), (lines ?? ["(no trace answered)"]).join("\n") + "\n");
  return lines?.length ?? 0;
}
