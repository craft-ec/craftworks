// A SAMPLE APP, run in its own frame (FOUNDATION.md §3): what an app not ours can do — its own rows, where it can be,
// and moving the page — through the system API only. (A demo of the boundary, not an app anyone needs.)
export async function mount(api, el) {
  el.innerHTML = `<h2>Sample (framed)</h2><form><input name="t" placeholder="A line to keep" autocomplete="off"> <button>Keep</button></form><ul></ul><p class="w"></p>`;
  const ul = el.querySelector("ul");
  const draw = async () => {
    const rows = await api.rows();
    ul.replaceChildren(...rows.map(r => Object.assign(document.createElement("li"), { textContent: r.value })));
  };
  el.querySelector("form").onsubmit = async e => {
    e.preventDefault();
    const t = e.target.elements.t;
    if (!t.value.trim()) return;
    await api.put(`line-${Date.now()}`, t.value.trim()).catch(async err => (el.querySelector(".w").textContent = await api.say(err)));
    t.value = "";
    draw();
  };
  api.onChange(draw);
  const w = await api.where();
  el.querySelector(".w").textContent = `Here: ${w.route}${w.space ? ` in a space` : ""}`;
  await draw();
}
