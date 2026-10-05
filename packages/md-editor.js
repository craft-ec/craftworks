// MD EDITOR, a component: the one editor for what a person writes — a post, a comment, a message, an edit (Reddit's:
// RICH TEXT by default, Markdown as the alternative). What is KEPT is always Markdown (`markdown` shows it).
// - RICH: what is written shows as it will read — the toolbar formats the selection (heading, bold, italic, strike,
//   superscript, hidden, code, link, quote, lists, a task list, a table, a divider), media shows as itself.
// - MARKDOWN: the text as it is kept (rich text shows it as it reads). Switching carries the text across (the person's choice is kept).
// ＋ INSERT, the one menu: UPLOAD an image, a video, an audio or a book — from this device or Drive (the attachments
// picker's: sent with its progress; from Drive, taken into the item's space) — shown where the cursor was, kept as
// `![name](file:KEY)`; a NEW post or note (the one composer); or any item FOUND (yours, saved, or by its reference),
// kept as `![title](item:REF)`. 📎 attaches any file below. The item keeps the files (`files()`): who reads it reads them.
// COMPACT (a chat line): the toolbar behind "Aa", Enter sends (`onSubmit`), Shift+Enter a new line. MENTIONS, in every
// composer alike: after "@", people to pick (`suggest(q)` → [{ did, shown }]; by default `person.mentionable`), each
// written `[@name#abc123](person:DID)` — drawn by `markdown` as a link to their card.
//
//   const ed = (await ctx.require("md-editor")).create({ value, placeholder, pick, compact, onSubmit, suggest })
//   form.append(ed.el)   ed.value()   ed.set(md)   ed.files()   ed.busy()   ed.focus()   ed.clear()   ed.disable(bool)
export async function start(ctx) {
  const [markdown, kinds, attachments] = await Promise.all([ctx.require("markdown"), ctx.require("kinds"), ctx.require("attachments")]);
  const style = document.createElement("style");
  style.textContent = `
    .cw-mde { display: grid; gap: 4px; position: relative; }
    .cw-mde .bar { display: flex; flex-wrap: wrap; gap: 3px; align-items: center; }
    .cw-mde .bar[hidden] { display: none; }
    .cw-mde .bar button:not(.cw-att-menu *), .cw-mde .aa { font: inherit; font-size: 13px; font-weight: 700; min-width: 28px; line-height: 1.4; padding: 2px 7px; cursor: pointer;
      border: 1px solid var(--cw-line); border-radius: var(--cw-radius-sm); background: var(--cw-surface); color: var(--cw-fg); }
    .cw-mde .bar button:not(.cw-att-menu *):hover, .cw-mde .bar button.on:not(.cw-att-menu *), .cw-mde .aa.on { border-color: var(--cw-accent); color: var(--cw-accent); }
    .cw-mde .bar .sp { flex: 1; }
    .cw-mde .bar .mode { font-weight: 400; }
    div.cw-mde textarea, div.cw-mde div.rich { font: inherit; width: 100%; box-sizing: border-box; min-height: 110px; resize: none; overflow-y: hidden; padding: var(--cw-space-2);
      border: 1px solid var(--cw-line); border-radius: var(--cw-radius-sm); background: var(--cw-surface); color: var(--cw-fg); outline: none; }
    div.cw-mde div.rich:focus, div.cw-mde textarea:focus { border-color: var(--cw-accent); }
    div.cw-mde div.rich:empty::before { content: attr(data-placeholder); color: var(--cw-muted); }
    div.cw-mde.compact textarea, div.cw-mde.compact div.rich { min-height: 38px; }
    .cw-mde .rich .media { display: inline-block; vertical-align: middle; margin: 4px 0; user-select: none; }
    .cw-mde .rich .media img { max-width: min(320px, 100%); max-height: 200px; border-radius: var(--cw-radius-sm); border: 1px solid var(--cw-line); display: block; }
    .cw-mde .rich .media .tag { display: inline-block; padding: 6px 10px; border: 1px solid var(--cw-line); border-radius: var(--cw-radius-sm); background: var(--cw-hover); }
    .cw-mde .rich .spoiler { background: var(--cw-hover); color: inherit; outline: 1px dashed var(--cw-line); }
    .cw-mde .none { color: var(--cw-muted); }
    .cw-mde .line-in { display: grid; grid-template-columns: auto 1fr; gap: 4px; align-items: start; }
    .cw-mde .tools-in { display: flex; gap: 3px; align-items: center; padding-top: 5px; }
    .cw-mde .tools-in button:not(.cw-att-menu *) { font: inherit; font-size: 13px; min-width: 28px; line-height: 1.4; padding: 2px 6px; cursor: pointer; border: 1px solid var(--cw-line);
      border-radius: var(--cw-radius-sm); background: var(--cw-surface); color: var(--cw-fg); }
    .cw-mde .tools-in .cw-att-menu { bottom: 100%; top: auto; }
    .cw-mde .suggest { position: absolute; bottom: 100%; left: 0; z-index: 6; list-style: none; margin: 0; padding: 4px; min-width: 180px;
      background: var(--cw-surface); border: 1px solid var(--cw-line); border-radius: var(--cw-radius-sm); box-shadow: var(--cw-shadow-lg); }
    .cw-mde .suggest[hidden] { display: none; }
    .cw-mde .suggest li { padding: 4px 8px; cursor: pointer; border-radius: var(--cw-radius-sm); }
    .cw-mde .suggest li:hover { background: var(--cw-hover); }`;
  document.head.append(style);
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  // What is MEDIA (shown inline): what `kinds` declares a media domain.
  const isMedia = ref => !!kinds.mediaOf(ref);
  const MODE_KEY = "craftworks:editor-mode";
  const savedMode = () => {
    try {
      return localStorage.getItem(MODE_KEY) === "markdown" ? "markdown" : "rich";
    } catch {
      return "rich";
    }
  };
  const saveMode = m => {
    try {
      localStorage.setItem(MODE_KEY, m);
    } catch {}
  };

  // RICH → MARKDOWN: the DOM the person formatted, written back as the Markdown `markdown` reads.
  const escText = t => t.replace(/([\\`*_~[\]|^])/g, "\\$1").replace(/&gt;!|>!/g, "\\>!").replace(/\u00a0/g, " ");
  function inlineMd(node) {
    let out = "";
    for (const n of node.childNodes) {
      if (n.nodeType === 3) {
        out += escText(n.textContent);
        continue;
      }
      if (n.nodeType !== 1) continue;
      if (n.dataset?.md) {
        out += n.dataset.md;
        continue;
      }
      const tag = n.tagName.toLowerCase();
      const inner = () => inlineMd(n);
      if (tag === "br") out += "\n";
      else if (tag === "b" || tag === "strong") out += wrap("**", inner());
      else if (tag === "i" || tag === "em") out += wrap("*", inner());
      else if (tag === "s" || tag === "strike" || tag === "del") out += wrap("~~", inner());
      else if (tag === "code") out += "`" + n.textContent.replace(/`/g, "'") + "`";
      else if (tag === "sup") out += `^(${inner()})`;
      // A mention written before as plain `@name#abc123` stays plain text.
      else if (tag === "a" && n.dataset?.typed) out += n.textContent;
      else if (tag === "a") out += `[${inner() || n.href}](${n.getAttribute("href") || ""})`;
      else if (n.classList?.contains("spoiler")) out += `>!${inner()}!<`;
      else if (tag === "input" && n.type === "checkbox") out += n.checked ? "[x] " : "[ ] ";
      else if (/^(p|div|h[1-6]|blockquote|ul|ol|li|pre|table)$/.test(tag)) out += blockMd(n);
      else out += inner();
    }
    return out;
  }
  // Marks around text keep its edge spaces outside (`** bold **` is not bold in Markdown).
  const wrap = (m, s) => {
    const t = s.trim();
    if (!t) return s;
    const lead = s.slice(0, s.indexOf(t));
    const trail = s.slice(s.indexOf(t) + t.length);
    return `${lead}${m}${t}${m}${trail}`;
  };
  function listMd(n, depth) {
    const ordered = n.tagName.toLowerCase() === "ol";
    const lines = [];
    let i = 0;
    for (const li of n.children) {
      if (li.tagName.toLowerCase() !== "li") continue;
      i++;
      const own = document.createElement("div");
      const nested = [];
      for (const c of li.childNodes) (c.nodeType === 1 && /^(ul|ol)$/i.test(c.tagName) ? nested.push(c) : own.append(c.cloneNode(true)));
      lines.push(`${"  ".repeat(depth)}${ordered ? `${i}.` : "-"} ${inlineMd(own).trim()}`);
      for (const sub of nested) lines.push(listMd(sub, depth + 1));
    }
    return lines.join("\n");
  }
  const BLOCK = /^(p|div|h[1-6]|blockquote|ul|ol|pre|hr|table)$/i;
  function blockMd(n) {
    const tag = n.tagName.toLowerCase();
    if (n.dataset?.md) return n.dataset.md;
    // A paragraph the browser put blocks in (a list typed inside a paragraph): a container, block by block.
    if ((tag === "p" || tag === "div") && [...n.children].some(c => BLOCK.test(c.tagName))) return toMd(n);
    if (/^h[1-6]$/.test(tag)) return `${"#".repeat(Number(tag[1]))} ${inlineMd(n).trim()}`;
    if (tag === "blockquote") return toMd(n).split("\n").map(l => `> ${l}`.trimEnd()).join("\n");
    if (tag === "pre") return "```\n" + n.textContent.replace(/\n$/, "") + "\n```";
    if (tag === "ul" || tag === "ol") return listMd(n, 0);
    if (tag === "hr") return "---";
    if (tag === "table") {
      const rows = [...n.querySelectorAll("tr")].map(tr => [...tr.children].map(c => inlineMd(c).trim().replace(/\|/g, "\\|")));
      if (!rows.length) return "";
      const w = Math.max(...rows.map(r => r.length));
      const line = r => `| ${Array.from({ length: w }, (_, i) => r[i] ?? "").join(" | ")} |`;
      return [line(rows[0]), line(Array(w).fill("---")), ...rows.slice(1).map(line)].join("\n");
    }
    if (n.classList?.contains("spoiler")) return `>!${inlineMd(n)}!<`;
    return plainLines(inlineMd(n).trim());
  }
  // A paragraph's line that would START a heading, a list or a quote is written so it stays text.
  const plainLines = t =>
    t
      .split("\n")
      .map(l => l.replace(/^(#{1,6}\s|[-*+]\s|>(?!!))/, "\\$1").replace(/^(\d+)\.(\s)/, "$1\\.$2"))
      .join("\n");
  function toMd(root) {
    const blocks = [];
    let inline = document.createElement("span");
    const flush = () => {
      const t = inlineMd(inline).trim();
      if (t) blocks.push(plainLines(t));
      inline = document.createElement("span");
    };
    for (const n of [...root.childNodes]) {
      if (n.nodeType === 1 && (BLOCK.test(n.tagName) || n.classList?.contains("block"))) {
        flush();
        const b = blockMd(n);
        if (b.trim()) blocks.push(b);
      } else inline.append(n.cloneNode(true));
    }
    flush();
    return blocks.join("\n\n");
  }

  function create({ name = "body", value = "", placeholder = "", pick = null, label = "", compact = false, onSubmit = null, suggest = null } = {}) {
    let mode = savedMode();
    const ta = h("textarea", { name, value, placeholder, ariaLabel: label || placeholder || "Text", hidden: true });
    const rich = h("div", { className: "rich cw-md", contentEditable: "true", role: "textbox", ariaMultiLine: "true", ariaLabel: label || placeholder || "Text", hidden: true });
    rich.dataset.placeholder = placeholder;
    const least = compact ? 38 : 110;
    const grow = () => {
      ta.style.height = "auto";
      ta.style.height = `${Math.max(least, ta.scrollHeight + 2)}px`;
    };
    ta.addEventListener("input", grow);
    const byKey = () => new Map((pick?.files() ?? []).map(f => [markdown.keyOf(f), f]));

    // MARKDOWN → RICH: the text rendered, its media as they will show (not editable as text; kept as their Markdown).
    function mediaChip(md, key, alt) {
      const ref = byKey().get(key);
      const chip = h("span", { className: "media", contentEditable: "false" });
      chip.dataset.md = md;
      const small = ref?.preview ?? (ref?.inline && /^image\//.test(ref.type) ? `data:${ref.type};base64,${ref.inline}` : null);
      if (small) chip.append(h("img", { src: small, alt: alt || ref?.name || "" }));
      else chip.append(h("span", { className: "tag", textContent: `${ref ? (kinds.mediaOf(ref)?.icon ?? "📄") : "📄"} ${alt || ref?.name || key}` }));
      return chip;
    }
    // AN ITEM embedded (`![title](item:REF)`: any kind): a chip naming it, kept as its Markdown.
    function itemChip(ref, title) {
      const chip = h("span", { className: "media", contentEditable: "false" }, h("span", { className: "tag", textContent: `🔗 ${title || "an item"}` }));
      chip.dataset.md = `![${String(title ?? "").replace(/[\[\]\n]/g, " ")}](item:${ref})`;
      return chip;
    }
    function toRich(md) {
      rich.innerHTML = markdown.html(md);
      for (const s of rich.querySelectorAll("[data-item]")) s.replaceWith(itemChip(s.dataset.item, s.dataset.alt));
      for (const s of rich.querySelectorAll("[data-file]")) s.replaceWith(mediaChip(`![${s.dataset.alt ?? ""}](file:${s.dataset.file})`, s.dataset.file, s.dataset.alt));
      for (const a of rich.querySelectorAll("[data-file-link]")) {
        const c = h("span", { className: "media", contentEditable: "false", textContent: `📄 ${a.textContent}` });
        c.dataset.md = `[${a.textContent}](file:${a.dataset.fileLink})`;
        a.replaceWith(c);
      }
      for (const cb of rich.querySelectorAll("li.task input")) cb.disabled = false;
    }
    const value_ = () => (mode === "rich" ? toMd(rich) : ta.value);
    function setMode(m) {
      const text = value_();
      mode = m;
      saveMode(m);
      ta.hidden = m !== "markdown";
      rich.hidden = m !== "rich";
      modeBtn.textContent = m === "rich" ? "Markdown" : "Rich text";
      modeBtn.title = m === "rich" ? "Write in Markdown" : "Write in rich text";
      if (m === "rich") toRich(text);
      else ((ta.value = text), requestAnimationFrame(grow));
    }

    // THE SELECTION in rich text, kept while a menu or a dialog takes the focus.
    let saved = null;
    const keep = () => {
      const s = getSelection();
      if (s.rangeCount && rich.contains(s.anchorNode)) saved = s.getRangeAt(0).cloneRange();
    };
    document.addEventListener("selectionchange", () => el.isConnected && keep());
    const restore = () => {
      rich.focus();
      if (saved) {
        const s = getSelection();
        s.removeAllRanges();
        s.addRange(saved);
      }
    };
    const exec = (cmd, arg = null) => (restore(), document.execCommand(cmd, false, arg), keep());
    const wrapIn = (tag, cls = "", hint = "text") => {
      restore();
      const s = getSelection();
      if (!s.rangeCount) return;
      const r = s.getRangeAt(0);
      const e = document.createElement(tag);
      if (cls) e.className = cls;
      e.append(r.collapsed ? document.createTextNode(hint) : r.extractContents());
      r.insertNode(e);
      r.selectNodeContents(e);
      s.removeAllRanges();
      s.addRange(r);
      keep();
    };
    const blockIs = tag => {
      const s = getSelection();
      let n = s.anchorNode;
      while (n && n !== rich) {
        if (n.nodeType === 1 && n.tagName.toLowerCase() === tag) return true;
        n = n.parentNode;
      }
      return false;
    };
    const insertRich = node => {
      restore();
      const s = getSelection();
      if (!s.rangeCount) return rich.append(node);
      const r = s.getRangeAt(0);
      r.deleteContents();
      r.insertNode(node);
      r.setStartAfter(node);
      r.collapse(true);
      s.removeAllRanges();
      s.addRange(r);
      keep();
    };

    // MARKDOWN text operations.
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
    const insertText = t => {
      const [s, e, v] = at();
      const before = s > 0 && v[s - 1] !== "\n" ? "\n" : "";
      const after = v[e] && v[e] !== "\n" ? "\n" : "";
      const text = before + t + after;
      put(text, s, e, text.length, text.length);
    };

    // EACH TOOL, in both modes.
    const both = (r, m) => () => (mode === "rich" ? r() : m());
    const tools = [
      ["H", "Heading", both(() => exec("formatBlock", blockIs("h2") ? "P" : "H2"), () => linePrefix("## "))],
      ["B", "Bold", both(() => exec("bold"), () => surround("**"))],
      ["i", "Italic", both(() => exec("italic"), () => surround("*"))],
      ["S", "Strikethrough", both(() => exec("strikeThrough"), () => surround("~~"))],
      ["x²", "Superscript", both(() => exec("superscript"), () => surround("^(", ")"))],
      ["🙈", "Hidden (a spoiler: shown on a click)", both(() => wrapIn("span", "spoiler", "hidden"), () => surround(">!", "!<", "hidden"))],
      ["`", "Inline code", both(() => wrapIn("code", "", "code"), () => surround("`", "`", "code"))],
      ["```", "Code block", both(() => exec("formatBlock", blockIs("pre") ? "P" : "PRE"), () => surround("```\n", "\n```\n", "code"))],
      [
        "🔗",
        "Link",
        both(
          () => {
            const url = prompt("Link to (https://…)", "https://");
            if (url && /^(https?:\/\/|mailto:|\/)/i.test(url)) exec("createLink", url);
          },
          () => surround("[", "](https://)", "text"),
        ),
      ],
      ["❝", "Quote", both(() => exec("formatBlock", blockIs("blockquote") ? "P" : "BLOCKQUOTE"), () => linePrefix("> "))],
      ["•", "List", both(() => exec("insertUnorderedList"), () => linePrefix("- "))],
      ["1.", "Numbered list", both(() => exec("insertOrderedList"), () => linePrefix(i => `${i + 1}. `))],
      [
        "☑",
        "Task list",
        both(
          () => {
            if (!blockIs("ul")) exec("insertUnorderedList");
            insertRich(h("input", { type: "checkbox" }));
            insertRich(document.createTextNode(" "));
          },
          () => linePrefix("- [ ] "),
        ),
      ],
      ["▦", "Table", both(() => exec("insertHTML", "<table><tr><th>Column</th><th>Column</th></tr><tr><td>&nbsp;</td><td>&nbsp;</td></tr></table><p><br></p>"), () => insertText("| Column | Column |\n| --- | --- |\n| | |"))],
      ["—", "Divider", both(() => exec("insertHorizontalRule"), () => insertText("\n---\n"))],
    ];
    const btn = (label, title, fn) => h("button", { type: "button", textContent: label, title, onmousedown: e => e.preventDefault(), onclick: e => (e.preventDefault(), fn()) });

    // MEDIA: a file picked here goes inline once sent (and one attached with 📎 that is media, too).
    const inlineNext = new Set();
    // A file picked here goes inline once sent.
    const pickedInline = fs => {
      for (const f of fs) inlineNext.add(f);
      pick?.addFiles?.(fs);
    };
    // UPLOAD (Insert's first part): an image, a video, an audio or a book from this device — each kind its own line
    // (`attachments.fileButton`: a label the browser opens the chooser from), published as its kind and placed where
    // the caret is — or one from Drive.
    const uploads = pick
      ? [
          h("div", { className: "s", style: "margin:4px 8px 0", textContent: "Upload — placed here" }),
          ...kinds.media().map(m => attachments.fileButton(`${m.icon} ${m.label}`, { accept: m.accept, onFiles: pickedInline, after: () => (insertMenu.hidden = true) })),
          h("button", { type: "button", textContent: "🗂️ From Drive", onclick: e => (e.preventDefault(), (insertMenu.hidden = true), pick?.fromDrive?.({ media: true })) }),
        ]
      : [];
    pick?.onReady?.((ref, file) => {
      if (!ref || !isMedia(ref)) return;
      if (file && !inlineNext.has(file) && !kinds.mediaOf(file.type)) return;
      inlineNext.delete(file);
      const alt = String(ref.name ?? "").replace(/[\[\]\n]/g, " ");
      const md = `![${alt}](file:${markdown.keyOf(ref)})`;
      if (mode === "rich") insertRich(mediaChip(md, markdown.keyOf(ref), alt));
      else insertText(md);
    });
    // INSERT ANY ITEM (a post, a note, a file, a video…): one of this person's or one they SAVED — the newest, or
    // found by its title — or any by its reference (what Share copies). It shows where it is written (`cards.embed`), read as its reader may.
    const find = h("input", { type: "search", placeholder: "Find yours by title, or paste a reference", style: "width:18em;margin:4px 8px" });
    const found = h("div", {});
    // NEW: an item of any kind made in the one composer (`publisher.dialog`) where this text goes, then put here.
    const makeNew = async domain => {
      insertMenu.hidden = true;
      const w = pick?.where?.() ?? { space: null, public: false };
      const sp = w.space && w.space.kind !== "account" ? w.space : null;
      const done = await (await ctx.require("publisher")).dialog(null, { domain, space: sp, initial: w.public ? "public" : sp ? "members" : "private" }).catch(e => (ctx.log("insert", { what: e.message }), null));
      // Untitled (a note): named by its kind.
      if (done?.item) putItem(done.item, done.title || (kinds.media().find(m => m.domain === domain)?.label ?? kinds.of(kinds.inDomain(domain)[0])?.label));
    };
    // NEW (what is written, not uploaded: a post, a note) — an upload is Upload's.
    const newRow = h(
      "div",
      { style: "display:flex;flex-wrap:wrap;gap:4px;margin:4px 8px" },
      h("span", { className: "s", textContent: "New:" }),
      ...kinds.written().map(d => btn(kinds.of(kinds.inDomain(d)[0]).label, `A new ${kinds.of(kinds.inDomain(d)[0]).label.toLowerCase()}, put here`, () => makeNew(d))),
    );
    // INSERT, the one menu: Upload · New · Find (yours, saved, or by reference).
    const insertMenu = h("span", { className: "cw-att-menu", hidden: true }, ...uploads, newRow, find, found);
    let mine = null;
    const putItem = (ref, title) => {
      insertMenu.hidden = true;
      find.value = "";
      if (mode === "rich") insertRich(itemChip(ref, title));
      else insertText(`![${String(title ?? "").replace(/[\[\]\n]/g, " ")}](item:${ref})`);
    };
    // Only the NEWEST lookup draws (the list loading on opening must not draw over what was typed since).
    let asked = 0;
    const listFound = async () => {
      const n = ++asked;
      const items = await ctx.require("items");
      mine ??= (async () => {
        const [own, saved] = await Promise.all([
          items.list({ by: (await (await ctx.require("space")).account()).id }, "new", kinds.all(), { window: "all" }).catch(() => []),
          ctx.require("actions").then(a => Promise.all(a.saved().slice(0, 50).map(r => items.get(r).catch(() => null)))).catch(() => []),
        ]);
        const seen = new Set();
        return [...saved.filter(Boolean), ...own].filter(it => !seen.has(it.ref) && seen.add(it.ref));
      })();
      const q = find.value.trim();
      // A REFERENCE or a shared LINK (`items.refOf`): the item read first (as this person may), so it is named — or said
      // not to be readable.
      const named = items.refOf(q);
      if (named) {
        const it = await items.get(named).catch(() => null);
        if (n !== asked) return;
        return found.replaceChildren(it ? btn(`🔗 ${kinds.of(it.kind)?.label ?? it.kind}: ${it.title || (it.body ?? "").slice(0, 40) || "untitled"}`, "Insert it", () => putItem(it.ref, it.title)) : h("span", { className: "s", textContent: "That item is not there, or not yours to read." }));
      }
      const all = await mine;
      if (n !== asked) return;
      const list = all.filter(it => !q || (it.title ?? "").toLowerCase().includes(q.toLowerCase())).slice(0, 8);
      found.replaceChildren(...(list.length ? list.map(it => btn(`${kinds.of(it.kind)?.label ?? it.kind}: ${it.title || (it.body ?? "").slice(0, 40) || "untitled"}`, "Insert it", () => putItem(it.ref, it.title))) : [h("span", { className: "s", textContent: q ? "None found." : "Nothing of yours yet." })]));
    };
    find.oninput = () => listFound();
    find.onkeydown = e => e.key === "Escape" && (insertMenu.hidden = true);
    const insertGroup = h("span", { className: "cw-att-pick" }, btn("＋ Insert", "Insert: upload an image, a video, an audio or a book; a new post or note; or any item — yours, or by its reference", () => (mode === "rich" && keep(), (insertMenu.hidden = !insertMenu.hidden), insertMenu.hidden || (attachments.fitMenu(insertMenu), listFound(), find.focus()))), insertMenu);
    const modeBtn = h("button", { type: "button", className: "mode", onclick: e => (e.preventDefault(), setMode(mode === "rich" ? "markdown" : "rich")) });
    // FILES attached below (📎: any file, listed under what is written — not placed in it; on a chat line beside Aa).
    const mediaGroup = pick ? [pick.el] : [];
    const bar = h(
      "div",
      { className: "bar", hidden: compact },
      ...tools.map(([l, t, f]) => btn(l, t, f)),
      ...(compact ? [] : mediaGroup),
      ...(compact ? [] : [insertGroup]),
      h("span", { className: "sp" }),
      modeBtn,
    );
    // COMPACT: the formatting behind "Aa".
    const aa = compact ? h("button", { type: "button", className: "aa", textContent: "Aa", title: "Formatting", onclick: e => (e.preventDefault(), (bar.hidden = !bar.hidden), aa.classList.toggle("on", !bar.hidden)) }) : null;

    // MENTIONS: "@" and the start of a name → names to pick.
    const list = h("ul", { className: "suggest", hidden: true });
    const people = suggest ?? (q => ctx.require("person").then(p => p.mentionable(q)));
    async function suggestNow() {
      let before = "";
      if (mode === "markdown") before = ta.value.slice(0, ta.selectionStart);
      else {
        const s = getSelection();
        if (s.rangeCount && s.anchorNode?.nodeType === 3 && rich.contains(s.anchorNode)) before = s.anchorNode.textContent.slice(0, s.anchorOffset);
      }
      const m = before.match(/@([^\s@]*)$/);
      if (!m) return (list.hidden = true);
      const found = (await people(m[1]).catch(() => [])).slice(0, 6);
      list.replaceChildren(...found.map(p => h("li", { textContent: p.shown, onmousedown: e => (e.preventDefault(), take(p, m[0].length)) })));
      list.hidden = !found.length;
    }
    function take({ did, shown }, typed) {
      list.hidden = true;
      if (mode === "markdown") {
        const s = ta.selectionStart;
        const md = `[@${shown}](person:${did}) `;
        put(md, s - typed, s, md.length, md.length);
        return;
      }
      // Rich: the mention as a link (written back as `[@name](person:DID)`), the caret after it.
      const s = getSelection();
      const n = s.anchorNode;
      const o = s.anchorOffset;
      const after = n.splitText(o - typed);
      after.textContent = after.textContent.slice(typed);
      const a = h("a", { href: `person:${did}`, textContent: `@${shown}` });
      const space = document.createTextNode("\u00a0");
      after.before(a, space);
      const r = document.createRange();
      r.setStart(space, 1);
      r.collapse(true);
      s.removeAllRanges();
      s.addRange(r);
      rich.dispatchEvent(new Event("input"));
    }
    ta.addEventListener("input", suggestNow);
    rich.addEventListener("input", suggestNow);
    const blurList = () => setTimeout(() => (list.hidden = true), 150);
    ta.addEventListener("blur", blurList);
    rich.addEventListener("blur", blurList);

    // ENTER sends (compact), Shift+Enter a new line; a task box toggles by a click.
    const onKey = e => {
      if (onSubmit && e.key === "Enter" && !e.shiftKey && !e.isComposing) {
        if (!list.hidden && list.firstChild) {
          e.preventDefault();
          return list.firstChild.dispatchEvent(new MouseEvent("mousedown"));
        }
        e.preventDefault();
        onSubmit();
      }
    };
    ta.addEventListener("keydown", onKey);
    rich.addEventListener("keydown", onKey);
    // Pasted text comes in as text (no page's styles carried along).
    rich.addEventListener("paste", e => {
      const t = e.clipboardData?.getData("text/plain");
      if (t == null) return;
      e.preventDefault();
      document.execCommand("insertText", false, t);
    });
    document.execCommand("defaultParagraphSeparator", false, "p");

    // The compact line (a chat line): Aa, 🖼, 📎 and ＋ (Insert: any item — a note, a post, a file, a video… — or a new
    // one) beside the text: the same tools as every editor.
    if (compact) insertGroup.firstChild.textContent = "＋";
    const el = compact
      ? h("div", { className: "cw-mde compact" }, list, bar, h("div", { className: "line-in" }, h("span", { className: "tools-in" }, aa, ...mediaGroup, insertGroup), ta, rich))
      : h("div", { className: "cw-mde" }, list, bar, ta, rich);
    document.addEventListener("pointerdown", e => {
      if (!el.isConnected) return;
      // Insert's menu closes on a click anywhere outside it (and its button).
      if (!insertMenu.hidden && !insertMenu.parentElement.contains(e.target)) insertMenu.hidden = true;
    });
    setMode(mode);
    if (mode === "rich") toRich(value);
    else ((ta.value = value), requestAnimationFrame(grow));
    return {
      el,
      textarea: ta,
      value: () => value_().trim(),
      set: md => (mode === "rich" ? toRich(md) : ((ta.value = md), grow())),
      files: () => pick?.files() ?? [],
      busy: () => !!pick?.busy(),
      focus: () => (mode === "rich" ? rich.focus() : ta.focus()),
      clear: () => {
        ta.value = "";
        rich.replaceChildren();
        grow();
        pick?.clear();
      },
      disable: (off, why = "") => {
        ta.disabled = off;
        rich.contentEditable = off ? "false" : "true";
        const ph = off ? why : placeholder;
        ta.placeholder = ph;
        rich.dataset.placeholder = ph;
      },
    };
  }
  return { create, toMd };
}
