// The hero's scroll moment. On wide screens with motion the hero pins (CSS, Hero.astro) for
// STRETCH viewport heights. At the top the laptop is already open and lit, playing nothing
// yet but showing a song and its lyrics, smaller and leaning back a little under the name.
// As you scroll it rises, straightens and comes nearer until it fills the stage for the
// tour, and the name blurs away above it. Scroll drives it exactly: no smoothing, no timers.
// Only transforms, opacity and a small blur change (on the lid, the stage and the heading,
// never on their children).
//
// On first load, at the top, the lid opens once from nearly shut to where it rests, in
// FLOURISH_MS, on a hinge's curve (it picks up speed out of closed and settles softly, never
// swinging past), and the screen's light and the colour behind the laptop come up with it.
// All three are Web Animations on transform or opacity, one curve, so the compositor plays
// them even while the page's scripts are still starting up. Phones (768 px and under) and reduced motion get
// none of it: no pin, the laptop upright and full size under the name.
//
// A page that loads part-way down (a reload or back/forward to where it was left, kept in
// sessionStorage on the way out; or a link to a #section below the hero) gets none of the
// first-load motion: the hero is placed for where the page lands before it's first shown
// (Base.astro marks it data-land before the first paint; Hero.astro keeps the pinned screen
// unseen until data-placed), with no opening and no trip through the stretch from the top.

/** Degrees the lid leans back at the top of the page (0 faces you). */
export const LEAN = 12;
/** Viewport heights the hero stays pinned (Hero.astro's height is 100svh more than this). */
export const STRETCH = 1.1;
/** Share of the stretch by which the laptop has risen, straightened and settled. */
export const SETTLE_END = 0.7;
/** Share of the stretch by which the name has blurred away. */
const HEAD_FROM = 0.04;
const HEAD_END = 0.42;
/** The first-load lid opening: from this many degrees short of resting, in this long. */
const FLOURISH_DEG = 82;
export const FLOURISH_MS = 850;
/** How stiff the hinge's spring is: higher settles sooner. */
const STIFF = 9;
/** Stops the hinge's curve is sampled into for CSS (about one a frame of the opening). */
const EASE_STOPS = 48;
/** Pixels the settled laptop keeps clear at the top for the header's floating pills
 *  (Header.astro: they end 68 px down). */
const HEAD_ROOM = 80;
/** sessionStorage key (and the page's path) for where the page was left scrolled; Base.astro reads it. */
const LEFT_AT = "fp-scroll:";

const clamp01 = (t: number) => Math.min(1, Math.max(0, t));
const smooth = (t: number) => t * t * (3 - 2 * t);
const easeOut = (t: number) => 1 - Math.pow(1 - t, 3);

export type Moment = {
  /** 0 at the top (small, low, leaning back) to 1 settled (full size, upright). */
  zoom: number;
  /** The lid's lean back from upright, in degrees. */
  lean: number;
  /** 0 the name in full to 1 blurred away. */
  head: number;
  settled: boolean;
};

/** Where the moment is at progress `p` (0..1 of the pinned stretch). */
export function heroMoment(p: number): Moment {
  const zoom = p >= SETTLE_END ? 1 : smooth(clamp01(p / SETTLE_END));
  return {
    zoom,
    lean: LEAN * (1 - zoom),
    head: easeOut(clamp01((p - HEAD_FROM) / (HEAD_END - HEAD_FROM))),
    settled: p >= SETTLE_END,
  };
}

/** A critically damped spring let go from rest: 0 to (nearly) 1 over t in 0..1. */
const spring = (t: number) => 1 - (1 + STIFF * t) * Math.exp(-STIFF * t);

/**
 * The opening's hinge, 0 shut to 1 resting at `t` (0..1 of FLOURISH_MS): a critically damped
 * spring, so the lid picks up speed out of closed (no jump at the start) and eases into its
 * rest with a soft, damped finish, never swinging past it. Scaled to end exactly at 1.
 */
export function hinge(t: number): number {
  if (t <= 0) return 0;
  if (t >= 1) return 1;
  return spring(t) / spring(1);
}

/** The hinge as one CSS easing, finely sampled, so no straight stretch of it shows. */
export const FLOURISH_EASE = `linear(${Array.from({ length: EASE_STOPS + 1 }, (_, i) => +hinge(i / EASE_STOPS).toFixed(4)).join(", ")})`;

/** The first-load opening: degrees the lid is short of resting at `t` (0..1 of FLOURISH_MS). */
export function lidFlourish(t: number): number {
  return t >= 1 ? 0 : -FLOURISH_DEG * (1 - hinge(clamp01(t)));
}

/**
 * The laptop's scale at the top: as large as fits between the name and the bottom of the
 * first screen, within reason.
 */
export function startScale(room: number, laptopHeight: number): number {
  if (laptopHeight <= 0) return 0.74;
  return Math.min(0.8, Math.max(0.6, room / laptopHeight));
}

export function bindLid(section: HTMLElement, laptop: HTMLElement): () => void {
  const lid = laptop.querySelector<HTMLElement>("[data-laptop-body]");
  const stage = section.querySelector<HTMLElement>("[data-hero-stage]");
  const head = section.querySelector<HTMLElement>("[data-hero-head]");
  if (!lid || !stage) return () => {};

  const scrubbing = matchMedia("(min-width: 769px) and (prefers-reduced-motion: no-preference)");
  let frame = 0;
  let last = "";
  let flourish: Animation | null = null; // the first-load opening while it runs
  let lit: Animation[] = []; // the screen's light and the glow, coming up with it
  let geo = { scale: 0.74, rise: 0 };

  // The laptop is awake from the start: there's no dark screen at rest.
  laptop.dataset.awake = "true";

  /** Measures the start scale and the rise from the layout (transforms don't change these). */
  const measure = () => {
    const pin = stage.offsetParent as HTMLElement | null;
    if (!pin) return;
    const laptopH = laptop.offsetHeight;
    // The player line hangs under the laptop (Hero.astro), outside the stage's own height.
    const line = stage.querySelector<HTMLElement>("[data-now-playing]");
    const credit = Math.max(stage.offsetHeight - laptopH, line ? line.offsetTop + line.offsetHeight - laptopH : 0);
    const top = stage.offsetTop; // where the stage sits in its own layout, under the name
    // In the first screen the pin starts under the site's header.
    const firstScreen = innerHeight - (section.getBoundingClientRect().top + scrollY);
    const scale = startScale(firstScreen - top - credit - 24, laptopH);
    // Settled, the laptop and its line sit in the middle of the pinned screen below the
    // header's pills.
    const free = pin.clientHeight - laptopH - credit;
    const settledTop = HEAD_ROOM + Math.max(0, (free - HEAD_ROOM - 16) / 2);
    geo = { scale, rise: settledTop - top };
    last = "";
  };

  /** The opening's keyframes, ending at `lean` (FLOURISH_EASE moves it between them). */
  const opening = (lean: number): Keyframe[] => [
    { transform: `rotateX(${(lean - FLOURISH_DEG).toFixed(2)}deg)` },
    { transform: `rotateX(${lean.toFixed(2)}deg)` },
  ];
  const stopOpening = () => {
    flourish?.cancel();
    for (const a of lit) a.cancel();
    lit = [];
  };

  const apply = (p: number) => {
    const m = heroMoment(p);
    const lean = m.lean;
    const key = p.toFixed(4);
    if (key === last) return;
    last = key;
    lid.style.transform = lean === 0 ? "none" : `rotateX(${lean.toFixed(2)}deg)`;
    // Scrolled while it opens: it opens to where the lid now rests.
    if (flourish) (flourish.effect as KeyframeEffect).setKeyframes(opening(lean));
    const s = geo.scale + (1 - geo.scale) * m.zoom;
    const y = geo.rise * m.zoom;
    // Whole pixels, so the screen's text never sits (or shimmers) on a half pixel.
    stage.style.transform = s === 1 ? `translateY(${Math.round(y)}px)` : `translateY(${Math.round(y)}px) scale(${s.toFixed(4)})`;
    if (head) {
      if (m.head <= 0) {
        head.style.opacity = head.style.transform = head.style.filter = head.style.visibility = "";
      } else {
        head.style.opacity = (1 - m.head).toFixed(3);
        head.style.transform = `translateY(${(-40 * m.head).toFixed(1)}px)`;
        head.style.filter = `blur(${(8 * m.head).toFixed(2)}px)`;
        // Gone: out of the tab order and the pointer's way too.
        head.style.visibility = m.head >= 1 ? "hidden" : "";
      }
    }
    // Settled: the tour may point at things on the screen (Tour.astro waits for it).
    if (m.settled && !flourish) laptop.dataset.settled = "true";
    else delete laptop.dataset.settled;
  };

  const progress = () => {
    if (!scrubbing.matches) return 1;
    const start = section.getBoundingClientRect().top + scrollY;
    return clamp01((scrollY - start) / (innerHeight * STRETCH));
  };

  const reset = () => {
    lid.style.transform = "";
    stage.style.transform = "";
    if (head) head.style.opacity = head.style.transform = head.style.filter = head.style.visibility = "";
    laptop.dataset.settled = "true";
    last = "";
  };

  let waiting = false;
  const update = () => {
    frame = 0;
    if (!scrubbing.matches) {
      stopOpening();
      reset();
    } else apply(progress());
    // Placed: Hero.astro shows the pinned screen of a page that loaded part-way down.
    if (!waiting) section.dataset.placed = "";
  };
  const onScroll = () => {
    waiting = false;
    if (!frame) frame = requestAnimationFrame(update);
  };
  // A resize (or Manrope arriving) isn't the jump a reload waits for.
  const onResize = () => {
    measure();
    if (!frame) frame = requestAnimationFrame(update);
  };

  // Where the page lands. The browser restores a reload's position (history.scrollRestoration
  // stays "auto") as soon as the page is tall enough, usually before this runs; a link's
  // #section it scrolls to only as loading ends, so that jump is made now, the same one.
  const root = document.documentElement;
  const heroTop = () => section.getBoundingClientRect().top + scrollY;
  let land = root.dataset.land;
  if (land === "hash") {
    const id = decodeURIComponent(location.hash.slice(1));
    const target = id ? document.getElementById(id) : null;
    if (target && !section.contains(target) && target.getBoundingClientRect().top + scrollY > heroTop() + 4) {
      if (scrollY <= 4) target.scrollIntoView({ behavior: "instant" });
    } else land = undefined;
  }
  if (!land && scrollY > heroTop() + 4) land = "restore";
  if (land) root.dataset.land = land;
  else delete root.dataset.land;
  // The browser puts a reload back where it was left, but a reload made before the last one
  // had got back there records the top instead, and the browser then holds the page there
  // through loading. Where it was left is kept here too (sessionStorage, on pagehide), so a
  // page that should land part-way down and still sits at the top goes there now, and the
  // browser is kept from undoing it (for this load only).
  if (land === "restore" && scrollY <= 4) {
    let left = 0;
    try {
      left = Number(sessionStorage.getItem(LEFT_AT + location.pathname)) || 0;
    } catch {
      // Storage blocked: Base.astro couldn't have read it either.
    }
    if (left > 4) {
      history.scrollRestoration = "manual";
      addEventListener("pagehide", () => (history.scrollRestoration = "auto"));
      scrollTo({ top: left, behavior: "instant" });
      // Not tall enough yet for all of it: the rest once loaded, unless moved meanwhile.
      const short = Math.round(scrollY);
      if (short < left - 4 && document.readyState !== "complete")
        addEventListener(
          "load",
          () => {
            if (Math.abs(scrollY - short) <= 4) scrollTo({ top: left, behavior: "instant" });
          },
          { once: true },
        );
    }
  }
  // A reload whose jump hasn't come yet: shown once it has (its scroll), or once loaded.
  waiting = land === "restore" && scrollY <= heroTop() + 4 && document.readyState !== "complete";
  if (waiting) addEventListener("load", () => ((waiting = false), onScroll()), { once: true });
  addEventListener("pagehide", () => {
    // Left again before getting back to its place: where it was left still stands.
    if (waiting) return;
    try {
      sessionStorage.setItem(LEFT_AT + location.pathname, String(Math.round(scrollY)));
    } catch {
      // Storage blocked: a reload starts as a first load would.
    }
  });

  if (scrubbing.matches) {
    measure();
  }
  update();
  // The opening, once, when the page loads at the top.
  if (scrubbing.matches && !land && lid.animate) {
    const lean = heroMoment(progress()).lean;
    const timing = { duration: FLOURISH_MS, easing: FLOURISH_EASE };
    const run = lid.animate(opening(lean), timing);
    flourish = run;
    // The screen's dark glass clears as it turns towards you, and the song's colour behind
    // the laptop comes up with it, on the same curve.
    const sleep = laptop.querySelector<HTMLElement>("[data-screen-sleep]");
    const glow = section.querySelector<HTMLElement>("[data-glow]");
    lit = [
      sleep?.animate([{ opacity: 0.8 }, { opacity: 0 }], timing),
      glow?.animate([{ opacity: 0.3 }, { opacity: 1 }], timing),
    ].filter((a): a is Animation => !!a);
    const done = () => {
      if (flourish === run) {
        flourish = null;
        lit = [];
      }
      last = "";
      onScroll();
    };
    run.finished.then(done, done);
  }
  // Manrope arriving can change the name's height, so the start position.
  document.fonts?.ready.then(onResize);

  addEventListener("scroll", onScroll, { passive: true });
  addEventListener("resize", onResize, { passive: true });
  const onMedia = () => {
    if (scrubbing.matches) measure();
    update();
  };
  scrubbing.addEventListener("change", onMedia);

  return () => {
    cancelAnimationFrame(frame);
    stopOpening();
    removeEventListener("scroll", onScroll);
    removeEventListener("resize", onResize);
    scrubbing.removeEventListener("change", onMedia);
  };
}
