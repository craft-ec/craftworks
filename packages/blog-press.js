// BLOG PRESS, a capability: what the Blog app (`blog`) does to Board's POSTS — a LENS, no store of its own (ARCHITECTURE
// §1, §7): an article IS a post (`items`, kind `post`), the same one Board lists; Blog reads it as an article and keeps
// three states on it in its `meta`, nowhere else:
// - A DRAFT: a post "only you" (`items.submit({ audience: "private" })`: your account's sealed `journal`, read by your
//   devices alone) flagged `meta.blog = { draft: true, place, audience, due }` — `place` the space it is for (null:
//   your blog), `audience` whom it will be for once published, `due` (ms) when it publishes itself. Board shows it
//   among your posts as "only you"; Blog lists it under Drafts.
// - SCHEDULED: a draft with `due`. Whichever Craftworks PAGE of this person is open when it falls due publishes it
//   (`upkeep` asks `publishDue` every tick, 30 s): no delegate works for it. With NO page open then, it publishes the
//   next time one opens — late, never lost; two devices open at that very moment may both publish it (each checks the
//   published copy first, `meta.blog.from`, which narrows but cannot close that race).
// - IN REVIEW (a space as a PUBLICATION, its setting `blog`/`approval` = "editors"): a member who may not `moderate`
//   there (`roles`: the owner, admins, a role composed with it — its EDITORS) publishes a REVIEW COPY sealed to the
//   space's members (`audience: "members"`, `meta.blog.review = "pending"`: never public). An editor APPROVES or
//   DECLINES it with a comment on it (`approve`: "✅ Approved" / "❌ Declined"), counted only from someone the space's
//   roles let moderate. The author's page then publishes the public copy and takes the review copy and the draft out
//   (the author's own feed is the only one that can carry their post). Blog enforces it; Board's composer does not.
// Published: the post, its `meta.tags` and `meta.cover` (a file key among its files), `meta.blog.from` the draft's id.
//
//   const press = await ctx.require("blog-press");
//   await press.drafts()                        // this person's drafts, newest first
//   await press.saveDraft({ ref, title, body, files, tags, cover, place, audience, due })   // its ref
//   await press.publish(draft)                  // { ref, review } — published, or sent to the editors
//   await press.publishDue()                    // the due ones published, the approved ones too (`upkeep`)
//   await press.review(sp)  press.approve(ref, yes)  press.mayEdit(sp)  press.needsReview(sp)
//   press.isArticle(it)                         // a post Blog lists as published (no draft, no review copy)
// Pure (no page, no node — `tools/blog-press.test.mjs`): readingTime, headingsOf, isDue, approvalOf, coverOf.

// READING TIME: words at 230 a minute (a CJK character counts as a word), at least a minute for any text; code, images,
// embedded items and link targets are not read as words.
export const WPM = 230;
export function readingTime(body) {
  const text = String(body ?? "")
    .replace(/```[\s\S]*?(```|$)/g, " ")
    .replace(/!\[[^\]]*\]\([^)]*\)/g, " ")
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/https?:\/\/\S+/g, " ");
  const cjk = (text.match(/[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/gu) ?? []).length;
  const words = (text.replace(/[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/gu, " ").match(/[\p{L}\p{N}]+(?:['’-][\p{L}\p{N}]+)*/gu) ?? []).length + cjk;
  return { words, minutes: words ? Math.max(1, Math.ceil(words / WPM)) : 0 };
}

// THE CONTENTS: the headings as `markdown` draws them — a line `#`…`######`, a space, its text — outside fenced code;
// each with its level, its text without markup, and a SLUG unique in the article (a repeat gets -2, -3 …).
export function headingsOf(body) {
  const out = [];
  const used = new Map();
  let fenced = false;
  for (const ln of String(body ?? "").split(/\r?\n/)) {
    if (/^```/.test(ln)) {
      fenced = !fenced;
      continue;
    }
    if (fenced) continue;
    const m = /^(#{1,6})\s+(.*)$/.exec(ln);
    if (!m) continue;
    const text = m[2]
      .replace(/\s+#+\s*$/, "")
      .replace(/!\[[^\]]*\]\([^)]*\)/g, "")
      .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
      .replace(/(\*\*|__|\*|_|~~|`|\|\|)/g, "")
      .trim();
    if (!text) continue;
    const base = text.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, "-").replace(/^-+|-+$/g, "") || "section";
    const n = (used.get(base) ?? 0) + 1;
    used.set(base, n);
    out.push({ level: m[1].length, text, slug: n === 1 ? base : `${base}-${n}` });
  }
  return out;
}

// DUE: a draft whose time has come — not one in review (sent already), not one without a time.
export function isDue(it, now = Date.now()) {
  const b = it?.meta?.blog;
  return !!b?.draft && !b.review && Number.isFinite(b.due) && b.due <= now;
}

// THE EDITORS' WORD on a review copy, from its comments (a tree: `replies`): the LATEST "✅ Approved…" or
// "❌ Declined…" by someone `mayApprove(did)` says (roles: may moderate) — anyone else's counts for nothing; none: null.
export const APPROVED = "✅ Approved";
export const DECLINED = "❌ Declined";
export function approvalOf(comments, mayApprove) {
  let best = null;
  const walk = cs => {
    for (const c of cs ?? []) {
      const body = String(c?.body ?? "").trim();
      const word = body.startsWith(APPROVED) ? "approved" : body.startsWith(DECLINED) ? "declined" : null;
      if (word && mayApprove(c.by) && (!best || (c.at ?? 0) >= best.at)) best = { word, by: c.by, at: c.at ?? 0 };
      walk(c?.replies);
    }
  };
  walk(comments);
  return best;
}

// ITS COVER: the file its author chose (`meta.cover`: a file key), else its first image not already shown in its text.
// `kindOf(ref)` / `keyOf(ref)` / `inlined` (keys): `markdown`'s, given.
export function coverOf(it, { kindOf, keyOf, inlined = new Set() }) {
  const files = Array.isArray(it?.files) ? it.files : [];
  const chosen = it?.meta?.cover ? files.find(f => keyOf(f) === it.meta.cover) : null;
  return chosen ?? files.find(f => kindOf(f) === "image" && !inlined.has(keyOf(f))) ?? null;
}

export async function start(ctx) {
  const [items, space, roles] = await Promise.all(["items", "space", "roles"].map(n => ctx.require(n)));
  const me = async () => space.me();
  const blogOf = it => (it?.meta?.blog && typeof it.meta.blog === "object" ? it.meta.blog : {});

  // An article as Blog lists it: a post that is neither a draft nor a review copy.
  const isArticle = it => it?.kind === "post" && !blogOf(it).draft && !blogOf(it).review;

  // A PUBLICATION's rule: its editors approve (`config` act, on the space's settings: `app-settings`).
  const ruleOf = async sp => {
    const r = await roles.of(sp);
    await r.settled;
    return r;
  };
  const mayEdit = async (sp, did = null) => (await ruleOf(sp)).can(did ?? (await me()), "moderate");
  const needsReview = async sp => {
    const r = await ruleOf(sp);
    return r.config("blog", "approval", "") === "editors" && !r.can(await me(), "moderate");
  };

  // THIS PERSON's DRAFTS: their own posts "only you" flagged a draft (read whole: few, and their own).
  async function drafts() {
    const self = await me();
    const all = await items.inPlaces({ people: [self] }, "post", { withVotes: false });
    return all.filter(it => it.by === self && it.private && blogOf(it).draft).sort((a, b) => (b.edited || b.at) - (a.edited || a.at));
  }

  // SAVED as a draft (new, or the one `ref` names: a new version, kept in its history).
  // `write`, `level`: the audience picker's other two choices (who may interact; the level who may open it), applied
  // when it is published.
  async function saveDraft({ ref = null, title = "", body = "", files = [], tags = [], cover = null, place = null, audience = "public", write = null, level = "", due = null }) {
    const meta = { ...(tags.length ? { tags } : {}), ...(cover ? { cover } : {}), blog: { draft: true, place: place ?? null, audience, ...(write ? { write } : {}), ...(level ? { level } : {}), due: Number.isFinite(due) ? due : null } };
    title = String(title ?? "").trim() || "Untitled draft";
    if (ref) {
      await items.editItem(ref, body, { files, meta, title });
      return ref;
    }
    return items.submit({ kind: "post", title, body, files, audience: "private", meta });
  }

  // What a published copy carries: the draft's tags and cover, and where it came from (never its draft state).
  const publishedMeta = (d, extra = {}) => ({ ...(d.meta?.tags?.length ? { tags: d.meta.tags } : {}), ...(d.meta?.cover ? { cover: d.meta.cover } : {}), ...(blogOf(d).level ? { level: blogOf(d).level } : {}), blog: { from: d.id, ...extra } });
  // Its FILES, in a space: public exactly when what is published there is (a draft's were kept to the space's members).
  const filesFor = async (d, sp, pub) => sp && (await ctx.require("files")).publicity(d.files, sp, pub).catch(e => ctx.log("blog", { what: `its files: ${e.message}` }));
  const placeOf = async d => {
    const id = blogOf(d).place;
    if (!id) return null;
    const sp = await items.boardOf(id);
    if (!sp) throw new Error("you are no longer in the space this draft is for");
    return sp;
  };
  // PUBLISHED ALREADY (another device of this person's did, from the same draft)? Its copy, else null.
  async function publishedFrom(d, sp) {
    const days = Math.max(1, Math.ceil((Date.now() - (blogOf(d).due ?? d.at)) / 86400e3) + 1);
    // A space's last days; one's own profile whole (its own tail: never the followed profiles a list's votes read).
    const list = await (sp ? items.list({ board: sp.id }, "new", "post", { window: days }) : items.inPlaces({ people: [await me()] }, "post", { withVotes: false })).catch(() => []);
    return list.find(p => blogOf(p).from === d.id && !blogOf(p).draft) ?? null;
  }

  // PUBLISH a draft: to its place as its audience says — or, in a publication whose editors approve, to them first.
  async function publish(d) {
    const b = blogOf(d);
    const sp = await placeOf(d);
    const done = await publishedFrom(d, sp);
    if (done && !blogOf(done).review) {
      await items.remove(d.ref).catch(() => {});
      return { ref: done.ref, review: false };
    }
    if (sp && (await needsReview(sp))) {
      // SENT AGAIN (changed after a decline): the earlier review copy out, the editors read this one.
      for (const old of new Set([b.review?.ref, done?.ref].filter(Boolean))) await items.remove(old).catch(() => {});
      await filesFor(d, sp, false);
      const ref = await items.submit({ board: sp.id, kind: "post", title: d.title, body: d.body, files: d.files, audience: "members", meta: publishedMeta(d, { review: "pending" }) });
      await items.editItem(d.ref, d.body, { files: d.files, title: d.title, meta: { ...d.meta, blog: { ...b, due: null, declined: null, review: { ref, at: Date.now() } } } });
      return { ref, review: true };
    }
    const audience = sp ? (b.audience === "public" && !(await items.publicIn(sp, "post")) ? "members" : b.audience || "members") : b.audience || "public";
    await filesFor(d, sp, audience === "public");
    const ref = await items.submit({ board: sp?.id ?? null, kind: "post", title: d.title, body: d.body, files: d.files, audience, write: b.write ?? null, meta: publishedMeta(d) });
    await items.remove(d.ref);
    return { ref, review: false };
  }

  // A draft IN REVIEW, asked: approved → published (public as its author chose), the review copy and the draft out;
  // declined → the draft says so (back to the author to change and send again); the review copy gone → the draft back.
  async function settle(d) {
    const b = blogOf(d);
    const sp = await placeOf(d);
    const copy = await items.get(b.review.ref).catch(() => null);
    const back = extra => items.editItem(d.ref, d.body, { files: d.files, title: d.title, meta: { ...d.meta, blog: { ...b, review: null, ...extra } } });
    if (!copy) return back({ declined: null });
    const r = await ruleOf(sp);
    const word = approvalOf(await items.thread(copy.ref), did => did !== copy.by && r.can(did, "moderate"));
    if (word?.word === "declined" && !b.declined) return back({ review: b.review, declined: { by: word.by, at: word.at } });
    if (word?.word !== "approved") return;
    const audience = b.audience === "public" && !(await items.publicIn(sp, "post")) ? "members" : b.audience || "members";
    await filesFor(d, sp, audience === "public");
    const ref = await items.submit({ board: sp.id, kind: "post", title: d.title, body: d.body, files: d.files, audience, write: b.write ?? null, meta: publishedMeta(d, { approved: word.by }) });
    await items.remove(copy.ref).catch(e => ctx.log("blog", { what: `the review copy kept: ${e.message}` }));
    await items.remove(d.ref);
    return ref;
  }

  // EVERY TICK (`upkeep`, any page): the drafts due, published; those in review, asked (each at most every 5 minutes).
  let last = 0;
  const asked = new Map(); // draft id → when its review was last asked
  let running = null;
  function publishDue() {
    if (running || Date.now() - last < 55e3) return running ?? Promise.resolve();
    last = Date.now();
    return (running = (async () => {
      for (const d of await drafts()) {
        try {
          if (isDue(d)) {
            const { ref, review } = await publish(d);
            ctx.log("blog", { what: `scheduled draft ${review ? "sent for review" : "published"}: ${ref}` });
          } else if (blogOf(d).review?.ref && Date.now() - (asked.get(d.id) ?? 0) > 300e3) {
            asked.set(d.id, Date.now());
            const ref = await settle(d);
            if (typeof ref === "string") ctx.log("blog", { what: `approved and published: ${ref}` });
          }
        } catch (e) {
          ctx.log("blog", { what: `draft ${d.id}: ${e.message ?? e}` });
        }
      }
    })().finally(() => (running = null)));
  }

  // A PUBLICATION's REVIEW QUEUE (its editors'): the review copies there, oldest first, each with its editors' word.
  async function review(sp) {
    const r = await ruleOf(sp);
    // The last 90 days (a review copy older than that is not listed: its author sends it again).
    const list = await items.list({ board: sp.id }, "new", "post", { window: 90 });
    const pending = list.filter(p => blogOf(p).review === "pending");
    return Promise.all(pending.sort((a, b) => a.at - b.at).map(async p => ({ ...p, word: approvalOf(await items.thread(p.ref).catch(() => []), did => did !== p.by && r.can(did, "moderate")) })));
  }
  // AN EDITOR's WORD: a comment on the review copy (seen by its author and the other editors, kept with it).
  async function approve(ref, yes, note = "") {
    const sp = await items.boardOf(ref);
    if (!sp || !(await mayEdit(sp))) throw new Error("only this publication's editors approve");
    await items.comment(ref, null, `${yes ? APPROVED : DECLINED}${note ? ` — ${note}` : ""}`);
  }

  return { drafts, saveDraft, publish, publishDue, review, approve, mayEdit, needsReview, isArticle, readingTime, headingsOf, isDue, approvalOf, coverOf };
}
