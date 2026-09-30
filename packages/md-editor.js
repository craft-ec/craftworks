// MD EDITOR, a component: the one text editor for what a person writes at length — a post, a comment, an edit (Grid's
// editor): a Markdown toolbar (heading, bold, italic, strike, code, a code block, link, quote, lists, a table), a
// Preview (drawn by `markdown`, exactly as it will show), and MEDIA INLINE: 🖼 picks an image, a video or an audio
// (the attachments picker's: sent with its progress, listed in Drive), and once it is sent `![name](file:KEY)` is
// written where the cursor is — any file attached with 📎 that is an image, a video or an audio goes inline the same
// way; other files stay attachments below. The item keeps the files (`files()`): who reads it reads them.
//
//   const ed = (await ctx.require("md-editor")).create({ name: "body", value, placeholder, pick })   // pick: an
//                                                    // attachments picker (its space and publicness are the item's)
//   form.append(ed.el)        ed.value()      ed.files()      ed.busy()      ed.focus()      ed.clear()
export async function start(ctx) {
  const markdown = await ctx.require("markdown");
  const style = document.createElement("style");
  style.textContent = `
    .cw-mde { display: grid; gap: 4px; }
    .cw-mde .bar { display: flex; flex-wrap: wrap; gap: 3px; align-items: center; }
    .cw-mde .bar button { font: inherit; font-size: 13px; font-weight: 700; min-width: 28px; line-height: 1.4; padding: 2px 7px; cursor: pointer;
      border: 1px solid var(--cw-line); border-radius: var(--cw-radius-sm); background: var(--cw-surface); color: var(--cw-fg); }
    .cw-mde .bar button:hover, .cw-mde .bar button.on { border-color: var(--cw-accent); color: var(--cw-accent); }
    .cw-mde .bar .sp { flex: 1; }
    .cw-mde textarea { font: inherit; width: 100%; box-sizing: border-box; min-height: 110px; resize: none; overflow-y: hidden; padding: var(--cw-space-2);
      border: 1px solid var(--cw-line); border-radius: var(--cw-radius-sm); background: var(--cw-surface); color: var(--cw-fg); }
    .cw-mde .prev { min-height: 110px; padding: var(--cw-space-2); border: 1px dashed var(--cw-line); border-radius: var(--cw-radius-sm); }
    .cw-mde .none { color: var(--cw-muted); }`;
  document.head.append(style);
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  const MEDIA = /^(image|video|audio)\//;

  function create({ name = "body", value = "", placeholder = "", pick = null, label = "" } = {}) {
    const ta = h("textarea", { name, value, placeholder, ariaLabel: label || placeholder || "Text" });
    // GROWS with what is written (no inner scroll), from its least height.
    const grow = () => {
      ta.style.height = "auto";
      ta.style.height = `${Math.max(110, ta.scrollHeight + 2)}px`;
    };
    ta.addEventListener("input", grow);
    requestAnimationFrame(grow);
    const prev = h("div", { className: "prev", hidden: true });
    const at = () => [ta.selectionStart, ta.selectionEnd, ta.value];
    const put = (text, s, e, selFrom, selTo) => {
      ta.value = ta.value.slice(0, s) + text + ta.value.slice(e);
      ta.focus();
      ta.selectionStart = s + selFrom;
      ta.selectionEnd = s + selTo;
      ta.dispatchEvent(new Event("input"));
    };
    const surround = (a, b = a, hint = "text") => {
      const [s, e, v] = at();
      const sel = v.slice(s, e) || hint;
      put(a + sel + b, s, e, a.length, a.length + sel.length);
    };
    const linePrefix = p => {
      const [s, e, v] = at();
      const ls = v.lastIndexOf("\n", s - 1) + 1;
      const lines = v.slice(ls, e).split("\n");
      const text = lines.map((l, i) => (typeof p === "function" ? p(i) : p) + l).join("\n");
      put(text, ls, e, 0, text.length);
    };
    const insert = t => {
      const [s, e, v] = at();
      // On a line of its own: media is a block.
      const before = s > 0 && v[s - 1] !== "\n" ? "\n" : "";
      const after = v[e] && v[e] !== "\n" ? "\n" : "";
      const text = before + t + after;
      put(text, s, e, text.length, text.length);
    };
    const btn = (label, title, fn) => h("button", { type: "button", textContent: label, title, onclick: e => (e.preventDefault(), fn()) });
    const pv = btn("👁", "Preview", () => {
      const on = prev.hidden;
      prev.hidden = !on;
      ta.hidden = on;
      if (!on) requestAnimationFrame(grow);
      pv.classList.toggle("on", on);
      if (on) prev.replaceChildren(ta.value.trim() ? markdown.render(ta.value, pick?.files() ?? []) : h("span", { className: "none", textContent: "Nothing to preview." }));
    });
    // MEDIA: a file picked here goes inline once sent (and one attached with 📎 that is media, too).
    const mediaIn = h("input", { type: "file", accept: "image/*,video/*,audio/*", multiple: true, hidden: true });
    const inlineNext = new Set(); // files picked with 🖼, waiting to be sent
    mediaIn.onchange = () => {
      for (const f of mediaIn.files) inlineNext.add(f);
      pick?.addFiles?.([...mediaIn.files]);
      mediaIn.value = "";
    };
    pick?.onReady?.((ref, file) => {
      if (!ref || !(MEDIA.test(ref.type ?? "") || ref.type === "application/vnd.craftworks.video+json")) return;
      if (file && !inlineNext.has(file) && !MEDIA.test(file.type)) return;
      inlineNext.delete(file);
      const alt = String(ref.name ?? "").replace(/[\[\]\n]/g, " ");
      insert(`![${alt}](file:${markdown.keyOf(ref)})`);
    });
    const bar = h(
      "div",
      { className: "bar" },
      btn("H", "Heading", () => linePrefix("## ")),
      btn("B", "Bold", () => surround("**")),
      btn("i", "Italic", () => surround("*")),
      btn("S", "Strikethrough", () => surround("~~")),
      btn("`", "Inline code", () => surround("`", "`", "code")),
      btn("```", "Code block", () => surround("```\n", "\n```\n", "code")),
      btn("🔗", "Link", () => surround("[", "](https://)", "text")),
      btn("❝", "Quote", () => linePrefix("> ")),
      btn("•", "List", () => linePrefix("- ")),
      btn("1.", "Numbered list", () => linePrefix(i => `${i + 1}. `)),
      btn("▦", "Table", () => insert("| Column | Column |\n| --- | --- |\n| | |")),
      pick ? btn("🖼", "Image, video or audio (inline)", () => mediaIn.click()) : null,
      h("span", { className: "sp" }),
      pv,
    );
    const el = h("div", { className: "cw-mde" }, bar, ta, prev, mediaIn, pick?.el ?? null);
    return {
      el,
      textarea: ta,
      value: () => ta.value,
      files: () => pick?.files() ?? [],
      busy: () => !!pick?.busy(),
      focus: () => ta.focus(),
      clear: () => ((ta.value = ""), grow(), pick?.clear(), prev.hidden || pv.click()),
    };
  }
  return { create };
}
