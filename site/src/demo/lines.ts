// Progress lines (the window's, the pieces'): each runs as one linear compositor animation from
// where the song is to its end, restarted when the song, play or the place changes. A restart
// is queued and done on the next frame, all of them together: every read first (is it rendered,
// how wide is its rail), then every write. Done where they are asked for, each read would come
// after the writes the change has just made (the colour, the titles) and force a restyle and a
// layout of the page for each line. Each line's animation is kept here, not asked of the
// element (getAnimations, which brings the element's style up to date too).

export type LineJob = {
  el: HTMLElement;
  prop: "transform" | "translate";
  /** Where it starts and ends, worked out at the frame (it may read the line's rail). */
  ends: () => [string, string];
  /** How long to its end, ms; 0 or less: it just takes its place. */
  ms: number;
  /** False: only take the place (paused, or out of view). */
  run: boolean;
};

const anims = new WeakMap<Element, Animation>();
const jobs = new Map<Element, LineJob>();
let frame = 0;

const rendered = (el: Element) => !el.checkVisibility || el.checkVisibility({ contentVisibilityAuto: true });

function flush() {
  frame = 0;
  const list = [...jobs.values()];
  jobs.clear();
  // Every read...
  const reads = list.map((j) => ({ shown: j.run && j.ms > 0 && rendered(j.el), ends: j.ends() }));
  // ...then every write.
  list.forEach((j, i) => {
    const { shown, ends } = reads[i];
    anims.get(j.el)?.cancel();
    anims.delete(j.el);
    j.el.style[j.prop] = ends[0];
    if (shown) anims.set(j.el, j.el.animate([{ [j.prop]: ends[0] }, { [j.prop]: ends[1] }], { duration: j.ms, easing: "linear", fill: "forwards" }));
  });
}

/** Restarts a line on the next frame (a later request for the same line replaces this one). */
export function runLine(job: LineJob): void {
  jobs.set(job.el, job);
  if (!frame) frame = requestAnimationFrame(flush);
}

/** Stops a line where it is (its place written at once). */
export function holdLine(el: HTMLElement, prop: "transform" | "translate", at: string): void {
  jobs.delete(el);
  anims.get(el)?.cancel();
  anims.delete(el);
  el.style[prop] = at;
}
