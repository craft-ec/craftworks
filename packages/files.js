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
//   const files = await ctx.require("files");
//   const ref = await files.put(file, { space, public: false, onProgress })  // { key, root, size, name, type } or
//                                                                            // { inline, size, name, type }
//   const blob = await files.get(ref, { onProgress })                        // the whole file
//   for await (const bytes of files.stream(ref, { from: 0 })) …             // a generation at a time, in order
//   const bytes = await files.chunk(ref, i)                                 // one chunk alone (a seek)
export async function start(ctx) {
  const { core, glue, ask } = await ctx.require("node");
  const [storage, space] = await Promise.all(["storage", "space"].map(n => ctx.require(n)));
  core.set_sealed_code(await ctx.require("sealed-wasm"));
  const INLINE_MAX = 64 * 1024;
  const GEN = 16;
  const EXTRA = 8;
  const MAX_LISTED = 32;
  const SLICE = 4 * 1024 * 1024;
  const hex = b => [...b].map(x => x.toString(16).padStart(2, "0")).join("");
  const bytes = h => new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
  const b64 = b => btoa(Array.from(b, c => String.fromCharCode(c)).join(""));
  const unb64 = s => Uint8Array.from(atob(s), c => c.charCodeAt(0));

  // A SPACE's dedup salt: a secret in its table `files`, made by the first upload there (two made at once: the merge
  // keeps one, and a file keyed by the other is still the file it is).
  async function saltOf(sp) {
    const t = sp && sp.kind !== "account" ? await storage.table(space.tableOf(sp, "files"), sp) : await storage.table("files");
    await t.settled;
    const had = t.rows().find(r => r.key === "salt")?.value;
    if (had) return bytes(had);
    const s = crypto.getRandomValues(new Uint8Array(32));
    await t.put("salt", hex(s));
    return s;
  }

  // One PUT: whether the node stored it (a refusal or no answer in 60 s: not).
  const putOne = ([, id, frames], what) =>
    ask(frames, x => (x.kind === "put" && x.key === id) || x.kind === "refused", what, 60000).then(
      x => x.kind === "put",
      () => false,
    );

  async function put(file, { space: sp = null, public: pub = false, onProgress = () => {} } = {}) {
    const size = file.size;
    const name = file.name ?? "file";
    const type = file.type || "application/octet-stream";
    if (size <= INLINE_MAX) return { inline: b64(new Uint8Array(await file.arrayBuffer())), size, name, type };
    // The content's hash, a slice at a time.
    const h = new glue.FileHasher();
    for (let at = 0; at < size; at += SLICE) {
      h.update(new Uint8Array(await file.slice(at, at + SLICE).arrayBuffer()));
      onProgress({ phase: "reading", done: Math.min(at + SLICE, size), size });
    }
    const key = glue.file_key(h.finish(), pub ? new Uint8Array(0) : await saltOf(sp));
    const keyHex = hex(key);
    const plan = JSON.parse(glue.file_plan(size));
    // RESUME: the generations already stored (the same key: the same fragments at the same addresses).
    const ups = await storage.table("uploads");
    await ups.settled;
    let prev = null;
    try {
      prev = JSON.parse(ups.rows().find(r => r.key === keyHex)?.value ?? "null");
    } catch {}
    const stored = prev?.stored ?? [];
    const genBytes = (plan.chunk * GEN);
    for (let g = 0; g < plan.gens; g++) {
      const k = Math.min(GEN, plan.chunks - g * GEN);
      if ((stored[g]?.length ?? 0) >= k + EXTRA) continue;
      const plain = new Uint8Array(await file.slice(g * genBytes, Math.min(size, (g + 1) * genBytes)).arrayBuffer());
      const have = new Set((stored[g] ?? []).map(x => x[0]));
      const ok = [...(stored[g] ?? [])];
      const sent = Array.from(core.file_encode(key, size, g, plain, EXTRA)).filter(f => !have.has(f[0]));
      await Promise.all(sent.map(f => putOne(f, `${name}: generation ${g}`).then(y => y && ok.push([f[0], f[3]]))));
      // RATELESS: a fragment not stored is replaced by a new one, not waited on.
      let next = Math.max(k + EXTRA, ...ok.map(x => x[0] + 1));
      while (ok.length < k + EXTRA && next < MAX_LISTED) {
        const f = core.file_mint(key, size, g, plain, next++);
        if (await putOne(f, `${name}: generation ${g}, fragment ${f[0]}`)) ok.push([f[0], f[3]]);
      }
      if (ok.length < k) throw new Error(`${name}: generation ${g} could not be stored (${ok.length} of the ${k} it needs)`);
      stored[g] = ok.sort((a, b) => a[0] - b[0]);
      await ups.put(keyHex, JSON.stringify({ name, size, type, stored, at: Date.now() })).catch(() => {});
      onProgress({ phase: "sending", done: Math.min(size, (g + 1) * genBytes), size });
    }
    // The INDEX last: once it is there, the file reads.
    const idx = core.file_index(key, size, JSON.stringify(stored));
    const puts = Array.from(idx.puts);
    const ok = await Promise.all(puts.slice(0, -1).map(p => putOne([0, ...p], `${name}: its index`)));
    if (ok.includes(false) || !(await putOne([0, ...puts.at(-1)], `${name}: its root`))) throw new Error(`${name}: its index could not be stored (the upload resumes)`);
    await ups.remove(keyHex).catch(() => {});
    onProgress({ phase: "done", done: size, size });
    return { key: keyHex, root: idx.root, size, name, type };
  }

  // One GET: the state, taken (a file's pieces are read once, never kept in the core), or null.
  async function fetchState(idHex, what) {
    const [, frames] = core.frames_get(bytes(idHex));
    const said = await ask(frames, x => (x.kind === "got" || x.kind === "get-failed") && x.id === idHex, what, 30000).catch(() => ({ kind: "get-failed" }));
    return said.kind === "got" ? core.take_got(idHex) : null;
  }

  // A file OPENED for reading: its root, and each generation's listed fragments (leaves read once).
  const opened = new Map();
  function open(ref) {
    if (!opened.has(ref.root))
      opened.set(
        ref.root,
        (async () => {
          const key = bytes(ref.key);
          const state = await fetchState(core.file_root_id(key), `${ref.name}: its root`);
          if (!state) throw new Error(`${ref.name}: not found on the network (yet)`);
          const root = JSON.parse(glue.file_root(key, ref.root, state));
          const leaves = new Map();
          // The leaf that lists generation g, down from the root through any inner levels.
          const leafOf = async g => {
            const n = Math.floor(g / 192);
            if (!leaves.has(n))
              leaves.set(
                n,
                (async () => {
                  let hashes = root.children;
                  for (let level = root.depth; level > 0; level--) {
                    const at = Math.floor(n / 7000 ** level);
                    const st = await fetchState(core.file_index_id(key, level, at), `${ref.name}: its index`);
                    if (!st) throw new Error(`${ref.name}: part of its index is missing`);
                    hashes = glue.file_inner(key, level, at, hashes[at % 7000], st);
                  }
                  const st = await fetchState(core.file_index_id(key, 0, n), `${ref.name}: its index`);
                  if (!st) throw new Error(`${ref.name}: part of its index is missing`);
                  return JSON.parse(glue.file_leaf(key, root.plan.size, n, hashes[root.depth ? n % 7000 : n], st));
                })(),
              );
            return (await leaves.get(n))[g % 192];
          };
          return { key, plan: root.plan, listed: leafOf };
        })().catch(e => (opened.delete(ref.root), Promise.reject(e))),
      );
    return opened.get(ref.root);
  }

  // A GENERATION, RACED: every listed fragment asked at once, decoded on the first k valid, independent ones.
  async function generation(ref, f, g) {
    const listed = await f.listed(g);
    const d = new glue.FileDecoder(f.key, f.plan.size, g, JSON.stringify(listed));
    await new Promise((resolve, reject) => {
      let left = listed.length;
      for (const [j] of listed)
        fetchState(core.file_fragment_id(f.key, g, j), `${ref.name}: part ${g + 1} of ${f.plan.gens}`).then(state => {
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
    return d.plain();
  }

  async function* stream(ref, { from = 0 } = {}) {
    if (ref.inline) return yield unb64(ref.inline);
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

  // ONE CHUNK alone (a seek): its systematic fragment, else its whole generation rebuilt.
  async function chunk(ref, i) {
    if (ref.inline) return unb64(ref.inline);
    const f = await open(ref);
    const g = Math.floor(i / GEN);
    const listed = await f.listed(g);
    const state = await fetchState(core.file_fragment_id(f.key, g, i % GEN), `${ref.name}: a part`);
    if (state) {
      try {
        return glue.file_read_chunk(f.key, f.plan.size, JSON.stringify(listed), i, state);
      } catch {}
    }
    const all = await generation(ref, f, g);
    const at = (i % GEN) * f.plan.chunk;
    return all.slice(at, at + f.plan.chunk);
  }

  return { put, get, stream, chunk, plan: size => JSON.parse(glue.file_plan(size)) };
}
