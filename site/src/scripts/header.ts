// The header (Header.astro): a bar at the top of the page, floating pills once the page has
// scrolled past FLOAT_AT, and the section on screen lit in the middle pill. Reading scrollY
// is all a scroll costs; the sections are watched by an IntersectionObserver on a line a
// third of the way down the window, so nothing is measured while the page moves.

/** Pixels scrolled before the bar becomes pills. */
export const FLOAT_AT = 24;

let started = false;

export function bindHeader(): void {
  if (started) return;
  started = true;
  const header = document.querySelector<HTMLElement>("[data-site-header]");
  if (!header) return;

  let floating: boolean | null = null;
  let frame = 0;
  const update = () => {
    frame = 0;
    const now = scrollY > FLOAT_AT;
    if (now === floating) return;
    floating = now;
    header.toggleAttribute("data-floating", now);
  };
  update();
  addEventListener(
    "scroll",
    () => {
      if (!frame) frame = requestAnimationFrame(update);
    },
    { passive: true },
  );

  // The section on screen: the one crossing a line a third of the way down the window.
  const links = new Map<string, HTMLAnchorElement>();
  for (const a of header.querySelectorAll<HTMLAnchorElement>("a[data-section]")) {
    const section = document.getElementById(a.dataset.section!);
    if (section) links.set(section.id, a);
  }
  if (!links.size || !("IntersectionObserver" in window)) return;
  const crossing = new Set<string>();
  const light = () => {
    // The last in page order, if two meet the line at once.
    let current: string | null = null;
    for (const id of links.keys()) if (crossing.has(id)) current = id;
    for (const [id, a] of links) {
      if (id === current) a.setAttribute("aria-current", "true");
      else a.removeAttribute("aria-current");
    }
  };
  const seen = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (e.isIntersecting) crossing.add(e.target.id);
        else crossing.delete(e.target.id);
      }
      light();
    },
    { rootMargin: "-33% 0px -66% 0px" },
  );
  for (const id of links.keys()) seen.observe(document.getElementById(id)!);
}
