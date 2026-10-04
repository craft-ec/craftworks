// ZIP, a capability: a ZIP file's entries READ — what an EPUB and a comic (CBZ) are. Its central directory read once;
// an entry's bytes sliced from the file only when asked (stored, or deflated: the browser's own `DecompressionStream`).
// Reading only: nothing here writes a ZIP. ZIP64 (over 4 GB, or 65,535 entries) is refused by name.
//
//   const zip = await ctx.require("zip");
//   const z = await zip.open(blob)        // throws: "not a ZIP file", "a ZIP64 file is not read here"
//   z.names()                             // every entry's name, in the file's order (folders left out)
//   await z.blob(name, type)              // its bytes as a Blob (or null: no such entry)
//   await z.text(name)                    // its text (UTF-8), or null
export async function start(ctx) {
  const u16 = (v, o) => v.getUint16(o, true);
  const u32 = (v, o) => v.getUint32(o, true);
  const bytes = async (blob, from, to) => new DataView(await blob.slice(from, to).arrayBuffer());

  async function open(blob) {
    // The END record: in the last 22 bytes, or before a comment of at most 65,535.
    const tailFrom = Math.max(0, blob.size - 22 - 65535);
    const tail = await bytes(blob, tailFrom, blob.size);
    let end = -1;
    for (let i = tail.byteLength - 22; i >= 0; i--)
      if (u32(tail, i) === 0x06054b50) {
        end = i;
        break;
      }
    if (end < 0) throw new Error("not a ZIP file");
    const count = u16(tail, end + 10);
    const dirSize = u32(tail, end + 12);
    const dirAt = u32(tail, end + 16);
    if (count === 0xffff || dirAt === 0xffffffff || dirSize === 0xffffffff) throw new Error("a ZIP64 file is not read here");
    const dir = await bytes(blob, dirAt, dirAt + dirSize);
    const entries = new Map();
    const utf8 = new TextDecoder();
    for (let p = 0, n = 0; n < count; n++) {
      if (u32(dir, p) !== 0x02014b50) throw new Error("a broken ZIP directory");
      const method = u16(dir, p + 10);
      const packed = u32(dir, p + 20);
      const size = u32(dir, p + 24);
      const nameLen = u16(dir, p + 28);
      const extraLen = u16(dir, p + 30);
      const noteLen = u16(dir, p + 32);
      const at = u32(dir, p + 42);
      const name = utf8.decode(new Uint8Array(dir.buffer, dir.byteOffset + p + 46, nameLen));
      if (!name.endsWith("/")) entries.set(name, { method, packed, size, at });
      p += 46 + nameLen + extraLen + noteLen;
    }

    async function blobOf(name, type = "") {
      const e = entries.get(name);
      if (!e) return null;
      // The LOCAL header's own name and extra lengths (they may differ from the directory's).
      const local = await bytes(blob, e.at, e.at + 30);
      if (u32(local, 0) !== 0x04034b50) throw new Error(`${name}: a broken ZIP entry`);
      const from = e.at + 30 + u16(local, 26) + u16(local, 28);
      const raw = blob.slice(from, from + e.packed);
      if (e.method === 0) return new Blob([raw], { type });
      if (e.method !== 8) throw new Error(`${name}: packed a way not read here (method ${e.method})`);
      return new Blob([await new Response(raw.stream().pipeThrough(new DecompressionStream("deflate-raw"))).arrayBuffer()], { type });
    }

    return {
      names: () => [...entries.keys()],
      blob: blobOf,
      text: async name => {
        const b = await blobOf(name);
        return b ? await b.text() : null;
      },
    };
  }

  return { open };
}
