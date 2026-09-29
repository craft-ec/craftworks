// PLACES, a component: what a SPACE is made of, as the top bar shows it — its MESSAGES (a server's channels in Chat, a
// conversation in Messages), its BOARD (posts) and its NOTES (shared): one space, the same members, roles and
// moderation, three places. Every page showing a space puts ONE dropdown first in its top bar: the space and the place
// open (`Kiln · Board ▾`), its list the space's places — switching places is a suite's move, kept out of the way. The PERSONAL space (the
// account) too: its messages are Messages (your conversations), its board your profile (public: your followers read
// it), its notes your Notes. UI only: the space is `space`'s.
//
//   const places = await ctx.require("places");
//   places.of(sp, "messages" | "board" | "notes")   // top-bar actions: [one menu]
export async function start() {
  const links = (sp, here) =>
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
  const NAMES = { messages: "Messages", board: "Board", notes: "Notes" };
  const of = (sp, here) => [{ label: `${sp.kind === "account" ? "Your space" : sp.name} · ${NAMES[here]}`, menu: links(sp, here) }];
  return { of };
}
