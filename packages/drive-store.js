// DRIVE STORE, a capability: the catalogue of FILES. Every file uploaded is listed in its uploader's PERSONAL Drive and,
// uploaded in a SPACE that uses Drive, in that space's Drive — each entry an ITEM of kind `file` (`items`: the same
// path, access control and audience as a post), holding the file's reference (`files`), with its folder and where it
// came from (an app) in `meta`. An empty folder made in Drive is an item of kind `folder`. A file attached from Drive is
// its reference given to the item: its readers read it; nothing is uploaded again. Removing an entry stops listing the
// file (its pieces are kept or not by Lifecycle, like everything else).
//
//   const drive = await ctx.require("drive-store");
//   const ref = await drive.upload(file, { space, public, from, onProgress })   // put (`files`) and listed
//   await drive.add(ref, { space, from, folder, public })  // list a reference already made (a file saved from a message)
//   await drive.list(space)                             // [{ id, ref, at, folder, from, aud }], newest first (null: yours)
//   await drive.folders(space)                          // ["/", "/Photos", …]
//   await drive.mkdir(space, path)   await drive.move(space, id, folder)   await drive.remove(space, id)
//   await drive.onChange(space, fn)
//   await drive.drives()                                // the spaces whose Drive this person can pick (Drive in use)
export async function start(ctx) {
  const [storage, space, files, roles, kinds, items] = await Promise.all(["storage", "space", "files", "roles", "kinds", "items"].map(n => ctx.require(n)));
  const shared = sp => sp && sp.kind !== "account";
  // A space HAS a Drive only when it uses the Drive app (an `app` act); yours always.
  const usesDrive = async sp => !shared(sp) || (await roles.of(sp).then(r => r.apps().includes("drive"), () => false));
  // THE DRIVES this person can pick from: theirs, and every space they are in that uses Drive.
  async function drives() {
    const all = (await space.mine().catch(() => [])).filter(s => s.kind === "server" && !space.isGroup(s));
    const on = await Promise.all(all.map(usesDrive));
    return all.filter((_, i) => on[i]);
  }
  const me = async () => (await space.account()).id;
  const place = async sp => (shared(sp) ? { spaces: [sp] } : { people: [await me()] });
  const hex = b => [...new Uint8Array(b)].map(x => x.toString(16).padStart(2, "0")).join("");
  // A FILE's id: its own (its first index root; an inline file's own hash) — the same file listed twice is one entry.
  const fidOf = async ref => ref.id ?? ref.root ?? hex(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(ref.inline ?? ""))).slice(0, 32);
  const clean = f => `/${String(f ?? "").split("/").map(x => x.trim()).filter(Boolean).join("/")}`;
  // WHERE A FILE GOES with no folder named — uploaded through another app (a photo in a post, audio in a message, a
  // video): the folder of WHAT IT IS (`kinds`: /Videos, /Audio, /Images, /Documents, /Files), whichever app it came
  // through. Uploaded IN Drive: where it was put (Drive always names its folder).
  const typeFolder = ref => `/${kinds.domainName(kinds.ofType(ref?.type))}`;
  // WHO SEES an entry: as the file is (public — put in the clear — or not): yours public or only you, a space's public
  // or its members.
  const audienceOf = (sp, pub) => (pub ? "public" : shared(sp) ? "members" : "private");

  // THE ENTRIES of a Drive (items of kind `file`; `folder`: an empty folder).
  const entries = async (sp, kind = "file") => items.inPlaces(await place(sp), kind, { withVotes: false });
  // THE LIST: Drive's own entries (uploaded or saved in Drive: moved, removed, seen as set here) — and, READ-ONLY, the
  // files of the items made in other apps (a video, a track, an image in a post), each in the folder of what its item
  // is (/Videos, /Audio, /Images), else of what the file is: a VIEW, never a second entry. Who sees one, its edits and
  // its removal are its item's, in its app (`page`: its item's page — `items.pageOf`).
  const MEDIA = new Set(kinds.media().map(m => m.domain));
  const viewed = async sp =>
    (await items.inPlaces(await place(sp), [...kinds.all()].filter(k => !["note", "file", "folder"].includes(k)), { withVotes: false }).catch(() => [])).flatMap(it =>
      (it.files ?? []).map((ref, i) => ({
        id: `${it.ref}#${i}`,
        ref,
        at: it.at,
        folder: MEDIA.has(kinds.domain(it.kind)) ? `/${kinds.domainName(kinds.domain(it.kind))}` : typeFolder(ref),
        from: { app: kinds.domain(it.kind) },
        item: it.ref,
        page: items.pageOf(it.ref, it.kind),
        readOnly: true,
      })),
    );
  async function list(sp = null, { person = null, discover = false } = {}) {
    // SOMEONE ELSE's files — a person's (their space: what you may read) or DISCOVER's (anyone's public ones): read
    // only, by the same reads as every app's (`items`); Discover's in one folder (each person's folders are theirs).
    if (discover || person) {
      const its = discover ? await items.list({ discover: true }, "new", "file") : await items.inPlaces({ people: [person] }, "file", { withVotes: false });
      return its.filter(it => it.files?.[0]).map(it => ({ id: it.ref, ref: it.files[0], at: it.at, folder: discover ? "/" : clean(it.meta?.folder), by: it.by, readOnly: true, others: true }));
    }
    const own = (await entries(sp))
      .map(it => {
        const ref = it.files?.[0];
        if (!ref) return null;
        return { id: it.ref, ref, at: it.at, folder: clean(it.meta?.folder), from: it.meta?.from ?? null, aud: it.aud ?? (it.private ? "private" : null), mayEdit: it.mayEdit !== false };
      })
      .filter(Boolean);
    return [...own, ...(await viewed(sp))].sort((a, b) => b.at - a.at);
  }

  async function add(ref, { space: sp = null, from = null, folder = null, public: pub = !!ref?.public, at = Date.now(), write = null } = {}) {
    // A file another app made (a video, a post's image): its item lists it (`list`'s views) — not a second entry.
    if (from?.app && from.app !== "drive" && !from.saved) return null;
    const fid = await fidOf(ref);
    const meta = { folder: clean(folder ?? (from?.app === "drive" ? "/" : typeFolder(ref))), fid, ...(from ? { from } : {}) };
    // Yours always; the space's too when it is one that uses Drive. Listed once (the same file: one entry).
    const where = [null, ...(shared(sp) && (await usesDrive(sp)) ? [sp] : [])];
    let id = null;
    for (const w of where) {
      const had = (await entries(w)).find(it => it.meta?.fid === fid);
      if (had) {
        id ??= had.ref;
        continue;
      }
      const made = await items.submit({ board: w?.id ?? null, title: ref.name ?? "", body: "", kind: "file", files: [ref], meta, audience: audienceOf(w, pub), at, write });
      id ??= made;
    }
    return id;
  }

  async function upload(file, { space: sp = null, public: pub = false, from = null, folder = null, write = null, onProgress = () => {} } = {}) {
    const ref = await files.put(file, { space: sp, public: pub, app: from?.app ?? "drive", onProgress });
    await add(ref, { space: sp, from, folder, public: pub, write }).catch(e => ctx.log("drive", { what: `listing ${file.name}: ${e.message}` }));
    return ref;
  }

  async function folders(sp = null, opts = {}) {
    const all = new Set(["/"]);
    for (const r of await list(sp, opts)) all.add(r.folder);
    for (const f of await entries(sp, "folder")) all.add(clean(f.meta?.folder));
    // Every parent of a folder is a folder.
    for (const f of [...all]) for (let p = f; p !== "/"; p = p.slice(0, p.lastIndexOf("/")) || "/") all.add(p);
    return [...all].sort();
  }
  const mkdir = async (sp, path) => items.submit({ board: shared(sp) ? sp.id : null, title: clean(path), body: "", kind: "folder", meta: { folder: clean(path) }, audience: shared(sp) ? "members" : "private" });
  async function move(sp, id, folder) {
    const it = (await entries(sp)).find(x => x.ref === id);
    if (!it) throw new Error("that file is not in this Drive");
    await items.editItem(id, it.body ?? "", { meta: { ...(it.meta ?? {}), folder: clean(folder) } });
  }
  const remove = async (sp, id) => items.remove(id);
  const onChange = async (sp, f) => items.onChange(f);

  // (MIGRATIONS — `upkeep`.) v3: files another app put at an old catalogue's root, into their type's folder (read by
  // v5 then). v5: an old catalogue (the table `drive`) brought over as items — this person's own entries (only an
  // author writes their items), each with its time, folder and source; one brought over before is not again (`fid`).
  const oldTable = sp => (shared(sp) ? storage.table(space.tableOf(sp, "drive"), sp) : storage.table("drive"));
  const parse = r => {
    try {
      const v = JSON.parse(r.value);
      return v?.ref ? { fid: r.key.slice(2), ...v, folder: clean(v.folder) } : null;
    } catch {
      return null;
    }
  };
  async function sortByApp(sp = null) {
    if (!(await usesDrive(sp))) return true;
    const t = await oldTable(sp);
    await t.settled;
    for (const r of t.rows().filter(x => x.key.startsWith("f/") && x.value)) {
      const v = parse(r);
      const to = v && v.folder === "/" && v.from?.app !== "drive" ? typeFolder(v.ref) : "/";
      if (to !== "/") await t.put(r.key, JSON.stringify({ ...JSON.parse(r.value), folder: to }));
    }
    return true;
  }
  async function migrate(sp = null) {
    if (!(await usesDrive(sp))) return true;
    const t = await oldTable(sp);
    await t.settled;
    const self = await me();
    const r = shared(sp) ? await roles.of(sp) : null;
    const old = t.rows().filter(x => x.value && (!r || r.author(x) === self));
    if (!old.length) return true;
    const have = new Set((await entries(sp)).map(it => it.meta?.fid).filter(Boolean));
    const madeDirs = new Set((await entries(sp, "folder")).map(it => clean(it.meta?.folder)));
    let n = 0;
    for (const row of old) {
      if (row.key.startsWith("d/")) {
        const path = clean(row.key.slice(2));
        if (!madeDirs.has(path)) await mkdir(sp, path), madeDirs.add(path), (n += 1);
        continue;
      }
      const v = parse(row);
      if (!v || have.has(v.fid)) continue;
      const meta = { folder: v.folder, fid: v.fid, ...(v.from ? { from: v.from } : {}) };
      await items.submit({ board: shared(sp) ? sp.id : null, title: v.ref.name ?? "", body: "", kind: "file", files: [v.ref], meta, audience: audienceOf(sp, !!v.ref.public), at: Number(v.at) || Date.now() });
      have.add(v.fid);
      n += 1;
    }
    if (n) ctx.log("drive", { what: `${sp?.name ?? "your"} Drive: ${n} entr${n === 1 ? "y" : "ies"} brought over as items` });
    return true;
  }

  // v6: the entries v5 made for files OTHER apps made (their items list them now): removed — what was uploaded or
  // saved in Drive kept.
  async function unduplicate(sp = null) {
    if (!(await usesDrive(sp))) return true;
    for (const it of await entries(sp)) {
      const from = it.meta?.from;
      if (from?.app && from.app !== "drive" && !from.saved && it.mayEdit !== false) await items.remove(it.ref).catch(() => {});
    }
    return true;
  }

  return { upload, add, list, folders, mkdir, move, remove, onChange, drives, usesDrive, sortByApp, migrate, unduplicate };
}
