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
  // The app's own face (the replicas' --font-ui, tokens.css), looked up now, while the page is
  // still being read: the first lookup of a system font costs some 14 ms (Windows), and done
  // in the page's first layout it made that one long task.
  try {
    const c = document.createElement("canvas").getContext("2d");
    for (const w of [400, 600, 700]) {
      c.font = w + ' 12px "Segoe UI Variable Text", "Segoe UI", system-ui';
      c.measureText("featherplay 0:00");
    }
  } catch (e) {}
  // The scroll story's pinned stage (src/scripts/story.ts STORY), before the first paint, so
  // the page's height doesn't change under the visitor. Only a promise: the story's own script
  // keeps it (data-story-live). If the page's scripts didn't run (blocked, failed to load, or
  // one of them stopped by an error), soon after the page has loaded it is the page without
  // script: no pinned stage, and the first screen and its window shown as the still they are
  // without script, not hidden waiting to be placed. (The first screen is placed over the
  // first two frames, src/scripts/first.ts: the check waits for them.)
  if (matchMedia("(min-width: 761px) and (min-height: 600px) and (prefers-reduced-motion: no-preference)").matches)
    document.documentElement.dataset.story = "";
  const check = () => {
    const r = document.documentElement;
    const win = document.querySelector("[data-window]");
    const first = document.querySelector("[data-first]");
    const ran = (!win || win.classList.contains("placed")) && (!first || first.hasAttribute("data-shown"));
    if (!win || !ran || !r.hasAttribute("data-story-live")) delete r.dataset.story;
    if (!ran) r.classList.remove("js", "split");
  };
  addEventListener("load", () => requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(check, 0))));
  // Nothing eases until the styles are in (base.css): DOMContentLoaded comes after the module
  // scripts, which wait for the stylesheets; then two frames.
  addEventListener("DOMContentLoaded", () =>
    requestAnimationFrame(() => requestAnimationFrame(() => document.documentElement.classList.remove("booting"))),
  );
  // Loading at a link to a #section: the first screen shows nothing of its arrival
  // (src/scripts/first.ts). A reload or a step back or forward lands where the browser puts it.
  try {
    const nav = performance.getEntriesByType("navigation")[0];
    const again = nav && (nav.type === "reload" || nav.type === "back_forward");
    const hash = location.hash.slice(1);
    if (!again && hash && hash !== "top") document.documentElement.dataset.land = "hash";
    // A fresh load at the top: the page's first layout is split (src/scripts/release.ts). Not
    // where the browser puts the page somewhere down it as it loads: that needs every section
    // at its real height.
    // Not in Safari (WebKit): it doesn't keep the page still as sections above the view take
    // their real height, so a jump past them (a focus, a key) would move the page under the
    // visitor. There the first layout stays whole.
    const ua = navigator.userAgent;
    const webkit = /AppleWebKit/.test(ua) && !/Chrome|Chromium|Edg|OPR/.test(ua);
    if (!again && !(hash && hash !== "top") && !webkit) document.documentElement.classList.add("split");
  } catch (e) {}
})();
