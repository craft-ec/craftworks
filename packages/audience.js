// AUDIENCE, a component: the ONE choice of who sees an item, for every app that makes one (Board, Videos, Audio,
// Notes, Drive). In YOUR space: everyone (public), your followers, your friends (each a CIRCLE: `circles`), or only
// you. In a SHARED space: its members, or everyone — public
// only while the space lets its app read in public (its policy is the ceiling: members-only = members post and read
// it, nobody makes it public). What it means for where the item is kept is `items`' (`submit({ audience })`).
//
//   const audience = await ctx.require("audience");
//   const a = await audience.picker({ space, kind, initial })   // space: null = yours; kind: what is made ("post",
//                                                               // "movie", "note" …); initial: the choice it starts on
//   form.append(a.el)
//   a.value()      // "public" | "private" (yours) | "members" (a space's)
//   a.isPublic()   // read by everyone (its files are put public)
//   a.level()      // "" (anyone who sees it) | "pro" | "vip": the LEVEL who may open it (rewards §5a) — enforced in
//                  // Craftworks' pages only until gated keys are built (said beside the choice)
export async function start(ctx) {
  const [items, space] = await Promise.all(["items", "space"].map(n => ctx.require(n)));
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };

  async function picker({ space: sp = null, kind = "post", initial = null } = {}) {
    // The choices here: yours, or what the space's policy allows for this kind.
    // A space's: everyone (where it may), its members — or fewer: a role's holders, the admins, the owner (sealed to
    // them alone: its audience's group).
    const r = sp ? await (await ctx.require("roles")).of(sp).catch(() => null) : null;
    const me = (await space.account())?.id;
    const directory = await ctx.require("directory");
    const listName = l => l.people.map(d => directory.shown(d)).join(", ");
    // A space's: its roles, admins — and NAMED PEOPLE: this person's lists, or a new one chosen from its members.
    const fewer = sp
      ? [
          ...(r?.roles() ?? []).map(ro => [`role:${ro.id}`, `🔐 ${ro.name} only`]),
          ["admins", "🔐 Admins only"],
          ...Object.entries(r?.lists?.() ?? {}).filter(([, l]) => l.by === me).map(([id, l]) => [`list:${id}`, `🔐 ${listName(l).slice(0, 60)}`]),
          ["list:new", "🔐 Chosen people…"],
        ]
      : [];
    const choices = sp
      ? [
          ...((await items.publicIn(sp, kind).catch(() => false))
            ? [["public", "🌐 Everyone (public)"], ["members", `👥 ${space.shown(sp)}'s members`]]
            : [["members", `👥 ${space.shown(sp)}'s members (the space keeps this app to its members)`]]),
          ...fewer,
        ]
      : [["public", "🌐 Everyone (public)"], ["followers", "👣 Your followers"], ["friends", "🤝 Your friends"], ["private", "🔒 Only you"]];
    const sel = h("select", { className: "field", name: "audience" }, ...choices.map(([value, textContent]) => h("option", { value, textContent })));
    // CHOSEN PEOPLE: the space's members ticked; a list made of them (`list` act) and chosen.
    let was = initial ?? sel.value;
    sel.onchange = async () => {
      if (sel.value !== "list:new") return void (was = sel.value);
      const picked = await choosePeople(sp, r, me);
      if (!picked?.length) return void (sel.value = was);
      const id = [...crypto.getRandomValues(new Uint8Array(6))].map(x => x.toString(16).padStart(2, "0")).join("");
      await r.act({ act: "list", list: id, people: picked });
      const o = h("option", { value: `list:${id}`, textContent: `🔐 ${picked.map(d => directory.shown(d)).join(", ").slice(0, 60)}` });
      sel.querySelector('option[value="list:new"]').before(o);
      sel.value = was = `list:${id}`;
    };
    // Its starting choice is the select's DEFAULT: a form reset (a composer closing) returns to it, never to the first.
    for (const o of sel.options) o.defaultSelected = o.value === initial;
    if (initial && choices.some(([v]) => v === initial)) sel.value = initial;
    // WHO MAY COMMENT AND VOTE — its own rule (`roles.mayWrite`, the one check), apart from who sees it, as a
    // database's read and write: in your space anyone · your followers · your friends · only you; in a space its
    // members (as its policy allows) · only you.
    const writes = sp
      ? [["members", "👥 Who the space allows"], ["author", "🔒 Only you"]]
      : [["anyone", "🌐 Anyone who sees it"], ["followers", "👣 Your followers"], ["friends", "🤝 Your friends"], ["author", "🔒 Only you"]];
    // ONE choice for every interaction (comment, vote, and each signal's own: react, save, share, tag — `signals`);
    // CUSTOMIZE sets one apart ("" : as the one choice).
    const S = await ctx.require("signals");
    const ACTS = S.interactions().map(a => [a.action, a.label]);
    const wsel = h("select", { className: "field", name: "write" }, ...writes.map(([value, textContent]) => h("option", { value, textContent })));
    const each = Object.fromEntries(ACTS.map(([a]) => [a, h("select", { className: "field", name: `write.${a}` }, h("option", { value: "", textContent: "As above" }), ...writes.map(([value, textContent]) => h("option", { value, textContent })))]));
    const custom = h("details", {}, h("summary", { textContent: `Customize each (${ACTS.map(([a]) => a).join(", ")})` }), ...ACTS.map(([a, label]) => h("label", {}, label, each[a])));
    // WHO CAN OPEN IT: a contribution LEVEL (rewards §5a), on top of who sees it.
    const lsel = h("select", { className: "field", name: "level" }, ...[["", "Anyone who sees it"], ["pro", "🔒 PRO and VIP"], ["vip", "🔒 VIP only"]].map(([value, textContent]) => h("option", { value, textContent })));
    const lnote = h("small", { textContent: "Shown only to that level in Craftworks — not encrypted yet, so another app could still read it." });
    const el = h("div", { className: "cw-aud", style: "display:grid;gap:6px" }, h("label", {}, "Who sees it", sel), h("label", {}, "Who can open it", lsel), lnote, h("label", {}, "Who may interact", wsel), custom);
    return {
      el,
      value: () => sel.value,
      isPublic: () => sel.value === "public",
      level: () => lsel.value,
      // The write rule ({ comment, vote, react, … }): each action's own choice, else the one choice — only what differs
      // from the space's own (nothing to set on the item: null).
      write: () => {
        const w = Object.fromEntries(ACTS.map(([a]) => [a, each[a].value || wsel.value]).filter(([, v]) => v !== "anyone" && v !== "members"));
        return Object.keys(w).length ? w : null;
      },
    };
  }
  // PICK PEOPLE among a space's members (not this person): a small dialog, ticks; the DIDs chosen, or null.
  function choosePeople(sp, r, me) {
    return new Promise(resolve => {
      const d = h("dialog", { className: "cw-pick" });
      const list = h("div", { style: "display:grid;gap:4px;max-height:50vh;overflow:auto" });
      const directory = ctx.require("directory");
      directory.then(dir => list.append(...r.members().filter(m => m.did !== me).map(m => h("label", { style: "display:flex;gap:8px;align-items:center" }, h("input", { type: "checkbox", value: m.did }), dir.nameEl(m.did)))));
      const ok = h("button", { type: "button", textContent: "Share with these" });
      const no = h("button", { type: "button", textContent: "Cancel" });
      let out = null;
      ok.onclick = () => ((out = [...list.querySelectorAll("input:checked")].map(i => i.value)), d.close());
      no.onclick = () => d.close();
      d.append(h("h3", { textContent: `Who in ${space.shown(sp)}` }), list, h("div", { style: "display:flex;gap:8px;justify-content:flex-end;margin-top:8px" }, no, ok));
      d.onclose = () => (d.remove(), resolve(out?.length ? out : null));
      document.body.append(d);
      d.showModal();
    });
  }
  return { picker };
}
