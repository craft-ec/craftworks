// HEADER: Home on the left, Account on the right. Apps are reached from the desktop (Home), not from here. The app's
// own component (the layout names it), so editing it is publishing the app, never the loader.
export function mount(ctx, el) {
  el.innerHTML = `<nav style="display:flex;justify-content:space-between;align-items:baseline;border-bottom:1px solid #8884;padding-bottom:8px">
    <a href="#/">Home</a>
    <a href="#/account">Account</a>
  </nav>`;
  const mark = () => {
    for (const a of el.querySelectorAll("a")) a.style.fontWeight = a.getAttribute("href") === `#${ctx.route}` ? "bold" : "normal";
  };
  mark();
  addEventListener("craftworks:route", mark);
}
