// THE LOAD PIECES, on the page (sdk#347): hand any k of the SDK's k + m pieces to the DECODER wasm and get the
// bundle's files back. The decoding and the bundle format are the decoder's (built from the `pieces` crate, the one
// home of both); this file only MOVES BYTES in and out of it over its ABI (decoder/src/lib.rs). It parses nothing
// of the format.
//
// Carried in a published app's STARTER: it runs before the SDK exists. A starter ENTRY (build.sh
// STARTER_ENTRIES): the starter is what the entries reach, named in artefacts.json `starter`.

/** An instantiated decoder from its wasm bytes. */
export async function decoder(wasmBytes) {
  const { instance } = await WebAssembly.instantiate(wasmBytes, {});
  return instance.exports;
}

/**
 * The bundle's files from `pieces` (an array of k + m entries, `Uint8Array` or `null` for one not here, each
 * already checked against its sha256 by the caller) under `shape` = `{ k, m, payload, bundle_len }`.
 * Returns `{ files, bundle }`: a `Map` of name -> `Uint8Array`, and the rebuilt bundle (what the post-load repair
 * re-derives a missing piece from). Throws the decoder's refusal in its words.
 */
export function openPieces(dec, shape, pieces) {
  const { k, m, payload, bundle_len } = shape;
  if (pieces.length !== k + m) throw new Error(`openPieces: ${pieces.length} pieces, not k + m = ${k + m}`);
  const ABSENT = 0xffffffff;
  const len = 16 + pieces.reduce((n, p) => n + 4 + (p ? p.length : 0), 0);
  const ptr = dec.alloc(len);
  // A fresh view AFTER alloc: allocating may grow (and so replace) the module's memory buffer.
  const view = new DataView(dec.memory.buffer, ptr, len);
  const bytes = new Uint8Array(dec.memory.buffer, ptr, len);
  let at = 0;
  for (const v of [k, m, payload, bundle_len]) { view.setUint32(at, v, true); at += 4; }
  for (const p of pieces) {
    view.setUint32(at, p ? p.length : ABSENT, true);
    at += 4;
    if (p) { bytes.set(p, at); at += p.length; }
  }
  if (dec.open(ptr, len) !== 0) {
    throw new Error(`the SDK's pieces do not open: ${text(dec, dec.error_ptr(), dec.error_len())}`);
  }
  const files = new Map();
  for (let i = 0; i < dec.count(); i += 1) {
    const name = text(dec, dec.name_ptr(i), dec.name_len(i));
    files.set(name, new Uint8Array(dec.memory.buffer, dec.data_ptr(i), dec.data_len(i)).slice());
  }
  return { files, bundle: new Uint8Array(dec.memory.buffer, dec.bundle_ptr(), dec.bundle_len()).slice() };
}

const text = (dec, ptr, len) => new TextDecoder().decode(new Uint8Array(dec.memory.buffer, ptr, len));

/**
 * REPAIR AFTER LOAD (sdk#347; the owner: parity also REPAIRS): a loader that rebuilt the SDK's bundle from `k`
 * pieces PUTs back each piece a source answered NOT FOUND and that never arrived (`raced.notHeld`), so a lost piece is
 * restored by the next app that loads it -- never one the race merely cancelled at k.
 * Only those (never a piece this load did not wait on); each re-derived from the rebuilt bundle by the SDK
 * (`load_piece`), checked against the manifest's sha256 AND its container's address before it goes anywhere, and
 * PUT by `session.put_piece` (the SDK frames the container around the piece and PUTs it under its own webapp code:
 * content-addressed, never a page; the page's one app-PUT path: its deadline and re-send). Never a user write.
 *
 * `spec` is the manifest's `{ k, m, payload, bundle_len, pieces: [{ address, sha256 }] }`, `raced` what `raceK`
 * resolved, `sdk` the SDK as an app has it (`load()`'s: `sdk.pieces`, `sdk.webapp`). Returns one entry per piece considered: `{ piece, put: key }` or `{ piece, refused: why }`.
 */
export async function repairPieces({ spec, bundle, raced, sdk, session, webappCode, subtle }) {
  const hex = b => [...new Uint8Array(b)].map(x => x.toString(16).padStart(2, "0")).join("");
  const out = [];
  // ONLY the pieces the node answered "not held" and that never arrived (`raced.notHeld`): a piece the race CANCELLED
  // at k was not missing, just slower, and re-PUTting it is a needless PUT through the node's one queue (F61;
  // measured on the final realnet as ~every piece of both bundles PUT per open, before this).
  for (const i of raced.notHeld ?? []) {
    const want = spec.pieces[i];
    const piece = sdk.pieces.load(bundle, spec.payload, spec.m, i);
    if (hex(await subtle.digest("SHA-256", piece)) !== want.sha256) {
      out.push({ piece: i, refused: "the re-derived piece does not hash to the manifest's sha256" });
      continue;
    }
    const state = sdk.pieces.container(piece);
    if (sdk.webapp.address(webappCode, state) !== want.address) {
      out.push({ piece: i, refused: "the re-derived container is not at the manifest's address" });
      continue;
    }
    out.push({ piece: i, put: session.put_piece(webappCode, piece) });
  }
  return out;
}

/**
 * LINK THE BUNDLE'S MODULES (sdk#347): the bundle's `.js` files are ES modules that import each other by relative
 * path. Rebuilt in memory they have no URLs, so each is given one (`toUrl`, a blob: URL in the page), in DEPENDENCY
 * order, with its relative specifiers rewritten to the URLs of what they name:
 * - a path in the bundle -> that module's URL (one URL per path, so every importer shares ONE instance: the
 *   wasm-bindgen glue must be one module, whoever imports it);
 * - a path the STARTER serves (`external(path)` returns its URL: the modules artefacts.json `starter` names) -> that URL;
 * - anything else is left as written (a specifier in a comment, e.g.), and a real import of a missing file fails
 *   loudly in the browser when it is imported.
 * A cycle is refused: the build keeps the graph acyclic, and a cycle has no bottom to link from.
 *
 * `files` is the `Map` `openPieces` returned. Returns a `Map` of path -> URL for every `.js` in the bundle.
 */
export function linkModules(files, { external = () => null, toUrl = blobUrl } = {}) {
  const dec = new TextDecoder();
  const src = new Map([...files].filter(([p]) => p.endsWith(".js")).map(([p, b]) => [p, dec.decode(b)]));
  const SPEC = /(\bfrom\s*|\bimport\s*\(?\s*)(["'])(\.{1,2}\/[^"'\n]+)\2/g;
  const deps = new Map();
  for (const [p, text] of src) {
    const d = new Set();
    for (const m of text.matchAll(SPEC)) {
      const r = resolvePath(p, m[3]);
      if (src.has(r) && r !== p) d.add(r);
    }
    deps.set(p, d);
  }
  const url = new Map();
  const state = new Map(); // 1 = linking (on the stack), 2 = linked
  const link = (p, stack) => {
    if (state.get(p) === 2) return;
    if (state.get(p) === 1) throw new Error(`the bundle's modules import in a cycle: ${[...stack, p].join(" -> ")}`);
    state.set(p, 1);
    for (const d of deps.get(p)) link(d, [...stack, p]);
    const text = src.get(p).replace(SPEC, (all, lead, q, spec) => {
      const r = resolvePath(p, spec);
      const to = url.get(r) ?? (src.has(r) ? null : external(r));
      return to ? `${lead}${q}${to}${q}` : all;
    });
    url.set(p, toUrl(text));
    state.set(p, 2);
  };
  for (const p of src.keys()) link(p, []);
  return url;
}

function blobUrl(text) {
  return URL.createObjectURL(new Blob([text], { type: "text/javascript" }));
}

/** `spec` (./ or ../) resolved against the directory of `from`, as a bundle path. */
function resolvePath(from, spec) {
  const parts = from.split("/").slice(0, -1);
  for (const s of spec.split("/")) {
    if (s === "." || s === "") continue;
    if (s === "..") parts.pop();
    else parts.push(s);
  }
  return parts.join("/");
}
