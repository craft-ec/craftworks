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
  style.textContent = `
    .cw-join { display: grid; gap: 4px; justify-items: start; }
    .cw-join button, .cw-join a { font: inherit; border: 0; border-radius: var(--cw-radius-pill); padding: 6px var(--cw-space-4); font-weight: 600;
      background: var(--cw-accent); color: var(--cw-accent-fg); cursor: pointer; text-decoration: none; }
    .cw-join button.follow { background: var(--cw-surface); color: var(--cw-fg); border: 1px solid var(--cw-line); }
    .cw-join button[disabled] { background: var(--cw-surface); color: var(--cw-muted); border: 1px solid var(--cw-line); cursor: default; }
    .cw-join p { margin: 0; color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .cw-join p.err { color: var(--cw-danger); }`;
  document.head.append(style);
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  const ago = at => {
    const s = Math.max(0, Math.round((Date.now() - at) / 1000));
    if (s < 60) return "just now";
    for (const [n, u] of [[86400, "d"], [3600, "h"], [60, "min"]]) if (s >= n) return `${Math.floor(s / n)} ${u} ago`;
  };
  const WAITING = "A member who may invite lets you in — it takes a moment, and happens even while their app is closed.";

  // FOLLOW a space from outside (the same edge as following a person: their personal space): its public items in the
  // follower's feed, without joining.
  const people = await (await ctx.require("edge")).people();
  const followButton = d => {
    const on = people.is("follow", d.id);
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
  async function control(d, { open = `#/s/${d.id}`, joinable = true } = {}) {
    const box = h("div", { className: "cw-join" });
    const draw = async () => {
      if ((await space.mine()).some(s => s.id === d.id)) return box.replaceChildren(h("a", { href: open, textContent: "Open" }));
      if (!joinable) return box.replaceChildren(followButton(d));
      const asked = await conversation.asked(d.id).catch(() => null);
      if (asked) return box.replaceChildren(h("button", { type: "button", disabled: true, textContent: "Requested ✓", title: `Asked ${ago(asked.at)}` }), h("p", { textContent: `Asked ${ago(asked.at)}. ${WAITING}` }));
      const said = h("p", { className: "err", hidden: true });
      const b = h("button", {
        type: "button",
        textContent: "Join",
        onclick: async () => {
          b.disabled = true;
          b.textContent = "Asking…";
          await conversation.joinOpen(d).then(draw, err => ((said.textContent = err.message), (said.hidden = false), (b.disabled = false), (b.textContent = "Join")));
        },
      });
      box.replaceChildren(h("div", { style: "display:flex;gap:8px" }, b, followButton(d)), said);
    };
    await draw();
    people.onChange(() => box.isConnected && draw());
    // In at last (the welcome accepted): Open.
    (await ctx.require("storage")).table("spaces").then(t => t.onChange(() => box.isConnected && draw()), () => {});
    return box;
  }
  return { control };
}
