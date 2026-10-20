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

  // ITS GROUP, made the first time the item is shared: the editor role, the `edit` and `post` policies naming it, and
  // its kind's domain on — then the item moved in.
  async function groupFor(ref) {
    const had = await groupOf(ref);
    if (had) return { sp: had, ref };
    const me = await space.me.strict();
    if (spaceOf(ref) || !String(ref).startsWith(`${me}/`)) throw new Error("only your own items are shared from here (a space's are shared by the space)");
    const it = await items.get(ref);
    if (!it) throw new Error("no such item");
    const domain = (await ctx.require("kinds")).domain(it.kind);
    const sp = await groups.of(`${KEY}${it.id}`, {
      name: it.title || `A shared ${it.kind}`,
      setup: async (_, r) => {
        await r.act({ act: "role", role: ROLE, name: "Editor", perms: ["post"] });
        for (const action of ["edit", "post"]) await r.act({ act: "policy", path: "", action, who: `role:${ROLE}` });
        if (!r.base().includes(domain) && r.capabilities().includes(domain)) await r.enable(domain, true);
      },
    });
    const moved = await items.submit({ board: sp.id, title: it.title, body: it.body, kind: it.kind, files: it.files, meta: it.meta, at: it.at, audience: "members" });
    await items.remove(ref);
    return { sp, ref: moved };
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
        shareWith(ref, [{ did, role: as.value }]).then(at => ((ref = at), (who.value = ""), (said.textContent = ""), draw()), fail);
      } },
      who, opts, as, h("button", { type: "submit", textContent: "Add" }),
    );
    d.append(h("h3", { textContent: "Share" }), list, add, said, h("form", { method: "dialog" }, h("button", { textContent: "Done" })));
    document.body.append(d);
    d.addEventListener("close", () => d.remove());
    d.showModal();
    await draw().catch(fail);
  }

  return { with: shareWith, people, revoke, setRole, handOn, groups: mine, isShared, open, button: buttonOf(open) };
}
