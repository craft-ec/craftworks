// THE RUNTIME's HELPERS (FOUNDATION §3, "THE FRAME RUNTIME"): what every package reads on its `ctx` to handle a
// failure, keep a cache, run a view's clock and draw a screen — ONE implementation, installed by the page's loader on the
// page's ctx and by an app's frame on its own (as a phone's UI frameworks are one library loaded into every app).
//
//   install(ctx, { handled(source, e) → kind, stateOf(kind) → read state, theme() → Promise<theme> })
export function install(ctx, { handled, stateOf, theme }) {
  const require = name => (name === "theme" ? theme() : ctx.require(name));
  //   A DECISION that must not act on "unknown" (a join, a removal, a claim, a copy) reads `.strict` instead: the same
  //   read, throwing its fault — the step stops rather than treat a failed read as an empty one (review 10-13: a failed
  //   `space.mine()` read as "no spaces" re-keyed every Drive file; a failed `directory.devices` wrote a ban with no
  //   nodes). Services read strict; views read total.
  // A COPY to the clipboard (the bar's Share): in the page, the clipboard itself; an app's frame has its own (`copy`:
  // through the host, on a gesture) and keeps it.
  ctx.copy ??= text => navigator.clipboard.writeText(String(text));
  ctx.total = (what, empty, read) =>
    Object.assign(
      (...args) =>
        Promise.resolve()
          .then(() => read(...args))
          .catch(e => {
            const kind = handled(what, e);
            const v = empty();
            if (v && typeof v === "object") Object.defineProperty(v, "state", { value: { s: stateOf(kind), why: String(e?.message ?? e).slice(0, 120) }, enumerable: false });
            return v;
          }),
      { strict: (...args) => Promise.resolve().then(() => read(...args)) },
    );
  ctx.background = (what, p) => Promise.resolve(p).catch(e => void handled(what, e));
  ctx.optional = (what, f, fallback) =>
    Promise.resolve()
      .then(() => (typeof f === "function" ? f() : f))
      .catch(e => (handled(what, e), typeof fallback === "function" ? fallback() : fallback));
  // A CLOCK FOR WHAT IS SHOWN (FOUNDATION §7.4): `ctx.every(el, ms, fn)` — `fn` every `ms` while `el` is on the page
  // (once it was), stopped when it leaves (or by the stop it returns); `el` null: the page's own. The ONE timer for a
  // view; background work is a lifecycle JOB (`lifecycle.job`). `tools/job-check.mjs` fails any other setInterval.
  ctx.every = (el, ms, fn) => {
    let shown = false;
    const t = setInterval(() => {
      if (el && !el.isConnected) return shown ? clearInterval(t) : undefined;
      shown = true;
      fn();
    }, ms);
    return () => clearInterval(t);
  };
  // A VIEW (FOUNDATION §3 `view`): ONE BEHAVIOUR for every screen, the app's own TEMPLATE. `ctx.view(el, { read, draw,
  // empty?, looks? })`: `read()` what the screen needs (a capability's total read carries its read state); `draw(data)`
  // the app's own elements. The view: the loading look while not read (after 300 ms — a quick screen never flickers);
  // the empty look only for an ANSWERED nothing (`empty(data)`, default: an empty list); "still arriving" for a read not
  // answered yet (it stays and redraws when it comes); a failure in its kind's words with Try again; and on every DATA
  // CHANGE (`craftworks:data`: a push, a write) read again and drawn again — only if what it read differs, and never
  // while the person is typing inside it (drawn once they leave the field). `looks: { loading, empty, notRead, failed }`:
  // the app's own (each `() => element` or text), the theme's by default. Returns `{ refresh(), stop() }`.
  // `{ render }` in place of `{ read, draw }`: a screen that builds itself in one step (its reads and its elements
  // interleaved) — built again on each data change, put in place only when what it built differs (the same rules).
  // `hold()`: true while the screen must not be drawn again (an editor open in it, focused or not) — as the typing.
  ctx.view = (el, { read, draw, render = null, hold = () => false, empty = d => Array.isArray(d) && d.length === 0, looks = {} }) => {
    if (render) ((read = render), (draw = n => n), (empty = () => false));
    let seen = null; // what was last drawn (its JSON), so an unchanged read draws nothing
    let running = null;
    let again = false;
    let stopped = false;
    const look = async (k, text) => {
      const own = looks[k];
      if (own) return typeof own === "function" ? own() : Object.assign(document.createElement("p"), { className: "none", textContent: own });
      if (k === "loading") return (await require("theme")).loading(text);
      return Object.assign(document.createElement("p"), { className: k === "failed" ? "said" : "none", textContent: text });
    };
    const typing = () => {
      if (hold()) return true;
      const a = document.activeElement;
      return !!a && el.contains(a) && (a.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(a.tagName));
    };
    const once = async () => {
      const slow = setTimeout(async () => !seen && el.replaceChildren(await look("loading", "Loading…")), 300);
      let data;
      try {
        data = await read();
      } catch (e) {
        clearTimeout(slow);
        const b = Object.assign(document.createElement("button"), { type: "button", className: "cw-btn", textContent: "Try again", onclick: () => refresh() });
        seen = null;
        return el.replaceChildren(await look("failed", ctx.say(e)), b);
      }
      clearTimeout(slow);
      if (stopped || !el.isConnected) return;
      const st = data?.state?.s;
      const key = (() => {
        try {
          if (data instanceof Node) return data.outerHTML ?? data.textContent; // (a built screen: as it reads)
          return JSON.stringify([st ?? null, data]);
        } catch {
          return String(Math.random());
        }
      })();
      if (key === seen) return;
      if (typing()) return void (again = true); // drawn when they leave the field
      seen = key;
      if (st === "unknown" && empty(data)) return el.replaceChildren(await look("notRead", "Still arriving…"));
      if (st === "failed") return el.replaceChildren(await look("failed", data.state.why ?? "Could not be read"));
      if (empty(data)) return el.replaceChildren(await look("empty", "Nothing here yet."));
      const out = await draw(data);
      if (out instanceof Node && out !== el) el.replaceChildren(out);
    };
    const refresh = () => {
      if (stopped) return Promise.resolve();
      if (running) return void (again = true), running;
      running = once()
        .catch(e => handled("view", e))
        .finally(() => ((running = null), again && !stopped && ((again = false), refresh())));
      return running;
    };
    const onData = () => (el.isConnected ? refresh() : stop());
    const onLeave = () => again && setTimeout(refresh, 0); // (after focus has moved)
    addEventListener("craftworks:data", onData);
    el.addEventListener("focusout", onLeave);
    const stop = () => ((stopped = true), removeEventListener("craftworks:data", onData), el.removeEventListener("focusout", onLeave));
    refresh();
    return { refresh, stop };
  };
  // A CACHE that never holds a failure (FOUNDATION §2 R4): `ctx.cache(map, key, make, fallback?)` — the promise kept
  // while it is wanted; one that FAILS leaves the map at once (asked for again, it is made again) and answers with
  // `fallback` if one is given (a view's empty), else its fault. One implementation for every per-page cache.
  ctx.cache = (map, key, make, ...fallback) => {
    if (map.has(key)) return map.get(key);
    const p = Promise.resolve()
      .then(make)
      .catch(e => {
        if (map.get(key) === p) map.delete(key);
        if (!fallback.length) return Promise.reject(e);
        handled(`cache: ${String(key).slice(0, 40)}`, e);
        return typeof fallback[0] === "function" ? fallback[0]() : fallback[0];
      });
    map.set(key, p);
    return p;
  };
}
