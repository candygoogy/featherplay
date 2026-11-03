// A link to a place on this page (the header's Questions, Back to top, the hero's Download)
// glides there: the root scrolls smoothly for that one jump. The page doesn't set
// scroll-behavior: smooth for good (base.css), so the browser's own jumps as a page loads (to
// a #section, or back to where a reload left it) are instant, and the hero's scroll moment
// isn't played through on the way. Reduced motion: every jump is instant.

let started = false;

export function bindAnchors(): void {
  if (started) return;
  started = true;
  const root = document.documentElement;
  let clear = 0;
  const done = () => {
    clearTimeout(clear);
    root.style.scrollBehavior = "";
  };
  // Bubbling to the document: anything that handles the click itself has had its turn.
  document.addEventListener("click", (e) => {
    if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    const link = (e.target as Element | null)?.closest?.("a[href*='#']");
    if (!(link instanceof HTMLAnchorElement) || !link.hash) return;
    if (link.origin !== location.origin || link.pathname !== location.pathname || link.search !== location.search) return;
    if (matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    root.style.scrollBehavior = "smooth";
    addEventListener("scrollend", done, { once: true });
    clearTimeout(clear);
    clear = window.setTimeout(done, 2500);
  });
}
