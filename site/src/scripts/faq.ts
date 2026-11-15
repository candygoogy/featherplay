// Questions (Questions.astro): native <details>, so it works without script and with the
// keyboard. With script and motion an answer opens and closes by easing its height (Web
// Animations, the same in every engine), and the question clicked never moves: several
// answers may be open at once (nothing above the question is ever closed for it), the list
// opts out of the browser's scroll anchoring (which picks some line below and moves the
// clicked question to keep that line still), and while the height eases the page is scrolled
// by whatever the question would have moved. Near the end of the page, where a closing answer
// would pull the page up, the list keeps its height until it's out of view.
// A link to a question (#official, #connects, #uninstall) opens its answer.

const MS = 280;
const EASE = "cubic-bezier(0.22, 1, 0.36, 1)";

export function bindFaq(root: HTMLElement): void {
  const reduce = matchMedia("(prefers-reduced-motion: reduce)");
  let held = false;
  const release = new IntersectionObserver(([e]) => {
    if (!e.isIntersecting && held) {
      held = false;
      root.style.minHeight = "";
    }
  });
  release.observe(root);

  for (const d of root.querySelectorAll<HTMLDetailsElement>("details")) {
    const summary = d.querySelector<HTMLElement>("summary")!;
    const body = d.querySelector<HTMLElement>("[data-answer]")!;
    summary.addEventListener("click", (e) => {
      if (reduce.matches) return;
      e.preventDefault();
      const top = summary.getBoundingClientRect().top;
      const from = d.open ? body.getBoundingClientRect().height : 0;
      for (const a of body.getAnimations()) a.cancel();
      const opening = !d.open || d.dataset.closing === "true";
      let to = 0;
      if (opening) {
        delete d.dataset.closing;
        d.open = true;
        to = body.scrollHeight;
      } else {
        d.dataset.closing = "true";
        // At the page's end the page would be pulled up as the list shrinks: hold its height.
        const doc = document.documentElement;
        if (scrollY + innerHeight > doc.scrollHeight - from - 2) {
          root.style.minHeight = `${root.offsetHeight}px`;
          held = true;
        }
      }
      const anim = body.animate([{ height: `${from}px` }, { height: `${to}px` }], { duration: MS, easing: EASE });
      body.style.overflow = "clip";
      // The question stays where it was clicked.
      const keep = () => {
        const dy = summary.getBoundingClientRect().top - top;
        if (Math.abs(dy) > 0.5) scrollBy(0, dy);
        if (anim.playState === "running") requestAnimationFrame(keep);
      };
      requestAnimationFrame(keep);
      anim.finished
        .then(() => {
          body.style.overflow = "";
          if (d.dataset.closing === "true") {
            delete d.dataset.closing;
            d.open = false;
          }
          keep();
        })
        .catch(() => {});
    });
  }

  const openTarget = () => {
    const id = decodeURIComponent(location.hash.slice(1));
    const target = id ? document.getElementById(id) : null;
    if (target instanceof HTMLDetailsElement && root.contains(target)) target.open = true;
  };
  addEventListener("hashchange", openTarget);
  openTarget();
}
