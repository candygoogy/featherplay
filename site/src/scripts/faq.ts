// Questions (Questions.astro): native <details> sharing one name, so without script the browser
// keeps one answer open at a time where it can (and they open independently where it can't),
// and the keyboard works. With script the script keeps the one-at-a-time rule itself, so it can
// ease both answers together: opening a question closes the one that was open, the two heights
// eased on one clock (Web Animations, the same in every engine). The page itself never scrolls
// for it: when the answer above closes, the question clicked rises with the list, as the rest of
// the page stays where it is. (The list opts out of the browser's scroll anchoring, which would
// scroll the page to keep some line below still.) Only a question that would rise out of sight
// under the bar is held at the reading line, by scrolling just that much. Near the end of the
// page, where a closing answer would pull the page up, the list keeps its height until it's out
// of view. Reduced motion: the same swap at once. Clicking the open question closes it, leaving
// none open.
// A link to a question (#official, #connects, #uninstall) opens its answer, at the click.

const MS = 280;
const EASE = "cubic-bezier(0.22, 1, 0.36, 1)";
/** A question never rises above this line (under the bar) when the answer above it closes. */
const LINE = 84;

export function bindFaq(root: HTMLElement): void {
  const reduce = matchMedia("(prefers-reduced-motion: reduce)");
  const all = [...root.querySelectorAll<HTMLDetailsElement>("details")];
  // The rule is the script's from here (the browser's would shut the other one at once).
  for (const d of all) d.removeAttribute("name");

  let held = false;
  const release = new IntersectionObserver(([e]) => {
    if (!e.isIntersecting && held) {
      held = false;
      root.style.minHeight = "";
    }
  });
  release.observe(root);

  const bodyOf = (d: HTMLDetailsElement) => d.querySelector<HTMLElement>("[data-answer]")!;
  const isOpen = (d: HTMLDetailsElement) => d.open && d.dataset.closing !== "true";

  /** Eases one answer to open or shut; resolves when it has. */
  function ease(d: HTMLDetailsElement, open: boolean, ms: number): Promise<void> {
    const body = bodyOf(d);
    const from = d.open ? body.getBoundingClientRect().height : 0;
    for (const a of body.getAnimations()) a.cancel();
    let to = 0;
    if (open) {
      delete d.dataset.closing;
      d.open = true;
      to = body.scrollHeight;
    } else {
      d.dataset.closing = "true";
    }
    if (!ms) {
      if (!open) {
        delete d.dataset.closing;
        d.open = false;
      }
      return Promise.resolve();
    }
    const anim = body.animate([{ height: `${from}px` }, { height: `${to}px` }], { duration: ms, easing: EASE });
    body.style.overflow = "clip";
    return anim.finished
      .then(() => {
        body.style.overflow = "";
        if (d.dataset.closing === "true") {
          delete d.dataset.closing;
          d.open = false;
        }
      })
      .catch(() => {});
  }

  for (const d of all) {
    const summary = d.querySelector<HTMLElement>("summary")!;
    summary.addEventListener("click", (e) => {
      e.preventDefault();
      const ms = reduce.matches ? 0 : MS;
      const floor = Math.min(summary.getBoundingClientRect().top, LINE);
      const opening = !isOpen(d);
      const others = opening ? all.filter((o) => o !== d && isOpen(o)) : [];
      // At the page's end the page would be pulled up as the list shrinks: hold its height.
      const closing = opening ? others : [d];
      if (closing.length) {
        const doc = document.documentElement;
        const shrink = closing.reduce((n, o) => n + bodyOf(o).getBoundingClientRect().height, 0);
        if (scrollY + innerHeight > doc.scrollHeight - shrink - 2) {
          root.style.minHeight = `${root.offsetHeight}px`;
          held = true;
        }
      }
      // One clock for both.
      const done = Promise.all([ease(d, opening, ms), ...others.map((o) => ease(o, false, ms))]);
      // The page stays put and the question moves with the list; it is only kept from rising
      // out of sight under the bar (a long answer closing above it).
      let running = true;
      const keep = () => {
        const dy = summary.getBoundingClientRect().top - floor;
        if (dy < -0.5) scrollBy({ top: dy, behavior: "instant" });
        if (running) requestAnimationFrame(keep);
      };
      keep();
      done.then(() => {
        running = false;
        keep();
      });
    });
  }

  const openAt = (hash: string) => {
    const id = decodeURIComponent(hash.slice(1));
    const target = id ? document.getElementById(id) : null;
    if (!(target instanceof HTMLDetailsElement) || !root.contains(target)) return;
    for (const o of all) if (o !== target && o.open) o.open = false;
    target.open = true;
  };
  const openTarget = () => openAt(location.hash);
  addEventListener("hashchange", openTarget);
  openTarget();
  // A link on this page to a question opens its answer as it is clicked, before the browser
  // sets off for it: opened a moment later (on hashchange), it changed the page under the
  // glide, and Firefox's scroll anchoring then shifted the glide's end or stopped it.
  document.addEventListener(
    "click",
    (e) => {
      const link = (e.target as Element | null)?.closest?.("a[href*='#']");
      if (link instanceof HTMLAnchorElement && link.origin === location.origin && link.pathname === location.pathname) openAt(link.hash);
    },
    true,
  );

  // The index marks the group in view.
  const index = new Map<Element, HTMLAnchorElement>();
  for (const a of document.querySelectorAll<HTMLAnchorElement>("[data-faq-index] a")) {
    const g = document.getElementById(a.hash.slice(1));
    if (g) index.set(g, a);
  }
  if (index.size && "IntersectionObserver" in window) {
    const shown = new Set<Element>();
    const mark = () => {
      // The first group (in order) that reaches into the reading band.
      const at = [...index.keys()].find((g) => shown.has(g));
      for (const [g, a] of index) {
        if (g === at) a.setAttribute("aria-current", "true");
        else a.removeAttribute("aria-current");
      }
    };
    const io = new IntersectionObserver(
      (es) => {
        for (const e of es) e.isIntersecting ? shown.add(e.target) : shown.delete(e.target);
        mark();
      },
      { rootMargin: "-30% 0px -55% 0px" },
    );
    for (const g of index.keys()) io.observe(g);
  }
}
