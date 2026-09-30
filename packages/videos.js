// VIDEOS, a page (the Videos app, YouTube-shaped, the basic): a LENS on the video DOMAIN of content (`kinds`: video,
// movie, TV episode, music video, short) — items `posts` keeps, like Board's posts. CHANNEL-FIRST: your videos (each
// public on your profile, or only you) and those of the people you follow (`#/videos`: the feed; `#/videos/mine`: your
// channel; `#/videos/c/<did>`: someone's). In a SPACE (`#/s/<space>/videos`): its members' videos, on its board, public
// while its board reads in public. WATCH: `…/videos/w/<ref>` — played by byte range (`video-player`), a like (▲, the
// post vote), comments. UPLOAD: `…/videos/up`. UI only.
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  const [posts, directory, person, theme, space, roles, drive, player, kinds, edge] = await Promise.all(["posts", "directory", "person", "theme", "space", "roles", "drive-store", "video-player", "kinds", "edge"].map(n => ctx.require(n)));
  const [pins, people] = await Promise.all([edge.pins(), edge.people()]);
  const VIDEO = kinds.inDomain("video");
  const me = (await space.account()).id;
  el.innerHTML = `
    <style>
      .vd { max-width: 1200px; margin: 0 auto; display: grid; gap: var(--cw-space-3); }
      .vd .top { display: flex; align-items: center; gap: var(--cw-space-2); flex-wrap: wrap; }
      .vd .top h2 { margin: 0; font-size: 1.3rem; }
      .vd .tabs { display: flex; gap: var(--cw-space-2); flex: 1; }
      .vd .tabs a { color: var(--cw-muted); text-decoration: none; padding: 4px 10px; border-radius: 999px; }
      .vd .tabs a.on { color: var(--cw-fg); background: var(--cw-hover); }
      .vd .up { background: var(--cw-accent); color: var(--cw-accent-fg); border-radius: var(--cw-radius-sm); padding: 6px 12px; text-decoration: none; }
      .vd .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(240px, 1fr)); gap: var(--cw-space-4) var(--cw-space-3); }
      .vd .card { text-decoration: none; color: inherit; display: grid; gap: 6px; }
      .vd .thumb { position: relative; aspect-ratio: 16 / 9; background: #000; border-radius: var(--cw-radius); overflow: hidden; display: grid; place-items: center; font-size: 2rem; }
      .vd .thumb img { width: 100%; height: 100%; object-fit: cover; }
      .vd .dur { position: absolute; right: 6px; bottom: 6px; background: rgba(0,0,0,.8); color: #fff; font-size: var(--cw-text-xs); padding: 1px 5px; border-radius: 4px; }
      .vd .kind { position: absolute; left: 6px; top: 6px; background: rgba(0,0,0,.7); color: #fff; font-size: var(--cw-text-xs); padding: 1px 6px; border-radius: 4px; }
      .vd .t { font-weight: 600; line-height: 1.3; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
      .vd .s { color: var(--cw-muted); font-size: var(--cw-text-sm); }
      .vd .by { cursor: pointer; }
      .vd .by:hover { color: var(--cw-fg); }
      .vd .none { color: var(--cw-muted); text-align: center; padding: var(--cw-space-5); }
      .vd .watch { display: grid; gap: var(--cw-space-3); max-width: 1000px; }
      .vd .watch video { width: 100%; max-height: 70vh; background: #000; border-radius: var(--cw-radius); }
      .vd .watch h1 { font-size: 1.25rem; margin: 0; }
      .vd .row { display: flex; gap: var(--cw-space-3); align-items: center; flex-wrap: wrap; }
      .vd button { font: inherit; cursor: pointer; border: 1px solid var(--cw-line); background: var(--cw-surface); color: var(--cw-fg); border-radius: 999px; padding: 5px 14px; }
      .vd button.on { background: var(--cw-accent); color: var(--cw-accent-fg); border-color: transparent; }
      .vd .about { background: var(--cw-hover); border-radius: var(--cw-radius); padding: var(--cw-space-3); white-space: pre-wrap; }
      .vd .meta { display: flex; gap: var(--cw-space-3); flex-wrap: wrap; font-size: var(--cw-text-sm); color: var(--cw-muted); }
      .vd .comments { display: grid; gap: var(--cw-space-2); }
      .vd .c { display: grid; gap: 2px; }
      .vd .c .rep { margin-left: var(--cw-space-4); display: grid; gap: var(--cw-space-2); }
      .vd form { display: grid; gap: var(--cw-space-2); max-width: 640px; }
      .vd input, .vd textarea, .vd select { font: inherit; padding: 6px 8px; border-radius: var(--cw-radius-sm); border: 1px solid var(--cw-line); background: var(--cw-surface); color: var(--cw-fg); }
      .vd .said { color: var(--cw-danger); margin: 0; }
    </style>
    <div class="vd"></div>`;
  const root = el.querySelector(".vd");
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  const discovering = () => ctx.space === "discover";
  const sp = () => (ctx.space && !discovering() ? ctx.space : null);
  const base = () => (discovering() ? "#/discover/videos" : sp() ? `#/s/${sp()}/videos` : "#/videos");
  const route = () => {
    const s = ctx.sub || "";
    if (s.startsWith("w/")) return { watch: decodeURIComponent(s.slice(2)) };
    if (s === "up") return { up: true };
    if (s.startsWith("c/")) return { by: decodeURIComponent(s.slice(2)) };
    if (discovering()) return { discover: true };
    if (sp()) return { board: sp() };
    return s === "mine" ? { by: me } : s === "saved" ? { saved: true } : { feed: true };
  };
  // A public space's video seen from OUTSIDE (Discover, a followed space): its public description.
  const outsideOf = async ref => {
    if (!ref.startsWith("space:")) return null;
    const id = ref.slice(6, ref.indexOf("/"));
    if ((await space.mine()).some(x => x.id === id)) return null;
    return (await posts.publicSpaces().catch(() => [])).find(d => d.id === id) ?? people.about("follow", id);
  };
  const SAVED = ref => `videos:${ref}`;
  const ago = at => {
    const s = Math.max(0, (Date.now() - at) / 1000);
    if (s < 60) return "just now";
    for (const [n, u] of [[31536000, "year"], [2592000, "month"], [86400, "day"], [3600, "hour"], [60, "minute"]]) if (s >= n) return `${Math.floor(s / n)} ${u}${Math.floor(s / n) > 1 ? "s" : ""} ago`;
  };
  const clock = d => {
    d = Math.round(d || 0);
    const hh = Math.floor(d / 3600);
    const mm = Math.floor((d % 3600) / 60);
    const ss = String(d % 60).padStart(2, "0");
    return hh ? `${hh}:${String(mm).padStart(2, "0")}:${ss}` : `${mm}:${ss}`;
  };
  const who = did => {
    const n = h("span", { className: "by", textContent: directory.shown(did), onclick: e => (e.preventDefault(), e.stopPropagation(), (location.hash = `#/videos/c/${encodeURIComponent(did)}`)) });
    directory.name(did).then(t => (n.textContent = t), () => {});
    return n;
  };
  const fileOf = v => v.files?.find(f => /^video\//.test(f.type ?? "")) ?? v.files?.[0] ?? null;

  let spaceName = null;
  function top(w) {
    const tabs = discovering()
      ? [{ label: "Discover", href: base(), on: !w.watch }]
      : sp()
        ? [{ label: "Videos", href: base(), on: !w.watch && !w.up }]
        : [
            { label: "Following", href: "#/videos", on: !!w.feed },
            { label: "Your channel", href: "#/videos/mine", on: w.by === me },
            { label: "Saved", href: "#/videos/saved", on: !!w.saved },
            { label: "Discover", href: "#/discover/videos", on: false },
          ];
    // In a space, it is a CHANNEL (the space named in this app's own words).
    const title = sp() ? `▶️ ${spaceName ?? "Channel"} · channel` : "▶️ Videos";
    return h("div", { className: "top" }, h("h2", { textContent: title }), h("nav", { className: "tabs" }, ...tabs.map(t => h("a", { href: t.href, textContent: t.label, className: t.on ? "on" : "" }))), discovering() ? null : h("a", { className: "up", href: `${base()}/up`, textContent: "⬆ Upload" }));
  }

  function card(v) {
    const f = fileOf(v);
    return h(
      "a",
      { className: "card", href: `${base()}/w/${encodeURIComponent(v.ref)}` },
      h("div", { className: "thumb" }, f?.preview ? h("img", { src: f.preview, alt: "" }) : "🎬", f?.duration ? h("span", { className: "dur", textContent: clock(f.duration) }) : null, v.kind !== "video" ? h("span", { className: "kind", textContent: kinds.of(v.kind)?.label ?? v.kind }) : null),
      h("div", { className: "t", textContent: v.title }),
      h("div", { className: "s" }, who(v.by), ` · ${ago(v.at)}${v.private ? " · only you" : ""}`),
    );
  }

  async function list(w) {
    // SAVED: the videos this person pinned (the pin edge), newest saved first.
    const vs = w.saved
      ? (await Promise.all(pins.refs("videos:").map(async k => {
          const ref = k.slice(7);
          return posts.get(ref, { outside: await outsideOf(ref) }).catch(() => null);
        }))).filter(Boolean)
      : await posts.list(w, "new", VIDEO);
    const none = w.saved ? "Nothing saved: “Save” on a video keeps it here." : w.feed ? "No videos yet from you or what you follow." : w.discover ? "No public videos yet." : w.by === me ? "Your channel is empty: upload a video." : "No videos here yet.";
    return vs.length ? h("div", { className: "grid" }, ...vs.map(card)) : h("p", { className: "none", textContent: none });
  }

  async function watch(ref) {
    const outside = await outsideOf(ref);
    const v = await posts.get(ref, { outside });
    if (!v) return h("p", { className: "none", textContent: "This video is not here (removed, or not shared with you)." });
    const f = fileOf(v);
    const note = h("span", { className: "s" });
    const video = h("video", { controls: true, playsInline: true, poster: f?.preview ?? "" });
    const like = h("button", { type: "button", className: v.mine === 1 ? "on" : "", disabled: !!outside, title: outside ? "Join to like" : "", textContent: `▲ ${v.score ?? 0}`, onclick: async () => ((like.disabled = true), await posts.vote(ref, v.mine === 1 ? 0 : 1).catch(() => {}), draw()) });
    const saved = () => pins.has(SAVED(ref));
    const save = h("button", { type: "button", className: saved() ? "on" : "", textContent: saved() ? "Saved ✓" : "Save", onclick: async () => (await pins.set(SAVED(ref), !saved()), (save.className = saved() ? "on" : ""), (save.textContent = saved() ? "Saved ✓" : "Save")) });
    const k = kinds.of(v.kind);
    const fields = (k?.fields ?? []).filter(x => v.meta?.[x]).map(x => h("span", { textContent: `${kinds.fieldLabel(x)}: ${v.meta[x]}` }));
    const comments = h("div", { className: "comments" }, theme.loading("Reading the comments…"));
    const form = h("form", {}, h("textarea", { name: "body", rows: 2, placeholder: "Add a comment…", required: true }), h("div", {}, h("button", { textContent: "Comment" })));
    form.onsubmit = async e => {
      e.preventDefault();
      await posts.comment(ref, null, form.elements.body.value).catch(() => {});
      form.reset();
      drawComments();
    };
    const drawOne = c => h("div", { className: "c" }, h("div", { className: "s" }, who(c.by), ` · ${ago(c.at)}`), h("div", { textContent: c.body }), c.replies?.length ? h("div", { className: "rep" }, ...c.replies.map(drawOne)) : null);
    const drawComments = async () => {
      const cs = await posts.thread(ref, { outside }).catch(() => []);
      comments.replaceChildren(...(cs.length ? cs.map(drawOne) : [h("p", { className: "s", textContent: "No comments yet." })]));
    };
    const out = h(
      "div",
      { className: "watch" },
      video,
      note,
      h("h1", { textContent: v.title }),
      h("div", { className: "row" }, h("span", { className: "s" }, who(v.by), ` · ${ago(v.at)}${k && v.kind !== "video" ? ` · ${k.label}` : ""}${v.private ? " · only you" : ""}`), like, save),
      fields.length ? h("div", { className: "meta" }, ...fields) : null,
      v.body ? h("div", { className: "about", textContent: v.body }) : null,
      h("h3", { textContent: `Comments` }),
      outside ? null : form,
      comments,
    );
    queueMicrotask(() => {
      if (f) player.play(video, f, { onNote: t => (note.textContent = t) }).catch(e => (note.textContent = e.message ?? String(e)));
      drawComments();
    });
    return out;
  }

  async function upload() {
    const said = h("p", { className: "said", hidden: true });
    const progress = h("span", { className: "s" });
    const kindSel = h("select", { name: "kind" }, ...kinds.inDomain("video").map(k => h("option", { value: k, textContent: kinds.of(k).label })));
    const fieldsBox = h("div", { style: "display:grid;gap:6px" });
    const drawFields = () => fieldsBox.replaceChildren(...kinds.of(kindSel.value).fields.map(x => h("input", { name: `meta.${x}`, placeholder: kinds.fieldLabel(x) })));
    kindSel.onchange = drawFields;
    drawFields();
    const inSpace = sp() ? (await space.mine()).find(s => s.id === sp()) : null;
    const f = h(
      "form",
      {},
      h("input", { type: "file", name: "file", accept: "video/*", required: true }),
      h("input", { name: "title", placeholder: "Title", required: true, maxLength: 300 }),
      h("textarea", { name: "body", rows: 4, placeholder: "Description" }),
      h("label", { className: "s" }, "What it is ", kindSel),
      fieldsBox,
      inSpace ? h("p", { className: "s", textContent: `For ${space.shown(inSpace)}: its members (and everyone, while its board reads in public).` }) : h("select", { name: "audience" }, h("option", { value: "public", textContent: "🌐 Everyone (on your channel)" }), h("option", { value: "private", textContent: "🔒 Only you" })),
      h("div", { className: "row" }, h("button", { className: "on", textContent: "Upload" }), progress),
      said,
    );
    f.onsubmit = async e => {
      e.preventDefault();
      said.hidden = true;
      const file = f.elements.file.files[0];
      if (!file) return;
      const btn = f.querySelector("button");
      btn.disabled = true;
      try {
        progress.textContent = "Reading the video…";
        const m = await player.meta(file);
        const pub = inSpace ? (await roles.of(inSpace)).policy("board", "read") === "anyone" : f.elements.audience.value !== "private";
        const up = await drive.upload(file, { space: inSpace, public: pub, from: { app: "videos" }, onProgress: p => (progress.textContent = p.phase === "reading" ? "Reading…" : `Uploading ${Math.round((100 * p.done) / Math.max(1, p.size))}%`) });
        const ref = { ...up, ...(m.poster ? { preview: m.poster } : {}), ...(m.duration ? { duration: m.duration } : {}) };
        const meta = Object.fromEntries(kinds.of(kindSel.value).fields.map(x => [x, String(f.elements[`meta.${x}`]?.value ?? "").trim()]).filter(([, v]) => v));
        const posted = await posts.submit({ board: inSpace?.id ?? null, title: f.elements.title.value, body: f.elements.body.value, kind: kindSel.value, meta, private: !inSpace && !pub, files: [ref] });
        location.hash = `${base()}/w/${encodeURIComponent(posted)}`;
      } catch (err) {
        said.textContent = err.message ?? String(err);
        said.hidden = false;
        btn.disabled = false;
        progress.textContent = "";
      }
    };
    return h("div", {}, h("h3", { textContent: "Upload a video" }), f);
  }

  let drawn = "";
  async function draw() {
    const w = route();
    spaceName = sp() ? space.shown((await space.mine()).find(x => x.id === sp()) ?? { id: sp(), name: "" }) : null;
    drawn = `${ctx.space ?? ""}|${ctx.sub ?? ""}`;
    root.replaceChildren(top(w), theme.loading(w.watch ? "Opening the video…" : "Reading the videos…"));
    const body = await (w.watch ? watch(w.watch) : w.up ? upload() : list(w)).catch(e => h("p", { className: "said", textContent: e.message ?? String(e) }));
    if (drawn === `${ctx.space ?? ""}|${ctx.sub ?? ""}`) root.replaceChildren(top(w), ...(w.by && w.by !== me && !w.watch ? [h("div", { className: "row" }, h("h3", {}, who(w.by)), h("button", { type: "button", textContent: "Follow…", onclick: e => person.open(e.currentTarget, w.by) }))] : []), body);
  }
  await draw();
  posts.onChange(() => el.isConnected && !route().watch && !route().up && draw());
  addEventListener("craftworks:route", () => el.isConnected && ctx.route === "/videos" && `${ctx.space ?? ""}|${ctx.sub ?? ""}` !== drawn && draw());
}
