// A link to a place on this page (the bar's sections, Back to top, the first screen's
// Download) glides there: the root scrolls smoothly for that one jump. The page doesn't set
// scroll-behavior: smooth for good (base.css), so the browser's own jumps as a page loads (to
// a #section, or back to where a reload left it) are instant. Reduced motion: every jump is
// instant.
//
// The sections below the first screen wait to render (content-visibility, base.css) and hold
// an estimate of their height until they have. A glide's end is fixed as it starts, so for
// the jump every section is laid out at its real size (data-reach): the target is where the
// link says, and nothing moves under the glide. Once it is over each keeps its real height as
// its estimate (a section shown as visible leaves no remembered size behind), so nothing moves
// when they go back to waiting.

let started = false;

export function bindAnchors(): void {
  if (started) return;
  started = true;
  const root = document.documentElement;
  let clear = 0;
  let waiting: HTMLElement[] = [];
  const done = () => {
    clearTimeout(clear);
    root.style.scrollBehavior = "";
    if (!root.hasAttribute("data-reach")) return;
    for (const el of waiting) el.style.containIntrinsicSize = `auto ${el.offsetHeight}px`;
    root.removeAttribute("data-reach");
  };
  // Bubbling to the document: anything that handles the click itself has had its turn.
  document.addEventListener("click", (e) => {
    if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    const link = (e.target as Element | null)?.closest?.("a[href*='#']");
    if (!(link instanceof HTMLAnchorElement) || !link.hash) return;
    if (link.origin !== location.origin || link.pathname !== location.pathname || link.search !== location.search) return;
    // Laid out at their real sizes before the browser works out where the link goes.
    if (!root.hasAttribute("data-reach")) {
      waiting = [...document.querySelectorAll<HTMLElement>("main > section, main > * > section, body > footer")].filter((el) => getComputedStyle(el).contentVisibility === "auto");
      root.toggleAttribute("data-reach", true);
    }
    if (!matchMedia("(prefers-reduced-motion: reduce)").matches) root.style.scrollBehavior = "smooth";
    removeEventListener("scrollend", done);
    addEventListener("scrollend", done, { once: true });
    clearTimeout(clear);
    clear = window.setTimeout(done, 2500);
  });
}
