// SHARE, a capability: an item SHARED WITH CHOSEN PEOPLE (ARCHITECTURE §7 "Shared items") — the one sharing piece,
// for every app. A shared item is a GROUP (`groups`, key `item:<id>`: a space of its own, never on the spaces panel)
// holding the item, so nothing here is new machinery:
//   OWNER    the group's owner (`transfer` hands it on: an item's identity is not its owner)
//   EDITORS  holders of the group's composed role `editor`, which its `edit` policy names — every reader checks an edit
//            by it, as of the edit's time (`content`: `roles.mayWrite`); only editors (and the owner) post there
//   VIEWERS  its other members: they read, comment and react
// Sharing puts people on the group's list (they enter by themselves; the key tree seals what is written to them);
// revoking takes them off (the tree re-keys: they read nothing written after). An item first shared is MOVED into its
// group (`items`: the same kind, body, files and time; the original removed) — its ref changes once.
//
//   const share = await ctx.require("share");
//   const ref2 = await share.with(ref, [{ did, role: "editor" | "viewer" }])   // shared (moved the first time)
//   await share.people(ref)        // [{ did, role: "owner" | "editor" | "viewer" }] — null: not a shared item
//   await share.revoke(ref, did)   await share.setRole(ref, did, role)   await share.handOn(ref, did)
//   await share.groups()           // the item groups this person is in (by them or with them)
//   share.isShared(ref)            // its ref names an item group this person is in
//   await share.privateIn(spaceId, kind, title)   // a space's PRIVATE item's group (its maker alone; `items.submit`)
//   await share.moveTo(item, place, meta?)        // an item to a group, a space or yours (null): its files adopted there
//   await share.setAudience(ref, audience)        // WHO SEES IT changed after it was made (the picker's value): moved
//   share.parentId(group)                         // the space a group's item is in (its key), or null: a person's
// IN A SPACE its setting rules (`share`/`with`, The space): shared with its MEMBERS only (the default — and anyone who
// leaves the space taken off, `keepAll`), or with people outside it too. Only its author or the space's admins share it.
//   share.button(it)               // the bar's "👥 Share with…" (`actions`): opens the dialog — people, roles, add,
//                                  // remove, hand on (`share.open(ref)`: drawn by the host, in an app's frame too)
// THE BUTTON, the same in the page and in an app's frame (FOUNDATION §3: a host capability's frame helper): it asks
// the host to open the dialog — what crosses is the item's ref, never the page.
const buttonOf = open => it => {
  const b = Object.assign(document.createElement("button"), { type: "button", textContent: "👥 Share with…" });
  b.onclick = e => (e.stopPropagation(), e.preventDefault(), open(it.ref));
  return b;
};
export function frame(cap) {
  return { button: buttonOf(ref => cap.open(ref)) };
}

export async function start(ctx) {
  const [space, roles, items, groups, directory] = await Promise.all(["space", "roles", "items", "groups", "directory"].map(n => ctx.require(n)));
  const ROLE = "editor";
  const KEY = "item:";
  const spaceOf = ref => (String(ref).startsWith("space:") ? String(ref).slice(6, String(ref).indexOf("/")) : null);
  const mine = async () => (await space.mine.strict()).filter(s => s.group?.startsWith(KEY));
  const groupOf = async ref => {
    const id = spaceOf(ref);
    return id ? ((await mine()).find(s => s.id === id) ?? null) : null;
  };
  const isShared = async ref => !!(await groupOf(ref));
  // ITS SPACE (a space's item: its group's key names it, `item:<space>/<id>` — what an invite carries; yours: none).
  const parentId = g => /^item:([0-9a-f]{64})\//.exec(g?.group ?? "")?.[1] ?? null;
  const parentOf = async g => {
    const id = parentId(g);
    return id ? ((await space.mine.strict()).find(s => s.id === id) ?? null) : null;
  };
  // THE SPACE's RULE for what is shared from it (its setting `share`/`with`, The space): its members only (the
  // default), or people outside it too ("anyone").
  async function ruleOf(psp) {
    const r = await roles.of.strict(psp);
    await r.settled;
    return { r, anyone: r.config("share", "with", "") === "anyone" };
  }
  // WHO IT MAY BE SHARED WITH: a space's item, as its space's rule says — said by name when someone is not a member.
  async function mayShareWith(pid, dids) {
    const p = pid ? ((await space.mine.strict()).find(s => s.id === pid) ?? null) : null;
    if (!p) return;
    const { r, anyone } = await ruleOf(p);
    if (anyone) return;
    const members = new Set(r.members().map(m => m.did));
    const out = dids.filter(d => !members.has(d));
    if (out.length) throw new Error(`${out.map(d => directory.shown(d)).join(", ")}: not ${space.shown(p)}'s member${out.length > 1 ? "s" : ""} — it shares what is in it with its members only`);
  }

  // A GROUP's EDITOR ROLE (once): the role, the `edit` and `post` policies naming it, and the item's domain on.
  async function editorsIn(g, kind) {
    const r = await roles.of.strict(g);
    await r.settled;
    if (!r.roles().some(x => x.id === ROLE)) {
      await r.act({ act: "role", role: ROLE, name: "Editor", perms: ["post"] });
      for (const action of ["edit", "post"]) await r.act({ act: "policy", path: "", action, who: `role:${ROLE}` });
    }
    const domain = (await ctx.require("kinds")).domain(kind);
    if (!r.base().includes(domain) && r.capabilities().includes(domain) && !r.enabled().includes(domain)) await r.enable(domain, true);
  }
  // A NEW GROUP for an item: yours (`item:<id>`), or a space's (`item:<space>/<id>`).
  async function newGroup(psp, id, name, kind) {
    const g = await groups.of(`${KEY}${psp ? `${psp.id}/` : ""}${id}`, { name });
    await editorsIn(g, kind);
    return g;
  }
  // PRIVATE IN A SPACE (`items.submit`, "Only you" in a space): a group of its own, its maker alone — shared later.
  async function privateIn(spId, kind, title) {
    const psp = (await space.mine.strict()).find(s => s.id === spId);
    if (!psp) throw new Error("not in that space");
    return newGroup(psp, (await ctx.require("node")).newHex(8), title || `A private ${kind}`, kind);
  }

  // ITS GROUP, made the first time the item is shared — then the item moved in. Yours: by you; a space's: by its
  // author or the space's admins.
  async function groupFor(ref) {
    const had = await groupOf(ref);
    if (had) return { sp: had, ref };
    const me = await space.me.strict();
    const it = await items.get(ref);
    if (!it) throw new Error("no such item");
    let psp = null;
    if (spaceOf(ref)) {
      psp = (await space.mine.strict()).find(s => s.id === spaceOf(ref)) ?? null;
      if (!psp) throw new Error("not in that space");
      const { r } = await ruleOf(psp);
      if (it.by !== me && !r.isAdmin(me)) throw new Error("only its author or the space's admins share it");
    } else if (!String(ref).startsWith(`${me}/`)) throw new Error("only your own items are shared");
    const sp = await newGroup(psp, it.id, it.title || `A shared ${it.kind}`, it.kind);
    const moved = await moveTo(it, sp);
    // A FOLDER: what is under its path moves in with it (`drive-store`: its entries, each by `moveTo`).
    if (it.kind === "folder") await (await ctx.require("drive-store")).folderInto(sp, it.meta?.folder ?? it.title, psp);
    return { sp, ref: moved };
  }

  // AN ITEM MOVED — to a place (a group: shared; into a shared folder; back to a space or yours, `to` null) or to
  // another AUDIENCE where it is (`setAudience`): where it goes decided by the one routing (`items.route`), the same
  // kind, body, files, meta and time written there (`submit` puts its files where it went) and the original removed.
  // The one way an item changes place. (Its comments and votes stay with the old ref.)
  async function relocate(it, { board = null, audience, meta = it.meta }) {
    // (Its files: put where it goes by `submit` — `items.placeFiles`, the one rule.)
    const at = await items.route({ board, audience, kind: it.kind, title: it.title, meta });
    const fs = it.files ?? [];
    // A DOCUMENT's text is its own table's (`codoc`): taken whole before it moves, its new table's first row after.
    const codoc = (await ctx.require("kinds")).coedited(it.kind) ? await ctx.require("codoc") : null;
    const snap = codoc ? await codoc.snapshot(it.ref) : null;
    const moved = await items.submit({ board: at.board, title: it.title, body: it.body, kind: it.kind, files: fs, meta: at.meta, at: it.at, audience: at.audience });
    if (codoc) await codoc.seed(moved, snap);
    // (Its changes taken back where it was: what was public there is not left public.)
    if (codoc) await codoc.clear(it.ref);
    await items.remove(it.ref);
    return moved;
  }
  const moveTo = (it, to, meta = it.meta) => relocate(it, { board: to?.id ?? null, audience: to ? "members" : "private", meta });

  // WHERE AN ITEM IS and WHO SEES IT, as the audience picker names it: its HOME (the space it belongs to; yours: null)
  // and its audience there — a private item's group, a circle, a narrower audience's group each read back to theirs.
  async function placeOf(it) {
    const id = spaceOf(it.ref);
    if (!id) return { home: null, audience: it.private ? "private" : "public" };
    const all = await space.mine.strict();
    const sp = all.find(s => s.id === id);
    if (!sp) throw new Error("you are not in that item's space");
    const g = sp.group ?? "";
    const find = pid => all.find(s => s.id === pid) ?? null;
    if (g.startsWith("circle:")) return { home: null, audience: g.slice(7) };
    if (g.startsWith("level:")) return { home: null, audience: "public", level: true };
    if (g.startsWith("audience:")) return { home: find(/^audience:([0-9a-f]{64})\//.exec(g)?.[1]), audience: it.meta?.aud ?? "members" };
    if (g.startsWith(KEY)) return { home: parentId(sp) ? find(parentId(sp)) : null, audience: "private", group: sp };
    return { home: sp, audience: it.pub ? "public" : "members" };
  }
  // WHO SEES IT, changed after it was made (owner 10-20: "after docs created, there is no way to change its policy"):
  // its author, or its space's admins; the item moved to that audience where it is. One shared with chosen people
  // keeps them (its sharing is changed in Share: the people taken off first).
  async function setAudience(ref, audience) {
    const me = await space.me.strict();
    const it = await items.get.strict(ref);
    if (!it) throw new Error("no such item");
    const at = await placeOf(it);
    if (at.level) throw new Error("a level's item: who sees it is its level");
    if (at.audience === audience) return ref;
    let admin = false;
    if (at.home) {
      const { r } = await ruleOf(at.home);
      admin = r.isAdmin(me);
    }
    if (it.by !== me && !admin) throw new Error(at.home ? "only its author or the space's admins change who sees it" : "only your own items");
    if (at.group && ((await people(ref)) ?? []).length > 1) throw new Error("it is shared with chosen people: take them off in Share first");
    return relocate(it, { board: at.home?.id ?? null, audience });
  }

  async function people(ref) {
    const sp = await groupOf(ref);
    if (!sp) return null;
    const r = await roles.of.strict(sp);
    await r.settled;
    return r.roster().map(did => ({ did, role: did === r.owner ? "owner" : (r.held(did) ?? []).some(x => (x?.id ?? x) === ROLE) ? "editor" : "viewer" }));
  }

  async function setRole(ref, did, role) {
    const sp = await groupOf(ref);
    if (!sp) throw new Error("not a shared item");
    const r = await roles.of.strict(sp);
    await r.settled;
    if (did === r.owner) return;
    await r.act({ act: "assign", did, role: ROLE, on: role === "editor" });
  }

  async function shareWith(ref, people_) {
    // (Checked BEFORE anything moves: a space's item goes nowhere its space does not allow.)
    const had = await groupOf(ref);
    await mayShareWith(had ? parentId(had) : spaceOf(ref), people_.map(p => p.did));
    const { sp, ref: at } = await groupFor(ref);
    const r = await roles.of.strict(sp);
    await r.settled;
    await groups.keep(sp, new Set([...r.roster(), ...people_.map(p => p.did)]));
    for (const p of people_) if (p.role === "editor") await setRole(at, p.did, "editor");
    return at;
  }

  async function revoke(ref, did) {
    const sp = await groupOf(ref);
    if (!sp) throw new Error("not a shared item");
    const r = await roles.of.strict(sp);
    await r.settled;
    // (Their editor role taken back first — where they never held it, the act is refused: said, not fatal.)
    await ctx.optional("share: editor role taken back", setRole(ref, did, "viewer"), () => null);
    await groups.keep(sp, new Set(r.roster().filter(d => d !== did)));
  }

  async function handOn(ref, did) {
    const sp = await groupOf(ref);
    if (!sp) throw new Error("not a shared item");
    const r = await roles.of.strict(sp);
    await r.settled;
    await r.act({ act: "transfer", did });
  }

  // THE PAGE FOLLOWS ITS ITEM: an item moved (its ref changed) — the address showing it now shows it where it is.
  const follow = (from, to) => {
    for (const f of [encodeURIComponent(from), from]) if (location.hash.includes(f)) return void (location.hash = location.hash.replace(f, encodeURIComponent(to)));
  };
  // THE DIALOG: who has it, each one's role (owner fixed), add from your contacts or by their id, remove; hand it on.
  const h = (await ctx.require("theme")).h;
  async function open(first) {
    const me = await space.me.strict();
    const d = h("dialog", { className: "cw-share" });
    const said = h("p", { className: "said" });
    const list = h("div", { style: "display:grid;gap:6px;max-height:50vh;overflow:auto" });
    let ref = first;
    const fail = e => (said.textContent = ctx.say(e));
    const draw = async () => {
      const ps = (await people(ref)) ?? [{ did: me, role: "owner" }];
      const owner = ps.find(p => p.role === "owner")?.did;
      const mayManage = owner === me;
      list.replaceChildren(
        ...ps.map(p => {
          const role =
            p.role === "owner" || !mayManage
              ? h("span", { className: "chip", textContent: p.role })
              : h("select", { className: "field", onchange: e => setRole(ref, p.did, e.target.value).then(draw, fail) }, ...["editor", "viewer"].map(v => h("option", { value: v, textContent: v, selected: v === p.role })));
          const out = mayManage && p.role !== "owner" ? h("button", { type: "button", textContent: "Remove", onclick: () => revoke(ref, p.did).then(draw, fail) }) : null;
          const give = mayManage && p.role !== "owner" ? h("button", { type: "button", textContent: "Make owner", onclick: () => handOn(ref, p.did).then(draw, fail) }) : null;
          return h("div", { style: "display:flex;gap:6px;align-items:center" }, directory.nameEl(p.did), role, out, give);
        }),
      );
      add.hidden = !mayManage;
    };
    // ADD: one of your contacts (friends, people you follow), or a person's id pasted.
    const edge = await ctx.require("edge");
    const ppl = await edge.people();
    const known = [...new Set([...ppl.list("friend"), ...ppl.list("follow")])].filter(x => String(x).startsWith("did:craftec:") && x !== me);
    const who = h("input", { className: "field", placeholder: "did:craftec:… or a contact" });
    who.setAttribute("list", "cw-share-known"); // (`list` is read-only as a property: an attribute)
    const opts = h("datalist", { id: "cw-share-known" }, ...known.map(x => h("option", { value: x, textContent: directory.shown(x) })));
    const as = h("select", { className: "field" }, h("option", { value: "viewer", textContent: "viewer" }), h("option", { value: "editor", textContent: "editor" }));
    const add = h(
      "form",
      { style: "display:flex;gap:6px", onsubmit: e => {
        e.preventDefault();
        const did = who.value.trim();
        if (!/^did:craftec:[1-9A-HJ-NP-Za-km-z]{20,64}$/.test(did)) return fail(new Error("a person's id: did:craftec:…"));
        said.textContent = "Sharing…";
        shareWith(ref, [{ did, role: as.value }]).then(at => (at !== ref && follow(ref, at), (ref = at), (who.value = ""), (said.textContent = ""), draw()), fail);
      } },
      who, opts, as, h("button", { type: "submit", textContent: "Add" }),
    );
    // WHO SEES IT (the one picker, as when it was made): changed here, moved where that audience reads it.
    const whoBox = h("div", { style: "display:grid;gap:6px" });
    const drawWho = async () => {
      const it = await items.get.strict(ref);
      const at = it ? await placeOf(it) : null;
      if (!it || at.level || (at.group && ((await people(ref)) ?? []).length > 1)) return whoBox.replaceChildren();
      const pick = await (await ctx.require("audience")).picker({ space: at.home, kind: it.kind, initial: at.audience });
      const save = h("button", { type: "button", className: "cw-btn", textContent: "Change", onclick: async () => {
        save.disabled = true;
        said.textContent = "Moving it…";
        await setAudience(ref, pick.value()).then(at2 => ((ref = at2), (said.textContent = "Who sees it: changed"), follow(first, at2), drawWho(), draw()), fail);
        save.disabled = false;
      } });
      whoBox.replaceChildren(h("h4", { textContent: "Who sees it", style: "margin:0" }), pick.el, save);
    };
    d.append(h("h3", { textContent: "Share" }), whoBox, h("h4", { textContent: "People", style: "margin:0" }), list, add, said, h("form", { method: "dialog" }, h("button", { textContent: "Done" })));
    document.body.append(d);
    d.addEventListener("close", () => d.remove());
    d.showModal();
    await Promise.all([draw(), drawWho()]).catch(fail);
  }

  // KEPT TO ITS SPACE (its rule members-only): each space item's group this person owns — whoever is no longer in the
  // space taken off (`groups.keep`, the one keeper of a list; only who is wanted is decided here).
  async function keepAll() {
    const me = await space.me.strict();
    if (!me) return 0;
    let n = 0;
    for (const g of await mine()) {
      const p = await parentOf(g);
      if (!p) continue;
      const gr = await roles.of.strict(g);
      await gr.settled;
      if (gr.owner !== me) continue;
      const { r, anyone } = await ruleOf(p);
      if (anyone) continue;
      const members = new Set(r.members().map(m => m.did));
      const want = new Set(gr.roster().filter(d => d === me || members.has(d)));
      if (want.size < gr.roster().length) (await groups.keep(g, want), (n += 1));
    }
    return n;
  }
  (await ctx.require("lifecycle")).job("share keep", { every: 600e3, delay: 60e3, needs: async () => !!(await space.me()) || "nobody is logged in", run: () => keepAll() });

  return { with: shareWith, privateIn, keepAll, moveTo, setAudience, parentId, people, revoke, setRole, handOn, groups: mine, isShared, open, button: buttonOf(open) };
}
