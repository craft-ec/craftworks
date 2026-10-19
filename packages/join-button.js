// JOIN BUTTON, a component: joining a space seen from outside (Discover) — JOIN; once asked, REQUESTED (when, and that
// a member who may invite lets you in: it takes a moment, and happens even while their app is closed); once in, OPEN.
// One look wherever a space can be joined (Board's and Chat's Discover views). The request is kept on the account
// (`conversation.asked`), so every device shows it; it is gone once the space is yours.
//
//   const join = await ctx.require("join-button");
//   host.append(await join.control(desc, { open: `#/s/${desc.id}/chat` }))   // desc: { id, name, governance }
export async function start(ctx) {
  const [conversation, space] = await Promise.all(["conversation", "space"].map(n => ctx.require(n)));
  const style = document.createElement("style");
  style.textContent = `@layer components {

    .cw-join { display: grid; gap: 4px; justify-items: start; }
    .cw-join button, .cw-join a { font: inherit; border: 0; border-radius: var(--cw-radius-pill); padding: 6px var(--cw-space-4); font-weight: 600;
      background: var(--cw-accent); color: var(--cw-accent-fg); cursor: pointer; text-decoration: none; }
    .cw-join button.follow { background: var(--cw-surface); color: var(--cw-fg); border: 1px solid var(--cw-line); }
    .cw-join button[disabled] { background: var(--cw-surface); color: var(--cw-muted); border: 1px solid var(--cw-line); cursor: default; }
    .cw-join p { margin: 0; color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .cw-join p.err { color: var(--cw-danger); }
}`;
  document.head.append(style);
  const h = (await ctx.require("theme")).h;
  const ago = at => {
    const s = Math.max(0, Math.round((Date.now() - at) / 1000));
    if (s < 60) return "just now";
    for (const [n, u] of [[86400, "d"], [3600, "h"], [60, "min"]]) if (s >= n) return `${Math.floor(s / n)} ${u} ago`;
  };

  // FOLLOW a space from outside (the same edge as following a person: their personal space): its public items in the
  // follower's feed, without joining.
  const people = await (await ctx.require("edge")).people();
  const followButton = async d => {
    const on = await people.is("follow", d.id);
    return h("button", {
      type: "button",
      className: "follow",
      textContent: on ? "Following ✓" : "Follow",
      onclick: async e => {
        e.currentTarget.disabled = true;
        await people.set("follow", d.id, !on, Date.now(), on ? null : { id: d.id, name: d.name ?? "", governance: d.governance });
      },
    });
  };
  // `policy`: the space's join policy as seen from outside — the open door ("anyone": in at once), a request ("ask"), or
  // by invite only (a Follow button).
  async function control(d, { open = `#/s/${d.id}`, policy = "anyone" } = {}) {
    const joinable = policy === "anyone" ? true : policy === "ask" ? "ask" : false;
    const box = h("div", { className: "cw-join" });
    const draw = async () => {
      if ((await space.mine.strict()).some(s => s.id === d.id)) return box.replaceChildren(h("a", { href: open, textContent: "Open" }));
      // ASKED (a request its people decide): said so, with CANCEL (owner 10-17: no request hangs).
      const asked = await conversation.asked(d.id);
      if (!asked && !joinable) return box.replaceChildren(await followButton(d));
      if (asked) {
        const cancel = h("button", {
          type: "button",
          className: "cw-flat",
          textContent: "Cancel request",
          onclick: async () => ((cancel.disabled = true), await conversation.cancelJoin(d.id).then(draw, err => ((cancel.disabled = false), ctx.failed("join", err, ctx.say(err))))),
        });
        return box.replaceChildren(h("button", { type: "button", disabled: true, textContent: "Requested ✓", title: `Asked ${ago(asked.at)}` }), cancel, h("p", { textContent: `Asked ${ago(asked.at)}. A member lets you in or says no.` }));
      }
      const said = h("p", { className: "err", hidden: true });
      // The OPEN door: in at once, by itself; a space that asks: a request.
      const b = h("button", {
        type: "button",
        textContent: joinable === "ask" ? "Request to join" : "Join",
        onclick: async () => {
          b.disabled = true;
          b.textContent = joinable === "ask" ? "Asking…" : "Joining…";
          await conversation.joinOpen(d).then(draw, err => ((said.textContent = ctx.say(err)), (said.hidden = false), (b.disabled = false), (b.textContent = joinable === "ask" ? "Request to join" : "Join")));
        },
      });
      box.replaceChildren(h("div", { style: "display:flex;gap:8px" }, b, await followButton(d)), said);
    };
    await draw();
    people.onChange(() => box.isConnected && draw());
    // In at last: Open.
    // Drawn again when this person's spaces change (a join let in): the one data signal — every table's change, the
    // page's and an app frame's alike.
    const again = () => (box.isConnected ? draw() : removeEventListener("craftworks:data", again));
    addEventListener("craftworks:data", again);
    return box;
  }
  return { control };
}
