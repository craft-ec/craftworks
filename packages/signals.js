// SIGNALS, a capability: what a person's INTERACTION with an item is — the one catalog every count, rank and check
// reads (as `kinds` is for what an item IS). A signal is kept as a REACTION where its giver's vote would be (the item's
// space's room, or their own profile pointed in the space's bag from outside): one implementation writes, counts,
// windows and seals them all, and a new signal is one line here.
// - `mark`: the reaction it is kept as (a vote: two, one per side).
// - `value`: "once" (one per person: a view, a save, a share), "updown" (+1 or −1: a vote), "many" (several values per
//   person, each counted: an emoji REACTION — any emoji not another signal's mark, its value the emoji; a tag someone
//   puts on another's item — its value the tag).
// - `action`: the POLICY action it is checked against — inherited as every policy is (the item's own rule, else its
//   domain's in its space, else the space's). A signal with its OWN action names its parent (`inherits`): where no
//   rule is set for it, its parent's applies — save and share as who may READ, react and tag as who may VOTE; a view is
//   who may read. Every settings page lists these (`policyActions`), so a new one appears there by itself.
// - `counted`: shown as a count (on cards, an item's page) to whoever reads the item; `noun`: its words.
// - `point`: an outsider's points them in the space's bag (so the space finds it); `activity`: counts toward Hot and
//   Rising.
//
//   const signals = await ctx.require("signals");
//   signals.of("save")          // { id, mark, value, action, counted, noun, point, activity } or null
//   signals.byMark("👁")         // the signal a reaction is ({ id: "view", … }), or null
//   signals.all()               // every signal
//   signals.summary(item)       // " · 3 views · 1 save" — its counted signals, in one wording everywhere
export async function start() {
  const CATALOG = [
    { id: "vote", mark: ["▲", "▼"], value: "updown", action: "vote", counted: false, point: true, activity: true },
    { id: "view", mark: "👁", value: "once", action: "read", counted: true, noun: ["view", "views"], point: true, activity: false },
    { id: "save", mark: "★", value: "once", action: "save", inherits: "read", label: "Who may save", counted: true, noun: ["save", "saves"], point: true, activity: true },
    { id: "share", mark: "↗", value: "once", action: "share", inherits: "read", label: "Who may share", counted: true, noun: ["share", "shares"], point: true, activity: true },
    // A REACTION: an emoji (`counts.react`: emoji → how many) — Chat's messages and every kind alike. `choices`: the
    // ones offered (any other emoji kept as given).
    { id: "react", mark: "", value: "many", action: "react", inherits: "vote", label: "Who may react", counted: false, point: true, activity: true, choices: Object.freeze(["👍", "❤️", "😂", "🎉", "😮", "🙏"]) },
    // A TAG someone puts on an item (theirs or another's): each value counted (`counts.tag`: tag → how many).
    { id: "tag", mark: "#", value: "many", action: "tag", inherits: "vote", label: "Who may tag", counted: false, point: true, activity: true },
  ].map(s => Object.freeze(s));
  const byId = new Map(CATALOG.map(s => [s.id, s]));
  const byMark = new Map(CATALOG.filter(s => s.mark).flatMap(s => [s.mark].flat().map(m => [m, s])));
  const REACT = CATALOG.find(s => s.id === "react");
  // A "many" signal's reaction carries its value after the mark (`#rust`); any other emoji is a REACTION.
  const markOf = e => (byMark.has(e) ? e : ([...byMark.keys()].find(m => byMark.get(m).value === "many" && String(e).startsWith(m)) ?? null));
  const isEmoji = e => /\p{Extended_Pictographic}/u.test(String(e ?? ""));
  const signalOf = e => byMark.get(markOf(e)) ?? (isEmoji(e) ? REACT : null);
  return {
    all: () => CATALOG,
    of: id => byId.get(id) ?? null,
    // The policy actions signals add, each with its parent and its words: [{ action, inherits, label }].
    policyActions: () => CATALOG.filter(s => s.inherits).map(s => ({ action: s.action, inherits: s.inherits, label: s.label })),
    // An action's parent where no rule is set for it (or null).
    parentOf: action => CATALOG.find(s => s.inherits && s.action === action)?.inherits ?? null,
    byMark: e => signalOf(e),
    // The VALUE a reaction carries: a vote's +1/−1, a many-signal's text (a reaction: its emoji), else true.
    valueOf: e => {
      const s = signalOf(e);
      if (!s) return null;
      if (s.value === "updown") return e === s.mark[0] ? 1 : -1;
      if (s.value === "many") return String(e).slice(s.mark.length);
      return true;
    },
    // The reaction a signal (and its value) is kept as.
    markFor: (id, value = true) => {
      const s = byId.get(id);
      if (!s) throw new Error(`no such signal: ${id}`);
      if (s.value === "updown") return value > 0 ? s.mark[0] : s.mark[1];
      if (s.id === "react") return String(value);
      if (s.value === "many") return `${s.mark}${String(value).replace(/\s+/g, "-").toLowerCase().slice(0, 40)}`;
      return s.mark;
    },
    summary: it =>
      CATALOG.filter(s => s.counted && it?.counts?.[s.id])
        .map(s => ` · ${it.counts[s.id]} ${s.noun[it.counts[s.id] === 1 ? 0 : 1]}`)
        .join(""),
  };
}
