// "Everything you'd miss, kept": one feature at a time. On wide screens the step in the middle
// of the screen is the one on; pointing at a step (hover, focus or a tap) shows it too. The
// one on is marked where it lives on the laptop, and the screen zooms to that place (one
// transform on the layer inside the screen; the laptop stays whole). The status card and the media overlay
// show the playing song.

import { formatTime, type Player } from "./player";

type Zoom = { x: number; y: number; z: number };

/** How close each place is worth looking, on the screen: the small ones closer. */
const ZOOM: Record<string, number> = { scrobble: 1.9, discord: 1.75, keys: 1.75, sponsor: 1.6, tray: 2.4 };

/**
 * The zoom layer's transform to bring point (px, py) of a w x h box to its middle at `z`,
 * never past the box's own edges (so the frame is always filled).
 */
export function zoomTo(w: number, h: number, px: number, py: number, z: number): string {
  const tx = Math.min(0, Math.max(w * (1 - z), w / 2 - z * px));
  const ty = Math.min(0, Math.max(h * (1 - z), h / 2 - z * py));
  return `translate(${tx.toFixed(1)}px, ${ty.toFixed(1)}px) scale(${z})`;
}

export function bindFeatures(root: HTMLElement, player: Player): void {
  const items = [...root.querySelectorAll<HTMLButtonElement>("[data-feature]")];
  const steps = [...root.querySelectorAll<HTMLElement>("[data-step]")];
  const spots = root.querySelector<HTMLElement>(".spots");
  const zoom = root.querySelector<HTMLElement>("[data-kept-zoom]");
  if (!spots) return;
  const desktop = spots.closest<HTMLElement>("[data-desktop]")!;
  const wide = matchMedia("(min-width: 961px)");
  const reduced = matchMedia("(prefers-reduced-motion: reduce)");
  let pinned: string | null = null;
  let pointed: string | null = null;
  let scrolled: string | null = null;

  const targetOf = (id: string): Element | null => {
    // Scrobbling sends the song: the bar's title and artist.
    if (id === "scrobble") return desktop.querySelector("[data-replica] [data-bar] .bar-song");
    if (id === "tray") return desktop.querySelector("[data-tray-icon]");
    if (id === "sponsor") return desktop.querySelector("[data-replica] [data-seek-line] .rail");
    return spots.querySelector(`[data-spot="${id}"]`);
  };

  // The ring and the marks, on their targets, in the desktop's own 1440x900 pixels.
  const place = (id: string) => {
    const box = spots.getBoundingClientRect();
    const scale = box.width / spots.offsetWidth || 1;
    const put = (spot: HTMLElement | null, target: Element | null, pad = 0) => {
      if (!spot || !target) return;
      const r = target.getBoundingClientRect();
      spot.style.left = `${(r.left - box.left) / scale - pad}px`;
      spot.style.top = `${(r.top - box.top) / scale - pad}px`;
      spot.style.width = `${r.width / scale + 2 * pad}px`;
      if (id !== "sponsor") spot.style.height = `${r.height / scale + 2 * pad}px`;
    };
    const spot = spots.querySelector<HTMLElement>(`[data-spot="${id}"]`);
    if (id === "scrobble") put(spot, targetOf(id), 4);
    if (id === "tray") put(spot, targetOf(id), 0);
    if (id === "sponsor") {
      const rail = targetOf(id);
      put(spot, rail);
      if (spot && rail) {
        const r = rail.getBoundingClientRect();
        spot.style.top = `${(r.top - box.top) / scale - 1}px`;
      }
    }
  };

  // Where each place is on the laptop at rest (no zoom), as a share of the zoom layer.
  let points: Record<string, Zoom> = {};
  const measure = () => {
    if (!zoom) return;
    const keep = zoom.style.transform;
    zoom.style.transition = "none";
    zoom.style.transform = "none";
    const box = zoom.getBoundingClientRect();
    points = {};
    for (const b of items) {
      const id = b.dataset.feature!;
      const t = targetOf(id);
      if (!t || !box.width) continue;
      const r = t.getBoundingClientRect();
      points[id] = { x: (r.left + r.width / 2 - box.left) / box.width, y: (r.top + r.height / 2 - box.top) / box.height, z: ZOOM[id] ?? 1.5 };
    }
    zoom.style.transform = keep;
    void zoom.offsetWidth;
    zoom.style.transition = "";
  };

  const zoomFor = (id: string | null) => {
    if (!zoom) return;
    const p = id ? points[id] : undefined;
    if (!p || !wide.matches) {
      zoom.style.transform = "";
      return;
    }
    const w = zoom.offsetWidth;
    const h = zoom.offsetHeight;
    zoom.style.transform = zoomTo(w, h, p.x * w, p.y * h, reduced.matches ? Math.min(p.z, 1.45) : p.z);
  };

  let on: string | null | undefined;
  const show = () => {
    const id = pointed ?? pinned ?? scrolled;
    if (id === on) return;
    on = id;
    if (id) place(id);
    if (id) root.dataset.highlight = id;
    else delete root.dataset.highlight;
    for (const b of items) b.setAttribute("aria-pressed", String(b.dataset.feature === id));
    zoomFor(id);
  };

  for (const b of items) {
    const id = b.dataset.feature!;
    b.addEventListener("pointerenter", (e) => {
      if (e.pointerType !== "mouse") return;
      pointed = id;
      show();
    });
    b.addEventListener("pointerleave", () => {
      pointed = null;
      show();
    });
    b.addEventListener("focus", () => {
      pointed = id;
      show();
    });
    b.addEventListener("blur", () => {
      pointed = null;
      show();
    });
    b.addEventListener("click", () => {
      pinned = pinned === id ? null : id;
      pointed = id;
      on = undefined;
      show();
    });
  }

  // The step crossing the middle of the screen is the one on (wide screens). Above the
  // first, none: the whole laptop. Past the last, the last stays.
  if (typeof IntersectionObserver !== "undefined" && steps.length) {
    const io = new IntersectionObserver(
      (entries) => {
        if (!wide.matches) return;
        for (const e of entries) {
          const id = (e.target as HTMLElement).dataset.step!;
          if (e.isIntersecting) scrolled = id;
          else if (scrolled === id) {
            // It left the middle line: upwards past the first means none.
            const above = e.boundingClientRect.top > innerHeight / 2;
            if (above && id === steps[0].dataset.step) scrolled = null;
          }
        }
        // A tap pins until the visitor scrolls on to another step.
        if (pinned && scrolled !== pinned) pinned = null;
        show();
      },
      { rootMargin: "-50% 0px -50% 0px" },
    );
    for (const s of steps) io.observe(s);
  }

  measure();
  addEventListener(
    "resize",
    () => {
      measure();
      on = undefined;
      show();
    },
    { passive: true },
  );
  wide.addEventListener("change", () => {
    on = undefined;
    show();
  });

  let song = "";
  let second = -1;
  player.subscribe((s) => {
    if (!s.song) return;
    if (s.song.id !== song) {
      song = s.song.id;
      for (const img of root.querySelectorAll<HTMLImageElement>("[data-kept-cover]")) img.src = s.song.cover;
      for (const el of root.querySelectorAll("[data-kept-title]")) el.textContent = s.song.title;
      for (const el of root.querySelectorAll("[data-kept-artist]")) el.textContent = `by ${s.song.artist}`;
      for (const el of root.querySelectorAll("[data-kept-artist-plain]")) el.textContent = s.song.artist;
    }
    const sec = Math.floor(s.positionMs / 1000);
    if (sec !== second) {
      second = sec;
      for (const el of root.querySelectorAll("[data-kept-time]")) el.textContent = `${formatTime(s.positionMs)} elapsed`;
    }
  });
}
