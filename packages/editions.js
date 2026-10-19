// EDITIONS, a component: an item's EDITIONS on its page (TOKEN §1c; owner 10-10: item > editions > copies, bought and
// sold on the item itself — no separate shop). The item lists its editions in its own `meta.editions` (only its author
// writes it); each is shown only if it names this item and its author made it (`core.edition_of_item`). Anyone buys a
// copy on demand — a payment and a paid mint, no one else acting — and collects it; the author makes new editions.
// Every token entry is built by the core (the wallet) and signed by this node's identity delegate — `tokens`' run.
//
//   const editions = await ctx.require("editions");
//   page.append(await editions.section(it, { changed }))   // null when editions are not set up, or none to show
//
// SET UP by `tokens` (the page's beacon; its `tokens-config`: the token copies are paid in); no pay token: no section.
export async function start(ctx) {
  const tokens = await ctx.require("tokens");
  if (!tokens.on || !tokens.pay) return { section: async () => null };
  const [node, auth, items] = await Promise.all(["node", "auth", "items"].map(n => ctx.require(n)));
  const { core } = node;
  const { beacon, pay, code: tokenCode, idlogCode, run, putAll, now, bytes, hexOf, purse } = tokens;

  const style = document.createElement("style");
  style.textContent = `@layer components {

    .cw-editions { background: var(--cw-surface); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); padding: var(--cw-space-3); display: grid; gap: var(--cw-space-2); }
    .cw-editions h3 { margin: 0; font-size: 1rem; }
    .cw-editions .ed { display: flex; gap: var(--cw-space-3); align-items: center; flex-wrap: wrap; }
    .cw-editions .ed .name { font-weight: 600; }
    .cw-editions .s { color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .cw-editions .said { color: var(--cw-danger); margin: 0; font-size: var(--cw-text-sm); }
    .cw-editions form { display: grid; gap: var(--cw-space-2); grid-template-columns: repeat(auto-fit, minmax(9rem, 1fr)); align-items: end; }
    .cw-editions label { display: grid; gap: var(--cw-space-1); font-size: var(--cw-text-sm); }
}`;
  document.head.append(style);
  const h = (await ctx.require("theme")).h;

  // An edition AS THIS ITEM SHOWS IT (null: not shown — another item's). Its author is the item's (only the author
  // writes `meta.editions`); `mine`: the copies minted to this wallet's one-time owners.
  async function edition(it, idHex, me) {
    const owners = me ? purse.owners() : [];
    const e = await run(() => core.edition_of_item(tokenCode, beacon, idlogCode, bytes(idHex), it.ref, owners));
    if (!e.shown) ctx.log("editions", { what: `${idHex.slice(0, 12)}… not shown on ${it.ref}: ${e.why}` });
    return e.shown ? e : null;
  }

  // BUY one copy PRIVATELY (TOKEN §7f): paid from this wallet's private chains of the sale's token (each seed's in
  // turn), the copy minted to the paying one-time owner; then COLLECT it into that owner's chain of the edition.
  async function buy(e) {
    const at = await now();
    let b = null;
    let why = "no private balance of the sale's token: move some to private in your Wallet";
    for (const g of await purse.privately(e.sale.token)) {
      try {
        b = await run(() => core.edition_buy(tokenCode, beacon, idlogCode, bytes(e.id), bytes(g.seed), g.mine.map(f => bytes(f.a)), 1, at));
        break;
      } catch (err) {
        why = err.message ?? String(err);
      }
    }
    if (!b) throw new Error(why);
    await putAll(b.puts ?? []);
    const at2 = await now();
    const c = await run(() => core.edition_collect(tokenCode, beacon, idlogCode, bytes(e.id), bytes(b.to), Number(b.first), at2));
    await putAll(c.puts ?? []);
    return b;
  }

  // A NEW EDITION of this item by its author, listed in the item's own `meta.editions`: its sales and royalty paid to a
  // fresh one-time owner of the author's (TOKEN §7f) — kept in the purse, which collects its sales.
  async function create(it, f) {
    const nonce = crypto.getRandomValues(new Uint8Array(16));
    const payee = await purse.payee(hexOf(pay));
    const made = core.edition_create(tokenCode, beacon, idlogCode, it.ref, bytes(payee.to), f.name, f.cap, f.numbered, pay, f.price, f.royaltyBps, nonce);
    await payee.keep(made.id);
    await putAll(made.puts);
    const list = [...(it.meta?.editions ?? []), made.id];
    await items.editItem(it.ref, it.body, { files: it.files ?? null, meta: { ...(it.meta ?? {}), editions: list }, title: it.title });
    return made.id;
  }

  function form(it, changed) {
    const said = h("p", { className: "said" });
    const name = h("input", { placeholder: "Gold (empty: the item's title)" });
    const price = h("input", { type: "number", min: "1", value: "10" });
    const cap = h("input", { type: "number", min: "0", value: "100" });
    const numbered = h("input", { type: "checkbox", checked: true });
    const royalty = h("input", { type: "number", min: "0", max: "100", value: "10" });
    const go = h("button", { type: "submit", textContent: "Make edition" });
    const f = h(
      "form",
      {},
      h("label", {}, "Name", name),
      h("label", {}, "Price a copy", price),
      h("label", {}, "Copies (0: no limit)", cap),
      h("label", {}, "Royalty % on resales", royalty),
      h("label", {}, h("span", {}, numbered, " Numbered copies")),
      go,
    );
    f.onsubmit = async ev => {
      ev.preventDefault();
      go.disabled = true;
      said.textContent = "";
      try {
        await create(it, { name: name.value.trim(), price: Number(price.value), cap: Number(cap.value), numbered: numbered.checked, royaltyBps: Math.round(Number(royalty.value) * 100) });
        changed?.();
      } catch (e) {
        said.textContent = ctx.say(e);
        go.disabled = false;
      }
    };
    return h("div", {}, h("div", { className: "s", textContent: "Sell copies of this item: anyone buys one by paying — you need not be online." }), f, said);
  }

  // THE SECTION on an item's page.
  async function section(it, { changed } = {}) {
    const me = await auth.check();
    const author = me && me.did === it.by;
    const ids = Array.isArray(it.meta?.editions) ? it.meta.editions : [];
    if (!ids.length && !author) return null;
    const box = h("div", { className: "cw-editions" }, h("h3", { textContent: "Editions" }));
    for (const id of ids) {
      const row = h("div", { className: "ed" }, h("span", { className: "s", textContent: "reading…" }));
      box.append(row);
      edition(it, id, me).then(
        e => {
          if (!e) return row.remove();
          const left = e.cap === "0" ? "no limit" : `${BigInt(e.cap) - BigInt(e.minted)} of ${e.cap} left`;
          const mine = e.mine?.length ? `you hold ${e.numbered ? e.mine.map(n => `#${n}`).join(", ") : `${e.mine.length}`}` : null;
          const said = h("span", { className: "said" });
          const b = me && e.open != null ? h("button", { type: "button", textContent: `Buy · ${e.sale.price}` }) : null;
          if (b)
            b.onclick = async () => {
              b.disabled = true;
              said.textContent = "";
              try {
                const got = await buy(e);
                b.textContent = `Bought #${got.first}`;
                changed?.();
              } catch (err) {
                said.textContent = ctx.say(err);
                b.disabled = false;
              }
            };
          row.replaceChildren(
            ...[
            h("span", { className: "name", textContent: e.name || it.title || "Edition" }),
            h("span", { className: "s", textContent: `${e.numbered ? "numbered" : "copies"} · ${left}${e.royalty ? ` · ${e.royalty.bps / 100}% royalty` : ""}` }),
            mine ? h("span", { className: "s", textContent: mine }) : null,
            e.open == null ? h("span", { className: "s", textContent: "sold out" }) : b,
            said,
            ].filter(Boolean),
          );
        },
        err => row.replaceChildren(h("span", { className: "said", textContent: `an edition could not be read: ${ctx.say(err)}` })),
      );
    }
    if (author) box.append(form(it, changed));
    return box;
  }

  return { section };
}
