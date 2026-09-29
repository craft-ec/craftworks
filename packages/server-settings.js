// SERVER SETTINGS, a component: a space's GOVERNANCE (who owns it, handing it on), ADMINISTRATION (members and their
// roles, channels, invite codes) and MODERATION (removing people; the log of every act that counted). UI only: roles
// and acts are `roles`', removals `moderation`'s, invites and admissions `conversation`'s, names `directory`'s; the
// server's channels are the caller's (`channels`: { list, add, rename, remove }).
//
//   const settings = await ctx.require("server-settings");
//   settings.open(server, { tab: "members", channels })
export async function start(ctx) {
  const [roles, moderation, conversation, directory, theme] = await Promise.all(["roles", "moderation", "conversation", "directory", "theme"].map(n => ctx.require(n)));
  const style = document.createElement("style");
  style.textContent = `
    .cw-set { border: 0; border-radius: var(--cw-radius); padding: 0; width: min(760px, calc(100vw - 32px)); height: min(620px, calc(100vh - 64px));
      box-shadow: var(--cw-shadow-lg); background: var(--cw-surface); color: var(--cw-fg); }
    .cw-set[open] { display: grid; grid-template-columns: 180px 1fr; }
    .cw-set nav { background: var(--cw-bg); border-right: 1px solid var(--cw-line); padding: var(--cw-space-3) var(--cw-space-2); display: grid; align-content: start; gap: 2px; }
    .cw-set nav h2 { font-size: var(--cw-text-xs); letter-spacing: .08em; color: var(--cw-muted); margin: var(--cw-space-2) var(--cw-space-2) var(--cw-space-2); text-transform: uppercase;
      overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .cw-set nav button { text-align: left; border: 0; background: none; color: var(--cw-fg); padding: 6px var(--cw-space-2); border-radius: var(--cw-radius-sm); cursor: pointer; font: inherit; }
    .cw-set nav button:hover { background: var(--cw-hover); }
    .cw-set nav button[aria-current="true"] { background: var(--cw-pressed); font-weight: 600; }
    .cw-set nav .close { margin-top: var(--cw-space-4); color: var(--cw-muted); }
    .cw-set main { overflow-y: auto; padding: var(--cw-space-4) var(--cw-space-5); min-width: 0; }
    .cw-set main h3 { margin: 0 0 var(--cw-space-3); font-size: 1.1rem; }
    .cw-set main p.note { color: var(--cw-muted); font-size: var(--cw-text-sm); margin: 0 0 var(--cw-space-3); }
    .cw-set .said { color: var(--cw-danger); font-size: var(--cw-text-sm); min-height: 1.2em; margin: 0 0 var(--cw-space-2); }
    .cw-set table { width: 100%; border-collapse: collapse; font-size: var(--cw-text-sm); }
    .cw-set td, .cw-set th { text-align: left; padding: 6px var(--cw-space-2); border-bottom: 1px solid var(--cw-line); vertical-align: middle; }
    .cw-set th { color: var(--cw-muted); font-weight: 500; }
    .cw-set tr.me td { background: var(--cw-hover); }
    .cw-set tr.focus td { outline: 2px solid var(--cw-accent); outline-offset: -2px; }
    .cw-set button.act, .cw-set select, .cw-set input { font: inherit; }
    .cw-set button.act { border: 1px solid var(--cw-line); background: none; color: var(--cw-fg); border-radius: var(--cw-radius-sm); padding: 2px var(--cw-space-2); cursor: pointer; }
    .cw-set button.act:hover { background: var(--cw-hover); }
    .cw-set button.act.main { background: var(--cw-accent); color: var(--cw-accent-fg); border-color: transparent; }
    .cw-set button.act.danger { color: var(--cw-danger); }
    .cw-set .row { display: flex; gap: var(--cw-space-2); align-items: center; flex-wrap: wrap; margin-bottom: var(--cw-space-3); }
    .cw-set code.code { font-size: 1.1rem; letter-spacing: .06em; padding: 4px 8px; border-radius: var(--cw-radius-sm); background: var(--cw-hover); }
    .cw-set dl { display: grid; grid-template-columns: max-content 1fr; gap: 6px var(--cw-space-4); margin: 0 0 var(--cw-space-4); }
    .cw-set dt { color: var(--cw-muted); }
    .cw-set dd { margin: 0; }
    .cw-set ol.log { list-style: none; margin: 0; padding: 0; display: grid; gap: 6px; font-size: var(--cw-text-sm); }
    .cw-set ol.log time { color: var(--cw-muted); margin-right: var(--cw-space-2); font-size: var(--cw-text-xs); }
    @media (max-width: 640px) { .cw-set[open] { grid-template-columns: 1fr; grid-template-rows: auto 1fr; } .cw-set nav { display: flex; overflow-x: auto; } }`;
  document.head.append(style);
  const el = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids);
    return e;
  };
  const when = t => (t ? new Date(t).toLocaleString() : "never");

  const TABS = [
    ["overview", "Overview"],
    ["members", "Members & roles"],
    ["invites", "Invites"],
    ["log", "Moderation log"],
  ];

  async function open(server, { tab = "overview", focus = null, channels, left = () => {} } = {}) {
    const [r, mod, me] = await Promise.all([roles.of(server), moderation.of(server), ctx.require("space").then(s => s.account())]);
    await r.refresh();
    const d = el("dialog", { className: "cw-set" });
    const nav = el("nav", {}, el("h2", { textContent: server.name }));
    const main = el("main");
    d.append(nav, main);
    document.body.append(d);
    let current = tab;
    const said = el("p", { className: "said" });
    const say = m => (said.textContent = m ? String(m) : "");
    // An action button. `confirm`: a lasting act asks again on the button itself ("Confirm: …") before it runs.
    const act = (label, run, cls = "", confirm = null) => {
      let armed = false;
      const b = el("button", { type: "button", className: `act ${cls}`, textContent: label });
      b.onclick = async () => {
        if (confirm && !armed) {
          armed = true;
          b.textContent = `Confirm: ${confirm}`;
          setTimeout(() => ((armed = false), (b.textContent = label)), 5000);
          return;
        }
        b.disabled = true;
        say("");
        try {
          await run();
          await r.refresh();
          draw();
        } catch (err) {
          say(`${label}: ${err?.message ?? err}`);
        } finally {
          b.disabled = false;
        }
      };
      return b;
    };
    const mine = () => r.role(me.id);
    const may = what => r.can(me.id, what);
    const RANK = { owner: 3, admin: 2, member: 1 };
    const names = new Map();
    const nameOf = did => {
      if (!names.has(did)) {
        names.set(did, directory.shown(did, null));
        directory.name(did).then(n => (names.set(did, n), d.open && draw()));
      }
      return names.get(did);
    };

    const pages = {
      overview() {
        main.append(
          el("h3", { textContent: "Overview" }),
          el(
            "dl",
            {},
            el("dt", { textContent: "Server" }),
            el("dd", { textContent: server.name }),
            el("dt", { textContent: "Owner" }),
            el("dd", { textContent: r.owner ? nameOf(r.owner) : "nobody provable (made before owners were proven)" }),
            el("dt", { textContent: "Your role" }),
            el("dd", { textContent: mine() ?? "not a member" }),
            el("dt", { textContent: "Members" }),
            el("dd", { textContent: String(r.members().length) }),
          ),
          el("p", { className: "note", textContent: "Owner: everything, and hands the server on. Admins: invite, channels, remove messages and members. Members: post and invite." }),
        );
        // LEAVE: out of this person's list. An owner with others still in hands it on first.
        const others = r.members().filter(m => m.did !== me.id);
        main.append(el("h3", { textContent: "Leave" }));
        if (mine() === "owner" && others.length)
          main.append(el("p", { className: "note", textContent: "You own this server: hand it to someone below before you leave." }));
        else
          main.append(
            el("p", { className: "note", textContent: "Takes the server out of your list, on every device of your account." }),
            el("div", { className: "row" }, act("Leave server", async () => {
              await (await ctx.require("space")).leave(server);
              d.close();
              left(server);
            }, "danger", `leave ${server.name}`)),
          );
        if (mine() === "owner") {
          const pick = el("select", {}, el("option", { value: "", textContent: "Hand ownership to…" }), ...r.members().filter(m => m.did !== me.id).map(m => el("option", { value: m.did, textContent: nameOf(m.did) })));
          main.append(
            el("h3", { textContent: "Ownership" }),
            el("p", { className: "note", textContent: "The new owner gets everything; you become an admin." }),
            el(
              "div",
              { className: "row" },
              pick,
              act(
                "Transfer ownership",
                async () => {
                  if (!pick.value) throw new Error("pick a member first");
                  await r.act({ act: "transfer", did: pick.value });
                },
                "danger",
                "hand it on",
              ),
            ),
          );
        }
      },
      members() {
        main.append(
          el("h3", { textContent: "Members & roles" }),
          el("p", { className: "note", textContent: mine() === "owner" ? "Set each person's role. Removing someone takes them out: they read nothing new, and cannot rejoin by a code." : "Only the owner sets roles. Admins remove members." }),
        );
        const rows = r.members().map(m => {
          const tr = el("tr", { className: `${m.did === me.id ? "me" : ""} ${m.did === focus ? "focus" : ""}` });
          const role = m.role;
          let roleCell;
          if (mine() === "owner" && m.did !== me.id && role !== "owner") {
            const sel = el("select", {}, ...["member", "admin"].map(v => el("option", { value: v, textContent: v, selected: v === role })));
            sel.onchange = async () => {
              say("");
              try {
                await r.grant(m.did, sel.value);
                await r.refresh();
                draw();
              } catch (err) {
                say(`Role: ${err?.message ?? err}`);
              }
            };
            roleCell = sel;
          } else roleCell = document.createTextNode(role);
          const actions = el("td");
          if (m.did !== me.id && may("remove") && RANK[mine()] > RANK[role])
            actions.append(
              act(
                "Remove",
                () => mod.remove(m.did),
                "danger",
                `remove ${nameOf(m.did)}`,
              ),
            );
          tr.append(el("td", { textContent: `${nameOf(m.did)}${m.did === me.id ? " (you)" : ""}`, title: m.did }), el("td", {}, roleCell), actions);
          return tr;
        });
        main.append(el("table", {}, el("thead", {}, el("tr", {}, el("th", { textContent: "Person" }), el("th", { textContent: "Role" }), el("th"))), el("tbody", {}, ...rows)));
      },
      channels() {
        main.append(el("h3", { textContent: "Channels" }));
        if (!channels) return main.append(el("p", { className: "note", textContent: "Open the server in Chat to manage its channels." }));
        const can = may("channels");
        main.append(el("p", { className: "note", textContent: can ? "Rename or delete a channel, or add one." : "Only the owner and admins change channels." }));
        const rows = channels.list().map(c => {
          const name = el("input", { value: c.name, disabled: !can });
          return el(
            "tr",
            {},
            el("td", {}, el("span", { textContent: "# " }), name),
            el(
              "td",
              {},
              ...(can
                ? [
                    act("Rename", () => channels.rename(c, name.value)),
                    act("Delete", () => channels.remove(c), "danger", `delete #${c.name}`),
                  ]
                : []),
            ),
          );
        });
        main.append(el("table", {}, el("tbody", {}, ...rows)));
        if (can) {
          const fresh = el("input", { placeholder: "new-channel" });
          main.append(el("div", { className: "row", style: "margin-top: var(--cw-space-3)" }, fresh, act("Add channel", () => channels.add(fresh.value), "main")));
        }
      },
      invites() {
        main.append(el("h3", { textContent: "Invites" }));
        if (!may("invite")) return main.append(el("p", { className: "note", textContent: "You cannot invite to this server." }));
        main.append(
          el("p", {
            className: "note",
            textContent: "Anyone with a code asks to join (the rail's + → Join a space). A member who may invite lets them in — their app when it is open, else their node on its own (it takes a moment).",
          }),
        );
        const who = el("input", { placeholder: "name#abc123 or did:craftec:…", style: "min-width: 18em" });
        main.append(
          el("div", { className: "row" }, who, act("Invite this person", async () => {
            const did = await conversation.person(who.value);
            await conversation.invite(server, did);
            who.value = "";
          }, "main")),
          el("p", { className: "note", textContent: "Or make a code to share:" }),
        );
        const days = el("select", {}, ...[["1", "1 day"], ["7", "7 days"], ["30", "30 days"], ["0", "never"]].map(([v, t]) => el("option", { value: v, textContent: `expires: ${t}`, selected: v === "7" })));
        const uses = el("select", {}, ...[["0", "no limit"], ["1", "1 use"], ["5", "5 uses"], ["25", "25 uses"]].map(([v, t]) => el("option", { value: v, textContent: t })));
        const made = el("div", { className: "row" });
        if (lastCode) made.append(el("code", { className: "code", textContent: lastCode }), act("Copy", () => navigator.clipboard.writeText(lastCode)));
        main.append(
          el(
            "div",
            { className: "row" },
            days,
            uses,
            act(
              "Create invite code",
              async () => {
                lastCode = await conversation.createInvite(server, { days: Number(days.value), uses: Number(uses.value) });
              },
              "main",
            ),
          ),
          made,
        );
        const list = r.invites();
        if (!list.length) return main.append(el("p", { className: "note", textContent: "No codes in force." }));
        main.append(
          el(
            "table",
            {},
            el("thead", {}, el("tr", {}, ...["Code", "Made by", "Expires", "Used", ""].map(t => el("th", { textContent: t })))),
            el(
              "tbody",
              {},
              ...list.map(i =>
                el(
                  "tr",
                  {},
                  el("td", {}, el("code", { textContent: i.code })),
                  el("td", { textContent: nameOf(i.by) }),
                  el("td", { textContent: when(i.expires) }),
                  el("td", { textContent: `${i.admitted.length}${i.uses ? ` of ${i.uses}` : ""}` }),
                  el("td", {}, act("Copy", () => navigator.clipboard.writeText(i.code)), ...(i.by === me.id || may("moderate") ? [act("Revoke", () => conversation.revokeInvite(server, i.code), "danger")] : [])),
                ),
              ),
            ),
          ),
        );
      },
      log() {
        main.append(el("h3", { textContent: "Moderation log" }), el("p", { className: "note", textContent: "Every act that counted, newest first. Acts nobody was allowed to make are never shown: they do not count." }));
        const say = a => {
          const who = nameOf(a.by);
          const whom = a.did ? nameOf(a.did) : "";
          switch (a.act) {
            case "grant":
              return `${who} made ${whom} ${a.role}`;
            case "remove":
              return `${who} removed ${whom}`;
            case "hide":
              return `${who} removed ${a.table?.endsWith("-channels") ? "a channel" : "a message"}`;
            case "transfer":
              return `${who} handed the server to ${whom}`;
            case "invite":
              return `${who} made invite ${a.code}`;
            case "revoke-invite":
              return `${who} revoked invite ${a.code}`;
            case "admitted":
              return `${who} let ${whom} in by invite ${a.code}`;
            default:
              return `${who}: ${a.act}`;
          }
        };
        const acts = r.acts().reverse();
        main.append(
          acts.length
            ? el("ol", { className: "log" }, ...acts.map(a => el("li", {}, el("time", { textContent: when(a.at) }), document.createTextNode(say(a)))))
            : el("p", { className: "note", textContent: "Nothing yet." }),
        );
      },
    };

    let lastCode = null; // the code just made: shown until another tab
    function draw() {
      nav.replaceChildren(
        nav.firstChild,
        ...TABS.map(([k, label]) => {
          const b = el("button", { type: "button", textContent: label, onclick: () => ((current = k), (lastCode = null), draw()) });
          b.setAttribute("aria-current", String(k === current));
          return b;
        }),
        el("button", { type: "button", className: "close", textContent: "Close", onclick: () => d.close() }),
      );
      main.replaceChildren(said);
      pages[current]();
    }
    r.onChange(() => d.open && draw());
    d.onclose = () => d.remove();
    // A click outside (on the backdrop: the dialog itself, not its panes) closes it, as Esc does.
    d.addEventListener("click", e => e.target === d && d.close());
    draw();
    d.showModal();
    return { close: () => d.close() };
  }

  return { open };
}
