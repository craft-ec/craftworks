// THE SHARED ARTEFACT CACHE: one download per node, not one per app (sdk#5).
//
// Every assertion here has a control beside it, because the claim is about
// something NOT happening — a second fetch — and "it did not happen" is the
// shape that passes when the mechanism is absent, when the test is wrong, and
// when the environment differs. So each arm runs twice: once with the cache
// and once with it off.

import assert from "node:assert/strict";
import { webcrypto } from "node:crypto";
import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { artefactBytes, allArtefactBytes, CACHE_NAME, ambientCaches } from "../../js/artefacts.js";
import { RTO_SCHEDULE_MS } from "../../js/rto.js";

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.message}\n`); }
};

const subtle = webcrypto.subtle;
const hex = b => [...new Uint8Array(b)].map(x => x.toString(16).padStart(2, "0")).join("");
const sha = async bytes => hex(await subtle.digest("SHA-256", bytes));

/** A Cache Storage stand-in: one Map per named box, shared by every "app". */
function fakeCaches({ failOpen = false, failPut = false } = {}) {
  const boxes = new Map();
  return {
    boxes,
    async open(name) {
      if (failOpen) throw new Error("storage unavailable (private window)");
      if (!boxes.has(name)) boxes.set(name, new Map());
      const m = boxes.get(name);
      return {
        async match(k) { const v = m.get(k); return v ? new Response(v) : undefined; },
        async put(k, res) {
          if (failPut) throw new Error("QuotaExceededError");
          m.set(k, new Uint8Array(await res.arrayBuffer()));
        },
        async delete(k) { return m.delete(k); },
      };
    },
  };
}

/** A fetch that counts, so "no second download" is a number and not a vibe. */
function countingFetch(bytes) {
  let calls = 0;
  const f = async () => { calls += 1; return new Response(bytes); };
  return { fetch: f, calls: () => calls };
}

const PAYLOAD = new Uint8Array(4096).map((_, i) => (i * 7) % 251);

await t("a cold cache fetches, and a second app does not", async () => {
  const sha256 = await sha(PAYLOAD);
  const caches = fakeCaches();
  const net = countingFetch(PAYLOAD);
  const spec = { url: "/shared/sdk.wasm", sha256 };

  const a = await artefactBytes(spec, { fetch: net.fetch, caches, subtle });
  assert.equal(net.calls(), 1, "the first app must actually fetch (cold cache)");
  assert.deepEqual(a, PAYLOAD);

  const b = await artefactBytes(spec, { fetch: net.fetch, caches, subtle });
  assert.equal(net.calls(), 1, "the second app must not fetch again");
  assert.deepEqual(b, PAYLOAD, "and must get the same bytes");
});

await t("CONTROL: with the cache off, the second app fetches again", async () => {
  const sha256 = await sha(PAYLOAD);
  const net = countingFetch(PAYLOAD);
  const spec = { url: "/shared/sdk.wasm", sha256 };

  await artefactBytes(spec, { fetch: net.fetch, caches: null, subtle });
  await artefactBytes(spec, { fetch: net.fetch, caches: null, subtle });
  assert.equal(
    net.calls(), 2,
    "without the cache there must be two fetches, or the test above proves \
nothing about the cache",
  );
});

await t("a POISONED entry is discarded and the bytes are fetched", async () => {
  const sha256 = await sha(PAYLOAD);
  const caches = fakeCaches();
  const net = countingFetch(PAYLOAD);

  // Right key, wrong bytes: what any other script on the origin could write.
  const box = await caches.open(CACHE_NAME);
  await box.put(`/artefact/${sha256}`, new Response(new Uint8Array([1, 2, 3])));

  const got = await artefactBytes(
    { url: "/shared/sdk.wasm", sha256 },
    { fetch: net.fetch, caches, subtle },
  );
  assert.equal(net.calls(), 1, "a claim that does not match its hash must be re-fetched");
  assert.deepEqual(got, PAYLOAD, "and the REAL bytes returned, never the poison");
});

await t("a poisoned entry is not left behind for the next app", async () => {
  const sha256 = await sha(PAYLOAD);
  const caches = fakeCaches();
  const net = countingFetch(PAYLOAD);
  const box = await caches.open(CACHE_NAME);
  await box.put(`/artefact/${sha256}`, new Response(new Uint8Array([9, 9, 9])));

  await artefactBytes({ url: "/u", sha256 }, { fetch: net.fetch, caches, subtle });

  const after = caches.boxes.get(CACHE_NAME).get(`/artefact/${sha256}`);
  assert.deepEqual(
    new Uint8Array(after), PAYLOAD,
    "the poison must be gone, replaced by verified bytes — not left to be \
retried by every app after this one",
  );
  const second = countingFetch(PAYLOAD);
  await artefactBytes({ url: "/u", sha256 }, { fetch: second.fetch, caches, subtle });
  assert.equal(second.calls(), 0, "and the next app must get a clean hit");
});

await t("a poisoned entry is removed even when the cache REFUSES to replace it", async () => {
  // The case that makes the delete load-bearing, found by mutation: while the
  // put succeeds it OVERWRITES the poison, so removing the delete changes
  // nothing and the test above still passes. When the cache is full the put
  // fails — and without the delete the poison stays, and every app after this
  // one reads it, fails its hash check, and re-fetches for ever.
  const sha256 = await sha(PAYLOAD);
  const caches = fakeCaches({ failPut: true });
  // Seeded behind the stand-in's back, because it refuses puts — which is
  // the condition under test.
  await caches.open(CACHE_NAME);
  caches.boxes.get(CACHE_NAME).set(`/artefact/${sha256}`, new Uint8Array([7, 7, 7]));

  const net = countingFetch(PAYLOAD);
  const got = await artefactBytes({ url: "/u", sha256 }, { fetch: net.fetch, caches, subtle });
  assert.deepEqual(got, PAYLOAD, "the caller still gets the real bytes");

  assert.equal(
    caches.boxes.get(CACHE_NAME).has(`/artefact/${sha256}`), false,
    "the poison must be GONE. A bad entry that cannot be replaced must be \
removed, or it is re-tried by every app that follows.",
  );
});

await t("bytes that do not hash to what was promised are never installed", async () => {
  const caches = fakeCaches();
  const net = countingFetch(new Uint8Array([4, 5, 6]));
  const promised = await sha(PAYLOAD);
  // Bounded (sdk#312): the same wrong bytes twice are refused; were they
  // waited on instead, the third wait cancels and the match below fails.
  const c = new AbortController();
  let waits = 0;
  await assert.rejects(
    () => artefactBytes({ url: "/u", sha256: promised }, {
      fetch: net.fetch, caches, subtle,
      sleep: async () => {}, signal: c.signal, onWait: () => { if ((waits += 1) > 2) c.abort(); },
    }),
    /refused: a hash mismatch[\s\S]*does not hash to/,
    "a wrong artefact is not a smaller one: it must be refused",
  );
  assert.equal(caches.boxes.get(CACHE_NAME)?.size ?? 0, 0, "and never cached");
});

await t("an artefact with no hash is refused rather than cached unverified", async () => {
  const net = countingFetch(PAYLOAD);
  await assert.rejects(
    () => artefactBytes({ url: "/u" }, { fetch: net.fetch, caches: fakeCaches(), subtle }),
    /no sha256/,
  );
  assert.equal(net.calls(), 0, "and not even fetched");
});

await t("storage that is UNAVAILABLE falls back to fetching", async () => {
  const sha256 = await sha(PAYLOAD);
  const net = countingFetch(PAYLOAD);
  const got = await artefactBytes(
    { url: "/u", sha256 },
    { fetch: net.fetch, caches: fakeCaches({ failOpen: true }), subtle },
  );
  assert.deepEqual(got, PAYLOAD, "a private window must still load the app");
  assert.equal(net.calls(), 1);
});

await t("storage that REFUSES to store falls back to fetching", async () => {
  const sha256 = await sha(PAYLOAD);
  const caches = fakeCaches({ failPut: true });
  const net = countingFetch(PAYLOAD);

  const a = await artefactBytes({ url: "/u", sha256 }, { fetch: net.fetch, caches, subtle });
  assert.deepEqual(a, PAYLOAD, "a full quota must not fail the load");
  const b = await artefactBytes({ url: "/u", sha256 }, { fetch: net.fetch, caches, subtle });
  assert.deepEqual(b, PAYLOAD);
  assert.equal(net.calls(), 2, "it costs the next app a fetch, which is what it would have paid anyway");
});

await t("all three are fetched together, and a missing one fails the set", async () => {
  const sha256 = await sha(PAYLOAD);
  const caches = fakeCaches();
  const net = countingFetch(PAYLOAD);
  const spec = {
    signer: { url: "/s", sha256 },
    block: { url: "/b", sha256 },
    register: { url: "/r", sha256 },
  };
  const all = await allArtefactBytes(spec, { fetch: net.fetch, caches, subtle });
  assert.deepEqual(Object.keys(all).sort(), ["block", "register", "signer"]);
  // All three are the same bytes here, so the cache makes it ONE fetch — which
  // is the content-addressing working: same hash, same entry, whatever the url.
  assert.ok(net.calls() >= 1);

  await assert.rejects(
    () => allArtefactBytes({ signer: spec.signer }, { fetch: net.fetch, caches, subtle }),
    /no block artefact/,
    "a partial set is not a smaller provisioning",
  );
});

await t("the manifest's hashes ARE the shipped files", async () => {
  // The acceptance that the shared bytes are byte-identical to what a per-app
  // build produced. It is the same question as "is the hash right", because
  // the hash is the only thing an app checks before running the wasm — a
  // manifest that drifts from its files is a cache that can never hit, or
  // worse, one that rejects the correct bytes.
  //
  // NOT a silent skip when the build is missing: a test that quietly passes
  // without the artefacts is the gate this file exists to avoid.
  const dir = new URL("../../pkg/web/", import.meta.url);
  const manifest = JSON.parse(
    await readFile(new URL("artefacts.json", dir), "utf8"),
  );
  let checked = 0;
  for (const name of ["signer", "block", "register", "sdk"]) {
    const entry = manifest[name];
    assert.ok(entry, `artefacts.json has no ${name}`);
    const bytes = new Uint8Array(await readFile(new URL(entry.file, dir)));
    assert.equal(
      await sha(bytes), entry.sha256,
      `${entry.file} does not hash to what artefacts.json claims`,
    );
    assert.equal(bytes.length, entry.bytes, `${entry.file} is not ${entry.bytes} B`);
    checked += 1;
  }
  assert.equal(checked, 4, "every artefact must be checked, not some of them");
  // The artefacts CONTAINER (builder#104): the shipped file is what the
  // manifest says, and it names the address a builder PUTs it under.
  const c = manifest.container;
  assert.ok(c && c.address && c.sha256 && c.xz, `artefacts.json has no usable container entry: ${JSON.stringify(c)}`);
  const cbytes = new Uint8Array(await readFile(new URL("artefacts.webapp", dir)));
  assert.equal(await sha(cbytes), c.sha256, "artefacts.webapp does not hash to what artefacts.json claims");
  assert.equal(cbytes.length, c.bytes);
  // The `webapp` CODE ships beside it, and is what the manifest says.
  const w = manifest.webapp;
  assert.ok(w && w.file === "webapp.wasm", `artefacts.json has no webapp entry: ${JSON.stringify(w)}`);
  const wbytes = new Uint8Array(await readFile(new URL(w.file, dir)));
  assert.equal(await sha(wbytes), w.sha256, "webapp.wasm does not hash to what artefacts.json claims");
  assert.equal(wbytes.length, w.bytes);
  // And the three AGREE: the shipped container under the shipped code, as
  // the wasm derives its address, is the address the manifest advertises.
  // Otherwise every app would name an artefacts address nobody published.
  const { webapp_address } = createRequire(import.meta.url)("../../pkg/node/craftworks_sdk.js");
  assert.equal(webapp_address(wbytes, cbytes), c.address, "the shipped container's address is not the manifest's");
  const tampered = cbytes.slice(); tampered[tampered.length - 1] ^= 1;
  assert.notEqual(webapp_address(wbytes, tampered), c.address, "THE CONTROL: a different container got the same address");
  const otherCode = wbytes.slice(); otherCode[0] ^= 1;
  assert.notEqual(webapp_address(otherCode, cbytes), c.address, "THE CONTROL: other contract code got the same address");
  // The `site` CODE (builder#117) ships beside it too: an app's stable link is a site contract under it.
  const st = manifest.site;
  assert.ok(st && st.file === "site.wasm", `artefacts.json has no site entry: ${JSON.stringify(st)}`);
  const sbytes = new Uint8Array(await readFile(new URL(st.file, dir)));
  assert.equal(await sha(sbytes), st.sha256, "site.wasm does not hash to what artefacts.json claims");
  assert.equal(sbytes.length, st.bytes);
});

// ---------------------------------------------------------------------------
// ONE HASH, SEVERAL PLACES TO GET IT.
//
// An app that named ONE source for its artefacts is dead whenever that source
// is unavailable — and if every app names the same one, every app is dead
// together. The hash is the identity, so a second source costs nothing in
// trust: it cannot serve anything different, because anything different does
// not hash to this.
// ---------------------------------------------------------------------------

await t("**the first source that VERIFIES wins, not the first that answers**", async () => {
  const good = new Uint8Array([1, 2, 3]);
  const sha256 = await sha(good);
  const asked = [];
  const fetchWith = async url => {
    asked.push(url);
    // A source that answers 200 with the WRONG bytes. It must not end the
    // search: a wrong artefact is not a smaller one.
    if (url === "a") return new Response(new Uint8Array([9, 9, 9]));
    if (url === "b") return new Response(good);
    return new Response("", { status: 404 });
  };
  const bytes = await artefactBytes(
    { urls: ["a", "b"], sha256 },
    { fetch: fetchWith, caches: null, subtle: crypto.subtle },
  );
  assert.deepEqual([...bytes], [...good], "it took the bytes that did not verify");
  assert.deepEqual(asked, ["a", "b"], "it stopped at the first source that merely answered");
});

await t("a source that is DOWN is skipped, not fatal", async () => {
  const good = new Uint8Array([4, 5, 6]);
  const sha256 = await sha(good);
  const fetchWith = async url => {
    if (url === "down") throw new Error("connection refused");
    return new Response(good);
  };
  const bytes = await artefactBytes(
    { urls: ["down", "up"], sha256 },
    { fetch: fetchWith, caches: null, subtle: crypto.subtle },
  );
  assert.deepEqual([...bytes], [...good], "one unreachable source killed the lot");
});

await t("**a wait names the artefact AND what each source did**", async () => {
  // An app that will not open is the symptom a person reports, so what it
  // shows must identify the block and say what was tried. A 404 beside a
  // mismatch is a WAIT (the 404 source may yet serve it), not an end.
  const sha256 = await sha(new Uint8Array([7]));
  const fetchWith = async url => {
    if (url === "missing") return new Response("", { status: 404 });
    return new Response(new Uint8Array([8]));      // wrong bytes
  };
  const c = new AbortController();
  const waits = [];
  await assert.rejects(
    () => artefactBytes({ urls: ["missing", "wrong"], sha256 }, {
      fetch: fetchWith, caches: null, subtle: crypto.subtle,
      signal: c.signal, onWait: w => { waits.push(w); c.abort(); }, sleep: async () => {},
    }),
    e => {
      assert.equal(waits.length, 1, "a 404 beside a mismatch ended the fetch instead of waiting");
      assert.match(waits[0].failures.join("\n"), /missing: 404/, "the wait does not say what the first source did");
      assert.match(e.message, new RegExp(sha256), "the message does not name WHICH artefact");
      assert.match(e.message, /cancelled/, "only a person's cancel ends a wait");
      assert.match(e.message, /missing: 404/, "it does not say what the first source did");
      assert.match(e.message, /wrong: does not hash/, "it does not say what the second source did");
      return true;
    },
  );
});

// ---------------------------------------------------------------------------
// A NODE THAT DOES NOT HOLD IT YET HAS NOT ANSWERED (sdk#312).
// ---------------------------------------------------------------------------

/** A source that answers `status` for its first `n` asks, then the bytes. */
const notYet = (n, good, status = 404) => {
  const asked = { n: 0 };
  return { asked, fetch: async () => (++asked.n <= n ? new Response("", { status }) : new Response(good)) };
};

await t("**the first 3 fetches 404, then 200: the app's code arrives, re-asked on the page's RTO**", async () => {
  const good = new Uint8Array([3, 1, 2]);
  const sha256 = await sha(good);
  const src = notYet(3, good);
  const waits = [];
  const slept = [];
  const bytes = await artefactBytes({ url: "node", sha256 }, {
    fetch: src.fetch, caches: null, subtle: crypto.subtle,
    onWait: w => waits.push(w), sleep: async ms => { slept.push(ms); },
  });
  assert.deepEqual([...bytes], [...good], "the artefact did not arrive after the node came to hold it");
  assert.equal(src.asked.n, 4, "it did not ask again after each 404");
  assert.deepEqual(slept, RTO_SCHEDULE_MS.slice(0, 3), "the waits are not the page's RTO back-off");
  assert.equal(waits.length, 3);
  assert.match(waits[2].says, /^loading the app… not available on this node yet \(\d+ s\)$/, waits[2].says);
});

await t("a 503, a 400 and a network error are NOT answers either", async () => {
  const good = new Uint8Array([5]);
  const sha256 = await sha(good);
  for (const fail of [503, 400, "network"]) {
    let n = 0;
    const fetchWith = async () => {
      n += 1;
      if (n === 1) { if (fail === "network") throw new Error("connection reset"); return new Response("", { status: fail }); }
      return new Response(good);
    };
    const bytes = await artefactBytes({ url: "x", sha256 }, { fetch: fetchWith, caches: null, subtle: crypto.subtle, sleep: async () => {} });
    assert.deepEqual([...bytes], [...good], `${fail} ended the fetch`);
  }
});

await t("the wait is the page's RTO back-off to its ceiling, then the ceiling for ever", async () => {
  assert.ok(RTO_SCHEDULE_MS.length > 2 && RTO_SCHEDULE_MS[0] < RTO_SCHEDULE_MS.at(-1), `a schedule that does not back off: ${RTO_SCHEDULE_MS}`);
  const good = new Uint8Array([6]);
  const sha256 = await sha(good);
  const src = notYet(RTO_SCHEDULE_MS.length + 5, good, 503);
  const slept = [];
  await artefactBytes({ url: "x", sha256 }, { fetch: src.fetch, caches: null, subtle: crypto.subtle, sleep: async ms => { slept.push(ms); } });
  assert.deepEqual(slept.slice(0, RTO_SCHEDULE_MS.length), [...RTO_SCHEDULE_MS]);
  assert.ok(slept.slice(RTO_SCHEDULE_MS.length).every(ms => ms === RTO_SCHEDULE_MS.at(-1)), `past the ceiling: ${slept}`);
});

await t("**a hash MISMATCH from every source, the SAME wrong bytes twice, is the one refusal: named**", async () => {
  const sha256 = await sha(new Uint8Array([1]));
  let waited = 0;
  // Bounded: were the same wrong answer waited on past its second showing, the
  // next wait cancels it, and the rejection says `cancelled` rather than
  // `refused`, so it fails here instead of spinning.
  const c = new AbortController();
  await assert.rejects(
    () => artefactBytes({ urls: ["a", "b"], sha256 }, {
      fetch: async () => new Response(new Uint8Array([2])), caches: null, subtle: crypto.subtle,
      onWait: () => { if ((waited += 1) > 1) c.abort(); }, signal: c.signal, sleep: async () => {},
    }),
    e => /refused: a hash mismatch/.test(e.message) && e.message.includes(sha256) && /a: does not hash.*the same again/.test(e.message),
  );
  assert.equal(waited, 1, "the same wrong bytes were not asked for exactly once more before refusing");
});

await t("**a TORN 200 (truncated, hashing wrong) is not a refusal: re-asked, and the whole file opens**", async () => {
  // A node unpacking its web cache served 1,242,614 of 1,259,519 bytes for a
  // moment (engineer1, measured). One wrong hash is a torn read, not an answer.
  const good = new Uint8Array([1, 2, 3, 4, 5, 6]);
  const sha256 = await sha(good);
  let n = 0;
  const bytes = await artefactBytes({ url: "node", sha256 }, {
    fetch: async () => new Response((n += 1) === 1 ? good.slice(0, 4) : good),
    caches: null, subtle: crypto.subtle, sleep: async () => {},
  });
  assert.deepEqual([...bytes], [...good]);
  assert.equal(n, 2, "the torn read was not re-asked");
});

await t("DIFFERENT wrong bytes each round keep it waiting: only the same wrong answer twice ends it", async () => {
  const sha256 = await sha(new Uint8Array([0]));
  let n = 0;
  const c = new AbortController();
  let waits = 0;
  await assert.rejects(
    () => artefactBytes({ url: "node", sha256 }, {
      fetch: async () => new Response(new Uint8Array([(n += 1)])), caches: null, subtle: crypto.subtle,
      onWait: () => { if ((waits += 1) === 3) c.abort(); }, signal: c.signal, sleep: async () => {},
    }),
    /cancelled/,
  );
  assert.equal(waits, 3, "a changing wrong answer was refused as though it were the same one");
});

await t("THE rto.js SCHEDULE IS THE ONE page/src/rto.rs PRODUCES, not a copy", async () => {
  // Generated by `build.sh` from the real `Rto`; re-run here, so a hand edit
  // of js/rto.js (or a stale one) is caught.
  const { execFileSync } = await import("node:child_process");
  const out = execFileSync("cargo", ["run", "-q", "-p", "page", "--example", "rto_js"], { encoding: "utf8" });
  assert.equal(await readFile(new URL("../../js/rto.js", import.meta.url), "utf8"), out, "js/rto.js is not what the Rto produces");
});

await t("THE CONTROL: a single `url` still works, unchanged", async () => {
  // Every existing caller passes one url. If the list form had replaced it
  // rather than joined it, they would all fail — and this is the shape the
  // manifest still produces today.
  const good = new Uint8Array([1]);
  const sha256 = await sha(good);
  const bytes = await artefactBytes(
    { url: "only", sha256 },
    { fetch: async () => new Response(good), caches: null, subtle: crypto.subtle },
  );
  assert.deepEqual([...bytes], [...good]);
});

await t("THE CONTROL: no url at all is refused, not silently empty", async () => {
  await assert.rejects(
    () => artefactBytes({ sha256: "abc" }, { fetch: async () => new Response(new Uint8Array()), caches: null, subtle: crypto.subtle }),
    /no url to fetch it from/,
  );
});

// ---------------------------------------------------------------------------
// FETCHING A NAMED ARTEFACT: where caches cannot be read, and what refuses.
// ---------------------------------------------------------------------------

await t("**in a SANDBOXED frame, where even READING `caches` throws, an artefact is still fetched and verified**", async () => {
  // How a node serves every web app (builder#104): an iframe sandboxed without
  // allow-same-origin, where the `caches` getter throws a SecurityError. The
  // DEFAULT is what a loader uses, so the default is what is tested.
  const bytes = new Uint8Array([4, 5, 6]);
  const sha256 = await sha(bytes);
  const had = Object.getOwnPropertyDescriptor(globalThis, "caches");
  Object.defineProperty(globalThis, "caches", { configurable: true, get() { throw new DOMException("Cache storage is disabled because the context is sandboxed", "SecurityError"); } });
  try {
    assert.throws(() => globalThis.caches, /sandboxed/, "THE CONTROL: the getter really throws, as a sandboxed frame's does");
    assert.equal(ambientCaches(), null);
    const got = await artefactBytes({ urls: ["https://node/v1/contract/web/ARTEFACTS/block.wasm"], sha256 }, { fetch: async () => new Response(bytes) });
    assert.deepEqual(new Uint8Array(got), bytes);
  } finally {
    if (had) Object.defineProperty(globalThis, "caches", had); else delete globalThis.caches;
  }
  const store = { open: async () => ({}) };
  Object.defineProperty(globalThis, "caches", { configurable: true, value: store });
  try { assert.equal(ambientCaches(), store, "an ordinary page's Cache Storage is still used"); }
  finally { if (had) Object.defineProperty(globalThis, "caches", had); else delete globalThis.caches; }
});

await t("**THE CONTROL THAT MATTERS: bytes that do not match the named hash are REFUSED**", async () => {
  // The acceptance names this one specifically. A shared artefact is only
  // safe because the hash is checked — an app must not load something
  // unverified, it must not load at all.
  const sha256 = await sha(new Uint8Array([1, 2, 3]));
  const cancel = new AbortController();
  let waits = 0;
  await assert.rejects(
    () =>
      artefactBytes(
        { urls: ["https://node/v1/contract/web/ARTEFACTS/signer.wasm"], sha256 },
        {
          fetch: async () => new Response(new Uint8Array([9, 9, 9])),
          caches: null,
          subtle: crypto.subtle,
          // Bounded (sdk#312): refused on the same wrong bytes' second
          // showing; waited on instead, the third wait cancels and fails here.
          sleep: async () => {},
          signal: cancel.signal,
          onWait: () => { if ((waits += 1) > 2) cancel.abort(); },
        },
      ),
    e => {
      assert.match(e.message, /refused: a hash mismatch/, "it was not REFUSED");
      assert.match(e.message, /does not hash/, "it did not say the bytes failed verification");
      assert.match(e.message, new RegExp(sha256), "it did not name the artefact");
      return true;
    },
  );
});

await t("`url` and `urls` together is refused, not silently half-used", async () => {
  // Taking either one drops a source the caller believed it supplied, and
  // the symptom is an artefact resolving from the wrong place with no hint
  // that half the request was discarded.
  await assert.rejects(
    () => artefactBytes(
      { url: "a", urls: ["b"], sha256: "f".repeat(64) },
      { fetch: async () => new Response(new Uint8Array()), caches: null, subtle: crypto.subtle },
    ),
    /both .url. and .urls./,
  );
});

process.stdout.write(failures ? `\n${failures} failing\n` : "\nall ok\n");
process.exit(failures ? 1 : 0);
