// MARKDOWN, a service: the ONE way text a person wrote is shown — posts, comments (Grid's renderer: small, no
// dependency, XSS-safe). The source is HTML-escaped FIRST, so the only markup in the output is what these rules make;
// link targets are kept to http(s), relative and mailto. Headings, **bold**, *italic*, ~~strike~~, `code`, fenced code,
// > quotes, lists (nested), tables, --- rules, ||spoilers||, ^superscript, links and bare URLs.
//
// FILES INLINE: `![name](file:KEY)` shows the item's own file KEY (an image, a video, an audio) where it is written;
// `[name](file:KEY)` is a download link. KEY names a file among the ITEM's `files` (`keyOf`), so a file shows only
// to who reads the item — its access is the item's. An image shows its preview at once and loads whole once it comes
// into view; a video or an audio shows only its COVER (poster, ▶, length) and loads when played (a video streams).
//
//   const markdown = await ctx.require("markdown");
//   host.append(markdown.render(item.body, item.files))   // an element (class cw-md)
//   markdown.keyOf(ref)                                   // a file's key in `file:KEY`
//   markdown.inlined(body)                                // the keys written inline (the rest show as attachments)
//   markdown.plain(body)                                  // a one-line preview, no markup
export async function start(ctx) {
  const style = document.createElement("style");
  style.textContent = `
    .cw-md { overflow-wrap: anywhere; line-height: 1.5; }
    .cw-md > :first-child { margin-top: 0; } .cw-md > :last-child { margin-bottom: 0; }
    .cw-md p { margin: 0 0 0.6em; }
    .cw-md h1, .cw-md h2, .cw-md h3, .cw-md h4, .cw-md h5, .cw-md h6 { margin: 0.8em 0 0.4em; line-height: 1.25; }
    .cw-md h1 { font-size: 1.4em; } .cw-md h2 { font-size: 1.25em; } .cw-md h3 { font-size: 1.1em; }
    .cw-md code { font-family: var(--cw-mono, ui-monospace, monospace); font-size: 0.9em; background: var(--cw-surface); border: 1px solid var(--cw-line); border-radius: 4px; padding: 0 4px; }
    .cw-md pre { background: var(--cw-surface); border: 1px solid var(--cw-line); border-radius: var(--cw-radius-sm); padding: 8px 10px; overflow-x: auto; }
    .cw-md pre code { border: 0; padding: 0; background: none; }
    .cw-md blockquote { margin: 0 0 0.6em; padding-left: 10px; border-left: 3px solid var(--cw-line); color: var(--cw-muted); }
    .cw-md ul, .cw-md ol { margin: 0 0 0.6em; padding-left: 1.4em; }
    .cw-md table { border-collapse: collapse; margin: 0 0 0.6em; display: block; overflow-x: auto; }
    .cw-md th, .cw-md td { border: 1px solid var(--cw-line); padding: 4px 8px; }
    .cw-md hr { border: 0; border-top: 1px solid var(--cw-line); margin: 0.8em 0; }
    .cw-md a { color: var(--cw-accent); }
    .cw-md .spoiler { background: var(--cw-fg); color: transparent; border-radius: 3px; cursor: pointer; }
    .cw-md .spoiler.shown { background: none; color: inherit; }
    .cw-md .spoiler.block { display: inline-block; padding: 2px 6px; }
    .cw-md li.task { list-style: none; margin-left: -1.2em; }
    .cw-md sup { font-size: 0.75em; }
    .cw-md .cw-md-media { display: block; max-width: min(560px, 100%); margin: 6px 0; border-radius: var(--cw-radius-sm); border: 1px solid var(--cw-line); background: var(--cw-surface); }
    .cw-md img.cw-md-media { max-height: 480px; cursor: zoom-in; }
    .cw-md audio.cw-md-media { width: min(560px, 100%); border: 0; background: none; }
    .cw-md .cw-md-cover { position: relative; padding: 0; cursor: pointer; aspect-ratio: 16 / 9; width: min(560px, 100%); overflow: hidden; color: #fff; background: #111; }
    .cw-md .cw-md-cover.audio { aspect-ratio: 1; width: min(240px, 100%); }
    .cw-md .cw-md-album { width: min(240px, 100%); aspect-ratio: 1; object-fit: cover; }
    .cw-md .cw-md-line { margin: 4px 0 0; white-space: pre-wrap; font-weight: 600; }
    .cw-md .cw-md-cover img { width: 100%; height: 100%; object-fit: cover; display: block; }
    .cw-md .cw-md-cover .ic { font-size: 2rem; position: absolute; inset: 0; display: grid; place-items: center; opacity: 0.6; }
    .cw-md .cw-md-cover .play { position: absolute; left: 50%; top: 50%; transform: translate(-50%, -50%); width: 56px; height: 40px; border-radius: 12px;
      background: rgba(0, 0, 0, 0.7); display: grid; place-items: center; font-size: 18px; }
    .cw-md .cw-md-cover:hover .play { background: #e00; }
    .cw-md .cw-md-cover .cap { position: absolute; left: 0; right: 0; bottom: 0; padding: 4px 8px; font-size: 12px; text-align: left;
      background: linear-gradient(transparent, rgba(0, 0, 0, 0.7)); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .cw-md .cw-md-missing { color: var(--cw-muted); font-size: var(--cw-text-sm); }`;
  document.head.append(style);

  const NUL = String.fromCharCode(0);
  const esc = s =>
    String(s ?? "")
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#39;");
  const href = u => ((u = (u || "").trim()), /^(https?:\/\/|\/|\?|#|mailto:)/i.test(u) ? u : "#");
  // A URL may hold one level of balanced parens (https://en.wikipedia.org/wiki/Freenet_(software)).
  const URL_RE = "((?:[^()\\s]|\\([^()\\s]*\\))*)";
  const IMG_RE = new RegExp("!\\[([^\\]]*)\\]\\(" + URL_RE + "\\)", "g");
  const LINK_RE = new RegExp("\\[([^\\]]+)\\]\\(" + URL_RE + "\\)", "g");
  const FILE = /^file:([A-Za-z0-9_-]{1,80})$/;

  // A backslash keeps the next mark as it is (`\*` shows a star).
  const ESC = String.fromCharCode(1);
  const inline = s => {
    const codes = [];
    const kept = [];
    s = s.replace(/\\([\\`*_{}\[\]()#+\-.!|~^]|&gt;|&lt;)/g, (_, c) => (kept.push(c), ESC + (kept.length - 1) + ESC));
    s = s.replace(/`([^`]+)`/g, (_, c) => (codes.push(c), NUL + (codes.length - 1) + NUL));
    s = s.replace(IMG_RE, (_, a, u) => (FILE.test(u) ? `<span data-file="${FILE.exec(u)[1]}" data-alt="${a}"></span>` : `<img src="${href(u)}" alt="${a}" class="cw-md-media">`));
    s = s.replace(LINK_RE, (_, t, u) => (FILE.test(u) ? `<a data-file-link="${FILE.exec(u)[1]}" href="#">${t}</a>` : `<a href="${href(u)}" target="_blank" rel="noopener">${t}</a>`));
    s = s.replace(/(^|[\s(])((?:https?:\/\/)[^\s<)]+)/g, (_, pre, u) => `${pre}<a href="${href(u)}" target="_blank" rel="noopener">${u}</a>`);
    s = s
      .replace(/\*\*\*([^*]+)\*\*\*/g, "<strong><em>$1</em></strong>")
      .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
      .replace(/(^|[^\w])__([^_]+)__(?!\w)/g, "$1<strong>$2</strong>")
      .replace(/(^|[^*])\*([^*\s][^*]*)\*/g, "$1<em>$2</em>")
      .replace(/(^|[^\w])_([^_\s][^_]*)_(?!\w)/g, "$1<em>$2</em>")
      .replace(/~~([^~]+)~~/g, "<del>$1</del>");
    // HIDDEN (a spoiler): Reddit's >!text!<, and ||text||.
    s = s.replace(/&gt;!(.+?)!&lt;/g, '<span class="spoiler" title="Reveal">$1</span>').replace(/\|\|([^|]+)\|\|/g, '<span class="spoiler" title="Reveal">$1</span>');
    s = s.replace(/\^\(([^)]+)\)/g, "<sup>$1</sup>").replace(/\^([^\s^]+)/g, "<sup>$1</sup>");
    return s.replace(new RegExp(NUL + "([0-9]+)" + NUL, "g"), (_, i) => `<code>${codes[+i]}</code>`).replace(new RegExp(ESC + "([0-9]+)" + ESC, "g"), (_, i) => kept[+i]);
  };
  const cells = r =>
    r
      .trim()
      .replace(/^\||\|$/g, "")
      .split(/(?<!\\)\|/)
      .map(c => c.replace(/\\\|/g, "|").trim());
  const list = items => {
    let out = "";
    const stack = [];
    const open = ordered => ((out += ordered ? "<ol>" : "<ul>"), stack.push(ordered));
    const close = () => (out += stack.pop() ? "</ol>" : "</ul>");
    for (const it of items) {
      while (stack.length > it.level + 1) close();
      if (stack.length < it.level + 1) open(it.ordered);
      else if (stack.length && stack.at(-1) !== it.ordered && stack.length === it.level + 1) (close(), open(it.ordered));
      const task = /^\[( |x|X)\]\s+(.*)$/.exec(it.text);
      out += task ? `<li class="task"><input type="checkbox" disabled${task[1] === " " ? "" : " checked"}> ${inline(task[2])}</li>` : `<li>${inline(it.text)}</li>`;
    }
    while (stack.length) close();
    return out;
  };
  const blocks = lines => {
    const out = [];
    let i = 0;
    let para = [];
    let items = null;
    const flushPara = () => para.length && (out.push("<p>" + para.map(inline).join("<br>") + "</p>"), (para = []));
    const flushList = () => items && (out.push(list(items)), (items = null));
    const flush = () => (flushPara(), flushList());
    while (i < lines.length) {
      const ln = lines[i];
      if (/^```/.test(ln)) {
        flush();
        i++;
        const buf = [];
        while (i < lines.length && !/^```/.test(lines[i])) buf.push(lines[i++]);
        i++;
        out.push("<pre><code>" + buf.join("\n") + "</code></pre>");
        continue;
      }
      const hd = ln.match(/^(#{1,6})\s+(.*)$/);
      if (hd) {
        flush();
        out.push(`<h${hd[1].length}>${inline(hd[2])}</h${hd[1].length}>`);
        i++;
        continue;
      }
      // A HIDDEN paragraph: lines that start with >! (Reddit's), to its closing !< or its end.
      if (/^&gt;!/.test(ln) && !/!&lt;/.test(ln.slice(5))) {
        flush();
        const buf = [ln.slice(5)];
        i++;
        while (i < lines.length && lines[i].trim() && !/!&lt;\s*$/.test(buf.at(-1))) buf.push(lines[i++]);
        out.push(`<p><span class="spoiler block" title="Reveal">${buf.map(l => inline(l.replace(/!&lt;\s*$/, ""))).join("<br>")}</span></p>`);
        continue;
      }
      if (/^&gt;(?!!)\s?/.test(ln)) {
        flush();
        const buf = [];
        while (i < lines.length && /^&gt;(?!!)\s?/.test(lines[i])) buf.push(lines[i++].replace(/^&gt;\s?/, ""));
        out.push("<blockquote>" + blocks(buf) + "</blockquote>");
        continue;
      }
      if (/^\s*(---+|\*\*\*+|___+)\s*$/.test(ln)) {
        flush();
        out.push("<hr>");
        i++;
        continue;
      }
      if (/\|/.test(ln) && i + 1 < lines.length && /-/.test(lines[i + 1]) && /^\s*\|?[\s:|-]+\|[\s:|-]*$/.test(lines[i + 1])) {
        flush();
        const head = cells(ln);
        i += 2;
        const rows = [];
        while (i < lines.length && lines[i].trim() && /\|/.test(lines[i])) rows.push(cells(lines[i++]));
        out.push(
          "<table><thead><tr>" + head.map(c => `<th>${inline(c)}</th>`).join("") + "</tr></thead><tbody>" + rows.map(r => "<tr>" + head.map((_, j) => `<td>${inline(r[j] || "")}</td>`).join("") + "</tr>").join("") + "</tbody></table>",
        );
        continue;
      }
      const m = ln.match(/^(\s*)([-*+]|\d+\.)\s+(.*)$/);
      if (m) {
        flushPara();
        (items ??= []).push({ level: Math.floor(m[1].replace(/\t/g, "  ").length / 2), ordered: /\d/.test(m[2]), text: m[3] });
        i++;
        continue;
      }
      if (!ln.trim()) {
        flush();
        i++;
        continue;
      }
      flushList();
      para.push(ln);
      i++;
    }
    flush();
    return out.join("");
  };
  const html = src => blocks(esc(src).split("\n"));

  // A FILE's key in `file:KEY`: its id (its first root), else — a small file riding inline — a hash of its bytes.
  const fnv = s => {
    let x = 0x811c9dc5;
    for (let i = 0; i < s.length; i++) x = Math.imul(x ^ s.charCodeAt(i), 0x01000193) >>> 0;
    return x.toString(36);
  };
  const keyOf = ref => (ref?.id ? String(ref.id).slice(0, 32) : ref?.root ? String(ref.root).slice(0, 32) : `i${fnv(String(ref?.inline ?? ""))}${(ref?.size ?? 0).toString(36)}`);
  const inlined = body => new Set([...String(body ?? "").matchAll(/\]\(file:([A-Za-z0-9_-]{1,80})\)/g)].map(m => m[1]));
  const MANIFEST = "application/vnd.craftworks.video+json";
  const kindOf = ref => (/^image\//.test(ref?.type) ? "image" : ref?.type === MANIFEST ? (ref.audio ? "audio" : "video") : /^video\//.test(ref?.type) ? "video" : /^audio\//.test(ref?.type) ? "audio" : "file");

  // The media, loaded when wanted: an image once in view, a video or an audio when played.
  const seen = typeof IntersectionObserver !== "undefined" ? new IntersectionObserver((es, io) => es.forEach(e => e.isIntersecting && (io.unobserve(e.target), e.target._load?.())), { rootMargin: "300px" }) : null;
  const whenSeen = (el, load) => (seen ? ((el._load = load), seen.observe(el)) : load());
  const urls = new Map(); // a file's key → its object URL (read once a page)
  const urlOf = async (key, ref) => {
    if (!urls.has(key)) urls.set(key, (async () => URL.createObjectURL(await (await ctx.require("files")).get(ref)))().catch(e => (urls.delete(key), Promise.reject(e))));
    return urls.get(key);
  };
  function media(key, ref, alt) {
    const kind = kindOf(ref);
    if (kind === "image") {
      const img = Object.assign(document.createElement("img"), { className: "cw-md-media", alt: alt || ref.name || "", title: ref.name || "" });
      const small = ref.preview ?? (ref.inline ? `data:${ref.type};base64,${ref.inline}` : null);
      if (small) img.src = small;
      if (!ref.inline) whenSeen(img, () => urlOf(key, ref).then(u => (img.src = u), () => {}));
      img.onclick = () => img.src && window.open(img.src, "_blank", "noopener");
      return img;
    }
    if (kind === "video" || kind === "audio") {
      // A COVER until played (as Videos, as YouTube): its poster (or an icon), ▶ and its length — nothing loads. A
      // click puts the player in its place and plays (a video streams: `video-player`).
      const cover = document.createElement("button");
      cover.type = "button";
      cover.className = `cw-md-cover cw-md-media${kind === "audio" ? " audio" : ""}`;
      cover.title = `Play ${ref.name || kind}`;
      if (ref.preview) cover.append(Object.assign(document.createElement("img"), { src: ref.preview, alt: alt || ref.name || "" }));
      else cover.append(Object.assign(document.createElement("span"), { className: "ic", textContent: kind === "audio" ? "🎵" : "🎬" }));
      cover.append(Object.assign(document.createElement("span"), { className: "play", textContent: "▶" }));
      const len = ref.duration ? `${Math.floor(ref.duration / 60)}:${String(Math.floor(ref.duration % 60)).padStart(2, "0")}` : "";
      cover.append(Object.assign(document.createElement("span"), { className: "cap", textContent: [ref.name, len].filter(Boolean).join(" · ") }));
      cover.onclick = async e => {
        e.stopPropagation();
        const el = Object.assign(document.createElement(kind), { className: "cw-md-media", controls: true, autoplay: true, playsInline: true });
        if (ref.preview && kind === "video") el.poster = ref.preview;
        // AUDIO keeps its cover above the player, and the line playing (its lyrics, its transcript) below.
        const box = document.createElement("div");
        const line = Object.assign(document.createElement("p"), { className: "cw-md-line", hidden: true });
        if (kind === "audio" && ref.preview) box.append(Object.assign(document.createElement("img"), { className: "cw-md-media cw-md-album", src: ref.preview, alt: ref.name || "" }));
        box.append(el, line);
        cover.replaceWith(box);
        try {
          // Streamed (a manifest adaptively, a plain file by its byte ranges), else read whole: `video-player`.
          if (!ref.inline) await (await ctx.require("video-player")).play(el, ref);
          else ((el.src = await urlOf(key, ref)), await el.play().catch(() => {}));
        } catch (err) {
          return box.replaceWith(Object.assign(document.createElement("span"), { className: "cw-md-missing", textContent: `${ref.name}: ${err.message ?? err}` }));
        }
        // Its SUBTITLES, lyrics or transcript (`subtitle-store`, by the file's video id): tracks of the player.
        const tracks = await (await ctx.require("subtitle-store")).forFile(ref).catch(() => []);
        tracks.forEach((t, i) => el.append(Object.assign(document.createElement("track"), { kind: "subtitles", label: t.label || t.lang || "Subtitles", srclang: t.lang || "und", default: i === 0, src: URL.createObjectURL(new Blob([t.text], { type: "text/vtt" })) })));
        if (kind === "audio" && el.textTracks.length) {
          const tt = el.textTracks[0];
          tt.mode = "hidden";
          tt.oncuechange = () => {
            const text = [...(tt.activeCues ?? [])].map(c => c.text).join("\n");
            line.hidden = !text;
            line.textContent = text;
          };
        }
      };
      return cover;
    }
    return link(key, ref, ref.name);
  }
  function link(key, ref, text) {
    const a = Object.assign(document.createElement("a"), { href: "#", textContent: text || ref.name || "file" });
    a.onclick = async e => {
      e.preventDefault();
      const u = await urlOf(key, ref).catch(() => null);
      if (!u) return;
      const d = Object.assign(document.createElement("a"), { href: u, download: ref.name || "file" });
      document.body.append(d);
      d.click();
      d.remove();
    };
    return a;
  }

  function render(body, files = []) {
    const el = document.createElement("div");
    el.className = "cw-md";
    el.innerHTML = html(body);
    const byKey = new Map((files ?? []).map(f => [keyOf(f), f]));
    const missing = key => Object.assign(document.createElement("span"), { className: "cw-md-missing", textContent: `(a file not attached here: ${key})` });
    for (const s of el.querySelectorAll("[data-file]")) {
      const ref = byKey.get(s.dataset.file);
      s.replaceWith(ref ? media(s.dataset.file, ref, s.dataset.alt) : missing(s.dataset.file));
    }
    for (const a of el.querySelectorAll("[data-file-link]")) {
      const ref = byKey.get(a.dataset.fileLink);
      a.replaceWith(ref ? link(a.dataset.fileLink, ref, a.textContent) : missing(a.dataset.fileLink));
    }
    for (const s of el.querySelectorAll(".spoiler")) s.onclick = e => (e.stopPropagation(), s.classList.toggle("shown"));
    // A link inside a card that opens on a click: it opens itself, not the card.
    for (const a of el.querySelectorAll("a, video, audio")) a.addEventListener("click", e => e.stopPropagation());
    return el;
  }

  const plain = body =>
    String(body ?? "")
      .replace(/!\[[^\]]*\]\([^)]*\)/g, " ")
      .replace(/```[\s\S]*?```/g, " ")
      .replace(/`([^`]+)`/g, "$1")
      .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
      .replace(/^\s{0,3}#{1,6}\s+/gm, "")
      .replace(/^\s*>\s?/gm, "")
      .replace(/^\s*([-*+]|\d+\.)\s+/gm, "")
      .replace(/\*\*([^*]+)\*\*/g, "$1")
      .replace(/\*([^*]+)\*/g, "$1")
      .replace(/~~([^~]+)~~/g, "$1")
      .replace(/\|\|([^|]+)\|\|/g, "$1")
      .replace(/\s+/g, " ")
      .trim();

  return { render, html, keyOf, inlined, kindOf, plain };
}
