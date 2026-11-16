// The handwritten notes (Note.astro) and pen marks: each writes itself in once, the first time
// it is wholly in view, and a note is hidden rather than drawn over anything its words would
// touch (another's words, a picture, a control, a live piece, the window) or run past the
// screen's sides, at whatever width the page has. Its arrow may point into what it points at.
// Measured on load, once the fonts are in, and on the frame after a resize; never otherwise.

const BLOCKERS =
  "h1, h2, h3, h4, p:not(.hand), li, button, a, img, td, th, label, legend, figure, [data-piece], [data-window], .win-body, .explorer, .lyric-card, .band, .panel-piece";
/** Text is compared by its line boxes, not its element's box. */
const TEXTY = "h1, h2, h3, h4, p, li, td, th, a, button, label, legend";
const M = 4;

type R = { left: number; top: number; right: number; bottom: number; width: number };
const overlaps = (a: R, b: R) => a.left < b.right + M && a.right > b.left - M && a.top < b.bottom + M && a.bottom > b.top - M;

function boxesOf(el: Element): R[] {
  if (!el.matches(TEXTY)) return [el.getBoundingClientRect()];
  const r = document.createRange();
  r.selectNodeContents(el);
  return [...r.getClientRects()];
}

/** Whether a note's words, where its CSS puts them now, would touch anything or leave the screen. */
export function collides(note: HTMLElement): boolean {
  const words = note.querySelector<HTMLElement>(":scope > span") ?? note;
  const r = words.getBoundingClientRect();
  if (!r.width) return false;
  const vw = document.documentElement.clientWidth;
  if (r.left < M || r.right > vw - M) return true;
  const scope = note.closest("[data-note-scope], section, footer") ?? document.body;
  for (const b of scope.querySelectorAll(BLOCKERS)) {
    if (b.contains(note) || note.contains(b) || b.closest(".visually-hidden, [hidden]")) continue;
    // A live piece counts as its own box: what it holds may lie clipped outside it (the lyric
    // card's lines and its blurred cover).
    if (b.parentElement?.closest("[data-piece]")) continue;
    if (!b.getClientRects().length || getComputedStyle(b).visibility === "hidden") continue;
    for (const x of boxesOf(b)) if (x.width > 1 && overlaps(r, x)) return true;
  }
  return false;
}

export function bindNotes(): void {
  const root = document.documentElement;
  const all = [...document.querySelectorAll<HTMLElement>(".hand[data-note]")];
  const marks = [...document.querySelectorAll<HTMLElement>(".pen-mark")];
  const placed = all.filter((n) => !n.hasAttribute("data-note-manual"));

  // ---- Hidden rather than colliding: measured all first, then marked (one layout) ----
  // Each note is out of the layout until first judged (system.css), so none widens the page.
  const check = () => {
    for (const n of placed) {
      n.classList.remove("note-off");
      n.toggleAttribute("data-judged", true);
    }
    const off = placed.map((n) => getComputedStyle(n).display !== "none" && collides(n));
    placed.forEach((n, i) => n.classList.toggle("note-off", off[i]));
  };
  // A resize judges them again on the next frame (one check a frame at most), so a note that no
  // longer fits never widens the page for longer than a frame; and once more when the resize
  // has settled, for what lays itself out after it (a window fitted to its new room).
  let t = 0;
  let settle = 0;
  const later = () => {
    if (!t)
      t = requestAnimationFrame(() => {
        t = 0;
        check();
      });
    clearTimeout(settle);
    settle = window.setTimeout(check, 200);
  };
  // First judged when the page is idle (every note lies below the first screen): not in the
  // start-up work; then again once the fonts are in.
  const idle = (fn: () => void) => ("requestIdleCallback" in window ? requestIdleCallback(fn, { timeout: 600 }) : setTimeout(fn, 200));
  idle(() => {
    check();
    document.fonts?.ready.then(check);
  });
  addEventListener("resize", later);

  // ---- Written in once, when wholly in view ----
  if (matchMedia("(prefers-reduced-motion: reduce)").matches || !("IntersectionObserver" in window)) {
    for (const n of [...placed, ...marks]) n.classList.add("written");
    return;
  }
  root.classList.add("pen");
  const io = new IntersectionObserver(
    (es) => {
      for (const e of es) {
        if (!e.isIntersecting) continue;
        e.target.classList.add("written");
        io.unobserve(e.target);
      }
    },
    { rootMargin: "0px 0px -5% 0px", threshold: 1 },
  );
  for (const n of [...placed, ...marks]) io.observe(n);
}

type Rect = { left: number; top: number; right: number; bottom: number };

/**
 * Shows a note its owner places (data-note-manual: the story's, the window's) at a point on
 * the screen (its top left, in viewport px), written in the first time, but only where its
 * words stay on the screen and clear of `avoid`; otherwise it stays hidden. Returns whether
 * it shows. `hideNote` puts it away again (it stays written).
 */
export function showNoteAt(note: HTMLElement, x: number, y: number, avoid: Rect[] = []): boolean {
  note.style.translate = `${Math.round(x)}px ${Math.round(y)}px`;
  const words = (note.querySelector(":scope > span") ?? note).getBoundingClientRect();
  const vw = document.documentElement.clientWidth;
  const fits =
    words.width > 0 &&
    words.left >= M &&
    words.right <= vw - M &&
    words.top >= M &&
    words.bottom <= innerHeight - M &&
    // (Sides copied one by one: a DOMRect's are getters, which a spread would lose.)
    !avoid.some((a) => overlaps(words as R, { left: a.left, top: a.top, right: a.right, bottom: a.bottom, width: a.right - a.left }));
  note.toggleAttribute("data-on", fits);
  if (fits) {
    // Written the first time it shows (at once with reduced motion: no transition runs).
    requestAnimationFrame(() => note.classList.add("written"));
  }
  return fits;
}

export function hideNote(note: HTMLElement): void {
  note.removeAttribute("data-on");
}

type Pt = { x: number; y: number };

/**
 * Draws a note's free arrow (Note.astro `free`): a pen stroke from `from` to `to` (viewport
 * px), bowed `bow` px to one side of the straight line (negative: the other side), with its
 * head at `to`. It is written in with the words (the same pen reveal).
 */
export function penArrow(note: HTMLElement, from: Pt, to: Pt, bow: number): void {
  const path = note.querySelector<SVGPathElement>("svg.free path");
  if (!path) return;
  const o = note.getBoundingClientRect();
  const a = { x: from.x - o.left, y: from.y - o.top };
  const b = { x: to.x - o.left, y: to.y - o.top };
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const len = Math.hypot(dx, dy) || 1;
  const n = { x: -dy / len, y: dx / len };
  const c1 = { x: a.x + dx * 0.3 + n.x * bow, y: a.y + dy * 0.3 + n.y * bow };
  const c2 = { x: a.x + dx * 0.78 + n.x * bow * 0.55, y: a.y + dy * 0.78 + n.y * bow * 0.55 };
  // The head: two short strokes back from the point, along the curve's last direction.
  const tl = Math.hypot(b.x - c2.x, b.y - c2.y) || 1;
  const u = { x: (b.x - c2.x) / tl, y: (b.y - c2.y) / tl };
  const size = Math.min(14, Math.max(8, len * 0.12)) * (parseFloat(getComputedStyle(note).fontSize) / 30);
  const turn = (ang: number) => ({ x: b.x - size * (u.x * Math.cos(ang) - u.y * Math.sin(ang)), y: b.y - size * (u.y * Math.cos(ang) + u.x * Math.sin(ang)) });
  const h1 = turn(0.5);
  const h2 = turn(-0.5);
  const f = (p: Pt) => `${p.x.toFixed(1)} ${p.y.toFixed(1)}`;
  path.setAttribute("d", `M${f(a)} C${f(c1)}, ${f(c2)}, ${f(b)} M${f(h1)} L${f(b)} L${f(h2)}`);
}
