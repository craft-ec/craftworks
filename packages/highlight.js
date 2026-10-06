// HIGHLIGHT, a service: code coloured by token — comments, strings, numbers, keywords — for the languages a paste
// names. Small and dependency-free: one tokenizer per FAMILY (C-like, Python-like, shell, markup, SQL, data), good
// enough to read code by, not a parser. What it gives is text in spans (never HTML from the code: no injection).
//
//   const hl = await ctx.require("highlight");
//   el.append(hl.code(text, "rust"))     // a <code> of spans, classes cw-hl-k (keyword) -s (string) -c (comment) -n (number)
//   hl.LANGUAGES                         // [["plain", "Plain text"], ["js", "JavaScript"], …]
export async function start() {
  const LANGUAGES = [
    ["plain", "Plain text"],
    ["js", "JavaScript"],
    ["ts", "TypeScript"],
    ["rust", "Rust"],
    ["go", "Go"],
    ["python", "Python"],
    ["c", "C / C++"],
    ["java", "Java"],
    ["sh", "Shell"],
    ["sql", "SQL"],
    ["json", "JSON"],
    ["yaml", "YAML"],
    ["html", "HTML / XML"],
    ["css", "CSS"],
    ["md", "Markdown"],
  ];
  const words = s => new Set(s.split(" "));
  const KW = {
    js: words("await async break case catch class const continue default delete do else export extends false finally for from function if import in instanceof let new null of return static super switch this throw true try typeof undefined var void while yield"),
    rust: words("as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while Some None Ok Err"),
    go: words("break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var nil true false"),
    python: words("and as assert async await break class continue def del elif else except False finally for from global if import in is lambda None nonlocal not or pass raise return True try while with yield self"),
    c: words("auto break case char class const continue default delete do double else enum extern float for goto if inline int long namespace new nullptr private protected public return short signed sizeof static struct switch template this typedef union unsigned using virtual void volatile while true false"),
    java: words("abstract boolean break byte case catch char class const continue default do double else enum extends final finally float for if implements import instanceof int interface long new null package private protected public return short static super switch this throw throws try void while true false"),
    sh: words("if then else elif fi for while until do done case esac function in return local export echo exit set unset source alias"),
    sql: words("select from where and or not insert into values update set delete create table index drop alter join left right inner outer on group by order having limit offset as distinct union all null is in like between primary key foreign references default"),
    yaml: words("true false null yes no"),
    json: words("true false null"),
    css: words("important"),
  };
  KW.ts = new Set([...KW.js, ...words("interface type enum implements namespace declare readonly public private protected abstract as keyof never unknown any string number boolean")]);
  // Per family: the comment and string forms, in the order they are tried.
  const LINE = { js: "//", ts: "//", rust: "//", go: "//", c: "//", java: "//", css: null, python: "#", sh: "#", yaml: "#", sql: "--", json: null, html: null, md: null };
  const BLOCK = { js: ["/*", "*/"], ts: ["/*", "*/"], rust: ["/*", "*/"], go: ["/*", "*/"], c: ["/*", "*/"], java: ["/*", "*/"], css: ["/*", "*/"], html: ["<!--", "-->"], sql: ["/*", "*/"] };
  const QUOTES = { python: ['"""', "'''", '"', "'"], js: ['"', "'", "`"], ts: ['"', "'", "`"], sh: ['"', "'"], default: ['"', "'"] };

  function code(text, lang = "plain") {
    const out = document.createElement("code");
    const span = (cls, s) => {
      if (!s) return;
      if (!cls) return out.append(s);
      const e = document.createElement("span");
      e.className = `cw-hl-${cls}`;
      e.textContent = s;
      out.append(e);
    };
    if (!lang || lang === "plain" || lang === "md") return (out.textContent = text), out;
    if (lang === "html") {
      // Tags and their attribute values; the rest as text.
      const re = /(<!--[\s\S]*?-->)|(<\/?[A-Za-z][^>]*>)/g;
      let at = 0;
      for (const m of text.matchAll(re)) {
        span(null, text.slice(at, m.index));
        if (m[1]) span("c", m[1]);
        else {
          const parts = m[2].split(/("[^"]*"|'[^']*')/);
          parts.forEach((p, i) => span(i % 2 ? "s" : "k", p));
        }
        at = m.index + m[0].length;
      }
      span(null, text.slice(at));
      return out;
    }
    const line = LINE[lang] ?? null;
    const block = BLOCK[lang] ?? null;
    const quotes = QUOTES[lang] ?? QUOTES.default;
    const kw = KW[lang] ?? new Set();
    let i = 0;
    let plain = "";
    const flush = () => {
      if (!plain) return;
      // Words and numbers inside the plain run.
      for (const m of plain.split(/(\b\d[\d_]*(?:\.\d+)?(?:e[+-]?\d+)?\b|\b0x[0-9a-fA-F_]+\b|\b[A-Za-z_$][\w$]*\b)/)) {
        if (!m) continue;
        if (/^(\d|0x)/.test(m)) span("n", m);
        else if (kw.has(lang === "sql" ? m.toLowerCase() : m)) span("k", m);
        else span(null, m);
      }
      plain = "";
    };
    while (i < text.length) {
      if (block && text.startsWith(block[0], i)) {
        const end = text.indexOf(block[1], i + block[0].length);
        const j = end < 0 ? text.length : end + block[1].length;
        flush(), span("c", text.slice(i, j)), (i = j);
        continue;
      }
      if (line && text.startsWith(line, i)) {
        const end = text.indexOf("\n", i);
        const j = end < 0 ? text.length : end;
        flush(), span("c", text.slice(i, j)), (i = j);
        continue;
      }
      const q = quotes.find(q => text.startsWith(q, i));
      if (q) {
        let j = i + q.length;
        while (j < text.length && !text.startsWith(q, j)) {
          if (text[j] === "\\") j++;
          else if (q.length === 1 && q !== "`" && text[j] === "\n") break;
          j++;
        }
        j = Math.min(text.length, j + (text.startsWith(q, j) ? q.length : 0));
        flush(), span("s", text.slice(i, j)), (i = j);
        continue;
      }
      plain += text[i++];
    }
    flush();
    return out;
  }
  // The colours: the theme's, so light and dark both read.
  const style = document.createElement("style");
  style.textContent = `@layer components {
    .cw-hl-k { color: var(--cw-accent); font-weight: 600; }
    .cw-hl-s { color: var(--cw-ok, #2f7d4a); }
    .cw-hl-c { color: var(--cw-muted); font-style: italic; }
    .cw-hl-n { color: var(--cw-danger); }
  }`;
  document.head.append(style);
  return { code, LANGUAGES };
}
