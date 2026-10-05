// MEMBERS LIST, a component: a SPACE's people, each with their roles — the ONE list wherever a space's people show: its
// Contact app, Settings' "Members & roles", a channel's side panel (its readers). MANAGE: where this person may, each
// one's roles given and taken back (Admin: the owner's alone to give; a role only by who holds all it may) and people
// removed (below one's own rank). A name opens what can be done with that person (`person`). Drawn again as the
// space's acts change. UI only: roles and acts are `roles`', removals `moderation`'s, names `directory`'s.
//
//   const ml = await ctx.require("members-list");
//   host.append(await ml.show(sp, { manage, who }))   // manage: the tools above; who: only who may read a channel
//                                                      // ("admins", "role:<id>"…; none: every member)
export async function start(ctx) {
  const [roles, moderation, directory, person, space] = await Promise.all(["roles", "moderation", "directory", "person", "space"].map(n => ctx.require(n)));
  const style = document.createElement("style");
  style.textContent = `
    .cw-members { display: grid; gap: 2px; }
    .cw-members .m { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; padding: 6px 8px; border-radius: var(--cw-radius-sm); }
    .cw-members .m:hover { background: var(--cw-hover); }
    .cw-members .n { font-weight: 600; cursor: pointer; }
    .cw-members .chip.key.ok { color: var(--cw-muted); }
    .cw-members .chip.key.warn { color: var(--cw-danger); border-color: var(--cw-danger); }
    .cw-members .chip { font-size: var(--cw-text-xs); border: 1px solid var(--cw-line); border-radius: 999px; padding: 1px 8px; color: var(--cw-muted); display: inline-flex; gap: 4px; align-items: center; }
    .cw-members .acts { margin-left: auto; }
    .cw-members .acts button { font: inherit; font-size: var(--cw-text-sm); cursor: pointer; border: 0; background: none; color: var(--cw-danger); }
    .cw-members .said { color: var(--cw-danger); margin: 0; font-size: var(--cw-text-sm); }`;
  document.head.append(style);
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  const RANK = { owner: 3, admin: 2, member: 1 };

  async function show(sp, { manage = false, who = null } = {}) {
    const [r, mod, me] = await Promise.all([roles.of(sp), moderation.of(sp), space.account()]);
    const box = h("div", { className: "cw-members" });
    const said = h("p", { className: "said", hidden: true });
    const say = t => ((said.textContent = t ?? ""), (said.hidden = !t));
    const mine = () => r.role(me.id);
    const may = what => manage && r.can(me.id, what);
    // A lasting act asks again on its own button ("Confirm: …") before it runs.
    const act = (label, run, confirm) => {
      let armed = false;
      const b = h("button", { type: "button", textContent: label });
      b.onclick = async e => {
        e.stopPropagation();
        if (!armed) return ((armed = true), (b.textContent = `Confirm: ${confirm}`), setTimeout(() => ((armed = false), (b.textContent = label)), 5000));
        b.disabled = true;
        await run().then(() => r.refresh(), err => say(`${label}: ${err?.message ?? err}`));
        b.disabled = false;
      };
      return b;
    };
    const tick = (label, on, run) => {
      const c = h("input", { type: "checkbox", checked: on });
      c.onchange = () => (say(""), run(c.checked).then(() => r.refresh(), err => say(`Role: ${err?.message ?? err}`)));
      return h("label", { className: "chip", onclick: e => e.stopPropagation() }, c, label);
    };
    // EACH MEMBER's KEYS (`conversation.keyStatus`: what their devices announced) — seen by any member, so nobody has
    // to ask another for a log: on the owner's keys, behind, or on another branch, and who caught them up.
    // DIAGNOSTICS only (debugging: `edge.prefs` "diagnostics", Settings → Content).
    let keyState = null;
    ctx
      .require("edge")
      .then(e => e.prefs())
      .then(p => (p.get("diagnostics") ? ctx.require("conversation").then(c => c.keyStatus(sp)) : null))
      .then(k => k && ((keyState = k), box.isConnected && draw()), () => {});
    const keyChip = did => {
      const k = keyState?.members.get(did);
      if (!k) return null;
      const o = keyState.owner;
      const text = k.state === "current" ? "🔑 keys current" : k.state === "behind" ? `🔑 behind: epoch ${k.epoch} of ${o?.epoch}` : k.state === "another branch" ? `🔑 on another branch (epoch ${k.epoch})` : k.state === "not announced" ? "🔑 keys not announced" : `🔑 ${k.state}`;
      const by = k.repairedBy.length ? ` · caught up by ${[...new Set(k.repairedBy)].join(", ")}` : "";
      const chip = h("span", { className: `chip key ${k.state === "current" ? "ok" : "warn"}`, textContent: text + by });
      // What their page reported it cannot read here — shown in full (no hover on a touch screen).
      if (!k.unread?.length) return chip;
      return h("span", { className: "reads" }, chip, h("span", { className: "chip key warn", textContent: `⚠ can't read: ${k.unread.join(" · ")}` }));
    };
    const draw = () => {
      const people = r.members().filter(m => !who || r.passes(who, m.did));
      box.replaceChildren(
        ...people.map(m => {
          const role = m.role;
          const held = new Set(r.held(m.did));
          const chips = [];
          if (role === "owner") chips.push(h("span", { className: "chip", textContent: "Owner" }));
          else if (mine() === "owner" && manage && m.did !== me.id) chips.push(tick("Admin", role === "admin", on => r.grant(m.did, on ? "admin" : "member")));
          else if (role === "admin") chips.push(h("span", { className: "chip", textContent: "Admin" }));
          if (role !== "owner")
            for (const ro of r.roles()) {
              const mayGive = may("roles") && ro.perms.every(p => r.can(me.id, p));
              if (mayGive) chips.push(tick(ro.name, held.has(ro.id), on => r.act({ act: "assign", did: m.did, role: ro.id, on })));
              else if (held.has(ro.id)) chips.push(h("span", { className: "chip", textContent: ro.name }));
            }
          const remove = m.did !== me.id && may("remove") && RANK[mine()] > RANK[role] ? act("Remove", () => mod.remove(m.did), `remove ${directory.shown(m.did)}`) : null;
          return h(
            "div",
            { className: "m", title: m.did },
            directory.nameEl(m.did, "span", { className: "n", onclick: e => person.open(e.currentTarget, m.did, { space: sp }) }),
            m.did === me.id ? h("span", { className: "chip", textContent: "you" }) : null,
            ...chips,
            keyChip(m.did),
            remove ? h("span", { className: "acts" }, remove) : null,
          );
        }),
        said,
      );
    };
    draw();
    r.onChange(() => box.isConnected && draw());
    return box;
  }

  return { show };
}
