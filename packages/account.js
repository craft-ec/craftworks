// ACCOUNT, a PRIVATE page: it shows nothing until someone is logged in. It asks `login`, which shows its dialog if
// nobody is; closing the dialog goes home. Its SUB-PAGES are in the top bar (`#/account/<sub>`), one shown at a time:
// Card, Nodes, Security (how the keys stand), Storage, Apps, Recovery (the words, and a passphrase carrying them),
// Moderation; Log out is the bar's action.
export async function mount(ctx, el) {
  const [auth, login, grantsOf, membership, keys, storage, blocks] = await Promise.all(
    ["auth", "login", "access", "membership", "keys", "storage", "blocks"].map(n => ctx.require(n)),
  );
  const s = await login.session();
  if (!s) {
    location.hash = "#/";
    return;
  }
  const box = document.createElement("div");

  // Each sub-page: made only when it is the one shown.
  const sections = {
    // CARD: who you are (your id, this node's key) and your public card.
    async card() {
      const line = (label, value) => {
        const row = document.createElement("p");
        const code = document.createElement("code");
        code.textContent = value;
        row.append(`${label}: `, code);
        box.append(row);
      };
      line("Account (DID)", s.did);
      line("This node's member key", s.member);

      // YOUR CARD: the public face others find you by (your id, and a handle — not unique: people tell you apart by your
      // id), with this node's key so people can start a conversation with you while you are away.
      const card = document.createElement("section");
      card.innerHTML = `<h3>Your card</h3><p class="note">Public: anyone with your id sees it. Your handle is a name to show; your id is what makes you you.</p>
        <form class="card-form"><label>Handle <input name="handle" maxlength="40" autocomplete="off"></label> <button>Publish</button></form><p class="line said"></p>`;
      box.append(card);
      const directory = await ctx.require("directory");
      const cardForm = card.querySelector(".card-form");
      const cardSaid = card.querySelector(".said");
      directory
        .card(s.did)
        .then(c => {
          if (c?.handle) cardForm.elements.handle.value = c.handle;
          cardSaid.textContent = c ? `Published as ${directory.shown(s.did, c.handle)}${c.keyPackage ? "" : " (no key packages yet)"}.` : "Not published yet.";
        })
        .catch(e => (cardSaid.textContent = `Could not read your card: ${e?.message ?? e}`));
      cardForm.onsubmit = async e => {
        e.preventDefault();
        cardSaid.textContent = "Publishing…";
        try {
          const c = await directory.publish({ handle: cardForm.elements.handle.value.trim() || null });
          cardSaid.textContent = `Published as ${directory.shown(s.did, c.handle)}.`;
        } catch (err) {
          cardSaid.textContent = `Could not publish: ${err?.message ?? err}`;
        }
      };
    },

    async nodes() {
      // YOUR NODES: the account's members — its MLS group's. A node that is lost, stolen or retired is REMOVED: the group
      // moves to a new epoch, and that node reads nothing written from then on.
      const dev = document.createElement("section");
      dev.innerHTML = `<h3>Your nodes</h3><p class="line">Reading your account from the network…</p>
        <p class="note">Remove a node you lost or no longer use: it will not be able to read anything written after.</p>`;
      box.append(dev);
      const drawNodes = () =>
        membership.nodes().then(
          list => {
            const at = dev.querySelector(".line, ul");
            if (list === "removed") {
              dev.querySelector(".note")?.remove();
              at.replaceWith(Object.assign(document.createElement("p"), { className: "line",
                textContent: "This node was removed from your account: it keeps what it could already read, and gets nothing written since. Log out; to use it again, log in with your recovery words." }));
              return;
            }
            if (list === null) {
              at.replaceWith(Object.assign(document.createElement("p"), { className: "line",
                textContent: "This node is not in your account's group yet: log out and log in once with your recovery words (and a new PIN)." }));
              return;
            }
            const ul = document.createElement("ul");
            for (const m of list) {
              const li = document.createElement("li");
              const code = document.createElement("code");
              code.textContent = `${m.key.slice(0, 16)}…`;
              // A node is shown by its key: a name guessed from the browser was wrong (the browser is not the member).
              li.append("Node ", code);
              if (m.me) li.append(" (this node)");
              else {
                const rm = Object.assign(document.createElement("button"), { type: "button", textContent: "Remove", className: "remove" });
                rm.onclick = async () => {
                  if (!confirm(`Remove node ${m.key.slice(0, 16)}…? It will not be able to read anything written after this.`)) return;
                  rm.disabled = true;
                  rm.textContent = "Removing…";
                  try {
                    await membership.remove(m.index);
                  } catch (e) {
                    alert(`Could not remove it: ${e?.message ?? e}`);
                  }
                  drawNodes();
                };
                li.append(" ", rm);
              }
              ul.append(li);
            }
            at.replaceWith(ul);
          },
          e => (dev.querySelector(".line").textContent = `Could not read your nodes: ${e?.message ?? e}`),
        );
      drawNodes();
    },

    async security() {
      // SECURITY: how the account's keys stand (getting back in — the words, a passphrase — is Recovery).
      const sec = document.createElement("section");
      sec.innerHTML = `<ul class="facts"><li>Reading…</li></ul>`;
      box.append(sec);
      async function drawSecurity() {
        const [log, st, escrowed] = await Promise.all([
          auth.identity.keyLogInfo(s.didBytes),
          keys.ready(),
          keys.escrowed().catch(() => null),
        ]);
        const facts = [
          log ? `Recovery words changed ${log.changes} time${log.changes === 1 ? "" : "s"} (your key log has ${log.events} event${log.events === 1 ? "" : "s"}).` : "Your key log is not on the network yet.",
          st?.removed ? "This node was removed from your account: it holds no keys for anything written since." :
          st ? `Your data is sealed with the keys of epoch ${st.epoch}; ${st.members.length} ${st.members.length === 1 ? "node holds" : "nodes hold"} them.` : "This site does not hold your account's keys.",
          escrowed === null ? null : `${escrowed} epoch${escrowed === 1 ? "" : "s"} kept in escrow for your recovery words: with the words alone, every one of them opens again.`,
        ].filter(Boolean);
        sec.querySelector(".facts").replaceChildren(...facts.map(t => Object.assign(document.createElement("li"), { textContent: t })));
      }
      drawSecurity().catch(e => (sec.querySelector(".facts").textContent = `Could not read: ${e?.message ?? e}`));

    },

    async storage() {
      // STORAGE: each of the account's tables, and how this page read them.
      const sto = document.createElement("section");
      sto.innerHTML = `<p class="note">KEPT: this node puts its tables back on the network — every block of each tree, and
        the table itself — about once a week, one table at a time while a page is open; a missing block is made again
        from its group. HEALTH: a group is whole (all its blocks there), degraded (enough to rebuild), or damaged.</p>
        <p><button type="button" class="keep-now">Keep all now</button> <span class="note kept-said"></span></p>
        <table class="tables"><thead><tr><th>Table</th><th>Rows</th><th>Sealed</th><th>Where</th><th>Kept</th><th>Health</th></tr></thead>
        <tbody><tr><td colspan="6">Reading…</td></tr></tbody></table><p class="files-kept"></p><p class="note blocks"></p>`;
      box.append(sto);
      const keep = await ctx.require("keep");
      const ago = at => {
        const m = Math.round((Date.now() - at) / 60000);
        return m < 1 ? "just now" : m < 60 ? `${m} min ago` : m < 1440 ? `${Math.round(m / 60)} h ago` : `${Math.round(m / 1440)} d ago`;
      };
      const health = k => (!k ? "—" : k.error ? `failed: ${k.error}` : !k.groups ? "no tree yet" : `${k.whole}/${k.groups} whole${k.degraded ? `, ${k.degraded} degraded` : ""}${k.damaged ? `, ${k.damaged} DAMAGED` : ""} · ${k.blocks - k.missing}/${k.blocks} blocks`);
      const draw = () => Promise.all([storage.describe(), keep.status().catch(() => [])]).then(
        ([list, kept]) => {
          const last = new Map();
          for (const k of kept) if (k.name && (!last.has(k.name) || last.get(k.name).at < k.at)) last.set(k.name, k);
          // FILES: how the coded files stand (each kept on its own; a damaged one named).
          const fk = kept.filter(k => k.file);
          const sum = (f, k) => fk.reduce((n, x) => n + (x[k] ?? 0), 0);
          const bad = fk.filter(x => x.error || x.damaged);
          sto.querySelector(".files-kept").textContent = fk.length
            ? `Files: ${fk.length} kept — ${sum(fk, "whole")}/${sum(fk, "gens")} generation(s) whole, ${sum(fk, "degraded")} degraded, ${sum(fk, "damaged")} damaged; ${sum(fk, "pieces") - sum(fk, "missing")}/${sum(fk, "pieces")} pieces there.${bad.length ? ` Needing attention: ${bad.map(x => `${x.file.slice(0, 8)}…${x.error ? ` (${x.error})` : ""}`).join(", ")}.` : ""}`
            : "Files: none kept yet.";
          sto.querySelector("tbody").replaceChildren(
            ...list.map(t => {
              const tr = document.createElement("tr");
              if (t.closed) {
                // Another app's table: not opened from here.
                for (const v of [t.name, "—", "—", "another app's", "—", "—"]) tr.append(Object.assign(document.createElement("td"), { textContent: v }));
                return tr;
              }
              const where = (t.unopened ? `${t.unopened} feed(s) not readable here — log in with your recovery words; ` : "") + (t.flushed ? (t.pending ? `tree + ${t.pending} in the tail` : "tree") : `tail (${t.pending} row${t.pending === 1 ? "" : "s"})`);
              const k = last.get(t.name);
              for (const v of [t.name, String(t.rows), t.sealed ? (t.writes === "table" ? "table key" : t.writes ?? "yes") : "no key here", where, k ? ago(k.at) : "not yet", health(k)]) tr.append(Object.assign(document.createElement("td"), { textContent: v }));
              return tr;
            }),
          );
          const b = blocks.stats();
          sto.querySelector(".blocks").textContent = `This page read ${b.read} tree block${b.read === 1 ? "" : "s"}; ${b.rebuilt} came from their recovery group first (rebuilt and checked).`;
        },
        e => (sto.querySelector("tbody").textContent = `Could not read: ${e?.message ?? e}`),
      );
      draw();
      sto.querySelector(".keep-now").onclick = async e => {
        e.target.disabled = true;
        const said = sto.querySelector(".kept-said");
        said.textContent = "Keeping every table…";
        const out = await keep.now().catch(err => [{ error: err?.message ?? String(err) }]);
        const bad = out.filter(k => k.error || k.damaged);
        said.textContent = `${out.length} table(s) kept${bad.length ? `; ${bad.length} need attention` : ""}.`;
        e.target.disabled = false;
        draw();
      };
    },

    async apps() {
      // APPS WITH ACCESS: the sites this account allowed to write its tables (the home app's own need no prompt).
      const access = document.createElement("section");
      access.innerHTML = `<h3>Apps with access</h3><p class="note">Your data belongs to your account. These sites may
        change it; any site can be removed, and will ask again next time.</p><ul class="grants"><li>Reading…</li></ul>`;
      box.append(access);
      const drawGrants = async () => {
        const ul = access.querySelector(".grants");
        const list = await grantsOf.grants();
        ul.replaceChildren();
        if (!list.length) ul.append(Object.assign(document.createElement("li"), { textContent: "None yet." }));
        const here = location.pathname.split("/")[4];
        // One line per SITE, with the kinds of data it may change; each can be removed on its own.
        const bySite = new Map();
        for (const g of list) bySite.set(g.app, [...(bySite.get(g.app) ?? []), g.table]);
        for (const [app, tables] of bySite) {
          const li = document.createElement("li");
          const code = document.createElement("code");
          code.textContent = `${app.slice(0, 10)}…`;
          li.append(app === here ? "This app " : "Site ", code, app === here ? "" : "", ": ");
          tables.forEach((table, i) => {
            const chip = Object.assign(document.createElement("button"), { type: "button", textContent: `${table} ✕`, title: `Remove its access to your ${table}` });
            chip.onclick = async () => {
              await grantsOf.revoke(app, table);
              drawGrants();
            };
            li.append(i ? " " : "", chip);
          });
          ul.append(li);
        }
      };
      drawGrants().catch(e => (access.querySelector(".grants").textContent = `Could not read: ${e?.message ?? e}`));
    },

    async recovery() {
      // RECOVERY WORDS: shown once, at registration; no node keeps them. They can be CHANGED: the account (its DID, its
      // data) stays; the old words stop opening it.
      const rec = document.createElement("section");
      rec.innerHTML = `<p class="note">Two ways back into your account on a new device: your recovery words (the key itself),
        and — if you set one — a passphrase that carries them.</p>
        <h3>1 · Recovery words</h3>
        <p>Your account's master key: twelve words, shown once when you registered. With them you log in on any node and
        get your account back. No node keeps them, so a lost or stolen node cannot give your account away.</p>
        <p>You can change them: your account and its data stay the same, and the old words stop working.</p>
        <button type="button" class="change">Change recovery words</button>
        <form class="old" hidden>
          <label>Your current recovery words<textarea name="words" rows="3" autocomplete="off" spellcheck="false"></textarea></label>
          <button>Next</button> <button type="button" class="cancel">Cancel</button>
        </form>
        <form class="new" hidden>
          <p><strong>Your new recovery words.</strong> Write them down and keep them offline. They are shown only now.</p>
          <ol class="shown"></ol>
          <label><input type="checkbox" name="kept" required> I have written them down</label>
          <button>Change to these words</button> <button type="button" class="cancel">Cancel</button>
        </form>
        <p class="said"></p>`;
      box.append(rec);
      {
        const [change, oldF, newF, said] = [".change", "form.old", "form.new", ".said"].map(q => rec.querySelector(q));
        const glue = (await ctx.require("node")).glue.CraftworksCore;
        let old = null;
        let fresh = null;
        const reset = () => {
          old?.fill(0);
          fresh?.fill(0);
          old = fresh = null;
          oldF.reset();
          newF.reset();
          oldF.hidden = newF.hidden = true;
          change.hidden = false;
          newF.querySelector(".shown").replaceChildren();
        };
        for (const c of rec.querySelectorAll(".cancel")) c.onclick = reset;
        change.onclick = () => {
          change.hidden = true;
          oldF.hidden = false;
          said.textContent = "";
        };
        oldF.onsubmit = e => {
          e.preventDefault();
          try {
            old = glue.entropy_of(oldF.words.value);
          } catch (err) {
            said.textContent = String(err);
            return;
          }
          fresh = crypto.getRandomValues(new Uint8Array(16));
          newF.querySelector(".shown").replaceChildren(
            ...glue.words_of(fresh).split(" ").map(w => Object.assign(document.createElement("li"), { textContent: w })),
          );
          oldF.hidden = true;
          newF.hidden = false;
        };
        newF.onsubmit = async e => {
          e.preventDefault();
          said.textContent = "Changing your recovery words…";
          try {
            await auth.identity.changeWords(s.didBytes, old, fresh);
            said.textContent = "Done. Your new words open your account; the old ones no longer do.";
          } catch (err) {
            said.textContent = `Could not change them: ${err?.message ?? err}`;
          }
          reset();
        };
      }
      // RECOVERY PASSPHRASE: a new device opens the account with it (and the account's id) instead of the words.
      const pass = document.createElement("section");
      pass.innerHTML = `<h3>2 · Recovery passphrase (optional)</h3>
        <p>A shortcut for your words: they are sealed with a passphrase you choose, and the sealed copy is kept on your
        card. On a new device you then log in with your account's id and this passphrase (Log in → Use a passphrase
        instead) — no words to type. It does not replace the words; changing the words removes it (set it again).</p>
        <p class="note">Anyone can fetch the sealed copy and try guesses, so make it long: a sentence of 4 or more words.</p>
        <p class="line state">Reading…</p>
        <form class="rec-form"><label>Your recovery words (once, to seal them) <textarea name="words" rows="2" autocomplete="off" spellcheck="false" required></textarea></label>
          <label>Passphrase (12 or more) <input name="passphrase" type="password" minlength="12" autocomplete="off" required></label>
          <label>Confirm it <input name="again" type="password" minlength="12" autocomplete="off" required></label>
          <button>Set the passphrase</button> <button type="button" class="clear">Remove it</button></form>
        <p class="line said"></p>`;
      box.append(pass);
      const recovery = await ctx.require("recovery");
      const recSaid = pass.querySelector(".said");
      const recState = async () => {
        const on = await recovery.has().catch(() => false);
        pass.querySelector(".state").textContent = on ? "A recovery passphrase is set." : "No recovery passphrase yet.";
        pass.querySelector(".clear").hidden = !on;
      };
      recState();
      const recForm = pass.querySelector(".rec-form");
      recForm.onsubmit = async e => {
        e.preventDefault();
        if (recForm.passphrase.value !== recForm.again.value) return (recSaid.textContent = "The two passphrases differ.");
        recSaid.textContent = "Sealing (a few seconds: the stretch that makes guessing slow)…";
        try {
          await recovery.set(recForm.words.value, recForm.passphrase.value);
          recForm.reset();
          recSaid.textContent = "Set. On a new device: Log in → Use a passphrase instead.";
          recState();
        } catch (err) {
          recSaid.textContent = `Not set: ${err?.message ?? err}`;
        }
      };
      pass.querySelector(".clear").onclick = async () => {
        await recovery.clear().catch(err => (recSaid.textContent = err.message));
        recSaid.textContent = "Removed.";
        recState();
      };
    },
  };
  // MODERATION: what this person sees in Discover — their own list (what they flagged) and the lists they apply.
  sections.moderation = async () => {
    const [moderation, directory] = await Promise.all(["moderation", "directory"].map(n => ctx.require(n)));
    const lists = await moderation.lists();
    const sec = document.createElement("section");
    const el = (tag, props = {}, ...kids) => {
      const e = Object.assign(document.createElement(tag), props);
      e.append(...kids.filter(Boolean));
      return e;
    };
    const draw = () => {
      const who = lists.followed();
      const entries = lists.entries();
      sec.replaceChildren(
        el("p", { className: "note", textContent: "Discover (the public network) has no owner: what you see there is filtered by your own list and the lists you choose to apply." }),
        el("h3", { textContent: `Your list (${entries.length})` }),
        entries.length
          ? el("ul", {}, ...entries.map(x => el("li", {}, `${x.kind}: ${x.ref.slice(0, 48)}${x.ref.length > 48 ? "…" : ""} `, el("button", { type: "button", textContent: "Remove", onclick: () => lists.unflag(x.kind, x.ref).then(draw) }))))
          : el("p", { className: "note", textContent: "Nothing flagged. Flag a post or its author in Discover." }),
        el("h3", { textContent: "Lists you apply" }),
        who.length
          ? el("ul", {}, ...who.map(did => {
              const n = el("span", { textContent: directory.shown(did) });
              directory.name(did).then(t => (n.textContent = t), () => {});
              return el("li", {}, n, " ", el("button", { type: "button", textContent: "Stop applying", onclick: async () => ((await (await (await ctx.require("edge")).people()).set("modlist", did, false)), draw()) }));
            }))
          : el("p", { className: "note", textContent: "None. Apply someone's list from their name: Use their moderation list." }),
      );
    };
    draw();
    lists.onChange(draw);
    box.append(sec);
  };
  const titles = { card: "Card", nodes: "Nodes", security: "Security", storage: "Storage", apps: "Apps", recovery: "Recovery", moderation: "Moderation" };
  const sub = sections[ctx.sub] ? ctx.sub : "card";
  // The top bar: the sub-pages, and Log out.
  ctx.actions["/account"] = [
    ...Object.entries(titles).map(([k, label]) => ({ label, href: k === "card" ? "#/account" : `#/account/${k}`, on: k === sub })),
    { label: "Log out", run: () => auth.logout(), end: true },
  ];
  dispatchEvent(new CustomEvent("craftworks:actions"));
  box.innerHTML = `<h2>${titles[sub]}</h2>`;
  el.append(box);
  await sections[sub]();
}
