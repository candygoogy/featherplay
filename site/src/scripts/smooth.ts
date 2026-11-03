// A mouse wheel glides the page: each notch eases in over a few frames (a short exponential
// ease, most of the way in about 400 ms) instead of jumping 100 px at once. Notches in a row
// add to where it's heading, so a long spin carries on smoothly and stops where the wheel
// stopped. It never fights the visitor: anything else that moves the page (a key, the
// scrollbar, a link within the page, a touch) takes over at once.
//
// Everything that isn't a mouse wheel's notch keeps the browser's own scrolling: a trackpad
// (fine pixel deltas, already smooth), touch (no wheel events), the keyboard, pinch and
// Ctrl+wheel zoom, sideways scrolling, and every wheel under prefers-reduced-motion. A wheel
// something on the page has already taken (a lyrics panel moving its lines, the replica's
// volume slider) or one over a box that scrolls itself is left alone too.
//
// The page is moved with scrollTo(), so the scroll events, the hero's moment (lid.ts), the
// header's pills and the position kept for a reload all follow the glide frame by frame. It
// runs before their own frame callbacks (requested from the scroll event this frame), so they
// read the position this frame shows.

/** How long the ease takes to cover 63 % of the way (about a 0.12 lerp a frame at 60 Hz). */
export const TAU_MS = 130;
/** Pixels a "line" of a line-mode wheel (Firefox's mouse) is worth: 3 lines, about 100 px. */
const LINE_PX = 33;
/** After a trackpad's fine delta, its stream of events keeps the browser's scrolling this long. */
export const FINE_HOLD_MS = 300;

let started = false;

/** Whether this wheel event is a mouse wheel's notch (not a trackpad's continuous stream). */
export function isNotch(e: Pick<WheelEvent, "deltaX" | "deltaY" | "deltaMode"> & { wheelDeltaY?: number }): boolean {
  if (e.deltaX !== 0 || e.deltaY === 0) return false;
  if (e.deltaMode !== 0) return true; // lines or pages: a wheel
  // Pixels: a notch is a large whole step (about 100 px), and the legacy wheelDelta is a
  // multiple of the OS's 120 a notch; a trackpad sends small, uneven deltas.
  return Math.abs(e.deltaY) >= 40 && (e.wheelDeltaY === undefined || e.wheelDeltaY % 120 === 0);
}

/** A box under the pointer that will scroll itself that way (a list, a text area). */
function scrollsItself(e: WheelEvent, dy: number): boolean {
  for (const node of e.composedPath()) {
    if (!(node instanceof Element) || node === document.documentElement || node === document.body) continue;
    if (node.scrollHeight <= node.clientHeight + 1) continue;
    const o = getComputedStyle(node).overflowY;
    if (o !== "auto" && o !== "scroll" && o !== "overlay") continue;
    if (dy > 0 ? node.scrollTop + node.clientHeight < node.scrollHeight - 1 : node.scrollTop > 0) return true;
  }
  return false;
}

export function bindSmoothScroll(): void {
  if (started) return;
  started = true;
  const root = document.documentElement;
  const reduce = matchMedia("(prefers-reduced-motion: reduce)");

  let frame = 0;
  let pos = 0; // where the glide has put the page (fractional)
  let target = 0; // where the wheel wants it
  let last = 0;
  let fineUntil = 0;

  const room = () => Math.max(0, root.scrollHeight - innerHeight);
  const stop = () => {
    if (frame) cancelAnimationFrame(frame);
    frame = 0;
    root.removeAttribute("data-gliding");
  };

  const step = (now: number) => {
    frame = 0;
    // Moved by something else since the last frame (a key, the scrollbar, a link): theirs.
    if (Math.abs(scrollY - pos) > 1.5) return stop();
    const dt = Math.min(64, Math.max(0, now - last));
    last = now;
    target = Math.min(target, room());
    pos += (target - pos) * (1 - Math.exp(-dt / TAU_MS));
    if (Math.abs(target - pos) < 0.5) pos = target;
    scrollTo({ top: pos, behavior: "instant" });
    // At an end (or the page got shorter): the browser has the last word on where it is.
    if (Math.abs(scrollY - pos) > 1.5) pos = scrollY;
    if (pos === target || Math.abs(scrollY - target) < 0.5) return stop();
    frame = requestAnimationFrame(step);
  };

  addEventListener(
    "wheel",
    (e) => {
      if (e.defaultPrevented || e.ctrlKey || e.shiftKey || reduce.matches) return;
      if (!isNotch(e as WheelEvent & { wheelDeltaY?: number }) || e.timeStamp < fineUntil) {
        // A trackpad's stream: the browser scrolls it, and a glide under way gives way.
        if (e.deltaMode === 0) fineUntil = e.timeStamp + FINE_HOLD_MS;
        stop();
        return;
      }
      const dy = e.deltaY * (e.deltaMode === 1 ? LINE_PX : e.deltaMode === 2 ? innerHeight * 0.9 : 1);
      if (scrollsItself(e, dy)) return;
      e.preventDefault();
      if (!frame) {
        pos = target = scrollY;
        last = performance.now();
        root.setAttribute("data-gliding", "");
        frame = requestAnimationFrame(step);
      }
      target = Math.min(room(), Math.max(0, target + dy));
    },
    { passive: false },
  );

  // Anything else that moves the page takes over at once.
  for (const type of ["pointerdown", "keydown", "touchstart"] as const) addEventListener(type, stop, { passive: true });
  addEventListener("visibilitychange", stop);
  reduce.addEventListener("change", stop);
}
