// DRAFTS, a service (FOUNDATION.md §3, "Close at any time"): people close tabs, they do not shut down — so text being
// typed is SAVED AS IT IS TYPED, and put back when the page is opened again. Every text field in the page's body is
// bound by itself (no app does anything): its value written to the account's private `drafts` table after a pause
// (PAUSE_MS) and at once when the page is hidden (the last moment a browser reliably gives); a field EMPTIED (sent,
// cleared) drops its draft. Opened again — here or on another device of the account — an empty field gets its draft back.
//
// NEVER a secret: no password field, nothing in a dialog (the login: PIN, recovery words), nothing marked
// `data-no-draft`, no search box. Drafts are the account's, sealed like every account table; a guest has none.
//
//   const drafts = await ctx.require("drafts");
//   await drafts.flush()      // everything typed written now (before an update reloads the page): true when all held
export async function start(ctx) {
  const PAUSE_MS = 2000;
  const space = await ctx.require("space");
  const storage = await ctx.require("storage");
  let table = null; // the drafts table, once someone is logged in
  const tableOf = async () => (table ??= (await space.account()) ? storage.table("drafts") : null);

  // A field's KEY: where it is (the route and what follows), the component it is in, and its own name — stable across
  // loads of the same page, never its content.
  const keyOf = el => {
    const comp = el.closest("[data-component]")?.dataset.component ?? "?";
    const own = el.name || el.id || el.getAttribute("aria-label") || el.placeholder || "";
    const same = own ? [] : [...(el.closest("[data-component]") ?? document).querySelectorAll(el.tagName)];
    const at = `${ctx.route}${ctx.sub ? `/${ctx.sub}` : ""}${ctx.space ? `@${ctx.space.slice(0, 16)}` : ""}`;
    return `${at}|${comp}|${own || `#${same.indexOf(el)}`}`.slice(0, 200);
  };
  const isText = el =>
    el instanceof HTMLTextAreaElement ||
    (el instanceof HTMLInputElement && ["text", "", "url", "email"].includes(el.type) && el.type !== "search") ||
    (el instanceof HTMLElement && el.isContentEditable && el.getAttribute("contenteditable") !== "false");
  // NEVER A SECRET (audit 10-13: two recovery-words fields of the account page — no name, `name="own"` — would have been
  // drafted and synced): nothing in the account, settings or login screens at all; nothing whose name, id, placeholder
  // or label speaks of a PIN, words, a phrase, a key or a secret; no password field; nothing marked `data-no-draft`.
  const SECRETISH = /pin|word|phrase|secret|key|recover|mnemonic|seed|passw/i;
  const secret = el =>
    !!el.closest("dialog, [data-no-draft], .auth-dialog, header, [role=search], [data-component=account], [data-component=settings], [data-component=login]") ||
    (el instanceof HTMLInputElement && el.type === "password") ||
    [el.name, el.id, el.getAttribute?.("placeholder"), el.getAttribute?.("aria-label")].some(v => SECRETISH.test(v ?? ""));
  const valueOf = el => (el.isContentEditable ? el.innerText : el.value);
  const setValue = (el, v) => {
    if (el.isContentEditable) el.innerText = v;
    else el.value = v;
    el.dispatchEvent(new Event("input", { bubbles: true }));
  };

  const owed = new Map(); // key → value (null: drop) not written yet
  const written = new Map(); // key → what the table holds (as this page knows it)
  let timer = null;
  async function flush() {
    clearTimeout(timer);
    if (!owed.size) return true;
    const t = await tableOf();
    if (!t) return (owed.clear(), true);
    let ok = true;
    for (const [k, v] of [...owed]) {
      owed.delete(k);
      if ((written.get(k) ?? null) === v) continue;
      const put = v === null ? t.remove(k) : t.put(k, JSON.stringify({ v, at: Date.now() }));
      await put.then(
        () => written.set(k, v),
        e => ((ok = false), owed.has(k) || owed.set(k, v), ctx.failed("drafts", e, `not saved: ${e?.message ?? e}`)),
      );
    }
    return ok;
  }
  const soon = () => {
    clearTimeout(timer);
    timer = setTimeout(flush, PAUSE_MS);
  };
  const bound = new Map(); // key → the field it was last typed in or put back into
  addEventListener("input", e => {
    const el = e.target;
    if (!isText(el) || secret(el) || !el.closest(".slot-body")) return;
    const v = valueOf(el);
    const k = keyOf(el);
    bound.set(k, el);
    owed.set(k, v.trim() ? v : null);
    soon();
  }, true);
  // SENT: what was typed went (Enter in a field, a form submitted, a button of its component pressed) — its draft is
  // dropped once the field is empty, and never put back into the next field drawn there.
  const sent = new Set();
  const sending = root => {
    for (const [k, el] of bound)
      if (root.contains(el)) {
        sent.add(k);
        setTimeout(() => {
          const now = bound.get(k);
          if (!now?.isConnected || !valueOf(now).trim()) (owed.set(k, null), soon());
          else sent.delete(k);
        }, 1000);
      }
  };
  addEventListener("keydown", e => e.key === "Enter" && !e.shiftKey && isText(e.target) && sending(e.target.closest("[data-component]") ?? e.target), true);
  addEventListener("submit", e => sending(e.target), true);
  addEventListener("click", e => e.target instanceof Element && e.target.closest("button") && sending(e.target.closest("[data-component]") ?? document.body), true);
  // EMPTIED BY THE APP (no input event): dropped on the next look.
  const sweep = () => {
    for (const [k, el] of bound) if (el.isConnected && !valueOf(el).trim() && (owed.get(k) ?? written.get(k))) (owed.set(k, null), soon());
  };
  document.addEventListener("visibilitychange", () => document.visibilityState === "hidden" && flush());

  // PUT BACK: a field that appears empty gets its draft (read once per page; a field the person already typed in is
  // never overwritten).
  let held = null;
  const heldNow = async () =>
    (held ??= ctx.optional("drafts: async", (async () => {
      const t = await tableOf();
      if (!t) return new Map();
      await t.settled;
      const m = new Map();
      for (const r of t.rows()) {
        try {
          const d = JSON.parse(r.value);
          if (typeof d?.v === "string" && d.v) (m.set(r.key, d.v), written.set(r.key, d.v));
        } catch {}
      }
      return m;
    })(), () => new Map()));
  async function restore(root) {
    const fields = [...root.querySelectorAll("textarea, input, [contenteditable]")].filter(el => isText(el) && !secret(el));
    if (!fields.length) return;
    const m = await heldNow();
    for (const el of fields) {
      const k = keyOf(el);
      if (bound.get(k) === el || sent.has(k) || owed.get(k) === null) continue;
      const v = m.get(k);
      if (v && !valueOf(el)) {
        bound.set(k, el);
        setValue(el, v);
        ctx.log("drafts", { what: `put back: ${keyOf(el).split("|").slice(0, 2).join(" ")}` });
      }
    }
  }
  const body = () => document.querySelector(".slot-body");
  // Looked at once things settle (not on every change): emptied fields swept, new empty ones given their drafts.
  let look = null;
  new MutationObserver(() => {
    clearTimeout(look);
    look = setTimeout(() => {
      sweep();
      if (body()) ctx.background("drafts: restore", restore(body()));
    }, 300);
  }).observe(document.body, { childList: true, subtree: true });
  addEventListener("craftworks:auth", () => ((table = null), (held = null)));

  return { flush, owed: () => owed.size };
}
