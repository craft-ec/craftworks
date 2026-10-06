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
  style.textContent = `/* THE CASCADE, in layers (owner 10-06): base (the theme) < apps (pages) < components (shared parts) < templates (a
   chosen look, last). A component's own look beats any page rule by LAYER, not by specificity: a page may place a
   component, never restyle it (the build's style check), and no selector tricks are needed to win. */
@layer base, apps, components, templates;
@layer base {

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
    /* THE SHARED PARTS — every app's, so a page looks the same wherever it is (an app adds only what is its own):
       one heading scale, one page width, one button family, one box. */
    :root { --cw-page: 860px; }
    h1 { font-size: 1.45rem; line-height: 1.3; font-weight: 700; margin: 0 0 var(--cw-space-2); }
    h2 { font-size: 1.2rem; line-height: 1.3; font-weight: 700; margin: 0 0 var(--cw-space-2); }
    h3 { font-size: 1rem; line-height: 1.35; font-weight: 600; margin: 0 0 var(--cw-space-1); }
    .cw-page-w { max-width: var(--cw-page); margin-inline: auto; width: 100%; }
    .cw-box { background: var(--cw-surface); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); padding: var(--cw-space-3); }
    .cw-btn { border: 1px solid var(--cw-line); background: none; color: var(--cw-fg); border-radius: var(--cw-radius-sm); padding: var(--cw-space-1) 10px;
      font-size: var(--cw-text-sm); font-weight: 600; cursor: pointer; }
    .cw-btn:hover { background: var(--cw-hover); }
    .cw-btn.primary, button.go { background: var(--cw-accent); color: var(--cw-accent-fg); border-color: transparent; }
    .cw-btn.danger { color: var(--cw-danger); }
    .cw-btn.chip { border-radius: var(--cw-radius-pill); padding: 2px 10px; font-size: var(--cw-text-xs); }
    .cw-btn:disabled { opacity: .5; cursor: default; }
    /* THE SHELL, like an app on a phone: the window and the canvas never scroll. The header on top and the footer at
       the bottom stay; between them, each component of the page is a PANEL that scrolls itself. The whole window, edge
       to edge. A page with panes of its own (each scrolling) marks its component .cw-fill: it gets the canvas bare. */
    html, body { height: 100%; overflow: hidden; }
    body { box-sizing: border-box; max-width: none; margin: 0; padding: 0; }
    #app { height: 100dvh; display: grid; grid-template: "head head" auto "side body" minmax(0, 1fr) "foot foot" auto / auto minmax(0, 1fr); }
    #app > .slot-header { grid-area: head; } #app > .slot-footer { grid-area: foot; } #app > .slot-body { grid-area: body; }
    #app > .slot-side { grid-area: side; min-height: 0; overflow: hidden; display: flex; }
    #app > .slot-side > section { margin: 0; display: flex; min-height: 0; position: relative; min-width: 64px; }
    /* On a phone the side (a layout's side slot, when one has any) folds away: the page has the whole width. */
    @media (max-width: 600px) {
      #app { grid-template: "head" auto "body" minmax(0, 1fr) "foot" auto / minmax(0, 1fr); }
      #app > .slot-side { position: fixed; top: var(--cw-bar); bottom: 0; left: 0; z-index: 40; background: var(--cw-bg); box-shadow: var(--cw-shadow-lg);
        transform: translateX(-110%); transition: transform .18s ease-out; }
      #app > .slot-side.open { transform: none; }
    }
    /* A side slot's placeholder: its bars only (no room for words). */
    #app > .slot-side > section > .cw-loading.cw-cover { padding: var(--cw-space-2); font-size: 0; }
    #app > .slot-side > section > .cw-loading.cw-cover i { height: 10px; }
    #app > .slot-side > section[aria-busy="true"]::before { content: ""; position: absolute; top: 0; left: 0; right: 0; z-index: 6; height: 2px;
      background: linear-gradient(90deg, transparent, var(--cw-accent), transparent) 0 0 / 40% 100% no-repeat; animation: cw-busy 1.1s ease-in-out infinite; }
    /* The two bars: a fixed height each, their content on the bar's middle line. */
    :root { --cw-bar: 48px; --cw-bar-low: 36px; --cw-gutter: 16px; }
    #app > .slot-header, #app > .slot-footer { flex: none; }
    #app > .slot-header > section, #app > .slot-footer > section { margin: 0; }
    #app > .slot-header .bar, #app > .slot-footer p { padding-left: var(--cw-gutter); padding-right: var(--cw-gutter); }
    #app > .slot-body { flex: 1 1 auto; min-height: 0; overflow: hidden; display: flex; flex-direction: column; }
    #app > .slot-body > section { flex: 1 1 auto; min-height: 0; margin: 0; overflow-y: auto; overscroll-behavior: contain;
      padding: var(--cw-space-3) var(--cw-gutter); box-sizing: border-box; }
    #app > .slot-body > section { position: relative; }
    /* A count of what is new (unread): the one badge. */
    .cw-badge { display: inline-block; min-width: 1.4em; padding: 0 .4em; margin-left: .4em; border-radius: 999px; background: var(--cw-accent);
      color: var(--cw-accent-fg); font-size: var(--cw-text-xs); font-weight: 700; line-height: 1.5; text-align: center; vertical-align: middle; }
    /* A panel still working after its first load: a thin moving line along its top. */
    #app > .slot-body > section[aria-busy="true"]::before { content: ""; position: sticky; top: 0; z-index: 6; display: block;
      flex: none; height: 2px; margin-bottom: -2px; background: linear-gradient(90deg, transparent, var(--cw-accent), transparent)
      0 0 / 40% 100% no-repeat; animation: cw-busy 1.1s ease-in-out infinite; }
    @keyframes cw-busy { from { background-position: -40% 0; } to { background-position: 140% 0; } }
    @media (prefers-reduced-motion: reduce) { #app > .slot-body > section[aria-busy="true"]::before { animation: none; background-size: 100% 100%; } }
    .cw-loading.cw-cover { position: absolute; inset: 0; z-index: 5; align-content: start; background: var(--cw-bg);
      padding: var(--cw-space-3) var(--cw-gutter); }
    #app > .slot-body > section.cw-fill { overflow: hidden; padding: 0; display: flex; flex-direction: column; }
    #app > .slot-body > section.cw-fill > * { flex: 1 1 auto; min-height: 0; }
    #status { position: fixed; left: 16px; bottom: 40px; margin: 0; }
    .cw-loading { display: grid; gap: var(--cw-space-2); padding: var(--cw-space-3) 0; color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .cw-loading i { display: block; height: .8em; border-radius: var(--cw-radius-sm); background: linear-gradient(90deg,
      var(--cw-hover) 25%, var(--cw-line) 50%, var(--cw-hover) 75%) 0 0 / 200% 100%; animation: cw-shimmer 1.4s linear infinite; }
    .cw-loading i:nth-of-type(2) { width: 80%; } .cw-loading i:nth-of-type(3) { width: 55%; }
    @keyframes cw-shimmer { to { background-position: -200% 0; } }
    @media (prefers-reduced-motion: reduce) { .cw-loading i { animation: none; } }
}`;
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
