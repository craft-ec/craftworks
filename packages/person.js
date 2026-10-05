// PERSON, a component: what can be done with a person, from wherever their name is shown (a message, a member list, a
// conversation, a mail). PERSONAL — message them, friend, follow, their posts, hide (their items unseen here), block (hidden, and
// their welcomes, mail and requests refused) — and, in a SPACE where this person may, as its admin or moderator: their
// role, remove (back by an invite), ban (never back until unbanned). UI only: the links are `edge.people`'s, friend
// requests and direct conversations `conversation`'s, roles `roles`', removals and bans `moderation`'s.
//
//   const person = await ctx.require("person");
//   person.open(anchor, did, { space })   // a menu by `anchor` (the name clicked); `space`: the one it was clicked in
//   person.openSpace(anchor, desc)        // a SPACE's card (its name clicked): about, members, apps, Join · Open
export async function start(ctx) {
  const [edge, conversation, directory, roles, moderation, space] = await Promise.all(["edge", "conversation", "directory", "roles", "moderation", "space"].map(n => ctx.require(n)));
  const style = document.createElement("style");
  style.textContent = `
    /* Above every panel and menu (the spaces panel is 70): a card opened from one shows over it. */
    .cw-person { position: fixed; z-index: 90; width: 280px; background: var(--cw-surface); color: var(--cw-fg); border: 1px solid var(--cw-line);
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
    e.append(...kids.filter(k => k != null && k !== false));
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
    // Roles, removal and bans only in a SHARED space (a server): a conversation is between equals.
    const shared = sp && (sp.parent ?? sp).kind === "server" ? sp.parent ?? sp : null;
    const [people, r, handle] = await Promise.all([edge.people(), shared ? roles.of(shared) : null, directory.handle(did)]);
    const mod = shared ? await moderation.of(shared) : null;
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

    // FLAG, as everywhere (`moderation.lists`: your public list).
    const flagToggle = async (kind, ref, label) => {
      const lists = await moderation.lists();
      const on = lists.isFlagged(kind, ref);
      return act(on ? "Flagged · unflag" : label, () => lists.setFlagged(kind, ref, !on), on ? "on" : "");
    };
    async function draw() {
      const self = did === me.id;
      // OPEN: their HOME (their personal space: its apps), never one app of it.
      const posts = act("Open", () => {
        close();
        location.hash = self ? "#/" : `#/u/${did}`;
      }, "main");
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
                  location.hash = `#/chat/${c.id}`;
                },
              ),
              people.is("friend", did)
                ? act("Friends ✓", () => conversation.unfriend(did), "on", "unfriend")
                : people.is("asked", did)
                  ? el("button", { type: "button", textContent: "Friend request sent", disabled: true })
                  : act("Add friend", () => conversation.befriend(did)),
              toggle("follow", "Follow", "Following ✓"),
              act("✉ Mail", () => (close(), (location.hash = `#/mail/to/${encodeURIComponent(did)}`))),
              posts,
              toggle("modlist", "Use their moderation list", "Their list applies ✓"),
            ),
            el(
              "div",
              { className: "grid" },
              // HIDE (private, everywhere) and FLAG (public: your list, Discover) — as on an item and a space.
              toggle("hide", "Hide everything they post", "Hidden · show again"),
              await flagToggle("person", did, "Flag author"),
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

  // A SPACE's CARD (the same card, for a space): its name, what it says it is (its `about`), its members and apps, and
  // Join · Requested · Open (`join-button`) — Open: its HOME (its apps), never one app of it.
  async function openSpace(anchor, desc) {
    close();
    const box = el("div", { className: "cw-person", role: "dialog" });
    openBox = box;
    const at = anchor.getBoundingClientRect();
    box.style.left = `${Math.max(8, Math.min(at.left, innerWidth - 296))}px`;
    box.style.top = `${Math.min(at.bottom + 6, innerHeight - 320)}px`;
    document.body.append(box);
    box.replaceChildren(el("h3", { textContent: space.shown(desc) }), el("p", { className: "id", textContent: "Reading it…" }));
    const mine = (await space.mine()).some(s => s.id === desc.id);
    const pr = mine ? await roles.of((await space.mine()).find(s => s.id === desc.id)) : await roles.ofPublic(desc);
    await pr.settled;
    const about = pr.config?.("space", "about", "") ?? "";
    const apps = ctx.apps.filter(a => (a.views ?? []).includes("shared") && !a.always && pr.apps().includes(a.route.slice(1)));
    const n = pr.members().length;
    const join = await (await ctx.require("join-button")).control(desc, { open: `#/s/${desc.id}`, joinable: pr.policy("", "join") === "anyone" });
    // Its MAIL, where its owner turned it on: written to from here, member or not.
    const mailable = await conversation.mail.accepts(desc.id).catch(() => false);
    // HIDE (private, everywhere) and FLAG (public: your list, Discover) the whole space — as a post and an author.
    const lists = await moderation.lists();
    const hideFlag = () =>
      el(
        "div",
        { className: "grid" },
        act0(lists.isHidden({ space: desc.id }) ? "Hidden · show again" : "Hide space", async e => (await lists.setHidden({ space: desc.id }, !lists.isHidden({ space: desc.id })), e.target.closest(".grid").replaceWith(hideFlag()))),
        act0(lists.isFlagged("space", desc.id) ? "Flagged · unflag" : "Flag space", async e => (await lists.setFlagged("space", desc.id, !lists.isFlagged("space", desc.id)), e.target.closest(".grid").replaceWith(hideFlag()))),
      );
    if (openBox !== box) return;
    box.replaceChildren(
      el("h3", { textContent: space.shown(desc) }),
      about ? el("p", { textContent: String(about).slice(0, 400) }) : el("p", { className: "id", textContent: "A space." }),
      el("p", { className: "id", textContent: `${n} member${n === 1 ? "" : "s"} · ${apps.map(a => `${a.icon ?? ""} ${a.name}`).join("  ") || "no apps yet"}` }),
      el("div", { className: "grid" }, act0("Open", () => (close(), (location.hash = `#/s/${desc.id}`)), "main"), mailable ? act0("✉ Mail", () => (close(), (location.hash = `#/mail/to/${encodeURIComponent(`space:${desc.id}`)}`))) : null),
      join,
      mine ? null : hideFlag(),
    );
  }
  const act0 = (label, run, cls = "") => el("button", { type: "button", className: cls, textContent: label, onclick: run });

  return { open, openSpace, close };
}
