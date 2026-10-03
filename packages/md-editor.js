// MD EDITOR, a component: the one editor for what a person writes — a post, a comment, a message, an edit (Reddit's:
// RICH TEXT by default, Markdown as the alternative). What is KEPT is always Markdown (`markdown` shows it).
// - RICH: what is written shows as it will read — the toolbar formats the selection (heading, bold, italic, strike,
//   superscript, hidden, code, link, quote, lists, a task list, a table, a divider), media shows as itself.
// - MARKDOWN: the text as it is kept (rich text shows it as it reads). Switching carries the text across (the person's choice is kept).
// MEDIA INLINE: 🖼 — an image, a video or an audio, from this device or Drive (the attachments picker's: sent with its
// progress; from Drive, taken into the item's space) — shows where the cursor was, kept as `![name](file:KEY)`; 📎
// attaches any file below. The item keeps the files (`files()`): who reads it reads them.
// COMPACT (a chat line): the toolbar behind "Aa", Enter sends (`onSubmit`), Shift+Enter a new line. `suggest`:
// after "@", names to pick (mentions).
//
//   const ed = (await ctx.require("md-editor")).create({ value, placeholder, pick, compact, onSubmit, suggest })
//   form.append(ed.el)   ed.value()   ed.set(md)   ed.files()   ed.busy()   ed.focus()   ed.clear()   ed.disable(bool)
export async function start(ctx) {
  const markdown = await ctx.require("markdown");
  const style = document.createElement("style");
  style.textContent = `
    .cw-mde { display: grid; gap: 4px; position: relative; }
    .cw-mde .bar { display: flex; flex-wrap: wrap; gap: 3px; align-items: center; }
    .cw-mde .bar[hidden] { display: none; }
    .cw-mde .bar button, .cw-mde .aa { font: inherit; font-size: 13px; font-weight: 700; min-width: 28px; line-height: 1.4; padding: 2px 7px; cursor: pointer;
      border: 1px solid var(--cw-line); border-radius: var(--cw-radius-sm); background: var(--cw-surface); color: var(--cw-fg); }
    .cw-mde .bar button:hover, .cw-mde .bar button.on, .cw-mde .aa.on { border-color: var(--cw-accent); color: var(--cw-accent); }
    .cw-mde .bar .sp { flex: 1; }
    .cw-mde .bar .mode { font-weight: 400; }
    .cw-mde .bar .cw-att-menu button { border: 0; font-weight: 400; min-width: 0; font-size: inherit; width: 100%; text-align: left; padding: 6px 10px; white-space: nowrap; }
    .cw-mde .bar .cw-att-menu button:hover { background: var(--cw-hover); color: var(--cw-fg); }
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
    .cw-mde .tools-in button { font: inherit; font-size: 13px; min-width: 28px; line-height: 1.4; padding: 2px 6px; cursor: pointer; border: 1px solid var(--cw-line);
      border-radius: var(--cw-radius-sm); background: var(--cw-surface); color: var(--cw-fg); }
    .cw-mde .tools-in .cw-att-menu { bottom: 100%; top: auto; }
    .cw-mde .tools-in .cw-att-menu button { border: 0; width: 100%; text-align: left; padding: 6px 10px; white-space: nowrap; }
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
  const MEDIA = /^(image|video|audio)\//;
  const isMedia = ref => MEDIA.test(ref?.type ?? "") || ref?.type === "application/vnd.craftworks.video+json";
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
      else chip.append(h("span", { className: "tag", textContent: `${ref ? (markdown.kindOf(ref) === "audio" ? "🎵" : markdown.kindOf(ref) === "video" ? "🎬" : "🖼") : "📄"} ${alt || ref?.name || key}` }));
      return chip;
    }
    function toRich(md) {
      rich.innerHTML = markdown.html(md);
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
    const mediaIn = h("input", { type: "file", accept: "image/*,video/*,audio/*", multiple: true, hidden: true });
    const inlineNext = new Set();
    const mediaMenu = h(
      "span",
      { className: "cw-att-menu", hidden: true },
      // Each kind its own line (the same picker: what it accepts narrowed), so audio is as plain to find as an image.
      ...[["🖼 Image", "image/*"], ["🎬 Video", "video/*"], ["🎵 Audio", "audio/*"]].map(([textContent, accept]) =>
        h("button", { type: "button", textContent, onclick: e => (e.preventDefault(), (mediaMenu.hidden = true), (mediaIn.accept = accept), mediaIn.click()) }),
      ),
      h("button", { type: "button", textContent: "From Drive", onclick: e => (e.preventDefault(), (mediaMenu.hidden = true), pick?.fromDrive?.({ media: true })) }),
    );
    mediaIn.onchange = () => {
      for (const f of mediaIn.files) inlineNext.add(f);
      pick?.addFiles?.([...mediaIn.files]);
      mediaIn.value = "";
    };
    pick?.onReady?.((ref, file) => {
      if (!ref || !isMedia(ref)) return;
      if (file && !inlineNext.has(file) && !MEDIA.test(file.type)) return;
      inlineNext.delete(file);
      const alt = String(ref.name ?? "").replace(/[\[\]\n]/g, " ");
      const md = `![${alt}](file:${markdown.keyOf(ref)})`;
      if (mode === "rich") insertRich(mediaChip(md, markdown.keyOf(ref), alt));
      else insertText(md);
    });
    const modeBtn = h("button", { type: "button", className: "mode", onclick: e => (e.preventDefault(), setMode(mode === "rich" ? "markdown" : "rich")) });
    // MEDIA and FILES side by side: 🖼 inline, 📎 attached below (on a chat line: always shown, beside Aa).
    const mediaGroup = pick ? [h("span", { className: "cw-att-pick" }, btn("🖼 Media", "An image, a video or an audio, inline: from this device or from Drive", () => (mediaMenu.hidden = !mediaMenu.hidden)), mediaMenu), pick.el] : [];
    const bar = h(
      "div",
      { className: "bar", hidden: compact },
      ...tools.map(([l, t, f]) => btn(l, t, f)),
      ...(compact ? [] : mediaGroup),
      h("span", { className: "sp" }),
      modeBtn,
    );
    // COMPACT: the formatting behind "Aa".
    const aa = compact ? h("button", { type: "button", className: "aa", textContent: "Aa", title: "Formatting", onclick: e => (e.preventDefault(), (bar.hidden = !bar.hidden), aa.classList.toggle("on", !bar.hidden)) }) : null;

    // MENTIONS: "@" and the start of a name → names to pick.
    const list = h("ul", { className: "suggest", hidden: true });
    async function suggestNow() {
      if (!suggest) return;
      let before = "";
      if (mode === "markdown") before = ta.value.slice(0, ta.selectionStart);
      else {
        const s = getSelection();
        if (s.rangeCount && s.anchorNode?.nodeType === 3 && rich.contains(s.anchorNode)) before = s.anchorNode.textContent.slice(0, s.anchorOffset);
      }
      const m = before.match(/@([^\s@]*)$/);
      if (!m) return (list.hidden = true);
      const found = (await suggest(m[1])).slice(0, 6);
      list.replaceChildren(...found.map(name => h("li", { textContent: name, onmousedown: e => (e.preventDefault(), take(name, m[0].length)) })));
      list.hidden = !found.length;
    }
    function take(name, typed) {
      list.hidden = true;
      if (mode === "markdown") {
        const s = ta.selectionStart;
        put(`@${name} `, s - typed, s, name.length + 2, name.length + 2);
        return;
      }
      const s = getSelection();
      const n = s.anchorNode;
      const o = s.anchorOffset;
      n.textContent = n.textContent.slice(0, o - typed) + `@${name}\u00a0` + n.textContent.slice(o);
      const r = document.createRange();
      r.setStart(n, o - typed + name.length + 2);
      r.collapse(true);
      s.removeAllRanges();
      s.addRange(r);
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

    // The compact line: Aa, 🖼 and 📎 beside the text.
    const el = compact
      ? h("div", { className: "cw-mde compact" }, list, bar, h("div", { className: "line-in" }, h("span", { className: "tools-in" }, aa, ...mediaGroup), ta, rich), mediaIn)
      : h("div", { className: "cw-mde" }, list, bar, ta, rich, mediaIn);
    document.addEventListener("pointerdown", e => {
      if (!el.isConnected) return;
      if (!mediaMenu.hidden && !mediaMenu.parentElement.contains(e.target)) mediaMenu.hidden = true;
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
