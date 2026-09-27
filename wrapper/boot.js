// THE WRAPPER: the only fixed code in an app's site. It fetches the LOADER from the loader's own site — a signed,
// versioned site: a new loader goes live there without touching this file or any app — and hands over.
//
// Everything is read from this machine's freenet node over plain HTTP: a site at /v1/contract/web/<site>/, a package
// at /v1/contract/web/<container>/piece (the SDK's one file name for a single-file container). LOADER_SITE is filled in
// when the app is published.
const LOADER_SITE = "__LOADER_SITE__";
const t0 = performance.now();
const params = new URLSearchParams(location.search);
const web = id => new URL(`/v1/contract/web/${id}/`, location.href);

// THE LOG: one list for the whole page, the wrapper's steps first. `on` shows a listener every event, past and future.
const events = [];
const listeners = [];
const log = (step, detail = {}) => {
  const e = { t: Math.round(performance.now() - t0), step, ...detail };
  events.push(e);
  for (const f of listeners) f(e);
};
const on = f => {
  listeners.push(f);
  for (const e of events) f(e);
};
const hex = buf => [...new Uint8Array(buf)].map(x => x.toString(16).padStart(2, "0")).join("");

// A node answers 503 while it is still fetching a contract from the network ("Loading contract…"): that is "not yet",
// never an answer. So a 503 is asked again, saying what it waits for, until LOADING_LIMIT; then it fails by name.
const LOADING_LIMIT = 120_000;
async function get(url, what) {
  const t = performance.now();
  for (let tries = 1; ; tries++) {
    const r = await fetch(url);
    if (r.ok) return { bytes: new Uint8Array(await r.arrayBuffer()), ms: Math.round(performance.now() - t) };
    const waited = Math.round(performance.now() - t);
    if (r.status !== 503 || waited > LOADING_LIMIT) throw new Error(`${what}: HTTP ${r.status} from ${url.pathname} after ${waited} ms`);
    log("waiting", { what: `${what}: the node is still fetching it (try ${tries})`, ms: waited });
    const s = document.getElementById("status");
    if (s) s.textContent = `Waiting for ${what} (${Math.round(waited / 1000)} s)…`;
    await new Promise(res => setTimeout(res, 1000));
  }
}

// One package: an immutable container, refused unless its bytes hash to what the manifest says.
async function fetchPackage(p, name) {
  const { bytes, ms } = await get(new URL("piece", web(p.address)), name);
  const got = hex(await crypto.subtle.digest("SHA-256", bytes));
  if (got !== p.sha256) throw new Error(`${name}: its bytes hash to ${got.slice(0, 12)}…, the manifest says ${p.sha256.slice(0, 12)}…`);
  return { bytes, ms };
}

const importBytes = bytes => import(URL.createObjectURL(new Blob([bytes], { type: "text/javascript" })));

try {
  log("wrapper started", { what: location.pathname.split("/")[4]?.slice(0, 12) + "…" });
  const base = web(LOADER_SITE);
  const { bytes, ms } = await get(new URL("loader.js", base), "loader");
  log("loaded", { what: "loader", bytes: bytes.length, hash: hex(await crypto.subtle.digest("SHA-256", bytes)).slice(0, 12), ms });
  const loader = await importBytes(bytes);
  // The app's manifest is this site's own file: publishing a new version of the app is a new version of this site.
  const manifest = () => get(new URL("manifest.json", location.href), "manifest");
  await loader.run({ t0, params, log, on, get, manifest, fetchPackage, importBytes, loaderBase: base });
} catch (e) {
  log("FAILED", { what: e.message });
  const s = document.getElementById("status");
  s.textContent = `Could not start: ${e.message}`;
  s.className = "bad";
}
