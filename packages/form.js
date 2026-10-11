// FORMS, an app (`#/form`, a space's `#/s/<space>/form`): collecting information — surveys, sign-ups, applications
// (ARCHITECTURE §7; polls are Vote's, never here). UI only, over the shared parts:
// - A FORM is an item of kind `form` (`items`, `kinds`): its title, its description (the body), and in `meta` its
//   FIELDS (`form-fields`: short and long text, number, email, date, single and multiple choice, checkbox, file
//   upload; required flags; choices), `open` (accepting responses; `closedAt` when it closed), `once` (one response
//   per account), `editable` (a response may be changed while open).
//   Who sees it — and so who may answer it — is its AUDIENCE (`audience`: public, followers, friends; a space's
//   members, a role, the admins, chosen people); who may MAKE one in a space is the space's policy (`form` domain,
//   `post`); answering is the form's `comment` action (`roles.mayWrite`: its own rule, else the space's).
// - A RESPONSE is a MAIL (`conversation.mail`, nothing of its own): to the SPACE's own address (`space:<id>`, space
//   mail) where the form is in a space, else to its MAKER (`form-fields.responseTo`; a circle's form — friends,
//   followers — is personal: its maker's). Its subject the form's title, its body structured (`form-fields`:
//   `{ kind: "form-response", form, by, at, answers }`). Who answered is who SENT it (mail's proof: whose tail it was
//   in); `responseOf` checks the body names this form and its sender — a body copied into another's mail is nothing.
// - WHO READS THEM is who reads that mail: a space's owner and admins (space mail: an admin added later reads all its
//   mail, one removed nothing new — as Mail does), a personal form's maker. A space form needs the space's mail ON
//   (its owner turns it on, here or in Mail); until then the builder says so and makes none.
// - The respondent KEEPS what they sent: their SENT mail (no table of Forms' own). Changing a response (where the form
//   allows, one per account, while open) sends a new one; the newest of each account counts.
// - RESULTS (who reads them): the mailbox's responses to this form — a table, per-field summaries (counts for
//   choices), CSV export (`form-fields`).
// Telemetry is the loader's collector's: nothing of its own here.
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  const [items, theme, F, directory, space, conversation, roles] = await Promise.all(["items", "theme", "form-fields", "directory", "space", "conversation", "roles"].map(n => ctx.require(n)));
  const mail = conversation.mail;
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.flat().filter(k => k != null && k !== false));
    return e;
  };
  el.innerHTML = `
    <style>@layer apps {
      .fm { display: grid; gap: var(--cw-space-3); padding: var(--cw-space-3) var(--cw-gutter); max-width: 960px; margin: 0 auto; }
      .fm h2 { margin: 0; }
      .fm .list { display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: var(--cw-space-3); align-items: start; }
      .fm .card { display: grid; gap: 4px; padding: var(--cw-space-3); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); background: var(--cw-surface); color: inherit; text-decoration: none; }
      .fm .card .t { font-weight: 600; }
      .fm .s, .fm .none, .fm .help { color: var(--cw-muted); font-size: var(--cw-text-sm); margin: 0; }
      .fm .said, .fm .err { color: var(--cw-danger); margin: 0; font-size: var(--cw-text-sm); }
      .fm .tools { display: flex; flex-wrap: wrap; gap: var(--cw-space-2); align-items: center; }
      .fm .q { display: grid; gap: 6px; padding: var(--cw-space-3); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); background: var(--cw-surface); }
      .fm .q .label { font-weight: 600; }
      .fm .q .req { color: var(--cw-danger); }
      .fm .q.bad { border-color: var(--cw-danger); }
      .fm .q input[type=text], .fm .q input[type=email], .fm .q input[type=number], .fm .q input[type=date], .fm .q textarea, .fm .q select { width: 100%; box-sizing: border-box; }
      .fm .opt { display: flex; gap: 8px; align-items: center; }
      .fm .ed { display: grid; gap: 6px; padding: var(--cw-space-3); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); }
      .fm .ed .row { display: flex; flex-wrap: wrap; gap: var(--cw-space-2); align-items: center; }
      .fm .ed .row input[type=text] { flex: 1 1 220px; }
      .fm .ed textarea { width: 100%; box-sizing: border-box; min-height: 5em; }
      .fm .meta input[type=text], .fm .meta textarea { width: 100%; box-sizing: border-box; }
      .fm .scroll { overflow-x: auto; }
      .fm table { border-collapse: collapse; font-size: var(--cw-text-sm); }
      .fm th, .fm td { border: 1px solid var(--cw-line); padding: 4px 8px; text-align: left; vertical-align: top; }
      .fm .bar { display: grid; grid-template-columns: minmax(80px, 30%) 1fr auto; gap: 8px; align-items: center; font-size: var(--cw-text-sm); }
      .fm .bar i { display: block; height: 10px; border-radius: 5px; background: var(--cw-accent); min-width: 2px; }
    }</style>
    <div class="fm"></div>`;
  const root = el.querySelector(".fm");
  const me = (await space.account()).id;
  const refIn = s => decodeURIComponent(s.slice(2));
  const spaceIdOf = ref => (String(ref).startsWith("space:") ? ref.slice(6, ref.indexOf("/")) : null);
  const metaOf = form => ({ fields: [], open: true, once: false, editable: true, ...(form?.meta ?? {}) });
  const fail = (said, e) => (said.textContent = e?.message ?? String(e));

  // ONE FORM, read where it is: a space this person is in, or from outside (a public space's, a person's).
  async function getForm(ref) {
    const sid = spaceIdOf(ref);
    const inside = !sid || (await space.mine()).some(s => s.id === sid);
    const outside = inside ? null : await items.outsideOf(ref).catch(() => null);
    const it = await items.get(ref, { outside }).catch(() => null);
    return it && it.kind === "form" ? { ...it, outside } : null;
  }
  // ITS PLACE: the space it is in (this person's, or the public one read from outside), or null (a person's own).
  async function placeOf(form) {
    const sid = spaceIdOf(form.ref);
    if (!sid) return null;
    return form.outside ?? (await space.mine()).find(s => s.id === sid) ?? null;
  }
  // WHO READS ITS RESPONSES — who reads the mail they are: a space's owner and admins (space mail's rule), else the maker.
  async function reads(form, place) {
    if (!F.toSpace(place)) return form.by === me;
    if (form.outside) return false;
    const r = await (await ctx.require("roles")).of(place);
    await r.settled;
    return ["owner", "admin"].includes(r.role(me));
  }
  // The MAILBOX its responses arrive in: the space's (space mail), else this person's own.
  const mailboxOf = async place => (F.toSpace(place) ? mail.of(place) : mail);
  // ITS RESPONSES in a mailbox's `which` ("in": received, "sent": this person's own), each checked (`responseOf`).
  // (`to`: only mail addressed to the form's mailbox counts.)
  const responsesIn = async (box, which, form, to = null) => (await box.list(which)).map(x => F.responseOf(x, form.ref, to)).filter(Boolean);
  // MAY `did` ANSWER this form: the form's `comment` rule, the one check (`roles.mayWrite`: its own rule within its
  // space's, a circle by the credential it cited) — mail is open to anyone, so who may answer is checked where the
  // answers are read (and before sending, for the respondent's sake). A rule not read yet: not counted yet.
  async function mayAnswer(form, place, did, cred = null) {
    const r = F.toSpace(place) ? await roles.ofPublic(place).catch(() => null) : null;
    await r?.settled;
    if (!r) await roles.personal(form.by).ready?.catch?.(() => {});
    if (r?.banned?.(did)) return false;
    return roles.mayWrite({ action: "comment", item: form, writer: did, cred, r }) === true;
  }

  // A RESPONSE sent: a mail to the space (a space's form) or to the maker — a change of answer is a new one (the
  // newest of each account counts).
  async function respond(form, answers) {
    const m = metaOf(form);
    if (!m.open) throw new Error("this form is closed");
    const v = F.validate(m.fields, answers);
    if (!v.ok) return v;
    const place = await placeOf(form);
    if (F.toSpace(place) && !(await mail.accepts(place.id))) throw new Error(`${space.shown(place)}'s mail is off, so its forms take no responses yet: its owner turns it on (Mail)`);
    // WHO MAY ANSWER: the form's rule (its readers who may comment); a circle's credential cited, as a comment does.
    const cred = F.toSpace(place) ? null : await roles.credToCite(form, "comment").catch(() => null);
    // (The respondent's own check is the one comments use — `items.mayWriteOn`: a rule not read yet passes here and is
    // checked where the answers are read; measured on fx: a strict check here refused a public form.)
    if (!(await items.mayWriteOn(form, "comment", { outside: F.toSpace(place) ? place : null }))) throw new Error("this form is not open to you");
    const x = F.responseMail(form, place, me, v.clean, Date.now(), cred);
    const sent = await mail.send(x.to, x.subject, x.body);
    return { ...v, ref: sent.id };
  }
  // This person's own response to a form: the newest in their Sent mail.
  async function myResponse(form) {
    return F.latestPerAccount(await responsesIn(mail, "sent", form))[0] ?? null;
  }

  // ONE QUESTION as an input, and how its answer is read back.
  async function question(f, value) {
    const box = h("div", { className: "q" }, h("div", { className: "label" }, f.label, f.required ? h("span", { className: "req", textContent: " *" }) : null));
    if (f.help) box.append(h("p", { className: "help", textContent: f.help }));
    let read;
    const input = props => h("input", { ...props, required: !!f.required });
    switch (f.type) {
      case "short":
      case "email":
      case "number":
      case "date": {
        const i = input({ type: f.type === "short" ? "text" : f.type, value: value ?? "", ...(f.type === "number" ? { step: "any" } : {}), maxLength: F.LIMITS.short });
        box.append(i);
        read = () => i.value;
        break;
      }
      case "long": {
        const t = h("textarea", { value: value ?? "", rows: 5, maxLength: F.LIMITS.long });
        box.append(t);
        read = () => t.value;
        break;
      }
      case "single": {
        const name = `${f.id}-${Math.random().toString(36).slice(2)}`;
        const rs = f.choices.map(c => h("input", { type: "radio", name, value: c, checked: value === c }));
        box.append(...f.choices.map((c, i) => h("label", { className: "opt" }, rs[i], c)));
        read = () => rs.find(r => r.checked)?.value ?? "";
        break;
      }
      case "multiple": {
        const cs = f.choices.map(c => h("input", { type: "checkbox", value: c, checked: Array.isArray(value) && value.includes(c) }));
        box.append(...f.choices.map((c, i) => h("label", { className: "opt" }, cs[i], c)));
        read = () => cs.filter(c => c.checked).map(c => c.value);
        break;
      }
      case "checkbox": {
        const c = h("input", { type: "checkbox", checked: value === true });
        box.append(h("label", { className: "opt" }, c, "Yes"));
        read = () => c.checked;
        break;
      }
      case "file": {
        // THE ONE FILES PATH (`attachments`: sealed, coded, listed in Drive): uploaded into the RESPONDENT's own space
        // (never the form's: a space's Drive lists its files to every member), its reference — with its key — riding
        // in the sealed answers, so who opens the response reads the file, nobody else.
        const pick = (await ctx.require("attachments")).picker({ space: null, public: false });
        if (Array.isArray(value) && value.length) pick.preset(value);
        box.append(pick.el);
        read = () => pick.files();
        box.busy = () => pick.busy();
        break;
      }
    }
    const err = h("p", { className: "err" });
    box.append(err);
    box.read = read;
    box.say = why => ((err.textContent = why ?? ""), box.classList.toggle("bad", !!why));
    return box;
  }

  // THE FORM's PAGE: answer it (or change an answer, where allowed); its owners' and maker's links.
  async function page(ref, at) {
    root.replaceChildren(theme.loading("Opening the form…"));
    const form = await getForm(ref);
    if (!form) return root.replaceChildren(h("p", { className: "none", textContent: "This form is not there, or not yours to see." }));
    const m = metaOf(form);
    const tools = h("div", { className: "tools" }, h("a", { href: at.base, textContent: "← Forms" }));
    const said = h("p", { className: "said" });
    const place = await placeOf(form);
    if (await reads(form, place).catch(() => false)) tools.append(h("a", { className: "cw-btn", href: at.href(`r/${encodeURIComponent(ref)}`), textContent: "Results" }));
    if (form.by === me) {
      tools.append(h("a", { className: "cw-btn", href: at.href(`e/${encodeURIComponent(ref)}`), textContent: "Edit" }));
      const flip = h("button", { type: "button", className: "cw-btn", textContent: m.open ? "Close responses" : "Reopen" });
      const run = theme.action(flip, () => items.editItem(ref, form.body ?? "", { meta: { ...m, open: !m.open, ...(m.open ? { closedAt: Date.now() } : { closedAt: null }) } }), { busy: "Saving…" });
      flip.onclick = () => run().then(() => page(ref, at), e => fail(said, e));
      tools.append(flip);
    }
    tools.append(h("button", { type: "button", className: "cw-btn", textContent: "Copy link", onclick: e => navigator.clipboard?.writeText(items.linkOf(ref, "form")).then(() => (e.target.textContent = "Copied"), x => fail(said, x)) }));
    const head = h("div", {}, h("h2", { textContent: `🗒 ${form.title || "Untitled form"}` }), h("p", { className: "s" }, m.open ? "Accepting responses" : "Closed", m.once ? " · one response per account" : "", " · by ", directory.nameEl(form.by)));
    if (form.body) head.append(h("p", { textContent: form.body, style: "white-space:pre-wrap" }));
    root.replaceChildren(tools, said, head);
    const mine = await myResponse(form).catch(() => null);
    if (!m.open) return root.append(h("p", { className: "none", textContent: mine ? "This form is closed. Your response is kept (Sent)." : "This form is closed." }));
    // ANSWERED already, one per account: changed where the form allows, else said. (Not one per account: each send a
    // further response.)
    if (mine && m.once && !m.editable) return root.append(h("p", { className: "none", textContent: "You answered this form. It does not allow changing a response." }));
    const had = mine && m.once && m.editable ? mine : null;
    const qs = await Promise.all(m.fields.map(f => question(f, had?.answers?.[f.id])));
    const send = h("button", { type: "submit", className: "cw-btn primary", textContent: had ? "Update response" : "Send" });
    const fsaid = h("p", { className: "said" });
    const fm = h("form", { className: "answer", style: "display:grid;gap:var(--cw-space-3)" }, had ? h("p", { className: "s", textContent: `You answered on ${new Date(had.at).toLocaleString()}; sending again replaces it (the newest counts).` }) : null, ...qs, h("div", { className: "tools" }, send), fsaid);
    const go = theme.action(send, async () => {
      fsaid.textContent = "";
      if (qs.some(q => q.busy?.())) throw new Error("a file is still uploading");
      const answers = Object.fromEntries(m.fields.map((f, i) => [f.id, qs[i].read()]));
      const v = await respond(form, answers);
      m.fields.forEach((f, i) => qs[i].say(v.errors[f.id]));
      if (!v.ok) throw new Error("some answers need a look");
      return v;
    }, { busy: "Sending…", done: "Sent ✓" });
    fm.onsubmit = e => {
      e.preventDefault();
      go().then(() => root.replaceChildren(tools, head, h("p", { textContent: F.toSpace(place) ? `Thank you — your response was sent as mail to ${space.shown(place)}. It is in your Sent mail.` : "Thank you — your response was sent as mail to the form's maker. It is in your Sent mail." })), e => fail(fsaid, e));
    };
    root.append(fm);
  }

  // THE BUILDER: a new form (in this place) or one of this person's own, edited.
  async function builder(at, ref = null) {
    root.replaceChildren(theme.loading("Opening…"));
    const was = ref ? await getForm(ref) : null;
    if (ref && (!was || was.by !== me)) return root.replaceChildren(h("p", { className: "none", textContent: "Only its maker edits a form." }));
    const sp = at.space ?? null;
    if (!was && sp) {
      const r = await (await ctx.require("roles")).of(sp);
      await r.settled;
      if (!r.allowsIn("post", me, "form")) return root.replaceChildren(h("p", { className: "none", textContent: `${space.shown(sp)} does not let you make forms here (its policy).` }));
      // ITS RESPONSES ARE SPACE MAIL: none made while the space's mail is off — its owner turns it on (here, or Mail).
      if (F.toSpace(sp) && !(await mail.accepts(sp.id))) {
        const off = h("p", { className: "none", textContent: `A form's responses here arrive as ${space.shown(sp)}'s mail (its owner and admins read them), and its mail is off.` });
        if (r.role(me) !== "owner") return root.replaceChildren(off, h("p", { className: "none", textContent: "Ask its owner to turn its mail on; then forms can be made here." }));
        const said = h("p", { className: "said" });
        const on = h("button", { type: "button", className: "cw-btn primary", textContent: "Turn on this space's mail" });
        const run = theme.action(on, async () => (await mail.of(sp)).enable(), { busy: "Turning on…" });
        on.onclick = () => run().then(() => builder(at, ref), e => fail(said, e));
        return root.replaceChildren(off, h("div", { className: "tools" }, on), said);
      }
    }
    const m = metaOf(was);
    const place = was ? await placeOf(was) : sp;
    const title = h("input", { type: "text", placeholder: "Form title", maxLength: 300, value: was?.title ?? "" });
    const about = h("textarea", { placeholder: "Description (optional)", rows: 3, value: was?.body ?? "" });
    const open = h("input", { type: "checkbox", checked: m.open !== false });
    const once = h("input", { type: "checkbox", checked: !!m.once });
    const editable = h("input", { type: "checkbox", checked: m.editable !== false });
    const who = was ? null : await (await ctx.require("audience")).picker({ space: sp, kind: "form", initial: sp ? "members" : "public" });
    let fields = m.fields.map(f => ({ ...f }));
    const list = h("div", { style: "display:grid;gap:var(--cw-space-2)" });
    const drawFields = () =>
      list.replaceChildren(
        ...fields.map((f, i) => {
          const type = h("select", {}, ...F.TYPES.map(t => h("option", { value: t.type, textContent: t.label, selected: t.type === f.type })));
          const label = h("input", { type: "text", placeholder: "Question", value: f.label ?? "", maxLength: F.LIMITS.label, oninput: e => (f.label = e.target.value) });
          const req = h("input", { type: "checkbox", checked: !!f.required, onchange: e => (f.required = e.target.checked) });
          const help = h("input", { type: "text", placeholder: "Help text (optional)", value: f.help ?? "", oninput: e => (f.help = e.target.value) });
          const choices = h("textarea", { placeholder: "One option per line", value: (f.choices ?? []).join("\n"), oninput: e => (f.choices = e.target.value.split("\n")) });
          const hasChoices = () => !!F.TYPES.find(t => t.type === f.type)?.choices;
          choices.hidden = !hasChoices();
          type.onchange = () => ((f.type = type.value), (choices.hidden = !hasChoices()));
          const move = d => () => {
            const j = i + d;
            if (j < 0 || j >= fields.length) return;
            [fields[i], fields[j]] = [fields[j], fields[i]];
            drawFields();
          };
          return h(
            "div",
            { className: "ed" },
            h("div", { className: "row" }, label, type, h("label", { className: "opt" }, req, "Required")),
            help,
            choices,
            h("div", { className: "row" }, h("button", { type: "button", className: "cw-btn", textContent: "↑", title: "Up", onclick: move(-1) }), h("button", { type: "button", className: "cw-btn", textContent: "↓", title: "Down", onclick: move(1) }), h("button", { type: "button", className: "cw-btn", textContent: "Remove", onclick: () => ((fields = fields.filter(x => x !== f)), drawFields()) })),
          );
        }),
      );
    drawFields();
    const add = h("button", { type: "button", className: "cw-btn", textContent: "＋ Add a question", onclick: () => (fields.push({ type: "short", label: "", required: false }), drawFields()) });
    const save = h("button", { type: "submit", className: "cw-btn primary", textContent: was ? "Save" : "Create form" });
    const said = h("p", { className: "said" });
    const run = theme.action(
      save,
      async () => {
        said.textContent = "";
        const clean = F.normalize(fields.filter(f => String(f.label ?? "").trim() || f.choices?.length));
        if (!clean.length) throw new Error("a form needs at least one question");
        const meta = { fields: clean, open: open.checked, once: once.checked, editable: editable.checked, closedAt: open.checked ? null : was && m.open === false ? (m.closedAt ?? Date.now()) : Date.now() };
        if (!title.value.trim()) throw new Error("a form needs a title");
        if (was) {
          await items.editItem(ref, about.value, { title: title.value.trim(), meta });
          return ref;
        }
        return items.submit({ board: sp?.id ?? null, kind: "form", title: title.value, body: about.value, meta, audience: who.value(), write: who.write() });
      },
      { busy: "Saving…", done: "Saved ✓" },
    );
    const form = h(
      "form",
      { className: "meta", style: "display:grid;gap:var(--cw-space-3)" },
      title,
      about,
      list,
      h("div", { className: "tools" }, add),
      h("div", { className: "tools" }, h("label", { className: "opt" }, open, "Accepting responses"), h("label", { className: "opt" }, once, "One response per account"), h("label", { className: "opt" }, editable, "Respondents may change their response while open (one per account)")),
      h("p", { className: "help", textContent: F.toSpace(place) ? `Responses arrive as ${space.shown(place)}'s mail: its owner and admins read them (an admin added later reads them all).` : "Responses arrive as mail to you (in Mail too)." }),
      who?.el,
      h("div", { className: "tools" }, save),
      said,
    );
    form.onsubmit = e => {
      e.preventDefault();
      run().then(r => (location.hash = at.href(`p/${encodeURIComponent(r)}`)), err => fail(said, err));
    };
    root.replaceChildren(h("div", { className: "tools" }, h("a", { href: at.base, textContent: "← Forms" })), h("h2", { textContent: was ? "Edit form" : "🗒 New form" }), form);
  }

  // RESULTS (who reads them): the mailbox's responses to this form — fetched first (as Mail does), checked — a table,
  // per-field summaries, CSV.
  async function results(ref, at) {
    root.replaceChildren(theme.loading("Opening the responses…"));
    const form = await getForm(ref);
    if (!form) return root.replaceChildren(h("p", { className: "none", textContent: "This form is not there, or not yours to see." }));
    const place = await placeOf(form);
    if (!(await reads(form, place))) return root.replaceChildren(h("p", { className: "none", textContent: F.toSpace(place) ? `Only ${space.shown(place)}'s owner and admins read its forms' responses (its mail).` : "Only the form's maker reads its responses." }));
    const m = metaOf(form);
    const box = await mailboxOf(place);
    await box.fetch().catch(e => ctx.log("form", { what: `fetching mail: ${e.message}` }));
    // Only mail addressed to this form's mailbox, from someone the form's rule lets answer.
    const addressed = await responsesIn(box, "in", form, F.responseTo(form, place));
    const allowed = await Promise.all(addressed.map(x => mayAnswer(form, place, x.by, x.cred)));
    const opened = addressed.filter((_, i) => allowed[i]);
    if (opened.length < addressed.length) ctx.log("form", { what: `${addressed.length - opened.length} response(s) from people the form is not open to: not counted` });
    const all = m.once ? F.latestPerAccount(opened) : opened;
    // Sent after the form closed: shown apart, never counted.
    const late = m.closedAt ? all.filter(r => (r.edited ?? r.at) > m.closedAt) : [];
    const counted = all.filter(r => !late.includes(r)).sort((a, b) => (a.edited ?? a.at) - (b.edited ?? b.at));
    const handles = new Map(await Promise.all([...new Set(counted.map(r => r.by))].map(async d => [d, await directory.name(d).catch(() => directory.shown(d))])));
    const csv = h("button", {
      type: "button",
      className: "cw-btn",
      textContent: "Export CSV",
      onclick: () => {
        const a = h("a", { href: URL.createObjectURL(new Blob([F.toCSV(m.fields, counted, { name: d => handles.get(d) ?? d })], { type: "text/csv" })), download: `${(form.title || "form").replace(/[^\w.-]+/g, "_")}.csv` });
        a.click();
        setTimeout(() => URL.revokeObjectURL(a.href), 10000);
      },
    });
    const summary = F.summarize(m.fields, counted).map(s => {
      const box = h("div", { className: "q" }, h("div", { className: "label", textContent: s.label }), h("p", { className: "s", textContent: `${s.answered} of ${s.of} answered` }));
      if (s.counts) {
        const max = Math.max(1, ...Object.values(s.counts));
        box.append(...Object.entries(s.counts).map(([c, n]) => h("div", { className: "bar" }, h("span", { textContent: c }), h("i", { style: `width:${(100 * n) / max}%` }), h("span", { textContent: String(n) }))));
      } else if (s.type === "checkbox") box.append(h("p", { className: "s", textContent: `Yes ${s.yes} · No ${s.no}` }));
      else if (s.type === "number" && s.mean != null) box.append(h("p", { className: "s", textContent: `Least ${s.min} · most ${s.max} · mean ${Math.round(s.mean * 100) / 100}` }));
      return box;
    });
    const attachments = m.fields.some(f => f.type === "file") ? await ctx.require("attachments") : null;
    const table = h(
      "table",
      {},
      h("thead", {}, h("tr", {}, h("th", { textContent: "Submitted" }), h("th", { textContent: "Respondent" }), ...m.fields.map(f => h("th", { textContent: f.label })))),
      h(
        "tbody",
        {},
        ...counted.map(r =>
          h(
            "tr",
            {},
            h("td", { textContent: new Date(r.edited ?? r.at).toLocaleString() }),
            h("td", {}, directory.nameEl(r.by)),
            ...m.fields.map(f => {
              const v = r.answers[f.id];
              if (f.type === "file" && Array.isArray(v) && v.length && attachments) return h("td", {}, attachments.show(v));
              return h("td", { textContent: String(F.cellOf(f, v)) });
            }),
          ),
        ),
      ),
    );
    root.replaceChildren(
      h("div", { className: "tools" }, h("a", { href: at.href(`p/${encodeURIComponent(ref)}`), textContent: "← The form" }), csv),
      h("h2", { textContent: `Results: ${form.title || "Untitled form"}` }),
      h("p", { className: "s", textContent: `${counted.length} response(s)${m.once ? " (one per account: each person's newest)" : ""}${late.length ? ` · ${late.length} sent after it closed, not counted` : ""}` }),
      ...summary,
      counted.length ? h("div", { className: "scroll" }, table) : h("p", { className: "none", textContent: "No responses yet." }),
    );
  }

  // SENT: what this person answered — their Sent mail's responses (each mail's body names its form; one sent by them).
  async function sent() {
    const rows = (await mail.list("sent"))
      .map(x => {
        const r = F.responseOf(x);
        return r ? { ...r, title: x.subject.replace(/^🗒 /, "") } : null;
      })
      .filter(Boolean);
    root.replaceChildren(
      h("h2", { textContent: "🗒 Your responses" }),
      rows.length
        ? h("div", { className: "list" }, ...rows.map(x => h("a", { className: "card", href: items.pageOf(x.form, "form") }, h("div", { className: "t", textContent: x.title || "Untitled form" }), h("p", { className: "s", textContent: `Sent ${new Date(x.at).toLocaleString()} · ${Object.keys(x.answers ?? {}).length} answer(s)` }))))
        : h("p", { className: "none", textContent: "You have not answered a form yet." }),
    );
  }

  // A PLACE's forms (`where`): yours (and those for your followers or friends: your circles' places), a space's, a
  // person's, Discover, Saved.
  async function place(at) {
    const list = h("div", { className: "list" }, theme.loading("Forms…"));
    const title = at.saved ? "Saved forms" : at.who === "discover" ? "Public forms" : at.who === "person" ? "Their forms" : at.who === "space" ? `${space.shown(at.space)}'s forms` : "Your forms";
    root.replaceChildren(h("h2", { textContent: `🗒 ${title}` }), list);
    let all = await at.read({ kinds: ["form"] }).catch(() => []);
    if (at.who === "mine" && !at.saved) {
      const circles = (await space.mine()).filter(s => s.circle && s.governance?.owner === me);
      if (circles.length) all = [...all, ...(await items.inPlaces({ spaces: circles }, "form", { withVotes: false }).catch(() => []))];
    }
    const seen = new Set();
    all = all.filter(f => f.kind === "form" && !seen.has(f.ref) && seen.add(f.ref)).sort((a, b) => b.at - a.at);
    const none = at.saved ? "Nothing saved." : at.who === "discover" ? "No public forms yet." : at.who === "mine" || at.who === "space" ? "No forms yet: ＋ New form above." : "No forms.";
    list.replaceChildren(
      ...(all.length
        ? all.map(f => {
            const m = metaOf(f);
            return h("a", { className: "card", href: at.href(`p/${encodeURIComponent(f.ref)}`) }, h("div", { className: "t", textContent: f.title || "Untitled form" }), h("p", { className: "s" }, `${m.fields.length} question(s) · ${m.open ? "open" : "closed"}${!spaceIdOf(f.ref) && f.by === me ? " · you read its responses" : ""}`, at.others ? h("span", {}, " · ", directory.nameEl(f.by)) : null));
          })
        : [h("p", { className: "none", textContent: none })]),
    );
  }

  async function draw() {
    let at;
    try {
      at = await (await ctx.require("where")).of({ kind: "form", app: "form", yours: "Your forms" });
    } catch (e) {
      return root.replaceChildren(h("p", { className: "none", textContent: e.message }));
    }
    const s = at.sub;
    const mayMake = at.who === "mine" || at.who === "space";
    at.tabs(at.who === "mine" || at.who === "discover" ? [{ label: "Sent", href: "#/form/sent", on: s === "sent" }] : [], { yoursOn: !s, ...(mayMake || at.who === "discover" ? { create: { label: "New form", href: at.who === "space" ? at.href("new") : "#/form/new", on: s === "new" } } : {}) });
    try {
      if (s.startsWith("p/")) return await page(refIn(s), at);
      if (s.startsWith("r/")) return await results(refIn(s), at);
      if (s.startsWith("e/")) return await builder(at, refIn(s));
      if (s === "new" && mayMake) return await builder(at);
      if (s === "sent") return await sent();
      return await place(at);
    } catch (e) {
      root.replaceChildren(h("p", { className: "said", textContent: e?.message ?? String(e) }));
    }
  }
  await draw();
  addEventListener("craftworks:route", () => el.isConnected && ctx.route === "/form" && draw());
}
