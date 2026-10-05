// FEED BAR, a component: the one way every content app (Board, Videos, Audio, a composed app) chooses and orders what
// it lists — as Grid's. A FEED picks the set: New (by date, a month at a time, older as the end is scrolled to), Hot
// (every vote and comment), Best (net votes and comments), Rising (interactions per hour of age), Top (net votes, or
// comments) or Popular (how many people viewed it) — the ranked ones over TODAY, THIS WEEK or THIS MONTH, their counts
// over the same window (`items.list`). A SORT then only reorders what is shown: by time, votes, comments or views, ▲▼.
// What is read is that window, never all.
//
//   const bar = (await ctx.require("feed-bar")).create({ onChange: draw })
//   host.append(bar.el())                         // the controls (drawn again with each list)
//   await items.list(where, bar.sort(), kind, bar.options())   // the feed and its window
//   bar.reorder(list)                              // the shown set in the chosen sort
//   bar.older(label, shown)                        // New: a button reaching a month further back — and, while the last
//                                                  // month added items (`shown`: how many are listed), its end in view
//                                                  // reaches further on its own; an empty or stalled list waits for a click
//   bar.span()                                     // words for the window ("this week", "in the last 30 days")
export async function start() {
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  const style = document.createElement("style");
  style.textContent = `
    .cw-feedbar { display: flex; gap: var(--cw-space-2); align-items: center; flex-wrap: wrap; }
    .cw-feedbar .lbl { color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .cw-feedbar select, .cw-feedbar button { font: inherit; padding: 3px 8px; border-radius: var(--cw-radius-sm); border: 1px solid var(--cw-line);
      background: var(--cw-surface); color: var(--cw-fg); cursor: pointer; }
    .cw-feedbar-older { display: block; margin: var(--cw-space-3) auto; font: inherit; padding: 5px 14px; border-radius: 999px;
      border: 1px solid var(--cw-line); background: var(--cw-surface); color: var(--cw-fg); cursor: pointer; }`;
  document.head.append(style);

  function create({ onChange = () => {}, start = "new" } = {}) {
    const st = { sort: start, window: "week", by: "votes", reorder: "ranked", desc: true, months: 1, counts: [] };
    const changed = () => onChange(st);
    const pick = (options, value, set) => h("select", { onchange: e => (set(e.target.value), changed()) }, ...options.map(([v, t]) => h("option", { value: v, textContent: t, selected: v === value })));
    return {
      state: st,
      sort: () => st.sort,
      options: () => ({ window: st.sort === "new" ? 30 * st.months : st.window, by: st.by }),
      el: () =>
        h(
          "div",
          { className: "cw-feedbar" },
          h("span", { className: "lbl", textContent: "feed" }),
          pick([["new", "✨ New"], ["hot", "🔥 Hot"], ["best", "👍 Best"], ["rising", "📈 Rising"], ["top", "🏆 Top"], ["popular", "👁 Popular"]], st.sort, v => ((st.sort = v), (st.months = 1), (st.counts = []))),
          st.sort === "new" ? null : pick([["day", "Today"], ["week", "This week"], ["month", "This month"]], st.window, v => (st.window = v)),
          st.sort === "top" ? pick([["votes", "Top votes"], ["comments", "Top comments"]], st.by, v => (st.by = v)) : null,
          h("span", { className: "lbl", textContent: "sort" }),
          pick([["ranked", "Default"], ["time", "Time"], ["votes", "Votes"], ["comments", "Comments"], ["views", "Views"]], st.reorder, v => (st.reorder = v)),
          st.reorder === "ranked" ? null : h("button", { type: "button", title: "Direction", textContent: st.desc ? "▼" : "▲", onclick: () => ((st.desc = !st.desc), changed()) }),
        ),
      reorder: list => {
        if (st.reorder === "ranked") return list;
        const key = { time: p => p.at, votes: p => p.score ?? 0, comments: p => p.comments ?? 0, views: p => p.counts?.view ?? 0 }[st.reorder];
        return [...list].sort((a, b) => (st.desc ? key(b) - key(a) : key(a) - key(b)) || b.at - a.at);
      },
      // NEW reaches a month further back — on a click, or as the end comes into view, but only while reaching back
      // still finds items (the month before added some): an empty list, or one a month added nothing to, would
      // otherwise reach back forever on its own.
      older: (label = "Older", shown = 0) => {
        if (st.sort !== "new") return null;
        st.counts[st.months] = shown;
        const b = h("button", { type: "button", className: "cw-feedbar-older", textContent: label, onclick: () => ((st.months += 1), changed()) });
        const grew = shown > 0 && shown > (st.counts[st.months - 1] ?? 0);
        if (grew) new IntersectionObserver((es, io) => es.some(e => e.isIntersecting) && (io.disconnect(), b.click()), { rootMargin: "300px" }).observe(b);
        return b;
      },
      span: () => (st.sort === "new" ? `in the last ${30 * st.months} days` : { day: "today", week: "this week", month: "this month" }[st.window]),
    };
  }
  return { create };
}
