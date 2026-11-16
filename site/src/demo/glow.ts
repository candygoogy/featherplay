// The playing song's colour on the page (--song). It is set on a few colour roots (each
// section, the bar, the footer, the selection's layers), never on the root: a change there
// would restyle every element of the page once, a frame of 80 ms at 4x CPU in every song change.
// A root in or near view (a screen ahead) takes the new colour at once, and what paints it in
// there fades to it (system.css --paint); one further away takes it as it comes within a
// screen, while it is still marked out of view, so with no fade. The replicas set their own.

const ROOTS = "main > section, [data-site-bar], body > footer, .sel-layer";

let current = "";
let io: IntersectionObserver | null = null;
const near = new WeakSet<Element>();
const watched = new WeakSet<Element>();

function apply(el: HTMLElement) {
  if (current && el.style.getPropertyValue("--song") !== current) el.style.setProperty("--song", current);
}

function watch() {
  if (!("IntersectionObserver" in window)) return;
  io ??= new IntersectionObserver(
    (es) => {
      for (const e of es) {
        if (e.isIntersecting) {
          near.add(e.target);
          apply(e.target as HTMLElement);
        } else near.delete(e.target);
      }
    },
    { rootMargin: "100% 0px" },
  );
  for (const r of document.querySelectorAll<HTMLElement>(ROOTS)) {
    if (watched.has(r)) continue;
    watched.add(r);
    io.observe(r);
  }
}

export function setGlow(colour: string): void {
  if (colour === current) return;
  current = colour;
  watch();
  for (const r of document.querySelectorAll<HTMLElement>(ROOTS)) {
    // Always on screen: the bar; drawn over what's in view: the selection's layers.
    if (!io || near.has(r) || r.matches("[data-site-bar], .sel-layer")) apply(r);
  }
}

/** The colour now (the selection's layers take it when they are made). */
export const songColour = () => current;
