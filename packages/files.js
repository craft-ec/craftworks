// FILES, a capability (ARCHITECTURE §6): a file as encrypted chunks, RLNC-coded in generations of 16, every piece a
// `sealed` contract at an address only the file's key gives; the index (a tree) goes up last, so a file reads only
// once complete. The KEY comes from the content and the file's SPACE's dedup salt (the account is the personal
// space; a PUBLIC file: the content alone), so the same file is stored once, and an upload RESUMES — the same key
// makes the same fragments at the same addresses (progress kept in the account's table `uploads`).
//
// RACED both ways: an upload sends a generation's fragments at once and replaces one not stored by a freshly minted
// fragment; a read asks a generation's listed fragments at once and decodes on the first 16 valid, independent ones.
// Files up to 64 KiB are not coded: they ride inline in the reference.
//
// THE KEY'S ONE OWNER: a coded file's current key and root live in ONE row of its space's table `files`, `k/<id>` →
// { key, root, h, pub, app, n } (n: the salt it is keyed under; -1: public; -2: adopted from another space). A reference
// names the file by `id` and `in` (its space; null: the account), and a reader who can read that space's rows reads
// the row: the key the reference carries is for readers outside it. Re-keying (`file-keys`) changes the row alone.
//
//   const files = await ctx.require("files");
//   const ref = await files.put(file, { space, public, app, onProgress })  // { id, in, key, root, size, name, type }, or
//                                                                            // { inline, size, name, type }
//   const blob = await files.get(ref, { onProgress })                        // the whole file
//   for await (const bytes of files.stream(ref, { from: 0 })) …             // a generation at a time, in order
//   const bytes = await files.chunk(ref, i)                                 // one chunk alone (a seek)
//   const bytes = await files.range(ref, start, length)                     // a byte range (its chunks alone)
//   const ref2 = await files.adopt(ref, space)          // a file from another space: listed in this one (then copied)
//   await files.publicity(refs, space, pub)             // the items holding them are (not) read by anyone now
export async function start(ctx) {
  const { core, glue, ask, slot, WAIT } = await ctx.require("node");
  const [storage, space] = await Promise.all(["storage", "space"].map(n => ctx.require(n)));
  core.set_sealed_code(await ctx.require("sealed-wasm"));
  core.set_piece_code(await ctx.require("piece-wasm"));
  const INLINE_MAX = 64 * 1024;
  const GEN = 16;
  const EXTRA = 8;
  const MAX_LISTED = 32;
  const GENS_IN_FLIGHT = 6;
  const SLICE = 4 * 1024 * 1024;
  const hex = b => [...b].map(x => x.toString(16).padStart(2, "0")).join("");
  const bytes = h => new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
  const b64 = b => btoa(Array.from(b, c => String.fromCharCode(c)).join(""));
  const unb64 = s => Uint8Array.from(atob(s), c => c.charCodeAt(0));

  const shared = sp => sp && sp.kind !== "account";
  const table = async sp => {
    const t = shared(sp) ? await storage.table(space.tableOf(sp, "files"), sp) : await storage.table("files");
    await t.settled;
    return t;
  };
  // A SPACE's dedup SALT: a secret in its table `files` — `{ s, n, removals }` (n counts the salts it has had; removals:
  // the removals it was made after) — made by the first upload there (two made at once: the merge keeps one, and a
  // file keyed by the other is still the file it is). A new one after a removal (`rotate`, by `file-keys`).
  const parseSalt = v => {
    if (!v) return null;
    if (/^[0-9a-f]{64}$/.test(v)) return { s: bytes(v), n: 0, removals: 0 };
    try {
      const o = JSON.parse(v);
      return { s: bytes(o.s), n: o.n | 0, removals: o.removals | 0 };
    } catch {
      return null;
    }
  };
  // One salt made at a time per space here (the first upload and `file-keys` asking at once made two).
  const making = new Map();
  async function salt(sp) {
    const t = await table(sp);
    const had = parseSalt(t.rows().find(r => r.key === "salt")?.value);
    if (had) return had;
    const k = sp?.id ?? "";
    if (!making.has(k))
      making.set(
        k,
        (async () => {
          const s = crypto.getRandomValues(new Uint8Array(32));
          await t.put(`salt/0`, hex(s));
          await t.put("salt", JSON.stringify({ s: hex(s), n: 0, removals: 0 }));
          return { s, n: 0, removals: 0 };
        })().finally(() => making.delete(k)),
      );
    return making.get(k);
  }
  async function rotate(sp, removals) {
    const t = await table(sp);
    const was = await salt(sp);
    const s = crypto.getRandomValues(new Uint8Array(32));
    // Each salt KEPT (`salt/<n>`): burning a file keyed under an older one needs it.
    if (!t.rows().some(r => r.key === `salt/${was.n}` && r.value)) await t.put(`salt/${was.n}`, hex(was.s));
    await t.put(`salt/${was.n + 1}`, hex(s));
    await t.put("salt", JSON.stringify({ s: hex(s), n: was.n + 1, removals }));
    return { s, n: was.n + 1, removals };
  }
  // BURNING (the `piece` contract): a file's pieces name the sha-256 of a SECRET only its space's salt then gives; a
  // public file names zero (never burned). A file keyed before burning has none: its pieces only fade.
  const sha = async b => new Uint8Array(await crypto.subtle.digest("SHA-256", b));
  const secretOf = async (s, key) => sha(new Uint8Array([...new TextEncoder().encode("craftworks burn"), ...s, ...key]));

  const keyFor = (h, s) => hex(glue.file_key(bytes(h), s ?? new Uint8Array(0)));

  // THE KEY ROWS: `k/<id>`.
  const parseRow = r => {
    try {
      const v = JSON.parse(r.value);
      return v?.key && v?.root ? { id: r.key.slice(2), ...v } : null;
    } catch {
      return null;
    }
  };
  const rows = async sp => (await table(sp)).rows().filter(r => r.key.startsWith("k/") && r.value).map(parseRow).filter(Boolean);
  async function rowOf(sp, id) {
    const r = (await table(sp)).rows().find(x => x.key === `k/${id}` && x.value);
    return r ? parseRow(r) : null;
  }
  const setRow = async (sp, row) => {
    const { id, ...v } = row;
    await (await table(sp)).put(`k/${id}`, JSON.stringify({ ...v, at: Date.now() }));
  };
  const inOf = sp => (shared(sp) ? sp.id : null);
  const spaceOf = async id => (id ? ((await space.mine().catch(() => [])).find(s => s.id === id) ?? undefined) : null);

  // A REFERENCE AS IT IS NOW: its key and root from its space's row, where this person reads that space.
  async function current(ref) {
    if (!ref || ref.inline || !ref.id) return ref;
    const sp = await spaceOf(ref.in);
    if (sp === undefined) return ref;
    const row = await rowOf(sp, ref.id).catch(() => null);
    if (!row) return ref;
    const { b, ...was } = ref;
    return { ...was, key: row.key, root: row.root, ...(row.b !== undefined ? { b: row.b } : {}) };
  }

  // ADOPT a file into a space (attached there from another space's Drive, saved to yours): listed at once with the key
  // it has, due to be COPIED under this space's salt (`file-keys`). Its hash comes along when this person reads it.
  async function adopt(ref, sp, { app = null, pub = false } = {}) {
    if (!ref || ref.inline || (ref.id && (ref.in ?? null) === inOf(sp))) return ref;
    const now = await current(ref);
    const from = ref.id ? await spaceOf(ref.in) : undefined;
    const src = from !== undefined && ref.id ? await rowOf(from, ref.id).catch(() => null) : null;
    const id = ref.id ?? ref.root;
    const had = await rowOf(sp, id).catch(() => null);
    if (!had) await setRow(sp, { id, key: now.key, root: now.root, ...(now.b !== undefined ? { b: now.b } : {}), ...(src?.h ? { h: src.h } : {}), pub: !!pub, app, n: -2 });
    return { ...now, id, in: inOf(sp) };
  }

  // PUBLICITY: the items holding these files are read by anyone (or no longer): their rows say so, and a row keyed
  // public that no longer is becomes due (a public key is never taken back, only replaced).
  async function publicity(refs, sp, pub) {
    for (const ref of refs ?? []) {
      if (!ref?.id || (ref.in ?? null) !== inOf(sp)) continue;
      const row = await rowOf(sp, ref.id).catch(() => null);
      if (row && !!row.pub !== !!pub) await setRow(sp, { ...row, pub: !!pub });
    }
  }

  // One PUT: whether the node stored it (a refusal or no answer in 60 s: not).
  const putOne = ([, id, frames], what) =>
    ask(frames, x => (x.kind === "put" && x.key === id) || x.kind === "refused", what, 60000).then(
      x => x.kind === "put",
      () => false,
    );

  async function put(file, { space: sp = null, public: pub = false, app = null, inline = true, made = false, onProgress = () => {} } = {}) {
    const size = file.size;
    const name = file.name ?? "file";
    const type = file.type || "application/octet-stream";
    // `inline: false`: coded even when small (what must not ride in a row: a video's manifest, which grows).
    if (inline && size <= INLINE_MAX) return { inline: b64(new Uint8Array(await file.arrayBuffer())), size, name, type };
    // The content's hash, a slice at a time.
    const h = new glue.FileHasher();
    for (let at = 0; at < size; at += SLICE) {
      h.update(new Uint8Array(await file.slice(at, at + SLICE).arrayBuffer()));
      onProgress({ phase: "reading", done: Math.min(at + SLICE, size), size });
    }
    const hash = h.finish();
    const st = pub ? null : await salt(sp);
    const key = glue.file_key(hash, pub ? new Uint8Array(0) : st.s);
    const keyHex = hex(key);
    const secret = pub ? null : await secretOf(st.s, key);
    const burn = secret ? await sha(secret) : new Uint8Array(32);
    const root = await store(key, size, name, type, (a, b) => file.slice(a, b).arrayBuffer().then(x => new Uint8Array(x)), onProgress, null, burn, made ? { made: true } : {});
    const id = root;
    // The row keeps the BURN SECRET (`x`): whoever re-keys the file later burns these pieces with it.
    if (!(await rowOf(sp, id).catch(() => null))) await setRow(sp, { id, key: keyHex, root, b: hex(burn), ...(secret ? { x: hex(secret) } : {}), h: hex(hash), pub: !!pub, app, n: pub ? -1 : st.n });
    return { id, in: inOf(sp), key: keyHex, root, b: hex(burn), size, name, type };
  }

  // The KEY a file would have (public: its content alone; otherwise with its space's salt) — without storing it: what
  // a video's id is made from when its original is not kept.
  async function keyOf(file, { space: sp = null, public: pub = false } = {}) {
    const h = new glue.FileHasher();
    for (let at = 0; at < file.size; at += SLICE) h.update(new Uint8Array(await file.slice(at, at + SLICE).arrayBuffer()));
    return hex(glue.file_key(h.finish(), pub ? new Uint8Array(0) : (await salt(sp)).s));
  }

  // STORE a file under `key`: each generation (`slice(from, to)` → its plaintext) coded and put, then the index.
  // RESUMES from its PROGRESS: an upload's in the account's table `uploads` (by the key); a re-key's in the space's
  // table (`p/<id>`: whichever member takes the work over goes on from there). Returns the index root.
  const uploadsProgress = async keyHex => {
    const ups = await storage.table("uploads");
    await ups.settled;
    return {
      read: () => ups.rows().find(r => r.key === keyHex)?.value,
      write: v => ups.put(keyHex, v),
      done: () => ups.remove(keyHex),
    };
  };
  async function progressOf(sp, id) {
    const t = await table(sp);
    const k = `p/${id}`;
    const at = () => {
      try {
        return JSON.parse(t.rows().find(r => r.key === k && r.value)?.value ?? "null");
      } catch {
        return null;
      }
    };
    return { at, read: () => t.rows().find(r => r.key === k && r.value)?.value, write: v => t.put(k, v), done: () => t.remove(k) };
  }
  // `extra`: kept with its progress (`made`: bytes a page made, not a file a person picked — not listed as their upload).
  async function store(key, size, name, type, slice, onProgress = () => {}, progress = null, burn = new Uint8Array(32), extra = {}) {
    const keyHex = hex(key);
    const plan = JSON.parse(glue.file_plan(size));
    // RESUME: the generations already stored (the same key: the same fragments at the same addresses).
    const prog = progress ?? (await uploadsProgress(keyHex));
    let prev = null;
    try {
      prev = JSON.parse(prog.read() ?? "null");
    } catch {}
    const stored = prev?.key === undefined || prev.key === keyHex ? (prev?.stored ?? []) : [];
    const genBytes = (plan.chunk * GEN);
    // SEVERAL GENERATIONS IN FLIGHT: one generation waits on its slowest put (a round trip through the network, up to
    // the put's deadline), so one at a time ran at that latency, not the link (measured 2026-10-06: 56 KB/s of a
    // 1.36 MB/s link). Each finished generation is recorded at once, so a resume still skips exactly the stored ones.
    let sentBytes = stored.reduce((n, s, g) => n + (s?.length >= Math.min(GEN, plan.chunks - g * GEN) + EXTRA ? Math.min(genBytes, size - g * genBytes) : 0), 0);
    let recording = Promise.resolve();
    const record = () => (recording = recording.then(() => prog.write(JSON.stringify({ key: keyHex, name, size, type, stored, at: Date.now(), ...extra })).catch(() => {})));
    let nextGen = 0;
    let failed = null;
    const worker = async () => {
      while (!failed && nextGen < plan.gens) {
        const g = nextGen++;
        try {
          await generation(g);
        } catch (e) {
          failed ??= e;
        }
      }
    };
    await Promise.all(Array.from({ length: Math.min(GENS_IN_FLIGHT, plan.gens) }, worker));
    await recording;
    if (failed) throw failed;
    async function generation(g) {
      const k = Math.min(GEN, plan.chunks - g * GEN);
      if ((stored[g]?.length ?? 0) >= k + EXTRA) return;
      const plain = await slice(g * genBytes, Math.min(size, (g + 1) * genBytes));
      const have = new Set((stored[g] ?? []).map(x => x[0]));
      const ok = [...(stored[g] ?? [])];
      const sent = Array.from(core.file_encode(key, size, g, plain, EXTRA, burn)).filter(f => !have.has(f[0]));
      await Promise.all(sent.map(f => putOne(f, `${name}: generation ${g}`).then(y => y && ok.push([f[0], f[3]]))));
      // RATELESS: a fragment not stored is replaced by a new one, not waited on.
      let next = Math.max(k + EXTRA, ...ok.map(x => x[0] + 1));
      while (ok.length < k + EXTRA && next < MAX_LISTED) {
        const f = core.file_mint(key, size, g, plain, next++, burn);
        if (await putOne(f, `${name}: generation ${g}, fragment ${f[0]}`)) ok.push([f[0], f[3]]);
      }
      if (ok.length < k) throw new Error(`${name}: generation ${g} could not be stored (${ok.length} of the ${k} it needs)`);
      stored[g] = ok.sort((a, b) => a[0] - b[0]);
      record();
      sentBytes += plain.length;
      onProgress({ phase: "sending", done: Math.min(size, sentBytes), size });
    }
    // The INDEX last: once it is there, the file reads.
    // HASH-ADDRESSED: each index piece at its own hash's address — two uploads of one file that stored other
    // fragments never meet at one address (the network keeps the first; the second would read as forged).
    const idx = core.file_index(key, size, JSON.stringify(stored), burn);
    const puts = Array.from(idx.puts);
    const ok = await Promise.all(puts.slice(0, -1).map(p => putOne([0, ...p], `${name}: its index`)));
    if (ok.includes(false) || !(await putOne([0, ...puts.at(-1)], `${name}: its root`))) throw new Error(`${name}: its index could not be stored (the upload resumes)`);
    await prog.done().catch(() => {});
    onProgress({ phase: "done", done: size, size });
    return idx.root;
  }

  // One GET: the state, taken (a file's pieces are read once, never kept in the core), or null. A piece's state is
  // `LIVE ‖ burn hash ‖ piece` (its piece given back), or burned (none: as good as missing); `raw`: as stored.
  async function fetchState(idHex, what, { raw = false } = {}) {
    // An answer that came LATE (after an earlier ask gave up) is kept by the core: taken now, not asked again.
    // Measured 2026-10-06: a 4K play logged hundreds of pieces arriving after their ask's deadline, each then asked again.
    const late = core.take_got(idHex);
    if (late) return raw ? late : livePiece(late);
    const [, frames] = core.frames_get(bytes(idHex));
    // Through the node's one cap on gets (`slot`).
    const said = await slot(() => ask(frames, x => (x.kind === "got" || x.kind === "get-failed") && x.id === idHex, what, WAIT.ask).catch(() => ({ kind: "get-failed" })));
    const st = said.kind === "got" ? core.take_got(idHex) : null;
    return !st || raw ? st : livePiece(st);
  }
  // A piece's state `LIVE ‖ burn hash ‖ piece`: the piece (burned, or another shape: none).
  const livePiece = st => (st[0] === 2 && st.length > 33 ? st.subarray(33) : null);

  // A file OPENED for reading: its root, and each generation's listed fragments (leaves read once).
  const opened = new Map();
  function open(ref) {
    if (!opened.has(ref.root))
      opened.set(
        ref.root,
        (async () => {
          const key = bytes(ref.key);
          // Each index piece fetched by its hash (the reference's for the root).
          const rootId = core.file_root_id(key, ref.root);
          // As stored: a live piece's body, or — when not — which: not on the node at all, or there but not live.
          const raw = await fetchState(rootId, `${ref.name}: its root`, { raw: true });
          const state = raw && livePiece(raw);
          if (!state) {
            const how = raw ? `there but not live (first byte ${raw[0]}, ${raw.length} B)` : "not on the node";
            ctx.log("files", { what: `${ref.name}: its root ${rootId.slice(0, 12)}… ${how} (key ${ref.key.slice(0, 8)}…, root ${ref.root.slice(0, 8)}…)` });
            throw new Error(`${ref.name}: not found on the network (yet)`);
          }
          const root = JSON.parse(glue.file_root(key, ref.root, state));
          // The hash of index piece `n` at `level` (from its parent, read once): what a hash-addressed one is fetched by.
          const inner = new Map();
          const indexHash = async (level, n) => {
            if (level === root.depth) return root.children[n];
            const at = Math.floor(n / 7000);
            const k = `${level + 1}/${at}`;
            if (!inner.has(k))
              inner.set(
                k,
                (async () => {
                  const h = await indexHash(level + 1, at);
                  const st = await fetchState(core.file_index_id(key, h), `${ref.name}: its index`);
                  if (!st) throw new Error(`${ref.name}: part of its index is missing`);
                  return glue.file_inner(key, h, st);
                })().catch(e => (inner.delete(k), Promise.reject(e))),
              );
            return (await inner.get(k))[n % 7000];
          };
          const leaves = new Map();
          // The leaf that lists generation g, down from the root through any inner levels.
          const leafOf = async g => {
            const n = Math.floor(g / 192);
            if (!leaves.has(n))
              leaves.set(
                n,
                (async () => {
                  const h = await indexHash(0, n);
                  const st = await fetchState(core.file_index_id(key, h), `${ref.name}: its index`);
                  if (!st) throw new Error(`${ref.name}: part of its index is missing`);
                  return JSON.parse(glue.file_leaf(key, root.plan.size, n, h, st));
                })(),
              );
            return (await leaves.get(n))[g % 192];
          };
          return { key, depth: root.depth, plan: root.plan, listed: leafOf, rootHash: ref.root, indexHash };
        })().catch(e => (opened.delete(ref.root), Promise.reject(e))),
      );
    return opened.get(ref.root);
  }

  // A GENERATION, RACED: every listed fragment asked at once, decoded on the first k valid, independent ones.
  async function generation(ref, f, g) {
    const listed = await f.listed(g);
    const d = new glue.FileDecoder(f.key, f.plan.size, g, JSON.stringify(listed));
    const absent = []; // listed fragments the node answered are not there
    await new Promise((resolve, reject) => {
      let left = listed.length;
      for (const [j] of listed)
        fetchState(core.file_fragment_id(f.key, g, j), `${ref.name}: part ${g + 1} of ${f.plan.gens}`).then(state => {
          if (!state) absent.push(j);
          if (d.done()) return;
          if (state) {
            try {
              d.add(j, state);
            } catch (e) {
              ctx.log("files", { what: `${ref.name}: a fragment refused (${e.message ?? e})` });
            }
          }
          if (d.done()) return resolve();
          if (--left === 0) reject(new Error(`${ref.name}: part ${g + 1} could not be rebuilt (${d.rank()} of ${Math.min(GEN, f.plan.chunks - g * GEN)} pieces found)`));
        });
    });
    const plain = d.plain();
    // READERS REPAIR: what a read had to rebuild around is made again and put back (in the background; a fragment is
    // deterministic, so a viewer's copy is the uploader's). Only with the file's burn hash — a piece's first write names it.
    if (absent.length && ref.b) {
      const burn = bytes(ref.b);
      for (const j of absent) putOne(core.file_mint(f.key, f.plan.size, g, plain, j, burn), `repairing ${ref.name}`).catch(() => {});
      ctx.log("files", { what: `${ref.name}: part ${g + 1}: ${absent.length} fragment(s) not there, made again and put` });
    }
    return plain;
  }

  async function* stream(ref, { from = 0 } = {}) {
    if (ref.inline) return yield unb64(ref.inline);
    ref = await current(ref);
    const f = await open(ref);
    for (let g = from; g < f.plan.gens; g++) yield await generation(ref, f, g);
  }

  async function get(ref, { onProgress = () => {} } = {}) {
    const parts = [];
    let done = 0;
    for await (const b of stream(ref)) {
      parts.push(b);
      done += b.length;
      onProgress({ done, size: ref.size });
    }
    return new Blob(parts, { type: ref.type });
  }

  // ONE CHUNK alone (a seek, a range's few chunks in a generation): its systematic fragment — HEDGED: not back in
  // HEDGE (or not there), its generation is raced as well (`genOnce`: every fragment asked, decoded on the first 16),
  // and the first to arrive is taken. Measured 2026-10-06: a 4K fragment's lone chunk waited a get's whole deadline
  // (30 s) while the rest of it took 0.1 s.
  const HEDGE = 2000;
  async function chunk(ref, i) {
    if (ref.inline) return unb64(ref.inline);
    ref = await current(ref);
    const f = await open(ref);
    const g = Math.floor(i / GEN);
    const listed = await f.listed(g);
    const at = (i % GEN) * f.plan.chunk;
    const alone = fetchState(core.file_fragment_id(f.key, g, i % GEN), `${ref.name}: a part`).then(state => {
      if (!state) throw new Error(`${ref.name}: a part is not there`);
      return glue.file_read_chunk(f.key, f.plan.size, JSON.stringify(listed), i, state);
    });
    return new Promise((resolve, reject) => {
      let raced = false;
      let fails = 0;
      const fail = e => ++fails === 2 && reject(e);
      const race = () => {
        if (raced) return;
        raced = true;
        clearTimeout(timer);
        genOnce(ref, f, g).then(all => resolve(all.slice(at, at + f.plan.chunk)), fail);
      };
      const timer = setTimeout(race, HEDGE);
      alone.then(
        c => (clearTimeout(timer), resolve(c)),
        e => (fail(e), race()),
      );
    });
  }

  // RE-KEY one row (`file-keys`): read under the key it has, coded under `to` (public, or the space's salt now) — the
  // new key derived from the content's hash, so a step done twice makes the same fragments — the row changed last.
  // A row with no hash is read once to hash it first.
  async function recode(sp, row, { pub }) {
    const old = { key: row.key, root: row.root, name: row.id, ...(row.b !== undefined ? { b: row.b } : {}) };
    const f = await open(old);
    const size = f.plan.size;
    let h = row.h;
    if (!h) {
      const hs = new glue.FileHasher();
      for (let g = 0; g < f.plan.gens; g++) hs.update(await generation(old, f, g));
      h = hex(hs.finish());
    }
    const st = pub ? null : await salt(sp);
    const key = bytes(keyFor(h, pub ? null : st.s));
    const n = pub ? -1 : st.n;
    const secret = pub ? null : await secretOf(st.s, key);
    const burn = secret ? await sha(secret) : new Uint8Array(32);
    if (hex(key) === row.key && row.b === hex(burn)) return setRow(sp, { ...row, h, n, ...(secret ? { x: hex(secret) } : {}) });
    const genBytes = f.plan.chunk * GEN;
    const root = await store(key, size, row.id, "", async a => generation(old, f, Math.floor(a / genBytes)), () => {}, await progressOf(sp, row.id), burn);
    const { x: oldSecret, ...kept } = row;
    await setRow(sp, { ...kept, key: hex(key), root, b: hex(burn), ...(secret ? { x: hex(secret) } : {}), h, n });
    // The OLD pieces BURNED — once no row of the space still names that key — when they were this space's (keyed
    // under its salt `n`; a public or adopted file's are not this space's to burn).
    if (row.b && row.n >= 0 && row.key !== hex(key) && !(await rows(sp)).some(r => r.key === row.key)) await burn_(sp, { ...row, x: oldSecret }, old, f).catch(e => ctx.log("files", { what: `${row.id.slice(0, 8)}…: burning its old pieces: ${e.message ?? e}` }));
  }

  // BURN every piece of a file (its root, its index, each generation's listed fragments), with the secret its space's
  // salt `row.n` gives.
  async function burn_(sp, row, old, f) {
    const key = bytes(row.key);
    // Its secret: kept in its row.
    const secret = row.x ? bytes(row.x) : null;
    if (!secret || hex(await sha(secret)) !== row.b) throw new Error("its burn secret is not known here");
    const puts = [core.file_burn(key, "root", 0, 0, secret, f.rootHash)];
    const leaves = Math.ceil(f.plan.gens / 192);
    for (let n = 0; n < leaves; n++) puts.push(core.file_burn(key, "index", 0, n, secret, await f.indexHash(0, n)));
    for (let level = 1; level <= f.depth; level++) for (let n = 0; n < Math.ceil(leaves / 7000 ** level); n++) puts.push(core.file_burn(key, "index", level, n, secret, await f.indexHash(level, n)));
    for (let g = 0; g < f.plan.gens; g++) for (const [j] of await f.listed(g)) puts.push(core.file_burn(key, "fragment", g, j, secret, ""));
    const ok = await Promise.all(puts.map(p => putOne([0, ...Array.from(p)], `${row.id.slice(0, 8)}…: burning`)));
    ctx.log("files", { what: `${row.id.slice(0, 8)}…: ${ok.filter(Boolean).length} of ${ok.length} old pieces burned` });
    return ok;
  }

  // KEEP a file (phase 4, Lifecycle): every piece — its root, its index, each generation's listed fragments — asked,
  // and put again as it is stored (a burned piece never: it stays burned). HEALTH per generation: WHOLE (every listed
  // fragment there), DEGRADED (at least the k it decodes from), DAMAGED (fewer). `{ id, at, pieces, missing, gens,
  // whole, degraded, damaged, put }`.
  async function keep(sp, row) {
    const t0 = performance.now();
    const ref = { key: row.key, root: row.root, name: `${row.id.slice(0, 8)}…`, ...(row.b ? { b: row.b } : {}) };
    const f = await open(ref);
    const out = { id: row.id, at: Date.now(), pieces: 0, missing: 0, gens: f.plan.gens, whole: 0, degraded: 0, damaged: 0, put: 0 };
    // READ, not re-put: a piece that answers is left as it is (re-putting every piece read flooded the node until its
    // own reads failed — measured 2026-10-06: 256 of 433 unanswered while re-putting, 5 when only reading). Only what
    // does not answer is made again (a fragment, through its generation's rebuild).
    const one = async (what, a, b, idHex) => {
      out.pieces += 1;
      const st = await fetchState(idHex, `keeping ${ref.name}`, { raw: true });
      if (!st || st[0] !== 2) return (out.missing += 1), false;
      return true;
    };
    await one("root", 0, 0, core.file_root_id(f.key, f.rootHash), f.rootHash);
    const leaves = Math.ceil(f.plan.gens / 192);
    for (let n = 0; n < leaves; n++) {
      const h = await f.indexHash(0, n);
      await one("index", 0, n, core.file_index_id(f.key, h), h);
    }
    for (let level = 1; level <= f.depth; level++)
      for (let n = 0; n < Math.ceil(leaves / 7000 ** level); n++) {
        const h = await f.indexHash(level, n);
        await one("index", level, n, core.file_index_id(f.key, h), h);
      }
    // Each generation: its listed fragments read and put again; a DEGRADED one (some missing, 16 still there) REPAIRED —
    // rebuilt from any 16 and its missing fragments made again (a fragment is deterministic: the same bytes, address and
    // hash as the index lists) and put. Measured 2026-10-06: 14% of a video's fragments unanswered a day after upload as
    // read then; later passes showed most of those were gets unanswered under load, not pieces gone. Put-again
    // alone never brings a lost one back. Several generations at a time (the gets are capped in `fetchState`).
    const burn = row.b ? bytes(row.b) : new Uint8Array(32);
    out.repaired = 0;
    const gen = async g => {
      const listed = await f.listed(g);
      const got = await Promise.all(listed.map(([j]) => one("fragment", g, j, core.file_fragment_id(f.key, g, j))));
      const there = got.filter(Boolean).length;
      const k = Math.min(GEN, f.plan.chunks - g * GEN);
      out[there === listed.length ? "whole" : there >= k ? "degraded" : "damaged"] += 1;
      if (there === listed.length) return;
      // Fewer than 16 answered HERE is not "lost": this read ran among keep's own puts (measured 2026-10-06: a pass
      // counted generations damaged whose pieces a plain read then found). The rebuild races every fragment itself and
      // fails cleanly when it truly cannot; a fragment made again that was there is the same bytes.
      const plain = await generation(ref, f, g).catch(() => null);
      if (!plain) return;
      const lost = listed.filter((_, n) => !got[n]);
      const made = await Promise.all(lost.map(([j]) => putOne(core.file_mint(f.key, f.plan.size, g, plain, j, burn), `repairing ${ref.name}`)));
      out.repaired += made.filter(Boolean).length;
    };
    let next = 0;
    await Promise.all(Array.from({ length: Math.min(GENS_IN_FLIGHT, f.plan.gens) }, async () => { while (next < f.plan.gens) await gen(next++); }));
    out.ms = Math.round(performance.now() - t0);
    ctx.log("kept", { what: `file ${ref.name}: ${out.pieces} piece(s) — ${out.whole}/${out.gens} generation(s) whole${out.degraded ? `, ${out.degraded} degraded` : ""}${out.damaged ? `, ${out.damaged} DAMAGED` : ""}; ${out.missing} unanswered, ${out.repaired} made again`, ms: out.ms });
    return out;
  }

  // When a re-key of this row last moved (its progress, else the row): for taking over from a member gone quiet.
  const lastMoved = async (sp, row) => Math.max(row.at ?? 0, (await progressOf(sp, row.id)).at()?.at ?? 0);

  // A BYTE RANGE of a file (a video played by range): the chunks it covers, each read alone (a seek reads only what it
  // lands in), the last few kept.
  const chunks = new Map(); // `${root}/${i}` → Promise<bytes>, the latest 96
  const chunkOnce = (ref, i) => {
    const k = `${ref.root}/${i}`;
    if (!chunks.has(k)) {
      chunks.set(k, chunk(ref, i).catch(e => (chunks.delete(k), Promise.reject(e))));
      while (chunks.size > 96) chunks.delete(chunks.keys().next().value);
    }
    return chunks.get(k);
  };
  // A GENERATION whole, raced (`generation`), the latest 8 kept (a 4K fragment's neighbours share them).
  const gens = new Map();
  const genOnce = (ref, f, g) => {
    const k = `${ref.root}/g${g}`;
    if (!gens.has(k)) {
      gens.set(k, generation(ref, f, g).catch(e => (gens.delete(k), Promise.reject(e))));
      while (gens.size > 8) gens.delete(gens.keys().next().value);
    }
    return gens.get(k);
  };
  async function range(ref, start, len) {
    if (ref.inline) return unb64(ref.inline).subarray(start, start + len);
    ref = await current(ref);
    const size = (await open(ref)).plan.chunk;
    const end = Math.min(ref.size, start + len);
    const out = new Uint8Array(Math.max(0, end - start));
    // ALL the range's chunks asked AT ONCE: one after another ran at a get's round trip per chunk, not the link. Where
    // the range needs half a generation or more (a 4K fragment: most of one), the generation is read RACED — every
    // fragment asked, decoded on the first 16 — so no one slow piece holds it; a few chunks are read alone.
    const at = [];
    for (let i = Math.floor(start / size); i * size < end; i++) at.push(i);
    const f = await open(ref);
    const perGen = new Map();
    for (const i of at) perGen.set(Math.floor(i / GEN), (perGen.get(Math.floor(i / GEN)) ?? 0) + 1);
    const raced = g => perGen.get(g) * 2 >= Math.min(GEN, f.plan.chunks - g * GEN);
    const got = await Promise.all(
      at.map(i => {
        const g = Math.floor(i / GEN);
        if (!raced(g)) return chunkOnce(ref, i);
        return genOnce(ref, f, g).then(all => all.subarray((i % GEN) * size, (i % GEN + 1) * size));
      }),
    );
    at.forEach((i, n) => {
      const from = Math.max(start, i * size);
      out.set(got[n].subarray(from - i * size, Math.min(end, (i + 1) * size) - i * size), from - start);
    });
    return out;
  }

  // UPLOADS NOT FINISHED (this account's, from any device): the background queue lists them.
  async function uploads() {
    const ups = await storage.table("uploads");
    await ups.settled;
    return ups
      .rows()
      .filter(r => r.value)
      .flatMap(r => {
        try {
          const v = JSON.parse(r.value);
          // A rendition's (rows from before `made` named it by its codec and height).
          if (v.made || /\.(avc|hevc|av1|aac)\.\d+p\.mp4$/.test(v.name ?? "")) return [];
          const gens = JSON.parse(glue.file_plan(v.size)).gens;
          return [{ name: v.name, size: v.size, done: (v.stored ?? []).filter(Boolean).length / Math.max(1, gens), at: v.at }];
        } catch {
          return [];
        }
      });
  }
  return { keep, put, uploads, keyOf, get, stream, chunk, range, current, adopt, publicity, rows, rowOf, salt, rotate, recode, lastMoved, plan: size => JSON.parse(glue.file_plan(size)) };
}
