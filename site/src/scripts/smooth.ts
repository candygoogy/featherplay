// The wheel's ease, shared with the lyrics that move by hand (src/demo/follow.ts): a mouse
// wheel's notch eases in over a few frames (a short exponential ease, most of the way in about
// 400 ms); a trackpad's fine pixel deltas, already smooth, are left as they come.

/** How long the ease takes to cover 63 % of the way (about a 0.12 lerp a frame at 60 Hz). */
export const TAU_MS = 130;
/** After a trackpad's fine delta, its stream of events keeps the browser's scrolling this long. */
export const FINE_HOLD_MS = 300;

/** Whether this wheel event is a mouse wheel's notch (not a trackpad's continuous stream). */
export function isNotch(e: Pick<WheelEvent, "deltaX" | "deltaY" | "deltaMode"> & { wheelDeltaY?: number }): boolean {
  if (e.deltaX !== 0 || e.deltaY === 0) return false;
  if (e.deltaMode !== 0) return true; // lines or pages: a wheel
  // Pixels: a notch is a large whole step (about 100 px), and the legacy wheelDelta is a
  // multiple of the OS's 120 a notch; a trackpad sends small, uneven deltas.
  return Math.abs(e.deltaY) >= 40 && (e.wheelDeltaY === undefined || e.wheelDeltaY % 120 === 0);
}
