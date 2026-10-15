// SPACE, an app (ARCHITECTURE §1, "The desktop"): where spaces are made, joined and KEPT — their settings and config,
// never their content (owner 10-13: a space's content is each app's). `#/spaces`: the shared spaces this person is in,
// those asked to join, Make one, Join with a code. `#/spaces/<id>[/<tab>]`: one space's settings (`server-settings`:
// overview, capabilities and their rules, members and roles, invites, the moderation log) — or, for a space this person
// is not in, what joining needs. UI only: spaces are `space`'s, joining `conversation`'s, the rules `roles`'.
export async function mount(ctx, el) {
  const [space, roles, conversation, theme] = await Promise.all(["space", "roles", "conversation", "theme"].map(n => ctx.require(n)));
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  el.innerHTML = `<style>@layer components {
    .sa { max-width: 760px; margin: 0 auto; padding: 16px; }
    .sa ul { list-style: none; padding: 0; }
    .sa li { padding: 8px 0; border-bottom: 1px solid var(--line, #8883); display: flex; gap: 12px; align-items: center; }
    .sa form { display: flex; gap: 8px; margin: 8px 0; flex-wrap: wrap; }
    .sa .caps label { display: flex; gap: 8px; align-items: center; padding: 6px 0; }
    .sa .said { color: var(--bad, #b3261e); }
  }</style><div class="sa"></div>`;
  const root = el.querySelector(".sa");
  const said = h("p", { className: "said", hidden: true });
  const fail = e => ((said.textContent = ctx.say(e)), (said.hidden = false));

  async function list() {
    const mine = (await space.mine()).filter(s => s.kind === "server");
    const make = h("form", {}, h("input", { name: "name", placeholder: "A new space's name", autocomplete: "off", required: true, ariaLabel: "New space" }), h("button", { textContent: "Make" }));
    make.onsubmit = async e => {
      e.preventDefault();
      try {
        const s = await space.create("server", make.elements.name.value.trim());
        location.hash = `#/spaces/${s.id}`;
      } catch (err) {
        fail(err);
      }
    };
    const join = h("form", {}, h("input", { name: "code", placeholder: "An invite code", autocomplete: "off", required: true, ariaLabel: "Invite code" }), h("button", { textContent: "Join" }));
    join.onsubmit = async e => {
      e.preventDefault();
      try {
        await conversation.join(join.elements.code.value);
        said.hidden = false;
        said.className = "";
        said.textContent = "Requested ✓ A member who may invite lets you in; the space appears here when you are in.";
      } catch (err) {
        fail(err);
      }
    };
    // REQUESTED: the spaces asked to join, each with where it stands (the spaces panel's list, which went with it —
    // audit 10-13). Read after the rest: the page never waits on it.
    const requested = h("div", { hidden: true });
    const STATUS = { asked: "asked · waiting for a member who may invite", "let-in": "let in · opening the welcome", code: "asked by a code · waiting" };
    ctx.optional("spaces: waiting", conversation.waiting(), () => []).then(async ws => {
      if (!ws?.length || !requested.isConnected) return;
      const person = await ctx.require("person");
      requested.hidden = false;
      requested.replaceChildren(
        h("h3", { textContent: "Requested" }),
        h("ul", {}, ...ws.map(w => h("li", {}, h("button", { type: "button", className: "cw-flat", title: w.id ?? w.code, textContent: w.name ?? (w.code ? `code ${w.code}` : `a space #${String(w.id).slice(0, 6)}`), onclick: e => w.desc && person.openSpace(e.currentTarget, w.desc) }), h("small", { textContent: STATUS[w.status] ?? w.status ?? "" })))),
      );
    });
    root.replaceChildren(
      h("h2", { textContent: "Your spaces" }),
      mine.length ? h("ul", {}, ...mine.map(s => h("li", {}, h("a", { href: `#/spaces/${s.id}`, textContent: space.shown(s) }), h("a", { href: `#/s/${s.id}`, textContent: "Open" })))) : h("p", { textContent: "You are in no shared space yet." }),
      requested,
      h("h3", { textContent: "Make one" }),
      make,
      h("h3", { textContent: "Join one" }),
      join,
      said,
    );
  }

  // ONE SPACE: its SETTINGS AND CONFIG only (owner 10-13: its content is each app's) — for a member the one settings
  // page (`server-settings`: overview, capabilities and their rules, members and roles, invites, the moderation log);
  // for a space this person is not in, what joining needs: what it says it is, how many are in it, Join or Request.
  async function one(id, tab, focus) {
    const sp = await space.find(id);
    if (!sp || sp.kind !== "server") return outside(id);
    const host = h("div", { className: "cw-fill" });
    root.replaceChildren(h("p", {}, h("a", { href: "#/spaces", textContent: "← Your spaces" })), host);
    await (await ctx.require("server-settings")).render(host, sp, { tab: tab || "overview", focus: focus || null, left: () => (location.hash = "#/spaces") });
  }
  async function outside(id) {
    const desc = await (await ctx.require("items")).publicSpace(id);
    if (!desc) return root.replaceChildren(h("p", { textContent: "Not listed: join it with an invite code." }), h("a", { href: "#/spaces", textContent: "← Your spaces" }));
    const pr = await roles.ofPublic(desc);
    await pr.settled;
    const about = pr.config?.("space", "about", "") ?? "";
    const n = pr.members().length;
    const join = await (await ctx.require("join-button")).control(desc, { open: `#/spaces/${desc.id}`, joinable: pr.policy("", "join") === "anyone" });
    root.replaceChildren(
      h("p", {}, h("a", { href: "#/spaces", textContent: "← Your spaces" })),
      h("h2", { textContent: space.shown(desc) }),
      about ? h("p", { textContent: String(about) }) : null,
      h("p", { textContent: `${n} member${n === 1 ? "" : "s"} · you are not in it` }),
      join,
      h("p", { textContent: "Its content opens in each app (Chat, Board, Wiki …) once you are in, or from outside where the space lets anyone read." }),
    );
  }

  const draw = () => {
    said.hidden = true;
    const [id, tab, focus] = (ctx.sub ?? "").split("/");
    root.replaceChildren(theme.loading?.("Opening…") ?? "");
    return (id ? one(id, tab, focus) : list()).catch(fail);
  };
  addEventListener("craftworks:route", () => el.isConnected && ctx.route === "/spaces" && draw(), { signal: ctx.signal });
  await draw();
}
