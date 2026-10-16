// VOTE, an app (`#/vote`, a space's `#/s/<space>/vote`): collective decisions (ARCHITECTURE §7, Vote). UI only, over
// the shared parts — its records and their one tally are `vote-tally`'s, every record an ITEM (`items`, `kinds`):
// - POLLS: one question, its options (single or multiple choice), open or closed, results shown live or after it
//   closes. Yours (who may answer = its audience and its `vote` rule: `audience`, `roles.mayWrite`), or a space's
//   (its members, as the space's `vote` policy for the `vote` domain allows).
// - PROPOSALS (a space's): Yes / No / Abstain; each member votes directly or DELEGATES (to another member, global per
//   space in v1, transitive — `vote-tally.resolve`); a direct vote overrides one's delegation. FINALIZED by the
//   proposer or an admin: the result and a snapshot of what it counted; later records change nothing. An OUTCOME (a
//   role granted or assigned, a policy set) is applied, once passed, as the space's own act by a member who may do that
//   act — by the first such member whose page opens it (or presses Apply); nobody with that right online: it waits,
//   shown "passed — waiting to be applied", until one opens Vote (no delegate does it with no page open).
// - THE DELEGATION screen (your delegate in this space: change, revoke) and the DELEGATION GRAPH (who delegates to whom,
//   ending at each direct vote; a proposal's, with its choices).
// PERSONAL view (`#/vote`): your polls, and the open proposals of every space you are in that uses Vote, with your part
// in each. Telemetry is the loader's collector's.
export async function mount(ctx, el) {
  const [items, theme, V, directory, space, roles] = await Promise.all(["items", "theme", "vote-tally", "directory", "space", "roles"].map(n => ctx.require(n)));
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.flat().filter(k => k != null && k !== false));
    return e;
  };
  el.innerHTML = `
    <style>@layer apps {
      .vt { display: grid; gap: var(--cw-space-3); padding: var(--cw-space-3) var(--cw-gutter); }
      .vt h2, .vt h3 { margin: 0; }
      .vt .list { display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: var(--cw-space-3); align-items: start; }
      .vt .card { display: grid; gap: 4px; padding: var(--cw-space-3); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); background: var(--cw-surface); color: inherit; text-decoration: none; }
      .vt .card .t { font-weight: 600; }
      .vt .s, .vt .none, .vt .help { color: var(--cw-muted); font-size: var(--cw-text-sm); margin: 0; }
      .vt .said { color: var(--cw-danger); margin: 0; font-size: var(--cw-text-sm); }
      .vt .you { padding: var(--cw-space-2) var(--cw-space-3); border-radius: var(--cw-radius); background: var(--cw-selected); color: var(--cw-on-selected); margin: 0; }
      .vt .tools { display: flex; flex-wrap: wrap; gap: var(--cw-space-2); align-items: center; }
      .vt .opt { display: flex; gap: 8px; align-items: center; }
      .vt .box { display: grid; gap: 6px; padding: var(--cw-space-3); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); background: var(--cw-surface); }
      .vt .bar { display: grid; grid-template-columns: minmax(80px, 30%) 1fr auto; gap: 8px; align-items: center; font-size: var(--cw-text-sm); }
      .vt .bar i { display: block; height: 10px; border-radius: 5px; background: var(--cw-accent); min-width: 2px; }
      .vt form { display: grid; gap: var(--cw-space-2); }
      .vt input[type=text], .vt textarea, .vt select { width: 100%; box-sizing: border-box; }
      .vt .graph { overflow-x: auto; border: 1px solid var(--cw-line); border-radius: var(--cw-radius); background: var(--cw-surface); }
      .vt .graph svg text { font: 12px var(--cw-font); fill: var(--cw-fg); }
      .vt .graph .n rect { fill: var(--cw-bg); stroke: var(--cw-line); }
      .vt .graph .n.dead rect { stroke-dasharray: 4 3; }
      .vt .graph .c rect { fill: var(--cw-accent); } .vt .graph .c text { fill: var(--cw-accent-fg); font-weight: 600; }
      .vt .graph path { stroke: var(--cw-muted); fill: none; }
    }</style>
    <div class="vt"></div>`;
  const outer = el.querySelector(".vt");
  // Where a page draws: the view's box being built (`ctx.view` render), or the page itself for a maker.
  let root = outer;
  const me = await space.me();
  const refIn = s => decodeURIComponent(s.slice(2));
  const fail = (said, e) => (said.textContent = ctx.say(e));
  const WORD = { yes: "Yes", no: "No", abstain: "Abstain" };
  // NAMES, read once per draw (the graph and the sentences are text, not elements).
  const names = new Map();
  const learnNames = dids => Promise.all([...new Set(dids)].filter(d => !names.has(d)).map(async d => names.set(d, await ctx.optional("vote: directory.name", directory.name(d), () => directory.shown(d)))));
  const nameOf = d => (d === me ? "you" : (names.get(d) ?? directory.shown(d)));
  const bars = (counts, of) => {
    const max = Math.max(1, ...Object.values(counts));
    return Object.entries(counts).map(([c, k]) => h("div", { className: "bar" }, h("span", { textContent: c }), h("i", { style: `width:${(100 * k) / max}%` }), h("span", { textContent: `${k}${of ? ` (${Math.round((100 * k) / Math.max(1, of))}%)` : ""}` })));
  };

  // ONE ITEM read where it is: a space this person is in, or from outside (a person's).
  async function getItem(ref) {
    const it = await items.get(ref);
    return it && ["poll", "proposal"].includes(it.kind) ? it : null;
  }

  // ── POLLS ─────────────────────────────────────────────────────────────────────────────────────────────────────────
  async function pollPage(poll, at) {
    const v = await V.pollView(poll);
    const said = h("p", { className: "said" });
    const m = v.meta;
    const tools = h("div", { className: "tools" }, h("a", { href: at.base, textContent: "← Vote" }));
    if (poll.by === me) {
      const flip = h("button", { type: "button", className: "cw-btn", textContent: m.open ? "Close the poll" : "Reopen" });
      const run = theme.action(flip, () => V.setOpen(poll, !m.open), { busy: "Saving…" });
      flip.onclick = () => run().then(() => draw(), e => fail(said, e));
      tools.append(flip);
    }
    tools.append(h("button", { type: "button", className: "cw-btn", textContent: "Copy link", onclick: e => navigator.clipboard?.writeText(items.linkOf(poll.ref, "poll")).then(() => (e.target.textContent = "Copied"), x => fail(said, x)) }));
    const head = h("div", {}, h("h2", { textContent: `📊 ${poll.title}` }), h("p", { className: "s" }, m.open ? "Open" : `Closed${m.closedAt ? ` ${new Date(m.closedAt).toLocaleString()}` : ""}`, ` · ${m.multiple ? "choose any" : "choose one"} · results ${m.results === "after" ? "after it closes" : "live"} · by `, directory.nameEl(poll.by)));
    root.replaceChildren(tools, said, head);
    root.append(h("p", { className: "you", textContent: v.mine?.length ? `You voted: ${v.mine.join(", ")}${m.open ? " (you may change it while the poll is open; your newest vote counts)" : ""}.` : m.open ? (v.may ? "You have not voted." : "This poll is not open to you.") : "You did not vote." }));
    if (m.open && v.may) {
      const name = `o-${Math.random().toString(36).slice(2)}`;
      const ins = m.options.map(o => h("input", { type: m.multiple ? "checkbox" : "radio", name, value: o, checked: !!v.mine?.includes(o) }));
      const send = h("button", { type: "submit", className: "cw-btn primary", textContent: v.mine?.length ? "Change my vote" : "Vote" });
      const clear = v.mine?.length ? h("button", { type: "button", className: "cw-btn", textContent: "Withdraw my vote" }) : null;
      const fsaid = h("p", { className: "said" });
      const go = theme.action(send, () => V.vote(poll, ins.filter(i => i.checked).map(i => i.value)), { busy: "Voting…", done: "Voted ✓" });
      const f = h("form", { className: "box" }, ...m.options.map((o, i) => h("label", { className: "opt" }, ins[i], o)), h("div", { className: "tools" }, send, clear), fsaid);
      f.onsubmit = e => (e.preventDefault(), go().then(() => draw(), x => fail(fsaid, x)));
      if (clear) clear.onclick = () => V.vote(poll, []).then(() => draw(), x => fail(fsaid, x));
      root.append(f);
    }
    // RESULTS: live, or once closed (a display rule — every record is readable by the poll's readers: transparent v1).
    const t = v.tally;
    if (m.results === "after" && m.open) root.append(h("p", { className: "none", textContent: "Results are shown when the poll closes." }));
    else
      root.append(
        h("div", { className: "box" }, h("h3", { textContent: "Results" }), ...bars(t.counts, t.voters), h("p", { className: "s", textContent: `${t.voters} voter(s), one vote each — each person's newest${t.late ? ` · ${t.late} vote(s) after it closed, not counted` : ""}${t.ineligible ? ` · ${t.ineligible} from people it is not open to, not counted` : ""}${t.pending ? ` · ${t.pending} still being checked` : ""}${t.invalid ? ` · ${t.invalid} not valid choices` : ""}` })),
      );
  }
  async function pollMaker(at) {
    const sp = at.space ?? null;
    if (sp) {
      const r = await roles.of(sp);
      await r.settled;
      if (!(await r.allowsIn("post", me, "vote"))) return root.replaceChildren(h("p", { className: "none", textContent: `${space.shown(sp)} does not let you make polls here (its policy).` }));
    }
    const q = h("input", { type: "text", placeholder: "The question", maxLength: 300 });
    const opts = h("textarea", { rows: 5, placeholder: "One option per line (two or more)" });
    const multiple = h("input", { type: "checkbox" });
    const results = h("select", {}, h("option", { value: "live", textContent: "Results shown live" }), h("option", { value: "after", textContent: "Results shown after it closes" }));
    const who = await (await ctx.require("audience")).picker({ space: sp, kind: "poll", initial: sp ? "members" : "public" });
    const save = h("button", { type: "submit", className: "cw-btn primary", textContent: "Create poll" });
    const said = h("p", { className: "said" });
    const run = theme.action(save, () => V.createPoll({ board: sp?.id ?? null, question: q.value, options: opts.value.split("\n"), multiple: multiple.checked, results: results.value, audience: who.value(), write: who.write() }), { busy: "Saving…", done: "Saved ✓" });
    const f = h("form", {}, q, opts, h("label", { className: "opt" }, multiple, "Several options may be chosen"), results, h("p", { className: "help", textContent: "Who may vote: who sees it and may interact (below). One vote per account; the newest counts." }), who.el, h("div", { className: "tools" }, save), said);
    f.onsubmit = e => (e.preventDefault(), run().then(ref => (location.hash = at.href(`p/${encodeURIComponent(ref)}`)), x => fail(said, x)));
    root.replaceChildren(h("div", { className: "tools" }, h("a", { href: at.base, textContent: "← Vote" })), h("h2", { textContent: "📊 New poll" }), f);
  }

  // ── PROPOSALS ─────────────────────────────────────────────────────────────────────────────────────────────────────
  const spaceOfRef = async ref => (await V.spaceOf(ref)) ?? null;
  async function proposalPage(p, at) {
    const sp = await spaceOfRef(p.ref);
    if (!sp) return root.replaceChildren(h("p", { className: "none", textContent: "A proposal is a space's: you are not in this one." }));
    const S = await V.inSpace(sp);
    let v = await S.proposalView(p);
    await learnNames([...S.members, p.by, ...(v.final ? [v.final.by] : []), ...(v.outcome?.did ? [v.outcome.did] : [])]);
    // THE OUTCOME applied by the first page of one who may (`mayApply`) — once finalized PASSED and checked.
    if (v.outcome && v.final?.final.result.passed && !v.applied && v.mayApply && !["forged", "mismatch"].includes(v.verified?.status)) {
      await S.applyOutcome(p).catch(e => ctx.failed("vote", e, `applying ${p.ref}'s outcome: ${e.message}`));
      v = await (await V.inSpace(sp)).proposalView(p);
    }
    const said = h("p", { className: "said" });
    const res = v.result;
    const status = v.final ? `Finalized — ${res.passed ? "PASSED" : "NOT PASSED"}` : "Open";
    root.replaceChildren(
      h("div", { className: "tools" }, h("a", { href: at.base, textContent: "← Vote" }), h("a", { className: "cw-btn", href: at.href(`g/${encodeURIComponent(p.ref)}`), textContent: "Delegation graph" }), h("a", { className: "cw-btn", href: at.href("delegation"), textContent: "Your delegation" })),
      said,
      h("h2", { textContent: `🗳 ${p.title}` }),
      h("p", { className: "s" }, `${status} · by `, directory.nameEl(p.by), ` · ${v.electorate.length} member(s) may vote · quorum ${res.quorum}`),
      p.body ? h("p", { textContent: p.body, style: "white-space:pre-wrap" }) : null,
      h("p", { className: "s", textContent: `If it passes: ${V.outcomeText(v.outcome, nameOf)}.` }),
      h("p", { className: "you", textContent: v.tally.resolution.has(me) ? V.participation(me, v.tally.resolution, nameOf) : "You are not in this proposal's electorate." }),
    );
    if (v.mayVote) {
      const row = h("div", { className: "tools" });
      for (const c of [...V.CHOICES, null]) {
        if (c === null && !v.mine) continue;
        const b = h("button", { type: "button", className: `cw-btn${v.mine === c ? " primary" : ""}`, textContent: c ? WORD[c] : "Withdraw my direct vote (back to my delegate)" });
        const run = theme.action(b, () => V.vote(p, c), { busy: "Voting…" });
        b.onclick = () => run().then(() => draw(), e => fail(said, e));
        row.append(b);
      }
      root.append(h("div", { className: "box" }, h("h3", { textContent: v.mine ? "Change your vote" : "Vote directly" }), h("p", { className: "help", textContent: "A direct vote overrides your delegation on this proposal; your newest vote counts until it is finalized." }), row));
    }
    const counts = { [WORD.yes]: res.yes, [WORD.no]: res.no, [WORD.abstain]: res.abstain };
    const box = h("div", { className: "box" }, h("h3", { textContent: v.final ? "Finalized result" : "Tally now" }), ...bars(counts, res.electorate));
    if (v.final) {
      const st = { verified: "✓ checked here: the same result from the records it names", partial: `not all its records are here yet (${v.verified?.missing} missing) — shown as recorded`, mismatch: "✗ DOES NOT CHECK: its records give another result", forged: "✗ DOES NOT CHECK: its hash does not match what it says" }[v.verified?.status] ?? "";
      box.append(h("p", { className: "s" }, "Finalized by ", directory.nameEl(v.final.by), ` on ${new Date(v.final.at).toLocaleString()} over ${v.final.final.inputs.length} record(s) of ${v.final.final.electorate.length} member(s) · snapshot ${v.final.final.hash.slice(0, 16)}… · ${st}`));
      const liveRes = v.tally.result;
      if (liveRes.yes !== res.yes || liveRes.no !== res.no || liveRes.abstain !== res.abstain) box.append(h("p", { className: "s", textContent: `Written since (not counted): today it would be ${liveRes.yes} yes · ${liveRes.no} no · ${liveRes.abstain} abstain.` }));
    } else box.append(h("p", { className: "s", textContent: `${res.yes + res.no + res.abstain} of ${res.electorate} count — directly or through their delegates; passes with more yes than no and at least ${res.quorum} vote(s).` }));
    root.append(box);
    if (v.mayFinalize) {
      const fin = h("button", { type: "button", className: "cw-btn primary", textContent: "Finalize now" });
      const run = theme.action(fin, () => S.finalize(p), { busy: "Finalizing…" });
      fin.onclick = async () => (await theme.ask(fin, { text: "Finalize: the result is fixed as it is now; later votes and delegations change nothing.", ok: "Finalize" })) && run().then(() => draw(), e => fail(said, e));
      root.append(h("div", { className: "tools" }, fin, h("span", { className: "s", textContent: "Its proposer or an admin finalizes." })));
    }
    if (v.outcome) {
      const o = h("div", { className: "box" }, h("h3", { textContent: "Outcome" }), h("p", { textContent: V.outcomeText(v.outcome, nameOf) }));
      if (v.applied) o.append(h("p", { className: "s" }, "✓ Applied by ", directory.nameEl(v.applied.by), ` (the space's act, ${new Date(v.applied.at).toLocaleString()}).`));
      else if (!v.final) o.append(h("p", { className: "s", textContent: "Applied once it is finalized as passed." }));
      else if (!res.passed) o.append(h("p", { className: "s", textContent: "Not applied: it did not pass." }));
      else if (v.mayApply) {
        const b = h("button", { type: "button", className: "cw-btn primary", textContent: "Apply it" });
        const run = theme.action(b, () => S.applyOutcome(p), { busy: "Applying…" });
        b.onclick = () => run().then(() => draw(), e => fail(said, e));
        o.append(b);
      } else o.append(h("p", { className: "s", textContent: "Passed — waiting to be applied: a member who may do this act applies it when they open Vote (nobody with that right has yet)." }));
      root.append(o);
    }
  }
  async function proposalMaker(at) {
    const sp = at.space;
    const S = await V.inSpace(sp);
    if (!S.mayPropose) return root.replaceChildren(h("p", { className: "none", textContent: `${space.shown(sp)} does not let you make proposals here (its policy).` }));
    await learnNames(S.members);
    const title = h("input", { type: "text", placeholder: "What is proposed", maxLength: 300 });
    const body = h("textarea", { rows: 5, placeholder: "Why (optional)" });
    const quorum = h("input", { type: "number", min: 1, value: 1, style: "width:6em" });
    const memberSel = () => h("select", {}, ...S.members.map(d => h("option", { value: d, textContent: nameOf(d) })));
    const kind = h("select", {}, h("option", { value: "", textContent: "No outcome (a decision only)" }), h("option", { value: "grant", textContent: "Make a member an admin, or a member again" }), h("option", { value: "assign", textContent: "Give or take back one of the space's roles" }), h("option", { value: "policy", textContent: "Set who may do something (a policy)" }));
    const parts = h("div", { style: "display:grid;gap:6px" });
    let read = () => null;
    kind.onchange = () => {
      const k = kind.value;
      if (!k) return (parts.replaceChildren(), (read = () => null));
      if (k === "grant") {
        const who = memberSel();
        const role = h("select", {}, h("option", { value: "admin", textContent: "an admin" }), h("option", { value: "member", textContent: "a member (not admin)" }));
        parts.replaceChildren(h("label", {}, "Member ", who), h("label", {}, "Becomes ", role), h("p", { className: "help", textContent: "Applied by the space's owner (only the owner grants admin)." }));
        read = () => ({ act: "grant", did: who.value, role: role.value });
      } else if (k === "assign") {
        const who = memberSel();
        const rs = S.roles;
        const role = h("select", {}, ...rs.map(x => h("option", { value: x.id, textContent: x.name })));
        const on = h("select", {}, h("option", { value: "1", textContent: "give it" }), h("option", { value: "", textContent: "take it back" }));
        parts.replaceChildren(rs.length ? h("div", {}, h("label", {}, "Member ", who), h("label", {}, "Role ", role), h("label", {}, on)) : h("p", { className: "none", textContent: "This space has no roles of its own yet (Settings → Roles)." }));
        read = () => (rs.length ? { act: "assign", did: who.value, role: role.value, on: !!on.value } : null);
      } else {
        const path = h("select", {}, h("option", { value: "", textContent: "the whole space" }), ...S.domains.map(d => h("option", { value: d, textContent: d })));
        const action = h("select", {}, ...["post", "comment", "vote", "edit", "read", "join", "invite"].map(a => h("option", { value: a, textContent: a })));
        const who = h("select", {}, ...["members", "admins", "owner", "anyone", "nobody", ...S.roles.map(x => `role:${x.id}`)].map(w => h("option", { value: w, textContent: w })));
        parts.replaceChildren(h("label", {}, "Where ", path), h("label", {}, "Who may ", action), h("label", {}, "Becomes ", who), h("p", { className: "help", textContent: "Applied by a member who may change the space's apps and policies (who may read: the owner)." }));
        read = () => ({ act: "policy", path: path.value, action: action.value, who: who.value });
      }
    };
    const save = h("button", { type: "submit", className: "cw-btn primary", textContent: "Propose" });
    const said = h("p", { className: "said" });
    const run = theme.action(save, () => S.propose({ title: title.value, body: body.value, quorum: quorum.value, action: read() }), { busy: "Saving…", done: "Saved ✓" });
    const f = h("form", {}, title, body, h("label", { className: "opt" }, "Votes needed (quorum) ", quorum), h("label", {}, "If it passes ", kind), parts, h("p", { className: "help", textContent: `Its electorate: ${space.shown(sp)}'s members whom its vote rule lets vote — directly, or through the member they delegate to.` }), h("div", { className: "tools" }, save), said);
    f.onsubmit = e => (e.preventDefault(), run().then(ref => (location.hash = at.href(`p/${encodeURIComponent(ref)}`)), x => fail(said, x)));
    root.replaceChildren(h("div", { className: "tools" }, h("a", { href: at.base, textContent: "← Vote" })), h("h2", { textContent: "🗳 New proposal" }), f);
  }

  // ── DELEGATION ────────────────────────────────────────────────────────────────────────────────────────────────────
  async function delegationPage(at) {
    const S = await V.inSpace(at.space);
    await learnNames(S.members);
    const cur = S.delegation;
    const dels = S.delegations;
    const toMe = [...dels].filter(([, to]) => to === me).map(([by]) => by);
    const sel = h("select", {}, ...S.members.filter(d => d !== me).map(d => h("option", { value: d, textContent: nameOf(d), selected: d === cur })));
    const said = h("p", { className: "said" });
    const save = h("button", { type: "button", className: "cw-btn primary", textContent: cur ? "Change my delegate" : "Delegate" });
    const run = theme.action(save, () => S.delegate(sel.value), { busy: "Saving…" });
    save.onclick = () => run().then(() => draw(), e => fail(said, e));
    const revoke = cur ? h("button", { type: "button", className: "cw-btn", textContent: "Revoke my delegation" }) : null;
    if (revoke) revoke.onclick = () => S.delegate(null).then(() => draw(), e => fail(said, e));
    root.replaceChildren(
      h("div", { className: "tools" }, h("a", { href: at.base, textContent: "← Vote" }), h("a", { className: "cw-btn", href: at.href("graph"), textContent: "Delegation graph" })),
      h("h2", { textContent: `🤝 Your delegation in ${space.shown(at.space)}` }),
      h("p", { className: "you", textContent: cur ? `Your vote is delegated to ${nameOf(cur)} on every proposal here where you do not vote yourself.` : "You do not delegate: only your own votes count." }),
      h("div", { className: "box" }, h("label", {}, "Delegate to ", sel), h("div", { className: "tools" }, save, revoke), said, h("p", { className: "help", textContent: `Transitive: if they delegate too, your vote follows (up to ${V.DEPTH} steps) to the first who votes; a circle nobody on it votes in counts for nothing. Your direct vote on a proposal always overrides this. Every member sees who delegates to whom.` })),
      h("div", { className: "box" }, h("h3", { textContent: "Who delegates to you" }), toMe.length ? h("p", {}, toMe.map(nameOf).join(", ")) : h("p", { className: "none", textContent: "Nobody yet." })),
    );
  }
  // THE GRAPH: arrows from delegators to their delegates, ending at each direct vote (a proposal's), or at whoever the
  // chains end at (the space's delegations alone). Plain SVG: columns by steps to the vote.
  async function graphPage(at, ref) {
    const S = await V.inSpace(at.space);
    const p = ref ? await getItem(ref) : null;
    const v = p ? await S.proposalView(p) : null;
    const dels = v ? v.tally.delegations : S.delegations;
    const resolution = v ? v.tally.resolution : V.delegationChains(S.members, dels);
    const g = V.graphOf(resolution, dels);
    await learnNames(g.nodes.map(n => n.did));
    const W = 150, H = 26, GX = 70, GY = 14, PAD = 12;
    const cols = g.cols + (v ? 1 : 0);
    const dead = g.nodes.filter(n => n.col < 0);
    const live = g.nodes.filter(n => n.col >= 0);
    const rowsOf = c => live.filter(n => n.col === c).length;
    const maxRows = Math.max(1, ...[...Array(g.cols)].map((_, c) => rowsOf(c)), v ? 3 : 0);
    const x = col => PAD + (g.cols - 1 - col) * (W + GX);
    const y = row => PAD + row * (H + GY);
    const pos = new Map();
    for (const n of live) pos.set(n.did, { x: x(n.col), y: y(n.row) });
    const deadTop = y(maxRows) + 24;
    dead.forEach((n, i) => pos.set(n.did, { x: PAD + (i % Math.max(1, cols)) * (W + GX), y: deadTop + Math.floor(i / Math.max(1, cols)) * (H + GY) }));
    const choiceX = PAD + g.cols * (W + GX);
    const choiceY = c => y(V.CHOICES.indexOf(c));
    const width = PAD * 2 + cols * (W + GX);
    const height = (dead.length ? deadTop + Math.ceil(dead.length / Math.max(1, cols)) * (H + GY) + 20 : y(maxRows)) + PAD;
    const NS = "http://www.w3.org/2000/svg";
    const s = (tag, attrs = {}, ...kids) => {
      const e = document.createElementNS(NS, tag);
      for (const [k, val] of Object.entries(attrs)) e.setAttribute(k, val);
      e.append(...kids.filter(Boolean));
      return e;
    };
    const svg = s("svg", { width, height, viewBox: `0 0 ${width} ${height}`, role: "img", "aria-label": "Delegation graph" });
    svg.append(s("defs", {}, s("marker", { id: "vt-arrow", viewBox: "0 0 10 10", refX: 10, refY: 5, markerWidth: 7, markerHeight: 7, orient: "auto-start-reverse" }, s("path", { d: "M0,0 L10,5 L0,10 z", style: "fill:var(--cw-muted)" }))));
    const arrow = (a, b) => svg.append(s("path", { d: `M${a.x + W},${a.y + H / 2} C${a.x + W + GX / 2},${a.y + H / 2} ${b.x - GX / 2},${b.y + H / 2} ${b.x},${b.y + H / 2}`, "marker-end": "url(#vt-arrow)" }));
    for (const e of g.edges) if (pos.has(e.from) && pos.has(e.to)) arrow(pos.get(e.from), pos.get(e.to));
    if (v) {
      for (const n of live.filter(n => n.how === "direct")) arrow(pos.get(n.did), { x: choiceX, y: choiceY(n.choice) });
      V.CHOICES.forEach(c => svg.append(s("g", { class: "c" }, s("rect", { x: choiceX, y: choiceY(c), width: W, height: H, rx: 6 }), s("text", { x: choiceX + 10, y: choiceY(c) + 17 }, `${WORD[c]} (${v.tally.result[c]})`))));
    }
    for (const n of g.nodes) {
      const at_ = pos.get(n.did);
      const label = `${nameOf(n.did)}${n.how === "cycle" ? " ⟲" : n.how === "depth" ? " (too deep)" : n.how === "stuck" ? " (no vote)" : n.how === "outside" ? " (not voting)" : ""}`;
      svg.append(s("g", { class: `n${n.col < 0 ? " dead" : ""}` }, s("title", {}, `${nameOf(n.did)}: ${n.how}${n.choice ? ` — ${WORD[n.choice]}` : ""}`), s("rect", { x: at_.x, y: at_.y, width: W, height: H, rx: 6 }), s("text", { x: at_.x + 8, y: at_.y + 17 }, label.length > 22 ? `${label.slice(0, 21)}…` : label)));
    }
    if (dead.length) svg.append(s("text", { x: PAD, y: deadTop - 8 }, "Counting for nothing (a circle, a chain that ends without a vote, or too long):"));
    root.replaceChildren(
      h("div", { className: "tools" }, h("a", { href: p ? at.href(`p/${encodeURIComponent(p.ref)}`) : at.base, textContent: p ? "← The proposal" : "← Vote" }), h("a", { className: "cw-btn", href: at.href("delegation"), textContent: "Your delegation" })),
      h("h2", { textContent: p ? `Delegation graph: ${p.title}` : `Delegation graph: ${space.shown(at.space)}` }),
      h("p", { className: "s", textContent: p ? "Each arrow: a member's vote carried to their delegate, ending at a direct vote (right). A direct vote overrides the voter's own delegation." : "Each arrow: a member's delegation. Who it reaches on a proposal depends on who votes there: open a proposal for its graph." }),
      g.nodes.length ? h("div", { className: "graph" }, svg) : h("p", { className: "none", textContent: "Nobody has voted or delegated yet." }),
    );
  }

  // ── LISTS ─────────────────────────────────────────────────────────────────────────────────────────────────────────
  const pollCard = (p, at) => {
    const m = V.normalizePoll(p.meta);
    return h("a", { className: "card", href: at.href(`p/${encodeURIComponent(p.ref)}`) }, h("div", { className: "t", textContent: `📊 ${p.title}` }), h("p", { className: "s" }, `${m.options.length} options · ${m.open ? "open" : "closed"}`, at.others ? h("span", {}, " · ", directory.nameEl(p.by)) : null));
  };
  async function proposalCards(S, at, { openOnly = false } = {}) {
    const out = [];
    for (const p of S.proposals) {
      const v = await S.proposalView(p);
      if (openOnly && v.final) continue;
      const r = v.result;
      out.push(h("a", { className: "card", href: `#/s/${S.sp.id}/vote/p/${encodeURIComponent(p.ref)}` }, h("div", { className: "t", textContent: `🗳 ${p.title}` }), h("p", { className: "s", textContent: `${v.final ? `Finalized — ${r.passed ? "passed" : "not passed"}` : "Open"} · ${r.yes} yes · ${r.no} no · ${r.abstain} abstain${at.who === "mine" ? ` · ${space.shown(S.sp)}` : ""}` }), h("p", { className: "s", textContent: v.tally.resolution.has(me) ? V.participation(me, v.tally.resolution, nameOf) : "Not in its electorate." })));
    }
    return out;
  }
  async function list(at) {
    const body = h("div", { style: "display:grid;gap:var(--cw-space-3)" }, theme.loading("Votes…"));
    root.replaceChildren(body);
    if (at.who === "space") {
      const S = await V.inSpace(at.space);
      await learnNames([...S.members, ...S.proposals.map(p => p.by)]);
      const cur = S.delegation;
      const props = await proposalCards(S, at);
      const polls = S.polls;
      body.replaceChildren(
        h("h2", { textContent: `🗳 ${space.shown(at.space)}'s votes` }),
        // (Read from outside — not a member, a guest: nobody's vote here to delegate.)
        at.outside ? null : h("p", { className: "you" }, cur ? `Your vote is delegated to ${nameOf(cur)} where you do not vote yourself. ` : "You do not delegate. ", h("a", { href: at.href("delegation"), textContent: "Change" })),
        h("h3", { textContent: "Proposals" }),
        props.length ? h("div", { className: "list" }, ...props) : h("p", { className: "none", textContent: "No proposals yet." }),
        h("h3", { textContent: "Polls" }),
        polls.length ? h("div", { className: "list" }, ...polls.map(p => pollCard(p, at))) : h("p", { className: "none", textContent: "No polls yet." }),
      );
      return;
    }
    // YOURS (and those for your followers or friends: your circles' places), or a PERSON's.
    let polls = await at.read({ kinds: ["poll"] });
    if (at.who === "mine" && !at.saved) {
      const circles = (await space.mine()).filter(s => s.circle && space.madeBy(s, me));
      if (circles.length) polls = [...polls, ...(await items.inPlaces({ spaces: circles }, "poll", { withVotes: false }))];
    }
    const seen = new Set();
    polls = polls.filter(p => p.kind === "poll" && !seen.has(p.ref) && seen.add(p.ref)).sort((a, b) => b.at - a.at);
    const parts = [h("h2", { textContent: at.saved ? "Saved polls" : at.discover ? "🗳 Public polls" : at.who === "person" ? "🗳 Their polls" : "🗳 Your polls" }), polls.length ? h("div", { className: "list" }, ...polls.map(p => pollCard(p, at))) : h("p", { className: "none", textContent: at.who === "mine" ? "No polls yet: ＋ New poll above." : "No polls." })];
    // The PERSONAL view gathers: every space you are in that uses Vote — its open proposals, your part in each.
    if (at.who === "mine" && !at.saved) {
      const withVote = [];
      for (const sp of (await space.mine()).filter(s => !s.circle)) {
        const r = await roles.of(sp);
        if (r && (await ctx.optional("vote: r.settled.then", Promise.resolve(r.settled).then(() => true), () => false)) && (await r.apps()).includes("vote")) withVote.push(sp);
      }
      const cards = [];
      for (const sp of withVote) {
        const S = await ctx.optional("vote: V.inSpace", V.inSpace(sp), () => null);
        if (!S) continue;
        await learnNames([...S.delegations.values()].filter(Boolean));
        cards.push(...(await proposalCards(S, at, { openOnly: true })));
      }
      parts.push(h("h2", { textContent: "🗳 Open proposals in your spaces" }), cards.length ? h("div", { className: "list" }, ...cards) : h("p", { className: "none", textContent: withVote.length ? "None open." : "None of your spaces uses Vote yet (a space adds it on its Home)." }));
    }
    body.replaceChildren(...parts);
  }

  async function draw() {
    let at;
    try {
      at = await (await ctx.require("where")).of({ kind: "poll", kinds: ["poll", "proposal"], app: "vote", yours: "Your votes" });
    } catch (e) {
      return outer.replaceChildren(h("p", { className: "none", textContent: ctx.say(e) }));
    }
    const s = at.sub;
    // A space this person is IN (read from outside: its public polls and proposals, nothing of a member's to do).
    const inSp = at.who === "space" && !at.outside;
    at.tabs(inSp ? [{ label: "Delegation", href: at.href("delegation"), on: s === "delegation" }, { label: "Graph", href: at.href("graph"), on: s === "graph" }, { label: "＋ New proposal", href: at.href("new-proposal"), on: s === "new-proposal" }] : [], { yoursOn: !s, ...(at.who === "mine" || inSp ? { create: { label: "New poll", href: inSp ? at.href("new-poll") : "#/vote/new-poll", on: s === "new-poll" } } : {}) });
    view?.stop();
    view = null;
    // A MAKER (a poll, a proposal): drawn once — never again under its writer.
    if (inSp && s === "new-proposal") return void ((root = outer), await proposalMaker(at).catch(e => outer.replaceChildren(h("p", { className: "said", textContent: ctx.say(e) }))));
    if (s === "new-poll" && (at.who === "mine" || inSp)) return void ((root = outer), await pollMaker(at).catch(e => outer.replaceChildren(h("p", { className: "said", textContent: ctx.say(e) }))));
    // EVERY OTHER PAGE: a VIEW (`ctx.view`, render form) — built, and built again when votes or polls change (another
    // member's vote, a finalization), put in place only when it differs and never while a choice is being made.
    view = ctx.view(outer, {
      render: async () => {
        const box = h("div", { style: "display:contents" });
        root = box;
        if (s.startsWith("p/")) {
          const it = await getItem(refIn(s));
          if (!it) return h("p", { className: "none", textContent: "This is not there, or not yours to see." });
          await (it.kind === "poll" ? pollPage(it, at) : proposalPage(it, at));
        } else if (inSp && s.startsWith("g/")) await graphPage(at, refIn(s));
        else if (inSp && s === "graph") await graphPage(at, null);
        else if (inSp && s === "delegation") await delegationPage(at);
        else await list(at);
        return box;
      },
      looks: { loading: () => theme.loading("Opening…") },
    });
  }
  let view = null;
  await draw();
  addEventListener("craftworks:route", () => el.isConnected && ctx.route === "/vote" && draw());
}
