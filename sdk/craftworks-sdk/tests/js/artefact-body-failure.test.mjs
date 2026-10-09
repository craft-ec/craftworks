// craftworks-sdk#128: A SOURCE WHOSE BODY FAILS IS A SOURCE THAT FAILED.
//
// `fetch()` resolves when the HEADERS arrive. The body can still fail afterwards — a dropped connection, a
// proxy that cuts the stream — and that failure surfaces from `arrayBuffer()`, not from `fetch()`. The resolver
// caught the second and not the first, so one bad source ended the search and the healthy one was never asked.
//
// Every arm names the sources that were ASKED, because the defect is a request that did not happen.
import assert from "node:assert/strict";
import { webcrypto } from "node:crypto";
import { artefactBytes } from "../../js/artefacts.js";

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${String(e.message).split("\n").join("\n    ")}\n`); }
};
const subtle = webcrypto.subtle;
const hex = b => [...new Uint8Array(b)].map(x => x.toString(16).padStart(2, "0")).join("");
const GOOD = new Uint8Array([1, 2, 3]);
const sha256 = hex(await subtle.digest("SHA-256", GOOD));

/** A network of named sources; records which were asked, in order. */
const net = sources => {
  const asked = [];
  return { asked, fetch: async url => { asked.push(url); const s = sources[url]; if (!s) throw new Error(`no route to ${url}`); return s(); } };
};
const ok = bytes => () => new Response(bytes);
const bodyThrows = message => () => ({ ok: true, status: 200, arrayBuffer: async () => { throw new Error(message); } });
const status = code => () => new Response("", { status: code });
const resolve = (n, urls) => artefactBytes({ urls, sha256 }, { fetch: n.fetch, caches: null, subtle });
/** A person who cancels after the first unanswered round (sdk#312: nothing
 * else ends a wait), with no real sleeping. */
const cancelFirst = () => {
  const c = new AbortController();
  const waits = [];
  return { waits, signal: c.signal, onWait: w => { waits.push(w); c.abort(); }, sleep: async () => {} };
};

await t("a 200 whose BODY is interrupted is that source's failure: the next source is asked and wins", async () => {
  const n = net({ broken: bodyThrows("stream interrupted"), healthy: ok(GOOD) });
  const got = await resolve(n, ["broken", "healthy"]);
  assert.deepEqual(n.asked, ["broken", "healthy"], "the healthy source was never asked");
  assert.deepEqual([...got], [1, 2, 3]);
});

await t("a 200 whose body is TRUNCATED fails the hash and falls through (a different line from the throw)", async () => {
  const n = net({ short: ok(new Uint8Array([1, 2])), healthy: ok(GOOD) });
  const got = await resolve(n, ["short", "healthy"]);
  assert.deepEqual(n.asked, ["short", "healthy"]);
  assert.deepEqual([...got], [1, 2, 3]);
});

await t("invalid bytes, then a body failure, then valid bytes: every source is asked, in order", async () => {
  const n = net({ wrong: ok(new Uint8Array([9, 9, 9])), broken: bodyThrows("reset by peer"), healthy: ok(GOOD) });
  const got = await resolve(n, ["wrong", "broken", "healthy"]);
  assert.deepEqual(n.asked, ["wrong", "broken", "healthy"]);
  assert.deepEqual([...got], [1, 2, 3]);
});

await t("EVERY body failing: the wait names the artefact and what EACH source did, not only the last", async () => {
  const n = net({ a: bodyThrows("stream interrupted"), b: bodyThrows("reset by peer"), c: status(503), d: ok(new Uint8Array([7])) });
  const p = cancelFirst();
  await assert.rejects(artefactBytes({ urls: ["a", "b", "c", "d"], sha256 }, { fetch: n.fetch, caches: null, subtle, ...p }), e => {
    assert.equal(p.waits.length, 1, "it was not a WAIT: a failure that is not a mismatch ended the fetch");
    assert.deepEqual(n.asked, ["a", "b", "c", "d"], "a body failure ended the search early");
    for (const part of [sha256, "a: ", "stream interrupted", "b: ", "reset by peer", "c: 503", "d: does not hash"]) {
      assert.ok(e.message.includes(part), `the error does not say \`${part}\`:\n${e.message}`);
    }
    return true;
  });
});

await t("a body failure is never CACHED as an artefact, and never served", async () => {
  const stored = new Map();
  const caches = { open: async () => ({ match: async k => stored.get(k), put: async (k, r) => { stored.set(k, r); }, delete: async k => stored.delete(k) }) };
  const n = net({ broken: bodyThrows("stream interrupted") });
  await assert.rejects(artefactBytes({ urls: ["broken"], sha256 }, { fetch: n.fetch, caches, subtle, ...cancelFirst() }));
  assert.equal(stored.size, 0, "something was cached for an artefact that never arrived");
});

// ---- controls: green BEFORE and after ----
await t("CONTROL fetch() itself rejecting already falls through", async () => {
  const n = net({ healthy: ok(GOOD) });
  const got = await resolve(n, ["down", "healthy"]);
  assert.deepEqual(n.asked, ["down", "healthy"]);
  assert.deepEqual([...got], [1, 2, 3]);
});
await t("CONTROL the first source that verifies ends the search: a healthy first source means the second is NOT asked", async () => {
  const n = net({ healthy: ok(GOOD), other: ok(GOOD) });
  await resolve(n, ["healthy", "other"]);
  assert.deepEqual(n.asked, ["healthy"], "falling through must not become asking everyone");
});

if (failures) { process.stdout.write(`\n${failures} failed\n`); process.exit(1); }
