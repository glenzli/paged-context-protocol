// Owns view location and tab accessibility; data loading remains with each view.
export function createConsoleNavigation({ tabs, views, host, onNavigate, onError }) {
  const names = new Set(tabs.map(tab => tab.dataset.view));
  const normalize = name => names.has(name) ? name : "overview";
  const locationView = () => normalize(host.location.hash.slice(1));
  let selectedView = null;
  function select(name, { history = true, focus = false } = {}) {
    name = normalize(name);
    const changed = selectedView !== null && selectedView !== name;
    selectedView = name;
    for (const tab of tabs) {
      const active = tab.dataset.view === name;
      tab.id = `tab-${tab.dataset.view}`;
      tab.setAttribute("role", "tab");
      tab.setAttribute("aria-controls", `view-${tab.dataset.view}`);
      tab.setAttribute("aria-selected", String(active));
      tab.tabIndex = active ? 0 : -1;
      tab.classList.toggle("active", active);
      if (active && focus) tab.focus();
      if (active) tab.scrollIntoView?.({ block: "nearest", inline: "nearest" });
    }
    for (const view of views) {
      const active = view.id === `view-${name}`;
      view.classList.toggle("active", active);
      view.hidden = !active;
      view.setAttribute("role", "tabpanel");
      view.setAttribute("aria-labelledby", view.id.replace("view-", "tab-"));
    }
    if (history && host.location.hash !== `#${name}`) host.history.pushState(null, "", `#${name}`);
    if (changed) host.scrollTo?.({ top: 0, behavior: "instant" });
    return name;
  }
  const navigate = (name, options) => Promise.resolve(onNavigate(name, options)).catch(onError);
  for (const [index, tab] of tabs.entries()) {
    tab.addEventListener("click", () => navigate(tab.dataset.view));
    tab.addEventListener("keydown", event => {
      if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
      event.preventDefault();
      const target = event.key === "Home" ? 0 : event.key === "End" ? tabs.length - 1
        : (index + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length;
      navigate(tabs[target].dataset.view, { focus: true });
    });
  }
  host.addEventListener("hashchange", () => navigate(locationView(), { history: false }));
  return { select, locationView };
}
