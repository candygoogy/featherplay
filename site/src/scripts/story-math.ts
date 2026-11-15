// The scroll story's rules (src/scripts/story.ts), apart from the page so they can be tested.
// The stage is pinned for a fixed distance; `p` runs 0..1 over it. The window takes three
// holds the page settles into: whole (centred and large), narrow (the app's real narrow
// layout, beside its words), in the tray. The moves between them are short, the holds long,
// and inside a hold's zone nothing moves at all (mockup 10's timings).

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
/** The scroll-driven travel's curve (the motion language's `--ease-move`). */
export const easeMove = bezier(0.37, 0, 0.63, 1);

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
