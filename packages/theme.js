// THEME, a component: the one place the look is decided. Design TOKENS as CSS variables — colours, surfaces, lines,
// accent, spacing, radius, type, shadows — light and dark (following the system), and base styles for text, links,
// buttons and inputs. The manifest names the app's theme and the loader applies it before anything mounts. Every other
// component and page uses the tokens (`var(--cw-…)`), never colours or sizes of its own: a new look, or a second
// theme, is one package.
//
// Tokens:
//   --cw-font --cw-font-mono --cw-text-sm --cw-text-xs
//   --cw-bg --cw-fg --cw-muted --cw-line --cw-surface --cw-hover --cw-pressed --cw-scrim
//   --cw-accent --cw-accent-fg --cw-selected --cw-on-selected --cw-danger --cw-on-pastel
//   --cw-space-1…5 --cw-radius-sm --cw-radius --cw-radius-pill --cw-shadow --cw-shadow-lg
export async function start() {
  const style = document.createElement("style");
  style.dataset.theme = "craftworks";
  style.textContent = `
    :root {
      color-scheme: light dark;
      --cw-font: system-ui, -apple-system, "Segoe UI", sans-serif;
      --cw-font-mono: ui-monospace, "SF Mono", Menlo, monospace;
      --cw-text-sm: .875rem;
      --cw-text-xs: .75rem;

      --cw-bg: #ffffff;
      --cw-fg: #1f1f1f;
      --cw-muted: #5f6368;
      --cw-line: #dadce0;
      --cw-surface: #ffffff;
      --cw-hover: #00000010;
      --cw-pressed: #0000001c;
      --cw-scrim: #00000080;
      --cw-accent: #1a73e8;
      --cw-accent-fg: #ffffff;
      --cw-selected: #feefc3;
      --cw-on-selected: #202124;
      --cw-danger: #b3261e;
      /* Text on a light, person-chosen colour (a note's): stays dark in both modes. */
      --cw-on-pastel: #202124;

      --cw-space-1: 4px;
      --cw-space-2: 8px;
      --cw-space-3: 12px;
      --cw-space-4: 16px;
      --cw-space-5: 24px;
      --cw-radius-sm: 4px;
      --cw-radius: 8px;
      --cw-radius-pill: 999px;
      --cw-shadow: 0 1px 3px #0000002e;
      --cw-shadow-lg: 0 4px 24px #0000004d;
    }
    @media (prefers-color-scheme: dark) {
      :root {
        --cw-bg: #202124;
        --cw-fg: #e8eaed;
        --cw-muted: #9aa0a6;
        --cw-line: #5f6368;
        --cw-surface: #2d2e31;
        --cw-hover: #ffffff14;
        --cw-pressed: #ffffff24;
        --cw-scrim: #000000a0;
        --cw-accent: #8ab4f8;
        --cw-accent-fg: #202124;
        --cw-selected: #41331c;
        --cw-on-selected: #fdd663;
        --cw-danger: #f2b8b5;
        --cw-shadow: 0 1px 3px #00000080;
        --cw-shadow-lg: 0 4px 24px #000000b3;
      }
    }
    html, body { background: var(--cw-bg); color: var(--cw-fg); font-family: var(--cw-font); }
    a { color: var(--cw-accent); }
    code, pre { font-family: var(--cw-font-mono); }
    button, input, textarea, select { font: inherit; color: inherit; }
    input:not([type="checkbox"]):not([type="radio"]), textarea, select {
      background: var(--cw-surface); border: 1px solid var(--cw-line); border-radius: var(--cw-radius-sm);
      padding: var(--cw-space-1) var(--cw-space-2);
    }
    dialog { background: var(--cw-surface); color: var(--cw-fg); }
    dialog::backdrop { background: var(--cw-scrim); }
    [hidden] { display: none !important; }
    /* THE SHELL, like an app on a phone: the window never scrolls. The header on top and the footer at the bottom stay;
       only the body between them scrolls. The whole window, edge to edge. A page that fills the body (its own
       panes scroll) marks its component .cw-fill. */
    html, body { height: 100%; overflow: hidden; }
    body { box-sizing: border-box; max-width: none; margin: 0; padding: 0 16px; }
    #app { height: 100dvh; display: flex; flex-direction: column; }
    #app > .slot-header { flex: none; padding-top: var(--cw-space-3); }
    #app > .slot-footer { flex: none; }
    #app > .slot-footer p { margin: 0; padding-bottom: var(--cw-space-2); }
    #app > .slot-body { flex: 1 1 auto; min-height: 0; overflow-y: auto; overscroll-behavior: contain; display: flex; flex-direction: column; }
    #app > .slot-body > section { flex: none; }
    #app > .slot-body > section.cw-fill { flex: 1 1 auto; min-height: 0; display: flex; flex-direction: column; }
    #app > .slot-body > section.cw-fill > * { flex: 1 1 auto; min-height: 0; }
    #status { position: fixed; left: 16px; bottom: 40px; margin: 0; }
    .cw-loading { display: grid; gap: var(--cw-space-2); padding: var(--cw-space-3) 0; color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .cw-loading i { display: block; height: .8em; border-radius: var(--cw-radius-sm); background: linear-gradient(90deg,
      var(--cw-hover) 25%, var(--cw-line) 50%, var(--cw-hover) 75%) 0 0 / 200% 100%; animation: cw-shimmer 1.4s linear infinite; }
    .cw-loading i:nth-of-type(2) { width: 80%; } .cw-loading i:nth-of-type(3) { width: 55%; }
    @keyframes cw-shimmer { to { background-position: -200% 0; } }
    @media (prefers-reduced-motion: reduce) { .cw-loading i { animation: none; } }`;
  document.head.append(style);
  // LOADING: what stands in place of something still on its way — a label and three shimmering lines. The one
  // placeholder: the loader's for a component, a component's for its own parts (a list, a room). `label` may change.
  function loading(label, lines = 3) {
    const box = document.createElement("div");
    box.className = "cw-loading";
    box.setAttribute("role", "status");
    box.setAttribute("aria-busy", "true");
    const said = document.createElement("span");
    said.textContent = label;
    box.append(said, ...Array.from({ length: lines }, () => document.createElement("i")));
    box.say = text => (said.textContent = text);
    return box;
  }
  return { loading };
}
