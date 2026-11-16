// The page's first layout, split. On a fresh load at the top (boot.js marks it "split"), the
// sections below the first screen start out laid out only when needed (content-visibility:
// auto, base.css); right after the first paint they are released one per idle moment, so each
// section's layout is a small task of its own instead of one long task for the whole page.
// Once all are released nothing waits to render: no estimate moves a link's target, no skipped
// section keeps the page working on every frame. A link to a place on the page releases the
// rest at once (src/scripts/anchors.ts); a load at a #section, a reload or a step back or
// forward isn't split at all.

const WAITING = "main[data-split] > section:not([data-stage]), main[data-split] ~ footer";
let queue: HTMLElement[] = [];

const release = (el: HTMLElement) => el.toggleAttribute("data-released", true);

/** Every section still waiting, now. */
export function releaseAll(): void {
  queue.splice(0).forEach(release);
}

export function releaseSections(): void {
  if (!document.documentElement.classList.contains("split")) return;
  queue = [...document.querySelectorAll<HTMLElement>(WAITING)];
  const idle = (fn: () => void) => ("requestIdleCallback" in window ? requestIdleCallback(fn, { timeout: 400 }) : setTimeout(fn, 60));
  const next = () => {
    const el = queue.shift();
    if (!el) return;
    release(el);
    idle(next);
  };
  // After the first paint.
  requestAnimationFrame(() => setTimeout(() => idle(next), 0));
}
