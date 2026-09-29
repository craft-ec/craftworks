// PERSON, a component: what can be done with a person, from wherever their name is shown (a message, a member list, a
// conversation, a mail). PERSONAL — message them, friend, follow, their posts, hide (their items unseen here), block (hidden, and
// their welcomes, mail and requests refused) — and, in a SPACE where this person may, as its admin or moderator: their
// role, remove (back by an invite), ban (never back until unbanned). UI only: the links are `edge.people`'s, friend
// requests and direct conversations `conversation`'s, roles `roles`', removals and bans `moderation`'s.
//
//   const person = await ctx.require("person");
//   person.open(anchor, did, { space })   // a menu by `anchor` (the name clicked); `space`: the one it was clicked in
export async function start(ctx) {
  const [edge, conversation, directory, roles, moderation, space] = await Promise.all(["edge", "conversation", "directory", "roles", "moderation", "space"].map(n => ctx.require(n)));
  const style = document.createElement("style");
  style.textContent = `
    .cw-person { position: fixed; z-index: 50; width: 280px; background: var(--cw-surface); color: var(--cw-fg); border: 1px solid var(--cw-line);
      border-radius: var(--cw-radius); box-shadow: var(--cw-shadow-lg); padding: var(--cw-space-3); display: grid; gap: var(--cw-space-2); font-size: var(--cw-text-sm); }
    .cw-person h3 { margin: 0; font-size: 1rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .cw-person .id { color: var(--cw-muted); font-size: var(--cw-text-xs); word-break: break-all; cursor: copy; }
    .cw-person .head { color: var(--cw-muted); font-size: var(--cw-text-xs); letter-spacing: .08em; text-transform: uppercase; margin-top: var(--cw-space-1); }
    .cw-person .grid { display: flex; flex-wrap: wrap; gap: 6px; }
    .cw-person button, .cw-person select { font: inherit; border: 1px solid var(--cw-line); background: none; color: var(--cw-fg); border-radius: var(--cw-radius-sm);
      padding: 3px var(--cw-space-2); cursor: pointer; }
    .cw-person button:hover { background: var(--cw-hover); }
    .cw-person button.main { background: var(--cw-accent); color: var(--cw-accent-fg); border-color: transparent; }
    .cw-person button.on { background: var(--cw-pressed); }
    .cw-person button.danger { color: var(--cw-danger); }
    .cw-person .said { color: var(--cw-danger); font-size: var(--cw-text-xs); min-height: 1em; margin: 0; }`;
  document.head.append(style);
  const el = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids);
    return e;
  };
  let openBox = null;
  const close = () => {
    openBox?.remove();
    openBox = null;
  };
  addEventListener("keydown", e => e.key === "Escape" && close());
  addEventListener("mousedown", e => openBox && !openBox.contains(e.target) && close(), true);
  addEventListener("hashchange", close);

  async function open(anchor, did, { space: sp = null } = {}) {
    close();
    const me = await space.account();
    if (!me) return;
    const box = el("div", { className: "cw-person", role: "dialog" });
    openBox = box;
    const at = anchor.getBoundingClientRect();
    box.style.left = `${Math.max(8, Math.min(at.left, innerWidth - 296))}px`;
    box.style.top = `${Math.min(at.bottom + 6, innerHeight - 380)}px`;
    document.body.append(box);
    const said = el("p", { className: "said" });
    const [people, r, handle] = await Promise.all([edge.people(), sp ? roles.of(sp.parent ?? sp) : null, directory.handle(did)]);
    const mod = sp ? await moderation.of(sp.parent ?? sp) : null;
    const RANK = { owner: 3, admin: 2, member: 1 };
    // A button that does something, then redraws; `confirm`: a lasting act asks again on the button itself.
    const act = (label, run, cls = "", confirm = null) => {
      let armed = false;
      const b = el("button", { type: "button", className: cls, textContent: label });
      b.onclick = async () => {
        if (confirm && !armed) {
          armed = true;
          b.textContent = `Confirm: ${confirm}`;
          return;
        }
        b.disabled = true;
        said.textContent = "";
        try {
          await run();
          if (openBox === box) draw();
        } catch (e) {
          said.textContent = `${label}: ${e?.message ?? e}`;
          b.disabled = false;
        }
      };
      return b;
    };
    const toggle = (rel, on, off) => act(people.is(rel, did) ? off : on, () => people.set(rel, did, !people.is(rel, did)), people.is(rel, did) ? "on" : "");

    function draw() {
      const self = did === me.id;
      const posts = act("Posts", () => {
        close();
        location.hash = `#/board/u/${did}`;
      });
      const personal = self
        ? [el("p", { className: "id", textContent: "This is you." }), el("div", { className: "grid" }, posts)]
        : [
            el(
              "div",
              { className: "grid" },
              act(
                "Message",
                async () => {
                  const c = await conversation.direct(did);
                  close();
                  location.hash = `#/messages/${c.id}`;
                },
                "main",
              ),
              people.is("friend", did)
                ? act("Friends ✓", () => conversation.unfriend(did), "on", "unfriend")
                : people.is("asked", did)
                  ? el("button", { type: "button", textContent: "Friend request sent", disabled: true })
                  : act("Add friend", () => conversation.befriend(did)),
              toggle("follow", "Follow", "Following ✓"),
              posts,
            ),
            el(
              "div",
              { className: "grid" },
              toggle("hide", "Hide their messages", "Hidden · show again"),
              people.is("block", did) ? act("Blocked · unblock", () => people.set("block", did, false), "on") : act("Block", () => people.set("block", did, true), "danger", "block them"),
            ),
          ];
      const server = [];
      if (r && !self) {
        const mine = r.role(me.id);
        const theirs = r.role(did);
        const rows = [];
        if (mine === "owner" && theirs && theirs !== "owner") {
          const sel = el("select", {}, ...["member", "admin"].map(v => el("option", { value: v, textContent: `role: ${v}`, selected: v === theirs })));
          sel.onchange = () => r.grant(did, sel.value).then(() => r.refresh()).then(draw, e => (said.textContent = e.message));
          rows.push(sel);
        } else if (theirs) rows.push(el("span", { textContent: `role: ${theirs}` }));
        if (r.can(me.id, "remove") && RANK[mine] > RANK[theirs ?? "member"]) {
          if (theirs) rows.push(act("Remove", () => mod.remove(did), "danger", "remove"));
          rows.push(r.banned(did) ? act("Banned · unban", () => mod.unban(did), "on") : act("Ban", () => mod.ban(did), "danger", "ban"));
        }
        if (rows.length) server.push(el("div", { className: "head", textContent: `In ${(sp.parent ?? sp).name}` }), el("div", { className: "grid" }, ...rows));
      }
      box.replaceChildren(
        el("h3", { textContent: directory.shown(did, handle) }),
        el("div", { className: "id", textContent: did, title: "Copy the id", onclick: () => navigator.clipboard.writeText(did) }),
        ...personal,
        ...server,
        said,
      );
    }
    draw();
  }

  return { open, close };
}
