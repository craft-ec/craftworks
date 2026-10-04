// BOOK STUDIO, a capability: the ONE way a BOOK or a COMIC is made and opened — as `video-studio` is for videos and
// audio and `image-studio` for images. Three formats: a PDF (read by PDF.js: the `pdfjs` and `pdfjs-worker` packages),
// an EPUB and a CBZ (both ZIP files: `zip`). MAKE: put (`files`) with its COVER (its first page, or the EPUB's cover
// image, as a thumbnail: what every shelf shows) and its own TAGS (title, author). OPEN: one shape whatever the format —
// PAGES (a PDF, a comic: drawn one by one) or CHAPTERS (an EPUB: its text, flowing). `kinds` names it the book
// domain's maker.
//
//   const studio = await ctx.require("book-studio");
//   studio.formatOf(fileOrRef)                                       // "pdf" | "epub" | "cbz" | null
//   const ref = await studio.make(file, { space, public, app, onProgress })   // its reference: `book`, `format`,
//                                                                    // `pages`, `preview`, `tags`
//   const b = await studio.open(blob, format)
//   b.pages  →  await b.page(i, width, height)   // an element: a PDF page drawn to fit, a comic's page as an <img>
//   b.chapters  →  await b.chapter(i, { style })   // an EPUB chapter: { html } — its pictures and styles in it
//   b.close()
export async function start(ctx) {
  const [files, zip, imageStudio] = await Promise.all(["files", "zip", "image-studio"].map(n => ctx.require(n)));
  const TYPES = { pdf: "application/pdf", epub: "application/epub+zip", cbz: "application/vnd.comicbook+zip" };
  // Its FORMAT: by its type, else by its name (a browser gives a .cbz no type).
  function formatOf(x) {
    const t = String(x?.type ?? "").toLowerCase();
    if (t === TYPES.pdf) return "pdf";
    if (t.includes("epub")) return "epub";
    if (t.includes("comicbook") || t === "application/x-cbz") return "cbz";
    const ext = String(x?.name ?? "").toLowerCase().match(/\.([a-z0-9]+)$/)?.[1];
    return ext && TYPES[ext] ? ext : null;
  }

  // PDF.js, loaded once from its bytes. Its WORKER runs on the page (PDF.js's own way: `globalThis.pdfjsWorker`): the
  // app's page is of no origin (sandboxed), where a module worker from bytes does not start.
  let pdfLoading = null;
  const asModule = async name => {
    const url = URL.createObjectURL(new Blob([await ctx.require(name)], { type: "text/javascript" }));
    return import(url).finally(() => URL.revokeObjectURL(url));
  };
  const pdfjs = () =>
    (pdfLoading ??= (async () => {
      const [lib, worker] = await Promise.all([asModule("pdfjs"), asModule("pdfjs-worker")]);
      globalThis.pdfjsWorker = worker;
      return lib;
    })().catch(e => ((pdfLoading = null), Promise.reject(e))));

  const IMAGE = /\.(jpe?g|png|gif|webp|avif|bmp)$/i;
  const IMAGE_TYPE = { jpg: "image/jpeg", jpeg: "image/jpeg", png: "image/png", gif: "image/gif", webp: "image/webp", avif: "image/avif", bmp: "image/bmp", svg: "image/svg+xml" };
  const typeOfName = n => IMAGE_TYPE[String(n).toLowerCase().match(/\.([a-z0-9]+)$/)?.[1]] ?? "";
  // A path inside an EPUB, from where it is named.
  const resolve = (from, href) => {
    const parts = from.split("/").slice(0, -1);
    for (const p of decodeURIComponent(href.split("#")[0]).split("/")) p === ".." ? parts.pop() : p && p !== "." && parts.push(p);
    return parts.join("/");
  };

  async function openPdf(blob) {
    const lib = await pdfjs();
    const task = lib.getDocument({ data: new Uint8Array(await blob.arrayBuffer()) });
    const doc = await task.promise;
    return {
      format: "pdf",
      pages: doc.numPages,
      // A page drawn to fit `width` (and `height`, when given) in CSS pixels, sharp on the screen it is on.
      async page(i, width = 800, height = Infinity) {
        const p = await doc.getPage(i + 1);
        const one = p.getViewport({ scale: 1 });
        const dpr = Math.min(2, globalThis.devicePixelRatio || 1);
        const vp = p.getViewport({ scale: Math.min(width / one.width, height / one.height) * dpr });
        const c = Object.assign(document.createElement("canvas"), { width: Math.ceil(vp.width), height: Math.ceil(vp.height) });
        c.style.width = `${Math.round(vp.width / dpr)}px`;
        await p.render({ canvas: c, canvasContext: c.getContext("2d"), viewport: vp }).promise;
        return c;
      },
      async meta() {
        const m = await doc.getMetadata().catch(() => null);
        return { title: m?.info?.Title || "", author: m?.info?.Author || "" };
      },
      close: () => task.destroy(),
    };
  }

  async function openCbz(blob) {
    const z = await zip.open(blob);
    const pages = z
      .names()
      .filter(n => IMAGE.test(n) && !n.split("/").some(p => p.startsWith(".") || p === "__MACOSX"))
      .sort((a, b) => a.localeCompare(b, undefined, { numeric: true, sensitivity: "base" }));
    const urls = [];
    return {
      format: "cbz",
      pages: pages.length,
      async page(i) {
        const url = URL.createObjectURL(await z.blob(pages[i], typeOfName(pages[i])));
        urls.push(url);
        return Object.assign(document.createElement("img"), { src: url, alt: `Page ${i + 1}` });
      },
      first: () => (pages.length ? z.blob(pages[0], typeOfName(pages[0])) : null),
      meta: async () => ({ title: "", author: "" }),
      close: () => urls.splice(0).forEach(u => URL.revokeObjectURL(u)),
    };
  }

  async function openEpub(blob) {
    const z = await zip.open(blob);
    const xml = async name => {
      const t = await z.text(name);
      if (t == null) throw new Error(`not an EPUB: ${name} is missing`);
      return new DOMParser().parseFromString(t, "application/xml");
    };
    const opfPath = (await xml("META-INF/container.xml")).querySelector("rootfile")?.getAttribute("full-path");
    if (!opfPath) throw new Error("not an EPUB: no package file named");
    const opf = await xml(opfPath);
    const items = new Map([...opf.getElementsByTagName("item")].map(i => [i.getAttribute("id"), { href: resolve(opfPath, i.getAttribute("href") ?? ""), type: i.getAttribute("media-type") ?? "", props: i.getAttribute("properties") ?? "" }]));
    const spine = [...opf.getElementsByTagName("itemref")].map(r => items.get(r.getAttribute("idref"))).filter(i => i && /html/.test(i.type));
    const dc = tag => opf.getElementsByTagNameNS("*", tag)[0]?.textContent?.trim() ?? "";
    const coverId = [...opf.getElementsByTagName("meta")].find(m => m.getAttribute("name") === "cover")?.getAttribute("content");
    const coverItem = [...items.values()].find(i => /\bcover-image\b/.test(i.props)) ?? items.get(coverId) ?? null;
    // A chapter is shown in a frame of no origin of its own (the reader's), where no object URL loads: its pictures
    // go in as data: URLs and its stylesheets as text, each read once.
    const datas = new Map(); // an entry's path → its data: URL
    const dataOf = path => {
      if (!datas.has(path))
        datas.set(
          path,
          (async () => {
            const item = [...items.values()].find(i => i.href === path);
            const b = await z.blob(path, item?.type || typeOfName(path));
            return b ? await new Promise(r => Object.assign(new FileReader(), { onload: e => r(e.target.result), onerror: () => r(null) }).readAsDataURL(b)) : null;
          })(),
        );
      return datas.get(path);
    };
    return {
      format: "epub",
      chapters: spine.length,
      // A CHAPTER: its document whole — scripts taken out, its pictures and stylesheets in it, `style` (the reader's)
      // first in its head.
      async chapter(i, { style = "" } = {}) {
        const path = spine[i].href;
        const src = (await z.text(path)) ?? "";
        let doc = new DOMParser().parseFromString(src, "application/xhtml+xml");
        if (doc.querySelector("parsererror")) doc = new DOMParser().parseFromString(src, "text/html");
        doc.querySelectorAll("script").forEach(s => s.remove());
        for (const link of doc.querySelectorAll("link[rel~=stylesheet][href]")) {
          const v = link.getAttribute("href");
          const css = v && !/^[a-z]+:/i.test(v) ? await z.text(resolve(path, v)) : null;
          css == null ? link.remove() : link.replaceWith(Object.assign(doc.createElementNS(link.namespaceURI, "style"), { textContent: css }));
        }
        for (const el of doc.querySelectorAll("img[src], image")) {
          const attr = el.hasAttribute("src") ? "src" : el.hasAttribute("href") ? "href" : "xlink:href";
          const v = el.getAttribute(attr);
          if (!v || /^[a-z]+:/i.test(v)) continue;
          const u = await dataOf(resolve(path, v));
          if (u) el.setAttribute(attr, u);
        }
        if (style) {
          const head = doc.querySelector("head") ?? doc.documentElement.insertBefore(doc.createElementNS(doc.documentElement.namespaceURI, "head"), doc.documentElement.firstChild);
          head.prepend(Object.assign(doc.createElementNS(doc.documentElement.namespaceURI, "style"), { textContent: style }));
        }
        return { html: doc.documentElement.outerHTML };
      },
      first: () => (coverItem ? z.blob(coverItem.href, coverItem.type) : null),
      meta: async () => ({ title: dc("title"), author: dc("creator") }),
      close: () => datas.clear(),
    };
  }

  async function open(blob, format = formatOf(blob)) {
    if (format === "pdf") return openPdf(blob);
    if (format === "cbz") return openCbz(blob);
    if (format === "epub") return openEpub(blob);
    throw new Error("not a book read here (a PDF, an EPUB or a CBZ)");
  }

  // Its COVER: a PDF's first page drawn, a comic's first page, an EPUB's cover image — as a thumbnail (or null).
  async function cover(book) {
    try {
      if (book.format === "pdf") {
        const c = await book.page(0, 320);
        return await imageStudio.thumbnail(await new Promise(r => c.toBlob(r, "image/png")));
      }
      const first = await book.first();
      return first ? await imageStudio.thumbnail(first) : null;
    } catch {
      return null;
    }
  }

  async function make(file, { space = null, public: pub = false, app = "book", onProgress = () => {} } = {}) {
    const format = formatOf(file);
    if (!format) throw new Error(`${file.name}: not a book read here (a PDF, an EPUB or a CBZ)`);
    onProgress({ stage: "reading", p: 0 });
    const book = await open(file, format);
    const [preview, tags] = await Promise.all([cover(book), book.meta().catch(() => ({}))]);
    const pages = book.pages ?? null;
    book.close();
    const typed = new File([file], file.name, { type: TYPES[format] });
    const up = await files.put(typed, { space, public: !!pub, app, onProgress: p => onProgress({ stage: "uploading", p: p.done / Math.max(1, p.size) }) });
    const t = Object.fromEntries(Object.entries(tags ?? {}).filter(([, v]) => v));
    return { ...up, name: file.name, book: true, format, ...(pages ? { pages } : {}), ...(preview ? { preview } : {}), ...(Object.keys(t).length ? { tags: t } : {}) };
  }

  return { formatOf, make, open, cover, TYPES };
}
