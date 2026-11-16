// The small moving pieces mockup 10 played with Lottie, as Web Animations with the Lottie files'
// own timings and curves (their keyframes at 60 fps: frame f is f / 60 s; each segment's curve
// is the key's out tangent and the next key's in tangent). No player, nothing loops, nothing
// runs at rest; under reduced motion none of them plays.
// - The bar's mark comes together once at load (mark-intro) and answers a song starting
//   (mark-play): layers of TriVane.astro, transforms only.
// - The bar's App, Numbers, Lyrics and Features icons move once when their section becomes the
//   current one, and when the pointer comes to them (icon-app, icon-numbers, icon-lyrics; the
//   Features tile is ours, on the same clock).
// - The download arrow drops into its tray on hover and on press (download-arrow).
// The theme toggle's turn (icon-theme) and the scrobble ring's tick (scrobble-tick) are CSS
// transitions where they are drawn (Bar.astro, pieces.css).

const reduced = () => matchMedia("(prefers-reduced-motion: reduce)").matches;
const ms = (frames: number) => (frames * 1000) / 60;
const bez = (a: number, b: number, c: number, d: number) => `cubic-bezier(${a}, ${b}, ${c}, ${d})`;

type Step = { at: number; v: Keyframe; ease?: string };
/** Each element's latest piece of motion, kept: asking an element for its animations
 *  (getAnimations) brings the page's style up to date, which in the middle of a song change
 *  was most of its cost. */
const last = new WeakMap<Element, Animation>();
/** A Lottie property's keys (frames, values, the curve into the next) as one animation. */
function play(el: Element | null | undefined, steps: Step[], delay = 0): Animation | undefined {
  if (!el) return;
  const start = steps[0].at;
  const end = steps[steps.length - 1].at;
  const frames = steps.map((s) => ({ ...s.v, offset: (s.at - start) / (end - start || 1), easing: s.ease ?? "linear" }));
  const a = el.animate(frames, { duration: ms(end - start), delay: delay + ms(start), fill: "backwards" });
  last.set(el, a);
  return a;
}
const running = (el: Element | null | undefined) => {
  const a = el ? last.get(el) : undefined;
  return !!a && (a.playState === "running" || a.pending);
};

// ---- The mark ----

/** mark-intro: the vanes sweep in one after another (each 44° and 82 % to rest, fading in over
 *  100 ms), then the triangle grows from 40 % with a push to the right and settles. */
export function markIntro(mark: HTMLElement): void {
  if (reduced()) return;
  const vane = (id: string, f: number) => {
    const el = mark.querySelector(`[data-tv="${id}"]`);
    play(el, [
      { at: f, v: { opacity: 0 }, ease: bez(0.2, 0, 0.6, 1) },
      { at: f + 6, v: { opacity: 1 } },
    ]);
    play(el, [
      { at: f, v: { rotate: "-44deg" }, ease: bez(0.33, 0, 0.1, 1) },
      { at: f + 26, v: { rotate: "0deg" } },
    ]);
    play(el, [
      { at: f, v: { scale: "0.82" }, ease: bez(0.22, 0, 0.12, 1) },
      { at: f + 22, v: { scale: "1" } },
    ]);
  };
  vane("t1", 0);
  vane("t2", 6);
  vane("t3", 12);
  for (const id of ["play", "ring"]) {
    const el = mark.querySelector(`[data-tv="${id}"]`);
    play(el, [
      { at: 24, v: { opacity: 0 }, ease: bez(0.2, 0, 0.6, 1) },
      { at: 31, v: { opacity: 1 } },
    ]);
    // 2.5 units of the 64-unit drawing.
    play(el, [
      { at: 24, v: { translate: "-3.9% 0" }, ease: bez(0.2, 0, 0, 1) },
      { at: 44, v: { translate: "0 0" } },
    ]);
    play(el, [
      { at: 24, v: { scale: "0.4" }, ease: bez(0.25, 0, 0.25, 1) },
      { at: 38, v: { scale: "1.012" }, ease: bez(0.45, 0, 0.4, 1) },
      { at: 50, v: { scale: "1" } },
    ]);
  }
}

/** mark-play: the triangle presses to 84 % and springs back through 101 %; the vanes turn 12°
 *  forward in a two-frame ripple and back. */
export function markPlay(mark: HTMLElement): void {
  if (reduced()) return;
  const tri = mark.querySelector('[data-tv="play"]');
  if (running(tri)) return;
  for (const id of ["play", "ring"]) {
    play(mark.querySelector(`[data-tv="${id}"]`), [
      { at: 0, v: { scale: "1" }, ease: bez(0.4, 0, 0.6, 1) },
      { at: 6, v: { scale: "0.84" }, ease: bez(0.2, 0, 0.1, 1) },
      { at: 20, v: { scale: "1.01" }, ease: bez(0.45, 0, 0.4, 1) },
      { at: 30, v: { scale: "1" } },
    ]);
  }
  ["t1", "t2", "t3"].forEach((id, i) => {
    play(mark.querySelector(`[data-tv="${id}"]`), [
      { at: 2 + 2 * i, v: { rotate: "0deg" }, ease: bez(0.3, 0, 0.2, 1) },
      { at: 13 + 2 * i, v: { rotate: "12deg" }, ease: bez(0.45, 0, 0.3, 1) },
      { at: 32, v: { rotate: "0deg" } },
    ]);
  });
}

// ---- The bar's section icons ----

export function iconMove(svg: SVGElement): void {
  if (reduced()) return;
  const parts = [...svg.querySelectorAll("[data-ip]")];
  if (parts.some(running)) return;
  const part = (p: string) => [...svg.querySelectorAll(`[data-ip="${p}"]`)];
  const id = svg.dataset.icon;
  if (id === "app") {
    // The title bar rises into the window's top edge, shortening between its corners, and
    // lowers back like a blind.
    play(part("titlebar")[0], [
      { at: 0, v: { transform: "translate(0px, 0px) scale(1, 1)" }, ease: bez(0.5, 0, 0.7, 1) },
      { at: 7, v: { transform: "translate(0px, -4.3px) scale(0.75, 1)" }, ease: bez(0.22, 0, 0, 1) },
      { at: 38, v: { transform: "translate(0px, 0px) scale(1, 1)" } },
    ]);
  } else if (id === "light") {
    // The needle drops back to the left, swings up past rest and settles.
    play(part("needle")[0], [
      { at: 0, v: { transform: "rotate(0deg)" }, ease: bez(0.45, 0, 0.6, 1) },
      { at: 10, v: { transform: "rotate(-96deg)" }, ease: bez(0.2, 0, 0.1, 1) },
      { at: 32, v: { transform: "rotate(2.5deg)" }, ease: bez(0.4, 0, 0.4, 1) },
      { at: 43, v: { transform: "rotate(0deg)" } },
    ]);
  } else if (id === "kept") {
    // The last tile draws back a little and comes home into its place among the others.
    play(part("tile")[0], [
      { at: 0, v: { transform: "translate(0px, 0px) scale(1)" }, ease: bez(0.5, 0, 0.7, 1) },
      { at: 8, v: { transform: "translate(1.7px, 1.7px) scale(0.78)" }, ease: bez(0.22, 0, 0, 1) },
      { at: 30, v: { transform: "translate(-0.25px, -0.25px) scale(1.03)" }, ease: bez(0.4, 0, 0.4, 1) },
      { at: 40, v: { transform: "translate(0px, 0px) scale(1)" } },
    ]);
  } else if (id === "lyrics") {
    // The capsule lifts; two sound arcs draw out from their middles, drift out and fade.
    play(part("capsule")[0], [
      { at: 0, v: { transform: "translate(0px, 0px)" }, ease: bez(0.3, 0, 0.2, 1) },
      { at: 12, v: { transform: "translate(0px, -1.3px)" } },
      { at: 20, v: { transform: "translate(0px, -1.3px)" }, ease: bez(0.45, 0, 0.2, 1) },
      { at: 42, v: { transform: "translate(0px, 0px)" } },
    ]);
    for (const arc of part("arc")) {
      play(arc, [
        { at: 3, v: { strokeDasharray: "0 1", strokeDashoffset: "-0.5" }, ease: bez(0.2, 0, 0.1, 1) },
        { at: 18, v: { strokeDasharray: "1 1", strokeDashoffset: "0" } },
      ]);
      play(arc, [
        { at: 3, v: { transform: "scale(0.9)" }, ease: bez(0.25, 0, 0.3, 1) },
        { at: 34, v: { transform: "scale(1.12)" } },
      ]);
      play(arc, [
        { at: 3, v: { opacity: 0 } },
        { at: 4, v: { opacity: 1 } },
        { at: 16, v: { opacity: 1 }, ease: bez(0.4, 0, 0.6, 1) },
        { at: 34, v: { opacity: 0 } },
      ]);
    }
  }
}

// ---- The download arrow ----

/** download-arrow: the arrow falls into the tray and is gone (clipped at it), the tray gives
 *  under it, and a new arrow fades in from above and settles. */
export function arrowDrop(glyph: SVGElement): void {
  if (reduced()) return;
  const arrow = glyph.querySelector(".dl-arrow-part");
  if (!arrow || running(arrow)) return;
  play(arrow, [
    { at: 0, v: { transform: "translate(0px, 0px)", opacity: 1 }, ease: bez(0.55, 0, 0.9, 0.55) },
    { at: 13, v: { transform: "translate(0px, 15px)", opacity: 1 }, ease: "step-end" },
    { at: 14, v: { transform: "translate(0px, -3.5px)", opacity: 0 }, ease: bez(0.2, 0, 0, 1) },
    { at: 37, v: { transform: "translate(0px, 0px)", opacity: 1 } },
  ]);
  play(glyph.querySelector(".dl-tray-part"), [
    { at: 8, v: { transform: "translate(0px, 0px)" }, ease: bez(0.4, 0, 0.3, 1) },
    { at: 14, v: { transform: "translate(0px, 0.9px)" }, ease: bez(0.3, 0, 0.1, 1) },
    { at: 32, v: { transform: "translate(0px, 0px)" } },
  ]);
}

/** Wires the pieces: the mark's intro now, its answer to a song starting (`onStart` is given a
 *  function to call when one does), the section icons, the arrows. */
export function bindMoves(onStart?: (fn: () => void) => void): void {
  const mark = document.querySelector<HTMLElement>("[data-site-bar] [data-trivane]");
  if (mark) {
    // After the first paint, once.
    requestAnimationFrame(() => requestAnimationFrame(() => markIntro(mark)));
    onStart?.(() => markPlay(mark));
  }
  // The section the page opens on is current from the start, not by becoming so: the icons
  // wait for the visitor to move the page.
  let moved = false;
  addEventListener("scroll", () => (moved = true), { once: true, passive: true });
  for (const svg of document.querySelectorAll<SVGElement>("[data-site-bar] svg[data-icon]")) {
    const link = svg.closest("a");
    if (!link) continue;
    link.addEventListener("pointerenter", (e) => {
      if (e.pointerType === "mouse") iconMove(svg);
    });
    // Once when its section first becomes the current one (the bar sets aria-current).
    new MutationObserver(() => {
      if (!moved || link.getAttribute("aria-current") !== "true" || link.hasAttribute("data-moved")) return;
      link.toggleAttribute("data-moved", true);
      iconMove(svg);
    }).observe(link, { attributes: true, attributeFilter: ["aria-current"] });
  }
  for (const glyph of document.querySelectorAll<SVGElement>("[data-dl-glyph]")) {
    const host = glyph.closest("a, button");
    if (!host) continue;
    host.addEventListener("pointerenter", (e) => {
      if ((e as PointerEvent).pointerType === "mouse") arrowDrop(glyph);
    });
    host.addEventListener("pointerdown", () => arrowDrop(glyph));
  }
}
