// Whether the page is still settling where the browser put it as it loaded (a load at a
// #section, a reload or a step back partway down, the scroll the browser restores a moment after
// the first paint): until the visitor first moves it (a wheel, a touch, a key, a press) or
// shortly after the load, whichever is first. Then what follows the scroll (the bar's capsule,
// the desk's calm) takes its state at once, with no glide or fade to watch; afterwards, however
// the page moves (the scrollbar too), it moves with it.

let moved = false;
const mark = () => {
  moved = true;
  for (const t of TYPES) removeEventListener(t, mark, true);
};
const TYPES = ["wheel", "touchstart", "keydown", "pointerdown"] as const;
for (const t of TYPES) addEventListener(t, mark, { capture: true, passive: true });
const SETTLE_MS = 400;
if (document.readyState === "complete") setTimeout(mark, SETTLE_MS);
else addEventListener("load", () => setTimeout(mark, SETTLE_MS), { once: true });

// Where the page was left, for a reload or a step back to it (boot.js reads it before the first
// paint). This tab only; nothing leaves the browser.
addEventListener("pagehide", () => {
  try {
    sessionStorage.setItem(`fp-y:${location.pathname}`, String(Math.round(scrollY)));
  } catch {
    // Storage off: the page lands as it did.
  }
});

/** True once the page has settled: the visitor has moved it, or the load is past. */
export const visitorMoved = (): boolean => moved;
