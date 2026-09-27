// HEADER: the app's name and its pages. The app's own component (the layout names it), so editing it is publishing
// the app, never the loader. It marks the current page when the route changes.
export function mount(ctx, el) {
  el.innerHTML = `<nav style="display:flex;gap:16px;align-items:baseline;border-bottom:1px solid #8884;padding-bottom:8px">
    <strong style="font-size:1.25rem"></strong>
    <a href="#/">Home</a>
    <a href="#/account">Account</a>
  </nav>`;
  el.querySelector("strong").textContent = ctx.app;
  const mark = () => {
    for (const a of el.querySelectorAll("a")) a.style.fontWeight = a.getAttribute("href") === `#${ctx.route}` ? "bold" : "normal";
  };
  mark();
  addEventListener("craftworks:route", mark);
}
