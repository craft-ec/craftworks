// PLACES, a component: what a SPACE is made of, as the top bar shows it — its MESSAGES (a server's channels in Chat, a
// conversation in Messages), its BOARD (posts) and its NOTES (shared): one space, the same members, roles and
// moderation, three places. Every page showing a space puts these first in its top bar. The PERSONAL space (the
// account) too: its messages are Messages (your conversations), its board your profile (public: your followers read
// it), its notes your Notes. UI only: the space is `space`'s.
//
//   const places = await ctx.require("places");
//   places.of(sp, "messages" | "board" | "notes")   // top-bar actions: [{ label, href, on }]
export async function start() {
  const of = (sp, here) =>
    sp.kind === "account"
      ? [
          { label: "Messages", href: "#/messages", on: here === "messages" },
          { label: "Board", href: `#/board/u/${sp.id}`, on: here === "board" },
          { label: "Notes", href: "#/notes", on: here === "notes" },
        ]
      : [
          { label: "Messages", href: sp.kind === "server" ? `#/chat/${sp.id}` : `#/messages/${sp.id}`, on: here === "messages" },
          { label: "Board", href: `#/board/b/${sp.id}`, on: here === "board" },
          { label: "Notes", href: `#/notes/s/${sp.id}`, on: here === "notes" },
        ];
  return { of };
}
