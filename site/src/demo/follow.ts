// Synced lyrics that follow the song and can still be scrolled by hand, as in the app
// (pages/lyrics.slint): the lines start at the top of the panel, and once the line being sung
// gets a third of the way down they glide to hold it there; the list never scrolls above its
// first line nor past its end (`followOffset`, the app's `SyncedLines.follow`). A wheel or a
// touch drag over the panel moves them freely and holds the follow for 5 s, then they ease
// back to the current line. At either end of the lines the wheel isn't taken: the page
// scrolls on, so the panel never traps the visitor.
//
// The lines move with one `transform: translateY()` on the list; `follow()` is told the line
// being sung and works out where the song wants the list, and the hand scroll moves it within
// the same range while it holds.
//
// A mouse wheel's notch glides the lines as the page's wheel glides the page
// (src/scripts/smooth.ts, the same ease): most of the way in about 400 ms, and notches in a
// row add to where they're heading. A trackpad's fine deltas, a finger and keyboard focus
// move them 1:1 at once, and so does every wheel under prefers-reduced-motion. While the
// lines move by hand the list carries data-moving, which holds back the hover highlight: a
// line passing under a still pointer would otherwise light up and go out frame by frame.

import { FINE_HOLD_MS, TAU_MS, isNotch } from "../scripts/smooth";

export type LyricsFollow = {
  /** The line being sung (its list item, or null for none yet): the list moves to keep it a
   *  third of the way down. */
  follow(line: HTMLElement | null, instant?: boolean): void;
  /** Back to following at once (a click on a line, a new song). */
  release(): void;
  readonly following: boolean;
};

export const HOLD_MS = 5000;
/** The fade where the lines leave the panel, top and bottom (theme.slint `lyric-edge`). */
export const EDGE_PX = 24;

/** A line's place in the list: its top from the list's own top (padding included), and its height. */
export type LineBox = { top: number; height: number };

/**
 * Where the list rests for the line being sung, as a distance scrolled from the top: the line's
 * centre a third of the way down a `view` tall panel, but never above the first line (0) nor
 * past the list's end (`content` tall, padding included). The app's
 * `clamp(height / 3 - line-y - line-h / 2, min(0, height - content-height), 0)`, negated.
 */
export function followOffset(line: LineBox | null, view: number, content: number): number {
  if (!line) return 0;
  const range = Math.max(0, content - view);
  return Math.min(range, Math.max(0, line.top + line.height / 2 - view / 3));
}
/** After the lines last moved by hand, the hover highlight waits this long to come back. */
const REST_MS = 120;

export function followLyrics(panel: HTMLElement, list: HTMLElement): LyricsFollow {
  let target = 0; // where the song wants the list
  let shown = 0; // where it is
  let following = true;
  let hold = 0;

  // How far the list can move: from its top to its end at the panel's bottom (its padding
  // below is half the panel, as the app's). The follow keeps to the same range.
  const range = () => Math.max(0, list.offsetHeight - panel.clientHeight);
  /** Where the song wants the list for this line (layout pixels, unscaled). */
  const offsetFor = (line: HTMLElement | null) => {
    if (!line) return 0;
    // The line's top from the list's own top: offsetTop counts from the list when it's
    // positioned, else from the same ancestor as the list's.
    const top = line.offsetParent === list ? line.offsetTop : line.offsetTop - list.offsetTop;
    return followOffset({ top, height: line.offsetHeight }, panel.clientHeight, list.offsetHeight);
  };
  // The panel may be drawn scaled (the replica on the laptop's screen): wheel and touch
  // distances are in screen pixels, the list's in its own.
  const scale = () => {
    const h = panel.offsetHeight;
    return h ? panel.getBoundingClientRect().height / h || 1 : 1;
  };

  let placed = false;
  const apply = (y: number, glide: boolean) => {
    shown = y;
    list.style.transition = glide ? "" : "none";
    list.style.transform = `translateY(${-y}px)`;
    // A jump is committed at once, so a glide right after it starts from here. Not the first
    // placement: nothing glides in the same task, and on load it would force the whole page's
    // style and layout while the page is still starting up.
    if (!glide && placed) {
      void list.offsetHeight;
    }
    placed = true;
  };

  const reduce = matchMedia("(prefers-reduced-motion: reduce)");
  let frame = 0; // the wheel's glide under way
  let aim = 0; // where it's heading
  let last = 0;
  let fineUntil = 0;
  let rest = 0;

  /** The lines are moving by hand: no hover highlight until they've been still a moment. */
  const moving = () => {
    list.toggleAttribute("data-moving", true);
    clearTimeout(rest);
    rest = window.setTimeout(() => list.removeAttribute("data-moving"), REST_MS);
  };
  const stopGlide = () => {
    if (frame) cancelAnimationFrame(frame);
    frame = 0;
  };
  /** Where the lines are on screen now, also part-way through the follow's own glide. */
  const onScreen = () => {
    if (!following) return shown;
    const t = getComputedStyle(list).transform;
    return t && t !== "none" ? -new DOMMatrixReadOnly(t).m42 : shown;
  };
  const holdFollow = () => {
    following = false;
    clearTimeout(hold);
    hold = window.setTimeout(release, HOLD_MS);
  };

  const release = () => {
    clearTimeout(hold);
    stopGlide();
    if (following) return;
    following = true;
    apply(target, true);
  };

  /** Moves the list by `dy` of its own pixels at once; false when it's already at that end. */
  const nudge = (dy: number) => {
    stopGlide();
    const from = onScreen();
    const next = Math.min(range(), Math.max(0, from + dy));
    if (Math.abs(next - from) < 0.5) return false;
    apply(next, false);
    holdFollow();
    return true;
  };

  const step = (now: number) => {
    frame = 0;
    const dt = Math.min(64, Math.max(0, now - last));
    last = now;
    let y = shown + (aim - shown) * (1 - Math.exp(-dt / TAU_MS));
    if (Math.abs(aim - y) < 0.5) y = aim;
    // Frame by frame: no transition, and no forced layout (apply()'s commit is for jumps).
    shown = y;
    list.style.transform = `translateY(${-y}px)`;
    moving();
    if (y !== aim) frame = requestAnimationFrame(step);
  };

  /** A notch: glides the list `dy` further than where it's heading; false at that end. */
  const glide = (dy: number) => {
    const from = frame ? aim : onScreen();
    const next = Math.min(range(), Math.max(0, from + dy));
    if (Math.abs(next - from) < 0.5) return false;
    if (!frame) {
      shown = onScreen();
      list.style.transition = "none";
      list.style.transform = `translateY(${-shown}px)`;
      last = performance.now();
      frame = requestAnimationFrame(step);
    }
    aim = next;
    holdFollow();
    return true;
  };

  panel.addEventListener(
    "wheel",
    (e) => {
      if (e.ctrlKey || Math.abs(e.deltaY) < Math.abs(e.deltaX)) return;
      // On the hero's laptop while it's still rising, the wheel is the page's: it drives the
      // scroll moment (src/scripts/lid.ts).
      if (panel.closest("[data-laptop]:not([data-settled])")) return;
      const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? panel.clientHeight : 1;
      const dy = (e.deltaY * unit) / scale();
      const notch = isNotch(e as WheelEvent & { wheelDeltaY?: number });
      // A trackpad's stream keeps its 1:1 for a while, even through a large delta.
      if (!notch && e.deltaMode === 0) fineUntil = e.timeStamp + FINE_HOLD_MS;
      const took = notch && !reduce.matches && e.timeStamp >= fineUntil ? glide(dy) : nudge(dy);
      if (took) {
        e.preventDefault();
        if (!frame) moving();
      }
    },
    { passive: false },
  );

  let touchY: number | null = null;
  panel.addEventListener(
    "touchstart",
    (e) => {
      stopGlide();
      touchY = e.touches.length === 1 ? e.touches[0].clientY : null;
    },
    { passive: true },
  );
  panel.addEventListener(
    "touchmove",
    (e) => {
      if (touchY === null || e.touches.length !== 1) return;
      const y = e.touches[0].clientY;
      if (nudge((touchY - y) / scale())) {
        moving();
        if (e.cancelable) e.preventDefault();
      }
      touchY = y;
    },
    { passive: false },
  );
  panel.addEventListener("touchend", () => (touchY = null), { passive: true });

  // Tab onto a line out of sight: the lines move to show it (as a hand scroll, held for a
  // while). The panel clips (overflow: clip), so focus can't scroll the box itself and leave
  // the lines' anchor off for good.
  panel.addEventListener("focusin", (e) => {
    const line = (e.target as Element).closest?.(".line");
    if (!line || !list.contains(line)) return;
    const box = panel.getBoundingClientRect();
    const r = line.getBoundingClientRect();
    const edge = EDGE_PX * scale(); // the fade at the panel's edges
    const dy =
      r.bottom > box.bottom - edge ? r.bottom - (box.bottom - edge) : r.top < box.top + edge ? r.top - (box.top + edge) : 0;
    if (dy) nudge(dy / scale());
  });

  return {
    follow(line, instant = false) {
      target = offsetFor(line);
      if (following) apply(target, !instant);
    },
    release,
    get following() {
      return following;
    },
  };
}
