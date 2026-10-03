// DRIVE STORE, a capability: the catalogue of FILES. Every file uploaded is listed in its uploader's PERSONAL Drive (the
// account's table `drive`) and, uploaded in a SPACE that uses Drive, in that space's Drive (its table `drive`, read by
// its members) —
// each row the file's reference (`files`) with when, in which folder, and where it came from (an app, an item). A file
// attached from Drive is its reference given to the item: its readers read it; nothing is uploaded again. Removing a
// row stops listing the file (its pieces are kept or not by Lifecycle, like everything else).
//
//   const drive = await ctx.require("drive-store");
//   const ref = await drive.upload(file, { space, public, from, onProgress })   // put (`files`) and listed
//   await drive.add(ref, { space, from, folder })       // list a reference already made (a file saved from a message)
//   await drive.list(space)                             // [{ id, ref, at, folder, from }], newest first (null: yours)
//   await drive.folders(space)                          // ["/", "/Photos", …]
//   await drive.mkdir(space, path)   await drive.move(space, id, folder)   await drive.remove(space, id)
//   await drive.onChange(space, fn)
//   await drive.drives()                                // the spaces whose Drive this person can pick (Drive in use)
export async function start(ctx) {
  const [storage, space, files, roles, kinds] = await Promise.all(["storage", "space", "files", "roles", "kinds"].map(n => ctx.require(n)));
  const shared = sp => sp && sp.kind !== "account";
  // A space HAS a Drive only when it uses the Drive app (an `app` act); yours always.
  const usesDrive = async sp => !shared(sp) || (await roles.of(sp).then(r => r.apps().includes("drive"), () => false));
  // THE DRIVES this person can pick from: theirs, and every space they are in that uses Drive.
  async function drives() {
    const all = (await space.mine().catch(() => [])).filter(s => s.kind === "server");
    const on = await Promise.all(all.map(usesDrive));
    return all.filter((_, i) => on[i]);
  }
  const tableOf = sp => (shared(sp) ? storage.table(space.tableOf(sp, "drive"), sp) : storage.table("drive"));
  const hex = b => [...new Uint8Array(b)].map(x => x.toString(16).padStart(2, "0")).join("");
  // A row's id: the file's (its id — its first index root; an inline file's own hash) — the same file listed twice is one row.
  const idOf = async ref => ref.id ?? ref.root ?? hex(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(ref.inline ?? ""))).slice(0, 32);
  const clean = f => `/${String(f ?? "").split("/").map(x => x.trim()).filter(Boolean).join("/")}`;

  // WHERE A FILE GOES with no folder named — uploaded through another app (a photo in a post, audio in a message, a
  // video): the folder of WHAT IT IS (`kinds`: /Videos, /Audio, /Images, /Documents, /Files), whichever app it came
  // through. Uploaded IN Drive: where it was put (Drive always names its folder).
  const typeFolder = ref => `/${kinds.domainName(kinds.ofType(ref?.type))}`;
  async function add(ref, { space: sp = null, from = null, folder = null } = {}) {
    const id = await idOf(ref);
    const row = { ref, at: Date.now(), folder: clean(folder ?? (from?.app === "drive" ? "/" : typeFolder(ref))), ...(from ? { from } : {}) };
    // Yours always; the space's too when it is one.
    const tables = [await tableOf(null), ...(shared(sp) && (await usesDrive(sp)) ? [await tableOf(sp)] : [])];
    for (const t of tables) {
      await t.settled;
      // Listed once; listed again only when the file moved spaces (adopted: its reference names the new one).
      const had = t.rows().find(r => r.key === `f/${id}` && r.value);
      const was = had ? parse(had) : null;
      if (!was || (was.ref.in ?? null) !== (ref.in ?? null)) await t.put(`f/${id}`, JSON.stringify(was ? { ...was, ref, id: undefined } : row));
    }
    return id;
  }

  async function upload(file, { space: sp = null, public: pub = false, from = null, folder = null, onProgress = () => {} } = {}) {
    const ref = await files.put(file, { space: sp, public: pub, app: from?.app ?? "drive", onProgress });
    await add(ref, { space: sp, from, folder }).catch(e => ctx.log("drive", { what: `listing ${file.name}: ${e.message}` }));
    return ref;
  }

  const parse = r => {
    try {
      const v = JSON.parse(r.value);
      return v?.ref ? { id: r.key.slice(2), ...v, folder: clean(v.folder) } : null;
    } catch {
      return null;
    }
  };
  async function list(sp = null) {
    const t = await tableOf(sp);
    await t.settled;
    return t
      .rows()
      .filter(r => r.key.startsWith("f/") && r.value)
      .map(parse)
      .filter(Boolean)
      // A reference from before files named their space: a space's Drive lists its own.
      .map(r => (r.ref.inline || r.ref.id ? r : { ...r, ref: { ...r.ref, id: r.ref.root, ...(shared(sp) ? { in: sp.id } : {}) } }))
      .sort((a, b) => b.at - a.at);
  }
  async function folders(sp = null) {
    const t = await tableOf(sp);
    await t.settled;
    const all = new Set(["/"]);
    for (const r of t.rows()) {
      if (!r.value) continue;
      if (r.key.startsWith("d/")) all.add(clean(r.key.slice(2)));
      else if (r.key.startsWith("f/")) all.add(parse(r)?.folder ?? "/");
    }
    // Every parent of a folder is a folder.
    for (const f of [...all]) for (let p = f; p !== "/"; p = p.slice(0, p.lastIndexOf("/")) || "/") all.add(p);
    return [...all].sort();
  }
  const mkdir = async (sp, path) => (await tableOf(sp)).put(`d/${clean(path)}`, "1");
  async function move(sp, id, folder) {
    const t = await tableOf(sp);
    const r = t.rows().find(x => x.key === `f/${id}` && x.value);
    if (!r) throw new Error("that file is not in this Drive");
    await t.put(r.key, JSON.stringify({ ...JSON.parse(r.value), folder: clean(folder) }));
  }
  const remove = async (sp, id) => (await tableOf(sp)).remove(`f/${id}`);
  const onChange = async (sp, f) => (await tableOf(sp)).onChange(f);

  // (A MIGRATION — `upkeep`.) Files at the root from another app, from before type folders: each moved to its type's.
  async function sortByApp(sp = null) {
    if (!(await usesDrive(sp))) return true;
    const t = await tableOf(sp);
    await t.settled;
    for (const r of t.rows().filter(x => x.key.startsWith("f/") && x.value)) {
      const v = parse(r);
      const to = v && v.folder === "/" && v.from?.app !== "drive" ? typeFolder(v.ref) : "/";
      if (to !== "/") await t.put(r.key, JSON.stringify({ ...JSON.parse(r.value), folder: to }));
    }
    return true;
  }

  return { upload, add, list, folders, mkdir, move, remove, onChange, drives, usesDrive, sortByApp };
}
