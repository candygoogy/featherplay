// The scroll story's rules (src/scripts/story.ts), apart from the page so they can be tested.
// The stage is pinned for a fixed distance; `p` runs 0..1 over it. The window takes three
// holds the page settles into: whole (centred and large), narrow (the app's real narrow
// layout, beside its words), in the tray. The moves between them are short, the holds long,
// and inside a hold's zone nothing moves at all (mockup 10's timings). A move follows the
// scroll closely (so the window answers the first notch), and a scroll that stops part-way is
// carried on by the page's glide, timed in the picture's terms: off at the speed it had, then
// a long soft landing.

/** The moves, as [from, to] in p. */
export const MOVES = { a: [0, 0.16], b: [0.27, 0.5], c: [0.6, 0.74] } as const;
/** Where the page rests for each hold. */
export const HOLDS = [0.21, 0.55, 0.8] as const;
/** The stretches where a hold's picture stands still. */
export const ZONES = [
  [0.16, 0.27],
  [0.5, 0.6],
  [0.74, 1],
] as const;

const clamp01 = (v: number) => Math.min(1, Math.max(0, v));
export const seg = (v: number, a: number, b: number) => clamp01((v - a) / (b - a));
const smooth = (t: number) => t * t * (3 - 2 * t);

/** A CSS cubic-bezier as a function of time, for moves the script draws. */
export function bezier(x1: number, y1: number, x2: number, y2: number): (t: number) => number {
  const cx = 3 * x1;
  const bx = 3 * (x2 - x1) - cx;
  const ax = 1 - cx - bx;
  const cy = 3 * y1;
  const by = 3 * (y2 - y1) - cy;
  const ay = 1 - cy - by;
  const x = (t: number) => ((ax * t + bx) * t + cx) * t;
  const y = (t: number) => ((ay * t + by) * t + cy) * t;
  const dx = (t: number) => (3 * ax * t + 2 * bx) * t + cx;
  return (v: number) => {
    if (v <= 0) return 0;
    if (v >= 1) return 1;
    let t = v;
    for (let i = 0; i < 8; i++) {
      const e = x(t) - v;
      const d = dx(t);
      if (Math.abs(e) < 1e-6 || !d) break;
      t -= e / d;
    }
    return y(clamp01(t));
  };
}
/** The scroll-driven travel's curve: nearly the scroll itself, its two ends a little eased. */
export const easeMove = (t: number) => t + (smooth(t) - t) * 0.4;

/** The settling glide's curve: off at once from the visitor's own scroll, a long soft landing. */
export const easeGlide = bezier(0.2, 0.7, 0.2, 1);
/** The glide after a scroll stopped at `p`, on its way to the hold `to`: `edge` is where the
 *  picture finishes moving (the move's end on that side), reached in `ms` on `easeGlide` (a whole
 *  move's worth of picture takes 820 ms, the last of one 400); the still stretch from there to
 *  the hold, where nothing shows, takes `tail` more. */
export function glidePath(p: number, to: number): { edge: number; ms: number; tail: number } {
  const move = Object.values(MOVES).find(([a, b]) => p > a && p < b);
  if (!move) return { edge: to, ms: 0, tail: 260 };
  const edge = to > p ? move[1] : move[0];
  const left = Math.abs(edge - p) / (move[1] - move[0]);
  return { edge, ms: Math.round(400 + 420 * left), tail: edge === to ? 0 : 260 };
}

export function storyProgress(scrollY: number, top: number, length: number): number {
  return length > 0 ? clamp01((scrollY - top) / length) : 0;
}

/** How far each move has gone, eased: a (to whole), b (to narrow), c (into the tray). */
export function phases(p: number): { a: number; b: number; c: number } {
  return { a: easeMove(seg(p, ...MOVES.a)), b: easeMove(seg(p, ...MOVES.b)), c: easeMove(seg(p, ...MOVES.c)) };
}

/** Where the page should settle when the scroll stops at `p` going `dir` (1 down, -1 up), or
 *  null where it rests already (a hold's zone, the story's ends). A move is finished the way it
 *  was going, unless it had only just begun (or just about ended). */
export function settleTarget(p: number, dir: number): number | null {
  if (p <= 0 || p >= 1) return null;
  if (ZONES.some(([a, b]) => p >= a && p <= b)) return null;
  const moves: [number, number, number, number][] = [
    [MOVES.a[0], MOVES.a[1], 0, HOLDS[0]],
    [ZONES[0][1], MOVES.b[1], HOLDS[0], HOLDS[1]],
    [ZONES[1][1], MOVES.c[1], HOLDS[1], HOLDS[2]],
  ];
  for (const [a, b, back, fwd] of moves) {
    if (p <= a || p >= b) continue;
    const t = (p - a) / (b - a);
    return dir > 0 ? (t > 0.06 ? fwd : back) : t < 0.94 ? back : fwd;
  }
  return null;
}

/** Each hold's words: in as its move ends, out as the next begins. */
export function captionOpacity(i: number, p: number): number {
  const ins = [seg(p, 0.11, 0.16), seg(p, 0.44, 0.5), seg(p, 0.69, 0.74)];
  const outs = [seg(p, 0.27, 0.3), seg(p, 0.6, 0.63), 0];
  return smooth(ins[i]) * (1 - smooth(outs[i]));
}
