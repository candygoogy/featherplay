// Before the first paint: the stored theme, else the system's, else dark. Base.astro loads this
// as a classic, blocking script in <head> (a file, so the Content-Security-Policy allows no
// inline script), copied as written: plain JavaScript, no imports.
(() => {
  let t;
  try {
    t = localStorage.getItem("fp-theme");
  } catch (e) {}
  if (t !== "dark" && t !== "light") t = matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
  document.documentElement.dataset.theme = t;
  document.documentElement.classList.add("js", "booting");
  // The scroll story's pinned stage (src/scripts/story.ts STORY), before the first paint, so
  // the page's height doesn't change under the visitor. Only a promise: the story's own script
  // keeps it (data-story-live). If the page's scripts never ran (blocked, failed to load), by
  // the time the page has loaded it is the page without script: no pinned stage, and the
  // window shown as the still it is without script, not hidden waiting to be placed.
  if (matchMedia("(min-width: 761px) and (min-height: 600px) and (prefers-reduced-motion: no-preference)").matches)
    document.documentElement.dataset.story = "";
  addEventListener("load", () => {
    const r = document.documentElement;
    const placed = document.querySelector("[data-window].placed");
    if (!placed || !r.hasAttribute("data-story-live")) delete r.dataset.story;
    if (!placed && document.querySelector("[data-window]")) r.classList.remove("js");
  });
  // Nothing eases until the styles are in (base.css): DOMContentLoaded comes after the module
  // scripts, which wait for the stylesheets; then two frames.
  addEventListener("DOMContentLoaded", () =>
    requestAnimationFrame(() => requestAnimationFrame(() => document.documentElement.classList.remove("booting"))),
  );
  // Loading part-way down: a reload or back/forward to where the hero last left the page
  // scrolled (src/scripts/lid.ts keeps it), or a link to a #section. The hero then shows
  // nothing of its first-load motion and waits to be placed (Hero.astro).
  try {
    const nav = performance.getEntriesByType("navigation")[0];
    const again = nav && (nav.type === "reload" || nav.type === "back_forward");
    const left = again ? Number(sessionStorage.getItem("fp-scroll:" + location.pathname)) : 0;
    const hash = location.hash.slice(1);
    if (left > 4) document.documentElement.dataset.land = "restore";
    else if (!again && hash && hash !== "top") document.documentElement.dataset.land = "hash";
  } catch (e) {}
})();
