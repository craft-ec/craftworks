// MAIL, a page: mail to anyone by their id — the Inbox (what others sent: opened from their own tails, kept here), Sent,
// and Compose; a mail open beside the list, with Reply. UI only: a mail is `conversation.mail`'s, names `directory`'s,
// people typed as `name#abc123` or their id resolved by `conversation.person` (a space: `space:<id>`). The top bar:
// Inbox · Sent · Compose. IN A SPACE: the space's own mail (`conversation.mail.of`) — its address `space:<id>`, its
// owner and admins writing and reading as the space; its owner turns it on.
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  const [conversation, directory, theme, person, attachments, markdown, mdEditor] = await Promise.all(["conversation", "directory", "theme", "person", "attachments", "markdown", "md-editor"].map(n => ctx.require(n)));
  const box = ctx.sub === "sent" ? "sent" : "in";
  // WHOSE MAIL: this person's, or (in a space) the space's.
  const sp = ctx.space ? ((await (await ctx.require("space")).mine()).find(s => s.id === ctx.space) ?? null) : null;
  const src = sp ? await conversation.mail.of(sp) : conversation.mail;
  const base = sp ? `#/s/${sp.id}/mail` : "#/mail";
  if (sp) {
    const say = t => (el.replaceChildren(Object.assign(document.createElement("p"), { className: "none", textContent: t, style: "padding:var(--cw-space-5);text-align:center;color:var(--cw-muted)" })), el.firstChild);
    if (!src.may()) return void say(`${sp.name}'s mail is its owner's and admins' to use.`);
    if (!(await src.enabled())) {
      const p = say(`${sp.name} has no mail address yet.`);
      if ((await (await ctx.require("roles")).of(sp)).role((await (await ctx.require("space")).account()).id) === "owner")
        p.append(" ", Object.assign(document.createElement("button"), { type: "button", textContent: "Turn on its mail", onclick: async e => ((e.target.disabled = true), await src.enable().then(() => mount(ctx, (el.replaceChildren(), el)), err => ((e.target.disabled = false), p.append(` ${err.message}`)))) }));
      return;
    }
  }
  ctx.actions["/mail"] = [
    { label: "Inbox", href: base, on: box === "in" },
    { label: "Sent", href: `${base}/sent`, on: box === "sent" },
    { label: "Compose", run: () => compose() },
  ];
  dispatchEvent(new CustomEvent("craftworks:actions"));
  // WHO a mail is from or to: a person (their name, their menu), or a space (its name).
  const spaceName = (m, ref) => (m.space?.id && ref === `space:${m.space.id}` ? m.space.name : sp && ref === `space:${sp.id}` ? sp.name : "a space");
  const nameOf = (m, ref, props) => (String(ref).startsWith("space:") ? Object.assign(document.createElement("span"), { textContent: `🏠 ${spaceName(m, ref)}` }) : directory.nameEl(ref, ...(props ? ["a", props(ref)] : [])));
  el.classList.add("cw-fill");
  el.innerHTML = `
    <style>
      .ml { display: grid; grid-template-columns: 340px 1fr; min-height: 420px; overflow: hidden; background: var(--cw-surface); }
      .ml button { font: inherit; cursor: pointer; }
      .ml .list { background: var(--cw-bg); border-right: 1px solid var(--cw-line); overflow-y: auto; min-width: 0; }
      .ml .list button { display: grid; gap: 2px; width: 100%; text-align: left; border: 0; border-bottom: 1px solid var(--cw-line);
        background: none; color: var(--cw-fg); padding: var(--cw-space-2) var(--cw-space-3); }
      .ml .list button:hover { background: var(--cw-hover); }
      .ml .list button[aria-current="true"] { background: var(--cw-pressed); }
      .ml .list .who { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .ml .list .sub, .ml .list .when { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: var(--cw-text-sm); }
      .ml .list .when { color: var(--cw-muted); font-size: var(--cw-text-xs); }
      .ml .read { overflow-y: auto; padding: var(--cw-space-4) var(--cw-space-5); min-width: 0; }
      .ml .read h2 { margin: 0 0 var(--cw-space-2); font-size: 1.2rem; }
      .ml .read .meta { color: var(--cw-muted); font-size: var(--cw-text-sm); margin-bottom: var(--cw-space-4); }
      .ml .read .body { overflow-wrap: anywhere; line-height: 1.5; }
      .ml .read .reply { margin-top: var(--cw-space-4); border: 1px solid var(--cw-line); background: none; color: var(--cw-accent);
        border-radius: var(--cw-radius-sm); padding: var(--cw-space-1) var(--cw-space-3); }
      .ml .empty { color: var(--cw-muted); text-align: center; padding: var(--cw-space-5); }
      .ml .said { color: var(--cw-danger); font-size: var(--cw-text-sm); padding: var(--cw-space-2) var(--cw-space-3); margin: 0; }
      .ml dialog { border: 0; border-radius: var(--cw-radius); padding: var(--cw-space-4); width: min(560px, calc(100vw - 32px));
        box-shadow: var(--cw-shadow-lg); }
      .ml dialog form { display: grid; gap: var(--cw-space-3); }
      .ml dialog label { display: grid; gap: var(--cw-space-1); font-size: var(--cw-text-sm); color: var(--cw-muted); }
      .ml dialog textarea { min-height: 12em; resize: vertical; font: inherit; }
      .ml dialog .row { display: flex; gap: var(--cw-space-2); justify-content: flex-end; align-items: center; }
      .ml dialog .row .said { flex: 1; padding: 0; }
      .ml dialog button { border: 1px solid var(--cw-line); background: none; color: inherit; border-radius: var(--cw-radius-sm);
        padding: var(--cw-space-1) var(--cw-space-3); }
      .ml dialog button[value="ok"] { background: var(--cw-accent); color: var(--cw-accent-fg); border-color: transparent; }
      @media (max-width: 700px) { .ml { grid-template-columns: 1fr; } .ml.reading .list { display: none; } .ml:not(.reading) .read { display: none; } }
    </style>
    <div class="ml">
      <nav class="list" aria-label="${box === "sent" ? "Sent" : "Inbox"}"></nav>
      <article class="read"><p class="empty">Pick a mail.</p></article>
      <dialog class="compose"><form method="dialog">
        <label>To — name#abc123 or ids (did:craftec:…), separated by commas <input name="to" autocomplete="off" required></label>
        <label>Subject <input name="subject" autocomplete="off"></label>
        <div class="message"></div>
        <div class="row"><p class="said" hidden></p><button value="cancel" formnovalidate>Cancel</button><button value="ok">Send</button></div>
      </form></dialog>
    </div>`;
  const $ = s => el.querySelector(s);
  const root = $(".ml"), list = $(".list"), read = $(".read"), dlg = $(".compose");
  const node = (tag, props) => Object.assign(document.createElement(tag), props);
  let open = null;

  async function draw() {
    const mails = await src.list(box);
    if (!mails.length) {
      list.replaceChildren(node("p", { className: "empty", textContent: box === "sent" ? "Nothing sent yet." : "No mail yet." }));
      return;
    }
    const toNames = m => m.to.flatMap((d, i) => [...(i ? [", "] : []), nameOf(m, d)]);
    list.replaceChildren(
      ...mails.map((m, i) => {
        const b = node("button", { type: "button" });
        b.setAttribute("aria-current", String(open?.id === m.id));
        const who = node("span", { className: "who" });
        who.append(...(box === "sent" ? ["To ", ...toNames(m)] : [nameOf(m, m.from)]));
        b.append(who, node("span", { className: "sub", textContent: m.subject || "(no subject)" }), node("span", { className: "when", textContent: new Date(m.at).toLocaleString() }));
        b.onclick = () => show(m);
        return b;
      }),
    );
  }

  async function show(m) {
    open = m;
    root.classList.add("reading");
    draw();
    // Each name: what can be done with that person.
    const who = ref => nameOf(m, ref, did => ({ href: "#", onclick: e => (e.preventDefault(), person.open(e.currentTarget, did)) }));
    const meta = node("div", { className: "meta" });
    meta.append("From ", who(m.from), " · to ");
    m.to.forEach((d, i) => meta.append(...(i ? [", "] : []), who(d)));
    meta.append(` · ${new Date(m.at).toLocaleString()}`);
    read.replaceChildren(
      node("h2", { textContent: m.subject || "(no subject)" }),
      meta,
      // WHAT WAS WRITTEN, as every item's text reads (`markdown`): its media and embedded items where they were written,
      // its other files below.
      node("div", { className: "body" }),
      attachments.show((m.files ?? []).filter(f => !markdown.inlined(m.body).has(markdown.keyOf(f)))) ?? "",
    );
    read.querySelector(".body").append(markdown.render(m.body ?? "", m.files ?? []));
    if (box === "in") read.append(node("button", { type: "button", className: "reply", textContent: "Reply", onclick: () => compose(m) }));
  }

  // COMPOSE (or reply to `re`): the recipients resolved one by one, then one mail to them all.
  function compose(re = null) {
    const f = dlg.querySelector("form");
    const said = dlg.querySelector(".said");
    f.reset();
    said.hidden = true;
    if (re) {
      f.elements.to.value = re.from;
      f.elements.subject.value = /^re:/i.test(re.subject) ? re.subject : `Re: ${re.subject}`;
    }
    dlg.onclose = null;
    // THE ONE EDITOR (`md-editor`): its media inline, any item inserted, files attached — sent as picked (sealed; the
    // mail's recipients read them).
    const pick = attachments.picker({ from: { app: "mail" }, media: true });
    const ed = mdEditor.create({ pick, placeholder: "Message", label: "Message" });
    f.querySelector(".message").replaceChildren(ed.el);
    f.onsubmit = async e => {
      if (e.submitter?.value !== "ok") return;
      e.preventDefault();
      said.hidden = true;
      try {
        if (ed.busy()) throw new Error("still sending the files: a moment");
        const to = await Promise.all(f.elements.to.value.split(",").map(x => x.trim()).filter(Boolean).map(x => (x.startsWith("space:") ? x : conversation.person(x))));
        await src.send(to, f.elements.subject.value.trim(), ed.value(), re?.id ?? null, ed.files());
        dlg.close();
        if (box === "sent") draw();
        else location.hash = `${base}/sent`;
      } catch (err) {
        said.textContent = `Not sent: ${err?.message ?? err}`;
        said.hidden = false;
      }
    };
    dlg.showModal();
    if (re) ed.focus();
    else f.elements.to.focus();
  }

  list.replaceChildren(theme.loading(box === "sent" ? "Loading sent mail…" : "Loading your mail…"));
  await draw();
  src.onChange(() => el.isConnected && draw());
  if (box === "in") {
    // New mail pointed to in the inbox: opened and kept (the list redraws as it arrives).
    list.append(theme.loading("Checking your inbox…", 1));
    src
      .fetch()
      .catch(e => ctx.log("mail", { what: e?.message ?? String(e) }))
      .finally(() => el.isConnected && draw());
  }
}
